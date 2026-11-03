//! Pure UI-state transitions.
//!
//! These functions are deterministic, total and panic-free so the proof lead
//! can freeze them into Verus spec/exec pairs (`metrolist-linux/proofs/**`).
//! Rendering, egui layout and async I/O are outside Verus's Rust subset and are
//! therefore not specified here.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::routes::Route;
use ml_core::{Artist, MlError, Queue, Song};
use ml_downloads::{progress_percent, Download, DownloadState};
use ml_recognition::{RecognitionError, RecognitionResult};
use ml_store::{StatsPeriod, StatsReport, StatsSummary};

/// Slow vinyl rotation, 10 degrees per second.
pub const VINYL_RADIANS_PER_SEC: f32 = 10.0 * std::f32::consts::PI / 180.0;

/// Wrap an angle into `[0, 2*pi)`.
pub fn wrap_vinyl_angle(angle: f32) -> f32 {
    const TAU: f32 = std::f32::consts::TAU;
    let wrapped = angle % TAU;
    if wrapped < 0.0 {
        wrapped + TAU
    } else {
        wrapped + 0.0
    }
}

/// Advance the vinyl angle by a frame, clamping long frames so a resume or
/// stall cannot jump the disc. Paused/loading frames freeze the angle.
pub fn advance_vinyl_angle(angle: f32, dt: f32, playing: bool) -> f32 {
    if !playing {
        return wrap_vinyl_angle(angle);
    }
    wrap_vinyl_angle(angle + dt.clamp(0.0, 0.05) * VINYL_RADIANS_PER_SEC)
}

/// Progress of a recognition capture, clamped to `0.0..=1.0`.
pub fn recognition_progress(elapsed_ms: u64, sample_ms: u64) -> f32 {
    if sample_ms == 0 {
        return 1.0;
    }
    (elapsed_ms.min(sample_ms) as f64 / sample_ms as f64) as f32
}

/// Listening timer text, e.g. `4.2s / 12.0s`.
pub fn recognition_timer_text(elapsed_ms: u64, sample_ms: u64) -> String {
    format!(
        "{:.1}s / {:.1}s",
        elapsed_ms.min(sample_ms) as f64 / 1000.0,
        sample_ms as f64 / 1000.0
    )
}

/// Capture and processing timing shown under a match.
pub fn recognition_timing_text(sample_ms: u64, elapsed_ms: u64) -> String {
    format!(
        "{:.1}s sample · matched in {:.1}s",
        sample_ms as f64 / 1000.0,
        elapsed_ms as f64 / 1000.0
    )
}

/// Artist plus album for the result card subtitle.
pub fn recognition_subtitle(result: &RecognitionResult) -> String {
    let artist = result.artist.trim();
    let album = result.album.as_deref().unwrap_or("").trim();
    match (artist.is_empty(), album.is_empty()) {
        (true, true) => "Unknown artist".to_string(),
        (false, true) => artist.to_string(),
        (true, false) => album.to_string(),
        (false, false) => format!("{artist} · {album}"),
    }
}

/// Genre, release date and label, omitting missing fields.
pub fn recognition_meta(result: &RecognitionResult) -> String {
    [
        result.genre.as_deref(),
        result.release_date.as_deref(),
        result.label.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// Search query used to turn a Shazam match into a playable YouTube song.
pub fn recognition_search_query(result: &RecognitionResult) -> String {
    let query = format!("{} {}", result.title.trim(), result.artist.trim());
    let query = query.trim();
    if query.is_empty() {
        "song".to_string()
    } else {
        query.to_string()
    }
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// YouTube Music search URL for the recognised track.
pub fn recognition_youtube_music_url(query: &str) -> String {
    format!(
        "https://music.youtube.com/search?q={}",
        percent_encode(query.trim())
    )
}

/// Plain-text summary for the Copy action.
pub fn recognition_copy_text(result: &RecognitionResult) -> String {
    let mut lines = vec![result.title.trim().to_string()];
    if !result.artist.trim().is_empty() {
        lines.push(result.artist.trim().to_string());
    }
    if let Some(album) = result
        .album
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        lines.push(album.to_string());
    }
    let meta = recognition_meta(result);
    if !meta.is_empty() {
        lines.push(meta);
    }
    if let Some(url) = result.shazam_url.as_deref().filter(|url| !url.is_empty()) {
        lines.push(url.to_string());
    }
    lines.join("\n")
}

fn folded(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Best search result for a match: exact video id first, then title+artist,
/// then title, then the first result.
pub fn best_recognition_match(result: &RecognitionResult, songs: &[Song]) -> Option<usize> {
    if songs.is_empty() {
        return None;
    }
    if let Some(video_id) = result.youtube_video_id.as_deref() {
        if let Some(index) = songs.iter().position(|song| song.id == video_id) {
            return Some(index);
        }
    }
    let title = folded(&result.title);
    let artist = folded(&result.artist);
    if !title.is_empty() {
        if let Some(index) = songs.iter().position(|song| {
            folded(&song.title) == title
                && (artist.is_empty()
                    || folded(&song.artist_names()) == artist
                    || folded(&song.artist_names()).contains(&artist))
        }) {
            return Some(index);
        }
        if let Some(index) = songs.iter().position(|song| folded(&song.title) == title) {
            return Some(index);
        }
        if let Some(index) = songs.iter().position(|song| {
            let candidate = folded(&song.title);
            !candidate.is_empty() && (candidate.contains(&title) || title.contains(&candidate))
        }) {
            return Some(index);
        }
    }
    Some(0)
}

/// Minimal playable song from a Shazam video id, used only when the YouTube
/// Music search has no usable result.
pub fn recognition_fallback_song(result: &RecognitionResult) -> Option<Song> {
    let id = result.youtube_video_id.as_deref()?.trim();
    if id.is_empty() {
        return None;
    }
    let title = result.title.trim();
    let artist = result.artist.trim();
    Some(Song {
        id: id.to_string(),
        title: if title.is_empty() {
            "Recognised track".to_string()
        } else {
            title.to_string()
        },
        artists: if artist.is_empty() {
            Vec::new()
        } else {
            vec![Artist::new(artist, None)]
        },
        // No browse id: leave the album off the playable fallback row.
        album: None,
        thumbnail_url: result
            .artwork_hq_url
            .clone()
            .or_else(|| result.artwork_url.clone()),
        ..Song::default()
    })
}

/// User-facing recognition error; never includes audio or credentials.
pub fn recognition_error_message(error: &RecognitionError) -> String {
    match error {
        RecognitionError::NoMatch => {
            "No match found. Move closer to the music and try again.".to_string()
        }
        RecognitionError::NoInputDevice(detail) => {
            format!("No microphone is available: {detail}")
        }
        RecognitionError::Capture(detail) => format!("Microphone capture failed: {detail}"),
        RecognitionError::Timeout => "Recognition timed out. Try again.".to_string(),
        RecognitionError::RateLimited => {
            "Recognition is rate limited. Wait a moment and try again.".to_string()
        }
        RecognitionError::Network(detail) => format!("Network error while recognising: {detail}"),
        RecognitionError::Unavailable(detail) => {
            format!("The recognition service is unavailable: {detail}")
        }
        RecognitionError::Http { status, message } => {
            format!("Recognition failed (HTTP {status}): {message}")
        }
        RecognitionError::Parse(detail) => format!("Unreadable recognition response: {detail}"),
        RecognitionError::InvalidAudio => "The recording was too short to recognise.".to_string(),
        RecognitionError::UnsupportedFormat(detail) => {
            format!("Unsupported microphone format: {detail}")
        }
        RecognitionError::Io(detail) => format!("Recognition I/O error: {detail}"),
    }
}

/// Actual listened duration: position is already pause-aware, then clamp to the
/// track length so a seek cannot overstate listening time.
pub fn played_duration_ms(position_ms: u64, duration_ms: u64) -> u64 {
    if duration_ms == 0 {
        position_ms
    } else {
        position_ms.min(duration_ms)
    }
}

/// Mirrors upstream's per-song lyrics offset range.
pub const LYRICS_OFFSET_MAX_MS: i64 = 3_000;

/// Next lyrics offset after a button step, clamped to the upstream ±3 s range.
pub fn lyrics_offset_after(current_ms: i64, delta_ms: i64) -> i64 {
    current_ms
        .saturating_add(delta_ms)
        .clamp(-LYRICS_OFFSET_MAX_MS, LYRICS_OFFSET_MAX_MS)
}

/// Typed, user-facing refusal before any translation request is made.
pub const TRANSLATION_MISSING_KEY_MESSAGE: &str =
    "Translation needs an API key. Add one in Settings > Lyrics.";
pub const TRANSLATION_ENDPOINT_MESSAGE: &str =
    "Add an http:// or https:// OpenAI-compatible endpoint in Settings > Lyrics.";

/// Basic endpoint sanity check; the request is never attempted otherwise.
pub fn translation_endpoint_valid(url: &str) -> bool {
    let url = url.trim();
    (url.starts_with("https://") || url.starts_with("http://")) && url.len() > 8
}

/// What the Last.fm Connect/Test button can do with the current fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastFmConnectMode {
    /// Key + secret + username + password: call `auth.getMobileSession`.
    Authenticate,
    /// A supplied session key is enough; no request is made.
    AcceptSession,
    /// Nothing usable was entered.
    Missing,
}

/// Pure gate for the Last.fm connect button.
pub fn lastfm_connect_mode(
    api_key: &str,
    secret: &str,
    username: &str,
    password: &str,
    session: &str,
) -> LastFmConnectMode {
    if [api_key, secret, username, password]
        .iter()
        .all(|value| !value.trim().is_empty())
    {
        LastFmConnectMode::Authenticate
    } else if !api_key.trim().is_empty() && !secret.trim().is_empty() && !session.trim().is_empty()
    {
        LastFmConnectMode::AcceptSession
    } else {
        LastFmConnectMode::Missing
    }
}

/// Remove every known secret value from a provider/error message before it
/// reaches the UI, a toast or a log. Values shorter than three characters are
/// still replaced; an empty value is ignored.
pub fn redact_secrets(text: &str, secrets: &[&str]) -> String {
    let mut redacted = text.to_string();
    for secret in secrets {
        let secret = secret.trim();
        if !secret.is_empty() {
            redacted = redacted.replace(secret, "[redacted]");
        }
    }
    redacted
}

/// Signed offset label, e.g. `+0.5 s`, `-0.5 s`, `0.0 s`.
pub fn lyrics_offset_label(offset_ms: i64) -> String {
    if offset_ms == 0 {
        "0.0 s".to_string()
    } else {
        format!("{:+.1} s", offset_ms as f64 / 1000.0)
    }
}

/// Seek target for a normalized slider value.
///
/// Postconditions:
/// - `result <= duration_ms`
/// - `duration_ms == 0 ==> result == 0`
/// - `normalized.is_nan() ==> result == 0`
/// - `normalized >= 1.0 ==> result == duration_ms`
/// - monotone in `normalized` for a fixed duration
pub fn seek_target(normalized: f32, duration_ms: u64) -> u64 {
    if duration_ms == 0 || normalized.is_nan() {
        return 0;
    }
    let normalized = normalized.clamp(0.0, 1.0);
    let scaled = (normalized as f64) * (duration_ms as f64);
    if !scaled.is_finite() {
        return 0;
    }
    (scaled as u64).min(duration_ms)
}

/// Volume from a slider value.
///
/// Postconditions: `0.0 <= result <= 1.0`; NaN maps to `0.0`.
pub fn volume_target(normalized: f32) -> f32 {
    if normalized.is_nan() {
        0.0
    } else {
        normalized.clamp(0.0, 1.0)
    }
}

/// Quantize a crossfade slider value to whole seconds in `0..=30`.
///
/// Postconditions: result `<= 30`; non-finite input maps to `0` (off).
pub fn crossfade_seconds(value: f32) -> u32 {
    if value.is_finite() {
        value.round().clamp(0.0, 30.0) as u32
    } else {
        0
    }
}

/// Display text for a crossfade duration.
pub fn crossfade_label(seconds: u32) -> String {
    if seconds == 0 {
        "Off".to_string()
    } else {
        format!("{seconds} s")
    }
}

/// Quantize playback speed to 0.05× in `0.5..=2.0`; non-finite maps to 1.0.
pub fn playback_speed(value: f32) -> f32 {
    if value.is_finite() {
        (value * 20.0).round().clamp(10.0, 40.0) / 20.0
    } else {
        1.0
    }
}

/// Playback-speed percentage shown in Settings.
pub fn speed_percent(value: f32) -> u32 {
    (playback_speed(value) * 100.0).round() as u32
}

/// Quantize normalization gain to 0.5 dB in `-15.0..=3.0`.
///
/// Postconditions: result is finite and within the range; NaN maps to `0.0`.
pub fn normalization_gain(value: f32) -> f32 {
    if value.is_finite() {
        (value * 2.0).round().clamp(-30.0, 6.0) / 2.0
    } else {
        0.0
    }
}

/// Signed dB text for the manual normalization gain.
pub fn gain_label(value: f32) -> String {
    let gain = normalization_gain(value);
    if gain == 0.0 {
        "0.0 dB".to_string()
    } else {
        format!("{gain:+.1} dB")
    }
}

/// Human period label used by Stats/Wrapped chips and clipboard text.
pub fn stats_period_label(period: StatsPeriod) -> &'static str {
    match period {
        StatsPeriod::Week => "Last 7 days",
        StatsPeriod::Month => "Last month",
        StatsPeriod::Year => "This year",
        StatsPeriod::All => "All time",
    }
}

/// Listening time as `h`/`m`, clamped at zero. Positive sub-minute values are
/// reported as `<1m` rather than `0m`.
pub fn listening_time_text(ms: i64) -> String {
    let ms = ms.max(0);
    let minutes = ms / 60_000;
    if minutes == 0 {
        return if ms > 0 {
            "<1m".to_string()
        } else {
            "0m".to_string()
        };
    }
    let hours = minutes / 60;
    let rest = minutes % 60;
    match (hours, rest) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Three-letter month name; out-of-range months render `?` instead of panicking.
pub fn month_short(month: u32) -> &'static str {
    if month == 0 {
        return "?";
    }
    MONTHS.get((month - 1) as usize).copied().unwrap_or("?")
}

/// `dd Mon yyyy` in UTC for a play timestamp. Total over every `i64` using
/// i128 calendar arithmetic (Hinnant civil-from-days).
pub fn stats_date_text(timestamp_ms: i64) -> String {
    let days = (timestamp_ms as i128).div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    let month = u32::try_from(month).unwrap_or(1);
    format!("{:02} {} {}", day, month_short(month), year)
}

/// Clipboard summary for the Wrapped story. Pure; never includes fake values.
pub fn wrapped_summary(report: &StatsReport, period: StatsPeriod) -> String {
    let summary = &report.summary;
    let top_song = report
        .top_songs
        .first()
        .map(|stat| {
            let artists = stat.song.artist_names();
            if artists.is_empty() {
                format!("{} ({} plays)", stat.song.title, stat.plays)
            } else {
                format!("{} — {} ({} plays)", stat.song.title, artists, stat.plays)
            }
        })
        .unwrap_or_else(|| "—".to_string());
    let top_artist = report
        .top_artists
        .first()
        .map(|stat| format!("{} ({} plays)", stat.artist.name, stat.plays))
        .unwrap_or_else(|| "—".to_string());
    let top_album = report
        .top_albums
        .first()
        .map(|stat| format!("{} ({} plays)", stat.album.title, stat.plays))
        .unwrap_or_else(|| "—".to_string());
    let first = summary
        .first_play_ms
        .map(stats_date_text)
        .unwrap_or_else(|| "—".to_string());
    let last = summary
        .last_play_ms
        .map(stats_date_text)
        .unwrap_or_else(|| "—".to_string());
    let monthly = report
        .monthly
        .iter()
        .rev()
        .take(12)
        .rev()
        .map(|month| {
            format!(
                "{} {}: {}",
                month_short(month.month),
                month.year,
                month.plays
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Metrolist Wrapped — {}\n{} listened across {} plays\nTop song: {}\nTop artist: {}\nTop album: {}\nLongest streak: {} days\nFirst play: {}\nLast play: {}\nMonthly plays: {}\n",
        stats_period_label(period),
        listening_time_text(summary.total_ms),
        summary.event_count,
        top_song,
        top_artist,
        top_album,
        summary.longest_streak_days,
        first,
        last,
        if monthly.is_empty() { "—" } else { &monthly }
    )
}

/// Summary numbers shown by the Stats screen; kept out of the render code so
/// the wording stays testable.
pub fn stats_summary_cards(summary: &StatsSummary) -> Vec<(&'static str, String)> {
    vec![
        ("Plays", summary.event_count.to_string()),
        ("Listening", listening_time_text(summary.total_ms)),
        ("Songs", summary.unique_songs.to_string()),
        ("Artists", summary.unique_artists.to_string()),
        ("Albums", summary.unique_albums.to_string()),
        ("Streak", format!("{} days", summary.longest_streak_days)),
        (
            "First play",
            summary
                .first_play_ms
                .map(stats_date_text)
                .unwrap_or_else(|| "—".to_string()),
        ),
        (
            "Last play",
            summary
                .last_play_ms
                .map(stats_date_text)
                .unwrap_or_else(|| "—".to_string()),
        ),
    ]
}

/// Default persistent session cookie file under the app data directory.
pub fn session_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("session-cookies.txt")
}

/// A pasted import path: surrounding whitespace and one pair of quotes are
/// stripped, and a leading `~/` (or bare `~`) expands against `home`. A path
/// with no `$HOME` is returned unchanged so the caller's error names it as typed.
pub fn expand_import_path(input: &str, home: Option<&Path>) -> PathBuf {
    let text = input.trim();
    let text = text
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| {
            text.strip_prefix('\'')
                .and_then(|inner| inner.strip_suffix('\''))
        })
        .unwrap_or(text);
    if text == "~" {
        return home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(text));
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = home {
            return home.join(rest);
        }
    }
    PathBuf::from(text)
}

/// Shape of a cookie file the user picked; used only for messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieImportFormat {
    Netscape,
    Json,
    Header,
    Unknown,
}

/// Detect the three formats accepted by `CookieJar::load` without parsing
/// secret values.
pub fn cookie_import_format(text: &str) -> CookieImportFormat {
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return CookieImportFormat::Json;
    }
    if trimmed
        .lines()
        .any(|line| line.starts_with("# Netscape") || line.split('\t').count() >= 7)
    {
        return CookieImportFormat::Netscape;
    }
    if trimmed.contains('=') {
        CookieImportFormat::Header
    } else {
        CookieImportFormat::Unknown
    }
}

/// User-facing account error text. Cookie values never enter this mapping.
pub fn account_error_message(error: &MlError) -> String {
    match error {
        MlError::AuthRequired => {
            "The cookie file has no SAPISID; export a signed-in YouTube Music session.".to_string()
        }
        MlError::Http { status, .. } => {
            format!("Google rejected the session (HTTP {status}); import fresh cookies.")
        }
        MlError::Unavailable(message) if message.contains("signed-out") => {
            "Google returned a signed-out menu; the cookies are stale or were exported while logged out. Import a fresh browser session.".to_string()
        }
        MlError::Network(message) => format!("Network error while checking the account: {message}"),
        MlError::Unavailable(message) => format!("Account unavailable: {message}"),
        MlError::Parse(message) => format!("Could not parse the account response: {message}"),
        MlError::Io(message) => format!("Account I/O error: {message}"),
        other => format!("Account request failed: {other}"),
    }
}

/// Short cookie-expiry hint; `None` when the session has no dated cookies.
pub fn expiry_hint(expires_at: Option<i64>, now_unix: i64) -> Option<String> {
    let expires_at = expires_at?;
    let remaining = expires_at.saturating_sub(now_unix);
    if remaining <= 0 {
        return Some("Session cookies have expired; import fresh cookies.".to_string());
    }
    let days = remaining / 86_400;
    Some(if days == 0 {
        "Session cookies expire in under a day.".to_string()
    } else if days == 1 {
        "Session cookies expire in 1 day.".to_string()
    } else {
        format!("Session cookies expire in {days} days.")
    })
}

/// Position of `id` in `songs` (first match).
///
/// Postconditions:
/// - `result.is_some() ==> result < songs.len()`
/// - `result == Some(i) ==> songs[i].id == id`
pub fn queue_index_for_id(songs: &[ml_core::Song], id: &str) -> Option<usize> {
    songs.iter().position(|song| song.id == id)
}

/// Validate a search box value.
///
/// Postconditions:
/// - `result.is_some() ==> !result.trim().is_empty()`
/// - `result.is_some() ==> result == query.trim()`
/// - `result.is_none() ==> query.trim().is_empty()`
pub fn valid_search_query(query: &str) -> Option<&str> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Start indices of runs of equal adjacent keys (used to group history under
/// "Today"/"Yesterday"/... headers).
///
/// Postconditions:
/// - `keys.is_empty() ==> result.is_empty()`
/// - `!keys.is_empty() ==> result[0] == 0`
/// - indices are strictly increasing and every index is `< keys.len()`
/// - `i == 0 || keys[i] != keys[i - 1]` holds exactly for indices in `result`
pub fn group_starts<T: PartialEq>(keys: &[T]) -> Vec<usize> {
    let mut starts = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        if index == 0 || keys[index - 1] != *key {
            starts.push(index);
        }
    }
    starts
}

/// Target index for an up/down move in a playlist of `len` songs.
///
/// Postconditions:
/// - `result == Some(target) ==> target < len && target != index`
/// - `index >= len || delta == 0 || no valid target ==> result.is_none()`
/// - no arithmetic panic for any inputs (checked arithmetic only)
pub fn playlist_move_target(index: usize, len: usize, delta: i8) -> Option<usize> {
    if index >= len || delta == 0 {
        return None;
    }
    let target = if delta < 0 {
        index.checked_sub(delta.unsigned_abs() as usize)?
    } else {
        index.checked_add(delta as usize)?
    };
    (target < len).then_some(target)
}

/// Appearance seed without an alpha channel.
///
/// Postcondition: `result <= 0x00FF_FFFF` and the low 24 bits equal `value`.
pub fn seed_color_value(value: u32) -> u32 {
    value & 0x00FF_FFFF
}

/// Validate and normalize a proxy URL.
///
/// Postconditions:
/// - `Some(url) ==> url == raw.trim() && !url.is_empty()`
/// - `Some(url) ==> url starts with http:// or https:// (ASCII-case-insensitive)`
/// - `Some(url) ==> url contains no whitespace`
/// - `None` for empty, whitespace-containing, or other-scheme input
pub fn sanitize_proxy_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

/// Relative seek target clamped to the track.
///
/// Postconditions:
/// - `result <= duration_ms` when `duration_ms > 0`
/// - negative offsets never underflow; positive offsets saturate
pub fn seek_relative(position_ms: u64, duration_ms: u64, offset_ms: i64) -> u64 {
    let target = if offset_ms < 0 {
        position_ms.saturating_sub(offset_ms.unsigned_abs())
    } else {
        position_ms.saturating_add(offset_ms as u64)
    };
    if duration_ms == 0 {
        target
    } else {
        target.min(duration_ms)
    }
}

/// One keyboard volume step, clamped and NaN-safe.
pub fn volume_step(volume: f32, delta: f32) -> f32 {
    if volume.is_nan() || delta.is_nan() {
        return volume_target(volume);
    }
    volume_target(volume + delta)
}

/// Clamp 10 EQ gains (dB) to the engine's usable range.
///
/// Postconditions:
/// - exactly 10 finite values in `-12.0..=12.0`
/// - missing bands are 0 dB; extra bands are ignored; NaN becomes 0 dB
pub fn clamp_eq_bands(bands: &[f32]) -> [f32; 10] {
    let mut out = [0.0; 10];
    for (slot, gain) in out.iter_mut().zip(bands) {
        *slot = if gain.is_finite() {
            gain.clamp(-12.0, 12.0)
        } else {
            0.0
        };
    }
    out
}

/// Sleep-timer minutes -> seconds, clamped to the controller's 24 h deadline.
pub fn sleep_seconds(minutes: u32) -> u64 {
    u64::from(minutes).saturating_mul(60).min(24 * 60 * 60)
}

/// Parse a `metrolist://play?v=<video id>` deep link (query params ignored
/// after `v`, fragment ignored). Returns the video id.
///
/// Postconditions: `Some(id)` implies 1..=64 ASCII `[A-Za-z0-9_-]` characters.
pub fn parse_deep_link(arg: &str) -> Option<String> {
    let rest = arg.trim().strip_prefix("metrolist://play")?;
    let query = rest.strip_prefix('?')?;
    let query = query.split('#').next().unwrap_or("");
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("v=") {
            return sanitize_video_id(value);
        }
    }
    None
}

/// First playable deep link in forwarded argv (the URL may be the only arg).
pub fn first_deep_link(args: &[String]) -> Option<String> {
    args.iter().find_map(|arg| parse_deep_link(arg))
}

fn sanitize_video_id(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > 64 {
        return None;
    }
    value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        .then(|| value.to_owned())
}

/// Tray menu action -> UI command. Total; no side effects.
pub fn tray_command(action: ml_integrations::TrayAction) -> TrayCommand {
    use ml_integrations::TrayAction;
    match action {
        TrayAction::ShowHide => TrayCommand::ToggleWindow,
        TrayAction::Show => TrayCommand::Show,
        TrayAction::Hide => TrayCommand::Hide,
        TrayAction::PlayPause => TrayCommand::PlayPause,
        TrayAction::Next => TrayCommand::Next,
        TrayAction::Previous => TrayCommand::Previous,
        TrayAction::Quit => TrayCommand::Quit,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleWindow,
    Show,
    Hide,
    PlayPause,
    Next,
    Previous,
    Quit,
}

/// `true` when a radio queue should fetch its next continuation soon enough
/// that gapless prefetch can consume it before the queue ends.
///
/// Total: empty queues and an unknown current index never extend.
pub fn radio_should_extend(position: Option<usize>, len: usize) -> bool {
    match position {
        Some(position) if len > 0 => position.saturating_add(5) >= len,
        _ => false,
    }
}

/// Parse the persisted `kv("queue")` value, rejecting corrupt JSON, duplicate
/// video ids and an inconsistent playback order. An empty queue is valid.
///
/// Postconditions: `Some(q)` has a valid `position` and every song id is
/// unique; malformed/legacy JSON yields `None` so startup can begin empty.
pub fn parse_persisted_queue(raw: &str) -> Option<Queue> {
    let queue: Queue = serde_json::from_str(raw).ok()?;
    let len = queue.len();
    if len == 0 {
        return Some(queue);
    }
    if queue.position() >= len {
        return None;
    }
    let mut ids = HashSet::with_capacity(len);
    for song in queue.songs() {
        if song.id.is_empty() || !ids.insert(song.id.as_str()) {
            return None;
        }
    }
    // A corrupt `order` makes `iter` drop entries.
    if queue.iter().count() != len {
        return None;
    }
    Some(queue)
}

/// Visual/actionable summary of one download entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadBadge {
    None,
    Queued,
    Downloading(u8),
    Completed,
    Failed,
    Cancelled,
}

/// What a click on the download icon should do for a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadClick {
    Download,
    Cancel,
    Retry,
    None,
}

/// Map an optional manifest entry to the row icon state.
///
/// Postconditions: `None` and non-running states are total; `Downloading`
/// carries exactly [`Download::percent`], hence `0..=100`.
pub fn download_badge(entry: Option<&Download>) -> DownloadBadge {
    match entry.map(|entry| entry.state) {
        None => DownloadBadge::None,
        Some(DownloadState::Queued) => DownloadBadge::Queued,
        Some(DownloadState::Downloading) => {
            DownloadBadge::Downloading(entry.map(Download::percent).unwrap_or(0))
        }
        Some(DownloadState::Completed) => DownloadBadge::Completed,
        Some(DownloadState::Failed) => DownloadBadge::Failed,
        Some(DownloadState::Cancelled) => DownloadBadge::Cancelled,
    }
}

/// Material Symbols glyph for each download state.
pub fn download_icon(badge: DownloadBadge) -> &'static str {
    match badge {
        DownloadBadge::None => "download",
        DownloadBadge::Queued => "schedule",
        DownloadBadge::Downloading(_) => "downloading",
        DownloadBadge::Completed => "download_done",
        DownloadBadge::Failed => "error",
        DownloadBadge::Cancelled => "restart_alt",
    }
}

/// Hover hint for each download state.
pub fn download_hint(badge: DownloadBadge) -> &'static str {
    match badge {
        DownloadBadge::None => "Download for offline",
        DownloadBadge::Queued => "Queued — click to cancel",
        DownloadBadge::Downloading(_) => "Downloading — click to cancel",
        DownloadBadge::Completed => "Downloaded — manage in Library > Downloads",
        DownloadBadge::Failed => "Failed — click to retry",
        DownloadBadge::Cancelled => "Cancelled — click to retry",
    }
}

/// Click action for each download state.
///
/// Completed is terminal in the downloads crate; removal is an explicit
/// Downloads-page action, never an accidental row click.
pub fn download_click(badge: DownloadBadge) -> DownloadClick {
    match badge {
        DownloadBadge::None => DownloadClick::Download,
        DownloadBadge::Queued | DownloadBadge::Downloading(_) => DownloadClick::Cancel,
        DownloadBadge::Failed | DownloadBadge::Cancelled => DownloadClick::Retry,
        DownloadBadge::Completed => DownloadClick::None,
    }
}

/// Human-readable byte count with binary units, one decimal above KiB.
///
/// Postconditions: never panics; `0` formats as `"0 B"`; unit is one of
/// `B`, `KiB`, `MiB`, `GiB`, `TiB`.
pub fn bytes_text(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if value < KIB * KIB {
        format!("{:.1} KiB", value / KIB)
    } else if value < KIB * KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else if value < KIB * KIB * KIB * KIB {
        format!("{:.1} GiB", value / (KIB * KIB * KIB))
    } else {
        format!("{:.1} TiB", value / (KIB * KIB * KIB * KIB))
    }
}

/// Download progress label. Unknown totals stay indeterminate instead of lying
/// with `0%`.
pub fn download_progress_text(bytes: u64, total: u64) -> String {
    if total == 0 {
        if bytes == 0 {
            "starting…".to_string()
        } else {
            format!("{}/unknown", bytes_text(bytes))
        }
    } else {
        format!("{}%", progress_percent(bytes, total))
    }
}

/// Used/limit percentage for the storage meter.
///
/// Postconditions: `None` iff `limit` is `None`; `Some(p) => p <= 100`.
/// A zero-byte limit is reported as 100% because the manager treats it as full.
pub fn storage_percent(used: u64, limit: Option<u64>) -> Option<u8> {
    let limit = limit?;
    if limit == 0 {
        return Some(100);
    }
    Some(progress_percent(used, limit))
}

/// Turn an absolute local path into a `file://` URL for mpv.
///
/// Every byte outside unreserved URL characters and `/` is percent-encoded, so
/// spaces and non-ASCII file names survive. Total; never panics.
pub fn local_file_url(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let bytes = path.as_os_str().as_bytes();
    let mut out = String::with_capacity(bytes.len() + 8);
    out.push_str("file://");
    for &byte in bytes {
        if byte == b'/' || byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
        {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0F) as usize] as char);
        }
    }
    out
}

/// Back-stack pop with a home fallback; never panics on an empty stack.
///
/// Postconditions: `stack.len()` decreases by one when it was non-empty; the
/// returned route is exactly the previous top of the stack.
pub fn back_route(stack: &mut Vec<Route>) -> Option<Route> {
    stack.pop()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_clamps_and_handles_invalid_values() {
        assert_eq!(seek_target(0.0, 120_000), 0);
        assert_eq!(seek_target(f32::NAN, 120_000), 0);
        assert_eq!(seek_target(f32::INFINITY, 120_000), 120_000);
        assert_eq!(seek_target(0.5, 120_000), 60_000);
        assert_eq!(seek_target(2.0, 120_000), 120_000);
        assert_eq!(seek_target(0.5, 0), 0);
        for step in 0..=100 {
            let value = seek_target(step as f32 / 100.0, 3_600_000);
            assert!(value <= 3_600_000);
        }
    }

    #[test]
    fn volume_is_clamped() {
        assert_eq!(volume_target(-1.0), 0.0);
        assert_eq!(volume_target(2.0), 1.0);
        assert_eq!(volume_target(f32::NAN), 0.0);
        assert_eq!(volume_target(0.25), 0.25);
    }

    #[test]
    fn queue_lookup_and_query_validation() {
        let songs = vec![
            ml_core::Song {
                id: "a".into(),
                ..ml_core::Song::default()
            },
            ml_core::Song {
                id: "b".into(),
                ..ml_core::Song::default()
            },
        ];
        assert_eq!(queue_index_for_id(&songs, "b"), Some(1));
        assert_eq!(queue_index_for_id(&songs, "z"), None);
        assert_eq!(valid_search_query("  aphex  "), Some("aphex"));
        assert_eq!(valid_search_query("   "), None);
    }

    #[test]
    fn history_grouping_is_total() {
        let keys: Vec<String> = vec!["Today", "Today", "Yesterday", "2026-10", "2026-10"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(group_starts(&keys), vec![0, 2, 3]);
        assert!(group_starts::<u8>(&[]).is_empty());
        assert_eq!(group_starts(&[7]), vec![0]);
        assert_eq!(group_starts(&[1, 2, 3]), vec![0, 1, 2]);
        assert_eq!(group_starts(&[1, 1, 1]), vec![0]);
    }

    #[test]
    fn playlist_move_target_is_bounded() {
        assert_eq!(playlist_move_target(0, 5, -1), None);
        assert_eq!(playlist_move_target(0, 5, 1), Some(1));
        assert_eq!(playlist_move_target(4, 5, 1), None);
        assert_eq!(playlist_move_target(4, 5, -1), Some(3));
        assert_eq!(playlist_move_target(0, 0, 1), None);
        assert_eq!(playlist_move_target(3, 5, 0), None);
        for index in 0..5usize {
            for delta in [-1i8, 1] {
                if let Some(target) = playlist_move_target(index, 5, delta) {
                    assert!(target < 5 && target != index);
                }
            }
        }
    }

    #[test]
    fn settings_validators_are_total() {
        assert_eq!(seed_color_value(0xFFED_5564), 0x00ED_5564);
        assert_eq!(
            sanitize_proxy_url("  http://127.0.0.1:8080  ").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(
            sanitize_proxy_url("https://proxy.example"),
            Some("https://proxy.example".to_string())
        );
        assert_eq!(sanitize_proxy_url("socks5://localhost"), None);
        assert_eq!(sanitize_proxy_url("http://bad url"), None);
        assert_eq!(sanitize_proxy_url("   "), None);
    }

    #[test]
    fn player_extras_are_bounded_and_total() {
        assert_eq!(seek_relative(10_000, 30_000, 5_000), 15_000);
        assert_eq!(seek_relative(1_000, 30_000, -5_000), 0);
        assert_eq!(seek_relative(29_000, 30_000, 5_000), 30_000);
        assert_eq!(seek_relative(10_000, 0, 5_000), 15_000);
        assert_eq!(volume_step(0.98, 0.05), 1.0);
        assert_eq!(volume_step(0.02, -0.05), 0.0);
        assert_eq!(volume_step(f32::NAN, 0.1), 0.0);

        let bands = clamp_eq_bands(&[30.0, -30.0, f32::NAN, 1.5]);
        assert_eq!(bands.len(), 10);
        assert_eq!(bands[0], 12.0);
        assert_eq!(bands[1], -12.0);
        assert_eq!(bands[2], 0.0);
        assert_eq!(bands[3], 1.5);
        assert!(bands[4..].iter().all(|gain| *gain == 0.0));

        assert_eq!(sleep_seconds(0), 0);
        assert_eq!(sleep_seconds(15), 900);
        assert_eq!(sleep_seconds(u32::MAX), 86_400);
    }

    #[test]
    fn deep_links_parse_only_play_urls() {
        assert_eq!(
            parse_deep_link("metrolist://play?v=abc_DEF-123"),
            Some("abc_DEF-123".to_string())
        );
        assert_eq!(
            parse_deep_link("metrolist://play?t=5&v=dQw4w9WgXcQ&x=1"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(parse_deep_link("metrolist://play?v=bad id"), None);
        assert_eq!(parse_deep_link("metrolist://play?v="), None);
        assert_eq!(parse_deep_link("metrolist://album?v=abc"), None);
        assert_eq!(parse_deep_link("https://example.com/play?v=abc"), None);
        let args = vec!["--flag".to_string(), "metrolist://play?v=xyz".to_string()];
        assert_eq!(first_deep_link(&args), Some("xyz".to_string()));
        assert_eq!(first_deep_link(&[]), None);
    }

    #[test]
    fn tray_actions_map_one_to_one() {
        use ml_integrations::TrayAction;
        assert_eq!(
            tray_command(TrayAction::ShowHide),
            TrayCommand::ToggleWindow
        );
        assert_eq!(tray_command(TrayAction::Show), TrayCommand::Show);
        assert_eq!(tray_command(TrayAction::Hide), TrayCommand::Hide);
        assert_eq!(tray_command(TrayAction::PlayPause), TrayCommand::PlayPause);
        assert_eq!(tray_command(TrayAction::Next), TrayCommand::Next);
        assert_eq!(tray_command(TrayAction::Previous), TrayCommand::Previous);
        assert_eq!(tray_command(TrayAction::Quit), TrayCommand::Quit);
    }

    #[test]
    fn download_badges_and_actions_are_total() {
        let mut entry = Download::new(
            ml_core::Song {
                id: "x".into(),
                ..ml_core::Song::default()
            },
            ml_core::AudioQuality::Auto,
            0,
        );
        assert_eq!(download_badge(None), DownloadBadge::None);
        assert_eq!(download_badge(Some(&entry)), DownloadBadge::Queued);
        entry.state = DownloadState::Downloading;
        entry.bytes_downloaded = 25;
        entry.total_bytes = Some(100);
        assert_eq!(download_badge(Some(&entry)), DownloadBadge::Downloading(25));
        entry.state = DownloadState::Completed;
        entry.size_bytes = 100;
        assert_eq!(download_badge(Some(&entry)), DownloadBadge::Completed);
        entry.state = DownloadState::Failed;
        assert_eq!(download_badge(Some(&entry)), DownloadBadge::Failed);
        entry.state = DownloadState::Cancelled;
        assert_eq!(download_badge(Some(&entry)), DownloadBadge::Cancelled);

        assert_eq!(download_click(DownloadBadge::None), DownloadClick::Download);
        assert_eq!(
            download_click(DownloadBadge::Downloading(1)),
            DownloadClick::Cancel
        );
        assert_eq!(
            download_click(DownloadBadge::Completed),
            DownloadClick::None
        );
        assert_eq!(download_click(DownloadBadge::Failed), DownloadClick::Retry);
        for badge in [
            DownloadBadge::None,
            DownloadBadge::Queued,
            DownloadBadge::Downloading(50),
            DownloadBadge::Completed,
            DownloadBadge::Failed,
            DownloadBadge::Cancelled,
        ] {
            assert!(!download_icon(badge).is_empty());
            assert!(!download_hint(badge).is_empty());
        }
    }

    #[test]
    fn download_formatting_is_bounded_and_exact() {
        assert_eq!(bytes_text(0), "0 B");
        assert_eq!(bytes_text(1023), "1023 B");
        assert_eq!(bytes_text(1024), "1.0 KiB");
        assert_eq!(bytes_text(1024 * 1024), "1.0 MiB");
        assert_eq!(bytes_text(5 * 1024 * 1024 * 1024), "5.0 GiB");
        assert_eq!(download_progress_text(0, 0), "starting…");
        assert_eq!(download_progress_text(512, 0), "512 B/unknown");
        assert_eq!(download_progress_text(0, 100), "0%");
        assert_eq!(download_progress_text(50, 100), "50%");
        assert_eq!(download_progress_text(200, 100), "100%");
        assert_eq!(storage_percent(0, None), None);
        assert_eq!(storage_percent(0, Some(0)), Some(100));
        assert_eq!(storage_percent(0, Some(100)), Some(0));
        assert_eq!(storage_percent(50, Some(100)), Some(50));
        assert_eq!(storage_percent(200, Some(100)), Some(100));
    }

    #[test]
    fn local_paths_become_percent_encoded_file_urls() {
        use std::path::Path;
        assert_eq!(
            local_file_url(Path::new("/tmp/a b/é.mp3")),
            "file:///tmp/a%20b/%C3%A9.mp3"
        );
        assert_eq!(local_file_url(Path::new("/a-b_c.d~e")), "file:///a-b_c.d~e");
    }

    #[test]
    fn persisted_queue_parsing_rejects_corruption_and_duplicates() {
        let song = |id: &str| ml_core::Song {
            id: id.to_string(),
            title: id.to_string(),
            ..ml_core::Song::default()
        };
        assert!(parse_persisted_queue("").is_none());
        assert!(parse_persisted_queue("{").is_none());
        let empty = serde_json::to_string(&Queue::default()).unwrap();
        assert!(parse_persisted_queue(&empty).is_some());

        let mut queue = Queue::new(vec![song("a"), song("b")], ml_core::QueueSource::Album);
        queue.jump_to(1);
        let parsed = parse_persisted_queue(&serde_json::to_string(&queue).unwrap()).unwrap();
        assert_eq!(parsed.position(), 1);
        assert_eq!(parsed.source(), ml_core::QueueSource::Album);
        assert_eq!(parsed.current().unwrap().id, "b");

        let duplicate = Queue::new(vec![song("a"), song("a")], ml_core::QueueSource::List);
        assert!(parse_persisted_queue(&serde_json::to_string(&duplicate).unwrap()).is_none());
    }

    #[test]
    fn radio_extension_window_is_total() {
        assert!(!radio_should_extend(None, 0));
        assert!(!radio_should_extend(None, 50));
        assert!(!radio_should_extend(Some(0), 50));
        assert!(radio_should_extend(Some(45), 50));
        assert!(radio_should_extend(Some(49), 50));
        assert!(radio_should_extend(Some(usize::MAX), 50));
    }

    #[test]
    fn playback_controls_clamp_and_format() {
        assert_eq!(crossfade_seconds(f32::NAN), 0);
        assert_eq!(crossfade_seconds(-4.0), 0);
        assert_eq!(crossfade_seconds(4.6), 5);
        assert_eq!(crossfade_seconds(99.0), 30);
        assert_eq!(crossfade_label(0), "Off");
        assert_eq!(crossfade_label(5), "5 s");
        assert_eq!(playback_speed(f32::NAN), 1.0);
        assert_eq!(playback_speed(0.1), 0.5);
        assert_eq!(playback_speed(1.234), 1.25);
        assert_eq!(playback_speed(9.0), 2.0);
        assert_eq!(speed_percent(1.25), 125);
        assert_eq!(normalization_gain(f32::NAN), 0.0);
        assert_eq!(normalization_gain(-99.0), -15.0);
        assert_eq!(normalization_gain(2.26), 2.5);
        assert_eq!(normalization_gain(99.0), 3.0);
        assert_eq!(gain_label(0.0), "0.0 dB");
        assert_eq!(gain_label(-6.0), "-6.0 dB");
        assert_eq!(gain_label(2.5), "+2.5 dB");
    }

    #[test]
    fn account_helpers_are_total() {
        assert_eq!(
            session_file_path(Path::new("/data")),
            PathBuf::from("/data/session-cookies.txt")
        );
        let home = Path::new("/home/user");
        assert_eq!(
            expand_import_path("  ~/Downloads/cookies.txt  ", Some(home)),
            PathBuf::from("/home/user/Downloads/cookies.txt")
        );
        assert_eq!(
            expand_import_path("\"/home/user/cookies.txt\"", Some(home)),
            PathBuf::from("/home/user/cookies.txt")
        );
        assert_eq!(
            expand_import_path("'/home/user/cookies.txt'", Some(home)),
            PathBuf::from("/home/user/cookies.txt")
        );
        assert_eq!(
            expand_import_path("/absolute/cookies.txt", Some(home)),
            PathBuf::from("/absolute/cookies.txt")
        );
        assert_eq!(expand_import_path("~", Some(home)), home.to_path_buf());
        assert_eq!(
            expand_import_path("~/Downloads/cookies.txt", None),
            PathBuf::from("~/Downloads/cookies.txt")
        );
        assert_eq!(
            cookie_import_format("# Netscape HTTP Cookie File\n.youtube.com\tTRUE"),
            CookieImportFormat::Netscape
        );
        assert_eq!(
            cookie_import_format("[{\"name\":\"SAPISID\"}]"),
            CookieImportFormat::Json
        );
        assert_eq!(
            cookie_import_format("SAPISID=abc; HSID=def"),
            CookieImportFormat::Header
        );
        assert_eq!(
            cookie_import_format("not a cookie file"),
            CookieImportFormat::Unknown
        );

        let auth = account_error_message(&MlError::AuthRequired);
        assert!(auth.contains("SAPISID"));
        let signed_out = account_error_message(&MlError::Unavailable(
            "account menu: signed-out response".to_string(),
        ));
        assert!(signed_out.contains("signed-out menu"));
        assert!(account_error_message(&MlError::Network("offline".into())).contains("offline"));
        assert!(account_error_message(&MlError::Http {
            status: 401,
            message: "no".into()
        })
        .contains("401"));

        assert_eq!(expiry_hint(None, 10), None);
        assert!(expiry_hint(Some(5), 10).unwrap().contains("expired"));
        assert!(expiry_hint(Some(10 + 3600), 10)
            .unwrap()
            .contains("under a day"));
        assert!(expiry_hint(Some(10 + 86400), 10).unwrap().contains("1 day"));
        assert!(expiry_hint(Some(10 + 3 * 86400), 10)
            .unwrap()
            .contains("3 days"));
    }

    #[test]
    fn stats_formatting_and_summary_are_total() {
        assert_eq!(listening_time_text(0), "0m");
        assert_eq!(listening_time_text(-5), "0m");
        assert_eq!(listening_time_text(1), "<1m");
        assert_eq!(listening_time_text(59_999), "<1m");
        assert_eq!(listening_time_text(60_000), "1m");
        assert_eq!(listening_time_text(3_600_000), "1h");
        assert_eq!(listening_time_text(3 * 3_600_000 + 12 * 60_000), "3h 12m");

        assert_eq!(stats_date_text(0), "01 Jan 1970");
        assert_eq!(stats_date_text(86_400_000), "02 Jan 1970");
        assert_eq!(stats_date_text(-1), "31 Dec 1969");
        assert_eq!(month_short(1), "Jan");
        assert_eq!(month_short(12), "Dec");
        assert_eq!(month_short(0), "?");
        assert_eq!(month_short(99), "?");

        assert_eq!(stats_period_label(StatsPeriod::Week), "Last 7 days");
        assert_eq!(stats_period_label(StatsPeriod::Month), "Last month");
        assert_eq!(stats_period_label(StatsPeriod::Year), "This year");
        assert_eq!(stats_period_label(StatsPeriod::All), "All time");

        let report = StatsReport {
            summary: StatsSummary {
                event_count: 3,
                total_ms: 3_720_000,
                longest_streak_days: 2,
                unique_songs: 2,
                unique_artists: 1,
                unique_albums: 1,
                first_play_ms: Some(0),
                last_play_ms: Some(86_400_000),
                ..StatsSummary::default()
            },
            ..StatsReport::default()
        };
        let summary = wrapped_summary(&report, StatsPeriod::Week);
        assert!(summary.contains("Metrolist Wrapped — Last 7 days"));
        assert!(summary.contains("1h 2m"));
        assert!(summary.contains("3 plays"));
        assert!(summary.contains("Top song: —"));
        assert!(summary.contains("01 Jan 1970"));
        let cards = stats_summary_cards(&report.summary);
        assert_eq!(cards.len(), 8);
        assert_eq!(cards[0].0, "Plays");
        assert_eq!(cards[0].1, "3");
        assert_eq!(cards[1].1, "1h 2m");
    }

    #[test]
    fn recognition_helpers_are_total() {
        use ml_recognition::RecognitionError;

        assert_eq!(recognition_progress(0, 12_000), 0.0);
        assert_eq!(recognition_progress(6_000, 12_000), 0.5);
        assert_eq!(recognition_progress(99_000, 12_000), 1.0);
        assert_eq!(recognition_progress(1, 0), 1.0);
        assert_eq!(recognition_timer_text(4_200, 12_000), "4.2s / 12.0s");
        assert_eq!(
            recognition_timing_text(12_000, 1_400),
            "12.0s sample · matched in 1.4s"
        );

        let mut result = RecognitionResult {
            title: "  Diamond Sword  ".to_string(),
            artist: "Minecraft Mix".to_string(),
            album: Some("Album".to_string()),
            genre: Some("Pop".to_string()),
            release_date: Some("2024".to_string()),
            label: Some("Label".to_string()),
            youtube_video_id: Some("video12345".to_string()),
            artwork_url: Some("https://img/cover".to_string()),
            shazam_url: Some("https://shazam/track".to_string()),
            ..RecognitionResult::default()
        };
        assert_eq!(recognition_subtitle(&result), "Minecraft Mix · Album");
        result.album = None;
        assert_eq!(recognition_subtitle(&result), "Minecraft Mix");
        result.artist.clear();
        assert_eq!(recognition_subtitle(&result), "Unknown artist");
        result.artist = "Minecraft Mix".to_string();
        result.album = Some("Album".to_string());
        assert_eq!(recognition_meta(&result), "Pop · 2024 · Label");
        assert_eq!(
            recognition_search_query(&result),
            "Diamond Sword Minecraft Mix"
        );
        let url = recognition_youtube_music_url("Diamond Sword & More");
        assert_eq!(
            url,
            "https://music.youtube.com/search?q=Diamond%20Sword%20%26%20More"
        );
        assert!(recognition_copy_text(&result).contains("https://shazam/track"));

        let songs = vec![
            Song {
                id: "other".to_string(),
                title: "Other".to_string(),
                ..Song::default()
            },
            Song {
                id: "video12345".to_string(),
                title: "Diamond Sword".to_string(),
                artists: vec![Artist::new("Minecraft Mix", None)],
                ..Song::default()
            },
        ];
        assert_eq!(best_recognition_match(&result, &songs), Some(1));
        assert_eq!(best_recognition_match(&result, &[]), None);
        let fallback = recognition_fallback_song(&result).expect("fallback song");
        assert_eq!(fallback.id, "video12345");
        assert_eq!(fallback.artist_names(), "Minecraft Mix");
        result.youtube_video_id = None;
        assert!(recognition_fallback_song(&result).is_none());

        assert!(recognition_error_message(&RecognitionError::NoMatch).contains("No match"));
        assert!(
            recognition_error_message(&RecognitionError::NoInputDevice("none".into()))
                .contains("No microphone")
        );
        assert!(recognition_error_message(&RecognitionError::RateLimited).contains("rate limited"));

        assert_eq!(played_duration_ms(30_000, 240_000), 30_000);
        assert_eq!(played_duration_ms(300_000, 240_000), 240_000);
        assert_eq!(played_duration_ms(30_000, 0), 30_000);
    }

    #[test]
    fn lastfm_connect_gate_and_secret_redaction() {
        use LastFmConnectMode::*;
        assert_eq!(
            lastfm_connect_mode("key", "secret", "user", "pass", ""),
            Authenticate
        );
        assert_eq!(
            lastfm_connect_mode("key", "secret", "", "", "session"),
            AcceptSession
        );
        assert_eq!(lastfm_connect_mode("", "", "", "", "session"), Missing);
        assert_eq!(lastfm_connect_mode("key", "", "", "", "session"), Missing);
        assert_eq!(lastfm_connect_mode("key", "", "user", "", ""), Missing);
        assert_eq!(lastfm_connect_mode("", "", "", "", ""), Missing);
        assert_eq!(
            redact_secrets(
                "auth failed for key-123 / secret-456 / session-789",
                &["key-123", "secret-456", "session-789"]
            ),
            "auth failed for [redacted] / [redacted] / [redacted]"
        );
        assert_eq!(redact_secrets("nothing", &["", "  "]), "nothing");
    }

    #[test]
    fn lyrics_offset_shifts_clamps_and_keeps_order() {
        use ml_core::{LyricsDocument, LyricsLine, LyricsWord};
        use ml_lyrics::apply_offset;

        assert_eq!(lyrics_offset_after(0, 500), 500);
        assert_eq!(lyrics_offset_after(2_900, 500), 3_000);
        assert_eq!(lyrics_offset_after(-2_900, -500), -3_000);
        assert_eq!(lyrics_offset_label(0), "0.0 s");
        assert_eq!(lyrics_offset_label(500), "+0.5 s");
        assert_eq!(lyrics_offset_label(-500), "-0.5 s");
        assert!(translation_endpoint_valid("https://llm.example/v1"));
        assert!(translation_endpoint_valid("http://127.0.0.1:8080/v1"));
        assert!(!translation_endpoint_valid(""));
        assert!(!translation_endpoint_valid("llm.example"));
        assert!(TRANSLATION_MISSING_KEY_MESSAGE.contains("API key"));

        let mut doc = LyricsDocument {
            provider: "fixture".to_string(),
            lines: vec![
                LyricsLine {
                    start_ms: 1_000,
                    end_ms: Some(2_000),
                    text: "one".to_string(),
                    words: vec![LyricsWord {
                        start_ms: 1_000,
                        end_ms: 2_000,
                        text: "one".to_string(),
                    }],
                    ..LyricsLine::default()
                },
                LyricsLine {
                    start_ms: 2_000,
                    end_ms: Some(3_000),
                    text: "two".to_string(),
                    ..LyricsLine::default()
                },
            ],
            ..LyricsDocument::default()
        };
        apply_offset(&mut doc, -1_500);
        assert_eq!(doc.lines[0].start_ms, 0);
        assert_eq!(doc.lines[0].end_ms, Some(500));
        assert_eq!(doc.lines[0].words[0].start_ms, 0);
        assert_eq!(doc.lines[1].start_ms, 500);
        assert!(doc.lines[0].start_ms <= doc.lines[1].start_ms);
        apply_offset(&mut doc, 500);
        assert_eq!(doc.lines[0].start_ms, 500);
        assert_eq!(doc.lines[1].start_ms, 1_000);
    }

    #[test]
    fn back_stack_is_total() {
        let mut stack = Vec::new();
        assert_eq!(back_route(&mut stack), None);
        stack.push(Route::Home);
        stack.push(Route::Search);
        assert_eq!(back_route(&mut stack), Some(Route::Search));
        assert_eq!(back_route(&mut stack), Some(Route::Home));
        assert_eq!(back_route(&mut stack), None);
    }
}
