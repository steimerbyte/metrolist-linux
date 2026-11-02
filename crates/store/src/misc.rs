//! Small persistent side tables: lyrics cache, search history, speed dial and
//! the generic key/value store (app state, persisted queue).

use ml_core::MlResult;
use rusqlite::{params, OptionalExtension};

use crate::rows::{store_error, CachedLyrics, SpeedDial};
use crate::Store;

impl Store {
    /// Cached lyrics for a song, if any provider's result was stored.
    pub fn cached_lyrics(&self, song_id: &str) -> MlResult<Option<CachedLyrics>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT song_id, provider, synced, body, translated, translation_lang, updated_at \
                 FROM lyrics_cache WHERE song_id = ?1",
                [song_id],
                |row| {
                    Ok(CachedLyrics {
                        song_id: row.get(0)?,
                        provider: row.get(1)?,
                        synced: row.get::<_, i64>(2)? != 0,
                        body: row.get(3)?,
                        translated: row.get(4)?,
                        translation_lang: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(store_error)
        })
    }

    /// Store a lyrics payload for a song, replacing any earlier one.
    pub fn put_lyrics(&self, lyrics: &CachedLyrics) -> MlResult<()> {
        self.put_lyrics_at(lyrics, crate::rows::now_ms())
    }

    /// [`Store::put_lyrics`] with an injected timestamp.
    pub fn put_lyrics_at(&self, lyrics: &CachedLyrics, updated_at_ms: i64) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO lyrics_cache (song_id, provider, synced, body, translated, \
                                            translation_lang, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(song_id) DO UPDATE SET \
                    provider = excluded.provider, synced = excluded.synced, body = excluded.body, \
                    translated = excluded.translated, \
                    translation_lang = excluded.translation_lang, \
                    updated_at = excluded.updated_at",
                params![
                    lyrics.song_id,
                    lyrics.provider,
                    lyrics.synced,
                    lyrics.body,
                    lyrics.translated,
                    lyrics.translation_lang,
                    updated_at_ms,
                ],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Drop a song's cached lyrics (e.g. when the provider returns nothing).
    pub fn delete_lyrics(&self, song_id: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM lyrics_cache WHERE song_id = ?1", [song_id])
                .map_err(store_error)?;
            Ok(())
        })
    }

    /// Record a search, moving an existing query back to the top.
    pub fn add_search_history(&self, query: &str) -> MlResult<()> {
        self.add_search_history_at(query, crate::rows::now_ms())
    }

    /// [`Store::add_search_history`] with an injected timestamp. The row id is
    /// reassigned so ordering is "most recent first" without relying on the
    /// clock.
    pub fn add_search_history_at(&self, query: &str, at_ms: i64) -> MlResult<()> {
        if query.trim().is_empty() {
            return Ok(());
        }
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            tx.execute("DELETE FROM search_history WHERE query = ?1", [query])
                .map_err(store_error)?;
            tx.execute(
                "INSERT INTO search_history (query, timestamp) VALUES (?1, ?2)",
                params![query, at_ms],
            )
            .map_err(store_error)?;
            tx.commit().map_err(store_error)
        })
    }

    /// Recent searches, newest first.
    pub fn search_history(&self, limit: u32) -> MlResult<Vec<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT query FROM search_history ORDER BY id DESC LIMIT ?1")
                .map_err(store_error)?;
            let queries = stmt
                .query_map([i64::from(limit)], |row| row.get(0))
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(queries)
        })
    }

    /// Forget every recorded search.
    pub fn clear_search_history(&self) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM search_history", [])
                .map_err(store_error)?;
            Ok(())
        })
    }

    /// Insert or replace a speed-dial tile, appending it when new.
    pub fn put_speed_dial(&self, item: &SpeedDial) -> MlResult<()> {
        self.put_speed_dial_at(item, crate::rows::now_ms())
    }

    /// [`Store::put_speed_dial`] with an injected timestamp. An existing tile
    /// keeps its position.
    pub fn put_speed_dial_at(&self, item: &SpeedDial, created_at_ms: i64) -> MlResult<()> {
        self.with_conn(|conn| {
            let position: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM speed_dial",
                    [],
                    |row| row.get(0),
                )
                .map_err(store_error)?;
            conn.execute(
                "INSERT INTO speed_dial (id, title, subtitle, thumbnail_url, item_type, \
                                         item_json, created_at, position) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(id) DO UPDATE SET \
                    title = excluded.title, subtitle = excluded.subtitle, \
                    thumbnail_url = excluded.thumbnail_url, item_type = excluded.item_type, \
                    item_json = excluded.item_json",
                params![
                    item.id,
                    item.title,
                    item.subtitle,
                    item.thumbnail_url,
                    item.item_type,
                    item.item_json,
                    created_at_ms,
                    position,
                ],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Speed-dial tiles in display order.
    pub fn speed_dial(&self) -> MlResult<Vec<SpeedDial>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, subtitle, thumbnail_url, item_type, item_json, \
                            created_at, position \
                     FROM speed_dial ORDER BY position, created_at, id",
                )
                .map_err(store_error)?;
            let items = stmt
                .query_map([], |row| {
                    Ok(SpeedDial {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        subtitle: row.get(2)?,
                        thumbnail_url: row.get(3)?,
                        item_type: row.get(4)?,
                        item_json: row.get(5)?,
                        created_at: row.get(6)?,
                        position: row.get(7)?,
                    })
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(items)
        })
    }

    /// Remove a tile and close the gap in positions.
    pub fn delete_speed_dial(&self, id: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            tx.execute("DELETE FROM speed_dial WHERE id = ?1", [id])
                .map_err(store_error)?;
            // Renumber from a snapshot: a self-referencing UPDATE would see
            // rows it already rewrote.
            let ids: Vec<String> = {
                let mut stmt = tx
                    .prepare("SELECT id FROM speed_dial ORDER BY position, created_at, id")
                    .map_err(store_error)?;
                let ids = stmt
                    .query_map([], |row| row.get(0))
                    .map_err(store_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(store_error)?;
                ids
            };
            for (position, id) in ids.iter().enumerate() {
                tx.execute(
                    "UPDATE speed_dial SET position = ?2 WHERE id = ?1",
                    params![id, position as i64],
                )
                .map_err(store_error)?;
            }
            tx.commit().map_err(store_error)
        })
    }

    /// Rewrite the tile order to `ids`; tiles not listed keep a stable tail
    /// order.
    pub fn reorder_speed_dial(&self, ids: &[String]) -> MlResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            let existing: Vec<String> = {
                let mut stmt = tx
                    .prepare("SELECT id FROM speed_dial ORDER BY position, created_at, id")
                    .map_err(store_error)?;
                let ids = stmt
                    .query_map([], |row| row.get(0))
                    .map_err(store_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(store_error)?;
                ids
            };
            let mut ordered: Vec<&String> = Vec::with_capacity(existing.len());
            for id in ids {
                if existing.iter().any(|known| known == id) && !ordered.contains(&id) {
                    ordered.push(id);
                }
            }
            for id in &existing {
                if !ordered.contains(&id) {
                    ordered.push(id);
                }
            }
            for (position, id) in ordered.iter().enumerate() {
                tx.execute(
                    "UPDATE speed_dial SET position = ?2 WHERE id = ?1",
                    params![id, position as i64],
                )
                .map_err(store_error)?;
            }
            tx.commit().map_err(store_error)
        })
    }

    /// Read a small piece of app state (persisted queue, last tab, …).
    pub fn kv_get(&self, key: &str) -> MlResult<Option<String>> {
        self.with_conn(|conn| {
            conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(store_error)
        })
    }

    /// Write a small piece of app state.
    pub fn kv_set(&self, key: &str, value: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO kv (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(store_error)?;
            Ok(())
        })
    }

    /// Delete one key.
    pub fn kv_delete(&self, key: &str) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM kv WHERE key = ?1", [key])
                .map_err(store_error)?;
            Ok(())
        })
    }
}
