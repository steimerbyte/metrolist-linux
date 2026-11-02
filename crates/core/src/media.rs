//! Metadata and resolved stream types shared by `ml-innertube` and `ml-audio`.

use serde::{Deserialize, Serialize};

/// Video-level metadata from the InnerTube `player` endpoint.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub video_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub author_id: Option<String>,
    pub author_thumbnail: Option<String>,
    pub description: Option<String>,
    pub upload_date: Option<String>,
    pub subscribers: Option<String>,
    pub view_count: Option<u64>,
    pub like: Option<u64>,
}

/// One playable audio format.
///
/// `expires_at` is epoch seconds (`expiresInSeconds` added to mint time).
/// URLs must be re-minted after this instant minus a 300 s safety margin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamInfo {
    pub url: String,
    pub mime: String,
    pub bitrate: Option<u32>,
    pub itag: u32,
    pub content_length: Option<u64>,
    pub expires_at: Option<i64>,
}

impl StreamInfo {
    pub fn is_expired(&self, now_epoch_secs: i64) -> bool {
        self.expires_at
            .map(|expires| now_epoch_secs >= expires.saturating_sub(300))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(expires_at: Option<i64>) -> StreamInfo {
        StreamInfo {
            expires_at,
            ..StreamInfo::default()
        }
    }

    #[test]
    fn expiry_saturates_at_zero_and_respects_the_safety_margin() {
        assert!(stream(Some(0)).is_expired(0));
        assert!(stream(Some(299)).is_expired(0));
        assert!(stream(Some(300)).is_expired(0));
        assert!(!stream(Some(301)).is_expired(0));
        assert!(stream(Some(301)).is_expired(1));
        assert!(!stream(None).is_expired(0));
    }
}
