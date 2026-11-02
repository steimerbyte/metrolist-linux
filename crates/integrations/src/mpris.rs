//! MPRIS2 service (`org.mpris.MediaPlayer2.metrolist`).
//!
//! [`MprisService`] owns one thread with a current-thread tokio runtime that
//! hosts the `mpris-server` connection. The UI calls [`MprisService::update`]
//! with every [`PlayerSnapshot`] it receives; the service diffs it against the
//! last published snapshot (see [`mapping::snapshot_changes`]) and emits
//! `PropertiesChanged` only for what actually moved.
//!
//! Property getters read `PlayerControl::state()` live, so a client never sees
//! a stale value even if the snapshot stream lags.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crossbeam_channel::bounded;
use ml_core::{MlError, MlResult, PlayerSnapshot, RepeatMode, Song};
use mpris_server::{
    zbus::fdo, LoopStatus, Metadata, PlaybackStatus, PlayerInterface, Property, RootInterface,
    Server, Signal, Time, TrackId,
};

use crate::mapping::{
    can_go_next, can_go_previous, ms_to_us, position_ms_from_us, repeat_from_loop,
    snapshot_changes, track_metadata, track_path, transport_loop, transport_status, us_to_ms,
    volume_of, Change, Published, TrackMetadata, TransportLoop, TransportStatus,
};

/// Bus name the service owns.
pub const MPRIS_BUS_NAME: &str = "org.mpris.MediaPlayer2.metrolist";
/// Bus-name suffix handed to `mpris-server` (it prefixes `org.mpris.MediaPlayer2.`).
pub const MPRIS_BUS_SUFFIX: &str = "metrolist";
/// Identity clients display.
pub const MPRIS_IDENTITY: &str = "Metrolist Desktop";
/// Basename of the installed desktop entry (see `packaging/Metrolist-Desktop.desktop`).
pub const DESKTOP_ENTRY: &str = "Metrolist-Desktop";
/// Only URI scheme we accept; the desktop file registers `x-scheme-handler/metrolist`.
pub const URI_SCHEME: &str = "metrolist";

/// Everything the desktop surfaces need from the app.
///
/// Implemented by the UI layer on top of the player/queue controller. All
/// methods take `&self` and must be cheap and non-blocking; they are called
/// from the MPRIS D-Bus thread. Errors are surfaced to the client as
/// `org.freedesktop.DBus.Error.Failed` instead of being swallowed.
pub trait PlayerControl: Send + Sync {
    fn play(&self) -> MlResult<()>;
    fn pause(&self) -> MlResult<()>;
    fn play_pause(&self) -> MlResult<()>;
    fn stop(&self) -> MlResult<()>;
    fn next(&self) -> MlResult<()>;
    fn previous(&self) -> MlResult<()>;
    /// Relative seek, negative moves backwards.
    fn seek_by_ms(&self, offset_ms: i64) -> MlResult<()>;
    /// Absolute seek inside the current track.
    fn set_position_ms(&self, position_ms: u64) -> MlResult<()>;
    fn set_volume(&self, volume: f64) -> MlResult<()>;
    fn set_shuffle(&self, shuffle: bool) -> MlResult<()>;
    fn set_repeat(&self, repeat: RepeatMode) -> MlResult<()>;
    fn state(&self) -> PlayerSnapshot;
    fn current_song(&self) -> Option<Song>;
    /// Artwork override (local file or URL) for the current track, if any.
    fn current_art_url(&self) -> Option<String> {
        None
    }
    /// Bring the window to the front. Override together with [`Self::can_raise`].
    fn raise(&self) -> MlResult<()> {
        Ok(())
    }
    fn can_raise(&self) -> bool {
        true
    }
    /// Handle a `metrolist://` (or other supported) URI.
    fn open_uri(&self, uri: &str) -> MlResult<()>;
    /// Quit the application.
    fn quit(&self) -> MlResult<()>;
}

/// Commands a [`SnapshotControl`] forwards to the app's player.
///
/// The UI applies these to its player/queue controller; keeping them as values
/// (not a callback) means the MPRIS thread never calls into UI state directly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    SeekBy(i64),
    SetPosition(u64),
    SetVolume(f32),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    Raise,
    Quit,
}

/// [`PlayerControl`] adapter over shared snapshot state.
///
/// The app publishes the latest [`PlayerSnapshot`] and current [`Song`] with
/// [`SnapshotControl::set_state`]; MPRIS/D-Bus reads it live and every control
/// becomes a [`PlayerCommand`] on a channel the player thread drains. That
/// removes the need to write a `PlayerControl` impl per player.
#[derive(Debug)]
pub struct SnapshotControl {
    state: Mutex<PlayerState>,
    commands: crossbeam_channel::Sender<PlayerCommand>,
}

/// Deep-link handler installed by the app.
type UriHandler = Box<dyn Fn(&str) + Send + Sync>;

/// What [`SnapshotControl`] stores.
#[derive(Default)]
struct PlayerState {
    snapshot: PlayerSnapshot,
    song: Option<Song>,
    art_url: Option<String>,
    uri_handler: Option<UriHandler>,
}

impl std::fmt::Debug for PlayerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerState")
            .field("song", &self.song.as_ref().map(|song| song.id.as_str()))
            .field("art_url", &self.art_url)
            .field("has_uri_handler", &self.uri_handler.is_some())
            .finish()
    }
}

impl SnapshotControl {
    /// Build an adapter; the returned receiver yields [`PlayerCommand`]s.
    pub fn new() -> (Arc<Self>, crossbeam_channel::Receiver<PlayerCommand>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (
            Arc::new(Self {
                state: Mutex::new(PlayerState::default()),
                commands: tx,
            }),
            rx,
        )
    }

    /// Publish player state. Call whenever the player produces a snapshot.
    pub fn set_state(&self, snapshot: PlayerSnapshot, song: Option<Song>, art_url: Option<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.snapshot = snapshot;
        state.song = song;
        state.art_url = art_url;
    }

    /// Install the deep-link handler (`metrolist://…`).
    pub fn set_uri_handler(&self, handler: impl Fn(&str) + Send + Sync + 'static) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.uri_handler = Some(Box::new(handler));
    }

    fn send(&self, command: PlayerCommand) -> MlResult<()> {
        self.commands
            .try_send(command)
            .map_err(|error| MlError::Unavailable(format!("player command channel: {error}")))
    }
}

impl PlayerControl for SnapshotControl {
    fn play(&self) -> MlResult<()> {
        self.send(PlayerCommand::Play)
    }
    fn pause(&self) -> MlResult<()> {
        self.send(PlayerCommand::Pause)
    }
    fn play_pause(&self) -> MlResult<()> {
        self.send(PlayerCommand::PlayPause)
    }
    fn stop(&self) -> MlResult<()> {
        self.send(PlayerCommand::Stop)
    }
    fn next(&self) -> MlResult<()> {
        self.send(PlayerCommand::Next)
    }
    fn previous(&self) -> MlResult<()> {
        self.send(PlayerCommand::Previous)
    }
    fn seek_by_ms(&self, offset_ms: i64) -> MlResult<()> {
        self.send(PlayerCommand::SeekBy(offset_ms))
    }
    fn set_position_ms(&self, position_ms: u64) -> MlResult<()> {
        self.send(PlayerCommand::SetPosition(position_ms))
    }
    fn set_volume(&self, volume: f64) -> MlResult<()> {
        self.send(PlayerCommand::SetVolume(volume as f32))
    }
    fn set_shuffle(&self, shuffle: bool) -> MlResult<()> {
        self.send(PlayerCommand::SetShuffle(shuffle))
    }
    fn set_repeat(&self, repeat: RepeatMode) -> MlResult<()> {
        self.send(PlayerCommand::SetRepeat(repeat))
    }
    fn state(&self) -> PlayerSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .snapshot
            .clone()
    }
    fn current_song(&self) -> Option<Song> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .song
            .clone()
    }
    fn current_art_url(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .art_url
            .clone()
    }
    fn raise(&self) -> MlResult<()> {
        self.send(PlayerCommand::Raise)
    }
    fn open_uri(&self, uri: &str) -> MlResult<()> {
        // Call under the lock: the handler is an app closure and may not
        // re-enter this adapter, but serializing it keeps `set_uri_handler`
        // from swapping it mid-call.
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let handler = state
            .uri_handler
            .as_deref()
            .ok_or_else(|| MlError::Unavailable("no URI handler installed".into()))?;
        handler(uri);
        Ok(())
    }
    fn quit(&self) -> MlResult<()> {
        self.send(PlayerCommand::Quit)
    }
}

fn dbus_err(error: MlError) -> fdo::Error {
    fdo::Error::Failed(error.to_string())
}

fn proto_status(status: TransportStatus) -> PlaybackStatus {
    match status {
        TransportStatus::Playing => PlaybackStatus::Playing,
        TransportStatus::Paused => PlaybackStatus::Paused,
        TransportStatus::Stopped => PlaybackStatus::Stopped,
    }
}

fn proto_loop(loop_status: TransportLoop) -> LoopStatus {
    match loop_status {
        TransportLoop::None => LoopStatus::None,
        TransportLoop::Track => LoopStatus::Track,
        TransportLoop::Playlist => LoopStatus::Playlist,
    }
}

fn loop_from_proto(loop_status: LoopStatus) -> TransportLoop {
    match loop_status {
        LoopStatus::None => TransportLoop::None,
        LoopStatus::Track => TransportLoop::Track,
        LoopStatus::Playlist => TransportLoop::Playlist,
    }
}

fn track_id_of(path: &str) -> TrackId {
    // `track_path` always yields a valid object path; NO_TRACK is the only
    // safe stand-in if that invariant is ever broken.
    TrackId::try_from(path.to_owned()).unwrap_or(TrackId::NO_TRACK)
}

fn metadata_for(control: &dyn PlayerControl) -> Metadata {
    let Some(song) = control.current_song() else {
        return Metadata::builder().trackid(TrackId::NO_TRACK).build();
    };
    let art = control.current_art_url();
    let track = track_metadata(&song, art.as_deref());
    metadata_from(&track)
}

fn metadata_from(track: &TrackMetadata) -> Metadata {
    let mut builder = Metadata::builder()
        .trackid(track_id_of(&track.track_id))
        .title(track.title.clone())
        .length(Time::from_micros(track.length_us));
    if !track.artists.is_empty() {
        builder = builder.artist(track.artists.clone());
    }
    if let Some(album) = &track.album {
        builder = builder.album(album.clone());
    }
    if let Some(art_url) = &track.art_url {
        builder = builder.art_url(art_url.clone());
    }
    builder.build()
}

/// The `mpris-server` implementation. Holds no player state of its own beyond
/// the last published snapshot, which is only used for diffing.
struct MprisImp {
    control: Arc<dyn PlayerControl>,
    last: Mutex<Option<Published>>,
    /// Out-going `Seeked` notifications, drained by the service loop.
    events: tokio::sync::mpsc::UnboundedSender<Event>,
}

impl MprisImp {
    fn snapshot(&self) -> PlayerSnapshot {
        self.control.state()
    }

    fn publish_seeked(&self) {
        let position = ms_to_us(self.snapshot().position_ms);
        let _ = self.events.send(Event::Seeked(Time::from_micros(position)));
    }
}

impl RootInterface for MprisImp {
    async fn raise(&self) -> fdo::Result<()> {
        self.control.raise().map_err(dbus_err)
    }

    async fn quit(&self) -> fdo::Result<()> {
        self.control.quit().map_err(dbus_err)
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(self.control.can_raise())
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok(MPRIS_IDENTITY.to_owned())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok(DESKTOP_ENTRY.to_owned())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![URI_SCHEME.to_owned()])
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl PlayerInterface for MprisImp {
    async fn next(&self) -> fdo::Result<()> {
        self.control.next().map_err(dbus_err)
    }

    async fn previous(&self) -> fdo::Result<()> {
        self.control.previous().map_err(dbus_err)
    }

    async fn pause(&self) -> fdo::Result<()> {
        self.control.pause().map_err(dbus_err)
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        self.control.play_pause().map_err(dbus_err)
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.control.stop().map_err(dbus_err)
    }

    async fn play(&self) -> fdo::Result<()> {
        self.control.play().map_err(dbus_err)
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        self.control
            .seek_by_ms(us_to_ms(offset.as_micros()))
            .map_err(dbus_err)?;
        self.publish_seeked();
        Ok(())
    }

    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        let Some(song) = self.control.current_song() else {
            return Ok(());
        };
        if track_id.as_str() != track_path(&song.id) {
            // Stale call for a track that is no longer current: ignore per spec.
            return Ok(());
        }
        self.control
            .set_position_ms(position_ms_from_us(position.as_micros()))
            .map_err(dbus_err)?;
        self.publish_seeked();
        Ok(())
    }

    async fn open_uri(&self, uri: String) -> fdo::Result<()> {
        self.control.open_uri(&uri).map_err(dbus_err)
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(proto_status(transport_status(self.snapshot().status)))
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(proto_loop(transport_loop(self.snapshot().repeat)))
    }

    async fn set_loop_status(&self, loop_status: LoopStatus) -> mpris_server::zbus::Result<()> {
        self.control
            .set_repeat(repeat_from_loop(loop_from_proto(loop_status)))
            .map_err(dbus_err)?;
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn set_rate(&self, _rate: f64) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self.snapshot().shuffle)
    }

    async fn set_shuffle(&self, shuffle: bool) -> mpris_server::zbus::Result<()> {
        self.control.set_shuffle(shuffle).map_err(dbus_err)?;
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(metadata_for(self.control.as_ref()))
    }

    async fn volume(&self) -> fdo::Result<f64> {
        Ok(volume_of(&self.snapshot()))
    }

    async fn set_volume(&self, volume: f64) -> mpris_server::zbus::Result<()> {
        self.control.set_volume(volume).map_err(dbus_err)?;
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_micros(ms_to_us(self.snapshot().position_ms)))
    }

    async fn minimum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(can_go_next(&self.snapshot()))
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(can_go_previous(&self.snapshot()))
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(self.control.current_song().is_some())
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

/// Messages from the app into the service thread.
///
/// `Update` is boxed: a snapshot is ~350 bytes and the other two events are
/// single integers, so an unboxed enum would size every `Seeked` the same.
enum Event {
    Update(Box<PlayerSnapshot>),
    Seeked(Time),
    Quit,
}

/// Handle to the running MPRIS service. Dropping it stops the D-Bus thread.
pub struct MprisService {
    events: tokio::sync::mpsc::UnboundedSender<Event>,
    last_error: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl MprisService {
    /// Register `org.mpris.MediaPlayer2.metrolist` and return once the bus name
    /// is owned. Returns the D-Bus error if registration failed (no session
    /// bus, name already taken).
    pub fn spawn(control: Arc<dyn PlayerControl>) -> MlResult<Self> {
        let (events, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = bounded::<MlResult<()>>(1);
        let last_error = Arc::new(Mutex::new(None));

        let thread = std::thread::Builder::new()
            .name("ml-mpris".into())
            .spawn({
                let events = events.clone();
                let last_error = Arc::clone(&last_error);
                move || run(control, events, event_rx, ready_tx, last_error)
            })
            .map_err(|error| MlError::Io(format!("spawn MPRIS thread: {error}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                events,
                last_error,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(MlError::Io("MPRIS thread exited before registering".into()))
            }
        }
    }

    /// Publish a new snapshot. Cheap, non-blocking, safe from any thread.
    ///
    /// The service thread diffs it, so only properties that actually changed
    /// reach the bus. Snapshot-identical updates cost one clone and nothing else.
    pub fn update(&self, snapshot: &PlayerSnapshot) {
        let _ = self.events.send(Event::Update(Box::new(snapshot.clone())));
    }

    /// Publish at most once per `interval`, for position-tick callers.
    ///
    /// Position changes are intentionally *not* published as
    /// `PropertiesChanged` (clients poll `Position`), so a UI ticking every
    /// 100 ms gains nothing by forwarding each tick. `interval == Duration::ZERO`
    /// forwards everything. Returns whether the snapshot was forwarded.
    pub fn update_throttled(
        &self,
        snapshot: &PlayerSnapshot,
        throttler: &mut UpdateThrottle,
        now: std::time::Instant,
    ) -> bool {
        if !throttler.should_send(now) {
            return false;
        }
        throttler.mark_sent(now);
        self.update(snapshot);
        true
    }

    /// `false` once the service thread has stopped (bus connection lost).
    pub fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .map(|thread| !thread.is_finished())
            .unwrap_or(false)
    }

    /// Last D-Bus error hit while publishing, if any.
    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl std::fmt::Debug for MprisService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MprisService")
            .field("bus_name", &MPRIS_BUS_NAME)
            .field("running", &self.is_running())
            .finish()
    }
}

impl Drop for MprisService {
    fn drop(&mut self) {
        let _ = self.events.send(Event::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    control: Arc<dyn PlayerControl>,
    events: tokio::sync::mpsc::UnboundedSender<Event>,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
    ready: crossbeam_channel::Sender<MlResult<()>>,
    last_error: Arc<Mutex<Option<String>>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(MlError::Io(format!("MPRIS runtime: {error}"))));
            return;
        }
    };

    runtime.block_on(async move {
        let imp = MprisImp {
            control,
            last: Mutex::new(None),
            events: events.clone(),
        };
        let server = match Server::new(MPRIS_BUS_SUFFIX, imp).await {
            Ok(server) => server,
            Err(error) => {
                let _ = ready.send(Err(MlError::Io(format!(
                    "register {MPRIS_BUS_NAME}: {error}"
                ))));
                return;
            }
        };
        let _ = ready.send(Ok(()));

        while let Some(event) = event_rx.recv().await {
            let result = match event {
                Event::Update(snapshot) => publish(&server, *snapshot).await,
                Event::Seeked(position) => server.emit(Signal::Seeked { position }).await,
                Event::Quit => break,
            };
            if let Err(error) = result {
                // Keep the service alive: a transient signal failure must not
                // kill media-key handling, but it must stay observable.
                let mut slot = last_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *slot = Some(error.to_string());
            }
        }
        // Release the bus name before the thread exits.
        let _ = server.release_bus_name().await;
    });
}

async fn publish(
    server: &Server<MprisImp>,
    next: PlayerSnapshot,
) -> mpris_server::zbus::Result<()> {
    let imp = server.imp();
    let published = Published::new(next.clone(), imp.control.current_art_url());
    let changes = {
        let mut last = imp
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changes = snapshot_changes(last.as_ref(), &published);
        *last = Some(published.clone());
        changes
    };
    if changes.is_empty() {
        return Ok(());
    }

    let track = next
        .current
        .as_ref()
        .map(|song| track_metadata(song, published.art_url.as_deref()));

    let properties = changes.into_iter().map(|change| match change {
        Change::PlaybackStatus => {
            Property::PlaybackStatus(proto_status(transport_status(next.status)))
        }
        Change::LoopStatus => Property::LoopStatus(proto_loop(transport_loop(next.repeat))),
        Change::Shuffle => Property::Shuffle(next.shuffle),
        Change::Volume => Property::Volume(volume_of(&next)),
        Change::Metadata => match &track {
            Some(track) => Property::Metadata(metadata_from(track)),
            None => Property::Metadata(Metadata::builder().trackid(TrackId::NO_TRACK).build()),
        },
        Change::CanGoNext => Property::CanGoNext(can_go_next(&next)),
        Change::CanGoPrevious => Property::CanGoPrevious(can_go_previous(&next)),
    });
    server.properties_changed(properties).await
}

/// Rate limiter for [`MprisService::update_throttled`].
///
/// Deliberately tiny: one interval plus the last send time. Position never
/// flows through `PropertiesChanged`, so a dropped tick cannot make a client
/// stale — it only saves work.
#[derive(Debug, Clone)]
pub struct UpdateThrottle {
    interval: std::time::Duration,
    last_sent: Option<std::time::Instant>,
}

impl UpdateThrottle {
    pub fn new(interval: std::time::Duration) -> Self {
        Self {
            interval,
            last_sent: None,
        }
    }

    /// A sensible default for a 10 Hz UI: forward at most every 200 ms.
    pub fn for_ui() -> Self {
        Self::new(std::time::Duration::from_millis(200))
    }

    pub fn interval(&self) -> std::time::Duration {
        self.interval
    }

    pub fn should_send(&self, now: std::time::Instant) -> bool {
        match self.last_sent {
            None => true,
            Some(last) => now.saturating_duration_since(last) >= self.interval,
        }
    }

    pub fn mark_sent(&mut self, now: std::time::Instant) {
        self.last_sent = Some(now);
    }
}

/// The bus name clients should query.
pub fn mpris_bus_name() -> &'static str {
    MPRIS_BUS_NAME
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn song() -> Song {
        Song {
            id: "dQw4w9WgXcQ".into(),
            title: "Never Gonna Give You Up".into(),
            artists: vec![ml_core::Artist::new("Rick Astley", None)],
            duration: Some(213),
            ..Song::default()
        }
    }

    #[test]
    fn throttle_forwards_the_first_update_then_respects_the_interval() {
        let mut throttle = UpdateThrottle::new(Duration::from_millis(200));
        let start = Instant::now();
        assert!(throttle.should_send(start));
        throttle.mark_sent(start);
        assert!(!throttle.should_send(start));
        assert!(!throttle.should_send(start + Duration::from_millis(199)));
        assert!(throttle.should_send(start + Duration::from_millis(200)));
        throttle.mark_sent(start + Duration::from_millis(200));
        assert!(!throttle.should_send(start + Duration::from_millis(201)));
    }

    #[test]
    fn zero_interval_forwards_everything() {
        let mut throttle = UpdateThrottle::new(Duration::ZERO);
        let now = Instant::now();
        assert!(throttle.should_send(now));
        throttle.mark_sent(now);
        assert!(throttle.should_send(now));
        assert_eq!(
            UpdateThrottle::for_ui().interval(),
            Duration::from_millis(200)
        );
    }

    #[test]
    fn snapshot_control_forwards_every_command() {
        let (control, rx) = SnapshotControl::new();
        control.play().unwrap();
        control.pause().unwrap();
        control.play_pause().unwrap();
        control.stop().unwrap();
        control.next().unwrap();
        control.previous().unwrap();
        control.seek_by_ms(-1500).unwrap();
        control.set_position_ms(90_000).unwrap();
        control.set_volume(0.25).unwrap();
        control.set_shuffle(true).unwrap();
        control.set_repeat(RepeatMode::One).unwrap();
        control.raise().unwrap();
        control.quit().unwrap();

        let expected = vec![
            PlayerCommand::Play,
            PlayerCommand::Pause,
            PlayerCommand::PlayPause,
            PlayerCommand::Stop,
            PlayerCommand::Next,
            PlayerCommand::Previous,
            PlayerCommand::SeekBy(-1500),
            PlayerCommand::SetPosition(90_000),
            PlayerCommand::SetVolume(0.25),
            PlayerCommand::SetShuffle(true),
            PlayerCommand::SetRepeat(RepeatMode::One),
            PlayerCommand::Raise,
            PlayerCommand::Quit,
        ];
        let got: Vec<PlayerCommand> = rx.try_iter().collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn snapshot_control_reads_published_state() {
        let (control, _rx) = SnapshotControl::new();
        let snapshot = PlayerSnapshot {
            status: ml_core::PlaybackStatus::Playing,
            position_ms: 42_000,
            ..PlayerSnapshot::default()
        };
        control.set_state(snapshot.clone(), Some(song()), Some("file:///a.png".into()));
        assert_eq!(control.state(), snapshot);
        assert_eq!(
            control.current_song().map(|s| s.id),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(control.current_art_url().as_deref(), Some("file:///a.png"));

        // Publishing again replaces both halves.
        control.set_state(PlayerSnapshot::default(), None, None);
        assert!(control.current_song().is_none());
        assert_eq!(control.current_art_url(), None);
    }

    #[test]
    fn snapshot_control_open_uri_requires_a_handler() {
        let (control, _rx) = SnapshotControl::new();
        assert!(control.open_uri("metrolist://album/x").is_err());

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        control.set_uri_handler(move |uri| sink.lock().unwrap().push(uri.to_owned()));
        control.open_uri("metrolist://album/x").unwrap();
        assert_eq!(*seen.lock().unwrap(), vec!["metrolist://album/x"]);
    }

    /// The adapter must satisfy the same trait the D-Bus layer consumes.
    #[test]
    fn snapshot_control_is_a_player_control() {
        let (control, _rx) = SnapshotControl::new();
        control.set_state(
            PlayerSnapshot {
                status: ml_core::PlaybackStatus::Playing,
                ..PlayerSnapshot::default()
            },
            Some(song()),
            None,
        );
        let control: Arc<dyn PlayerControl> = control;
        assert_eq!(
            transport_status(control.state().status),
            TransportStatus::Playing
        );
    }

    #[test]
    fn metadata_mapping_uses_the_control_art_override() {
        let (control, _rx) = SnapshotControl::new();
        control.set_state(
            PlayerSnapshot::default(),
            Some(song()),
            Some("file:///c.png".into()),
        );
        let track = track_metadata(
            &control.current_song().unwrap(),
            control.current_art_url().as_deref(),
        );
        assert_eq!(track.art_url.as_deref(), Some("file:///c.png"));
        assert_eq!(track.track_id, track_path("dQw4w9WgXcQ"));
        assert_eq!(track.length_us, 213_000_000);
    }
}
