//! Typed errors for fingerprinting, capture and the Shazam API.

use thiserror::Error;

/// Everything that can go wrong between "record audio" and "here is the song".
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecognitionError {
    #[error("audio must be non-empty 16-bit PCM with an even byte length")]
    InvalidAudio,
    #[error("no audio input device: {0}")]
    NoInputDevice(String),
    #[error("microphone capture failed: {0}")]
    Capture(String),
    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("Shazam request timed out")]
    Timeout,
    #[error("Shazam rate limited the request (HTTP 429)")]
    RateLimited,
    #[error("no match found")]
    NoMatch,
    #[error("Shazam service unavailable: {0}")]
    Unavailable(String),
    #[error("HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("malformed Shazam response: {0}")]
    Parse(String),
}

impl From<std::io::Error> for RecognitionError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}
