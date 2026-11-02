//! Song rows and their artist links.

use std::collections::HashMap;

use ml_core::{LibrarySort, MlResult, Song};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};

use crate::rows::SQL_CHUNK;
use crate::rows::{
    fold, hydrate_songs, like_pattern, now_ms, song_from_row, store_error, SONG_COLS,
};
use crate::Store;

impl Store {
    /// Write a song and its artist links. The caller owns the row: this
    /// overwrites `title`, `artists_json` and the remote fields, but never
    /// clears local state (`liked`, `liked_at`, `in_library_at`, `created_at`).
    pub fn upsert_song(&self, song: &Song) -> MlResult<()> {
        self.upsert_songs(std::slice::from_ref(song))
    }

    /// Batch form, one transaction.
    pub fn upsert_songs(&self, songs: &[Song]) -> MlResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            for song in songs {
                write_song(&tx, song)?;
            }
            tx.commit().map_err(store_error)
        })
    }

    /// Refresh the remote fields of a song without touching the locally owned
    /// ones (title, artists, like/library state), as upstream's
    /// `update(overwriteTitle = false, overwriteArtists = false)` does. Inserts
    /// the song when absent.
    pub fn refresh_song(&self, song: &Song) -> MlResult<()> {
        self.with_conn(|conn| {
            let updated = conn
                .execute(
                    "UPDATE song SET \
                        duration = COALESCE(?2, duration), \
                        thumbnail_url = COALESCE(?3, thumbnail_url), \
                        album_id = COALESCE(?4, album_id), \
                        album_name = COALESCE(?5, album_name), \
                        music_video_type = COALESCE(?6, music_video_type), \
                        set_video_id = COALESCE(?7, set_video_id), \
                        is_episode = ?8, \
                        explicit = MAX(explicit, ?9), \
                        updated_at = ?10 \
                     WHERE id = ?1",
                    params![
                        song.id,
                        song.duration.map(i64::from),
                        song.thumbnail_url,
                        song.album.as_ref().map(|album| &album.id),
                        song.album.as_ref().map(|album| &album.title),
                        song.music_video_type,
                        song.set_video_id,
                        song.is_episode,
                        song.explicit,
                        now_ms(),
                    ],
                )
                .map_err(store_error)?;
            if updated == 0 {
                write_song(conn, song)?;
            }
            Ok(())
        })
    }

    /// One song, with artists hydrated from `song_artist_map`.
    pub fn get_song(&self, id: &str) -> MlResult<Option<Song>> {
        self.with_conn(|conn| {
            let sql = format!("SELECT {SONG_COLS} FROM song WHERE song.id = ?1");
            let Some(song) = conn
                .query_row(&sql, [id], song_from_row)
                .optional()
                .map_err(store_error)?
            else {
                return Ok(None);
            };
            let mut songs = vec![song];
            hydrate_songs(conn, &mut songs)?;
            Ok(songs.pop())
        })
    }

    /// The requested songs in the requested order; unknown ids are skipped.
    pub fn songs_by_ids(&self, ids: &[String]) -> MlResult<Vec<Song>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            let mut found: HashMap<String, Song> = HashMap::with_capacity(ids.len());
            for chunk in ids.chunks(SQL_CHUNK) {
                let sql = format!(
                    "SELECT {SONG_COLS} FROM song WHERE song.id IN ({})",
                    placeholders(chunk.len())
                );
                let mut stmt = conn.prepare(&sql).map_err(store_error)?;
                let rows = stmt
                    .query_map(
                        params_from_iter(chunk.iter().map(String::as_str)),
                        song_from_row,
                    )
                    .map_err(store_error)?;
                for row in rows {
                    let song = row.map_err(store_error)?;
                    found.insert(song.id.clone(), song);
                }
            }
            let mut songs: Vec<Song> = ids.iter().filter_map(|id| found.remove(id)).collect();
            hydrate_songs(conn, &mut songs)?;
            Ok(songs)
        })
    }

    /// Every stored song in the requested order (library membership ignored).
    pub fn all_songs(&self, sort: LibrarySort, limit: u32, offset: u32) -> MlResult<Vec<Song>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS} FROM song ORDER BY {} LIMIT ?1 OFFSET ?2",
                sort_columns(sort)
            );
            query_songs(conn, &sql, params![i64::from(limit), i64::from(offset)])
        })
    }

    /// Songs the user saved to the library, in the requested order.
    pub fn library_songs(&self, sort: LibrarySort, limit: u32, offset: u32) -> MlResult<Vec<Song>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS} FROM song WHERE song.in_library_at IS NOT NULL \
                 ORDER BY {} LIMIT ?1 OFFSET ?2",
                sort_columns(sort)
            );
            query_songs(conn, &sql, params![i64::from(limit), i64::from(offset)])
        })
    }

    /// Liked songs, most recently liked first.
    pub fn liked_songs(&self, limit: u32, offset: u32) -> MlResult<Vec<Song>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS} FROM song WHERE song.liked = 1 \
                 ORDER BY song.liked_at DESC, song.id DESC LIMIT ?1 OFFSET ?2"
            );
            query_songs(conn, &sql, params![i64::from(limit), i64::from(offset)])
        })
    }

    /// Library songs whose title, artist or album matches `query`.
    pub fn search_songs(&self, query: &str, limit: u32) -> MlResult<Vec<Song>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS} FROM song \
                 WHERE song.in_library_at IS NOT NULL AND (ml_fold(song.title) LIKE ?1 ESCAPE '!' \
                    OR EXISTS (SELECT 1 FROM song_artist_map m JOIN artist a ON a.id = m.artist_id \
                               WHERE m.song_id = song.id AND ml_fold(a.name) LIKE ?1 ESCAPE '!') \
                    OR ml_fold(song.album_name) LIKE ?1 ESCAPE '!') \
                 ORDER BY ml_fold(song.title), song.id LIMIT ?2"
            );
            query_songs(
                conn,
                &sql,
                params![like_pattern(&fold(query)), i64::from(limit)],
            )
        })
    }

    /// Like or unlike, stamped with the current time. Keeps the song in the
    /// library when liking, as upstream's `toggleLike` does.
    pub fn set_liked(&self, id: &str, liked: bool) -> MlResult<()> {
        self.set_liked_at(id, liked, now_ms())
    }

    /// [`Store::set_liked`] with an injected clock.
    pub fn set_liked_at(&self, id: &str, liked: bool, at_ms: i64) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE song SET liked = ?2, liked_at = ?3, \
                    in_library_at = CASE WHEN ?2 = 1 THEN COALESCE(in_library_at, ?3) \
                                         ELSE in_library_at END, \
                    updated_at = ?4 \
                 WHERE id = ?1",
                params![id, liked, if liked { Some(at_ms) } else { None }, at_ms,],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Add to or remove from the library without touching the like state.
    pub fn set_in_library(&self, id: &str, in_library: bool) -> MlResult<()> {
        self.set_in_library_at(id, if in_library { Some(now_ms()) } else { None })
    }

    /// [`Store::set_in_library`] with an injected timestamp.
    pub fn set_in_library_at(&self, id: &str, at_ms: Option<i64>) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE song SET in_library_at = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, at_ms, at_ms.unwrap_or_else(now_ms)],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Delete a song; playlist entries, events and play counts cascade.
    pub fn delete_song(&self, id: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM song WHERE id = ?1", [id])
                .map_err(store_error)?;
            Ok(())
        })
    }
}

/// `1,2,3` bind markers for an `IN (...)` list.
pub(crate) fn placeholders(count: usize) -> String {
    let mut sql = String::with_capacity(count * 2);
    for index in 1..=count {
        if index > 1 {
            sql.push(',');
        }
        sql.push_str(&format!("?{index}"));
    }
    sql
}

/// `ORDER BY` clause for a library sort. Static fragments only.
fn sort_columns(sort: LibrarySort) -> &'static str {
    match sort {
        LibrarySort::RecentlyAdded => {
            "song.in_library_at IS NULL, song.in_library_at DESC, song.id"
        }
        LibrarySort::Alphabetical => "song.title COLLATE NOCASE",
        LibrarySort::MostPlayed => {
            // No denormalised total: sum the events, which `event_song` covers.
            "(SELECT COALESCE(SUM(e.play_time), 0) FROM event e WHERE e.song_id = song.id) DESC, \
             song.title COLLATE NOCASE"
        }
        LibrarySort::Artist => {
            "(SELECT a.name FROM song_artist_map m JOIN artist a ON a.id = m.artist_id \
              WHERE m.song_id = song.id ORDER BY m.position LIMIT 1) COLLATE NOCASE, \
             song.title COLLATE NOCASE"
        }
    }
}

fn query_songs(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> MlResult<Vec<Song>> {
    let mut stmt = conn.prepare(sql).map_err(store_error)?;
    let mut songs = stmt
        .query_map(params, song_from_row)
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    hydrate_songs(conn, &mut songs)?;
    Ok(songs)
}

/// Insert or update one song plus its artist links.
pub(crate) fn write_song(conn: &Connection, song: &Song) -> MlResult<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO song (id, title, duration, thumbnail_url, album_id, album_name, \
                           artists_json, explicit, set_video_id, music_video_type, is_episode, \
                           liked, liked_at, in_library_at, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16) \
         ON CONFLICT(id) DO UPDATE SET \
            title = excluded.title, \
            duration = COALESCE(excluded.duration, song.duration), \
            thumbnail_url = COALESCE(excluded.thumbnail_url, song.thumbnail_url), \
            album_id = COALESCE(excluded.album_id, song.album_id), \
            album_name = COALESCE(excluded.album_name, song.album_name), \
            artists_json = excluded.artists_json, \
            explicit = excluded.explicit, \
            set_video_id = COALESCE(excluded.set_video_id, song.set_video_id), \
            music_video_type = COALESCE(excluded.music_video_type, song.music_video_type), \
            is_episode = excluded.is_episode, \
            liked = CASE WHEN excluded.liked_at IS NULL THEN song.liked ELSE 1 END, \
            liked_at = COALESCE(excluded.liked_at, song.liked_at), \
            in_library_at = COALESCE(excluded.in_library_at, song.in_library_at), \
            updated_at = excluded.updated_at",
        params![
            song.id,
            song.title,
            song.duration.map(i64::from),
            song.thumbnail_url,
            song.album.as_ref().map(|album| &album.id),
            song.album.as_ref().map(|album| &album.title),
            crate::rows::artists_to_json(&song.artists),
            song.explicit,
            song.set_video_id,
            song.music_video_type,
            song.is_episode,
            song.liked.is_some(),
            song.liked,
            song.in_library,
            now,
            now,
        ],
    )
    .map_err(store_error)?;
    sync_artist_links(conn, song)
}

/// Replace a song's artist links. An empty credit list never wipes known
/// links: untrusted responses would otherwise erase local edits.
fn sync_artist_links(conn: &Connection, song: &Song) -> MlResult<()> {
    if song.artists.is_empty() {
        return Ok(());
    }
    conn.execute("DELETE FROM song_artist_map WHERE song_id = ?1", [&song.id])
        .map_err(store_error)?;
    for (position, artist) in song.artists.iter().enumerate() {
        let artist_id = match &artist.id {
            Some(id) => id.clone(),
            // Store-local fallback: reuse the existing row for this name, else
            // mint one. No core helper needed.
            None => artist_id_by_name(conn, &artist.name)?.unwrap_or_else(|| crate::local_id("LA")),
        };
        conn.execute(
            "INSERT INTO artist (id, name, channel_id) VALUES (?1, ?2, ?3) \
             ON CONFLICT(id) DO NOTHING",
            params![artist_id, artist.name, artist.id],
        )
        .map_err(store_error)?;
        conn.execute(
            "INSERT OR IGNORE INTO song_artist_map (song_id, artist_id, position) VALUES (?1, ?2, ?3)",
            params![song.id, artist_id, position as i64],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

fn artist_id_by_name(conn: &Connection, name: &str) -> MlResult<Option<String>> {
    conn.query_row(
        "SELECT id FROM artist WHERE name = ?1 LIMIT 1",
        [name],
        |row| row.get(0),
    )
    .optional()
    .map_err(store_error)
}
