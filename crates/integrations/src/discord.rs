//! Discord Rich Presence over the local Discord IPC socket.
//!
//! Feature-gated (`discord`, off by default). No OAuth, no HTTP: it speaks the
//! same protocol the official client uses locally — connect to
//! `$XDG_RUNTIME_DIR/discord-ipc-0`, handshake `{op:0, v:1, client_id}`, then
//! `SET_ACTIVITY` (`op:1`) with the current track.
//!
//! The socket may not exist (Discord closed, Flatpak nesting, other runtime
//! dir); every failure is reported and the caller keeps running. Reconnection is
//! attempted on the next update after the socket drops.
//!
//! ## Verification scope
//!
//! Socket framing and JSON are I/O; the IPC path list and the activity JSON
//! shape are unit-tested. Not a Verus candidate.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ml_core::{MlResult, PlayerSnapshot};
use serde_json::{json, Value};

/// Default Discord application id: the upstream Metrolist client id
/// (`DISCORD_APP_ID` in the Android `app/build.gradle.kts`). Not a secret —
/// local IPC presence takes no OAuth. Override with [`DiscordPresence::with_app_id`].
pub const CLIENT_ID: &str = "1447278780795064401";
/// First reconnect delay; doubled per failed attempt.
const BACKOFF_START: Duration = Duration::from_secs(1);
/// Reconnect delay ceiling.
const BACKOFF_MAX: Duration = Duration::from_secs(60);

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

/// Candidate IPC sockets, in the order Discord tries them.
pub fn candidate_sockets() -> Vec<PathBuf> {
    sockets_in(
        &std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir),
    )
}

/// The `discord-ipc-0..9` names under one directory.
pub fn sockets_in(base: &std::path::Path) -> Vec<PathBuf> {
    (0..10)
        .map(|index| base.join(format!("discord-ipc-{index}")))
        .collect()
}

fn encode(op: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string();
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body.as_bytes());
    frame
}

fn read_frame(stream: &mut UnixStream) -> Option<(u32, Value)> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).ok()?;
    let op = u32::from_le_bytes(header[0..4].try_into().ok()?);
    let len = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).ok()?;
    let value = serde_json::from_slice(&body).ok()?;
    Some((op, value))
}

/// A live Discord IPC connection.
pub struct DiscordPresence {
    stream: Option<UnixStream>,
    /// Timestamp-free identity of the last activity sent, to skip duplicate
    /// frames (the embedded timestamps move every millisecond).
    activity: Option<String>,
    enabled: bool,
    app_id: String,
    /// Earliest instant the next reconnect may be attempted.
    retry_at: Option<Instant>,
    consecutive_failures: u32,
    /// Last connection error, surfaced to the settings UI.
    last_error: Option<String>,
}

impl std::fmt::Debug for DiscordPresence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordPresence")
            .field("connected", &self.stream.is_some())
            .field("enabled", &self.enabled)
            .field("app_id", &self.app_id)
            .field(
                "retry_in",
                &self
                    .retry_at
                    .map(|at| at.saturating_duration_since(Instant::now())),
            )
            .finish()
    }
}

impl DiscordPresence {
    pub fn new() -> Self {
        Self::with_app_id(CLIENT_ID)
    }

    /// Use a different Discord application id (settings override).
    pub fn with_app_id(app_id: impl Into<String>) -> Self {
        Self {
            stream: None,
            activity: None,
            enabled: true,
            app_id: app_id.into(),
            retry_at: None,
            consecutive_failures: 0,
            last_error: None,
        }
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.clear();
        }
    }

    /// Last connection error, if Discord is unreachable.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Drop the connection so the next update reconnects.
    pub fn disconnect(&mut self) {
        self.stream = None;
        self.activity = None;
    }

    /// Connect + handshake. Returns the socket used.
    ///
    /// Honours the reconnect backoff: while `retry_at` is in the future this
    /// reports Unavailable without touching any socket.
    pub fn connect(&mut self) -> MlResult<PathBuf> {
        let now = Instant::now();
        if let Some(retry_at) = self.retry_at {
            if now < retry_at {
                return Err(ml_core::MlError::Unavailable(format!(
                    "discord ipc: retrying in {:?}",
                    retry_at.saturating_duration_since(now)
                )));
            }
        }
        match self.connect_inner() {
            Ok(path) => {
                self.retry_at = None;
                self.consecutive_failures = 0;
                self.last_error = None;
                Ok(path)
            }
            Err(error) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                let delay = backoff_for(self.consecutive_failures);
                self.retry_at = Some(Instant::now() + delay);
                self.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Connect using explicit socket candidates (tests point at a temp dir).
    pub fn connect_to(&mut self, sockets: &[PathBuf]) -> MlResult<PathBuf> {
        let now = Instant::now();
        if let Some(retry_at) = self.retry_at {
            if now < retry_at {
                return Err(ml_core::MlError::Unavailable(
                    "discord ipc: backing off".into(),
                ));
            }
        }
        match self.connect_paths(sockets) {
            Ok(path) => {
                self.retry_at = None;
                self.consecutive_failures = 0;
                self.last_error = None;
                Ok(path)
            }
            Err(error) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                self.retry_at = Some(Instant::now() + backoff_for(self.consecutive_failures));
                self.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn connect_inner(&mut self) -> MlResult<PathBuf> {
        self.connect_paths(&candidate_sockets())
    }

    fn connect_paths(&mut self, candidates: &[PathBuf]) -> MlResult<PathBuf> {
        let mut last_error = String::from("no discord-ipc socket found");
        for path in candidates {
            if !path.exists() {
                continue;
            }
            match UnixStream::connect(path) {
                Ok(mut stream) => {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(500)))
                        .ok();
                    let handshake = json!({ "v": 1, "client_id": self.app_id });
                    if let Err(error) = stream.write_all(&encode(OP_HANDSHAKE, &handshake)) {
                        last_error = format!("handshake write {}: {error}", path.display());
                        continue;
                    }
                    match read_frame(&mut stream) {
                        Some((_, value)) if is_ready(&value) => {
                            self.stream = Some(stream);
                            self.activity = None;
                            return Ok(path.clone());
                        }
                        Some((op, value)) => {
                            last_error = format!("handshake op {op}: {value}");
                        }
                        None => {
                            last_error = format!("handshake reply from {}", path.display());
                        }
                    }
                }
                Err(error) => last_error = format!("connect {}: {error}", path.display()),
            }
        }
        Err(ml_core::MlError::Unavailable(format!(
            "discord ipc: {last_error}"
        )))
    }

    fn send(&mut self, op: u32, payload: &Value) -> MlResult<()> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| ml_core::MlError::Unavailable("discord ipc not connected".into()))?;
        stream
            .write_all(&encode(op, payload))
            .map_err(|error| ml_core::MlError::Io(format!("discord write: {error}")))
    }

    /// Push the current track as the Discord activity.
    ///
    /// A paused or stopped player clears the presence instead of showing a
    /// frozen timestamp; Discord being down is not an error for the player.
    pub fn on_snapshot(&mut self, snapshot: &PlayerSnapshot) -> MlResult<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.stream.is_none() {
            // Discord may have been closed since the last update; the backoff
            // keeps a closed client from costing a connect per snapshot.
            if self.connect().is_err() {
                return Ok(());
            }
        }
        if !snapshot.is_playing() {
            self.clear();
            return Ok(());
        }
        let activity = build_activity(snapshot);
        let identity = activity_identity(&activity);
        if self.activity.as_deref() == Some(identity.as_str()) {
            return Ok(());
        }
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity },
            "nonce": format!("{}-{}", std::process::id(), now_nonce()),
        });
        match self.send(OP_FRAME, &payload) {
            Ok(()) => {
                self.activity = Some(identity);
                Ok(())
            }
            Err(error) => {
                self.disconnect();
                self.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Clear the presence (paused or no track).
    pub fn clear(&mut self) {
        if self.stream.is_none() || self.activity.is_none() {
            self.activity = None;
            return;
        }
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": null },
            "nonce": format!("{}-{}", std::process::id(), now_nonce()),
        });
        let _ = self.send(OP_FRAME, &payload);
        self.activity = None;
    }

    /// Answer a pending PING so Discord keeps the connection open.
    pub fn pump(&mut self) {
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        while let Some((op, _)) = read_frame(stream) {
            if op == OP_PING {
                let _ = stream.write_all(&encode(OP_PONG, &json!({})));
            } else if op == OP_CLOSE {
                self.stream = None;
                return;
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }
}

impl Default for DiscordPresence {
    fn default() -> Self {
        Self::new()
    }
}

/// Discord's handshake succeeds with `DISPATCH`/`READY` (older clients use
/// `CONNECTED`); anything else means the app id was rejected or the frame is
/// not a handshake at all.
fn is_ready(value: &Value) -> bool {
    let field = |name: &str| value.get(name).and_then(Value::as_str);
    field("cmd") == Some("DISPATCH") && matches!(field("evt"), Some("READY" | "CONNECTED"))
}

/// Exponential backoff with a ceiling: 1 s, 2 s, 4 s, ... capped at 60 s.
pub fn backoff_for(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(6);
    BACKOFF_START.saturating_mul(1u32 << shift).min(BACKOFF_MAX)
}

fn now_nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

/// Activity JSON for a snapshot: title/artist state, elapsed/end timestamps
/// while playing, and the YouTube Music deep link.
pub fn build_activity(snapshot: &PlayerSnapshot) -> Value {
    let Some(song) = snapshot.current.as_ref() else {
        return Value::Null;
    };
    let now_ms = now_epoch_ms();
    let mut activity = json!({
        "details": song.title,
        "state": if song.artists.is_empty() {
            "YouTube Music".to_string()
        } else {
            song.artist_names()
        },
        "details_url": song.share_url(),
        "assets": {
            "large_image": song.thumb(512),
            "large_text": song.album.as_ref().map(|album| album.title.clone()).unwrap_or_default(),
        },
    });
    let duration_ms = u64::from(song.duration.unwrap_or(0)).saturating_mul(1000);
    if snapshot.is_playing() && duration_ms > 0 {
        let elapsed = now_ms.saturating_sub(snapshot.position_ms);
        activity["timestamps"] =
            json!({ "start": elapsed, "end": elapsed.saturating_add(duration_ms) });
    }
    activity
}

/// Dedup identity for an activity: every field except the elapsed/end
/// timestamps, which move every millisecond. Anything else (including fields
/// added later) is part of the identity, so only real presence changes resend.
fn activity_identity(activity: &Value) -> String {
    let mut identity = activity.clone();
    if let Some(object) = identity.as_object_mut() {
        object.remove("timestamps");
    }
    identity.to_string()
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Kept for tests: the frame header layout.
pub fn frame_header(op: u32, len: u32) -> [u8; 8] {
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&op.to_le_bytes());
    header[4..8].copy_from_slice(&len.to_le_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::*;
    use ml_core::{Artist, PlaybackStatus, Song};
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn frames_are_little_endian_opcodes_plus_json() {
        let frame = encode(OP_HANDSHAKE, &json!({ "v": 1 }));
        assert_eq!(&frame[0..4], &0u32.to_le_bytes());
        let len = u32::from_le_bytes(frame[4..8].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len() - 8);
        assert_eq!(&frame[8..], b"{\"v\":1}");
        assert_eq!(frame_header(1, 7), [1, 0, 0, 0, 7, 0, 0, 0]);
    }

    #[test]
    fn sockets_are_numbered_candidates_in_the_runtime_dir() {
        let sockets = candidate_sockets();
        assert_eq!(sockets.len(), 10);
        assert!(sockets[0].ends_with("discord-ipc-0"));
        assert!(sockets[9].ends_with("discord-ipc-9"));
    }

    #[test]
    fn activity_carries_track_and_timestamps_only_while_playing() {
        use ml_core::{Artist, PlaybackStatus, Song};
        let song = Song {
            id: "dQw4w9WgXcQ".into(),
            title: "Never Gonna Give You Up".into(),
            artists: vec![Artist::new("Rick Astley", None)],
            duration: Some(213),
            ..Song::default()
        };
        let playing = PlayerSnapshot {
            status: PlaybackStatus::Playing,
            position_ms: 1_000,
            current: Some(song.clone()),
            ..PlayerSnapshot::default()
        };
        let activity = build_activity(&playing);
        assert_eq!(activity["details"], "Never Gonna Give You Up");
        assert_eq!(activity["state"], "Rick Astley");
        assert_eq!(
            activity["details_url"],
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        let start = activity["timestamps"]["start"].as_u64().unwrap();
        let end = activity["timestamps"]["end"].as_u64().unwrap();
        assert_eq!(end - start, 213_000);

        let paused = PlayerSnapshot {
            status: PlaybackStatus::Paused,
            ..playing
        };
        assert!(build_activity(&paused)["timestamps"].is_null());
        assert_eq!(build_activity(&PlayerSnapshot::default()), Value::Null);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff_for(0), Duration::from_secs(1));
        assert_eq!(backoff_for(1), Duration::from_secs(1));
        assert_eq!(backoff_for(2), Duration::from_secs(2));
        assert_eq!(backoff_for(3), Duration::from_secs(4));
        assert_eq!(backoff_for(4), Duration::from_secs(8));
        assert_eq!(backoff_for(7), Duration::from_secs(60));
        assert_eq!(backoff_for(u32::MAX), Duration::from_secs(60));
    }

    #[test]
    fn app_id_is_configurable() {
        assert_eq!(DiscordPresence::new().app_id(), CLIENT_ID);
        assert_eq!(DiscordPresence::with_app_id("123").app_id(), "123");
    }

    /// A track with album, artist and duration for activity tests.
    fn song() -> Song {
        Song {
            id: "dQw4w9WgXcQ".into(),
            title: "Never Gonna Give You Up".into(),
            artists: vec![Artist::new("Rick Astley", None)],
            duration: Some(213),
            album: Some(ml_core::AlbumRef {
                id: "al".into(),
                title: "Whenever You Need Somebody".into(),
            }),
            ..Song::default()
        }
    }

    // --- fake Discord IPC server -----------------------------------------

    /// Accepts one client, replies to the handshake, and records everything.
    struct FakeDiscord {
        dir: PathBuf,
        received: Arc<Mutex<Vec<(u32, Value)>>>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl FakeDiscord {
        fn start(handshake_reply: Value) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "ml-discord-{}-{}",
                std::process::id(),
                now_nonce()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("discord-ipc-0");
            let listener = UnixListener::bind(&path).unwrap();
            let received = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&received);
            let handle = std::thread::spawn(move || {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                // First frame: handshake.
                if let Some(frame) = read_frame(&mut stream) {
                    sink.lock().unwrap().push(frame);
                }
                let reply = encode(OP_FRAME, &handshake_reply);
                let _ = stream.write_all(&reply);
                while let Some(frame) = read_frame(&mut stream) {
                    let close = frame.0 == OP_CLOSE;
                    sink.lock().unwrap().push(frame);
                    if close {
                        break;
                    }
                }
            });
            Self {
                dir,
                received,
                handle: Some(handle),
            }
        }

        fn socket(&self) -> PathBuf {
            self.dir.join("discord-ipc-0")
        }

        fn frames(&self) -> Vec<(u32, Value)> {
            self.received.lock().unwrap().clone()
        }

        fn finish(mut self) -> Vec<(u32, Value)> {
            let _ = self.socket();
            // Wake the reader so it exits after the client disconnects.
            let _ = UnixStream::connect(self.socket());
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
            let frames = self.frames();
            let _ = std::fs::remove_dir_all(&self.dir);
            frames
        }
    }

    fn dispatch_ready() -> Value {
        json!({ "cmd": "DISPATCH", "evt": "READY", "data": { "v": 1 } })
    }

    #[test]
    fn handshake_uses_the_configured_app_id_and_connects() {
        let server = FakeDiscord::start(dispatch_ready());
        let sockets = vec![server.socket()];
        let mut presence = DiscordPresence::with_app_id("999888777");
        let path = presence.connect_to(&sockets).expect("handshake");
        assert_eq!(path, server.socket());
        assert!(presence.is_connected());
        assert!(presence.last_error().is_none());

        presence.disconnect();
        drop(presence);
        let frames = server.finish();
        assert_eq!(frames[0].0, OP_HANDSHAKE);
        assert_eq!(frames[0].1["v"], 1);
        assert_eq!(frames[0].1["client_id"], "999888777");
    }

    #[test]
    fn set_activity_is_sent_and_cleared_on_pause() {
        let server = FakeDiscord::start(dispatch_ready());
        let sockets = vec![server.socket()];
        let mut presence = DiscordPresence::with_app_id("1");
        presence.connect_to(&sockets).expect("handshake");

        let playing = PlayerSnapshot {
            status: PlaybackStatus::Playing,
            position_ms: 5_000,
            current: Some(song()),
            ..PlayerSnapshot::default()
        };
        presence.on_snapshot(&playing).expect("set activity");
        // The same snapshot must not re-send (diffing).
        presence.on_snapshot(&playing).expect("no duplicate");

        let paused = PlayerSnapshot {
            status: PlaybackStatus::Paused,
            ..playing
        };
        presence.on_snapshot(&paused).expect("clear on pause");
        assert!(presence.activity.is_none());

        presence.disconnect();
        drop(presence);
        let frames = server.finish();
        let activity_frames: Vec<&(u32, Value)> = frames
            .iter()
            .filter(|(_, value)| value["cmd"] == "SET_ACTIVITY")
            .collect();
        assert_eq!(activity_frames.len(), 2, "one set, one clear: {frames:?}");
        assert_eq!(activity_frames[0].0, OP_FRAME);
        assert_eq!(
            activity_frames[0].1["args"]["activity"]["details"],
            "Never Gonna Give You Up"
        );
        assert_eq!(activity_frames[1].1["args"]["activity"], Value::Null);
        assert!(activity_frames[0].1["args"]["pid"].as_u64().unwrap() > 0);
    }

    #[test]
    fn dedup_ignores_timestamp_drift_but_sends_a_track_change() {
        let server = FakeDiscord::start(dispatch_ready());
        let mut presence = DiscordPresence::with_app_id("1");
        presence
            .connect_to(std::slice::from_ref(&server.socket()))
            .expect("handshake");

        let playing = PlayerSnapshot {
            status: PlaybackStatus::Playing,
            position_ms: 5_000,
            current: Some(song()),
            ..PlayerSnapshot::default()
        };
        presence.on_snapshot(&playing).expect("set activity");

        // Same track and status after a guaranteed millisecond boundary: the
        // elapsed/end timestamps differ, the presence identity does not.
        let first_ms = now_epoch_ms();
        let deadline = Instant::now() + Duration::from_secs(2);
        while now_epoch_ms() < first_ms + 2 && Instant::now() < deadline {
            std::hint::spin_loop();
        }
        presence.on_snapshot(&playing).expect("no duplicate");

        let mut next = song();
        next.id = "other".into();
        next.title = "Another Track".into();
        let changed = PlayerSnapshot {
            status: PlaybackStatus::Playing,
            position_ms: 5_000,
            current: Some(next),
            ..PlayerSnapshot::default()
        };
        presence.on_snapshot(&changed).expect("track change");

        presence.disconnect();
        drop(presence);
        let frames = server.finish();
        let activities: Vec<&Value> = frames
            .iter()
            .filter(|(_, value)| value["cmd"] == "SET_ACTIVITY")
            .map(|(_, value)| &value["args"]["activity"])
            .collect();
        assert_eq!(
            activities.len(),
            2,
            "one set per track, no timestamp duplicate: {frames:?}"
        );
        assert_eq!(activities[0]["details"], "Never Gonna Give You Up");
        assert!(activities[0]["timestamps"]["start"].as_u64().is_some());
        assert_eq!(activities[1]["details"], "Another Track");
    }

    #[test]
    fn a_missing_socket_is_reported_and_backed_off() {
        let dir = std::env::temp_dir().join(format!("ml-discord-none-{}", std::process::id()));
        let sockets = vec![dir.join("discord-ipc-0")];
        let mut presence = DiscordPresence::new();
        let error = presence.connect_to(&sockets).unwrap_err();
        assert!(error.to_string().contains("discord ipc"), "{error}");
        assert!(presence.last_error().is_some());
        // Second attempt within the backoff window is refused without I/O.
        let error = presence.connect_to(&sockets).unwrap_err();
        assert!(error.to_string().contains("backing off"), "{error}");
        // and `on_snapshot` treats it as "no presence", not a player failure.
        assert!(presence
            .on_snapshot(&PlayerSnapshot {
                status: PlaybackStatus::Playing,
                current: Some(song()),
                ..PlayerSnapshot::default()
            })
            .is_ok());
    }

    #[test]
    fn a_rejected_handshake_does_not_connect() {
        let server = FakeDiscord::start(json!({ "cmd": "DISPATCH", "evt": "OTHER" }));
        let sockets = vec![server.socket()];
        let mut presence = DiscordPresence::new();
        assert!(presence.connect_to(&sockets).is_err());
        assert!(!presence.is_connected());
        presence.disconnect();
        drop(presence);
        let _ = server.finish();
    }

    #[test]
    fn ping_is_answered_with_pong() {
        let listener_dir = std::env::temp_dir().join(format!("ml-discord-ping-{}", now_nonce()));
        std::fs::create_dir_all(&listener_dir).unwrap();
        let path = listener_dir.join("discord-ipc-0");
        let listener = UnixListener::bind(&path).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_frame(&mut stream);
            let _ = stream.write_all(&encode(OP_FRAME, &dispatch_ready()));
            // Send a PING, then read the PONG.
            let _ = stream.write_all(&encode(OP_PING, &json!({})));
            if let Some(frame) = read_frame(&mut stream) {
                sink.lock().unwrap().push(frame);
            }
            // Leave the client's pump loop to time out.
            std::thread::sleep(Duration::from_millis(700));
        });
        let mut presence = DiscordPresence::new();
        presence
            .connect_to(std::slice::from_ref(&path))
            .expect("handshake");
        presence.pump();
        drop(presence);
        let _ = handle.join();
        let frames = received.lock().unwrap().clone();
        assert_eq!(frames[0].0, OP_PONG, "ping must be answered: {frames:?}");
        let _ = std::fs::remove_dir_all(&listener_dir);
    }
}
