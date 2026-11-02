//! Download records, state machine, events and the storage arithmetic.
//!
//! Everything here is total: no panics, no unwraps on user data.
//! `progress_percent`, `occupied_bytes`/`storage_sum`, `clamp_concurrency` and
//! `DownloadState::can_transition` are the pure functions handed to the proof
//! lead (see `VERUS.md` / the final report).

use std::path::Path;

use ml_core::{AudioQuality, Song};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Identifier of one download: the song id (== the YouTube video id).
pub type DownloadId = String;

/// Upper bound of the worker pool (`DownloadConfig::max_concurrent`).
pub const MAX_CONCURRENT: usize = 8;

/// Clamp a requested worker count into `1..=MAX_CONCURRENT`.
///
/// Total for every `usize`: `0` becomes `1`, huge values become [`MAX_CONCURRENT`].
pub fn clamp_concurrency(requested: usize) -> usize {
    requested.clamp(1, MAX_CONCURRENT)
}

/// Integer progress in `0..=100`. `total == 0` (unknown) reports `0`; an
/// overshoot reports `100`.
///
/// The multiply runs in `u128`, so `bytes * 100` cannot overflow for any `u64`.
pub fn progress_percent(bytes: u64, total: u64) -> u8 {
    if total == 0 {
        0
    } else {
        ((bytes as u128 * 100) / total as u128).min(100) as u8
    }
}

/// Lifecycle of one download. Persisted in `manifest.json` as snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Queued,
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

impl DownloadState {
    /// The allowed state machine. `from == to` is always allowed so repeated
    /// signals are idempotent.
    ///
    /// `Completed` is terminal except for `remove`: only a failure to verify can
    /// turn a completed entry into `Queued` (never done today), and `enqueue` /
    /// `retry` refuse to touch it (upstream parity: a completed download is
    /// never prepared again).
    pub fn can_transition(from: Self, to: Self) -> bool {
        use DownloadState::*;
        from == to
            || matches!(
                (from, to),
                (Queued, Downloading)
                    | (Queued, Failed)
                    | (Queued, Cancelled)
                    | (Downloading, Completed)
                    | (Downloading, Failed)
                    | (Downloading, Cancelled)
                    | (Failed, Queued)
                    | (Failed, Cancelled)
                    | (Cancelled, Queued)
            )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

fn default_state() -> DownloadState {
    DownloadState::Queued
}

/// One manifest entry: full song metadata plus transfer bookkeeping, so the
/// offline library renders without the network.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Download {
    pub id: DownloadId,
    pub song: Song,
    #[serde(default = "default_state")]
    pub state: DownloadState,
    /// Final file name inside the download dir; `Some` only once completed.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Final size on disk; `path_for` compares it to detect a truncated file.
    #[serde(default)]
    pub size_bytes: u64,
    /// Bytes received so far (in-flight only; reset by retry).
    #[serde(default)]
    pub bytes_downloaded: u64,
    /// Total reported by the server once a ranged response carried one.
    #[serde(default)]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub quality: AudioQuality,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub created_ms: u64,
}

impl Download {
    pub fn new(song: Song, quality: AudioQuality, created_ms: u64) -> Self {
        Self {
            id: song.id.clone(),
            song,
            state: DownloadState::Queued,
            file_name: None,
            size_bytes: 0,
            bytes_downloaded: 0,
            total_bytes: None,
            quality,
            error: None,
            created_ms,
        }
    }

    pub fn percent(&self) -> u8 {
        progress_percent(
            self.bytes_downloaded,
            self.total_bytes.unwrap_or(self.size_bytes),
        )
    }
}

/// Bytes one entry occupies on disk.
///
/// `Completed` counts its final size, `Downloading` the bytes already written,
/// and the remaining states nothing (no file, or the `.part` is deleted).
pub fn occupied_bytes(state: DownloadState, size_bytes: u64, bytes_downloaded: u64) -> u64 {
    match state {
        DownloadState::Completed => size_bytes,
        DownloadState::Downloading => bytes_downloaded,
        DownloadState::Queued | DownloadState::Failed | DownloadState::Cancelled => 0,
    }
}

/// Sum of [`occupied_bytes`] over `entries`, saturating at `u64::MAX`.
pub fn usage_of<'a>(entries: impl IntoIterator<Item = &'a Download>) -> u64 {
    entries.into_iter().fold(0u64, |total, entry| {
        total.saturating_add(occupied_bytes(
            entry.state,
            entry.size_bytes,
            entry.bytes_downloaded,
        ))
    })
}

/// [`usage_of`] over a manifest slice (the manager's `storage_usage`).
pub fn storage_sum(entries: &[Download]) -> u64 {
    usage_of(entries)
}

/// Manager configuration. `max_concurrent` is clamped through
/// [`clamp_concurrency`] when the pool is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadConfig {
    /// Directory that holds `manifest.json` and the audio files.
    pub dir: std::path::PathBuf,
    pub max_concurrent: usize,
    pub quality: AudioQuality,
    /// Soft cap, checked at enqueue and before a transfer starts. `None` is unlimited.
    pub max_storage_bytes: Option<u64>,
}

impl DownloadConfig {
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            ..Self::default()
        }
    }
}

impl Default for DownloadConfig {
    fn default() -> Self {
        let dir = directories::UserDirs::new()
            .and_then(|dirs| dirs.audio_dir().map(Path::to_path_buf))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("Metrolist");
        Self {
            dir,
            max_concurrent: 3,
            quality: AudioQuality::Auto,
            max_storage_bytes: None,
        }
    }
}

/// Typed failure carried by [`DownloadEvent::Failed`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DownloadError {
    #[error("storage budget exceeded: {used}/{limit} bytes used")]
    StorageFull { used: u64, limit: u64 },
    #[error("cancelled")]
    Cancelled,
    #[error("download failed: {0}")]
    Download(String),
}

impl From<ml_core::MlError> for DownloadError {
    fn from(error: ml_core::MlError) -> Self {
        match error {
            ml_core::MlError::Cancelled => Self::Cancelled,
            other => Self::Download(other.to_string()),
        }
    }
}

/// Events published on the manager's `crossbeam_channel`.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadEvent {
    Queued(DownloadId),
    /// Coalesced to at most one per 100 ms per transfer; `total == 0` while unknown.
    Progress {
        id: DownloadId,
        bytes: u64,
        total: u64,
    },
    Completed {
        id: DownloadId,
        path: std::path::PathBuf,
    },
    Failed {
        id: DownloadId,
        error: DownloadError,
    },
    Cancelled(DownloadId),
    Removed(DownloadId),
    StorageChanged {
        used: u64,
        limit: Option<u64>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Song {
        Song {
            id: id.to_string(),
            ..Song::default()
        }
    }

    fn entry(state: DownloadState, size: u64, bytes: u64) -> Download {
        let mut download = Download::new(song("x"), AudioQuality::Auto, 0);
        download.state = state;
        download.size_bytes = size;
        download.bytes_downloaded = bytes;
        download
    }

    #[test]
    fn percent_is_total_and_clamped() {
        assert_eq!(progress_percent(0, 0), 0);
        assert_eq!(progress_percent(10, 0), 0);
        assert_eq!(progress_percent(0, 100), 0);
        assert_eq!(progress_percent(50, 100), 50);
        assert_eq!(progress_percent(150, 100), 100);
        assert_eq!(progress_percent(u64::MAX, u64::MAX), 100);
        assert_eq!(progress_percent(u64::MAX, 1), 100);
    }

    #[test]
    fn storage_sum_is_saturating_and_state_sensitive() {
        assert_eq!(occupied_bytes(DownloadState::Completed, 100, 5), 100);
        assert_eq!(occupied_bytes(DownloadState::Downloading, 0, 30), 30);
        assert_eq!(occupied_bytes(DownloadState::Queued, 0, 9), 0);
        assert_eq!(occupied_bytes(DownloadState::Failed, 0, 9), 0);
        assert_eq!(occupied_bytes(DownloadState::Cancelled, 0, 9), 0);

        let entries = vec![
            entry(DownloadState::Completed, 100, 100),
            entry(DownloadState::Downloading, 0, 30),
            entry(DownloadState::Queued, 0, 0),
            entry(DownloadState::Failed, 0, 0),
        ];
        assert_eq!(storage_sum(&entries), 130);
        assert_eq!(storage_sum(&[]), 0);
        assert_eq!(
            storage_sum(&[
                entry(DownloadState::Completed, u64::MAX, 0),
                entry(DownloadState::Completed, 1, 0)
            ]),
            u64::MAX
        );
    }

    #[test]
    fn state_machine_matches_the_documented_table() {
        use DownloadState::*;
        assert!(DownloadState::can_transition(Queued, Downloading));
        assert!(DownloadState::can_transition(Downloading, Completed));
        assert!(DownloadState::can_transition(Downloading, Cancelled));
        assert!(DownloadState::can_transition(Failed, Queued));
        assert!(DownloadState::can_transition(Cancelled, Queued));
        assert!(DownloadState::can_transition(Completed, Completed));
        assert!(!DownloadState::can_transition(Completed, Queued));
        assert!(!DownloadState::can_transition(Completed, Downloading));
        assert!(!DownloadState::can_transition(Queued, Completed));
        assert!(Completed.is_terminal());
        assert!(!Queued.is_terminal());
    }

    #[test]
    fn concurrency_is_clamped_into_range() {
        assert_eq!(clamp_concurrency(0), 1);
        assert_eq!(clamp_concurrency(3), 3);
        assert_eq!(clamp_concurrency(usize::MAX), MAX_CONCURRENT);
    }

    #[test]
    fn percent_prefers_the_live_total() {
        let mut download = entry(DownloadState::Completed, 10, 10);
        download.file_name = Some("f.webm".into());
        download.total_bytes = Some(10);
        assert_eq!(download.percent(), 100);
        download.total_bytes = None;
        download.bytes_downloaded = 5;
        assert_eq!(download.percent(), 50, "falls back to the completed size");
    }
}
