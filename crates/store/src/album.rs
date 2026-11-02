//! Album rows, their artist credits and their song list.

use ml_core::{Album, Artist, MlResult, Song};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};

use crate::rows::{
    album_from_row, fold, hydrate_songs, like_pattern, now_ms, song_at, store_error, AlbumState,
    ALBUM_COLS, SONG_COLS, SQL_CHUNK,
};
use crate::song::placeholders;
use crate::Store;

impl Store {
    /// Write an album. Locally owned state (`bookmarked_at`, `in_library_at`,
    /// `theme_color`) is never cleared by an upsert.
    pub fn upsert_album(&self, album: &Album) -> MlResult<()> {
        self.with_conn(|conn| write_album(conn, album, None, None))
    }

    /// [`Store::upsert_album`] with an injected `updated_at`.
    pub fn upsert_album_at(&self, album: &Album, updated_at_ms: i64) -> MlResult<()> {
        self.with_conn(|conn| write_album(conn, album, None, Some(updated_at_ms)))
    }

    /// Replace an album's artist credits. Only this album's links change; the
    /// artists themselves (and their other albums) are untouched.
    pub fn set_album_artists(&self, album_id: &str, artists: &[Artist]) -> MlResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            tx.execute(
                "DELETE FROM album_artist_map WHERE album_id = ?1",
                [album_id],
            )
            .map_err(store_error)?;
            for (position, artist) in artists.iter().enumerate() {
                if let Some(artist) = resolve_artist(&tx, artist)? {
                    tx.execute(
                        "INSERT OR IGNORE INTO album_artist_map (album_id, artist_id, position) \
                         VALUES (?1, ?2, ?3)",
                        params![album_id, artist, position as i64],
                    )
                    .map_err(store_error)?;
                }
            }
            tx.commit().map_err(store_error)
        })
    }

    /// Album plus its bookmark/library state.
    pub fn get_album(&self, id: &str) -> MlResult<Option<Album>> {
        self.with_conn(|conn| {
            let sql = format!("SELECT {ALBUM_COLS} FROM album WHERE album.id = ?1");
            conn.query_row(&sql, [id], album_from_row)
                .optional()
                .map_err(store_error)
        })
    }

    /// Bookmark/library/theme state of one album.
    pub fn album_state(&self, id: &str) -> MlResult<Option<AlbumState>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT bookmarked_at, in_library_at, theme_color FROM album WHERE id = ?1",
                [id],
                |row| {
                    Ok(AlbumState {
                        bookmarked_at: row.get(0)?,
                        in_library_at: row.get(1)?,
                        theme_color: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(store_error)
        })
    }

    /// Albums with songs, newest library additions first.
    pub fn albums(&self, limit: u32, offset: u32) -> MlResult<Vec<Album>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {ALBUM_COLS} FROM album ORDER BY album.title COLLATE NOCASE, album.id \
                 LIMIT ?1 OFFSET ?2"
            );
            query_albums(conn, &sql, params![i64::from(limit), i64::from(offset)])
        })
    }

    /// Bookmarked albums, by title.
    pub fn bookmarked_albums(&self, limit: u32, offset: u32) -> MlResult<Vec<Album>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {ALBUM_COLS} FROM album WHERE album.bookmarked_at IS NOT NULL \
                 ORDER BY album.title COLLATE NOCASE, album.id LIMIT ?1 OFFSET ?2"
            );
            query_albums(conn, &sql, params![i64::from(limit), i64::from(offset)])
        })
    }

    /// Toggle the bookmark.
    pub fn set_album_bookmarked(&self, id: &str, bookmarked: bool) -> MlResult<()> {
        self.set_album_bookmarked_at(id, if bookmarked { Some(now_ms()) } else { None })
    }

    /// [`Store::set_album_bookmarked`] with an injected timestamp.
    pub fn set_album_bookmarked_at(&self, id: &str, at_ms: Option<i64>) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE album SET bookmarked_at = ?2 WHERE id = ?1",
                params![id, at_ms],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Add to or remove from the library.
    pub fn set_album_in_library(&self, id: &str, in_library: bool) -> MlResult<()> {
        self.set_album_in_library_at(id, in_library.then(now_ms))
    }

    /// [`Store::set_album_in_library`] with an injected timestamp.
    pub fn set_album_in_library_at(&self, id: &str, at_ms: Option<i64>) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE album SET in_library_at = ?2 WHERE id = ?1",
                params![id, at_ms],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Store the local theme colour (an ARGB integer), if any.
    pub fn set_album_theme_color(&self, id: &str, color: Option<i64>) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE album SET theme_color = ?2 WHERE id = ?1",
                params![id, color],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Albums whose title matches and that hold a library song.
    pub fn search_albums(&self, query: &str, limit: u32) -> MlResult<Vec<Album>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {ALBUM_COLS} FROM album WHERE ml_fold(album.title) LIKE ?1 ESCAPE '!' \
                 AND EXISTS (SELECT 1 FROM song WHERE song.album_id = album.id \
                             AND song.in_library_at IS NOT NULL) \
                 ORDER BY ml_fold(album.title), album.id LIMIT ?2"
            );
            query_albums(
                conn,
                &sql,
                params![like_pattern(&fold(query)), i64::from(limit)],
            )
        })
    }

    /// Album songs in track order, with the artist credits resolved.
    pub fn album_songs(&self, album_id: &str) -> MlResult<Vec<Song>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS} FROM song_album_map m JOIN song ON song.id = m.song_id \
                 WHERE m.album_id = ?1 ORDER BY m.index_no, song.id"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let mut songs = stmt
                .query_map([album_id], |row| song_at(row, 0))
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            hydrate_songs(conn, &mut songs)?;
            Ok(songs)
        })
    }

    /// Record a song's place on an album and its artist list.
    ///
    /// `artists` replaces the album's credits only when it is trusted (an empty
    /// slice means "the response had no credits", which must not erase local
    /// edits, matching upstream's `artists = null` handling).
    pub fn set_album_songs(
        &self,
        album: &Album,
        songs: &[Song],
        artists: Option<&[Artist]>,
        updated_at_ms: i64,
    ) -> MlResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            write_album(&tx, album, None, Some(updated_at_ms))?;
            for (index, song) in songs.iter().enumerate() {
                crate::song::write_song(&tx, song)?;
                tx.execute(
                    "INSERT INTO song_album_map (song_id, album_id, index_no) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(song_id, album_id) DO UPDATE SET index_no = excluded.index_no",
                    params![song.id, album.id, index as i64],
                )
                .map_err(store_error)?;
            }
            let songs_json = crate::rows::artists_to_json(&album.artists);
            tx.execute(
                "UPDATE album SET artists_json = ?2 WHERE id = ?1",
                params![album.id, songs_json],
            )
            .map_err(store_error)?;
            if let Some(artists) = artists.filter(|artists| !artists.is_empty()) {
                tx.execute("DELETE FROM album_artist_map WHERE album_id = ?1", [&album.id])
                    .map_err(store_error)?;
                for (position, artist) in artists.iter().enumerate() {
                    if let Some(artist_id) = resolve_artist(&tx, artist)? {
                        tx.execute(
                            "INSERT OR IGNORE INTO album_artist_map (album_id, artist_id, position) \
                             VALUES (?1, ?2, ?3)",
                            params![album.id, artist_id, position as i64],
                        )
                        .map_err(store_error)?;
                    }
                }
            }
            tx.commit().map_err(store_error)
        })
    }

    /// Artist ids credited on an album, in order.
    pub fn album_artists(&self, album_id: &str) -> MlResult<Vec<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT artist_id FROM album_artist_map WHERE album_id = ?1 \
                     ORDER BY position, artist_id",
                )
                .map_err(store_error)?;
            let ids = stmt
                .query_map([album_id], |row| row.get(0))
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(ids)
        })
    }

    /// Albums a song appears on, in index order.
    pub fn song_albums(&self, song_id: &str) -> MlResult<Vec<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT album_id FROM song_album_map WHERE song_id = ?1 \
                     ORDER BY index_no, album_id",
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

    /// Albums by id, in the requested order, skipping unknown ids.
    pub fn albums_by_ids(&self, ids: &[String]) -> MlResult<Vec<Album>> {
        self.with_conn(|conn| {
            let mut found: Vec<(String, Album)> = Vec::with_capacity(ids.len());
            for chunk in ids.chunks(SQL_CHUNK) {
                let sql = format!(
                    "SELECT {ALBUM_COLS} FROM album WHERE album.id IN ({})",
                    placeholders(chunk.len())
                );
                let mut stmt = conn.prepare(&sql).map_err(store_error)?;
                let rows = stmt
                    .query_map(
                        params_from_iter(chunk.iter().map(String::as_str)),
                        album_from_row,
                    )
                    .map_err(store_error)?;
                for row in rows {
                    let album = row.map_err(store_error)?;
                    found.push((album.id.clone(), album));
                }
            }
            Ok(ids
                .iter()
                .filter_map(|id| {
                    let index = found.iter().position(|(found_id, _)| found_id == id)?;
                    Some(found.remove(index).1)
                })
                .collect())
        })
    }
}

fn write_album(
    conn: &Connection,
    album: &Album,
    bookmarked_at: Option<i64>,
    updated_at_ms: Option<i64>,
) -> MlResult<()> {
    conn.execute(
        "INSERT INTO album (id, playlist_id, title, year, thumbnail_url, song_count, duration, \
                            explicit, is_uploaded, bookmarked_at, in_library_at, theme_color, \
                            artists_json, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, NULL, NULL, ?10, ?11) \
         ON CONFLICT(id) DO UPDATE SET \
            playlist_id = COALESCE(excluded.playlist_id, album.playlist_id), \
            title = excluded.title, \
            year = COALESCE(excluded.year, album.year), \
            thumbnail_url = COALESCE(excluded.thumbnail_url, album.thumbnail_url), \
            song_count = COALESCE(excluded.song_count, album.song_count), \
            duration = COALESCE(excluded.duration, album.duration), \
            explicit = MAX(album.explicit, excluded.explicit), \
            artists_json = CASE WHEN excluded.artists_json = '[]' \
                                THEN album.artists_json ELSE excluded.artists_json END, \
            updated_at = COALESCE(excluded.updated_at, album.updated_at)",
        params![
            album.id,
            (!album.playlist_id.is_empty()).then_some(&album.playlist_id),
            album.title,
            album.year.map(i64::from),
            album.thumbnail_url,
            album.song_count.map(i64::from),
            album.duration.map(i64::from),
            album.explicit,
            bookmarked_at,
            crate::rows::artists_to_json(&album.artists),
            updated_at_ms,
        ],
    )
    .map_err(store_error)?;
    sync_album_artists(conn, album)
}

/// Insert the album's credited artists. Never deletes links: untrusted
/// responses omit credits, and deleting on omission would lose local edits.
fn sync_album_artists(conn: &Connection, album: &Album) -> MlResult<()> {
    for (position, artist) in album.artists.iter().enumerate() {
        if let Some(artist_id) = resolve_artist(conn, artist)? {
            conn.execute(
                "INSERT OR IGNORE INTO album_artist_map (album_id, artist_id, position) \
                 VALUES (?1, ?2, ?3)",
                params![album.id, artist_id, position as i64],
            )
            .map_err(store_error)?;
        }
    }
    Ok(())
}

/// The artist's id, creating the row when needed. `None` only for a blank name
/// with no id, which is not a real credit.
fn resolve_artist(conn: &Connection, artist: &Artist) -> MlResult<Option<String>> {
    let id = match &artist.id {
        Some(id) if !id.is_empty() => id.clone(),
        _ if artist.name.is_empty() => return Ok(None),
        _ => {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT id FROM artist WHERE name = ?1 LIMIT 1",
                    [&artist.name],
                    |row| row.get(0),
                )
                .optional()
                .map_err(store_error)?;
            existing.unwrap_or_else(|| crate::local_id("LA"))
        }
    };
    conn.execute(
        "INSERT INTO artist (id, name, channel_id) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO NOTHING",
        params![id, artist.name, artist.id],
    )
    .map_err(store_error)?;
    Ok(Some(id))
}

fn query_albums(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> MlResult<Vec<Album>> {
    let mut stmt = conn.prepare(sql).map_err(store_error)?;
    let albums = stmt
        .query_map(params, album_from_row)
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    Ok(albums)
}
