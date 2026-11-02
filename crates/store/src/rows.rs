//! Row decoding and the store-local record types.
//!
//! Every projection names its columns explicitly: a `SELECT *` here would make
//! `artist.cached_page_json` travel with every song read.

use std::collections::HashMap;

use ml_core::{Album, AlbumRef, Artist, MlResult, Playlist, Song};
use rusqlite::{params_from_iter, Connection, Row};

/// Song columns, always table-qualified so projections can be joined.
pub(crate) const SONG_COLS: &str = "song.id, song.title, song.duration, song.thumbnail_url, \
     song.album_id, song.album_name, song.artists_json, song.explicit, song.set_video_id, \
     song.music_video_type, song.is_episode, song.liked, song.liked_at, song.in_library_at, \
     song.created_at, song.updated_at";

/// Album columns.
pub(crate) const ALBUM_COLS: &str = "album.id, album.playlist_id, album.title, album.year, \
     album.thumbnail_url, album.song_count, album.duration, album.explicit, album.is_uploaded, \
     album.bookmarked_at, album.in_library_at, album.theme_color, album.artists_json";

/// Playlist columns.
pub(crate) const PLAYLIST_COLS: &str = "playlist.id, playlist.name, playlist.browse_id, \
     playlist.author, playlist.thumbnail_url, playlist.song_count, playlist.remote_song_count, \
     playlist.created_at, playlist.updated_at, playlist.is_editable, playlist.is_auto_sync, \
     playlist.play_endpoint_params, playlist.shuffle_endpoint_params, \
     playlist.radio_endpoint_params, playlist.bookmarked_at";

/// Artist columns without the cached page — the bulk projection.
pub(crate) const ARTIST_COLS: &str = "artist.id, artist.name, artist.thumbnail_url, \
     artist.channel_id, artist.subscribers, artist.description, artist.bookmarked_at, \
     artist.last_update";

/// Detail projection: [`ARTIST_COLS`] plus the cached page.
pub(crate) const ARTIST_COLS_DETAIL: &str = "artist.id, artist.name, artist.thumbnail_url, \
     artist.channel_id, artist.subscribers, artist.description, artist.bookmarked_at, \
     artist.last_update, artist.cached_page_json";

/// SQLite variable limit is 999 on older builds; stay well below it.
pub(crate) const SQL_CHUNK: usize = 400;

pub(crate) fn artists_to_json(artists: &[Artist]) -> String {
    serde_json::to_string(artists).unwrap_or_else(|_| "[]".to_string())
}

pub(crate) fn artists_from_json(raw: Option<String>) -> Vec<Artist> {
    raw.and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Decode a [`Song`] whose columns start at `offset` in `row`.
pub(crate) fn song_at(row: &Row<'_>, offset: usize) -> rusqlite::Result<Song> {
    let duration: Option<i64> = row.get(offset + 2)?;
    let album_id: Option<String> = row.get(offset + 4)?;
    let album_name: Option<String> = row.get(offset + 5)?;
    let liked_at: Option<i64> = row.get(offset + 12)?;
    let liked: i64 = row.get(offset + 11)?;
    Ok(Song {
        id: row.get(offset)?,
        title: row.get(offset + 1)?,
        duration: duration.and_then(|d| u32::try_from(d).ok()),
        thumbnail_url: row.get(offset + 3)?,
        album: match (album_id, album_name) {
            (Some(id), Some(title)) => Some(AlbumRef { id, title }),
            (Some(id), None) => Some(AlbumRef {
                id,
                title: String::new(),
            }),
            _ => None,
        },
        artists: artists_from_json(row.get(offset + 6)?),
        explicit: row.get::<_, i64>(offset + 7)? != 0,
        set_video_id: row.get(offset + 8)?,
        music_video_type: row.get(offset + 9)?,
        is_episode: row.get::<_, i64>(offset + 10)? != 0,
        liked: if liked != 0 { liked_at } else { None },
        in_library: row.get(offset + 13)?,
        library_add_token: None,
        library_remove_token: None,
    })
}

pub(crate) fn song_from_row(row: &Row<'_>) -> rusqlite::Result<Song> {
    song_at(row, 0)
}

pub(crate) fn album_from_row(row: &Row<'_>) -> rusqlite::Result<Album> {
    let year: Option<i64> = row.get(3)?;
    let song_count: Option<i64> = row.get(5)?;
    let duration: Option<i64> = row.get(6)?;
    Ok(Album {
        id: row.get(0)?,
        playlist_id: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        title: row.get(2)?,
        artists: artists_from_json(row.get(12)?),
        year: year.and_then(|y| u16::try_from(y).ok()),
        thumbnail_url: row.get(4)?,
        explicit: row.get::<_, i64>(7)? != 0,
        song_count: song_count.and_then(|c| u32::try_from(c).ok()),
        duration: duration.and_then(|d| u32::try_from(d).ok()),
    })
}

/// An artist row including the cached page. Bulk reads leave that `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistRecord {
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub channel_id: Option<String>,
    pub subscribers: Option<String>,
    pub description: Option<String>,
    pub bookmarked_at: Option<i64>,
    pub last_update: Option<i64>,
    pub cached_page_json: Option<String>,
}

impl ArtistRecord {
    /// Decode a row from [`ARTIST_COLS`] (`with_page = false`) or
    /// [`ARTIST_COLS_DETAIL`] (`with_page = true`).
    pub fn from_row(row: &Row<'_>, with_page: bool) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            thumbnail_url: row.get(2)?,
            channel_id: row.get(3)?,
            subscribers: row.get(4)?,
            description: row.get(5)?,
            bookmarked_at: row.get(6)?,
            last_update: row.get(7)?,
            cached_page_json: if with_page { row.get(8)? } else { None },
        })
    }

    /// The card shape the rest of the app uses.
    pub fn as_item(&self) -> ml_core::ArtistItem {
        ml_core::ArtistItem {
            id: self.id.clone(),
            name: self.name.clone(),
            thumbnail_url: self.thumbnail_url.clone(),
            channel_id: self.channel_id.clone(),
            subscribers: self.subscribers.clone(),
        }
    }
}

/// A playlist row as stored (the API's browse/endpoint plumbing included).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistRecord {
    pub id: String,
    pub name: String,
    pub browse_id: Option<String>,
    pub author: Option<String>,
    pub thumbnail_url: Option<String>,
    pub song_count: i64,
    pub remote_song_count: Option<i64>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub is_editable: bool,
    pub is_auto_sync: bool,
    pub play_endpoint_params: Option<String>,
    pub shuffle_endpoint_params: Option<String>,
    pub radio_endpoint_params: Option<String>,
    pub bookmarked_at: Option<i64>,
}

impl PlaylistRecord {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            browse_id: row.get(2)?,
            author: row.get(3)?,
            thumbnail_url: row.get(4)?,
            song_count: row.get(5)?,
            remote_song_count: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
            is_editable: row.get::<_, i64>(9)? != 0,
            is_auto_sync: row.get::<_, i64>(10)? != 0,
            play_endpoint_params: row.get(11)?,
            shuffle_endpoint_params: row.get(12)?,
            radio_endpoint_params: row.get(13)?,
            bookmarked_at: row.get(14)?,
        })
    }

    pub fn as_playlist(&self) -> Playlist {
        Playlist {
            id: self.id.clone(),
            title: self.name.clone(),
            author: self.author.clone().map(|name| Artist { id: None, name }),
            song_count_text: None,
            thumbnail_url: self.thumbnail_url.clone(),
            description: None,
            is_editable: self.is_editable,
            is_podcast: false,
            explicit: false,
        }
    }
}

/// A playlist entry: the song, where it sits, and the per-membership
/// `setVideoId` playlist mutations need.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaylistSong {
    pub song: Song,
    pub position: i64,
    pub set_video_id: Option<String>,
}

/// One play-history row joined with its song.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryEntry {
    pub id: i64,
    pub song: Song,
    pub timestamp: i64,
    pub play_time: i64,
}

/// A cached lyrics payload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CachedLyrics {
    pub song_id: String,
    pub provider: String,
    pub synced: bool,
    pub body: String,
    pub translated: Option<String>,
    pub translation_lang: Option<String>,
    pub updated_at: i64,
}

/// A speed-dial tile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeedDial {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub thumbnail_url: Option<String>,
    pub item_type: String,
    pub item_json: String,
    pub created_at: i64,
    pub position: i64,
}

/// Locally owned state of an album row: bookmark time, library time and the
/// theme colour derived from its artwork.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlbumState {
    pub bookmarked_at: Option<i64>,
    pub in_library_at: Option<i64>,
    pub theme_color: Option<i64>,
}

/// A `(song, month)` play tally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayCount {
    pub song_id: String,
    pub count: i64,
}

/// Which slice of the library a page asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LibraryFilter {
    #[default]
    All,
    Songs,
    Albums,
    Artists,
    Playlists,
    Liked,
}

pub(crate) fn store_error(error: rusqlite::Error) -> ml_core::MlError {
    ml_core::MlError::Store(error.to_string())
}

/// Current wall clock as epoch milliseconds, as a SQLite INTEGER.
pub(crate) fn now_ms() -> i64 {
    ml_core::now_millis() as i64
}

/// Case-fold a string for substring comparison: [`str::to_lowercase`], which
/// handles every Unicode script, unlike SQLite's `COLLATE NOCASE` (ASCII only).
///
/// Total and deterministic. Folding is not one-to-one for a few characters
/// (`İ` becomes two), which cannot change "does the query appear here" because
/// both the stored text and the query go through it.
pub(crate) fn fold(text: &str) -> String {
    text.to_lowercase()
}

/// Bind value for a case-insensitive substring match: `%escaped%`, with `!`
/// as the escape character so a user's `%` or `_` stays literal.
pub(crate) fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for ch in query.chars() {
        if matches!(ch, '!' | '%' | '_') {
            pattern.push('!');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}

/// Fill in `song.artists` from `song_artist_map`, using the bulk projection so
/// a library read never pulls one cached artist page per song.
pub(crate) fn hydrate_songs(conn: &Connection, songs: &mut [Song]) -> MlResult<()> {
    if songs.is_empty() {
        return Ok(());
    }
    // A prepared statement cannot be open while another query runs on the same
    // connection, so collect the maps first and stamp them afterwards.
    let mut by_song: HashMap<String, Vec<Artist>> = HashMap::with_capacity(songs.len());
    for chunk in songs.chunks(SQL_CHUNK) {
        let sql = format!(
            "SELECT m.song_id, a.id, a.name, a.thumbnail_url, a.channel_id, a.subscribers \
             FROM song_artist_map m JOIN artist a ON a.id = m.artist_id \
             WHERE m.song_id IN ({}) ORDER BY m.song_id, m.position",
            crate::song::placeholders(chunk.len())
        );
        let mut stmt = conn.prepare(&sql).map_err(store_error)?;
        let rows = stmt
            .query_map(
                params_from_iter(chunk.iter().map(|song| song.id.as_str())),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        Artist {
                            id: Some(row.get(1)?),
                            name: row.get(2)?,
                        },
                    ))
                },
            )
            .map_err(store_error)?;
        for row in rows {
            let (song_id, artist) = row.map_err(store_error)?;
            by_song.entry(song_id).or_default().push(artist);
        }
    }
    for song in songs.iter_mut() {
        if let Some(artists) = by_song.remove(&song.id) {
            song.artists = artists;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_pattern_escapes_only_the_escape_char_and_wildcards() {
        assert_eq!(like_pattern("müller"), "%müller%");
        assert_eq!(like_pattern("100%"), "%100!%%");
        assert_eq!(like_pattern("a_b"), "%a!_b%");
        assert_eq!(like_pattern("!"), "%!!%");
        assert_eq!(like_pattern("%_!"), "%!%!_!!%");
        assert_eq!(like_pattern(""), "%%");
        // Quotes and non-ASCII pass through untouched: they are data.
        assert_eq!(like_pattern("it's"), "%it's%");
        assert_eq!(like_pattern("東京"), "%東京%");
    }

    #[test]
    fn fold_is_a_total_case_map_and_a_substring_test() {
        assert_eq!(fold("MÜLLER"), "müller");
        assert_eq!(fold("ÅNGSTRÖM"), "ångström");
        assert_eq!(fold("Björk"), "björk");
        assert_eq!(fold("東京"), "東京", "scripts without case are unchanged");
        // Folding both sides is what makes the SQL `LIKE` work: a folded needle
        // is always a substring of the folded haystack when the originals match
        // case-insensitively.
        for (haystack, needle) in [
            ("Müller über alles", "MÜLLER"),
            ("Ångström", "ång"),
            ("Nightcall", "NIGHTCALL"),
        ] {
            assert!(fold(haystack).contains(&fold(needle)));
        }
    }
}
