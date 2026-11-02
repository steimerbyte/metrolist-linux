//! `ml-store` — SQLite persistence for the Metrolist Linux port.
//!
//! One [`Store`] owns one bundled-SQLite connection behind a
//! [`parking_lot::Mutex`], so it is `Sync` and every DAO method takes `&self`:
//! the app may call it straight from a UI task or a `spawn_blocking` closure
//! without a dedicated DB thread. Locks are held for one statement (or one
//! transaction), which SQLite's microsecond-scale work makes irrelevant.
//!
//! Design rules (see `arch/api-notes.md` §5): equivalent shapes rather than a
//! copy of the Room schema, timestamps as INTEGER epoch milliseconds,
//! `setVideoId` carried from day one because playlist mutations need it, and
//! every value bound as a parameter.
//!
//! Forward-only migrations keyed off `PRAGMA user_version`; a database written
//! by a newer build is refused rather than silently corrupted.

mod album;
mod artist;
mod browser;
mod history;
mod library;
mod misc;
mod playlist;
mod rows;
mod schema;
mod song;
mod stats;
mod transfer;

pub use browser::{
    firefox_profiles, firefox_profiles_in, netscape_from_rows, newest_firefox_profile,
    read_firefox_cookies, read_firefox_cookies_at, usable, FirefoxCookie, FirefoxProfile,
};
pub use history::{civil_from_days, days_from_civil, PeriodBounds, PeriodBucket};
pub use library::{AutoPlaylist, BookmarkKind};
pub use playlist::{merged_order, moved_indices};
pub use rows::{
    AlbumState, ArtistRecord, CachedLyrics, HistoryEntry, LibraryFilter, PlayCount, PlaylistRecord,
    PlaylistSong, SpeedDial,
};
pub use schema::SCHEMA_VERSION;
pub use stats::{
    days_in_month, longest_run, sub_months, AlbumStat, ArtistStat, MonthPlays, SongStat,
    StatsPeriod, StatsRange, StatsReport, StatsSummary,
};
pub use transfer::{
    export_csv, export_m3u, parse_csv, parse_m3u, Backup, BackupAlbum, BackupArtist,
    BackupBookmark, BackupEntry, BackupPlaylist, BackupSong, ImportReport, ImportedSong,
    RestoreReport, BACKUP_SCHEMA_VERSION, CSV_HEADER,
};

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ml_core::{MlError, MlResult};
use parking_lot::Mutex;
use rusqlite::Connection;

use rows::store_error;

/// SQLite-backed persistence shared by every part of the app.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (creating if needed) the database file, set pragmas and migrate.
    pub fn open(path: &Path) -> MlResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::from_connection(Connection::open(path).map_err(store_error)?)
    }

    /// In-memory store for tests and the desktop "offline" toggle.
    pub fn open_in_memory() -> MlResult<Self> {
        Self::from_connection(Connection::open_in_memory().map_err(store_error)?)
    }

    fn from_connection(conn: Connection) -> MlResult<Self> {
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(store_error)?;
        // `journal_mode` returns a row, so `execute` refuses it; and WAL is
        // unavailable for `:memory:`. Both cases fall back to the rollback
        // journal, which is still correct, so the pragma is best-effort.
        let _ = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0));
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(store_error)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(store_error)?;
        // `LIKE` only case-folds ASCII; local search folds both sides in Rust
        // so "MÜLLER" finds "müller". Deterministic, so SQLite may cache it.
        conn.create_scalar_function(
            "ml_fold",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| Ok(ctx.get::<Option<String>>(0)?.map(|text| rows::fold(&text))),
        )
        .map_err(store_error)?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Bring the schema up to [`SCHEMA_VERSION`]. Idempotent; cheap enough to
    /// call on every startup.
    pub fn migrate(&self) -> MlResult<()> {
        self.with_conn(|conn| {
            let current: u32 = conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(store_error)?;
            if current > SCHEMA_VERSION {
                return Err(MlError::Store(format!(
                    "database schema v{current} is newer than this build's v{SCHEMA_VERSION}"
                )));
            }
            for (version, ddl) in schema::MIGRATIONS
                .iter()
                .filter(|(version, _)| *version > current)
            {
                let tx = conn.transaction().map_err(store_error)?;
                tx.execute_batch(ddl).map_err(store_error)?;
                tx.pragma_update(None, "user_version", *version)
                    .map_err(store_error)?;
                tx.commit().map_err(store_error)?;
            }
            Ok(())
        })
    }

    /// Schema version currently on disk.
    pub fn schema_version(&self) -> MlResult<u32> {
        self.with_conn(|conn| {
            conn.pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(store_error)
        })
    }

    /// Run `f` with the connection locked. The escape hatch for statements the
    /// DAO surface does not cover (future Android-backup import, debugging);
    /// it is as thread-safe as any DAO call and just as synchronous.
    pub(crate) fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> MlResult<T>,
    ) -> MlResult<T> {
        let mut conn = self.conn.lock();
        f(&mut conn)
    }

    /// [`Store::with_conn`], public: the locked connection, for statements the
    /// DAO surface does not cover yet (backup import, diagnostics).
    pub fn with_connection<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> MlResult<T>,
    ) -> MlResult<T> {
        self.with_conn(f)
    }
}

static LOCAL_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A local-only id in the upstream shape: `prefix` + 8 ASCII letters, e.g.
/// `LPAbcdefGh` for a local playlist.
///
/// shortcut: seeded from the clock and a process counter, so two processes
/// writing the same file could collide; upgrade to uuid v4 if that matters.
pub fn local_id(prefix: &str) -> String {
    let seed = ml_core::now_millis().wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ LOCAL_ID_COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0xA24B_AED4_963E_E407);
    local_id_from_seed(prefix, seed)
}

/// The pure half of [`local_id`]: total, deterministic, no clock, no I/O.
///
/// Postcondition (frozen for `proofs/store`): the result is
/// `prefix` followed by exactly [`LOCAL_ID_LETTERS`] ASCII letters, so
/// `result.len() == prefix.len() + 8` and `result.starts_with(prefix)`.
pub fn local_id_from_seed(prefix: &str, seed: u64) -> String {
    let mut state = seed;
    let mut id = String::with_capacity(prefix.len() + LOCAL_ID_LETTERS);
    id.push_str(prefix);
    for _ in 0..LOCAL_ID_LETTERS {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let index = ((state >> 33) % 52) as u8;
        id.push(if index < 26 {
            b'A' + index
        } else {
            b'a' + index - 26
        } as char);
    }
    id
}

/// Random letters appended by [`local_id`]; upstream uses eight.
pub const LOCAL_ID_LETTERS: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;
    use ml_core::Song;
    use std::path::PathBuf;

    fn temp_db(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "metrolist-store-{name}-{}-{}",
            std::process::id(),
            ml_core::now_millis()
        ));
        (dir.clone(), dir.join("library.db"))
    }

    fn song(id: &str) -> Song {
        Song {
            id: id.to_string(),
            title: format!("Title {id}"),
            ..Song::default()
        }
    }

    #[test]
    fn opens_empty_database_and_migrates_to_the_current_version() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let version: String = store
            .with_conn(|conn| {
                conn.query_row("select sqlite_version()", [], |row| row.get(0))
                    .map_err(store_error)
            })
            .unwrap();
        assert!(!version.is_empty());
    }

    #[test]
    fn opens_file_database_and_creates_parent_directories() {
        let (dir, path) = temp_db("file");
        let store = Store::open(&path.join("nested/library.db")).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn migrate_is_idempotent_and_keeps_data() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_song(&song("a")).unwrap();
        store.migrate().unwrap();
        store.migrate().unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(store.get_song("a").unwrap().is_some());
    }

    #[test]
    fn file_database_uses_wal_and_enforces_foreign_keys() {
        let (dir, path) = temp_db("wal");
        let store = Store::open(&path).unwrap();
        let (journal, fk): (String, i64) = store
            .with_conn(|conn| {
                let journal = conn
                    .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
                    .map_err(store_error)?;
                let fk = conn
                    .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                    .map_err(store_error)?;
                Ok((journal, fk))
            })
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
        assert_eq!(fk, 1);
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn foreign_keys_cascade_and_reject_dangling_rows() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_song(&song("s")).unwrap();
        let id = store.create_playlist("Mix").unwrap();
        store.add_songs(&id, &[song("s")]).unwrap();
        store
            .record_play("s", 30_000, ml_core::now_millis() as i64)
            .unwrap();

        // A map row for an unknown song is rejected by the FK.
        assert!(store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO playlist_song_map (playlist_id, song_id, position) VALUES (?1, ?2, 1)",
                    rusqlite::params![id, "ghost"],
                )
                .map_err(store_error)
            })
            .is_err());

        store.delete_playlist(&id).unwrap();
        assert!(store.playlist_songs(&id).unwrap().is_empty());
        store.delete_song("s").unwrap();
        assert!(store
            .history_at(ml_core::now_millis() as i64, 10, 0)
            .unwrap()
            .is_empty());
        assert!(store.play_counts(2026, 11).unwrap().is_empty());
    }

    #[test]
    fn playlist_positions_are_unique_per_song() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_song(&song("s")).unwrap();
        let id = store.create_playlist("Mix").unwrap();
        store.add_songs(&id, &[song("s")]).unwrap();
        let duplicate = store.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playlist_song_map (playlist_id, song_id, position) VALUES (?1, ?2, 0)",
                rusqlite::params![id, "s"],
            )
            .map_err(store_error)
        });
        assert!(duplicate.is_err());
    }

    #[test]
    fn newer_schema_version_is_rejected() {
        let store = Store::open_in_memory().unwrap();
        store
            .with_conn(|conn| {
                conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
                    .map_err(store_error)
            })
            .unwrap();
        assert!(store.migrate().is_err());
    }

    #[test]
    fn two_threads_share_a_sync_store() {
        fn assert_sync<T: Send + Sync>() {}
        assert_sync::<Store>();

        let store = std::sync::Arc::new(Store::open_in_memory().unwrap());
        let mut handles = Vec::new();
        for worker in 0..2 {
            let store = store.clone();
            handles.push(std::thread::spawn(move || {
                for index in 0..50 {
                    let id = format!("w{worker}-{index}");
                    store.upsert_song(&song(&id)).unwrap();
                    assert!(store.get_song(&id).unwrap().is_some());
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(
            store
                .all_songs(ml_core::LibrarySort::RecentlyAdded, 500, 0)
                .unwrap()
                .len(),
            100
        );
    }

    #[test]
    fn local_ids_carry_a_prefix_and_eight_letters() {
        let id = local_id("LP");
        assert_eq!(id.len(), 10);
        assert!(id.starts_with("LP"));
        assert!(id[2..].bytes().all(|byte| byte.is_ascii_alphabetic()));
        assert_ne!(id, local_id("LP"));
    }

    #[test]
    fn local_id_from_seed_is_total_and_keeps_the_shape() {
        // Every seed, including both ends of u64, yields prefix + 8 letters.
        for seed in [0, 1, u64::MAX, u64::MAX / 2, 0x9E37_79B9_7F4A_7C15] {
            for prefix in ["", "L", "LP", "LA", "prefix-"] {
                let id = local_id_from_seed(prefix, seed);
                assert_eq!(id.len(), prefix.len() + LOCAL_ID_LETTERS);
                assert!(id.starts_with(prefix));
                assert!(id[prefix.len()..].bytes().all(|b| b.is_ascii_alphabetic()));
            }
        }
        assert_eq!(local_id_from_seed("LP", 42), local_id_from_seed("LP", 42));
    }
}
