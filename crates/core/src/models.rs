//! Content models projected from YouTube Music / InnerTube responses.
//!
//! Field names follow the Kotlin `innertube.models` package (§2 of
//! `docs/ARCHITECTURE.md`); every API-provided string is optional unless the
//! upstream model guarantees it. Timestamps are epoch milliseconds (UTC).

use serde::{Deserialize, Serialize};

use crate::util::{format_duration_opt, thumbnail_url};

/// A lightweight artist credit attached to songs, albums and videos.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub id: Option<String>,
    pub name: String,
}

impl Artist {
    pub fn new(name: impl Into<String>, id: Option<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }
}

/// The album reference carried by a song (id + display title only).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: String,
    pub title: String,
}

/// A playable track. `duration` is seconds; `liked`/`in_library` are epoch ms.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Song {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<Artist>,
    pub duration: Option<u32>,
    pub thumbnail_url: Option<String>,
    pub album: Option<AlbumRef>,
    #[serde(default)]
    pub explicit: bool,
    pub set_video_id: Option<String>,
    pub music_video_type: Option<String>,
    #[serde(default)]
    pub is_episode: bool,
    pub liked: Option<i64>,
    pub in_library: Option<i64>,
    pub library_add_token: Option<String>,
    pub library_remove_token: Option<String>,
}

impl Song {
    pub fn thumb(&self, size: u32) -> String {
        match &self.thumbnail_url {
            Some(url) if !url.is_empty() => crate::util::resize_thumbnail_url(url, size),
            _ => thumbnail_url(&self.id, size),
        }
    }

    /// `true` for official music videos / user uploads, `false` for ATV tracks.
    pub fn is_video_song(&self) -> bool {
        match self.music_video_type.as_deref() {
            None | Some("") | Some("MUSIC_VIDEO_TYPE_ATV") => false,
            Some(_) => true,
        }
    }

    pub fn artist_names(&self) -> String {
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn duration_text(&self) -> String {
        format_duration_opt(self.duration)
    }

    pub fn share_url(&self) -> String {
        format!("https://music.youtube.com/watch?v={}", self.id)
    }
}

/// A full album result (browse id + playlist id, as returned by search).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Album {
    pub id: String,
    pub playlist_id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<Artist>,
    pub year: Option<u16>,
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub explicit: bool,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
}

impl Album {
    pub fn as_ref(&self) -> AlbumRef {
        AlbumRef {
            id: self.id.clone(),
            title: self.title.clone(),
        }
    }

    pub fn artist_names(&self) -> String {
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn share_url(&self) -> String {
        format!(
            "https://music.youtube.com/playlist?list={}",
            self.playlist_id
        )
    }
}

/// An artist card (search result / carousel item).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistItem {
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub channel_id: Option<String>,
    pub subscribers: Option<String>,
}

impl ArtistItem {
    pub fn share_url(&self) -> String {
        format!("https://music.youtube.com/channel/{}", self.id)
    }
}

/// A playlist card or page header.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub author: Option<Artist>,
    pub song_count_text: Option<String>,
    pub thumbnail_url: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub is_editable: bool,
    #[serde(default)]
    pub is_podcast: bool,
    #[serde(default)]
    pub explicit: bool,
}

impl Playlist {
    pub fn share_url(&self) -> String {
        format!("https://music.youtube.com/playlist?list={}", self.id)
    }
}

/// A podcast episode (also playable as a song).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub id: String,
    pub title: String,
    pub author: Option<Artist>,
    pub podcast: Option<AlbumRef>,
    pub duration: Option<u32>,
    pub publish_date_text: Option<String>,
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub explicit: bool,
}

impl Episode {
    /// Episodes share the player path with songs; `is_episode` marks them.
    pub fn as_song(&self) -> Song {
        Song {
            id: self.id.clone(),
            title: self.title.clone(),
            artists: self.author.iter().cloned().collect(),
            duration: self.duration,
            thumbnail_url: self.thumbnail_url.clone(),
            album: self.podcast.clone(),
            explicit: self.explicit,
            is_episode: true,
            ..Song::default()
        }
    }
}

/// A music video result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<Artist>,
    pub duration: Option<u32>,
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub explicit: bool,
}

impl Video {
    pub fn share_url(&self) -> String {
        format!("https://music.youtube.com/watch?v={}", self.id)
    }
}

/// The polymorphic item produced by every browse/search shelf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YtItem {
    Song(Song),
    Album(Album),
    Artist(ArtistItem),
    Playlist(Playlist),
    Episode(Episode),
    Video(Video),
}

impl YtItem {
    pub fn id(&self) -> &str {
        match self {
            Self::Song(v) => &v.id,
            Self::Album(v) => &v.id,
            Self::Artist(v) => &v.id,
            Self::Playlist(v) => &v.id,
            Self::Episode(v) => &v.id,
            Self::Video(v) => &v.id,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Song(v) => &v.title,
            Self::Album(v) => &v.title,
            Self::Artist(v) => &v.name,
            Self::Playlist(v) => &v.title,
            Self::Episode(v) => &v.title,
            Self::Video(v) => &v.title,
        }
    }

    pub fn thumbnail(&self) -> Option<&str> {
        match self {
            Self::Song(v) => v.thumbnail_url.as_deref(),
            Self::Album(v) => v.thumbnail_url.as_deref(),
            Self::Artist(v) => v.thumbnail_url.as_deref(),
            Self::Playlist(v) => v.thumbnail_url.as_deref(),
            Self::Episode(v) => v.thumbnail_url.as_deref(),
            Self::Video(v) => v.thumbnail_url.as_deref(),
        }
    }

    pub fn explicit(&self) -> bool {
        match self {
            Self::Song(v) => v.explicit,
            Self::Album(v) => v.explicit,
            Self::Artist(_) => false,
            Self::Playlist(v) => v.explicit,
            Self::Episode(v) => v.explicit,
            Self::Video(v) => v.explicit,
        }
    }

    pub fn as_song(&self) -> Option<&Song> {
        match self {
            Self::Song(v) => Some(v),
            _ => None,
        }
    }
}

/// What kind of shelf a [`HomeSection`] represents; drives card rendering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    #[default]
    Unknown,
    QuickPicks,
    KeepListening,
    NewReleases,
    Songs,
    Albums,
    Artists,
    Playlists,
    Videos,
    Episodes,
    Related,
}

/// One shelf on Home/Artist pages.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HomeSection {
    pub title: String,
    #[serde(default)]
    pub items: Vec<YtItem>,
    pub continuation: Option<String>,
    #[serde(default)]
    pub kind: SectionKind,
}

/// Home feed with the continuation token for "more shelves".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HomeFeed {
    #[serde(default)]
    pub sections: Vec<HomeSection>,
    pub continuation: Option<String>,
}

/// Search results. `items` keeps mixed shelves; `songs` is the filtered song list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchPage {
    pub query: String,
    #[serde(default)]
    pub items: Vec<YtItem>,
    #[serde(default)]
    pub songs: Vec<Song>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumPage {
    pub album: Album,
    #[serde(default)]
    pub songs: Vec<Song>,
    pub description: Option<String>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistPage {
    pub artist: ArtistItem,
    #[serde(default)]
    pub top_songs: Vec<Song>,
    #[serde(default)]
    pub sections: Vec<HomeSection>,
    pub description: Option<String>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistPage {
    pub playlist: Playlist,
    #[serde(default)]
    pub songs: Vec<Song>,
    pub continuation: Option<String>,
}

/// Result of `next` (radio / album queue / playlist queue).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueuePage {
    pub title: Option<String>,
    #[serde(default)]
    pub songs: Vec<Song>,
    pub current_index: Option<usize>,
    pub continuation: Option<String>,
}

/// Library tab payload (requires an authenticated session for remote content).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryPage {
    #[serde(default)]
    pub songs: Vec<Song>,
    #[serde(default)]
    pub albums: Vec<Album>,
    #[serde(default)]
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub artists: Vec<ArtistItem>,
    pub continuation: Option<String>,
}

/// A single timed lyric line (`start_ms` is absolute from track start).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsLine {
    pub start_ms: u64,
    #[serde(default)]
    pub end_ms: Option<u64>,
    pub text: String,
    /// Word/segment timings for karaoke-style rendering.
    #[serde(default)]
    pub words: Vec<LyricsWord>,
    /// Background vocal line (e.g. TTML `ttm:role="x-bg"`).
    #[serde(default)]
    pub background: bool,
    /// Per-line agent/voice (duet lyrics); `None` for the primary singer.
    #[serde(default)]
    pub agent: Option<String>,
}

/// One timed word (or short segment) inside a [`LyricsLine`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsWord {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Lyrics returned by any provider, plus optional translation/romanization.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LyricsDocument {
    pub provider: String,
    #[serde(default)]
    pub lines: Vec<LyricsLine>,
    pub plain: Option<String>,
    pub translated: Option<Vec<LyricsLine>>,
    pub language: Option<String>,
}

impl LyricsDocument {
    pub fn is_synced(&self) -> bool {
        self.lines.iter().any(|line| line.start_ms > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_lyrics_json_without_word_timing_still_loads() {
        let doc: LyricsDocument = serde_json::from_str(
            r#"{"provider":"lrclib","lines":[{"start_ms":1200,"end_ms":2000,"text":"hello"}],
                "plain":null,"translated":null,"language":null}"#,
        )
        .unwrap();
        assert!(doc.is_synced());
        assert!(doc.lines[0].words.is_empty());
        assert!(!doc.lines[0].background);
        assert_eq!(doc.lines[0].agent, None);
    }

    #[test]
    fn word_timing_fields_round_trip() {
        let line = LyricsLine {
            start_ms: 0,
            end_ms: Some(900),
            text: "hi".to_string(),
            words: vec![LyricsWord {
                start_ms: 0,
                end_ms: 450,
                text: "hi".to_string(),
            }],
            background: true,
            agent: Some("v2".to_string()),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(serde_json::from_str::<LyricsLine>(&json).unwrap(), line);
    }
}
