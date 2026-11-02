//! `player` response parsing, format selection and playback tracking.

use ml_core::{AudioQuality, MediaInfo, MlError, MlResult, StreamInfo};
use serde_json::{json, Value};

use crate::error::playability_error;
use crate::parse::thumbnail_url;

/// One audio format from `streamingData`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioFormat {
    pub itag: u32,
    pub url: Option<String>,
    pub mime: String,
    pub bitrate: u32,
    pub content_length: Option<u64>,
    pub audio_quality: Option<String>,
    pub approx_duration_ms: Option<u64>,
    /// Auto-dubbed tracks are alternates; upstream only plays the original.
    pub auto_dubbed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerData {
    pub formats: Vec<AudioFormat>,
    pub expires_in_seconds: i64,
    pub media: MediaInfo,
    pub playback_url: Option<String>,
    /// `requiresPurchase` and similar; surfaced verbatim on failure.
    pub playability_reason: Option<String>,
}

/// InnerTube sends sizes/durations as JSON strings in some client responses.
fn number(value: &Value, key: &str) -> Option<u64> {
    let field = value.get(key)?;
    field
        .as_u64()
        .or_else(|| field.as_str().and_then(|text| text.parse().ok()))
}

/// Request shape measured by the POT probe for VISIONOS.
pub fn player_body(video_id: &str) -> Value {
    json!({ "videoId": video_id, "contentCheckOk": true, "racyCheckOk": true })
}

/// Parse a successful `player` response; non-OK playability becomes an error.
pub fn parse_player(value: &Value) -> MlResult<PlayerData> {
    let status = value
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
        .unwrap_or("OK");
    let reason = value
        .pointer("/playabilityStatus/reason")
        .and_then(Value::as_str);
    if status != "OK" {
        return Err(playability_error(status, reason));
    }

    let mut formats = Vec::new();
    for key in ["adaptiveFormats", "formats"] {
        let Some(list) = value
            .pointer(&format!("/streamingData/{key}"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for format in list {
            let mime = format
                .get("mimeType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !mime.starts_with("audio/") {
                continue;
            }
            let Some(itag) = format.get("itag").and_then(Value::as_u64) else {
                continue;
            };
            formats.push(AudioFormat {
                itag: itag as u32,
                url: format
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                mime: mime.to_string(),
                bitrate: number(format, "bitrate").unwrap_or(0) as u32,
                content_length: number(format, "contentLength"),
                audio_quality: format
                    .get("audioQuality")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                approx_duration_ms: number(format, "approxDurationMs"),
                auto_dubbed: format
                    .pointer("/audioTrack/isAutoDubbed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }
    }
    if formats.is_empty() {
        return Err(MlError::Unavailable(
            "player response carried no audio formats".to_string(),
        ));
    }

    let details = value.get("videoDetails");
    let media = MediaInfo {
        video_id: details
            .and_then(|d| d.get("videoId"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title: details
            .and_then(|d| d.get("title"))
            .and_then(Value::as_str)
            .map(str::to_string),
        author: details
            .and_then(|d| d.get("author"))
            .and_then(Value::as_str)
            .map(str::to_string),
        author_id: details
            .and_then(|d| d.get("channelId"))
            .and_then(Value::as_str)
            .map(str::to_string),
        author_thumbnail: None,
        description: details
            .and_then(|d| d.get("shortDescription"))
            .and_then(Value::as_str)
            .map(str::to_string),
        upload_date: None,
        subscribers: None,
        view_count: details
            .and_then(|d| d.get("viewCount"))
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok()),
        like: None,
    };

    Ok(PlayerData {
        formats,
        expires_in_seconds: value
            .pointer("/streamingData/expiresInSeconds")
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str()?.parse::<i64>().ok())
            })
            .unwrap_or(21540),
        playback_url: value
            .pointer("/playbackTracking/videostatsPlaybackUrl/baseUrl")
            .and_then(Value::as_str)
            .map(str::to_string),
        playability_reason: reason.map(str::to_string),
        media,
    })
}

fn playable(formats: &[AudioFormat]) -> impl Iterator<Item = &AudioFormat> {
    formats.iter().filter(|format| {
        !format.auto_dubbed && format.url.as_deref().is_some_and(|url| !url.is_empty())
    })
}

/// Deterministic measured preference: Opus 251, then AAC 140, considering
/// only formats with a direct URL that are not auto-dubbed. Never guesses an
/// absent itag.
pub fn pick_itag(formats: &[AudioFormat]) -> Option<u32> {
    [251u32, 140]
        .into_iter()
        .find(|itag| playable(formats).any(|format| format.itag == *itag))
}

/// Pick one format. `Auto`/`High` use [`pick_itag`] then the highest bitrate;
/// `Low` prefers itag 139, then the lowest bitrate.
pub fn select_format(formats: &[AudioFormat], quality: AudioQuality) -> Option<&AudioFormat> {
    let pick_itag = |itag: u32| playable(formats).find(|format| format.itag == itag);
    match quality {
        AudioQuality::Auto | AudioQuality::High => pick_itag(251)
            .or_else(|| pick_itag(140))
            .or_else(|| {
                super::player::pick_itag(formats)
                    .and_then(|itag| playable(formats).find(|format| format.itag == itag))
            })
            .or_else(|| playable(formats).max_by_key(|format| format.bitrate)),
        AudioQuality::Low => {
            pick_itag(139).or_else(|| playable(formats).min_by_key(|format| format.bitrate))
        }
    }
}

/// Build `StreamInfo`, appending the GVS PO token as `pot=` (yt-dlp does the same).
pub fn stream_info(
    format: &AudioFormat,
    expires_in_seconds: i64,
    now_epoch_secs: i64,
    gvs_token: Option<&str>,
) -> MlResult<StreamInfo> {
    let mut url = format
        .url
        .clone()
        .ok_or_else(|| MlError::Unavailable("selected format has no direct url".to_string()))?;
    if let Some(token) = gvs_token.filter(|token| !token.is_empty()) {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str("pot=");
        url.push_str(token);
    }
    Ok(StreamInfo {
        url,
        mime: format.mime.clone(),
        bitrate: (format.bitrate != 0).then_some(format.bitrate),
        itag: format.itag,
        content_length: format.content_length,
        expires_at: Some(now_epoch_secs.saturating_add(expires_in_seconds.max(0))),
    })
}

/// Only `https://s.youtube.com:443/api/stats/playback` is trusted
/// (`PlaybackRegistrationTest`); anything else is refused before a request.
pub fn validate_tracking_url(raw: &str) -> MlResult<reqwest::Url> {
    let url = reqwest::Url::parse(raw)
        .map_err(|_| MlError::Parse("invalid playback tracking URL".to_string()))?;
    let scheme_ok = url.scheme() == "https";
    let host_ok = url.host_str() == Some("s.youtube.com");
    let port_ok = matches!(url.port(), None | Some(443));
    let path_ok = url.path() == "/api/stats/playback";
    let no_userinfo = url.username().is_empty() && url.password().is_none();
    if scheme_ok && host_ok && port_ok && path_ok && no_userinfo {
        Ok(url)
    } else {
        Err(MlError::Parse(format!(
            "untrusted playback tracking URL: {raw}"
        )))
    }
}

/// Largest thumbnail from `videoDetails.thumbnail`.
pub fn video_thumbnail(value: &Value) -> Option<String> {
    thumbnail_url(value.pointer("/videoDetails/thumbnail"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(itag: u32, bitrate: u32, mime: &str) -> AudioFormat {
        AudioFormat {
            itag,
            url: Some(format!(
                "https://rr1.googlevideo.com/videoplayback?itag={itag}"
            )),
            mime: mime.to_string(),
            bitrate,
            content_length: Some(1000),
            ..AudioFormat::default()
        }
    }

    #[test]
    fn player_body_asks_for_content_and_racy_checks() {
        let body = player_body("v");
        assert_eq!(body["videoId"], "v");
        assert_eq!(body["contentCheckOk"], true);
        assert_eq!(body["racyCheckOk"], true);
    }

    #[test]
    fn quality_selection_prefers_measured_itags() {
        let formats = vec![
            format(139, 48000, "audio/mp4; codecs=\"mp4a.40.5\""),
            format(140, 129000, "audio/mp4; codecs=\"mp4a.40.2\""),
            format(251, 140000, "audio/webm; codecs=\"opus\""),
        ];
        assert_eq!(
            select_format(&formats, AudioQuality::Auto).unwrap().itag,
            251
        );
        assert_eq!(
            select_format(&formats, AudioQuality::High).unwrap().itag,
            251
        );
        assert_eq!(
            select_format(&formats, AudioQuality::Low).unwrap().itag,
            139
        );
        let aac_only = vec![
            format(139, 48000, "audio/mp4"),
            format(140, 129000, "audio/mp4"),
        ];
        assert_eq!(
            select_format(&aac_only, AudioQuality::Auto).unwrap().itag,
            140
        );
        assert_eq!(
            select_format(&aac_only, AudioQuality::High).unwrap().itag,
            140
        );
    }

    #[test]
    fn pick_itag_is_deterministic_and_requires_a_url() {
        let both = vec![
            format(140, 129000, "audio/mp4"),
            format(251, 140000, "audio/webm"),
        ];
        assert_eq!(pick_itag(&both), Some(251));
        assert_eq!(pick_itag(&[format(140, 129000, "audio/mp4")]), Some(140));
        assert_eq!(pick_itag(&[format(139, 48000, "audio/mp4")]), None);
        let mut no_url = format(251, 140000, "audio/webm");
        no_url.url = None;
        assert_eq!(
            pick_itag(&[no_url, format(140, 129000, "audio/mp4")]),
            Some(140)
        );
    }

    #[test]
    fn auto_dubbed_formats_are_skipped() {
        let mut dubbed = format(251, 140000, "audio/webm");
        dubbed.auto_dubbed = true;
        let mut original = format(140, 129000, "audio/mp4");
        original.auto_dubbed = false;
        assert_eq!(
            select_format(&[dubbed, original], AudioQuality::Auto)
                .unwrap()
                .itag,
            140
        );
    }

    #[test]
    fn stream_info_appends_gvs_token_and_expiry() {
        let format = format(251, 140000, "audio/webm");
        let info = stream_info(&format, 21540, 1_800_000_000, Some("GVS")).unwrap();
        assert!(info.url.ends_with("&pot=GVS"));
        assert_eq!(info.expires_at, Some(1_800_000_000 + 21540));
        assert!(info.is_expired(1_800_000_000 + 21540 - 300));
        assert!(!info.is_expired(1_800_000_000 + 100));
    }

    #[test]
    fn tracking_urls_are_allowlisted() {
        assert!(validate_tracking_url("https://s.youtube.com:443/api/stats/playback?x=1").is_ok());
        for raw in [
            "https://evil.example/api/stats/playback",
            "http://s.youtube.com/api/stats/playback",
            "https://s.youtube.com:8443/api/stats/playback",
            "https://user:pass@s.youtube.com/api/stats/playback",
            "https://s.youtube.com/api/stats/watchtime",
        ] {
            assert!(
                validate_tracking_url(raw).is_err(),
                "{raw} should be rejected"
            );
        }
    }
}
