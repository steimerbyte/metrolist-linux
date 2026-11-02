//! The single error type crossing crate boundaries.

use thiserror::Error;

/// Result alias used by every `ml-*` crate.
pub type MlResult<T> = Result<T, MlError>;

/// Errors surfaced by the backend crates.
///
/// Stubs in this skeleton return [`MlError::Unimplemented`]; they never panic.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MlError {
    #[error("not implemented: {0}")]
    Unimplemented(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("parse error: {0}")]
    Parse(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("content unavailable: {0}")]
    Unavailable(String),
    #[error("authentication required")]
    AuthRequired,
    #[error("I/O error: {0}")]
    Io(String),
    #[error("database error: {0}")]
    Store(String),
    #[error("playback error: {0}")]
    Playback(String),
    #[error("lyrics error: {0}")]
    Lyrics(String),
    #[error("cancelled")]
    Cancelled,
}

impl MlError {
    /// Convenience constructor for honest stubs.
    pub fn unimplemented(what: impl Into<String>) -> Self {
        Self::Unimplemented(what.into())
    }
}

impl From<std::io::Error> for MlError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}
