//! Remote and local playlists, their song maps and the position arithmetic
//! behind append/reorder/move.
//!
//! Positions are dense `0..n-1` for local playlists, but a remote playlist's
//! stored map can be sparse (upstream seeds it from the server's own order), so
//! an append is always `max(position) + 1` and never `count`, and every move is
//! range-containment-safe.

use std::collections::HashSet;

use ml_core::{MlResult, Playlist, Song};
use rusqlite::{params, Connection, OptionalExtension};

use crate::rows::{
    fold, hydrate_songs, like_pattern, now_ms, song_at, store_error, PlaylistRecord, PlaylistSong,
    PLAYLIST_COLS, SONG_COLS, SQL_CHUNK,
};
use crate::song::placeholders;
use crate::Store;

impl Store {
    /// Write a playlist header. Remote song counts and endpoints are kept when
    /// the new value is absent.
    pub fn upsert_playlist(&self, playlist: &PlaylistRecord) -> MlResult<()> {
        self.with_conn(|conn| write_playlist(conn, playlist, now_ms()))
    }

    /// Create a local (editable, non-synced) playlist and return its id.
    pub fn create_playlist(&self, name: &str) -> MlResult<String> {
        self.create_playlist_at(name, now_ms())
    }

    /// [`Store::create_playlist`] with an injected timestamp.
    pub fn create_playlist_at(&self, name: &str, created_at_ms: i64) -> MlResult<String> {
        let mut id = crate::local_id("LP");
        while self.get_playlist(&id)?.is_some() {
            id = crate::local_id("LP");
        }
        self.upsert_playlist(&PlaylistRecord {
            id: id.clone(),
            name: name.to_string(),
            created_at: Some(created_at_ms),
            updated_at: Some(created_at_ms),
            is_editable: true,
            ..PlaylistRecord::default()
        })?;
        Ok(id)
    }

    /// Rename a playlist.
    pub fn rename_playlist(&self, id: &str, name: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE playlist SET name = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, name, now_ms()],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Delete a playlist; its song map rows cascade.
    pub fn delete_playlist(&self, id: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM playlist WHERE id = ?1", [id])
                .map_err(store_error)?;
            Ok(())
        })
    }

    /// Playlist header plus its live song count.
    pub fn get_playlist(&self, id: &str) -> MlResult<Option<PlaylistRecord>> {
        self.with_conn(|conn| {
            let sql = format!("SELECT {PLAYLIST_COLS} FROM playlist WHERE playlist.id = ?1");
            conn.query_row(&sql, [id], PlaylistRecord::from_row)
                .optional()
                .map_err(store_error)
        })
    }

    /// All playlists by name, with their live song counts.
    pub fn playlists(&self, limit: u32, offset: u32) -> MlResult<Vec<PlaylistRecord>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {PLAYLIST_COLS} FROM playlist ORDER BY playlist.name COLLATE NOCASE, \
                 playlist.id LIMIT ?1 OFFSET ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let rows = stmt
                .query_map(
                    params![i64::from(limit), i64::from(offset)],
                    PlaylistRecord::from_row,
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(rows)
        })
    }

    /// Bookmarked playlists, by name.
    pub fn bookmarked_playlists(&self, limit: u32, offset: u32) -> MlResult<Vec<PlaylistRecord>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {PLAYLIST_COLS} FROM playlist WHERE playlist.bookmarked_at IS NOT NULL \
                 ORDER BY playlist.name COLLATE NOCASE, playlist.id LIMIT ?1 OFFSET ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let rows = stmt
                .query_map(
                    params![i64::from(limit), i64::from(offset)],
                    PlaylistRecord::from_row,
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(rows)
        })
    }

    /// Toggle the playlist bookmark.
    pub fn set_playlist_bookmarked(&self, id: &str, bookmarked: bool) -> MlResult<()> {
        self.set_playlist_bookmarked_at(id, bookmarked.then(now_ms))
    }

    /// [`Store::set_playlist_bookmarked`] with an injected timestamp.
    pub fn set_playlist_bookmarked_at(&self, id: &str, at_ms: Option<i64>) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE playlist SET bookmarked_at = ?2 WHERE id = ?1",
                params![id, at_ms],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Playlists whose name matches.
    pub fn search_playlists(&self, query: &str, limit: u32) -> MlResult<Vec<Playlist>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {PLAYLIST_COLS} FROM playlist WHERE ml_fold(playlist.name) LIKE ?1 ESCAPE '!' \
                 ORDER BY ml_fold(playlist.name), playlist.id LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let records = stmt
                .query_map(
                    params![like_pattern(&fold(query)), i64::from(limit)],
                    PlaylistRecord::from_row,
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(records.iter().map(PlaylistRecord::as_playlist).collect())
        })
    }

    /// Entries in playlist order: song, position and the membership's
    /// `set_video_id` (needed to edit or remove on the remote side).
    pub fn playlist_songs(&self, playlist_id: &str) -> MlResult<Vec<PlaylistSong>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS}, m.position, m.set_video_id \
                 FROM playlist_song_map m JOIN song ON song.id = m.song_id \
                 WHERE m.playlist_id = ?1 ORDER BY m.position, m.id"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let mut entries = stmt
                .query_map([playlist_id], |row| {
                    Ok(PlaylistSong {
                        song: song_at(row, 0)?,
                        position: row.get(16)?,
                        set_video_id: row.get(17)?,
                    })
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            let mut songs: Vec<Song> = entries.iter().map(|entry| entry.song.clone()).collect();
            hydrate_songs(conn, &mut songs)?;
            for (entry, song) in entries.iter_mut().zip(songs) {
                entry.song = song;
            }
            Ok(entries)
        })
    }

    /// Just the song ids, in playlist order.
    pub fn playlist_song_ids(&self, playlist_id: &str) -> MlResult<Vec<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT song_id FROM playlist_song_map WHERE playlist_id = ?1 \
                     ORDER BY position, id",
                )
                .map_err(store_error)?;
            let ids = stmt
                .query_map([playlist_id], |row| row.get(0))
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(ids)
        })
    }

    /// Append songs, skipping ids already in the playlist and ids with no song
    /// row. Positions continue from `max(position) + 1`; `set_video_id` is
    /// carried through, and appended songs join the library as upstream does.
    pub fn add_songs(&self, playlist_id: &str, songs: &[Song]) -> MlResult<usize> {
        self.add_songs_at(playlist_id, songs, now_ms())
    }

    /// [`Store::add_songs`] with an injected timestamp.
    pub fn add_songs_at(
        &self,
        playlist_id: &str,
        songs: &[Song],
        now_ms_value: i64,
    ) -> MlResult<usize> {
        if songs.is_empty() {
            return Ok(0);
        }
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            let present: HashSet<String> = existing_song_ids(&tx, playlist_id)?;
            let mut position: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM playlist_song_map \
                     WHERE playlist_id = ?1",
                    [playlist_id],
                    |row| row.get(0),
                )
                .map_err(store_error)?;
            let mut seen: HashSet<&str> = HashSet::new();
            let mut added = 0usize;
            for song in songs {
                if present.contains(&song.id) || !seen.insert(song.id.as_str()) {
                    continue;
                }
                let exists: bool = tx
                    .query_row("SELECT 1 FROM song WHERE id = ?1", [&song.id], |_| Ok(true))
                    .optional()
                    .map_err(store_error)?
                    .unwrap_or(false);
                if !exists {
                    // The FK would reject it; skipping matches upstream's
                    // "only insert songs we know about".
                    continue;
                }
                tx.execute(
                    "INSERT INTO playlist_song_map (playlist_id, song_id, position, set_video_id) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![playlist_id, song.id, position, song.set_video_id],
                )
                .map_err(store_error)?;
                tx.execute(
                    "UPDATE song SET in_library_at = COALESCE(in_library_at, ?2) WHERE id = ?1",
                    params![song.id, now_ms_value],
                )
                .map_err(store_error)?;
                position += 1;
                added += 1;
            }
            if added > 0 {
                tx.execute(
                    "UPDATE playlist SET updated_at = ?2 WHERE id = ?1",
                    params![playlist_id, now_ms_value],
                )
                .map_err(store_error)?;
            }
            tx.commit().map_err(store_error)?;
            Ok(added)
        })
    }

    /// Prepend songs, shifting existing positions down. Same skipped-id rules
    /// as [`Store::add_songs`].
    pub fn prepend_songs(&self, playlist_id: &str, songs: &[Song]) -> MlResult<usize> {
        if songs.is_empty() {
            return Ok(0);
        }
        let now = now_ms();
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            let present: HashSet<String> = existing_song_ids(&tx, playlist_id)?;
            let mut seen: HashSet<&str> = HashSet::new();
            let fresh: Vec<&Song> = songs
                .iter()
                .filter(|song| !present.contains(&song.id) && seen.insert(song.id.as_str()))
                .collect();
            let known: Vec<&Song> = fresh
                .into_iter()
                .filter(|song| {
                    tx.query_row("SELECT 1 FROM song WHERE id = ?1", [&song.id], |_| Ok(true))
                        .optional()
                        .map(|row| row.is_some())
                        .unwrap_or(false)
                })
                .collect();
            if known.is_empty() {
                tx.commit().map_err(store_error)?;
                return Ok(0);
            }
            tx.execute(
                "UPDATE playlist_song_map SET position = position + ?2 WHERE playlist_id = ?1",
                params![playlist_id, known.len() as i64],
            )
            .map_err(store_error)?;
            for (position, song) in known.iter().enumerate() {
                tx.execute(
                    "INSERT INTO playlist_song_map (playlist_id, song_id, position, set_video_id) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![playlist_id, song.id, position as i64, song.set_video_id],
                )
                .map_err(store_error)?;
                tx.execute(
                    "UPDATE song SET in_library_at = COALESCE(in_library_at, ?2) WHERE id = ?1",
                    params![song.id, now],
                )
                .map_err(store_error)?;
            }
            tx.execute(
                "UPDATE playlist SET updated_at = ?2 WHERE id = ?1",
                params![playlist_id, now],
            )
            .map_err(store_error)?;
            tx.commit().map_err(store_error)?;
            Ok(known.len())
        })
    }

    /// Ids from `song_ids` that the playlist already contains, chunked to stay
    /// under SQLite's bind-variable limit.
    pub fn playlist_duplicates(
        &self,
        playlist_id: &str,
        song_ids: &[String],
    ) -> MlResult<Vec<String>> {
        if song_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            let mut duplicates = Vec::new();
            for chunk in song_ids.chunks(SQL_CHUNK) {
                let sql = format!(
                    "SELECT DISTINCT song_id FROM playlist_song_map \
                     WHERE playlist_id = ?1 AND song_id IN ({})",
                    placeholders(chunk.len() + 1)
                        .split(',')
                        .skip(1)
                        .collect::<Vec<_>>()
                        .join(",")
                );
                // `placeholders(n)` numbers from 1; the playlist id takes ?1.
                let mut binds: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 1);
                let playlist_id_ref: &dyn rusqlite::ToSql = &playlist_id;
                binds.push(playlist_id_ref);
                for id in chunk {
                    binds.push(id);
                }
                let mut stmt = conn.prepare(&sql).map_err(store_error)?;
                let rows = stmt
                    .query_map(binds.as_slice(), |row| row.get::<_, String>(0))
                    .map_err(store_error)?;
                for row in rows {
                    duplicates.push(row.map_err(store_error)?);
                }
            }
            duplicates.sort();
            duplicates.dedup();
            Ok(duplicates)
        })
    }

    /// Remove one song from a playlist and close the gap in its positions.
    pub fn remove_song(&self, playlist_id: &str, song_id: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            let removed = tx
                .execute(
                    "DELETE FROM playlist_song_map WHERE playlist_id = ?1 AND song_id = ?2",
                    params![playlist_id, song_id],
                )
                .map_err(store_error)?;
            if removed > 0 {
                compact_positions(&tx, playlist_id)?;
            }
            tx.commit().map_err(store_error)
        })
    }

    /// Remove every song; the playlist header stays.
    pub fn clear_playlist(&self, playlist_id: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM playlist_song_map WHERE playlist_id = ?1",
                [playlist_id],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Move the entry at `from` to `to`, shifting everything in between.
    ///
    /// Out-of-range or equal indices are no-ops (upstream's `move` `CASE` would
    /// otherwise leave the map inconsistent). Implemented as a full renumber
    /// inside one transaction, so even a sparse map ends dense and no
    /// intermediate state can collide.
    pub fn move_song(&self, playlist_id: &str, from: i64, to: i64) -> MlResult<()> {
        if from == to {
            return Ok(());
        }
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            let ids: Vec<i64> = {
                let mut stmt = tx
                    .prepare(
                        "SELECT id FROM playlist_song_map WHERE playlist_id = ?1 \
                         ORDER BY position, id",
                    )
                    .map_err(store_error)?;
                let rows = stmt
                    .query_map([playlist_id], |row| row.get(0))
                    .map_err(store_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(store_error)?;
                rows
            };
            let Ok(from) = usize::try_from(from) else {
                tx.commit().map_err(store_error)?;
                return Ok(());
            };
            let Ok(to) = usize::try_from(to) else {
                tx.commit().map_err(store_error)?;
                return Ok(());
            };
            let Some(order) = moved_indices(ids.len(), from, to) else {
                tx.commit().map_err(store_error)?;
                return Ok(());
            };
            for (position, index) in order.iter().enumerate() {
                tx.execute(
                    "UPDATE playlist_song_map SET position = ?2 WHERE id = ?1",
                    params![ids[*index], position as i64],
                )
                .map_err(store_error)?;
            }
            tx.execute(
                "UPDATE playlist SET updated_at = ?2 WHERE id = ?1",
                params![playlist_id, now_ms()],
            )
            .map_err(store_error)?;
            tx.commit().map_err(store_error)
        })
    }

    /// Rewrite the playlist to exactly `song_ids` order, appending any ids not
    /// already present (and skipping unknown ones). This is the "set the order"
    /// operation playlist sync needs.
    pub fn reorder(&self, playlist_id: &str, song_ids: &[String]) -> MlResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            let known: HashSet<String> = {
                let mut stmt = tx
                    .prepare(
                        "SELECT DISTINCT song_id FROM playlist_song_map WHERE playlist_id = ?1",
                    )
                    .map_err(store_error)?;
                let ids = stmt
                    .query_map([playlist_id], |row| row.get(0))
                    .map_err(store_error)?
                    .collect::<Result<HashSet<_>, _>>()
                    .map_err(store_error)?;
                ids
            };
            // Only ids that have a song row may enter the map (the FK would
            // reject the rest).
            let known_songs: Vec<String> = known
                .iter()
                .filter(|id| {
                    tx.query_row("SELECT 1 FROM song WHERE id = ?1", [id], |_| Ok(true))
                        .optional()
                        .map(|row| row.is_some())
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            let ordered: Vec<String> = merged_order(&known_songs, song_ids);
            for (position, song_id) in ordered.iter().enumerate() {
                if known.contains(song_id) {
                    tx.execute(
                        "UPDATE playlist_song_map SET position = ?3 \
                         WHERE playlist_id = ?1 AND song_id = ?2",
                        params![playlist_id, song_id, position as i64],
                    )
                    .map_err(store_error)?;
                } else {
                    tx.execute(
                        "INSERT INTO playlist_song_map (playlist_id, song_id, position) \
                         VALUES (?1, ?2, ?3)",
                        params![playlist_id, song_id, position as i64],
                    )
                    .map_err(store_error)?;
                }
            }
            tx.commit().map_err(store_error)
        })
    }

    /// Point the playlist at a new song list (a server refresh). Songs that
    /// vanished from the map are removed, surviving entries keep their order
    /// and take the server's `set_video_id`.
    pub fn replace_playlist_songs(
        &self,
        playlist_id: &str,
        entries: &[(String, Option<String>)],
    ) -> MlResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            tx.execute(
                "DELETE FROM playlist_song_map WHERE playlist_id = ?1",
                [playlist_id],
            )
            .map_err(store_error)?;
            let mut position = 0i64;
            for (song_id, set_video_id) in entries {
                let exists: bool = tx
                    .query_row("SELECT 1 FROM song WHERE id = ?1", [song_id], |_| Ok(true))
                    .optional()
                    .map_err(store_error)?
                    .unwrap_or(false);
                if !exists {
                    continue;
                }
                tx.execute(
                    "INSERT INTO playlist_song_map (playlist_id, song_id, position, set_video_id) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![playlist_id, song_id, position, set_video_id],
                )
                .map_err(store_error)?;
                position += 1;
            }
            tx.execute(
                "UPDATE playlist SET updated_at = ?2 WHERE id = ?1",
                params![playlist_id, now_ms()],
            )
            .map_err(store_error)?;
            tx.commit().map_err(store_error)
        })
    }

    /// Songs of a playlist as plain `(Song, position, set_video_id)` triples,
    /// the shape the UI pages over.
    pub fn playlist_page(
        &self,
        playlist_id: &str,
        limit: u32,
        offset: u32,
    ) -> MlResult<Vec<PlaylistSong>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS}, m.position, m.set_video_id \
                 FROM playlist_song_map m JOIN song ON song.id = m.song_id \
                 WHERE m.playlist_id = ?1 ORDER BY m.position, m.id LIMIT ?2 OFFSET ?3"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let mut entries = stmt
                .query_map(
                    params![playlist_id, i64::from(limit), i64::from(offset)],
                    |row| {
                        Ok(PlaylistSong {
                            song: song_at(row, 0)?,
                            position: row.get(16)?,
                            set_video_id: row.get(17)?,
                        })
                    },
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            let mut songs: Vec<Song> = entries.iter().map(|entry| entry.song.clone()).collect();
            hydrate_songs(conn, &mut songs)?;
            for (entry, song) in entries.iter_mut().zip(songs) {
                entry.song = song;
            }
            Ok(entries)
        })
    }

    /// Songs that belong to at least one bookmarked playlist.
    pub fn songs_in_bookmarked_playlists(&self) -> MlResult<Vec<Song>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT DISTINCT {SONG_COLS} FROM song \
                 JOIN playlist_song_map m ON m.song_id = song.id \
                 JOIN playlist p ON p.id = m.playlist_id \
                 WHERE p.bookmarked_at IS NOT NULL ORDER BY song.title COLLATE NOCASE"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let mut songs = stmt
                .query_map([], |row| song_at(row, 0))
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            hydrate_songs(conn, &mut songs)?;
            Ok(songs)
        })
    }
}

/// The new index order after moving `from` to `to`, or `None` when the move is
/// out of range or a no-op.
///
/// Pure and total: it is the whole of [`Store::move_song`]'s arithmetic, so the
/// `proofs/store` twin can state its postcondition without SQL — the result is
/// a permutation of `0..len`, it has the same length as the input, the element
/// at `from` is at `to`, and every other element keeps its relative order.
pub fn moved_indices(len: usize, from: usize, to: usize) -> Option<Vec<usize>> {
    if from == to || from >= len || to >= len {
        return None;
    }
    let mut order: Vec<usize> = (0..len).collect();
    let moved = order.remove(from);
    order.insert(to, moved);
    Some(order)
}

/// The order [`Store::reorder`] writes: `requested` in order, then any
/// `existing` entry it did not name, in existing order.
///
/// Pure and total. Postcondition for `proofs/store`: the result is a
/// permutation of `existing` (no duplicates, no additions, no drops), it starts
/// with the requested ids that exist, and it is stable for the tail.
pub fn merged_order(existing: &[String], requested: &[String]) -> Vec<String> {
    let mut ordered: Vec<String> = Vec::with_capacity(existing.len());
    for id in requested {
        if existing.iter().any(|known| known == id) && !ordered.iter().any(|seen| seen == id) {
            ordered.push(id.clone());
        }
    }
    for id in existing {
        if !ordered.iter().any(|seen| seen == id) {
            ordered.push(id.clone());
        }
    }
    ordered
}

fn write_playlist(conn: &Connection, playlist: &PlaylistRecord, now: i64) -> MlResult<()> {
    let song_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM playlist_song_map WHERE playlist_id = ?1",
            [&playlist.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(store_error)?
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO playlist (id, name, browse_id, author, thumbnail_url, song_count, \
                               remote_song_count, created_at, updated_at, is_editable, \
                               is_auto_sync, play_endpoint_params, shuffle_endpoint_params, \
                               radio_endpoint_params, bookmarked_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
         ON CONFLICT(id) DO UPDATE SET \
            name = excluded.name, \
            browse_id = COALESCE(excluded.browse_id, playlist.browse_id), \
            author = COALESCE(excluded.author, playlist.author), \
            thumbnail_url = COALESCE(excluded.thumbnail_url, playlist.thumbnail_url), \
            song_count = excluded.song_count, \
            remote_song_count = COALESCE(excluded.remote_song_count, playlist.remote_song_count), \
            created_at = COALESCE(playlist.created_at, excluded.created_at), \
            updated_at = COALESCE(excluded.updated_at, ?9), \
            is_editable = excluded.is_editable, \
            is_auto_sync = excluded.is_auto_sync, \
            play_endpoint_params = COALESCE(excluded.play_endpoint_params, playlist.play_endpoint_params), \
            shuffle_endpoint_params = COALESCE(excluded.shuffle_endpoint_params, playlist.shuffle_endpoint_params), \
            radio_endpoint_params = COALESCE(excluded.radio_endpoint_params, playlist.radio_endpoint_params), \
            bookmarked_at = COALESCE(excluded.bookmarked_at, playlist.bookmarked_at)",
        params![
            playlist.id,
            playlist.name,
            playlist.browse_id,
            playlist.author,
            playlist.thumbnail_url,
            song_count.max(playlist.song_count),
            playlist.remote_song_count,
            playlist.created_at,
            playlist.updated_at.unwrap_or(now),
            playlist.is_editable,
            playlist.is_auto_sync,
            playlist.play_endpoint_params,
            playlist.shuffle_endpoint_params,
            playlist.radio_endpoint_params,
            playlist.bookmarked_at,
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Ids already linked to a playlist.
fn existing_song_ids(conn: &Connection, playlist_id: &str) -> MlResult<HashSet<String>> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT song_id FROM playlist_song_map WHERE playlist_id = ?1")
        .map_err(store_error)?;
    let ids = stmt
        .query_map([playlist_id], |row| row.get(0))
        .map_err(store_error)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(store_error)?;
    Ok(ids)
}

/// Renumber a playlist's entries to `0..n-1` in their current order.
fn compact_positions(conn: &Connection, playlist_id: &str) -> MlResult<()> {
    let ids: Vec<i64> = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM playlist_song_map WHERE playlist_id = ?1 ORDER BY position, id",
            )
            .map_err(store_error)?;
        let ids = stmt
            .query_map([playlist_id], |row| row.get(0))
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?;
        ids
    };
    for (position, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE playlist_song_map SET position = ?2 WHERE id = ?1",
            params![id, position as i64],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn moved_indices_is_a_permutation_with_the_moved_element_in_place() {
        for len in 1..8usize {
            for from in 0..len {
                for to in 0..len {
                    let Some(order) = moved_indices(len, from, to) else {
                        assert_eq!(from, to, "only equal indices are refused");
                        continue;
                    };
                    assert_eq!(order.len(), len);
                    let mut sorted = order.clone();
                    sorted.sort_unstable();
                    assert_eq!(sorted, (0..len).collect::<Vec<_>>(), "permutation");
                    assert_eq!(order[to], from, "moved element lands on `to`");
                    let expected: Vec<usize> = {
                        let mut rest: Vec<usize> = (0..len).collect();
                        rest.remove(from);
                        let mut result = rest;
                        result.insert(to, from);
                        result
                    };
                    assert_eq!(order, expected, "every other element keeps its order");
                }
            }
        }
        assert_eq!(moved_indices(0, 0, 0), None);
        assert_eq!(moved_indices(4, 4, 0), None);
        assert_eq!(moved_indices(4, 0, 4), None);
        assert_eq!(moved_indices(3, 1, 1), None);
    }

    #[test]
    fn merged_order_is_a_permutation_preferring_the_request() {
        let existing = ids(&["a", "b", "c"]);
        assert_eq!(
            merged_order(&existing, &ids(&["c", "a"])),
            ids(&["c", "a", "b"])
        );
        // Unknown ids are dropped, duplicates collapse, empty request keeps order.
        assert_eq!(
            merged_order(&existing, &ids(&["ghost", "b", "b"])),
            ids(&["b", "a", "c"])
        );
        assert_eq!(merged_order(&existing, &[]), existing);
        assert_eq!(merged_order(&[], &ids(&["a"])), Vec::<String>::new());
        assert_eq!(merged_order(&[], &[]), Vec::<String>::new());
    }
}
