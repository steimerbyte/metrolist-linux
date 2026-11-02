//! Dedicated threads for Last.fm scrobbling and Discord IPC.
//!
//! The UI thread only sends snapshots/commands and reads a counters snapshot;
//! queue writes, HTTP signing/flushing and socket handshakes stay off the frame
//! path.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use ml_core::{MlError, PlaybackStatus, PlayerSnapshot};
use ml_integrations::lastfm::{
    playback_transition, LastFmClient, LastFmConfig, LastFmTransport, Scrobble, ScrobblePolicy,
    ScrobbleQueue, Scrobbler, ScrobblerHooks, Transition,
};
use ml_integrations::DiscordPresence;
use parking_lot::Mutex;

use crate::integrations::{DiscordStatus, LastFmStatus};
use crate::pure::redact_secrets;
use crate::services::{Publisher, UiEvent};

const FLUSH_INTERVAL: Duration = Duration::from_secs(30);
const DISCORD_UPDATE_INTERVAL: Duration = Duration::from_secs(15);
const SEEK_JUMP_MS: u64 = 2_000;

// Snapshots are sent at frame rate; boxing them would allocate every frame.
// Configuration commands are rare, so keeping the enum flat is the cheaper path.
#[allow(clippy::large_enum_variant)]
enum LastFmCommand {
    Snapshot(PlayerSnapshot),
    Configure {
        config: LastFmConfig,
        enabled: bool,
        username: Option<String>,
        flush: bool,
    },
    SignOut,
    Forget,
    Shutdown,
}

#[allow(clippy::large_enum_variant)]
enum LastFmInternal {
    NowPlaying(Result<(), MlError>),
    Flush {
        keys: Vec<String>,
        count: usize,
        result: Result<(), MlError>,
    },
}

/// Owner of the scrobbler thread and its published counters.
pub(crate) struct LastFmWorker {
    tx: Sender<LastFmCommand>,
    status: Arc<Mutex<LastFmStatus>>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl LastFmWorker {
    pub(crate) fn new(
        handle: tokio::runtime::Handle,
        queue_path: PathBuf,
        transport: Arc<dyn LastFmTransport>,
        publisher: Publisher,
    ) -> Self {
        let (tx, rx) = unbounded();
        let status = Arc::new(Mutex::new(LastFmStatus::default()));
        let worker_status = status.clone();
        let join = std::thread::Builder::new()
            .name("ml-lastfm".into())
            .spawn(move || run_lastfm(handle, queue_path, transport, publisher, worker_status, rx))
            .ok();
        Self {
            tx,
            status,
            join: Mutex::new(join),
        }
    }

    pub(crate) fn status(&self) -> LastFmStatus {
        self.status.lock().clone()
    }

    pub(crate) fn configure(
        &self,
        config: LastFmConfig,
        enabled: bool,
        username: Option<String>,
        flush: bool,
    ) {
        let _ = self.tx.send(LastFmCommand::Configure {
            config,
            enabled,
            username,
            flush,
        });
    }

    pub(crate) fn snapshot(&self, snapshot: PlayerSnapshot) {
        let _ = self.tx.send(LastFmCommand::Snapshot(snapshot));
    }

    pub(crate) fn sign_out(&self) {
        let _ = self.tx.send(LastFmCommand::SignOut);
    }

    pub(crate) fn forget(&self) {
        let _ = self.tx.send(LastFmCommand::Forget);
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.tx.send(LastFmCommand::Shutdown);
        if let Some(join) = self.join.lock().take() {
            let _ = join.join();
        }
    }
}

impl Drop for LastFmWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct LastFmMachine {
    scrobbler: Scrobbler,
    config: LastFmConfig,
    enabled: bool,
    username: Option<String>,
    session_invalid: bool,
    last_error: Option<String>,
    previous: Option<PlayerSnapshot>,
    last_position: u64,
    flushing: bool,
    retry_after: Instant,
}

impl LastFmMachine {
    fn new(queue: ScrobbleQueue) -> Self {
        Self {
            scrobbler: Scrobbler::new(ScrobblerHooks::default(), queue),
            config: LastFmConfig::default(),
            enabled: false,
            username: None,
            session_invalid: false,
            last_error: None,
            previous: None,
            last_position: 0,
            flushing: false,
            retry_after: Instant::now(),
        }
    }

    fn can_scrobble(&self) -> bool {
        self.config.can_scrobble()
            && self
                .config
                .secret
                .as_deref()
                .is_some_and(|secret| !secret.trim().is_empty())
    }

    fn sync_hooks(&mut self) {
        let enabled = self.enabled && self.can_scrobble() && !self.session_invalid;
        self.scrobbler.set_hooks(ScrobblerHooks {
            enabled,
            now_playing_enabled: true,
            policy: ScrobblePolicy::default(),
        });
    }

    fn status(&self) -> LastFmStatus {
        let stats = self.scrobbler.stats();
        let signed_in = self.can_scrobble() && !self.session_invalid;
        LastFmStatus {
            signed_in,
            user: signed_in.then(|| self.username.clone()).flatten(),
            scrobbles_sent: stats.scrobbles_sent,
            queued: self.scrobbler.queue().len() as u64,
            session_invalid: self.session_invalid,
            last_error: self
                .last_error
                .clone()
                .or_else(|| self.scrobbler.queue().last_error().map(str::to_owned)),
            flushing: self.flushing,
        }
    }

    fn redacted(&self, text: &str) -> String {
        redact_secrets(
            text,
            &[
                &self.config.api_key,
                self.config.secret.as_deref().unwrap_or(""),
                self.config.session.as_deref().unwrap_or(""),
            ],
        )
    }

    fn enqueue(&mut self, scrobble: Option<Scrobble>) -> bool {
        let Some(scrobble) = scrobble else {
            return false;
        };
        self.scrobbler.enqueue(scrobble)
    }

    fn note_success(&mut self, now_playing: bool) {
        self.scrobbler.record_success(now_playing);
        self.last_error = None;
        self.retry_after = Instant::now();
    }

    fn note_failure(&mut self, error: &MlError, publisher: &Publisher) -> bool {
        self.scrobbler.record_failure(error);
        self.retry_after = Instant::now() + Duration::from_secs(30);
        let invalid = self.scrobbler.take_invalid_session();
        self.last_error = Some(self.redacted(&error.to_string()));
        if let Some(event) = invalid {
            let was_invalid = self.session_invalid;
            self.session_invalid = true;
            self.sync_hooks();
            if !was_invalid {
                publisher.send(UiEvent::IntegrationNotice {
                    message: format!(
                        "Last.fm session rejected ({}). Reconnect in Settings > Integrations.",
                        event.code
                    ),
                    lastfm_disabled: true,
                });
            }
            true
        } else {
            false
        }
    }

    fn handle_snapshot(&mut self, snapshot: PlayerSnapshot) -> Option<ml_core::Song> {
        let now = Instant::now();
        let mut now_playing = None;
        let transition = playback_transition(self.previous.as_ref(), &snapshot);
        let seeked = self.previous.as_ref().is_some_and(|previous| {
            previous.current.as_ref().map(|song| &song.id)
                == snapshot.current.as_ref().map(|song| &song.id)
        }) && snapshot.position_ms.abs_diff(self.last_position) > SEEK_JUMP_MS;
        match transition {
            Transition::TrackChanged => {
                if self
                    .previous
                    .as_ref()
                    .and_then(|previous| previous.current.as_ref())
                    .is_some()
                {
                    let due = self.scrobbler.on_stopped(now);
                    self.enqueue(due);
                }
                if let Some(song) = snapshot.current.as_ref() {
                    now_playing = self.scrobbler.on_song_started(song, unix_now(), now);
                }
            }
            Transition::Paused if snapshot.status == PlaybackStatus::Ended => {
                let due = self.scrobbler.on_stopped(now);
                self.enqueue(due);
            }
            Transition::Paused => self.scrobbler.on_paused(now),
            Transition::Resumed => self.scrobbler.on_resumed(now),
            Transition::Stopped => {
                let due = self.scrobbler.on_stopped(now);
                self.enqueue(due);
            }
            Transition::None => {}
        }
        if seeked && transition != Transition::TrackChanged {
            self.scrobbler.on_seeked(snapshot.position_ms, now);
        }
        if snapshot.is_playing() {
            let due = self.scrobbler.poll(now);
            self.enqueue(due);
        }
        self.previous = Some(snapshot.clone());
        self.last_position = snapshot.position_ms;
        now_playing
    }

    fn try_flush(
        &mut self,
        handle: &tokio::runtime::Handle,
        transport: &Arc<dyn LastFmTransport>,
        tx: &Sender<LastFmInternal>,
        publisher: &Publisher,
    ) {
        if self.flushing
            || Instant::now() < self.retry_after
            || !self.enabled
            || !self.can_scrobble()
            || self.session_invalid
            || self.scrobbler.queue().is_empty()
        {
            return;
        }
        let batch = self.scrobbler.queue().peek_batch();
        if batch.is_empty() {
            return;
        }
        let keys: Vec<String> = batch.iter().map(|item| item.key.clone()).collect();
        let count = batch.len();
        let client = LastFmClient::new(self.config.clone(), transport.clone());
        let tx = tx.clone();
        self.flushing = true;
        publisher.repaint();
        handle.spawn(async move {
            let result = client.scrobble_batch(&batch).await;
            let _ = tx.send(LastFmInternal::Flush {
                keys,
                count,
                result,
            });
        });
    }

    fn spawn_now_playing(
        &self,
        handle: &tokio::runtime::Handle,
        transport: &Arc<dyn LastFmTransport>,
        tx: &Sender<LastFmInternal>,
        song: ml_core::Song,
    ) {
        let client = LastFmClient::new(self.config.clone(), transport.clone());
        let tx = tx.clone();
        handle.spawn(async move {
            let result = client.update_now_playing(&song).await;
            let _ = tx.send(LastFmInternal::NowPlaying(result));
        });
    }

    fn forget(&mut self) {
        self.scrobbler.queue_mut().clear();
        self.scrobbler.set_hooks(ScrobblerHooks::default());
        self.config = LastFmConfig::default();
        self.enabled = false;
        self.username = None;
        self.session_invalid = false;
        self.last_error = None;
        self.previous = None;
        self.flushing = false;
        self.retry_after = Instant::now();
    }
}

fn run_lastfm(
    handle: tokio::runtime::Handle,
    queue_path: PathBuf,
    transport: Arc<dyn LastFmTransport>,
    publisher: Publisher,
    status: Arc<Mutex<LastFmStatus>>,
    commands: Receiver<LastFmCommand>,
) {
    let (internal_tx, internal_rx) = unbounded();
    let mut machine = LastFmMachine::new(ScrobbleQueue::load(queue_path));
    let mut last_flush = Instant::now();
    *status.lock() = machine.status();
    publisher.repaint();
    loop {
        let command = match commands.recv_timeout(Duration::from_millis(200)) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match command {
            Some(LastFmCommand::Snapshot(snapshot)) => {
                if let Some(song) = machine.handle_snapshot(snapshot) {
                    machine.spawn_now_playing(&handle, &transport, &internal_tx, song);
                }
            }
            Some(LastFmCommand::Configure {
                config,
                enabled,
                username,
                flush,
            }) => {
                let session_changed = machine.config.session != config.session;
                machine.config = config;
                machine.enabled = enabled;
                machine.username = username;
                machine.session_invalid = false;
                machine.last_error = None;
                machine.retry_after = Instant::now();
                if session_changed {
                    machine.previous = None;
                }
                machine.sync_hooks();
                if flush {
                    machine.try_flush(&handle, &transport, &internal_tx, &publisher);
                }
            }
            Some(LastFmCommand::SignOut) => {
                machine.config.session = None;
                machine.enabled = false;
                machine.username = None;
                machine.session_invalid = false;
                machine.last_error = None;
                machine.sync_hooks();
            }
            Some(LastFmCommand::Forget) => machine.forget(),
            Some(LastFmCommand::Shutdown) => break,
            None => {
                if last_flush.elapsed() >= FLUSH_INTERVAL {
                    machine.try_flush(&handle, &transport, &internal_tx, &publisher);
                    last_flush = Instant::now();
                }
            }
        }
        while let Ok(result) = internal_rx.try_recv() {
            match result {
                LastFmInternal::NowPlaying(Ok(())) => machine.note_success(true),
                LastFmInternal::NowPlaying(Err(error)) => {
                    machine.note_failure(&error, &publisher);
                }
                LastFmInternal::Flush {
                    keys,
                    count,
                    result,
                } => {
                    machine.flushing = false;
                    match result {
                        Ok(()) => {
                            machine.scrobbler.queue_mut().remove(&keys);
                            for _ in 0..count {
                                machine.note_success(false);
                            }
                        }
                        Err(error) => {
                            machine.note_failure(&error, &publisher);
                        }
                    }
                }
            }
        }
        if machine.enabled && !machine.flushing {
            machine.try_flush(&handle, &transport, &internal_tx, &publisher);
        }
        let next = machine.status();
        let changed = {
            let mut current = status.lock();
            if *current == next {
                false
            } else {
                *current = next;
                true
            }
        };
        if changed {
            publisher.repaint();
        }
    }
    machine.scrobbler.set_hooks(ScrobblerHooks::default());
    *status.lock() = machine.status();
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

// ------------------------------------------------------------------ discord

#[allow(clippy::large_enum_variant)]
enum DiscordCommand {
    Snapshot(PlayerSnapshot),
    Shutdown,
}

/// Owner of the Discord IPC thread. Starts only while the feature is enabled.
pub(crate) struct DiscordWorker {
    tx: Sender<DiscordCommand>,
    status: Arc<Mutex<DiscordStatus>>,
    join: Mutex<Option<JoinHandle<()>>>,
    app_id: String,
}

impl DiscordWorker {
    pub(crate) fn new(app_id: String, publisher: Publisher) -> Self {
        let (tx, rx) = unbounded();
        let status = Arc::new(Mutex::new(DiscordStatus {
            enabled: true,
            waiting: true,
            ..DiscordStatus::default()
        }));
        let worker_status = status.clone();
        let worker_app_id = app_id.clone();
        let join = std::thread::Builder::new()
            .name("ml-discord".into())
            .spawn(move || run_discord(worker_app_id, worker_status, rx, publisher))
            .ok();
        Self {
            tx,
            status,
            join: Mutex::new(join),
            app_id,
        }
    }

    pub(crate) fn app_id(&self) -> &str {
        &self.app_id
    }

    pub(crate) fn status(&self) -> DiscordStatus {
        self.status.lock().clone()
    }

    pub(crate) fn snapshot(&self, snapshot: PlayerSnapshot) {
        let _ = self.tx.send(DiscordCommand::Snapshot(snapshot));
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.tx.send(DiscordCommand::Shutdown);
        if let Some(join) = self.join.lock().take() {
            let _ = join.join();
        }
    }
}

impl Drop for DiscordWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_discord(
    app_id: String,
    status: Arc<Mutex<DiscordStatus>>,
    commands: Receiver<DiscordCommand>,
    publisher: Publisher,
) {
    let mut presence = DiscordPresence::with_app_id(app_id);
    let mut last_key: Option<(String, PlaybackStatus)> = None;
    let mut last_sent = Instant::now() - DISCORD_UPDATE_INTERVAL;
    loop {
        let command = match commands.recv_timeout(Duration::from_millis(250)) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match command {
            Some(DiscordCommand::Snapshot(snapshot)) => {
                let key = snapshot
                    .current
                    .as_ref()
                    .map(|song| (song.id.clone(), snapshot.status));
                let changed = key != last_key;
                let due = snapshot.is_playing() && last_sent.elapsed() >= DISCORD_UPDATE_INTERVAL;
                if changed || due {
                    let _ = presence.on_snapshot(&snapshot);
                    last_key = key;
                    last_sent = Instant::now();
                }
            }
            Some(DiscordCommand::Shutdown) => break,
            None => {}
        }
        presence.pump();
        let next = {
            let mut current = status.lock();
            current.connected = presence.is_connected();
            current.waiting = current.enabled && !current.connected;
            current.last_error = presence.last_error().map(str::to_owned);
            current.clone()
        };
        let changed = {
            let mut current = status.lock();
            if *current == next {
                false
            } else {
                *current = next;
                true
            }
        };
        if changed {
            publisher.repaint();
        }
    }
    presence.set_enabled(false);
    presence.clear();
    *status.lock() = DiscordStatus::default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex as StdMutex;

    use ml_core::{Artist, MlResult, Song};

    #[derive(Default)]
    struct FakeTransport {
        requests: StdMutex<Vec<BTreeMap<String, String>>>,
        invalid_scrobble: bool,
    }

    impl LastFmTransport for FakeTransport {
        fn post<'a>(
            &'a self,
            params: &'a BTreeMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = MlResult<String>> + Send + 'a>> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(params.clone());
                if self.invalid_scrobble
                    && params.get("method").map(String::as_str) == Some("track.scrobble")
                {
                    return Ok(r#"{"error":9,"message":"Invalid session key"}"#.to_string());
                }
                Ok(r#"{"ok":true}"#.to_string())
            })
        }
    }

    fn wait_until(mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !check() {
            assert!(Instant::now() < deadline, "worker did not settle");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn worker_sends_now_playing_and_flushes_once() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let ctx = egui::Context::default();
        let (_publisher, _events) = Publisher::for_test(ctx);
        let dir = std::env::temp_dir().join(format!(
            "metrolist-worker-{}-{}",
            std::process::id(),
            unix_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let worker = LastFmWorker::new(
            rt.handle().clone(),
            dir.join("lastfm-queue.jsonl"),
            transport.clone(),
            _publisher,
        );
        worker.configure(
            LastFmConfig::new(
                "api-key",
                Some("secret".to_string()),
                Some("session".to_string()),
            ),
            true,
            Some("listener".to_string()),
            true,
        );
        let song = Song {
            id: "worker-song".to_string(),
            title: "Worker Song".to_string(),
            artists: vec![Artist::new("Artist", None)],
            duration: Some(200),
            ..Song::default()
        };
        let mut snapshot = PlayerSnapshot {
            status: PlaybackStatus::Playing,
            position_ms: 0,
            current: Some(song),
            duration_ms: 200_000,
            ..PlayerSnapshot::default()
        };
        worker.snapshot(snapshot.clone());
        wait_until(|| {
            transport.requests.lock().unwrap().iter().any(|request| {
                request.get("method").map(String::as_str) == Some("track.updateNowPlaying")
            })
        });

        // A large forward seek makes the track due; the worker enqueues and
        // flushes it without waiting for a real 50-second threshold.
        snapshot.position_ms = 100_000;
        worker.snapshot(snapshot.clone());
        wait_until(|| worker.status().scrobbles_sent == 1);
        assert_eq!(worker.status().queued, 0);
        worker.snapshot(snapshot);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            worker.status().scrobbles_sent,
            1,
            "repeat must not duplicate"
        );
        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(
                    |request| request.get("method").map(String::as_str) == Some("track.scrobble")
                )
                .count(),
            1
        );
        drop(requests);
        drop(worker);
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn worker_reports_invalid_session_and_keeps_queue() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let transport = Arc::new(FakeTransport {
            invalid_scrobble: true,
            ..FakeTransport::default()
        });
        let ctx = egui::Context::default();
        let (publisher, events) = Publisher::for_test(ctx);
        let dir = std::env::temp_dir().join(format!(
            "metrolist-worker-invalid-{}-{}",
            std::process::id(),
            unix_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let worker = LastFmWorker::new(
            rt.handle().clone(),
            dir.join("lastfm-queue.jsonl"),
            transport,
            publisher,
        );
        worker.configure(
            LastFmConfig::new(
                "api-key",
                Some("secret".to_string()),
                Some("session".to_string()),
            ),
            true,
            Some("listener".to_string()),
            false,
        );
        let song = Song {
            id: "invalid-song".to_string(),
            title: "Invalid Song".to_string(),
            artists: vec![Artist::new("Artist", None)],
            duration: Some(200),
            ..Song::default()
        };
        let mut snapshot = PlayerSnapshot {
            status: PlaybackStatus::Playing,
            position_ms: 0,
            current: Some(song),
            duration_ms: 200_000,
            ..PlayerSnapshot::default()
        };
        worker.snapshot(snapshot.clone());
        snapshot.position_ms = 100_000;
        worker.snapshot(snapshot);
        wait_until(|| worker.status().session_invalid);
        let status = worker.status();
        assert!(!status.signed_in, "invalid session must not look signed in");
        assert_eq!(status.queued, 1, "failed scrobble stays queued");
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)),
            Ok(UiEvent::IntegrationNotice {
                lastfm_disabled: true,
                ..
            })
        ));
        drop(worker);
        let _ = std::fs::remove_dir_all(dir);
    }
}
