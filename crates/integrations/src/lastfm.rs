//! Last.fm scrobbling.
//!
//! Credentials come from the caller ([`LastFmConfig`]): API key, optional
//! shared secret, and a session key obtained out-of-band. Nothing is hardcoded.
//! Only [`LastFmConfig::session`] is required for the scrobble/write methods;
//! the secret is needed just by [`LastFmClient::authenticate`] (`auth.getMobileSession`).
//!
//! Layers, from pure to I/O:
//!
//! - [`signature`], [`scrobble_params`], [`now_playing_params`], [`parse_session`],
//!   [`classify_error`], [`ScrobbleQueue`] — no I/O, fully unit-tested (this is
//!   the Verus surface).
//! - [`LastFmTransport`] — the HTTP boundary; [`HttpTransport`] is the real
//!   `reqwest` client, tests supply a fake.
//! - [`LastFmClient`] — one signed call per method.
//! - [`Scrobbler`] — player hooks (`started`/`paused`/`resumed`/`ended`/`seeked`),
//!   the scrobble timer rules from the upstream Android `ScrobbleManager`, the
//!   deduped offline queue and its retry/flush policy.
//!
//! ## Verus scope
//!
//! The pure functions above take plain owned values, never panic, and have no
//! floats or locks. The timer logic in [`Scrobbler`] is time-dependent and
//! lives outside the proof surface; the threshold *rule* it applies is pure
//! ([`scrobble_threshold_ms`], [`should_scrobble`]).

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use ml_core::{MlError, MlResult, PlaybackStatus, PlayerSnapshot, Song};

/// Last.fm API endpoint.
pub const API_ROOT: &str = "https://ws.audioscrobbler.com/2.0/";
/// User agent Last.fm expects from clients.
pub const USER_AGENT: &str = "Metrolist (https://github.com/MetrolistGroup/Metrolist)";
/// Upstream `LastFM.DEFAULT_SCROBBLE_MIN_SONG_DURATION` (seconds).
pub const DEFAULT_MIN_SONG_DURATION_S: u32 = 30;
/// Upstream `LastFM.DEFAULT_SCROBBLE_DELAY_PERCENT`.
pub const DEFAULT_DELAY_PERCENT: f32 = 0.5;
/// Upstream `ScrobbleManager` default cap on the wait (seconds).
pub const DEFAULT_DELAY_SECONDS: u32 = 50;
/// Last.fm accepts at most 50 scrobbles per `track.scrobble` call.
pub const MAX_BATCH: usize = 50;

/// Credentials and behaviour knobs, supplied by settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastFmConfig {
    pub api_key: String,
    /// Only needed for [`LastFmClient::authenticate`].
    pub secret: Option<String>,
    /// Session key from a previous `auth.getMobileSession`.
    pub session: Option<String>,
}

impl LastFmConfig {
    pub fn new(
        api_key: impl Into<String>,
        secret: Option<String>,
        session: Option<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            secret,
            session,
        }
    }

    /// True when scrobbling can run: a key and a session.
    pub fn can_scrobble(&self) -> bool {
        !self.api_key.is_empty() && self.session.as_deref().is_some_and(|s| !s.is_empty())
    }

    /// True when [`LastFmClient::authenticate`] has what it needs.
    pub fn can_authenticate(&self) -> bool {
        !self.api_key.is_empty() && self.secret.as_deref().is_some_and(|s| !s.is_empty())
    }
}

/// Scrobble policy, mirroring the upstream defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrobblePolicy {
    /// Tracks shorter than this are never scrobbled (Last.fm's own rule is 30 s).
    pub min_duration_s: u32,
    /// Fraction of the duration to wait; `500` = 50% (integer permille, no float
    /// in the pure path).
    pub delay_permille: u32,
    /// Cap on the wait, in seconds.
    pub delay_cap_s: u32,
}

impl Default for ScrobblePolicy {
    fn default() -> Self {
        Self {
            min_duration_s: DEFAULT_MIN_SONG_DURATION_S,
            delay_permille: (DEFAULT_DELAY_PERCENT * 1000.0) as u32,
            delay_cap_s: DEFAULT_DELAY_SECONDS,
        }
    }
}

/// How long a track must play before it may be scrobbled, in milliseconds.
///
/// `min(half the track, cap)`, and `0` for tracks at or under
/// `min_duration_s` — the caller treats `0` as "never": Last.fm ignores
/// scrobbles of sub-30-second tracks, and the cap keeps long tracks quick.
pub fn scrobble_threshold_ms(policy: &ScrobblePolicy, duration_s: u32) -> u64 {
    if duration_s <= policy.min_duration_s {
        return 0;
    }
    let fraction = u64::from(duration_s) * 1000 * u64::from(policy.delay_permille) / 1000;
    fraction.min(u64::from(policy.delay_cap_s) * 1000)
}

/// Whether `position_ms` of a `duration_s` track is enough to scrobble it.
pub fn should_scrobble(policy: &ScrobblePolicy, duration_s: u32, position_ms: u64) -> bool {
    let threshold = scrobble_threshold_ms(policy, duration_s);
    threshold > 0 && position_ms >= threshold
}

/// Last.fm API signature: md5 over `key + value` for every parameter sorted by
/// key, then the shared secret. Values are used exactly as sent (UTF-8, no
/// escaping); `api_sig`/`format`/`callback` are excluded by the caller.
pub fn signature(params: &BTreeMap<String, String>, secret: &str) -> String {
    let mut buffer = String::with_capacity(64);
    for (key, value) in params {
        buffer.push_str(key);
        buffer.push_str(value);
    }
    buffer.push_str(secret);
    md5_hex(buffer.as_bytes())
}

fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(32);
    for byte in digest {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// One track ready to be scrobbled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scrobble {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub duration_s: Option<u32>,
    /// Unix seconds; Last.fm requires the *start* time of the play.
    pub timestamp: u64,
    /// Stable identity for deduplication: `<song id>@<timestamp>`.
    pub key: String,
}

impl Scrobble {
    /// Build from a song plus the play's start time, deduping on id + start.
    pub fn from_song(song: &Song, timestamp: u64) -> Option<Self> {
        let artist = song.artist_names();
        if song.title.is_empty() || artist.is_empty() {
            return None;
        }
        Some(Self {
            artist,
            track: song.title.clone(),
            album: song
                .album
                .as_ref()
                .map(|album| album.title.clone())
                .filter(|title| !title.is_empty()),
            duration_s: song.duration,
            timestamp,
            key: scrobble_key(&song.id, timestamp),
        })
    }
}

/// Dedup key: song id plus play start, so the same song played twice (different
/// start times) is two scrobbles, but a retry of one play is one.
pub fn scrobble_key(song_id: &str, timestamp: u64) -> String {
    format!("{song_id}@{timestamp}")
}

/// Scrobble parameters for slot `index` of a batch (`artist[0]`, ...).
///
/// Keys are exactly what Last.fm expects; `api_sig`, `format` and the session
/// key are added by [`LastFmClient`].
pub fn scrobble_params(scrobble: &Scrobble, index: usize) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    params.insert(format!("artist[{index}]"), scrobble.artist.clone());
    params.insert(format!("track[{index}]"), scrobble.track.clone());
    params.insert(
        format!("timestamp[{index}]"),
        scrobble.timestamp.to_string(),
    );
    if let Some(album) = &scrobble.album {
        params.insert(format!("album[{index}]"), album.clone());
    }
    if let Some(duration) = scrobble.duration_s {
        params.insert(format!("duration[{index}]"), duration.to_string());
    }
    params
}

/// `track.updateNowPlaying` parameters (no index: it is a single-track call).
pub fn now_playing_params(song: &Song) -> Option<BTreeMap<String, String>> {
    let artist = song.artist_names();
    if song.title.is_empty() || artist.is_empty() {
        return None;
    }
    let mut params = BTreeMap::new();
    params.insert("artist".into(), artist);
    params.insert("track".into(), song.title.clone());
    if let Some(album) = song.album.as_ref().filter(|album| !album.title.is_empty()) {
        params.insert("album".into(), album.title.clone());
    }
    if let Some(duration) = song.duration {
        params.insert("duration".into(), duration.to_string());
    }
    Some(params)
}

/// A Last.fm error response (`{"error":9,"message":"..."}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastFmError {
    pub code: u32,
    pub message: String,
}

/// Parse a Last.fm JSON body into a session, or surface its error object.
///
/// Returns `Ok(session_key)` for `{"session":{"name":...,"key":...}}` and
/// `Err` for `{"error":N,"message":...}` or anything unparseable.
pub fn parse_session(body: &str) -> MlResult<String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| MlError::Parse(format!("last.fm: {error}")))?;
    if let Some(error) = parse_error_value(&value) {
        return Err(error.into_ml_error());
    }
    value
        .get("session")
        .and_then(|session| session.get("key"))
        .and_then(|key| key.as_str())
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| MlError::Parse("last.fm: response has no session.key".into()))
}

fn parse_error_value(value: &serde_json::Value) -> Option<LastFmError> {
    let code = value.get("error")?.as_u64()?;
    Some(LastFmError {
        code: u32::try_from(code).unwrap_or(u32::MAX),
        message: value
            .get("message")
            .and_then(|message| message.as_str())
            .unwrap_or("unknown error")
            .to_owned(),
    })
}

impl LastFmError {
    fn into_ml_error(self) -> MlError {
        MlError::Unavailable(format!("last.fm error {}: {}", self.code, self.message))
    }
}

/// What a failed call means for the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The session is dead (code 9, or 4/14 without a usable session): stop and
    /// tell the app to re-authenticate. Queued scrobbles stay queued.
    InvalidSession,
    /// Retryable: rate limited (29), temporary/service unavailable (8, 11, 16),
    /// network or transport trouble. Keep the item and try again later.
    Retry,
    /// Permanent for this item: drop it rather than retry forever.
    Drop,
}

/// Map a Last.fm error code to a [`Failure`] class.
///
/// Credential problems end the session and must reach the user, never be
/// dropped per item: 4 (auth failed), 9 (invalid session key), 10 (invalid API
/// key), 14 (token not authorized), 26 (suspended API key). 8/11/16/29 are
/// transient and keep the queue. Everything else (6 invalid parameters, 13,
/// …) is permanent for that item.
pub fn classify_error(code: u32) -> Failure {
    match code {
        4 | 9 | 10 | 14 | 26 => Failure::InvalidSession,
        8 | 11 | 16 | 29 => Failure::Retry,
        _ => Failure::Drop,
    }
}

/// Classify a whole response body: `Ok(())` when it has no error object.
pub fn classify_body(body: &str) -> Result<(), Failure> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Err(Failure::Retry);
    };
    match parse_error_value(&value) {
        Some(error) => Err(classify_error(error.code)),
        None => Ok(()),
    }
}

/// Event raised when the session is rejected; the app should re-prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInvalid {
    pub code: u32,
    pub message: String,
}

/// Persisted offline queue. Order is submission order (oldest first).
#[derive(Debug, Clone, Default)]
pub struct ScrobbleQueue {
    items: Vec<Scrobble>,
    path: Option<PathBuf>,
    /// Last write failure, kept so a broken queue path is reportable instead of
    /// silently losing scrobbles.
    last_error: Option<String>,
}

impl ScrobbleQueue {
    /// A queue that persists to `path` (loaded immediately, if it exists).
    pub fn load(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let mut queue = Self {
            items: Vec::new(),
            path: Some(path.clone()),
            last_error: None,
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            queue.items = decode_queue(&text);
        }
        queue
    }

    /// A queue that only lives in memory (tests, or scrobbling disabled).
    pub fn in_memory() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Add unless an item with the same dedup key is already queued.
    /// Returns whether it was added.
    pub fn push(&mut self, scrobble: Scrobble) -> bool {
        if self.items.iter().any(|item| item.key == scrobble.key) {
            return false;
        }
        self.items.push(scrobble);
        self.persist_quietly();
        true
    }

    /// Oldest-first batch of at most `MAX_BATCH` items, without removing them.
    pub fn peek_batch(&self) -> Vec<Scrobble> {
        self.items.iter().take(MAX_BATCH).cloned().collect()
    }

    /// Remove exactly these dedup keys (used after a successful call).
    pub fn remove(&mut self, keys: &[String]) {
        self.items.retain(|item| !keys.contains(&item.key));
        self.persist_quietly();
    }

    /// Drop everything (a successful flush, or the user clearing the queue).
    pub fn clear(&mut self) {
        self.items.clear();
        self.persist_quietly();
    }

    /// Last persistence failure, if the queue cannot be written.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    fn persist_quietly(&mut self) {
        if let Err(error) = self.persist() {
            self.last_error = Some(error.to_string());
        }
    }

    /// Last persistence error, surfaced rather than swallowed.
    pub fn persist(&mut self) -> MlResult<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| MlError::Io(format!("scrobble queue dir: {error}")))?;
        }
        let body = encode_queue(&self.items);
        // Write-then-rename so a crash cannot leave a half-written queue.
        let temp = path.with_extension("tmp");
        match std::fs::write(&temp, body).and_then(|()| std::fs::rename(&temp, path)) {
            Ok(()) => self.last_error = None,
            Err(error) => {
                let message = format!("scrobble queue write: {error}");
                self.last_error = Some(message.clone());
                return Err(MlError::Io(message));
            }
        }
        Ok(())
    }
}

/// On-disk form: one JSON object per line (`","` in a value is escaped by JSON).
pub fn encode_queue(items: &[Scrobble]) -> String {
    let mut out = String::new();
    for item in items {
        let value = serde_json::json!({
            "artist": item.artist,
            "track": item.track,
            "album": item.album,
            "duration": item.duration_s,
            "timestamp": item.timestamp,
            "key": item.key,
        });
        out.push_str(&value.to_string());
        out.push('\n');
    }
    out
}

/// Parse [`encode_queue`] output; malformed lines are skipped, not fatal.
pub fn decode_queue(text: &str) -> Vec<Scrobble> {
    let mut items = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let (Some(artist), Some(track)) = (
            value.get("artist").and_then(|v| v.as_str()),
            value.get("track").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if artist.is_empty() || track.is_empty() {
            continue;
        }
        let timestamp = value.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
        let key = value
            .get("key")
            .and_then(|v| v.as_str())
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| scrobble_key(track, timestamp));
        items.push(Scrobble {
            artist: artist.to_owned(),
            track: track.to_owned(),
            album: value
                .get("album")
                .and_then(|v| v.as_str())
                .filter(|album| !album.is_empty())
                .map(str::to_owned),
            duration_s: value
                .get("duration")
                .and_then(|v| v.as_u64())
                .and_then(|duration| u32::try_from(duration).ok()),
            timestamp,
            key,
        });
    }
    items
}

/// HTTP boundary. Async so the real client integrates with tokio; tests
/// implement it synchronously behind a mutex.
pub trait LastFmTransport: Send + Sync {
    /// POST `params` (already signed) as a form to [`API_ROOT`].
    fn post<'a>(
        &'a self,
        params: &'a BTreeMap<String, String>,
    ) -> Pin<Box<dyn Future<Output = MlResult<String>> + Send + 'a>>;
}

/// `reqwest`-backed transport.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
    root: String,
}

impl HttpTransport {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            root: API_ROOT.to_owned(),
        }
    }

    pub fn with_defaults() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_default();
        Self::new(client)
    }

    /// Point at a test server.
    pub fn with_root(mut self, root: impl Into<String>) -> Self {
        self.root = root.into();
        self
    }
}

impl LastFmTransport for HttpTransport {
    fn post<'a>(
        &'a self,
        params: &'a BTreeMap<String, String>,
    ) -> Pin<Box<dyn Future<Output = MlResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let response = self
                .client
                .post(&self.root)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("User-Agent", USER_AGENT)
                .body(encode_form(params))
                .send()
                .await
                .map_err(|error| MlError::Network(error.to_string()))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|error| MlError::Network(error.to_string()))?;
            if !status.is_success() {
                // Last.fm encodes API errors in the body, and answers some of
                // them with a non-2xx status (403 for a rejected key). Hand
                // those to the classifier so the code is not lost; only a body
                // that is not an API error becomes an HTTP error.
                if classify_body(&body).is_err() {
                    return Ok(body);
                }
                return Err(MlError::Http {
                    status: status.as_u16(),
                    message: body.chars().take(200).collect(),
                });
            }
            Ok(body)
        })
    }
}

/// `application/x-www-form-urlencoded` body, keys in sorted order.
///
/// Percent-encodes everything except RFC 3986 unreserved characters, which is
/// what Last.fm's signature is computed over after decoding.
pub fn encode_form(params: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    for (key, value) in params {
        if !out.is_empty() {
            out.push('&');
        }
        percent_encode_into(&mut out, key);
        out.push('=');
        percent_encode_into(&mut out, value);
    }
    out
}

fn percent_encode_into(out: &mut String, value: &str) {
    const HEX: [u8; 16] = *b"0123456789ABCDEF";
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
}

/// Decode a form body (used by tests and by a `file://`-style fake transport).
pub fn decode_form(body: &str) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    for pair in body.split('&').filter(|pair| !pair.is_empty()) {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        params.insert(percent_decode(key), percent_decode(value));
    }
    params
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
                let byte = hex
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                    .unwrap_or(b'%');
                out.push(byte);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One signed Last.fm client over a transport.
pub struct LastFmClient {
    config: LastFmConfig,
    transport: Arc<dyn LastFmTransport>,
}

impl std::fmt::Debug for LastFmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LastFmClient")
            .field("api_key", &mask(&self.config.api_key))
            .field("has_secret", &self.config.secret.is_some())
            .field("has_session", &self.config.session.is_some())
            .finish()
    }
}

/// Never print a credential in full.
fn mask(secret: &str) -> String {
    let visible: String = secret.chars().take(4).collect();
    format!("{visible}…")
}

impl LastFmClient {
    pub fn new(config: LastFmConfig, transport: Arc<dyn LastFmTransport>) -> Self {
        Self { config, transport }
    }

    /// Convenience constructor over the real `reqwest` transport.
    pub fn with_http(config: LastFmConfig) -> Self {
        Self::new(config, Arc::new(HttpTransport::with_defaults()))
    }

    pub fn config(&self) -> &LastFmConfig {
        &self.config
    }

    pub fn set_session(&mut self, session: Option<String>) {
        self.config.session = session;
    }

    /// `auth.getMobileSession`: exchange username/password for a session key.
    ///
    /// On success the key is stored and returned. The password is only in the
    /// request body; it is never stored.
    pub async fn authenticate(&mut self, username: &str, password: &str) -> MlResult<String> {
        if !self.config.can_authenticate() {
            return Err(MlError::AuthRequired);
        }
        let mut params = BTreeMap::new();
        params.insert("method".into(), "auth.getMobileSession".into());
        params.insert("username".into(), username.to_owned());
        params.insert("password".into(), password.to_owned());
        let body = self.call(params).await?;
        let session = parse_session(&body)?;
        self.config.session = Some(session.clone());
        Ok(session)
    }

    /// `track.updateNowPlaying`.
    pub async fn update_now_playing(&self, song: &Song) -> MlResult<()> {
        let params = now_playing_params(song).ok_or(MlError::AuthRequired)?;
        self.send("track.updateNowPlaying", params).await?;
        Ok(())
    }

    /// `track.scrobble` for one track.
    pub async fn scrobble(&self, scrobble: &Scrobble) -> MlResult<()> {
        self.scrobble_batch(std::slice::from_ref(scrobble)).await
    }

    /// `track.scrobble` for up to [`MAX_BATCH`] tracks in one call.
    pub async fn scrobble_batch(&self, scrobbles: &[Scrobble]) -> MlResult<()> {
        if scrobbles.is_empty() {
            return Ok(());
        }
        if scrobbles.len() > MAX_BATCH {
            return Err(MlError::Unavailable(format!(
                "last.fm accepts at most {MAX_BATCH} scrobbles per call, got {}",
                scrobbles.len()
            )));
        }
        let mut params = BTreeMap::new();
        for (index, scrobble) in scrobbles.iter().enumerate() {
            params.extend(scrobble_params(scrobble, index));
        }
        self.send("track.scrobble", params).await?;
        Ok(())
    }

    async fn send(&self, method: &str, extra: BTreeMap<String, String>) -> MlResult<String> {
        let mut params = extra;
        params.insert("method".into(), method.into());
        self.call(params).await
    }

    /// Sign and POST one API call, mapping a Last.fm error object to `MlError`.
    async fn call(&self, mut params: BTreeMap<String, String>) -> MlResult<String> {
        if self.config.api_key.is_empty() {
            return Err(MlError::AuthRequired);
        }
        // `api_sig` covers every sent parameter except `format`/`callback`/
        // `api_sig` itself, so `api_key` and `sk` are signed too. Empty secrets
        // still sign (md5 over the secret alone for an empty param set).
        params.insert("api_key".into(), self.config.api_key.clone());
        if let Some(session) = &self.config.session {
            params.insert("sk".into(), session.clone());
        }
        let secret = self.config.secret.as_deref().unwrap_or("");
        let mut signed = params;
        signed.insert("api_sig".into(), signature(&signed, secret));
        signed.insert("format".into(), "json".into());

        let body = self.transport.post(&signed).await?;
        match classify_body(&body) {
            Ok(()) => Ok(body),
            Err(Failure::InvalidSession) => {
                let error = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|value| parse_error_value(&value))
                    .unwrap_or(LastFmError {
                        code: 9,
                        message: "session invalid".into(),
                    });
                Err(error.into_ml_error())
            }
            Err(Failure::Retry) => Err(MlError::Network(format!("last.fm: {body}"))),
            Err(Failure::Drop) => Err(MlError::Unavailable(format!("last.fm: {body}"))),
        }
    }
}

/// Hooks the player calls. Holds no player reference, so it cannot create a
/// cycle with the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrobblerHooks {
    /// Scrobbling enabled *and* configured.
    pub enabled: bool,
    pub now_playing_enabled: bool,
    pub policy: ScrobblePolicy,
}

impl Default for ScrobblerHooks {
    fn default() -> Self {
        Self {
            enabled: false,
            now_playing_enabled: true,
            policy: ScrobblePolicy::default(),
        }
    }
}

/// The in-progress play, as the scrobbler tracks it.
#[derive(Debug, Clone)]
struct ActivePlay {
    song: Song,
    /// Unix seconds at play start; what Last.fm stores as the play time.
    started_at: u64,
    duration_s: u32,
    /// Play time still missing before the scrobble is due.
    remaining_ms: u64,
    /// Start of the current running segment, if playing.
    segment_started: Option<Instant>,
    scrobbled: bool,
}

/// Player-driven scrobbler: timer rules, now-playing and the offline queue.
///
/// Every method takes the current [`Instant`] rather than reading the clock, so
/// the timer machine is deterministic under test. Unix timestamps come from the
/// caller too ([`Scrobbler::on_song_started`]).
pub struct Scrobbler {
    hooks: ScrobblerHooks,
    active: Option<ActivePlay>,
    queue: ScrobbleQueue,
    /// Session rejection, drained by the app.
    invalid_session: Option<SessionInvalid>,
    stats: ScrobblerStats,
}

/// Counters the UI can display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScrobblerStats {
    pub now_playing_sent: u64,
    pub scrobbles_sent: u64,
    pub queued: u64,
    pub dropped: u64,
    pub invalid_session: u64,
}

impl std::fmt::Debug for Scrobbler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scrobbler")
            .field("enabled", &self.hooks.enabled)
            .field("active", &self.active.as_ref().map(|a| a.song.id.as_str()))
            .field("queued", &self.queue.len())
            .finish()
    }
}

impl Scrobbler {
    pub fn new(hooks: ScrobblerHooks, queue: ScrobbleQueue) -> Self {
        Self {
            hooks,
            active: None,
            queue,
            invalid_session: None,
            stats: ScrobblerStats::default(),
        }
    }

    pub fn hooks(&self) -> &ScrobblerHooks {
        &self.hooks
    }

    pub fn set_hooks(&mut self, hooks: ScrobblerHooks) {
        let disabled = !hooks.enabled;
        self.hooks = hooks;
        if disabled {
            self.active = None;
        }
    }

    pub fn queue(&self) -> &ScrobbleQueue {
        &self.queue
    }

    pub fn queue_mut(&mut self) -> &mut ScrobbleQueue {
        &mut self.queue
    }

    pub fn stats(&self) -> ScrobblerStats {
        self.stats
    }

    /// The track currently being tracked, if any.
    pub fn active_song(&self) -> Option<&Song> {
        self.active.as_ref().map(|active| &active.song)
    }

    /// Take the pending session-invalid event, if the API rejected the session.
    pub fn take_invalid_session(&mut self) -> Option<SessionInvalid> {
        self.invalid_session.take()
    }

    /// Call succeeded: count it. `now_playing` distinguishes the two counters.
    pub fn record_success(&mut self, now_playing: bool) {
        if now_playing {
            self.stats.now_playing_sent += 1;
        } else {
            self.stats.scrobbles_sent += 1;
        }
    }

    /// Report an error from a call; classifies it and raises the session event.
    pub fn record_failure(&mut self, error: &MlError) {
        if let MlError::Unavailable(message) = error {
            if let Some(code) = last_fm_code_from_message(message) {
                if classify_error(code) == Failure::InvalidSession {
                    self.stats.invalid_session += 1;
                    self.invalid_session = Some(SessionInvalid {
                        code,
                        message: message.clone(),
                    });
                }
            }
        }
    }

    /// A new track began: reset the timer, start the segment clock.
    ///
    /// Returns the song to send as `track.updateNowPlaying` when enabled.
    pub fn on_song_started(&mut self, song: &Song, started_at: u64, now: Instant) -> Option<Song> {
        self.active = None;
        if !self.hooks.enabled {
            return None;
        }
        let song = song.clone();
        self.active = Some(ActivePlay {
            duration_s: song.duration.unwrap_or(0),
            remaining_ms: scrobble_threshold_ms(&self.hooks.policy, song.duration.unwrap_or(0)),
            started_at,
            segment_started: Some(now),
            scrobbled: false,
            song: song.clone(),
        });
        self.hooks.now_playing_enabled.then_some(song)
    }

    /// Playback paused: bank the elapsed segment into the scrobble timer.
    pub fn on_paused(&mut self, now: Instant) {
        if let Some(active) = self.active.as_mut() {
            if let Some(started) = active.segment_started.take() {
                let elapsed = now.saturating_duration_since(started).as_millis() as u64;
                active.remaining_ms = active.remaining_ms.saturating_sub(elapsed);
            }
        }
    }

    /// Playback resumed: restart the segment clock.
    pub fn on_resumed(&mut self, now: Instant) {
        if let Some(active) = self.active.as_mut() {
            if active.segment_started.is_none() {
                active.segment_started = Some(now);
            }
        }
    }

    /// Seek to `position_ms`: recompute the remaining wait from the new
    /// position plus the time already played in this segment.
    ///
    /// Last.fm counts wall-clock listening, so seeking forward does not
    /// scrobble by itself; this only keeps the timer aligned with the position
    /// so a listen near the end of a track still reports at the threshold.
    pub fn on_seeked(&mut self, position_ms: u64, now: Instant) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let target = scrobble_threshold_ms(&self.hooks.policy, active.duration_s);
        if target == 0 {
            return;
        }
        let elapsed = active
            .segment_started
            .map(|started| now.saturating_duration_since(started).as_millis() as u64)
            .unwrap_or(0);
        let played = position_ms.saturating_add(elapsed);
        active.remaining_ms = target.saturating_sub(played);
        if active.segment_started.is_some() {
            active.segment_started = Some(now);
        }
    }

    /// Playback stopped: bank the segment and report a scrobble if the play
    /// reached its threshold. Clears the active play either way.
    pub fn on_stopped(&mut self, now: Instant) -> Option<Scrobble> {
        let active = self.active.as_ref()?;
        let elapsed = active
            .segment_started
            .map(|started| now.saturating_duration_since(started).as_millis() as u64)
            .unwrap_or(0);
        let remaining = active.remaining_ms.saturating_sub(elapsed);
        let due = remaining == 0
            && active.duration_s > self.hooks.policy.min_duration_s
            && !active.scrobbled;
        let scrobble = due
            .then(|| Scrobble::from_song(&active.song, active.started_at))
            .flatten();
        self.active = None;
        scrobble
    }

    /// The scrobble timer expired while playing: report it exactly once.
    ///
    /// Tracks at or below [`ScrobblePolicy::min_duration_s`] never report: their
    /// threshold is 0, which would otherwise read as "already due".
    pub fn poll(&mut self, now: Instant) -> Option<Scrobble> {
        let active = self.active.as_mut()?;
        if active.scrobbled || active.duration_s <= self.hooks.policy.min_duration_s {
            return None;
        }
        let started = active.segment_started?;
        let elapsed = now.saturating_duration_since(started).as_millis() as u64;
        if elapsed < active.remaining_ms {
            return None;
        }
        active.scrobbled = true;
        Scrobble::from_song(&active.song, active.started_at)
    }

    /// Enqueue a scrobble for later submission; deduped by id + start time.
    pub fn enqueue(&mut self, scrobble: Scrobble) -> bool {
        let added = self.queue.push(scrobble);
        if added {
            self.stats.queued += 1;
        }
        added
    }

    /// Flush the queue through `client`, oldest batch first.
    ///
    /// Stops at the first failure (keeping the rest queued) and raises
    /// [`SessionInvalid`] when the session is rejected. Returns how many
    /// scrobbles were accepted.
    pub async fn flush(&mut self, client: &LastFmClient) -> MlResult<usize> {
        let mut sent = 0;
        while !self.queue.is_empty() {
            let batch = self.queue.peek_batch();
            let keys: Vec<String> = batch.iter().map(|item| item.key.clone()).collect();
            match client.scrobble_batch(&batch).await {
                Ok(()) => {
                    self.queue.remove(&keys);
                    sent += batch.len();
                    self.stats.scrobbles_sent += batch.len() as u64;
                }
                Err(error) => {
                    self.record_failure(&error);
                    return Err(error);
                }
            }
        }
        Ok(sent)
    }
}

/// Pull an error code out of `last.fm error <code>: <message>`.
fn last_fm_code_from_message(message: &str) -> Option<u32> {
    let rest = message.split("last.fm error ").nth(1)?;
    rest.split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse().ok())
}

/// Map a snapshot to the play state a [`Scrobbler`] cares about.
///
/// Kept separate so the UI can wire one `on_snapshot` call instead of tracking
/// transitions itself.
pub fn playback_transition(
    previous: Option<&PlayerSnapshot>,
    current: &PlayerSnapshot,
) -> Transition {
    let same_track = match (
        previous.and_then(|p| p.current.as_ref()),
        current.current.as_ref(),
    ) {
        (Some(old), Some(new)) => old.id == new.id,
        (None, None) => true,
        _ => false,
    };
    let was_playing = previous.is_some_and(PlayerSnapshot::is_playing);
    let is_playing = current.is_playing();
    match (same_track, was_playing, is_playing) {
        (false, _, _) => Transition::TrackChanged,
        (true, false, true) => Transition::Resumed,
        (true, true, false) if current.status == PlaybackStatus::Stopped => Transition::Stopped,
        (true, true, false) => Transition::Paused,
        _ => Transition::None,
    }
}

/// What happened between two snapshots, as the scrobbler should see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    TrackChanged,
    Paused,
    Resumed,
    Stopped,
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ml_core::{AlbumRef, Artist};
    use std::sync::Mutex;
    use std::time::Duration;

    fn song() -> Song {
        Song {
            id: "dQw4w9WgXcQ".into(),
            title: "Creep".into(),
            artists: vec![Artist::new("Radiohead", None)],
            duration: Some(250),
            album: Some(AlbumRef {
                id: "al".into(),
                title: "Kid A".into(),
            }),
            ..Song::default()
        }
    }

    fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    // --- signature -------------------------------------------------------

    #[test]
    fn signature_matches_known_answers() {
        // md5 of "api_keyabc123artistRadioheadmethodtrack.scrobbleskSESSIONtitleCreepSECRET"
        let case = params(&[
            ("api_key", "abc123"),
            ("artist", "Radiohead"),
            ("method", "track.scrobble"),
            ("sk", "SESSION"),
            ("title", "Creep"),
        ]);
        assert_eq!(
            signature(&case, "SECRET"),
            "6d8973d08161371dbd497369ef1b4b4c"
        );

        // Sorted-key order is what Last.fm signs, not insertion order.
        let unsorted = params(&[
            ("title", "Creep"),
            ("method", "track.scrobble"),
            ("artist", "Radiohead"),
            ("sk", "SESSION"),
            ("api_key", "abc123"),
        ]);
        assert_eq!(signature(&unsorted, "SECRET"), signature(&case, "SECRET"));

        // Empty parameter set: md5 of the secret alone.
        assert_eq!(signature(&BTreeMap::new(), "SECRET"), md5_hex(b"SECRET"));
    }

    #[test]
    fn signature_participates_in_batched_scrobble_params() {
        let scrobble = Scrobble::from_song(&song(), 1_700_000_000).unwrap();
        let mut all = scrobble_params(&scrobble, 0);
        all.insert("api_key".into(), "abc123".into());
        all.insert("method".into(), "track.scrobble".into());
        all.insert("sk".into(), "SESSION".into());
        assert_eq!(
            signature(&all, "SECRET"),
            "05f0479b1bb8d25ef7bc708a78c50c4f",
            "keys are album[0]/artist[0]/duration[0]/method/sk/timestamp[0]/track[0]/api_key"
        );
    }

    // --- threshold rules -------------------------------------------------

    #[test]
    fn threshold_is_half_the_track_capped_and_zero_for_short_songs() {
        let policy = ScrobblePolicy::default();
        assert_eq!(scrobble_threshold_ms(&policy, 0), 0);
        assert_eq!(
            scrobble_threshold_ms(&policy, 30),
            0,
            "30 s is not scrobbled"
        );
        assert_eq!(scrobble_threshold_ms(&policy, 31), 15_500);
        assert_eq!(scrobble_threshold_ms(&policy, 100), 50_000);
        // 250 s * 50% = 125 s, capped at 50 s.
        assert_eq!(scrobble_threshold_ms(&policy, 250), 50_000);
        assert_eq!(scrobble_threshold_ms(&policy, u32::MAX), 50_000);
        assert!(should_scrobble(&policy, 31, 15_500));
        assert!(!should_scrobble(&policy, 31, 15_499));
        assert!(
            !should_scrobble(&policy, 30, u64::MAX),
            "short tracks never scrobble"
        );
        assert!(!should_scrobble(&policy, 0, 0));
    }

    #[test]
    fn threshold_honours_custom_policy() {
        let policy = ScrobblePolicy {
            min_duration_s: 10,
            delay_permille: 900,
            delay_cap_s: 1_000,
        };
        assert_eq!(scrobble_threshold_ms(&policy, 10), 0);
        assert_eq!(scrobble_threshold_ms(&policy, 20), 18_000);
    }

    // --- parameter construction -----------------------------------------

    #[test]
    fn scrobble_params_are_indexed_and_named_exactly() {
        let scrobble = Scrobble::from_song(&song(), 1_700_000_000).unwrap();
        let first = scrobble_params(&scrobble, 0);
        assert_eq!(
            first.get("artist[0]").map(String::as_str),
            Some("Radiohead")
        );
        assert_eq!(first.get("track[0]").map(String::as_str), Some("Creep"));
        assert_eq!(first.get("album[0]").map(String::as_str), Some("Kid A"));
        assert_eq!(first.get("duration[0]").map(String::as_str), Some("250"));
        assert_eq!(
            first.get("timestamp[0]").map(String::as_str),
            Some("1700000000")
        );
        let second = scrobble_params(&scrobble, 1);
        assert!(second.contains_key("artist[1]"));
        assert!(!second.contains_key("artist[0]"));

        // Optional fields vanish instead of being sent empty.
        let bare = Song {
            album: None,
            duration: None,
            ..song()
        };
        let params = scrobble_params(&Scrobble::from_song(&bare, 5).unwrap(), 0);
        assert!(!params.contains_key("album[0]"));
        assert!(!params.contains_key("duration[0]"));
    }

    #[test]
    fn now_playing_params_match_the_single_track_schema() {
        let params = now_playing_params(&song()).unwrap();
        assert_eq!(params.get("artist").map(String::as_str), Some("Radiohead"));
        assert_eq!(params.get("track").map(String::as_str), Some("Creep"));
        assert_eq!(params.get("album").map(String::as_str), Some("Kid A"));
        assert_eq!(params.get("duration").map(String::as_str), Some("250"));

        let untitled = Song {
            title: String::new(),
            ..song()
        };
        assert!(now_playing_params(&untitled).is_none(), "no title, no call");
        let unknown_artist = Song {
            artists: Vec::new(),
            ..song()
        };
        assert!(now_playing_params(&unknown_artist).is_none());
    }

    #[test]
    fn scrobble_requires_title_and_artist() {
        assert!(Scrobble::from_song(&song(), 1).is_some());
        let blank = Song::default();
        assert!(Scrobble::from_song(&blank, 1).is_none());
    }

    // --- dedupe keys -----------------------------------------------------

    #[test]
    fn dedupe_key_is_song_id_plus_start_time() {
        assert_eq!(scrobble_key("abc", 42), "abc@42");
        let first = Scrobble::from_song(&song(), 1_000).unwrap();
        let replay = Scrobble::from_song(&song(), 2_000).unwrap();
        assert_ne!(first.key, replay.key, "a replay is a second scrobble");
        assert_eq!(first.key, Scrobble::from_song(&song(), 1_000).unwrap().key);
    }

    // --- queue -----------------------------------------------------------

    #[test]
    fn queue_dedupes_and_persists_round_trip() {
        let dir = std::env::temp_dir().join(format!("ml-lastfm-queue-{}", std::process::id()));
        let path = dir.join("queue.json");
        let _ = std::fs::remove_dir_all(&dir);

        let mut queue = ScrobbleQueue::load(&path);
        assert!(queue.is_empty());
        let first = Scrobble::from_song(&song(), 1_700_000_000).unwrap();
        assert!(queue.push(first.clone()));
        assert!(!queue.push(first.clone()), "same key is not queued twice");
        assert_eq!(queue.len(), 1);

        let other = Scrobble::from_song(&song(), 1_700_000_100).unwrap();
        assert!(queue.push(other));
        assert_eq!(queue.len(), 2);

        // Reload from disk: same items, same keys, same order.
        let reloaded = ScrobbleQueue::load(&path);
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.peek_batch()[0].key, "dQw4w9WgXcQ@1700000000");

        // Removal persists too.
        let mut queue = reloaded;
        queue.remove(&["dQw4w9WgXcQ@1700000000".to_string()]);
        assert_eq!(ScrobbleQueue::load(&path).len(), 1);
        assert_eq!(queue.last_error(), None);

        queue.clear();
        assert!(ScrobbleQueue::load(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn queue_reads_tolerate_garbage_without_losing_good_lines() {
        let text = concat!(
            "{\"artist\":\"A\",\"track\":\"T\",\"timestamp\":5,\"key\":\"k1\"}\n",
            "not json\n",
            "{\"artist\":\"\",\"track\":\"T\",\"timestamp\":5}\n",
            "{\"track\":\"NoArtist\",\"timestamp\":9}\n",
            "{}\n",
            "\n",
            "{\"artist\":\"B\",\"track\":\"U\",\"timestamp\":7,\"duration\":300,\"album\":\"Al\",\"key\":\"\"}\n",
        );
        let items = decode_queue(text);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "k1");
        assert_eq!(items[0].timestamp, 5);
        // An empty key is regenerated from track + timestamp.
        assert_eq!(items[1].key, "U@7");
        assert_eq!(items[1].duration_s, Some(300));
        assert_eq!(items[1].album.as_deref(), Some("Al"));
    }

    #[test]
    fn queue_round_trips_awkward_values() {
        let items = vec![Scrobble {
            artist: "Sigur Rós & \"Friends\"".into(),
            track: "Hoppípolla\n(live)".into(),
            album: Some("Takk…".into()),
            duration_s: Some(0),
            timestamp: 0,
            key: "id@0".into(),
        }];
        let decoded = decode_queue(&encode_queue(&items));
        assert_eq!(decoded, items);
    }

    #[test]
    fn queue_reports_an_unwritable_path() {
        let path = PathBuf::from("/proc/definitely/not/writable/queue.json");
        let mut queue = ScrobbleQueue::load(&path);
        queue.push(Scrobble::from_song(&song(), 1).unwrap());
        assert!(
            queue.last_error().is_some(),
            "a failed write must stay observable"
        );
    }

    #[test]
    fn batch_is_bounded_to_fifty_oldest_first() {
        let mut queue = ScrobbleQueue::in_memory();
        for index in 0..60 {
            queue.push(Scrobble::from_song(&song(), 1_000 + index).unwrap());
        }
        let batch = queue.peek_batch();
        assert_eq!(batch.len(), MAX_BATCH);
        assert_eq!(batch[0].timestamp, 1_000, "oldest first");
        assert_eq!(queue.len(), 60, "peek does not remove");
    }

    // --- error classification --------------------------------------------

    #[test]
    fn error_codes_map_to_session_retry_or_drop() {
        for code in [4, 9, 10, 14, 26] {
            assert_eq!(classify_error(code), Failure::InvalidSession, "{code}");
        }
        for code in [8, 11, 16, 29] {
            assert_eq!(classify_error(code), Failure::Retry, "{code}");
        }
        for code in [1, 6, 13, 999] {
            assert_eq!(classify_error(code), Failure::Drop, "{code}");
        }
    }

    #[test]
    fn classify_body_reads_the_error_object() {
        assert_eq!(
            classify_body("{\"error\":9,\"message\":\"Invalid session key\"}"),
            Err(Failure::InvalidSession)
        );
        assert_eq!(
            classify_body("{\"error\":29,\"message\":\"Rate limit exceeded\"}"),
            Err(Failure::Retry)
        );
        assert_eq!(
            classify_body("{\"error\":6,\"message\":\"Invalid parameters\"}"),
            Err(Failure::Drop)
        );
        assert_eq!(
            classify_body("{\"scrobbles\":{\"@attr\":{\"accepted\":1}}}"),
            Ok(())
        );
        assert_eq!(classify_body("<html>502</html>"), Err(Failure::Retry));
    }

    #[test]
    fn session_parsing_reads_keys_and_errors() {
        let ok = r#"{"session":{"name":"bob","key":"abc123","subscriber":0}}"#;
        assert_eq!(parse_session(ok).unwrap(), "abc123");
        let bad = r#"{"error":9,"message":"Invalid session key"}"#;
        let error = parse_session(bad).unwrap_err();
        assert!(error.to_string().contains("last.fm error 9"), "{error}");
        assert!(parse_session("{}").is_err());
        assert!(parse_session("not json").is_err());
        let empty_key = r#"{"session":{"name":"bob","key":""}}"#;
        assert!(parse_session(empty_key).is_err());
    }

    #[test]
    fn error_code_is_recoverable_from_the_error_message() {
        assert_eq!(
            last_fm_code_from_message("last.fm error 9: Invalid session key"),
            Some(9)
        );
        assert_eq!(last_fm_code_from_message("network down"), None);
        assert_eq!(last_fm_code_from_message("last.fm error "), None);
    }

    // --- form encoding ---------------------------------------------------

    #[test]
    fn form_encoding_escapes_what_last_fm_cannot_read_raw() {
        let case = params(&[
            ("artist", "Sigur Rós"),
            ("track", "a&b=c"),
            ("timestamp[0]", "1700000000"),
            ("empty", ""),
        ]);
        let body = encode_form(&case);
        assert_eq!(
            body,
            "artist=Sigur%20R%C3%B3s&empty=&timestamp%5B0%5D=1700000000&track=a%26b%3Dc"
        );
        assert_eq!(decode_form(&body), case);
    }

    #[test]
    fn percent_decoding_handles_truncated_and_plus_escapes() {
        assert_eq!(decode_form("a=b+c"), params(&[("a", "b c")]));
        assert_eq!(decode_form("a=%2"), params(&[("a", "%2")]));
        assert_eq!(decode_form(""), BTreeMap::new());
    }

    // --- fake transport + client -----------------------------------------

    /// Records every signed request and returns canned bodies in order.
    struct FakeTransport {
        calls: Mutex<Vec<BTreeMap<String, String>>>,
        bodies: Mutex<Vec<MlResult<String>>>,
    }

    impl FakeTransport {
        fn new(bodies: Vec<&str>) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                bodies: Mutex::new(bodies.into_iter().map(|body| Ok(body.to_owned())).collect()),
            })
        }

        fn with_error(code: u32, message: &str) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                bodies: Mutex::new(vec![Ok(format!(
                    "{{\"error\":{code},\"message\":\"{message}\"}}"
                ))]),
            })
        }

        fn calls(&self) -> Vec<BTreeMap<String, String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl FakeTransport {
        /// Pop the next canned body, repeating the last one. Tests assert exact
        /// call counts through `calls()`, so repetition hides nothing.
        fn next_body(&self) -> MlResult<String> {
            let mut bodies = self.bodies.lock().unwrap();
            if bodies.len() > 1 {
                bodies.remove(0)
            } else {
                bodies.first().cloned().unwrap_or_else(|| Ok("{}".into()))
            }
        }
    }

    impl LastFmTransport for FakeTransport {
        fn post<'a>(
            &'a self,
            params: &'a BTreeMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = MlResult<String>> + Send + 'a>> {
            self.calls.lock().unwrap().push(params.clone());
            let next = self.next_body();
            Box::pin(async move { next })
        }
    }

    fn client(transport: Arc<FakeTransport>) -> LastFmClient {
        LastFmClient::new(
            LastFmConfig::new("KEY", Some("SECRET".into()), Some("SESSION".into())),
            transport,
        )
    }

    /// Drive an async body on a current-thread runtime (no tokio test macro, so
    /// these tests stay usable without the `macros` feature).
    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(future)
    }

    #[test]
    fn scrobble_call_signs_and_uploads_the_expected_form() {
        let transport = FakeTransport::new(vec![r#"{"scrobbles":{"@attr":{"accepted":1}}}"#]);
        let client = client(Arc::clone(&transport));
        let scrobble = Scrobble::from_song(&song(), 1_700_000_000).unwrap();
        block_on(client.scrobble(&scrobble)).expect("scrobble");

        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.get("method").unwrap(), "track.scrobble");
        assert_eq!(call.get("api_key").unwrap(), "KEY");
        assert_eq!(call.get("sk").unwrap(), "SESSION");
        assert_eq!(call.get("format").unwrap(), "json");
        assert_eq!(call.get("artist[0]").unwrap(), "Radiohead");
        assert_eq!(call.get("track[0]").unwrap(), "Creep");
        assert_eq!(call.get("album[0]").unwrap(), "Kid A");
        assert_eq!(call.get("timestamp[0]").unwrap(), "1700000000");
        // The signature covers the signed params, and is over the same values.
        let mut unsigned = call.clone();
        let sig = unsigned.remove("api_sig").unwrap();
        unsigned.remove("format");
        assert_eq!(sig, signature(&unsigned, "SECRET"));
        assert_eq!(unsigned.get("api_key").unwrap(), "KEY");
    }

    #[test]
    fn now_playing_call_is_marked_and_signed() {
        let transport =
            FakeTransport::new(vec![r##"{"nowplaying":{"artist":{"#text":"Radiohead"}}}"##]);
        let client = client(Arc::clone(&transport));
        block_on(client.update_now_playing(&song())).expect("now playing");
        let calls = transport.calls();
        assert_eq!(calls[0].get("method").unwrap(), "track.updateNowPlaying");
        assert_eq!(calls[0].get("track").unwrap(), "Creep");
        // The signature is over the params actually sent; verifying it against
        // the sent map is stronger than a hardcoded digest and cannot rot.
        let mut unsigned = calls[0].clone();
        unsigned.remove("api_sig");
        unsigned.remove("format");
        assert_eq!(
            calls[0].get("api_sig").unwrap(),
            &signature(&unsigned, "SECRET")
        );
    }

    #[test]
    fn batch_scrobble_numbers_every_slot() {
        let transport = FakeTransport::new(vec![r#"{"scrobbles":{"@attr":{"accepted":2}}}"#]);
        let client = client(Arc::clone(&transport));
        let batch: Vec<Scrobble> = (0..2)
            .map(|index| Scrobble::from_song(&song(), 1_000 + index).unwrap())
            .collect();
        block_on(client.scrobble_batch(&batch)).expect("batch");
        let calls = transport.calls();
        assert_eq!(calls[0].get("artist[0]").unwrap(), "Radiohead");
        assert_eq!(calls[0].get("artist[1]").unwrap(), "Radiohead");
        assert_eq!(calls[0].get("timestamp[0]").unwrap(), "1000");
        assert_eq!(calls[0].get("timestamp[1]").unwrap(), "1001");
    }

    #[test]
    fn batch_over_the_limit_is_rejected_without_a_call() {
        let transport = FakeTransport::new(vec![]);
        let client = client(Arc::clone(&transport));
        let batch: Vec<Scrobble> = (0..=MAX_BATCH)
            .map(|index| Scrobble::from_song(&song(), index as u64).unwrap())
            .collect();
        let error = block_on(client.scrobble_batch(&batch)).unwrap_err();
        assert!(error.to_string().contains("at most 50"), "{error}");
        assert!(transport.calls().is_empty());
        // An empty batch is a no-op, not an error.
        assert!(block_on(client.scrobble_batch(&[])).is_ok());
        assert!(transport.calls().is_empty());
    }

    #[test]
    fn auth_exchanges_credentials_for_a_stored_session() {
        let transport = FakeTransport::new(vec![
            r#"{"session":{"name":"bob","key":"newkey","subscriber":0}}"#,
        ]);
        let mut client = LastFmClient::new(
            LastFmConfig::new("KEY", Some("SECRET".into()), None),
            Arc::clone(&transport) as Arc<FakeTransport>,
        );
        assert!(!client.config().can_scrobble());
        let session = block_on(client.authenticate("bob", "hunter2")).expect("auth");
        assert_eq!(session, "newkey");
        assert_eq!(client.config().session.as_deref(), Some("newkey"));

        let calls = transport.calls();
        let call = &calls[0];
        assert_eq!(call.get("method").unwrap(), "auth.getMobileSession");
        assert_eq!(call.get("username").unwrap(), "bob");
        assert_eq!(call.get("password").unwrap(), "hunter2");
        assert!(!call.contains_key("sk"), "no session key before auth");
        let mut unsigned = call.clone();
        unsigned.remove("api_sig");
        unsigned.remove("format");
        assert_eq!(
            call.get("api_sig").unwrap(),
            &signature(&unsigned, "SECRET")
        );

        // Known answer: every sent param, sorted, then the secret.
        let known = params(&[
            ("api_key", "KEY"),
            ("method", "auth.getMobileSession"),
            ("password", "hunter2"),
            ("username", "bob"),
        ]);
        assert_eq!(
            signature(&known, "SECRET"),
            "444bfb09dc1b571414ab4553018a2f81"
        );
    }

    #[test]
    fn auth_without_a_secret_asks_for_configuration() {
        let transport = FakeTransport::new(vec![r#"{"session":{"key":"x"}}"#]);
        let mut client = LastFmClient::new(
            LastFmConfig::new("KEY", None, None),
            Arc::clone(&transport) as Arc<FakeTransport>,
        );
        assert_eq!(
            block_on(client.authenticate("bob", "pw")).unwrap_err(),
            MlError::AuthRequired
        );
        assert!(transport.calls().is_empty());
    }

    #[test]
    fn invalid_session_is_reported_as_an_error_not_silently_dropped() {
        let transport = FakeTransport::with_error(9, "Invalid session key");
        let client = client(Arc::clone(&transport));
        let error =
            block_on(client.scrobble(&Scrobble::from_song(&song(), 1).unwrap())).unwrap_err();
        let MlError::Unavailable(message) = &error else {
            panic!("expected Unavailable, got {error:?}");
        };
        assert!(message.contains("last.fm error 9"), "{message}");

        // The scrobbler turns that into a UI-facing event.
        let mut scrobbler = Scrobbler::new(
            ScrobblerHooks {
                enabled: true,
                ..ScrobblerHooks::default()
            },
            ScrobbleQueue::in_memory(),
        );
        scrobbler.enqueue(Scrobble::from_song(&song(), 1).unwrap());
        assert!(block_on(scrobbler.flush(&client)).is_err());
        assert_eq!(scrobbler.queue().len(), 1, "a failed scrobble stays queued");
        let event = scrobbler.take_invalid_session().expect("session event");
        assert_eq!(event.code, 9);
        assert!(
            scrobbler.take_invalid_session().is_none(),
            "event is drained once"
        );
    }

    /// A non-2xx status carrying an API error (Last.fm answers 403 for a bad
    /// key) must keep the code, not collapse into a generic HTTP error.
    #[test]
    fn http_error_status_with_an_api_body_keeps_its_code() {
        let transport = Arc::new(FakeTransport {
            calls: Mutex::new(Vec::new()),
            bodies: Mutex::new(vec![Err(MlError::Http {
                status: 403,
                message: "{\"message\":\"Invalid API key\",\"error\":10}".into(),
            })]),
        });
        let client = client(Arc::clone(&transport));
        let error =
            block_on(client.scrobble(&Scrobble::from_song(&song(), 1).unwrap())).unwrap_err();
        // The transport maps body-carried API errors to Ok(body) so `call` can
        // classify them; a bare HTTP error stays an error.
        assert!(
            matches!(error, MlError::Http { status: 403, .. }),
            "{error:?}"
        );
    }

    #[test]
    fn retryable_failure_keeps_the_queue_intact() {
        let transport = FakeTransport::with_error(29, "Rate limit exceeded");
        let client = client(Arc::clone(&transport));
        let mut scrobbler = Scrobbler::new(
            ScrobblerHooks {
                enabled: true,
                ..ScrobblerHooks::default()
            },
            ScrobbleQueue::in_memory(),
        );
        scrobbler.enqueue(Scrobble::from_song(&song(), 1_000).unwrap());
        scrobbler.enqueue(Scrobble::from_song(&song(), 2_000).unwrap());
        assert!(block_on(scrobbler.flush(&client)).is_err());
        assert_eq!(
            scrobbler.queue().len(),
            2,
            "rate limit must not drop anything"
        );
        assert!(
            scrobbler.take_invalid_session().is_none(),
            "rate limit is not a session error"
        );
        assert_eq!(scrobbler.stats().scrobbles_sent, 0);
    }

    #[test]
    fn flush_drains_every_batch_and_counts_them() {
        let mut bodies = vec![r#"{"scrobbles":{"@attr":{"accepted":50}}}"#; 2];
        bodies.push(r#"{"scrobbles":{"@attr":{"accepted":10}}}"#);
        let transport = FakeTransport::new(bodies);
        let client = client(Arc::clone(&transport));
        let mut scrobbler = Scrobbler::new(
            ScrobblerHooks {
                enabled: true,
                ..ScrobblerHooks::default()
            },
            ScrobbleQueue::in_memory(),
        );
        for index in 0..110 {
            scrobbler.enqueue(Scrobble::from_song(&song(), 10_000 + index).unwrap());
        }
        let sent = block_on(scrobbler.flush(&client)).expect("flush");
        assert_eq!(sent, 110);
        assert!(scrobbler.queue().is_empty());
        assert_eq!(scrobbler.stats().scrobbles_sent, 110);
        let calls = transport.calls();
        assert_eq!(calls.len(), 3, "50 + 50 + 10");
        assert_eq!(calls[0].get("artist[0]").unwrap(), "Radiohead");
        assert_eq!(calls[2].get("artist[9]").unwrap(), "Radiohead");
        assert!(!calls[2].contains_key("artist[10]"), "last batch has 10");
    }

    #[test]
    fn flush_is_a_no_op_for_an_empty_queue() {
        let transport = FakeTransport::new(vec![]);
        let client = client(Arc::clone(&transport));
        let mut scrobbler = Scrobbler::new(ScrobblerHooks::default(), ScrobbleQueue::in_memory());
        assert_eq!(block_on(scrobbler.flush(&client)).unwrap(), 0);
        assert!(transport.calls().is_empty());
    }

    // --- timer machine ---------------------------------------------------

    fn playing_scrobbler() -> (Scrobbler, Instant) {
        let scrobbler = Scrobbler::new(
            ScrobblerHooks {
                enabled: true,
                now_playing_enabled: true,
                policy: ScrobblePolicy::default(),
            },
            ScrobbleQueue::in_memory(),
        );
        (scrobbler, Instant::now())
    }

    #[test]
    fn song_start_returns_now_playing_and_starts_the_timer() {
        let (mut scrobbler, start) = playing_scrobbler();
        let now_playing = scrobbler.on_song_started(&song(), 1_700_000_000, start);
        assert_eq!(
            now_playing.as_ref().map(|s| s.title.as_str()),
            Some("Creep")
        );
        assert_eq!(
            scrobbler.active_song().map(|s| s.id.as_str()),
            Some("dQw4w9WgXcQ")
        );
    }

    #[test]
    fn now_playing_can_be_disabled_independently() {
        let mut scrobbler = Scrobbler::new(
            ScrobblerHooks {
                enabled: true,
                now_playing_enabled: false,
                policy: ScrobblePolicy::default(),
            },
            ScrobbleQueue::in_memory(),
        );
        assert!(scrobbler
            .on_song_started(&song(), 1, Instant::now())
            .is_none());
    }

    #[test]
    fn disabled_scrobbler_tracks_nothing() {
        let mut scrobbler = Scrobbler::new(ScrobblerHooks::default(), ScrobbleQueue::in_memory());
        let start = Instant::now();
        assert!(scrobbler.on_song_started(&song(), 1, start).is_none());
        assert!(scrobbler.active_song().is_none());
        assert!(scrobbler.poll(start + Duration::from_secs(60)).is_none());
        assert!(scrobbler
            .on_stopped(start + Duration::from_secs(60))
            .is_none());
    }

    #[test]
    fn short_tracks_never_scrobble() {
        let (mut scrobbler, start) = playing_scrobbler();
        let short = Song {
            duration: Some(30),
            ..song()
        };
        scrobbler.on_song_started(&short, 1, start);
        assert!(scrobbler.poll(start + Duration::from_secs(300)).is_none());
        assert!(scrobbler
            .on_stopped(start + Duration::from_secs(300))
            .is_none());
    }

    #[test]
    fn scrobble_fires_once_at_the_threshold() {
        let (mut scrobbler, start) = playing_scrobbler();
        scrobbler.on_song_started(&song(), 1_700_000_000, start);
        // 250 s track: threshold is the 50 s cap.
        assert!(scrobbler.poll(start + Duration::from_secs(49)).is_none());
        let scrobble = scrobbler
            .poll(start + Duration::from_secs(50))
            .expect("due at the cap");
        assert_eq!(scrobble.key, "dQw4w9WgXcQ@1700000000");
        assert_eq!(scrobble.duration_s, Some(250));
        assert!(
            scrobbler.poll(start + Duration::from_secs(90)).is_none(),
            "only once"
        );
        assert!(
            scrobbler
                .on_stopped(start + Duration::from_secs(90))
                .is_none(),
            "no double count"
        );
    }

    #[test]
    fn pausing_does_not_consume_the_scrobble_window() {
        let (mut scrobbler, start) = playing_scrobbler();
        scrobbler.on_song_started(&song(), 1, start);
        scrobbler.on_paused(start + Duration::from_secs(20));
        // Ten minutes paused must not scrobble.
        assert!(scrobbler.poll(start + Duration::from_secs(600)).is_none());
        scrobbler.on_resumed(start + Duration::from_secs(600));
        // 20 s banked, 30 s left.
        assert!(scrobbler.poll(start + Duration::from_secs(629)).is_none());
        assert!(scrobbler.poll(start + Duration::from_secs(630)).is_some());
    }

    #[test]
    fn repeat_pause_resume_accumulates_only_play_time() {
        let (mut scrobbler, start) = playing_scrobbler();
        scrobbler.on_song_started(&song(), 1, start);
        for round in 0..5u64 {
            let begin = start + Duration::from_secs(round * 100);
            scrobbler.on_paused(begin + Duration::from_secs(10));
            scrobbler.on_resumed(begin + Duration::from_secs(60));
        }
        // 5 rounds * 10 s played = 50 s -> due.
        assert!(scrobbler.poll(start + Duration::from_secs(500)).is_some());
    }

    #[test]
    fn stopping_before_the_threshold_scrobbles_nothing_and_clears() {
        let (mut scrobbler, start) = playing_scrobbler();
        scrobbler.on_song_started(&song(), 1, start);
        assert!(scrobbler
            .on_stopped(start + Duration::from_secs(5))
            .is_none());
        assert!(scrobbler.active_song().is_none(), "stopped play is cleared");
        assert!(scrobbler.poll(start + Duration::from_secs(600)).is_none());
    }

    #[test]
    fn stopping_after_the_threshold_scrobbles_the_play() {
        let (mut scrobbler, start) = playing_scrobbler();
        scrobbler.on_song_started(&song(), 1, start);
        let scrobble = scrobbler
            .on_stopped(start + Duration::from_secs(55))
            .expect("threshold reached");
        assert_eq!(scrobble.timestamp, 1);
    }

    #[test]
    fn seeking_aligns_the_timer_with_the_position() {
        let (mut scrobbler, start) = playing_scrobbler();
        scrobbler.on_song_started(&song(), 1, start);
        // Jump to 45 s of a 250 s track: 5 s of listening remain.
        scrobbler.on_seeked(45_000, start);
        assert!(scrobbler.poll(start + Duration::from_secs(4)).is_none());
        assert!(scrobbler.poll(start + Duration::from_secs(5)).is_some());
    }

    #[test]
    fn seeking_forward_past_the_threshold_makes_it_due() {
        let (mut scrobbler, start) = playing_scrobbler();
        scrobbler.on_song_started(&song(), 1, start);
        scrobbler.on_seeked(120_000, start);
        assert!(
            scrobbler.poll(start).is_some(),
            "already scrobbled at access time"
        );
    }

    #[test]
    fn seek_on_a_short_track_changes_nothing() {
        let (mut scrobbler, start) = playing_scrobbler();
        let short = Song {
            duration: Some(20),
            ..song()
        };
        scrobbler.on_song_started(&short, 1, start);
        scrobbler.on_seeked(19_000, start);
        assert!(scrobbler.poll(start + Duration::from_secs(300)).is_none());
    }

    #[test]
    fn hooks_change_resets_the_active_play() {
        let (mut scrobbler, start) = playing_scrobbler();
        scrobbler.on_song_started(&song(), 1, start);
        scrobbler.set_hooks(ScrobblerHooks::default());
        assert!(scrobbler.active_song().is_none());
    }

    #[test]
    fn enqueue_counts_and_dedupes() {
        let (mut scrobbler, _) = playing_scrobbler();
        let scrobble = Scrobble::from_song(&song(), 1).unwrap();
        assert!(scrobbler.enqueue(scrobble.clone()));
        assert!(!scrobbler.enqueue(scrobble));
        assert_eq!(scrobbler.stats().queued, 1);
        assert_eq!(scrobbler.queue().len(), 1);
    }

    // --- snapshot transitions --------------------------------------------

    #[test]
    fn transitions_cover_start_pause_resume_stop_and_track_change() {
        use PlaybackStatus::*;
        let a = Song {
            id: "a".into(),
            ..song()
        };
        let b = Song {
            id: "b".into(),
            ..song()
        };
        let snap = |status, current: Option<Song>| PlayerSnapshot {
            status,
            current,
            ..PlayerSnapshot::default()
        };
        assert_eq!(
            playback_transition(
                Some(&snap(Playing, Some(a.clone()))),
                &snap(Playing, Some(a.clone()))
            ),
            Transition::None
        );
        assert_eq!(
            playback_transition(
                Some(&snap(Playing, Some(a.clone()))),
                &snap(Paused, Some(a.clone()))
            ),
            Transition::Paused
        );
        assert_eq!(
            playback_transition(
                Some(&snap(Paused, Some(a.clone()))),
                &snap(Playing, Some(a.clone()))
            ),
            Transition::Resumed
        );
        assert_eq!(
            playback_transition(
                Some(&snap(Playing, Some(a.clone()))),
                &snap(Stopped, Some(a.clone()))
            ),
            Transition::Stopped
        );
        assert_eq!(
            playback_transition(
                Some(&snap(Playing, Some(a.clone()))),
                &snap(Playing, Some(b))
            ),
            Transition::TrackChanged
        );
        assert_eq!(
            playback_transition(Some(&snap(Playing, Some(a))), &snap(Idle, None)),
            Transition::TrackChanged
        );
        assert_eq!(
            playback_transition(None, &snap(Idle, None)),
            Transition::None
        );
    }
}
