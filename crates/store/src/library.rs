//! The offline library: one paged query per shelf, plus the cross-shelf
//! search the Library tab uses.
//!
//! Also home to the two library-wide operations that span several tables:
//! bookmarks (a flag on `album`, `artist` and `playlist`) and the auto
//! playlists, which are queries rather than stored playlists.

use ml_core::{Album, ArtistItem, LibraryPage, LibrarySort, MlResult, Playlist, Song};
use rusqlite::{params, OptionalExtension};

use crate::rows::{song_from_row, store_error, LibraryFilter, PlaylistRecord, SONG_COLS};
use crate::stats::StatsPeriod;
use crate::Store;

/// Which table a bookmark lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookmarkKind {
    Album,
    Artist,
    Playlist,
}

impl BookmarkKind {
    /// The table the bookmark column lives in. Only these three literals can
    /// come out of here, so formatting it into SQL below is not injectable.
    pub fn table(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        }
    }
}

/// The auto playlists the Library tab can offer.
///
/// Downloaded songs are absent on purpose: the download manifest lives in
/// `ml-downloads`, which this crate must not depend on, so there is no honest
/// query for them here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutoPlaylist {
    /// Every liked song, most recently liked first.
    Liked,
    /// Recently played songs, one row per song, newest first.
    RecentlyPlayed,
    /// Most-played songs in a period.
    MostPlayed(StatsPeriod),
    /// Songs saved to the library, newest first.
    Library,
}

/// Bytes of one page, whatever the shelf. Pages tile: `offset` marches by
/// `limit` and no row appears twice.
impl Store {
    /// One library shelf, or several when `filter` is [`LibraryFilter::All`].
    ///
    /// `limit`/`offset` apply per shelf rather than to the union, because the
    /// tab shows one page per shelf; callers paging a merged list should ask
    /// for a concrete filter.
    pub fn library_page(
        &self,
        filter: LibraryFilter,
        sort: LibrarySort,
        limit: u32,
        offset: u32,
    ) -> MlResult<LibraryPage> {
        let mut page = LibraryPage::default();
        match filter {
            LibraryFilter::All => {
                page.songs = self.library_songs(sort, limit, offset)?;
                page.albums = self.library_albums(sort, limit, offset)?;
                page.artists = self
                    .artists(limit, offset)?
                    .iter()
                    .map(|artist| artist.as_item())
                    .collect();
                page.playlists = self
                    .playlists(limit, offset)?
                    .iter()
                    .map(|playlist| playlist.as_playlist())
                    .collect();
            }
            LibraryFilter::Songs => page.songs = self.library_songs(sort, limit, offset)?,
            LibraryFilter::Liked => page.songs = self.liked_songs(limit, offset)?,
            LibraryFilter::Albums => page.albums = self.library_albums(sort, limit, offset)?,
            LibraryFilter::Artists => {
                page.artists = self
                    .bookmarked_artists(limit, offset)?
                    .iter()
                    .map(|artist| artist.as_item())
                    .collect();
            }
            LibraryFilter::Playlists => {
                page.playlists = self
                    .playlists(limit, offset)?
                    .iter()
                    .map(|playlist| playlist.as_playlist())
                    .collect();
            }
        }
        Ok(page)
    }

    /// Albums that hold at least one library song, in the requested order.
    pub fn library_albums(
        &self,
        sort: LibrarySort,
        limit: u32,
        offset: u32,
    ) -> MlResult<Vec<ml_core::Album>> {
        let ids: Vec<String> = self.with_conn(|conn| {
            // Every arm must reference `album` (or be a self-contained
            // correlated subquery): the `EXISTS` alias below is scoped to that
            // subquery and invisible to `ORDER BY`.
            let order = match sort {
                LibrarySort::RecentlyAdded => {
                    // `EXISTS` already guarantees a non-null timestamp, so no
                    // `IS NULL` tie-break is needed.
                    "(SELECT MAX(s.in_library_at) FROM song s WHERE s.album_id = album.id) DESC, \
                     album.title COLLATE NOCASE"
                }
                LibrarySort::Alphabetical => "album.title COLLATE NOCASE",
                LibrarySort::MostPlayed => {
                    "(SELECT COALESCE(SUM(e.play_time), 0) FROM event e \
                       JOIN song es ON es.id = e.song_id WHERE es.album_id = album.id) DESC, \
                     album.title COLLATE NOCASE"
                }
                LibrarySort::Artist => {
                    "(SELECT a.name FROM song s JOIN song_artist_map m ON m.song_id = s.id \
                       JOIN artist a ON a.id = m.artist_id \
                       WHERE s.album_id = album.id ORDER BY m.position LIMIT 1) COLLATE NOCASE, \
                     album.title COLLATE NOCASE"
                }
            };
            let sql = format!(
                "SELECT album.id FROM album \
                 WHERE EXISTS (SELECT 1 FROM song s WHERE s.album_id = album.id \
                               AND s.in_library_at IS NOT NULL) \
                 ORDER BY {order} LIMIT ?1 OFFSET ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(crate::rows::store_error)?;
            let ids = stmt
                .query_map(
                    rusqlite::params![i64::from(limit), i64::from(offset)],
                    |row| row.get(0),
                )
                .map_err(crate::rows::store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(crate::rows::store_error)?;
            Ok(ids)
        })?;
        self.albums_by_ids(&ids)
    }

    /// Search the local library: songs, albums, artists and playlists in one
    /// [`LibraryPage`]. Only rows the user actually has are returned.
    ///
    /// Matching is a case-folded substring test (`ml_fold`), so `MÜLLER` finds
    /// `müller`; `%` and `_` in the query are literal. Each shelf is ordered
    /// by folded title then id, so equal results never reorder between calls.
    pub fn search_library(&self, query: &str, limit: u32) -> MlResult<LibraryPage> {
        Ok(LibraryPage {
            songs: self.search_songs(query, limit)?,
            albums: self.search_albums(query, limit)?,
            artists: self.search_artists(query, limit)?,
            playlists: self.search_playlists(query, limit)?,
            continuation: None,
        })
    }

    /// Songs the user can play offline. Playback downloads are not implemented
    /// yet, so this is empty by construction rather than a lie about state.
    pub fn downloaded_songs(&self, _limit: u32, _offset: u32) -> MlResult<Vec<Song>> {
        Ok(Vec::new())
    }

    // ---------------------------------------------------------- bookmarks

    /// Set or clear one bookmark. An id with no row is a no-op, not an error:
    /// the UI bookmarks what it is displaying.
    pub fn set_bookmark(
        &self,
        kind: BookmarkKind,
        id: &str,
        bookmarked: bool,
        at_ms: i64,
    ) -> MlResult<()> {
        let sql = format!(
            "UPDATE {} SET bookmarked_at = ?2 WHERE id = ?1",
            kind.table()
        );
        self.with_conn(|conn| {
            conn.execute(
                &sql,
                params![id, if bookmarked { Some(at_ms) } else { None }],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// When one item was bookmarked, or `None` if it is not (or does not
    /// exist) — a missing row and an unbookmarked row read the same to a UI.
    pub fn bookmark(&self, kind: BookmarkKind, id: &str) -> MlResult<Option<i64>> {
        let sql = format!(
            "SELECT bookmarked_at FROM {} WHERE id = ?1 AND bookmarked_at IS NOT NULL",
            kind.table()
        );
        self.with_conn(|conn| {
            conn.query_row(&sql, [id], |row| row.get(0))
                .optional()
                .map_err(store_error)
        })
    }

    /// Ids bookmarked in one table, most recent bookmark first.
    pub fn bookmarked_ids(&self, kind: BookmarkKind, limit: u32) -> MlResult<Vec<String>> {
        let sql = format!(
            "SELECT id FROM {} WHERE bookmarked_at IS NOT NULL \
             ORDER BY bookmarked_at DESC, id LIMIT ?1",
            kind.table()
        );
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let ids = stmt
                .query_map([i64::from(limit)], |row| row.get(0))
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(ids)
        })
    }

    /// Every bookmarked item of every kind, in one page.
    ///
    /// [`LibraryPage`] carries exactly these four kinds; songs stay empty
    /// because a song is liked rather than bookmarked.
    pub fn bookmarks_page(&self, limit: u32) -> MlResult<LibraryPage> {
        let albums: Vec<Album> = self.bookmarked_albums(limit, 0)?;
        let artists: Vec<ArtistItem> = self
            .bookmarked_artists(limit, 0)?
            .iter()
            .map(|artist| artist.as_item())
            .collect();
        let playlists: Vec<Playlist> = self
            .bookmarked_playlists(limit, 0)?
            .iter()
            .map(PlaylistRecord::as_playlist)
            .collect();
        Ok(LibraryPage {
            albums,
            artists,
            playlists,
            ..LibraryPage::default()
        })
    }

    // ---------------------------------------------------- auto playlists

    /// Run one auto playlist.
    pub fn auto_playlist(
        &self,
        playlist: AutoPlaylist,
        now_ms: i64,
        limit: u32,
    ) -> MlResult<Vec<Song>> {
        match playlist {
            AutoPlaylist::Liked => self.liked_songs(limit, 0),
            AutoPlaylist::Library => self.library_songs(LibrarySort::RecentlyAdded, limit, 0),
            AutoPlaylist::MostPlayed(period) => Ok(self
                .top_songs(period, now_ms, limit)?
                .into_iter()
                .map(|stat| stat.song)
                .collect()),
            AutoPlaylist::RecentlyPlayed => self.recently_played(limit),
        }
    }

    /// The newest play of each distinct song, newest first.
    ///
    /// Unlike [`Store::history`] this does not collapse per period: a song
    /// played today and last month appears once, at its today position.
    pub fn recently_played(&self, limit: u32) -> MlResult<Vec<Song>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS} FROM song \
                 WHERE song.id IN (SELECT e.song_id FROM event e GROUP BY e.song_id) \
                 ORDER BY (SELECT MAX(e.timestamp) FROM event e WHERE e.song_id = song.id) DESC, \
                          song.id \
                 LIMIT ?1"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let mut songs = stmt
                .query_map([i64::from(limit)], song_from_row)
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            crate::rows::hydrate_songs(conn, &mut songs)?;
            Ok(songs)
        })
    }
}
