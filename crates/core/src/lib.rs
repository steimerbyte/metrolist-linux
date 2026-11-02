//! `ml-core` — headless domain layer shared by every Metrolist Linux crate.
//!
//! This crate deliberately has no UI, network or database dependencies. It owns
//! the types that cross crate boundaries: content models, the queue state
//! machine, settings, player snapshots and the async event vocabulary.

pub mod error;
pub mod events;
pub mod media;
pub mod models;
pub mod player;
pub mod queue;
pub mod settings;
pub mod util;

pub use error::{MlError, MlResult};
pub use events::AppEvent;
pub use media::{MediaInfo, StreamInfo};
pub use models::{
    Album, AlbumPage, AlbumRef, Artist, ArtistItem, ArtistPage, Episode, HomeFeed, HomeSection,
    LibraryPage, LyricsDocument, LyricsLine, LyricsWord, Playlist, PlaylistPage, QueuePage,
    SearchPage, SectionKind, Song, Video, YtItem,
};
pub use player::{PlaybackStatus, PlayerSnapshot};
pub use queue::{Queue, QueueSource, RepeatMode};
pub use settings::{
    AudioQuality, DownloadSettings, LibrarySort, LyricsSettings, NavStyle, ProxyConfig, Settings,
    ThemeMode,
};
pub use util::{
    format_duration, format_duration_opt, format_position, now_millis, parse_video_id,
    resize_thumbnail_url, thumbnail_url,
};
