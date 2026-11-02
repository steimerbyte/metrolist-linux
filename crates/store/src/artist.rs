//! Artist rows: two projections, one invariant.
//!
//! `artist.cached_page_json` is kilobytes per artist. A library read builds one
//! artist per song, so bulk queries use [`ARTIST_COLS`] and leave the page on
//! disk; only the artist screen's [`Store::artist_detail`] selects it.

use ml_core::{ArtistItem, MlResult, Song};
use rusqlite::{params, Connection, OptionalExtension};

use crate::rows::{
    fold, hydrate_songs, like_pattern, now_ms, song_at, store_error, ArtistRecord, ARTIST_COLS,
    ARTIST_COLS_DETAIL, SONG_COLS,
};
use crate::Store;

impl Store {
    /// Write an artist row, keeping any cached page and existing thumbnail.
    pub fn upsert_artist(&self, artist: &ArtistRecord) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO artist (id, name, thumbnail_url, channel_id, subscribers, description, \
                                     bookmarked_at, last_update, cached_page_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, COALESCE(?8, ?9), ?10) \
                 ON CONFLICT(id) DO UPDATE SET \
                    name = excluded.name, \
                    thumbnail_url = COALESCE(excluded.thumbnail_url, artist.thumbnail_url), \
                    channel_id = COALESCE(excluded.channel_id, artist.channel_id), \
                    subscribers = COALESCE(excluded.subscribers, artist.subscribers), \
                    description = COALESCE(excluded.description, artist.description), \
                    bookmarked_at = COALESCE(excluded.bookmarked_at, artist.bookmarked_at), \
                    last_update = COALESCE(excluded.last_update, artist.last_update), \
                    cached_page_json = COALESCE(excluded.cached_page_json, artist.cached_page_json)",
                params![
                    artist.id,
                    artist.name,
                    artist.thumbnail_url,
                    artist.channel_id,
                    artist.subscribers,
                    artist.description,
                    artist.bookmarked_at,
                    artist.last_update,
                    now_ms(),
                    artist.cached_page_json,
                ],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Bulk projection: the cached page is not returned.
    pub fn get_artist(&self, id: &str) -> MlResult<Option<ArtistRecord>> {
        self.with_conn(|conn| {
            let sql = format!("SELECT {ARTIST_COLS} FROM artist WHERE artist.id = ?1");
            conn.query_row(&sql, [id], |row| ArtistRecord::from_row(row, false))
                .optional()
                .map_err(store_error)
        })
    }

    /// Detail projection for the artist screen: includes the cached page.
    pub fn artist_detail(&self, id: &str) -> MlResult<Option<ArtistRecord>> {
        self.with_conn(|conn| {
            let sql = format!("SELECT {ARTIST_COLS_DETAIL} FROM artist WHERE artist.id = ?1");
            conn.query_row(&sql, [id], |row| ArtistRecord::from_row(row, true))
                .optional()
                .map_err(store_error)
        })
    }

    /// Store the artist page payload seen on screen.
    pub fn set_artist_page(&self, id: &str, page_json: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE artist SET cached_page_json = ?2, last_update = ?3 WHERE id = ?1",
                params![id, page_json, now_ms()],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Update only the thumbnail, leaving every other column (the cached page
    /// above all) intact.
    pub fn update_artist_thumbnail(&self, id: &str, thumbnail_url: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE artist SET thumbnail_url = ?2, last_update = ?3 WHERE id = ?1",
                params![id, thumbnail_url, now_ms()],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Toggle the bookmark, keeping the timestamp when it is set.
    pub fn set_artist_bookmarked(&self, id: &str, bookmarked: bool) -> MlResult<()> {
        self.set_artist_bookmarked_at(id, if bookmarked { Some(now_ms()) } else { None })
    }

    /// [`Store::set_artist_bookmarked`] with an injected timestamp.
    pub fn set_artist_bookmarked_at(&self, id: &str, at_ms: Option<i64>) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE artist SET bookmarked_at = ?2 WHERE id = ?1",
                params![id, at_ms],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Every artist, by name. Never carries a cached page.
    pub fn artists(&self, limit: u32, offset: u32) -> MlResult<Vec<ArtistRecord>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {ARTIST_COLS} FROM artist ORDER BY artist.name COLLATE NOCASE, artist.id \
                 LIMIT ?1 OFFSET ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let artists = stmt
                .query_map(params![i64::from(limit), i64::from(offset)], |row| {
                    ArtistRecord::from_row(row, false)
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(artists)
        })
    }

    /// Bookmarked (subscribed) artists, by name.
    pub fn bookmarked_artists(&self, limit: u32, offset: u32) -> MlResult<Vec<ArtistRecord>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {ARTIST_COLS} FROM artist WHERE artist.bookmarked_at IS NOT NULL \
                 ORDER BY artist.name COLLATE NOCASE, artist.id LIMIT ?1 OFFSET ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let artists = stmt
                .query_map(params![i64::from(limit), i64::from(offset)], |row| {
                    ArtistRecord::from_row(row, false)
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(artists)
        })
    }

    /// Artists whose name matches, restricted to those with a library song.
    pub fn search_artists(&self, query: &str, limit: u32) -> MlResult<Vec<ArtistItem>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {ARTIST_COLS} FROM artist WHERE ml_fold(artist.name) LIKE ?1 ESCAPE '!' \
                 AND EXISTS (SELECT 1 FROM song_artist_map m JOIN song ON song.id = m.song_id \
                             WHERE m.artist_id = artist.id AND song.in_library_at IS NOT NULL) \
                 ORDER BY ml_fold(artist.name), artist.id LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let artists = stmt
                .query_map(
                    params![like_pattern(&fold(query)), i64::from(limit)],
                    |row| ArtistRecord::from_row(row, false),
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(artists.iter().map(ArtistRecord::as_item).collect())
        })
    }

    /// Library songs credited to an artist.
    pub fn artist_songs(&self, artist_id: &str, limit: u32, offset: u32) -> MlResult<Vec<Song>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS} FROM song_artist_map m JOIN song ON song.id = m.song_id \
                 WHERE m.artist_id = ?1 AND song.in_library_at IS NOT NULL \
                 ORDER BY song.in_library_at, song.id LIMIT ?2 OFFSET ?3"
            );
            query_artist_songs(
                conn,
                &sql,
                params![artist_id, i64::from(limit), i64::from(offset)],
            )
        })
    }

    /// Number of library songs credited to an artist.
    pub fn artist_song_count(&self, artist_id: &str) -> MlResult<u32> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(DISTINCT song.id) FROM song_artist_map m \
                 JOIN song ON song.id = m.song_id \
                 WHERE m.artist_id = ?1 AND song.in_library_at IS NOT NULL",
                [artist_id],
                |row| row.get(0),
            )
            .map_err(store_error)
        })
    }

    /// Credit links of one song, in order.
    pub fn song_artists(&self, song_id: &str) -> MlResult<Vec<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT artist_id FROM song_artist_map WHERE song_id = ?1 ORDER BY position, artist_id",
                )
                .map_err(store_error)?;
            let ids = stmt
                .query_map([song_id], |row| row.get(0))
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(ids)
        })
    }

    /// Replace one song's artist credits. Other songs are untouched, and an
    /// empty list is a no-op rather than an erase.
    pub fn replace_song_artists(&self, song_id: &str, artists: &[ml_core::Artist]) -> MlResult<()> {
        if artists.is_empty() {
            return Ok(());
        }
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            // An empty list would erase trusted local credits, so this branch is
            // only reached with a real credit list.
            tx.execute("DELETE FROM song_artist_map WHERE song_id = ?1", [song_id])
                .map_err(store_error)?;
            for (position, artist) in artists.iter().enumerate() {
                let artist_id = match artist.id.clone() {
                    Some(id) => id,
                    None => tx
                        .query_row(
                            "SELECT id FROM artist WHERE name = ?1 LIMIT 1",
                            [&artist.name],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(store_error)?
                        .unwrap_or_else(|| crate::local_id("LA")),
                };
                tx.execute(
                    "INSERT INTO artist (id, name, channel_id) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(id) DO NOTHING",
                    params![artist_id, artist.name, artist.id],
                )
                .map_err(store_error)?;
                tx.execute(
                    "INSERT OR IGNORE INTO song_artist_map (song_id, artist_id, position) \
                     VALUES (?1, ?2, ?3)",
                    params![song_id, artist_id, position as i64],
                )
                .map_err(store_error)?;
            }
            tx.commit().map_err(store_error)
        })
    }
}

fn query_artist_songs(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> MlResult<Vec<Song>> {
    let mut stmt = conn.prepare(sql).map_err(store_error)?;
    let mut songs = stmt
        .query_map(params, |row| song_at(row, 0))
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    hydrate_songs(conn, &mut songs)?;
    Ok(songs)
}
