//! Small pure helpers shared by every crate.

use std::time::{SystemTime, UNIX_EPOCH};

use url::Url;

/// `m:ss` below one hour, `h:mm:ss` above. YouTube durations are seconds.
pub fn format_duration(total_seconds: u32) -> String {
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// UI placeholder for unknown durations.
pub fn format_duration_opt(duration: Option<u32>) -> String {
    duration
        .map(format_duration)
        .unwrap_or_else(|| "--:--".to_string())
}

/// Format a player position (milliseconds) with the same rules.
pub fn format_position(position_ms: u64) -> String {
    format_duration((position_ms / 1000).min(u32::MAX as u64) as u32)
}

/// YouTube thumbnail URL at the closest server-side size bucket.
pub fn thumbnail_url(video_id: &str, size: u32) -> String {
    let file = match size {
        0..=120 => "default.jpg",
        121..=320 => "mqdefault.jpg",
        321..=480 => "hqdefault.jpg",
        481..=640 => "sddefault.jpg",
        _ => "maxresdefault.jpg",
    };
    format!("https://i.ytimg.com/vi/{video_id}/{file}")
}

/// Rewrite a ytimg/ggpht thumbnail URL to roughly `size` pixels.
/// Unknown hosts are returned unchanged.
pub fn resize_thumbnail_url(url: &str, size: u32) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return url.to_string();
    };
    let host = parsed.host_str().unwrap_or_default();
    if host.ends_with("ytimg.com") {
        if let Some(id) = parsed
            .path_segments()
            .into_iter()
            .flatten()
            .find(|segment| is_video_id(segment))
        {
            return thumbnail_url(id, size);
        }
    }
    if host.ends_with("googleusercontent.com") || host.ends_with("ggpht.com") {
        // Google photo URLs carry a trailing size directive: `=w540-h540-l90-rj`,
        // `=s540-c`, `=w1200`. Replace only a directive-looking suffix.
        if let Some((base, suffix)) = url.rsplit_once('=') {
            if suffix
                .chars()
                .next()
                .is_some_and(|c| matches!(c, 'w' | 'h' | 's' | 'p' | 'c' | 'n' | 'd'))
            {
                return format!("{base}=w{size}-h{size}-l90-rj");
            }
        }
        return format!("{url}=w{size}-h{size}-l90-rj");
    }
    url.to_string()
}

/// Accepts a bare 11-character video id or a YouTube watch/short/embed URL.
pub fn parse_video_id(input: &str) -> Option<String> {
    let input = input.trim();
    if is_video_id(input) {
        return Some(input.to_string());
    }
    let with_scheme = if input.starts_with("http://") || input.starts_with("https://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let url = Url::parse(&with_scheme).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host
        .trim_start_matches("www.")
        .trim_start_matches("music.")
        .trim_start_matches("m.");
    let candidate = match host {
        "youtu.be" => url
            .path_segments()
            .and_then(|mut segments| segments.next())
            .map(str::to_string),
        "youtube.com" | "youtube-nocookie.com" => {
            if let Some((_, value)) = url.query_pairs().find(|(key, _)| key == "v") {
                Some(value.to_string())
            } else {
                let mut segments = url.path_segments()?;
                match segments.next()? {
                    "shorts" | "embed" | "live" | "v" => segments.next().map(str::to_string),
                    _ => None,
                }
            }
        }
        _ => None,
    }?;
    is_video_id(&candidate).then_some(candidate)
}

/// YouTube ids are exactly 11 URL-safe base64 characters.
pub fn is_video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Current wall-clock time as epoch milliseconds.
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn millis_to_seconds(millis: u64) -> u64 {
    millis / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(9), "0:09");
        assert_eq!(format_duration(257), "4:17");
        assert_eq!(format_duration(3600), "1:00:00");
        assert_eq!(format_duration(7322), "2:02:02");
        assert_eq!(format_duration_opt(None), "--:--");
        assert_eq!(format_position(257_999), "4:17");
    }

    #[test]
    fn parses_video_ids() {
        let id = "dQw4w9WgXcQ";
        assert_eq!(parse_video_id(id).as_deref(), Some(id));
        assert_eq!(
            parse_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10s").as_deref(),
            Some(id)
        );
        assert_eq!(
            parse_video_id("https://music.youtube.com/watch?v=dQw4w9WgXcQ").as_deref(),
            Some(id)
        );
        assert_eq!(
            parse_video_id("https://youtu.be/dQw4w9WgXcQ?si=x").as_deref(),
            Some(id)
        );
        assert_eq!(
            parse_video_id("youtube.com/shorts/dQw4w9WgXcQ").as_deref(),
            Some(id)
        );
        assert_eq!(
            parse_video_id("https://example.com/watch?v=dQw4w9WgXcQ"),
            None
        );
        assert_eq!(parse_video_id("not-an-id"), None);
    }

    #[test]
    fn thumbnail_urls() {
        assert_eq!(
            thumbnail_url("dQw4w9WgXcQ", 120),
            "https://i.ytimg.com/vi/dQw4w9WgXcQ/default.jpg"
        );
        assert_eq!(
            resize_thumbnail_url("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg", 320),
            "https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg"
        );
        assert_eq!(
            resize_thumbnail_url(
                "https://lh3.googleusercontent.com/abc=w540-h540-l90-rj",
                120
            ),
            "https://lh3.googleusercontent.com/abc=w120-h120-l90-rj"
        );
        assert_eq!(
            resize_thumbnail_url("https://example.com/a.jpg", 120),
            "https://example.com/a.jpg"
        );
    }
}
