//! Stream URL cache and ranged googlevideo transfers.
//!
//! URLs are cached by `(videoId, quality)` with the core `expires_at`/300 s
//! safety margin. Bytes are always fetched in ≤512 KiB ranges and every
//! response must be 206 (`arch/pot-notes.md` §2); an unbounded GET is what the
//! capped clients reject after 512 KiB/1 MiB.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use ml_core::{AudioQuality, MlError, MlResult, StreamInfo};

/// Measured safe ceiling per ranged request (VISIONOS proof used 512 KiB).
pub const RANGE_CHUNK: u64 = 512 * 1024;

/// Half-open byte ranges `[start, end]` into `0..total`, contiguous and
/// covering every byte once. `chunk == 0` or `total == 0` yields no ranges;
/// arithmetic is checked so `total == u64::MAX` cannot overflow.
pub fn chunk_ranges(total: u64, chunk: u64) -> Vec<(u64, u64)> {
    let mut ranges = Vec::new();
    if total == 0 || chunk == 0 {
        return ranges;
    }
    let mut offset = 0u64;
    while offset < total {
        let end = offset.saturating_add(chunk - 1).min(total - 1);
        ranges.push((offset, end));
        offset = end.saturating_add(1);
    }
    ranges
}

/// Cache deadline: `now + (expires_in - margin)`, or `None` when the whole TTL
/// is inside the safety margin. `checked_add` makes it total and panic-free.
pub fn expiry(now: SystemTime, expires_in: u64, margin: u64) -> Option<SystemTime> {
    if expires_in <= margin {
        return None;
    }
    now.checked_add(Duration::from_secs(expires_in - margin))
}

/// Parse `Content-Range` in its 206 form `bytes START-END/TOTAL` (or `*` total).
///
/// Returns `None` for malformed input, `START > END`, or `END >= TOTAL`; it
/// never panics or guesses a missing total.
pub fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let (unit, rest) = value.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.trim().parse::<u64>().ok()?;
    let end = end.trim().parse::<u64>().ok()?;
    if start > end {
        return None;
    }
    let total = match total.trim() {
        "*" => None,
        total => Some(total.parse::<u64>().ok()?),
    };
    if let Some(total) = total {
        if end >= total {
            return None;
        }
    }
    Some((start, end, total))
}

pub(crate) type StreamKey = (String, &'static str);
pub(crate) type StreamCache = Mutex<HashMap<StreamKey, StreamInfo>>;

pub(crate) fn quality_key(quality: AudioQuality) -> &'static str {
    match quality {
        AudioQuality::Auto => "auto",
        AudioQuality::Low => "low",
        AudioQuality::High => "high",
    }
}

fn key(video_id: &str, quality: AudioQuality) -> StreamKey {
    (video_id.to_string(), quality_key(quality))
}

/// Cached URL if it has not entered the 300 s expiry window.
pub(crate) fn cached(
    cache: &StreamCache,
    video_id: &str,
    quality: AudioQuality,
    now_epoch_secs: i64,
) -> Option<StreamInfo> {
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = key(video_id, quality);
    match cache.get(&key) {
        Some(info) if !info.is_expired(now_epoch_secs) => Some(info.clone()),
        Some(_) => {
            cache.remove(&key);
            None
        }
        None => None,
    }
}

pub(crate) fn store(cache: &StreamCache, video_id: &str, quality: AudioQuality, info: StreamInfo) {
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key(video_id, quality), info);
}

pub(crate) fn invalidate(cache: &StreamCache, video_id: &str, quality: AudioQuality) {
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&key(video_id, quality));
}

pub(crate) fn clear(cache: &StreamCache) {
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

/// Total size from `Content-Range` / `Content-Length`.
///
/// `206` requires the anchored `bytes 0-0/4096` form (the requested start must
/// be 0); `416` uses `bytes */4096`; `200` uses `Content-Length`. Anything
/// else is unknown, never a guess derived from the chunk we received.
pub fn content_total(
    status: u16,
    content_range: Option<&str>,
    content_length: Option<u64>,
) -> Option<u64> {
    let range_total = || {
        let value = content_range?.trim();
        let (unit, rest) = value.split_once(' ')?;
        if !unit.eq_ignore_ascii_case("bytes") {
            return None;
        }
        let (range, total) = rest.split_once('/')?;
        if range.starts_with('*') {
            return total.trim().parse::<u64>().ok().filter(|total| *total > 0);
        }
        let (start, end) = range.split_once('-')?;
        if start.trim() != "0" || end.trim().is_empty() {
            return None;
        }
        total.trim().parse::<u64>().ok().filter(|total| *total > 0)
    };
    match status {
        206 | 416 => range_total(),
        200 => content_length.filter(|length| *length > 0),
        _ => None,
    }
}

/// Lenient `Content-Range` total for real 206 responses
/// (`bytes 0-786431/3433755`). The strict parity helper is [`content_total`].
pub fn parse_content_range_total(value: &str) -> Option<u64> {
    let (unit, rest) = value.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (_range, total) = rest.split_once('/')?;
    total.trim().parse::<u64>().ok().filter(|total| *total > 0)
}

/// Issue one ranged GET. The caller streams the body and handles 403 re-mints.
pub(crate) async fn send_range(
    http: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
) -> MlResult<reqwest::Response> {
    let response = http
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
        .header(reqwest::header::ACCEPT, "*/*")
        .send()
        .await
        .map_err(|error| MlError::Network(error.to_string()))?;
    Ok(response)
}

/// Re-mint signal used by the download loop.
pub(crate) fn is_auth_failure(status: u16) -> bool {
    matches!(status, 403 | 410)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn cache_returns_fresh_and_drops_expired() {
        let cache = Arc::new(StreamCache::default());
        let info = StreamInfo {
            url: "https://example.invalid/a".into(),
            mime: "audio/webm".into(),
            bitrate: Some(140_000),
            itag: 251,
            content_length: Some(1),
            expires_at: Some(1000),
        };
        assert!(cached(&cache, "v", AudioQuality::Auto, 100).is_none());
        store(&cache, "v", AudioQuality::Auto, info.clone());
        assert_eq!(cached(&cache, "v", AudioQuality::Auto, 699), Some(info));
        // inclusive boundary: is_expired uses now >= expires - 300
        assert!(cached(&cache, "v", AudioQuality::Auto, 700).is_none());
        invalidate(&cache, "v", AudioQuality::Auto);
        assert!(cached(&cache, "v", AudioQuality::Auto, 0).is_none());
        store(&cache, "v", AudioQuality::Low, StreamInfo::default());
        clear(&cache);
        assert!(cached(&cache, "v", AudioQuality::Low, 0).is_none());
    }

    #[test]
    fn content_total_matches_upstream_rules() {
        assert_eq!(
            content_total(206, Some("bytes 0-0/4096"), Some(1)),
            Some(4096)
        );
        assert_eq!(content_total(200, None, Some(4096)), Some(4096));
        assert_eq!(content_total(206, None, Some(1)), None);
        assert_eq!(content_total(416, Some("bytes */4096"), None), Some(4096));
        for (status, range, length) in [
            (403, None, Some(1)),
            (200, None, Some(0)),
            (416, Some("bytes 0-0/0"), None),
            (206, Some("bytes 1-1/4096"), None),
            (416, Some("bytes 1-1/4096"), None),
            (206, Some("items 0-0/4096"), None),
        ] {
            assert_eq!(
                content_total(status, range, length),
                None,
                "{status} {range:?}"
            );
        }
    }

    #[test]
    fn chunk_ranges_cover_the_whole_resource_once() {
        assert!(chunk_ranges(0, 512).is_empty());
        assert!(chunk_ranges(10, 0).is_empty());
        assert_eq!(chunk_ranges(1, 512), vec![(0, 0)]);
        assert_eq!(chunk_ranges(512, 512), vec![(0, 511)]);
        assert_eq!(chunk_ranges(513, 512), vec![(0, 511), (512, 512)]);
        assert_eq!(chunk_ranges(1024, 512), vec![(0, 511), (512, 1023)]);
        let ranges = chunk_ranges(4_605_080, 524_288);
        assert_eq!(ranges.first(), Some(&(0, 524_287)));
        assert_eq!(ranges.last().map(|(_, end)| *end), Some(4_605_079));
        assert_eq!(
            ranges
                .iter()
                .map(|(start, end)| end - start + 1)
                .sum::<u64>(),
            4_605_080
        );
        assert!(ranges.windows(2).all(|pair| pair[0].1 + 1 == pair[1].0));
        assert_eq!(chunk_ranges(u64::MAX, u64::MAX), vec![(0, u64::MAX - 1)]);
    }

    #[test]
    fn expiry_respects_the_safety_margin() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        assert_eq!(expiry(now, 300, 300), None);
        assert_eq!(expiry(now, 200, 300), None);
        assert_eq!(
            expiry(now, 21_540, 300),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000 + 21_540 - 300))
        );
    }

    #[test]
    fn content_range_parser_rejects_malformed_and_inconsistent_ranges() {
        assert_eq!(
            parse_content_range("bytes 0-524287/4605080"),
            Some((0, 524_287, Some(4_605_080)))
        );
        assert_eq!(
            parse_content_range("BYTES 524288-1048575/4605080"),
            Some((524_288, 1_048_575, Some(4_605_080)))
        );
        assert_eq!(parse_content_range("bytes 0-0/*"), Some((0, 0, None)));
        for bad in [
            "",
            "bytes",
            "bytes */4605080",
            "bytes 5-4/10",
            "bytes 0-10/10",
            "bytes -1/10",
            "items 0-1/2",
            "bytes 0-1",
        ] {
            assert_eq!(parse_content_range(bad), None, "{bad:?}");
        }
    }
}
