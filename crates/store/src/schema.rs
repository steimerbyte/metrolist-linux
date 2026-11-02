//! Schema DDL and the forward-only migration list.
//!
//! Shapes follow the Android app's Room schema (see `arch/api-notes.md` §5) but
//! are not a copy: ids are TEXT, every timestamp is INTEGER epoch milliseconds,
//! and columns the desktop port does not need yet (local-file ID3 tags,
//! `format`, `relatedSongMap`, podcast tables) are simply absent. `speed_dial`
//! carries `item_json` so new item kinds need no migration.
//!
//! Adding a migration: append `(next_version, DDL)` to [`MIGRATIONS`]. Never
//! edit a released entry; `PRAGMA user_version` is the only migration state.

/// DDL for version 1. Applied inside one transaction by
/// [`crate::Store::migrate`].
pub(crate) const SCHEMA_V1: &str = r#"
CREATE TABLE song (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    duration INTEGER,
    thumbnail_url TEXT,
    album_id TEXT,
    album_name TEXT,
    artists_json TEXT NOT NULL DEFAULT '[]',
    explicit INTEGER NOT NULL DEFAULT 0,
    set_video_id TEXT,
    music_video_type TEXT,
    is_episode INTEGER NOT NULL DEFAULT 0,
    liked INTEGER NOT NULL DEFAULT 0,
    liked_at INTEGER,
    in_library_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE artist (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    thumbnail_url TEXT,
    channel_id TEXT,
    subscribers TEXT,
    description TEXT,
    bookmarked_at INTEGER,
    last_update INTEGER,
    -- Artist page as YouTube returned it. Read by the artist screen only; every
    -- bulk artist read uses an explicit column list that excludes it.
    cached_page_json TEXT
);

CREATE TABLE album (
    id TEXT PRIMARY KEY,
    playlist_id TEXT,
    title TEXT NOT NULL,
    year INTEGER,
    thumbnail_url TEXT,
    song_count INTEGER,
    duration INTEGER,
    explicit INTEGER NOT NULL DEFAULT 0,
    is_uploaded INTEGER NOT NULL DEFAULT 0,
    bookmarked_at INTEGER,
    in_library_at INTEGER,
    theme_color INTEGER,
    artists_json TEXT NOT NULL DEFAULT '[]',
    updated_at INTEGER
);

CREATE TABLE playlist (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    browse_id TEXT,
    author TEXT,
    thumbnail_url TEXT,
    song_count INTEGER NOT NULL DEFAULT 0,
    remote_song_count INTEGER,
    created_at INTEGER,
    updated_at INTEGER,
    is_editable INTEGER NOT NULL DEFAULT 1,
    is_auto_sync INTEGER NOT NULL DEFAULT 0,
    play_endpoint_params TEXT,
    shuffle_endpoint_params TEXT,
    radio_endpoint_params TEXT,
    bookmarked_at INTEGER
);

CREATE TABLE playlist_song_map (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id TEXT NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
    song_id TEXT NOT NULL REFERENCES song(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    set_video_id TEXT,
    UNIQUE(playlist_id, song_id, position)
);

CREATE TABLE song_artist_map (
    song_id TEXT NOT NULL REFERENCES song(id) ON DELETE CASCADE,
    artist_id TEXT NOT NULL REFERENCES artist(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (song_id, artist_id, position)
);

CREATE TABLE album_artist_map (
    album_id TEXT NOT NULL REFERENCES album(id) ON DELETE CASCADE,
    artist_id TEXT NOT NULL REFERENCES artist(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (album_id, artist_id, position)
);

CREATE TABLE song_album_map (
    song_id TEXT NOT NULL REFERENCES song(id) ON DELETE CASCADE,
    album_id TEXT NOT NULL REFERENCES album(id) ON DELETE CASCADE,
    index_no INTEGER NOT NULL,
    PRIMARY KEY (song_id, album_id)
);

CREATE TABLE event (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    song_id TEXT NOT NULL REFERENCES song(id) ON DELETE CASCADE,
    timestamp INTEGER NOT NULL,
    play_time INTEGER NOT NULL
);

CREATE TABLE play_count (
    song_id TEXT NOT NULL REFERENCES song(id) ON DELETE CASCADE,
    year INTEGER NOT NULL,
    month INTEGER NOT NULL,
    count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (song_id, year, month)
);

CREATE TABLE lyrics_cache (
    song_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    synced INTEGER NOT NULL DEFAULT 0,
    body TEXT NOT NULL,
    translated TEXT,
    translation_lang TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE search_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    query TEXT NOT NULL UNIQUE,
    timestamp INTEGER NOT NULL
);

CREATE TABLE speed_dial (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    subtitle TEXT,
    thumbnail_url TEXT,
    item_type TEXT NOT NULL,
    item_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    position INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE INDEX song_album_id ON song(album_id);
CREATE INDEX song_in_library ON song(in_library_at);
CREATE INDEX song_liked_at ON song(liked_at);
CREATE INDEX artist_name ON artist(name);
CREATE INDEX album_title ON album(title);
CREATE INDEX playlist_name ON playlist(name);
-- Not UNIQUE: `move_song` rewrites a whole position range in one statement and
-- SQLite checks uniqueness per row, so a swap would fail mid-statement.
CREATE INDEX playlist_song_position ON playlist_song_map(playlist_id, position);
CREATE INDEX playlist_song_song ON playlist_song_map(song_id);
CREATE INDEX song_artist_artist ON song_artist_map(artist_id);
CREATE INDEX song_album_album ON song_album_map(album_id);
CREATE INDEX album_artist_artist ON album_artist_map(artist_id);
CREATE INDEX event_song ON event(song_id);
CREATE INDEX event_timestamp ON event(timestamp);
CREATE INDEX event_song_timestamp ON event(song_id, timestamp);
CREATE INDEX speed_dial_position ON speed_dial(position);
"#;

/// `(user_version after applying, DDL)`, ascending. Forward-only.
pub(crate) const MIGRATIONS: &[(u32, &str)] = &[(1, SCHEMA_V1)];

/// Version a freshly migrated database reports.
pub const SCHEMA_VERSION: u32 = 1;
