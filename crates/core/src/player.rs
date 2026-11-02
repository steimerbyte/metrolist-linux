//! Player state published by the audio engine to the UI.

use serde::{Deserialize, Serialize};

use crate::{queue::RepeatMode, Song};

/// Transport state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    #[default]
    Idle,
    Loading,
    Buffering,
    Playing,
    Paused,
    Stopped,
    Ended,
    Error,
}

/// Immutable player snapshot sent over the UI event channel (about 10 Hz).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub status: PlaybackStatus,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub current: Option<Song>,
    pub queue_len: usize,
    pub queue_index: Option<usize>,
    pub buffering: bool,
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Idle,
            position_ms: 0,
            duration_ms: 0,
            volume: 1.0,
            muted: false,
            shuffle: false,
            repeat: RepeatMode::Off,
            current: None,
            queue_len: 0,
            queue_index: None,
            buffering: false,
        }
    }
}

impl PlayerSnapshot {
    pub fn is_playing(&self) -> bool {
        matches!(
            self.status,
            PlaybackStatus::Playing | PlaybackStatus::Buffering
        )
    }
}
