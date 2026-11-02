//! Shazam discovery API client.
//!
//! The request is the one `com.metrolist.shazamkit.Shazam` sends from the
//! Android app: the signature URI is posted as JSON to
//! `https://amp.shazam.com/discovery/v5/en/US/android/-/tag/<uuid>/<uuid>` with
//! Android Dalvik user agents, one request per second, and up to three attempts
//! when Shazam answers HTTP 429.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::RecognitionError;

/// Shazam discovery host.
pub const ENDPOINT: &str = "https://amp.shazam.com";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(1000);
const MAX_ATTEMPTS: usize = 3;
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(2000);

const USER_AGENTS: [&str; 5] = [
    "Dalvik/2.1.0 (Linux; U; Android 5.0.2; VS980 4G Build/LRX22G)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SM-T210 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-P905V Build/LMY47X)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G920F Build/MMB29K)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; SM-G900F Build/LRX21T)",
];

const TIMEZONES: [&str; 6] = [
    "Europe/Paris",
    "Europe/London",
    "America/New_York",
    "America/Los_Angeles",
    "Asia/Tokyo",
    "Asia/Dubai",
];

/// A successful match, mirroring the Android `RecognitionResult`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecognitionResult {
    /// Shazam track key, or the tag id when the track has no key.
    pub track_key: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub artwork_url: Option<String>,
    pub artwork_hq_url: Option<String>,
    pub genre: Option<String>,
    pub release_date: Option<String>,
    pub label: Option<String>,
    /// Synced lyric lines, when Shazam returns a `LYRICS` section.
    pub lyrics: Option<Vec<String>>,
    pub shazam_url: Option<String>,
    pub apple_music_url: Option<String>,
    pub spotify_url: Option<String>,
    pub isrc: Option<String>,
    pub youtube_video_id: Option<String>,
}

/// Client for Shazam's discovery endpoint.
pub struct ShazamClient {
    http: reqwest::Client,
    base_url: String,
    last_request: Mutex<Option<Instant>>,
}

impl ShazamClient {
    /// Client pointing at the production endpoint.
    pub fn new() -> Self {
        Self::with_base_url(ENDPOINT)
    }

    /// Client pointing at another host (tests, proxies).
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("reqwest client construction cannot fail"),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            last_request: Mutex::new(None),
        }
    }

    /// Recognise the fingerprint and return the matched track.
    ///
    /// Retries HTTP 429 with exponential backoff (2 s, 4 s) like the Android
    /// client; every other failure is returned immediately.
    pub async fn recognize(
        &self,
        signature: &str,
        sample_duration_ms: u64,
    ) -> Result<RecognitionResult, RecognitionError> {
        for attempt in 0..MAX_ATTEMPTS {
            self.enforce_rate_limit().await;
            match self.perform(signature, sample_duration_ms).await {
                Ok(result) => return Ok(result),
                Err(RecognitionError::RateLimited) if attempt + 1 < MAX_ATTEMPTS => {
                    tokio::time::sleep(INITIAL_RETRY_DELAY * (1u32 << attempt)).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(RecognitionError::RateLimited)
    }

    async fn perform(
        &self,
        signature: &str,
        sample_duration_ms: u64,
    ) -> Result<RecognitionResult, RecognitionError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let tag_id = uuid_v4(true);
        let request_id = uuid_v4(false);
        let url = format!(
            "{}/discovery/v5/en/US/android/-/tag/{tag_id}/{request_id}?sync=true&webv3=true&sampling=true&connected=&shazamapiversion=v3&sharehub=true&video=v3",
            self.base_url
        );

        let body = TagRequest {
            geolocation: Geolocation {
                altitude: random_f64() * 400.0 + 100.0,
                latitude: random_f64() * 180.0 - 90.0,
                longitude: random_f64() * 360.0 - 180.0,
            },
            signature: RequestSignature {
                samplems: sample_duration_ms as i64,
                timestamp,
                uri: signature,
            },
            timestamp,
            timezone: pick(&TIMEZONES),
        };

        let response = self
            .http
            .post(url)
            .header(reqwest::header::USER_AGENT, pick(&USER_AGENTS))
            .header(reqwest::header::CONTENT_LANGUAGE, "en_US")
            .json(&body)
            .send()
            .await
            .map_err(network_error)?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status));
        }

        let payload = response.bytes().await.map_err(network_error)?;
        let tag: TagResponse = serde_json::from_slice(&payload)
            .map_err(|error| RecognitionError::Parse(error.to_string()))?;
        tag.into_result().ok_or(RecognitionError::NoMatch)
    }

    async fn enforce_rate_limit(&self) {
        let wait = {
            let mut last = self
                .last_request
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let wait = (*last).map_or(Duration::ZERO, |previous| {
                MIN_REQUEST_INTERVAL.saturating_sub(previous.elapsed())
            });
            *last = Some(Instant::now());
            wait
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

impl Default for ShazamClient {
    fn default() -> Self {
        Self::new()
    }
}

fn network_error(error: reqwest::Error) -> RecognitionError {
    if error.is_timeout() {
        RecognitionError::Timeout
    } else {
        RecognitionError::Network(error.to_string())
    }
}

fn status_error(status: reqwest::StatusCode) -> RecognitionError {
    match status.as_u16() {
        404 => RecognitionError::NoMatch,
        429 => RecognitionError::RateLimited,
        500..=599 => RecognitionError::Unavailable(format!("HTTP {}", status.as_u16())),
        code => RecognitionError::Http {
            status: code,
            message: status
                .canonical_reason()
                .unwrap_or("unexpected response")
                .to_string(),
        },
    }
}

#[derive(Serialize)]
struct TagRequest<'a> {
    geolocation: Geolocation,
    signature: RequestSignature<'a>,
    timestamp: i64,
    timezone: &'static str,
}

#[derive(Serialize)]
struct Geolocation {
    altitude: f64,
    latitude: f64,
    longitude: f64,
}

#[derive(Serialize)]
struct RequestSignature<'a> {
    samplems: i64,
    timestamp: i64,
    uri: &'a str,
}

#[derive(Deserialize)]
struct TagResponse {
    track: Option<Track>,
    tagid: Option<String>,
}

#[derive(Deserialize)]
struct Track {
    key: Option<String>,
    title: Option<String>,
    subtitle: Option<String>,
    images: Option<Images>,
    sections: Option<Vec<Option<Section>>>,
    url: Option<String>,
    isrc: Option<String>,
    genres: Option<Genres>,
    hub: Option<Hub>,
}

#[derive(Deserialize)]
struct Images {
    coverart: Option<String>,
    coverarthq: Option<String>,
}

#[derive(Deserialize)]
struct Section {
    #[serde(rename = "type")]
    section_type: Option<String>,
    text: Option<Vec<String>>,
    metadata: Option<Vec<Option<Metadata>>>,
}

#[derive(Deserialize)]
struct Metadata {
    title: Option<String>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct Genres {
    primary: Option<String>,
}

#[derive(Deserialize)]
struct Hub {
    options: Option<Vec<Option<HubOption>>>,
    providers: Option<Vec<Option<Provider>>>,
}

#[derive(Deserialize)]
struct HubOption {
    providername: Option<String>,
    #[serde(rename = "type")]
    option_type: Option<String>,
    actions: Option<Vec<Option<Action>>>,
}

#[derive(Deserialize)]
struct Provider {
    caption: Option<String>,
    actions: Option<Vec<Option<Action>>>,
}

#[derive(Deserialize)]
struct Action {
    uri: Option<String>,
}

impl TagResponse {
    /// Same mapping as `ShazamResponseJson.toRecognitionResult()` in the app.
    fn into_result(self) -> Option<RecognitionResult> {
        let track = self.track?;

        let sections = track.sections.unwrap_or_default();
        let song = sections
            .iter()
            .flatten()
            .find(|section| section.section_type.as_deref() == Some("SONG"));
        let metadata: &[Option<Metadata>] = song
            .and_then(|section| section.metadata.as_deref())
            .unwrap_or(&[]);
        let metadata_value = |title: &str| {
            metadata
                .iter()
                .flatten()
                .find(|entry| entry.title.as_deref() == Some(title))
                .and_then(|entry| entry.text.clone())
        };

        let lyrics = sections
            .iter()
            .flatten()
            .find(|section| section.section_type.as_deref() == Some("LYRICS"))
            .and_then(|section| section.text.clone());

        let hub = track.hub.unwrap_or(Hub {
            options: None,
            providers: None,
        });
        let options: &[Option<HubOption>] = hub.options.as_deref().unwrap_or(&[]);
        let providers: &[Option<Provider>] = hub.providers.as_deref().unwrap_or(&[]);

        let apple_music_url = options
            .iter()
            .flatten()
            .find(|option| contains_ignore_case(option.providername.as_deref(), "apple"))
            .and_then(|option| first_action_uri(option.actions.as_deref()));
        let spotify_url = providers
            .iter()
            .flatten()
            .find(|provider| contains_ignore_case(provider.caption.as_deref(), "spotify"))
            .and_then(|provider| first_action_uri(provider.actions.as_deref()));
        let youtube_video_id = options
            .iter()
            .flatten()
            .find(|option| contains_ignore_case(option.option_type.as_deref(), "video"))
            .and_then(|option| first_action_uri(option.actions.as_deref()))
            .and_then(|uri| youtube_id(&uri));

        Some(RecognitionResult {
            track_key: track.key.or(self.tagid).unwrap_or_default(),
            title: track.title.unwrap_or_default(),
            artist: track.subtitle.unwrap_or_default(),
            album: metadata_value("Album"),
            artwork_url: track
                .images
                .as_ref()
                .and_then(|images| images.coverart.clone()),
            artwork_hq_url: track
                .images
                .as_ref()
                .and_then(|images| images.coverarthq.clone()),
            genre: track.genres.and_then(|genres| genres.primary),
            release_date: metadata_value("Released"),
            label: metadata_value("Label"),
            lyrics,
            shazam_url: track.url,
            apple_music_url,
            spotify_url,
            isrc: track.isrc,
            youtube_video_id,
        })
    }
}

fn contains_ignore_case(haystack: Option<&str>, needle: &str) -> bool {
    haystack.is_some_and(|value| value.to_lowercase().contains(needle))
}

fn first_action_uri(actions: Option<&[Option<Action>]>) -> Option<String> {
    actions?
        .iter()
        .flatten()
        .find_map(|action| action.uri.clone())
}

/// Extract a YouTube video id the way the Kotlin does: `v=` first, else an
/// 11-character last path segment.
fn youtube_id(uri: &str) -> Option<String> {
    if let Some((_, query)) = uri.rsplit_once("v=") {
        if !query.is_empty() {
            return Some(query.to_string());
        }
    }
    let last = uri.rsplit('/').next().unwrap_or_default();
    if last.len() == 11 {
        Some(last.to_string())
    } else {
        None
    }
}

/// Non-cryptographic xorshift; only used to vary tag ids, geolocation and
/// timezone the way the app does.
fn random_u64() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    let mut value = counter ^ nanos;
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    value
}

fn random_f64() -> f64 {
    (random_u64() >> 11) as f64 / (1u64 << 53) as f64
}

fn pick(values: &'static [&'static str]) -> &'static str {
    values[(random_u64() % values.len() as u64) as usize]
}

fn uuid_v4(uppercase: bool) -> String {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&random_u64().to_be_bytes());
    bytes[8..].copy_from_slice(&random_u64().to_be_bytes());
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;

    let mut uuid = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            uuid.push('-');
        }
        uuid.push_str(&format!("{byte:02x}"));
    }
    if uppercase {
        uuid.to_uppercase()
    } else {
        uuid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_map_to_typed_errors() {
        assert_eq!(
            status_error(reqwest::StatusCode::NOT_FOUND),
            RecognitionError::NoMatch
        );
        assert_eq!(
            status_error(reqwest::StatusCode::TOO_MANY_REQUESTS),
            RecognitionError::RateLimited
        );
        assert_eq!(
            status_error(reqwest::StatusCode::SERVICE_UNAVAILABLE),
            RecognitionError::Unavailable("HTTP 503".to_string())
        );
        assert_eq!(
            status_error(reqwest::StatusCode::BAD_REQUEST),
            RecognitionError::Http {
                status: 400,
                message: "Bad Request".to_string()
            }
        );
    }

    #[test]
    fn response_maps_every_field_like_the_app() {
        let json = r#"{
            "matches": [{"id": "m1", "offset": 0.5}],
            "timestamp": 1700000000,
            "timezone": "Europe/Paris",
            "tagid": "tag-1",
            "track": {
                "key": "12345",
                "title": "Song Title",
                "subtitle": "Artist Name",
                "images": {
                    "background": "https://img/bg",
                    "coverart": "https://img/cover",
                    "coverarthq": "https://img/cover-hq"
                },
                "url": "https://www.shazam.com/track/12345/song",
                "isrc": "USRC17607839",
                "genres": {"primary": "Pop"},
                "sections": [
                    {
                        "type": "SONG",
                        "metadata": [
                            {"title": "Album", "text": "The Album"},
                            {"title": "Label", "text": "Label Name"},
                            {"title": "Released", "text": "2020-01-01"}
                        ]
                    },
                    {"type": "LYRICS", "text": ["line one", "line two"]}
                ],
                "hub": {
                    "options": [
                        {
                            "providername": "Apple Music",
                            "type": "APPLE",
                            "actions": [{"uri": "https://music.apple.com/track/1"}]
                        },
                        {
                            "providername": "YouTube",
                            "type": "VIDEO",
                            "actions": [{"uri": "https://www.youtube.com/watch?v=dQw4w9WgXcQ"}]
                        }
                    ],
                    "providers": [
                        {
                            "caption": "Spotify",
                            "actions": [{"uri": "https://open.spotify.com/track/abc"}]
                        }
                    ]
                }
            }
        }"#;

        let tag: TagResponse = serde_json::from_str(json).expect("valid response");
        let result = tag.into_result().expect("track");
        assert_eq!(result.track_key, "12345");
        assert_eq!(result.title, "Song Title");
        assert_eq!(result.artist, "Artist Name");
        assert_eq!(result.album.as_deref(), Some("The Album"));
        assert_eq!(result.label.as_deref(), Some("Label Name"));
        assert_eq!(result.release_date.as_deref(), Some("2020-01-01"));
        assert_eq!(result.genre.as_deref(), Some("Pop"));
        assert_eq!(result.artwork_url.as_deref(), Some("https://img/cover"));
        assert_eq!(
            result.artwork_hq_url.as_deref(),
            Some("https://img/cover-hq")
        );
        assert_eq!(result.isrc.as_deref(), Some("USRC17607839"));
        assert_eq!(
            result.lyrics,
            Some(vec!["line one".to_string(), "line two".to_string()])
        );
        assert_eq!(
            result.apple_music_url.as_deref(),
            Some("https://music.apple.com/track/1")
        );
        assert_eq!(
            result.spotify_url.as_deref(),
            Some("https://open.spotify.com/track/abc")
        );
        assert_eq!(result.youtube_video_id.as_deref(), Some("dQw4w9WgXcQ"));
    }

    #[test]
    fn missing_track_is_a_no_match_and_minimal_tracks_parse() {
        let empty: TagResponse = serde_json::from_str(r#"{"matches": []}"#).expect("valid");
        assert!(empty.into_result().is_none());

        let minimal: TagResponse =
            serde_json::from_str(r#"{"tagid": "42", "track": {"title": "T"}}"#).expect("valid");
        let result = minimal.into_result().expect("track");
        assert_eq!(result.track_key, "42");
        assert_eq!(result.title, "T");
        assert_eq!(result.artist, "");
        assert_eq!(result.album, None);
        assert_eq!(result.youtube_video_id, None);
    }

    #[test]
    fn youtube_id_handles_both_uri_shapes() {
        assert_eq!(
            youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            youtube_id("https://youtu.be/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(youtube_id("https://example.com/short"), None);
    }

    #[test]
    fn uuid_is_v4_shaped() {
        let uuid = uuid_v4(false);
        assert_eq!(uuid.len(), 36);
        assert!(uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert_eq!(&uuid[14..15], "4");
        assert!(matches!(&uuid[19..20], "8" | "9" | "a" | "b"));
        assert_eq!(uuid, uuid.to_lowercase());
        let upper = uuid_v4(true);
        assert_eq!(upper, upper.to_uppercase());
    }

    #[test]
    fn request_json_field_names_match_the_kotlin_model() {
        let body = TagRequest {
            geolocation: Geolocation {
                altitude: 1.0,
                latitude: 2.0,
                longitude: 3.0,
            },
            signature: RequestSignature {
                samplems: 12000,
                timestamp: 1700000000,
                uri: "data:audio/vnd.shazam.sig;base64,AA==",
            },
            timestamp: 1700000000,
            timezone: "Europe/Paris",
        };
        let json = serde_json::to_string(&body).expect("serialize");
        assert!(json.starts_with(r#"{"geolocation":{"altitude":1.0,"latitude":2.0,"longitude":3.0},"signature":{"samplems":12000,"timestamp":1700000000,"uri":"data:audio/vnd.shazam.sig;base64,AA=="},"timestamp":1700000000,"timezone":"Europe/Paris"}"#));
    }
}
