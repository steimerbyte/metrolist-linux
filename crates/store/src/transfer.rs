//! Import and export: M3U (extended), CSV, and a JSON backup of local
//! playlists, likes and library membership.
//!
//! This is *our* format set, not Android's. Upstream restores a ZIP holding a
//! Room database and a DataStore file; nothing here parses or produces that,
//! and no Android build can read these files as a backup. The CSV header and
//! M3U body match what `PlaylistExporter.kt` writes so the *playlist* files
//! travel between the two apps; the JSON backup is Linux-only.
//!
//! Every parser is pure, total and allocation-bounded by its input. Every
//! writer is a pure `String` producer: no file I/O lives in this crate, the
//! caller decides where the bytes go.

use ml_core::{AlbumRef, Artist, MlError, MlResult, Song};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::rows::store_error;
use crate::song::write_song;
use crate::Store;

/// Schema version of the JSON backup this build writes. A restore refuses a
/// newer number rather than guessing at unknown fields.
pub const BACKUP_SCHEMA_VERSION: u32 = 1;

/// CSV columns, in order. Upstream's `PlaylistExporter` writes the same four.
pub const CSV_HEADER: &str = "Title,Artist,Album,YouTube Video ID";

/// One song read from an M3U or CSV file, before it becomes a row.
///
/// There is no album field: neither format stores an album *id*, and inventing
/// one would point at an album the database does not have. The CSV's Album
/// column is exported for other tools and read back only as a display string.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedSong {
    pub id: String,
    pub title: String,
    pub artists: Vec<Artist>,
    pub duration: Option<u32>,
}

impl ImportedSong {
    /// The row to store for a song the database does not know yet. Only the
    /// fields the file carried are set; everything else stays absent.
    fn to_song(&self) -> Song {
        Song {
            id: self.id.clone(),
            title: self.title.clone(),
            artists: self.artists.clone(),
            duration: self.duration,
            ..Song::default()
        }
    }
}

/// What an import did, including how many entries it had to drop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub playlist_id: String,
    pub added: usize,
    /// Entries that produced no playlist row: no usable YouTube id (local
    /// file paths, junk lines) or a repeat of an id already added.
    pub skipped: usize,
    /// Songs that had no row and were created from the file's metadata.
    pub created_songs: usize,
}

/// Result of restoring a JSON backup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub songs: usize,
    pub playlists: usize,
    pub entries: usize,
    /// Entries whose song is neither in the backup nor in the database.
    pub skipped_entries: usize,
    /// Playlists skipped because the database holds a remote playlist with
    /// that id, which a restore must not overwrite.
    pub skipped_playlists: usize,
    pub album_bookmarks: usize,
    pub artist_bookmarks: usize,
    pub playlist_bookmarks: usize,
}

// ------------------------------------------------------------------ M3U

/// Extended M3U for one playlist, matching `PlaylistExporter.exportPlaylistAsM3U`.
pub fn export_m3u(name: &str, songs: &[Song]) -> String {
    let mut out = String::with_capacity(32 + songs.len() * 96);
    out.push_str("#EXTM3U\n");
    if !name.is_empty() {
        // `#PLAYLIST` is the extended-M3U title convention (Winamp).
        out.push_str("#PLAYLIST:");
        out.push_str(&single_line(name));
        out.push('\n');
    }
    for song in songs {
        out.push_str("#EXTINF:");
        out.push_str(&song.duration.unwrap_or(0).to_string());
        out.push(',');
        let artists = song
            .artists
            .iter()
            .map(|artist| single_line(&artist.name))
            .collect::<Vec<_>>()
            .join("; ");
        if artists.is_empty() {
            out.push_str(&single_line(&song.title));
        } else {
            out.push_str(&single_line(&artists));
            out.push_str(" - ");
            out.push_str(&single_line(&song.title));
        }
        out.push('\n');
        out.push_str(&format!("https://music.youtube.com/watch?v={}\n", song.id));
    }
    out
}

/// Read back [`export_m3u`] (or any extended M3U whose entries point at
/// YouTube URLs or carry a bare id). Lines that yield no video id are dropped.
pub fn parse_m3u(text: &str) -> Vec<ImportedSong> {
    let mut songs = Vec::new();
    let mut pending: Option<(Option<u32>, Option<String>, String)> = None;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            let (duration, rest) = match info.split_once(',') {
                Some((duration, rest)) => (duration.trim().parse::<u32>().ok(), rest),
                None => (None, info),
            };
            let (artist, title) = match rest.split_once(" - ") {
                Some((artist, title)) => (Some(artist.trim()), title.trim()),
                None => (None, rest.trim()),
            };
            pending = Some((duration, artist.map(str::to_string), title.to_string()));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let Some(id) = ml_core::parse_video_id(line) else {
            // A local path is not playable here; drop the whole entry.
            pending = None;
            continue;
        };
        let (duration, artist, title) = pending.take().unwrap_or((None, None, String::new()));
        songs.push(ImportedSong {
            title: if title.is_empty() { id.clone() } else { title },
            artists: artist
                .map(|names| {
                    names
                        .split(';')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(|name| Artist::new(name, None))
                        .collect()
                })
                .unwrap_or_default(),
            duration,
            id,
        });
    }
    songs
}

// ------------------------------------------------------------------ CSV

/// CSV for one playlist: [`CSV_HEADER`], one quoted row per song.
pub fn export_csv(songs: &[Song]) -> String {
    let mut out = String::with_capacity(CSV_HEADER.len() + 1 + songs.len() * 96);
    out.push_str(CSV_HEADER);
    out.push('\n');
    for song in songs {
        let artists = song
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        out.push_str(&csv_field(&song.title));
        out.push(',');
        out.push_str(&csv_field(&artists));
        out.push(',');
        out.push_str(&csv_field(
            &song
                .album
                .as_ref()
                .map(|album| album.title.clone())
                .unwrap_or_default(),
        ));
        out.push(',');
        out.push_str(&csv_field(&song.id));
        out.push('\n');
    }
    out
}

/// Read a CSV for [`CSV_HEADER`]. Columns are positional; a leading header row
/// is detected by its first cell and skipped. Rows whose fourth column is not a
/// YouTube id are dropped, so a non-Metrolist CSV yields no entries rather than
/// garbage.
pub fn parse_csv(text: &str) -> Vec<ImportedSong> {
    let mut rows = parse_csv_rows(text).into_iter();
    let mut songs = Vec::new();
    for (index, row) in rows.by_ref().enumerate() {
        if index == 0
            && row.first().is_some_and(|cell| {
                cell.trim_start_matches('\u{feff}')
                    .eq_ignore_ascii_case("title")
            })
        {
            continue;
        }
        let Some(id) = row.get(3).and_then(|cell| ml_core::parse_video_id(cell)) else {
            continue;
        };
        let title = row.first().cloned().unwrap_or_default();
        let artists = row
            .get(1)
            .map(|cell| {
                cell.split(';')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| Artist::new(name, None))
                    .collect()
            })
            .unwrap_or_default();
        songs.push(ImportedSong {
            title: if title.is_empty() { id.clone() } else { title },
            artists,
            id,
            duration: None,
        });
    }
    songs
}

/// Entries an M3U body claims: every non-blank line that is not a directive.
fn count_m3u_entries(body: &str) -> usize {
    body.lines()
        .filter(|line| {
            let line = line.trim().trim_start_matches('\u{feff}');
            !line.is_empty() && !line.starts_with('#')
        })
        .count()
}

/// RFC 4180 rows: quoted fields, `""` escapes, `\r\n` and embedded newlines.
///
/// Pure and total; an unterminated quote simply ends at end of input.
fn parse_csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                other => field.push(other),
            }
        } else {
            match ch {
                '"' if field.is_empty() => {
                    quoted = true;
                    started = true;
                }
                ',' => {
                    row.push(std::mem::take(&mut field));
                    started = true;
                }
                '\r' if chars.peek() == Some(&'\n') => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    started = false;
                }
                other => {
                    field.push(other);
                    started = true;
                }
            }
        }
    }
    if started || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn csv_field(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// M3U/CSV are line-oriented: a title with a newline would split its entry.
/// Runs of newlines collapse to one space so `x\r\ny` reads as `x y`.
fn single_line(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_space = false;
    for ch in value.trim().chars() {
        if matches!(ch, '\r' | '\n') {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(ch);
        }
    }
    out
}

// ------------------------------------------------------------------ backup

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupSong {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<Artist>,
    pub album_id: Option<String>,
    pub album_title: Option<String>,
    pub duration: Option<u32>,
    pub thumbnail_url: Option<String>,
    pub liked_at: Option<i64>,
    pub in_library_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupEntry {
    pub song_id: String,
    pub set_video_id: Option<String>,
}

/// One local playlist. `browse_id` is deliberately absent: only playlists this
/// app created are backed up.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupPlaylist {
    pub id: String,
    pub name: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub bookmarked_at: Option<i64>,
    #[serde(default)]
    pub entries: Vec<BackupEntry>,
}

/// Album metadata, so a bookmarked or library album restores as a row rather
/// than as an id nobody can display.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupAlbum {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<Artist>,
    pub playlist_id: Option<String>,
    pub year: Option<u16>,
    pub thumbnail_url: Option<String>,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
    #[serde(default)]
    pub explicit: bool,
    pub bookmarked_at: Option<i64>,
    pub in_library_at: Option<i64>,
}

/// Artist metadata for the same reason: a bookmarked artist must survive as a
/// row with a name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupArtist {
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub channel_id: Option<String>,
    pub subscribers: Option<String>,
    pub description: Option<String>,
    pub bookmarked_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupBookmark {
    pub id: String,
    pub bookmarked_at: Option<i64>,
}

/// The whole JSON backup.
///
/// `songs` carries likes (via `liked_at`) and library membership (via
/// `in_library_at`), so no separate like list can drift from it. Playlists
/// reference those songs by id; anything not listed there is either already in
/// the target database or dropped on restore.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    pub schema: u32,
    pub created_at_ms: i64,
    #[serde(default)]
    pub songs: Vec<BackupSong>,
    #[serde(default)]
    pub playlists: Vec<BackupPlaylist>,
    #[serde(default)]
    pub albums: Vec<BackupAlbum>,
    #[serde(default)]
    pub artists: Vec<BackupArtist>,
    #[serde(default)]
    pub album_bookmarks: Vec<BackupBookmark>,
    #[serde(default)]
    pub artist_bookmarks: Vec<BackupBookmark>,
    #[serde(default)]
    pub playlist_bookmarks: Vec<BackupBookmark>,
}

impl Store {
    // ------------------------------------------------------ playlist export

    /// M3U body for one stored playlist, in playlist order.
    pub fn export_playlist_m3u(&self, playlist_id: &str) -> MlResult<String> {
        let name = self
            .get_playlist(playlist_id)?
            .map(|record| record.name)
            .unwrap_or_default();
        let songs: Vec<Song> = self
            .playlist_songs(playlist_id)?
            .into_iter()
            .map(|entry| entry.song)
            .collect();
        Ok(export_m3u(&name, &songs))
    }

    /// CSV body for one stored playlist, in playlist order.
    pub fn export_playlist_csv(&self, playlist_id: &str) -> MlResult<String> {
        let songs: Vec<Song> = self
            .playlist_songs(playlist_id)?
            .into_iter()
            .map(|entry| entry.song)
            .collect();
        Ok(export_csv(&songs))
    }

    // ------------------------------------------------------ playlist import

    /// Create a local playlist from parsed entries, in one transaction.
    ///
    /// Songs the database does not know are inserted from the file's metadata
    /// so the playlist is complete; entries with no usable id never reach this
    /// call (the parsers drop them).
    pub fn import_playlist(&self, name: &str, entries: &[ImportedSong]) -> MlResult<ImportReport> {
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            let mut id = crate::local_id("LP");
            while tx
                .query_row("SELECT 1 FROM playlist WHERE id = ?1", [&id], |_| Ok(true))
                .optional()
                .map_err(store_error)?
                .unwrap_or(false)
            {
                id = crate::local_id("LP");
            }
            let now = crate::rows::now_ms();
            tx.execute(
                "INSERT INTO playlist (id, name, song_count, created_at, updated_at, is_editable) \
                 VALUES (?1, ?2, 0, ?3, ?3, 1) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, updated_at = excluded.updated_at",
                params![id, name, now],
            )
            .map_err(store_error)?;
            let mut report = ImportReport {
                playlist_id: id.clone(),
                ..ImportReport::default()
            };
            let mut position = 0i64;
            let mut seen = std::collections::HashSet::new();
            for entry in entries {
                if !seen.insert(entry.id.as_str()) {
                    report.skipped += 1;
                    continue;
                }
                let exists: bool = tx
                    .query_row("SELECT 1 FROM song WHERE id = ?1", [&entry.id], |_| Ok(true))
                    .optional()
                    .map_err(store_error)?
                    .unwrap_or(false);
                if !exists {
                    write_song(&tx, &entry.to_song())?;
                    report.created_songs += 1;
                }
                tx.execute(
                    "INSERT INTO playlist_song_map (playlist_id, song_id, position) \
                     VALUES (?1, ?2, ?3)",
                    params![id, entry.id, position],
                )
                .map_err(store_error)?;
                position += 1;
                report.added += 1;
            }
            tx.execute(
                "UPDATE playlist SET song_count = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, position, now],
            )
            .map_err(store_error)?;
            tx.commit().map_err(store_error)?;
            Ok(report)
        })
    }

    /// Parse an M3U body straight into a new playlist. Entries whose URL is not
    /// a YouTube track (a local file path, junk) are counted as skipped.
    pub fn import_m3u(&self, name: &str, body: &str) -> MlResult<ImportReport> {
        let entries = parse_m3u(body);
        let mut report = self.import_playlist(name, &entries)?;
        report.skipped += count_m3u_entries(body).saturating_sub(entries.len());
        Ok(report)
    }

    /// Parse a CSV body straight into a new playlist. Data rows whose last
    /// column is not a YouTube id are counted as skipped.
    pub fn import_csv(&self, name: &str, body: &str) -> MlResult<ImportReport> {
        let entries = parse_csv(body);
        let rows = parse_csv_rows(body);
        let data_rows = rows
            .iter()
            .position(|row| {
                row.first().is_some_and(|cell| {
                    cell.trim_start_matches('\u{feff}')
                        .eq_ignore_ascii_case("title")
                })
            })
            .map_or(rows.len(), |_| rows.len().saturating_sub(1));
        let mut report = self.import_playlist(name, &entries)?;
        report.skipped += data_rows.saturating_sub(entries.len());
        Ok(report)
    }

    // ------------------------------------------------------------- backup

    /// Snapshot every local playlist, like, library entry and bookmark.
    pub fn export_backup(&self, now_ms: i64) -> MlResult<Backup> {
        self.with_conn(|conn| {
            let mut backup = Backup {
                schema: BACKUP_SCHEMA_VERSION,
                created_at_ms: now_ms,
                ..Backup::default()
            };
            // Library/liked songs plus the songs in every local playlist;
            // `DISTINCT` keeps a song with several memberships once.
            let mut stmt = conn
                .prepare(
                    "SELECT s.id, s.title, s.artists_json, s.album_id, s.album_name, s.duration, \
                            s.thumbnail_url, s.liked_at, s.in_library_at \
                     FROM song s WHERE s.liked_at IS NOT NULL OR s.in_library_at IS NOT NULL \
                        OR EXISTS (SELECT 1 FROM playlist_song_map m JOIN playlist p \
                                   ON p.id = m.playlist_id \
                                   WHERE m.song_id = s.id AND p.browse_id IS NULL) \
                     ORDER BY s.id",
                )
                .map_err(store_error)?;
            backup.songs = stmt
                .query_map([], |row| {
                    let artists: Option<String> = row.get(2)?;
                    Ok(BackupSong {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        artists: crate::rows::artists_from_json(artists),
                        album_id: row.get(3)?,
                        album_title: row.get(4)?,
                        duration: row.get::<_, Option<i64>>(5)?.and_then(|d| u32::try_from(d).ok()),
                        thumbnail_url: row.get(6)?,
                        liked_at: row.get(7)?,
                        in_library_at: row.get(8)?,
                    })
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, name, created_at, updated_at, bookmarked_at FROM playlist \
                     WHERE browse_id IS NULL ORDER BY name COLLATE NOCASE, id",
                )
                .map_err(store_error)?;
            backup.playlists = stmt
                .query_map([], |row| {
                    Ok(BackupPlaylist {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        created_at: row.get(2)?,
                        updated_at: row.get(3)?,
                        bookmarked_at: row.get(4)?,
                        entries: Vec::new(),
                    })
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            for playlist in &mut backup.playlists {
                let mut stmt = conn
                    .prepare(
                        "SELECT song_id, set_video_id FROM playlist_song_map \
                         WHERE playlist_id = ?1 ORDER BY position, id",
                    )
                    .map_err(store_error)?;
                playlist.entries = stmt
                    .query_map([&playlist.id], |row| {
                        Ok(BackupEntry {
                            song_id: row.get(0)?,
                            set_video_id: row.get(1)?,
                        })
                    })
                    .map_err(store_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(store_error)?;
            }

            let mut stmt = conn
                .prepare(
                    "SELECT id, title, artists_json, playlist_id, year, thumbnail_url, song_count, \
                            duration, explicit, bookmarked_at, in_library_at \
                     FROM album WHERE bookmarked_at IS NOT NULL OR in_library_at IS NOT NULL \
                     ORDER BY id",
                )
                .map_err(store_error)?;
            backup.albums = stmt
                .query_map([], |row| {
                    Ok(BackupAlbum {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        artists: crate::rows::artists_from_json(row.get(2)?),
                        playlist_id: row.get(3)?,
                        year: row.get::<_, Option<i64>>(4)?.and_then(|y| u16::try_from(y).ok()),
                        thumbnail_url: row.get(5)?,
                        song_count: row
                            .get::<_, Option<i64>>(6)?
                            .and_then(|c| u32::try_from(c).ok()),
                        duration: row.get::<_, Option<i64>>(7)?.and_then(|d| u32::try_from(d).ok()),
                        explicit: row.get::<_, i64>(8)? != 0,
                        bookmarked_at: row.get(9)?,
                        in_library_at: row.get(10)?,
                    })
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, name, thumbnail_url, channel_id, subscribers, description, \
                            bookmarked_at \
                     FROM artist WHERE bookmarked_at IS NOT NULL ORDER BY id",
                )
                .map_err(store_error)?;
            backup.artists = stmt
                .query_map([], |row| {
                    Ok(BackupArtist {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        thumbnail_url: row.get(2)?,
                        channel_id: row.get(3)?,
                        subscribers: row.get(4)?,
                        description: row.get(5)?,
                        bookmarked_at: row.get(6)?,
                    })
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;

            backup.album_bookmarks = bookmarks_of(
                conn,
                "SELECT id, bookmarked_at FROM album WHERE bookmarked_at IS NOT NULL ORDER BY id",
            )?;
            backup.artist_bookmarks = bookmarks_of(
                conn,
                "SELECT id, bookmarked_at FROM artist WHERE bookmarked_at IS NOT NULL ORDER BY id",
            )?;
            backup.playlist_bookmarks = bookmarks_of(
                conn,
                "SELECT id, bookmarked_at FROM playlist WHERE bookmarked_at IS NOT NULL ORDER BY id",
            )?;
            Ok(backup)
        })
    }

    /// [`Store::export_backup`] as pretty JSON.
    pub fn export_backup_json(&self, now_ms: i64) -> MlResult<String> {
        serde_json::to_string_pretty(&self.export_backup(now_ms)?)
            .map_err(|error| MlError::Store(error.to_string()))
    }

    /// Restore a backup in one transaction: all rows or none.
    ///
    /// Additive by design — a like or library entry already present keeps its
    /// timestamp, and a song's other columns are not cleared. Playlist entries
    /// whose song is neither in the backup nor in the database are skipped and
    /// counted. A playlist id that the database already holds as a *remote*
    /// playlist is skipped whole, never overwritten.
    pub fn restore_backup(&self, backup: &Backup) -> MlResult<RestoreReport> {
        if backup.schema > BACKUP_SCHEMA_VERSION {
            return Err(MlError::Store(format!(
                "backup schema v{} is newer than this build's v{}",
                backup.schema, BACKUP_SCHEMA_VERSION
            )));
        }
        self.with_conn(|conn| restore_in(conn, backup))
    }

    /// [`Store::restore_backup`] from JSON.
    pub fn restore_backup_json(&self, json: &str) -> MlResult<RestoreReport> {
        let backup: Backup =
            serde_json::from_str(json).map_err(|error| MlError::Store(error.to_string()))?;
        self.restore_backup(&backup)
    }
}

fn bookmarks_of(conn: &Connection, sql: &str) -> MlResult<Vec<BackupBookmark>> {
    let mut stmt = conn.prepare(sql).map_err(store_error)?;
    let marks = stmt
        .query_map([], |row| {
            Ok(BackupBookmark {
                id: row.get(0)?,
                bookmarked_at: row.get(1)?,
            })
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    Ok(marks)
}

fn restore_in(conn: &mut Connection, backup: &Backup) -> MlResult<RestoreReport> {
    let tx = conn.transaction().map_err(store_error)?;
    let mut report = RestoreReport::default();
    for song in &backup.songs {
        write_song(
            &tx,
            &Song {
                id: song.id.clone(),
                title: song.title.clone(),
                artists: song.artists.clone(),
                album: match (&song.album_id, &song.album_title) {
                    (Some(id), Some(title)) => Some(AlbumRef {
                        id: id.clone(),
                        title: title.clone(),
                    }),
                    (Some(id), None) => Some(AlbumRef {
                        id: id.clone(),
                        title: String::new(),
                    }),
                    (None, Some(title)) => Some(AlbumRef {
                        id: String::new(),
                        title: title.clone(),
                    }),
                    (None, None) => None,
                },
                duration: song.duration,
                thumbnail_url: song.thumbnail_url.clone(),
                liked: song.liked_at,
                in_library: song.in_library_at,
                ..Song::default()
            },
        )?;
        report.songs += 1;
    }

    for album in &backup.albums {
        tx.execute(
            "INSERT INTO album (id, title, artists_json, playlist_id, year, thumbnail_url, \
                                song_count, duration, explicit, bookmarked_at, in_library_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(id) DO UPDATE SET \
                bookmarked_at = COALESCE(album.bookmarked_at, excluded.bookmarked_at), \
                in_library_at = COALESCE(album.in_library_at, excluded.in_library_at)",
            params![
                album.id,
                album.title,
                crate::rows::artists_to_json(&album.artists),
                album.playlist_id,
                album.year,
                album.thumbnail_url,
                album.song_count,
                album.duration,
                album.explicit,
                album.bookmarked_at,
                album.in_library_at,
            ],
        )
        .map_err(store_error)?;
    }
    for artist in &backup.artists {
        tx.execute(
            "INSERT INTO artist (id, name, thumbnail_url, channel_id, subscribers, description, \
                                 bookmarked_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(id) DO UPDATE SET \
                bookmarked_at = COALESCE(artist.bookmarked_at, excluded.bookmarked_at)",
            params![
                artist.id,
                artist.name,
                artist.thumbnail_url,
                artist.channel_id,
                artist.subscribers,
                artist.description,
                artist.bookmarked_at,
            ],
        )
        .map_err(store_error)?;
    }

    for playlist in &backup.playlists {
        let existing: Option<Option<String>> = tx
            .query_row(
                "SELECT browse_id FROM playlist WHERE id = ?1",
                [&playlist.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if matches!(existing, Some(Some(_))) {
            // A remote playlist with this id: the server owns it.
            report.skipped_playlists += 1;
            continue;
        }
        tx.execute(
            "INSERT INTO playlist (id, name, created_at, updated_at, is_editable, bookmarked_at, \
                                   song_count) \
             VALUES (?1, ?2, ?3, ?4, 1, ?5, 0) \
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
                created_at = COALESCE(playlist.created_at, excluded.created_at), \
                updated_at = excluded.updated_at, \
                bookmarked_at = COALESCE(excluded.bookmarked_at, playlist.bookmarked_at)",
            params![
                playlist.id,
                playlist.name,
                playlist.created_at,
                playlist.updated_at,
                playlist.bookmarked_at,
            ],
        )
        .map_err(store_error)?;
        tx.execute(
            "DELETE FROM playlist_song_map WHERE playlist_id = ?1",
            [&playlist.id],
        )
        .map_err(store_error)?;
        let mut position = 0i64;
        let mut seen = std::collections::HashSet::new();
        for entry in &playlist.entries {
            if !seen.insert(entry.song_id.as_str()) {
                continue;
            }
            let exists: bool = tx
                .query_row("SELECT 1 FROM song WHERE id = ?1", [&entry.song_id], |_| {
                    Ok(true)
                })
                .optional()
                .map_err(store_error)?
                .unwrap_or(false);
            if !exists {
                report.skipped_entries += 1;
                continue;
            }
            tx.execute(
                "INSERT INTO playlist_song_map (playlist_id, song_id, position, set_video_id) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![playlist.id, entry.song_id, position, entry.set_video_id],
            )
            .map_err(store_error)?;
            position += 1;
            report.entries += 1;
        }
        tx.execute(
            "UPDATE playlist SET song_count = ?2 WHERE id = ?1",
            params![playlist.id, position],
        )
        .map_err(store_error)?;
        report.playlists += 1;
    }

    report.album_bookmarks = restore_bookmarks(&tx, "album", &backup.album_bookmarks)?;
    report.artist_bookmarks = restore_bookmarks(&tx, "artist", &backup.artist_bookmarks)?;
    report.playlist_bookmarks = restore_bookmarks(&tx, "playlist", &backup.playlist_bookmarks)?;

    tx.commit().map_err(store_error)?;
    Ok(report)
}

/// Stamp bookmarks onto rows that already exist. Unknown ids are ignored: a
/// bookmark alone is not enough to reconstruct an album or artist.
fn restore_bookmarks(conn: &Connection, table: &str, marks: &[BackupBookmark]) -> MlResult<usize> {
    if marks.is_empty() || !matches!(table, "album" | "artist" | "playlist") {
        return Ok(0);
    }
    let sql =
        format!("UPDATE {table} SET bookmarked_at = COALESCE(bookmarked_at, ?2) WHERE id = ?1");
    let mut restored = 0;
    for mark in marks {
        let changed = conn
            .execute(&sql, params![mark.id, mark.bookmarked_at])
            .map_err(store_error)?;
        restored += changed;
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_rows_follow_rfc_4180() {
        assert_eq!(
            parse_csv_rows("a,b\r\nc,\"d\"\"e\"\n"),
            vec![
                vec!["a".to_string(), "b".to_string()],
                vec!["c".to_string(), "d\"e".to_string()],
            ]
        );
        // An embedded newline stays inside the quoted field.
        assert_eq!(
            parse_csv_rows("\"a\nb\",c\n"),
            vec![vec!["a\nb".to_string(), "c".to_string()]]
        );
        // An unterminated quote ends at end of input rather than panicking.
        assert_eq!(parse_csv_rows("\"a"), vec![vec!["a".to_string()]]);
        assert!(parse_csv_rows("").is_empty());
        // A final row without a newline still counts.
        assert_eq!(
            parse_csv_rows("a,b"),
            vec![vec!["a".to_string(), "b".to_string()]]
        );
    }

    #[test]
    fn csv_fields_escape_quotes_and_parse_back() {
        for value in ["", "plain", "a,b", "a\"b", "line\nbreak", "ümlaut"] {
            let line = csv_field(value);
            assert!(line.starts_with('"') && line.ends_with('"'));
            let rows = parse_csv_rows(&line);
            assert_eq!(rows[0][0], value);
        }
    }

    #[test]
    fn m3u_entries_need_a_video_id() {
        let text = "#EXTM3U\n#PLAYLIST:List\n#EXTINF:10,A - Title\n\
                    https://music.youtube.com/watch?v=dQw4w9WgXcQ\n\
                    #EXTINF:20,Named\nnot a link\n\
                    dQw4w9WgXcQ\n";
        let songs = parse_m3u(text);
        assert_eq!(
            songs
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            vec!["dQw4w9WgXcQ", "dQw4w9WgXcQ"],
            "the junk line drops its own entry; the bare id after it is its own"
        );
        assert_eq!(songs[0].title, "Title");
        assert_eq!(songs[0].duration, Some(10));
        assert_eq!(songs[0].artists[0].name, "A");
        assert_eq!(
            songs[1].title, "dQw4w9WgXcQ",
            "no metadata: the id stands in"
        );
        assert!(songs[1].artists.is_empty());
        // An unknown extension line is ignored, not treated as a URL.
        assert!(parse_m3u("#EXTM3U\n#EXTFOO:1\n").is_empty());
        // A BOM-prefixed first line still parses.
        assert_eq!(
            parse_m3u("\u{feff}#EXTM3U\n#EXTINF:5,T\nMV_3Dpw-BRY\n").len(),
            1
        );
    }

    #[test]
    fn export_m3u_is_harmless_with_odd_metadata() {
        let song = ml_core::Song {
            id: "dQw4w9WgXcQ".to_string(),
            title: "a\nb".to_string(),
            artists: vec![ml_core::Artist::new("x\r\ny", None)],
            duration: None,
            ..ml_core::Song::default()
        };
        let body = export_m3u("p\nl", &[song]);
        assert_eq!(
            body.lines().count(),
            4,
            "a newline in metadata must not split an entry"
        );
        assert!(body.contains("#PLAYLIST:p l\n"));
        assert!(body.contains("#EXTINF:0,x y - a b\n"));
        // And it still reads back.
        assert_eq!(parse_m3u(&body)[0].id, "dQw4w9WgXcQ");
    }

    #[test]
    fn backup_json_tolerates_missing_fields() {
        let parsed: Backup = serde_json::from_str("{\"schema\":1,\"created_at_ms\":0}").unwrap();
        assert!(parsed.songs.is_empty() && parsed.playlists.is_empty());
        // An unknown schema number is carried through, not silently dropped.
        let parsed: Backup =
            serde_json::from_str("{\"schema\":99,\"created_at_ms\":0,\"future\":true}").unwrap();
        assert_eq!(parsed.schema, 99);
    }
}
