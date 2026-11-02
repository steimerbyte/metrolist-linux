//! Async results sent from backend tasks to the UI.
//!
//! The UI owns all state; backend crates only produce these values. See the
//! data-flow section of `docs/ARCHITECTURE.md`.

use crate::{error::MlError, media::StreamInfo, models::*, player::PlayerSnapshot};

/// UI-facing event vocabulary. New variants require a `ml-core` minor bump.
#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    HomeLoaded(HomeFeed),
    SearchLoaded(SearchPage),
    AlbumLoaded(AlbumPage),
    ArtistLoaded(ArtistPage),
    PlaylistLoaded(PlaylistPage),
    QueueLoaded(QueuePage),
    LyricsLoaded(LyricsDocument),
    LibraryLoaded(LibraryPage),
    PlayerState(PlayerSnapshot),
    StreamResolved(StreamInfo),
    Error(MlError),
    Toast(String),
}
