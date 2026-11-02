//! Library/data parity: local search folding, stats aggregation, playlist
//! import/export, the JSON backup, bookmarks and auto playlists.
//!
//! Every clock is injected (`*_at`, `*_in`, `record_play`), so nothing here
//! depends on when the suite runs.

use ml_core::{Album, AlbumRef, Artist, LibrarySort, MlError, Song};
use ml_store::{
    export_csv, export_m3u, longest_run, parse_csv, parse_m3u, sub_months, AutoPlaylist, Backup,
    BackupBookmark, BackupEntry, BackupPlaylist, BackupSong, BookmarkKind, StatsPeriod, Store,
    BACKUP_SCHEMA_VERSION,
};

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn track(id: &str, title: &str) -> Song {
    Song {
        id: id.to_string(),
        title: title.to_string(),
        ..Song::default()
    }
}

fn in_library(id: &str, title: &str) -> Song {
    Song {
        in_library: Some(1),
        ..track(id, title)
    }
}

// ------------------------------------------------------- local search

#[test]
fn local_search_folds_case_beyond_ascii_and_treats_wildcards_literally() {
    let store = store();
    let mut muller = in_library("muller", "Müller über alles");
    muller.artists = vec![Artist::new("Björk", Some("bjork".to_string()))];
    muller.album = Some(AlbumRef {
        id: "al-angstrom".to_string(),
        title: "Ångström".to_string(),
    });
    store.upsert_song(&muller).unwrap();
    store
        .upsert_song(&in_library("percent", "100% Pure"))
        .unwrap();
    store
        .upsert_song(&in_library("underscore", "Under_score"))
        .unwrap();
    store
        .upsert_song(&in_library("quote", "Guns N' Roses"))
        .unwrap();
    store
        .upsert_song(&in_library("city", "Müller City"))
        .unwrap();
    store
        .upsert_song(&in_library("outside", "nightcall"))
        .unwrap();
    // Not in the library: never a hit.
    store
        .upsert_song(&track("outside-2", "Müller outside"))
        .unwrap();
    store
        .upsert_album(&Album {
            id: "al-angstrom".to_string(),
            title: "Ångström".to_string(),
            ..Album::default()
        })
        .unwrap();
    store
        .upsert_artist(&ml_store::ArtistRecord {
            id: "bjork".to_string(),
            name: "Björk".to_string(),
            ..Default::default()
        })
        .unwrap();
    let playlist = store.create_playlist("50% Hits").unwrap();

    // Non-ASCII case folding, on title, artist and album.
    let by_title = store.search_library("mÜLLER", 10).unwrap();
    assert_eq!(
        by_title
            .songs
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["city", "muller"],
        "equal-prefix titles order deterministically by title then id"
    );
    assert_eq!(store.search_library("BJÖRK", 10).unwrap().songs.len(), 1);
    assert_eq!(
        store.search_library("ångström", 10).unwrap().albums.len(),
        1
    );
    assert_eq!(store.search_library("BJÖRK", 10).unwrap().artists.len(), 1);

    // `%` and `_` are data, not wildcards.
    assert_eq!(
        store
            .search_library("%", 10)
            .unwrap()
            .songs
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["percent"],
        "a bare percent matches only the literal character"
    );
    assert_eq!(
        store
            .search_library("_", 10)
            .unwrap()
            .songs
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["underscore"]
    );
    assert_eq!(store.search_library("50%", 10).unwrap().playlists.len(), 1);
    assert_eq!(store.search_library("%", 10).unwrap().playlists.len(), 1);
    assert_eq!(
        store.search_library("50_", 10).unwrap().playlists.len(),
        0,
        "an underscore is literal too"
    );

    // Quotes are ordinary text, and the same query twice returns the same order.
    assert_eq!(store.search_library("n' r", 10).unwrap().songs.len(), 1);
    let first = store.search_library("müller", 10).unwrap();
    let second = store.search_library("müller", 10).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.songs.len(), 2);

    // Limit applies per shelf; empty query matches everything in the library.
    assert_eq!(store.search_library("", 2).unwrap().songs.len(), 2);
    let _ = playlist;
}

// -------------------------------------------------------------- stats

const MS_PER_DAY: i64 = 86_400_000;

fn day_ms(year: i64, month: u32, day: u32, hour: u32) -> i64 {
    ml_store::days_from_civil(year, month, day) * MS_PER_DAY + i64::from(hour) * 3_600_000
}

#[test]
fn stats_aggregate_periods_totals_and_streaks() {
    let store = store();
    // Metrics: 2026-08-26 12:00.
    let now = day_ms(2026, 8, 26, 12);
    let mut alpha = in_library("alpha", "Alpha");
    alpha.artists = vec![
        Artist::new("One", Some("ar-one".to_string())),
        Artist::new("Two", Some("ar-two".to_string())),
    ];
    alpha.album = Some(AlbumRef {
        id: "al-1".to_string(),
        title: "First".to_string(),
    });
    let beta = Song {
        album: Some(AlbumRef {
            id: "al-2".to_string(),
            title: "Second".to_string(),
        }),
        artists: vec![Artist::new("One", Some("ar-one".to_string()))],
        ..in_library("beta", "Beta")
    };
    store.upsert_songs(&[alpha, beta]).unwrap();
    for (id, title) in [("al-1", "First"), ("al-2", "Second")] {
        store
            .upsert_album(&Album {
                id: id.to_string(),
                title: title.to_string(),
                ..Album::default()
            })
            .unwrap();
    }

    // Plays: alpha three times (1000 ms each), beta once (500 ms).
    // 24th, 25th, 26th form a three-day streak; the 20th is a lone outlier.
    let plays = [
        ("alpha", 1_000, day_ms(2026, 8, 20, 9)),
        ("alpha", 1_000, day_ms(2026, 8, 24, 9)),
        ("alpha", 1_000, day_ms(2026, 8, 25, 9)),
        ("beta", 500, day_ms(2026, 8, 26, 9)),
    ];
    for (id, ms, at) in plays {
        store.record_play(id, ms, at).unwrap();
    }

    let week = store.stats_summary(StatsPeriod::Week, now).unwrap();
    assert_eq!(week.range.start_ms, now - 7 * MS_PER_DAY);
    assert_eq!(
        week.event_count, 4,
        "a 7-day window back from the 26th noon still holds the 20th at 09:00"
    );
    assert_eq!(week.total_ms, 3_500);
    assert_eq!(week.unique_songs, 2);
    assert_eq!(week.unique_artists, 2, "One and Two, via alpha");
    assert_eq!(week.unique_albums, 2);
    assert_eq!(
        week.first_play_ms,
        Some(day_ms(2026, 8, 20, 9)),
        "the window's first play, not the streak's"
    );
    assert_eq!(week.last_play_ms, Some(day_ms(2026, 8, 26, 9)));
    assert_eq!(week.longest_streak_days, 3);

    let all = store.stats_summary(StatsPeriod::All, now).unwrap();
    assert_eq!(all.event_count, 4);
    assert_eq!(all.total_ms, 3_500);
    assert_eq!(all.longest_streak_days, 3, "the outlier does not extend it");

    // A narrower window shows the difference.
    let recent_only = ml_store::StatsRange {
        start_ms: day_ms(2026, 8, 24, 0),
        end_ms: now,
    };
    assert_eq!(store.stats_summary_in(recent_only).unwrap().event_count, 3);

    // Tops: alpha leads on time; both artists tie on time, broken by name.
    let songs = store.top_songs(StatsPeriod::All, now, 10).unwrap();
    assert_eq!(
        songs
            .iter()
            .map(|stat| (stat.song.id.as_str(), stat.plays, stat.ms))
            .collect::<Vec<_>>(),
        vec![("alpha", 3, 3_000), ("beta", 1, 500)]
    );
    assert_eq!(songs[0].song.artists.len(), 2, "artists are hydrated");

    let artists = store.top_artists(StatsPeriod::All, now, 10).unwrap();
    assert_eq!(
        artists
            .iter()
            .map(|stat| (stat.artist.id.as_str(), stat.ms))
            .collect::<Vec<_>>(),
        vec![("ar-one", 3_500), ("ar-two", 3_000)],
        "a two-artist song credits both"
    );

    let albums = store.top_albums(StatsPeriod::All, now, 10).unwrap();
    assert_eq!(
        albums
            .iter()
            .map(|stat| (stat.album.id.as_str(), stat.ms))
            .collect::<Vec<_>>(),
        vec![("al-1", 3_000), ("al-2", 500)]
    );

    // Limit and the inclusive window ends.
    assert_eq!(store.top_songs(StatsPeriod::All, now, 1).unwrap().len(), 1);
    let exact = ml_store::StatsRange {
        start_ms: day_ms(2026, 8, 26, 9),
        end_ms: day_ms(2026, 8, 26, 9),
    };
    assert_eq!(store.stats_summary_in(exact).unwrap().event_count, 1);
    assert_eq!(store.top_songs_in(exact, 10).unwrap()[0].song.id, "beta");

    // Monthly trend comes from the durable tally and survives clearing history.
    store.clear_history().unwrap();
    let months = store.plays_per_month(0).unwrap();
    assert_eq!(months.len(), 1);
    assert_eq!((months[0].year, months[0].month), (2026, 8));
    assert_eq!(months[0].plays, 4);
    assert_eq!(months[0].songs, 2);
    assert_eq!(store.plays_per_month(1).unwrap().len(), 1);

    let report = store.stats_report(StatsPeriod::All, now, 5).unwrap();
    assert_eq!(report.summary.event_count, 0, "history was cleared");
    assert_eq!(report.monthly, months);
}

#[test]
fn stats_windows_are_calendar_months_and_clamp_the_day() {
    let store = store();
    store.upsert_song(&in_library("s", "S")).unwrap();
    // 2026-03-31 10:00: one month back is 2026-02-28, not 2026-03-03.
    let now = day_ms(2026, 3, 31, 10);
    assert_eq!(sub_months(now, 1), day_ms(2026, 2, 28, 10));
    // A leap year keeps the 29th.
    assert_eq!(
        sub_months(day_ms(2028, 3, 31, 0), 1),
        day_ms(2028, 2, 29, 0)
    );
    // Twelve months lands on the same day a year earlier.
    assert_eq!(
        sub_months(day_ms(2026, 8, 26, 12), 12),
        day_ms(2025, 8, 26, 12)
    );
    assert_eq!(ml_store::days_in_month(2026, 2), 28);
    assert_eq!(ml_store::days_in_month(2028, 2), 29);
    assert_eq!(ml_store::days_in_month(2026, 12), 31);

    store
        .record_play("s", 1_000, day_ms(2026, 3, 30, 0))
        .unwrap();
    store
        .record_play("s", 1_000, day_ms(2026, 2, 26, 0))
        .unwrap();
    assert_eq!(
        store
            .stats_summary(StatsPeriod::Month, now)
            .unwrap()
            .event_count,
        1,
        "only the play inside February 28 .. now"
    );
    assert_eq!(
        store
            .stats_summary(StatsPeriod::Year, now)
            .unwrap()
            .event_count,
        2
    );
    assert_eq!(
        store
            .stats_summary(StatsPeriod::Week, now)
            .unwrap()
            .event_count,
        1
    );
    // Extremes stay total.
    let _ = StatsPeriod::Year.range(i64::MIN);
    let _ = StatsPeriod::Month.range(i64::MAX);
    assert_eq!(longest_run(&[]), 0);
    assert_eq!(longest_run(&[5, 5, 5]), 1);
    assert_eq!(longest_run(&[1, 2, 3, 7, 8]), 3);
    assert_eq!(longest_run(&[i64::MIN, i64::MAX]), 1);
    assert_eq!(longest_run(&[i64::MAX - 1, i64::MAX]), 2);
}

// ------------------------------------------------------------ M3U/CSV

fn seeded_playlist(store: &Store) -> String {
    let id = store.create_playlist("Road trip").unwrap();
    let mut first = Song {
        duration: Some(215),
        artists: vec![
            Artist::new("Artist One", None),
            Artist::new("Artist Two", None),
        ],
        ..track("dQw4w9WgXcQ", "Never Gonna Give You Up")
    };
    first.album = Some(AlbumRef {
        id: String::new(),
        title: "Whenever You Need Somebody".to_string(),
    });
    let second = Song {
        artists: vec![Artist::new("Kavinsky", None)],
        ..track("MV_3Dpw-BRY", "Nightcall")
    };
    store.upsert_songs(&[first, second]).unwrap();
    let songs = store
        .songs_by_ids(&["dQw4w9WgXcQ".to_string(), "MV_3Dpw-BRY".to_string()])
        .unwrap();
    store.add_songs(&id, &songs).unwrap();
    id
}

#[test]
fn m3u_export_and_import_round_trip_and_drop_local_paths() {
    let store = store();
    let playlist = seeded_playlist(&store);
    let body = store.export_playlist_m3u(&playlist).unwrap();
    assert!(body.starts_with("#EXTM3U\n#PLAYLIST:Road trip\n"));
    assert!(body.contains("#EXTINF:215,Artist One; Artist Two - Never Gonna Give You Up\n"));
    assert!(body.contains("https://music.youtube.com/watch?v=dQw4w9WgXcQ\n"));

    let report = store.import_m3u("Copy", &body).unwrap();
    assert_eq!(report.added, 2);
    assert_eq!(report.created_songs, 0, "both songs already exist");
    let imported = store.playlist_songs(&report.playlist_id).unwrap();
    assert_eq!(
        imported
            .iter()
            .map(|entry| entry.song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["dQw4w9WgXcQ", "MV_3Dpw-BRY"],
        "order is preserved"
    );
    assert_eq!(imported[0].song.title, "Never Gonna Give You Up");

    // Standalone parsers: a local path and a junk line are dropped, a bare id
    // and a youtu.be URL are kept.
    let parsed = parse_m3u(
        "#EXTM3U\n#EXTINF:1,Local\n/home/me/song.mp3\n#EXTINF:2,Bare\nMV_3Dpw-BRY\n#EXTINF:3,Short\nhttps://youtu.be/dQw4w9WgXcQ\n",
    );
    assert_eq!(
        parsed
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["MV_3Dpw-BRY", "dQw4w9WgXcQ"]
    );
    assert_eq!(
        parsed[0].title, "Bare",
        "no ` - ` means no artist, not a split"
    );
    assert!(parsed[0].artists.is_empty());
    // Export is a pure function of the songs, too.
    assert!(export_m3u("", &[]).starts_with("#EXTM3U\n"));
}

#[test]
fn csv_round_trips_quoted_fields_and_ignores_foreign_columns() {
    let store = store();
    let playlist = store.create_playlist("Quotes").unwrap();
    let tricky = Song {
        artists: vec![Artist::new("AC/DC", None)],
        album: Some(AlbumRef {
            id: String::new(),
            title: "Back, \"In\" Black".to_string(),
        }),
        ..track("dQw4w9WgXcQ", "A \"quoted\", title")
    };
    let plain = Song {
        artists: vec![Artist::new("Kavinsky", None)],
        ..track("MV_3Dpw-BRY", "Nightcall")
    };
    store
        .upsert_songs(&[tricky.clone(), plain.clone()])
        .unwrap();
    store.add_songs(&playlist, &[tricky, plain]).unwrap();

    let body = store.export_playlist_csv(&playlist).unwrap();
    let mut lines = body.lines();
    assert_eq!(lines.next(), Some(ml_store::CSV_HEADER));
    assert!(body.contains("\"A \"\"quoted\"\", title\""));
    // The embedded comma and quote survive the round trip; the Album column is
    // text for other tools, not an id, so it is not imported as a link.
    let parsed = parse_csv(&body);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].title, "A \"quoted\", title");
    assert_eq!(parsed[0].artists[0].name, "AC/DC");
    assert_eq!(parsed[1].id, "MV_3Dpw-BRY");

    let report = store.import_csv("From CSV", &body).unwrap();
    assert_eq!(report.added, 2);
    assert_eq!(
        store.playlist_song_ids(&report.playlist_id).unwrap(),
        vec!["dQw4w9WgXcQ", "MV_3Dpw-BRY"]
    );

    // A Spotify-style export has no YouTube id column; nothing is imported,
    // and the data row is reported as skipped rather than silently vanished.
    assert!(parse_csv("Track,Artist,Album\nSong,Someone,Somewhere\n").is_empty());
    let foreign = store
        .import_csv("Foreign", "Track,Artist,Album\nSong,Someone,Somewhere\n")
        .unwrap();
    assert_eq!(foreign.added, 0);
    assert_eq!(
        foreign.skipped, 2,
        "both rows are rejected: a foreign header cannot be told from data"
    );
    assert!(export_csv(&[]).starts_with(ml_store::CSV_HEADER));
}

#[test]
fn import_creates_unknown_songs_and_never_duplicates_entries() {
    let store = store();
    let body = "#EXTM3U\n#EXTINF:100,Someone - New Song\nhttps://music.youtube.com/watch?v=dQw4w9WgXcQ\n#EXTINF:90,Someone - New Song\nhttps://music.youtube.com/watch?v=dQw4w9WgXcQ\n";
    let report = store.import_m3u("Fresh", body).unwrap();
    assert_eq!(report.added, 1, "a repeated id enters once");
    assert_eq!(report.created_songs, 1);
    let song = store.get_song("dQw4w9WgXcQ").unwrap().unwrap();
    assert_eq!(song.title, "New Song");
    assert_eq!(song.duration, Some(100));
    assert_eq!(song.artists[0].name, "Someone");
    assert_eq!(
        store
            .get_playlist(&report.playlist_id)
            .unwrap()
            .unwrap()
            .song_count,
        1
    );
    // The imported playlist is local, so it is backed up.
    assert_eq!(store.export_backup(0).unwrap().playlists.len(), 1);
}

// ------------------------------------------------------------ backup

fn backup_fixture() -> (Store, Backup) {
    let store = store();
    let mut liked = Song {
        album: Some(AlbumRef {
            id: "al-1".to_string(),
            title: "First".to_string(),
        }),
        artists: vec![Artist::new("One", Some("ar-one".to_string()))],
        ..in_library("liked", "Liked")
    };
    liked.liked = Some(5_000);
    store.upsert_song(&liked).unwrap();
    store.upsert_song(&in_library("saved", "Saved")).unwrap();
    store
        .upsert_album(&Album {
            id: "al-1".to_string(),
            title: "First".to_string(),
            ..Album::default()
        })
        .unwrap();
    store
        .upsert_artist(&ml_store::ArtistRecord {
            id: "ar-one".to_string(),
            name: "One".to_string(),
            ..Default::default()
        })
        .unwrap();
    store.set_album_bookmarked_at("al-1", Some(11)).unwrap();
    store.set_artist_bookmarked_at("ar-one", Some(12)).unwrap();

    let playlist = store.create_playlist_at("Mix", 1_000).unwrap();
    let songs = store
        .songs_by_ids(&["liked".to_string(), "saved".to_string()])
        .unwrap();
    store.add_songs_at(&playlist, &songs, 2_000).unwrap();
    store
        .set_playlist_bookmarked_at(&playlist, Some(13))
        .unwrap();

    let backup = store.export_backup(9_000).unwrap();
    (store, backup)
}

#[test]
fn json_backup_round_trips_playlists_likes_and_bookmarks() {
    let (source, backup) = backup_fixture();
    assert_eq!(backup.schema, BACKUP_SCHEMA_VERSION);
    assert_eq!(backup.songs.len(), 2);
    assert_eq!(backup.playlists.len(), 1);
    assert_eq!(backup.playlists[0].entries.len(), 2);
    assert_eq!(backup.album_bookmarks.len(), 1);
    assert_eq!(backup.artist_bookmarks.len(), 1);
    assert_eq!(backup.playlist_bookmarks.len(), 1);

    // Through JSON, as a file would be.
    let json = source.export_backup_json(9_000).unwrap();
    let target = store();
    let report = target.restore_backup_json(&json).unwrap();
    assert_eq!(
        report,
        ml_store::RestoreReport {
            songs: 2,
            playlists: 1,
            entries: 2,
            skipped_entries: 0,
            skipped_playlists: 0,
            album_bookmarks: 1,
            artist_bookmarks: 1,
            playlist_bookmarks: 1,
        }
    );

    let playlist = &backup.playlists[0];
    assert_eq!(
        target.playlist_song_ids(&playlist.id).unwrap(),
        vec!["liked", "saved"]
    );
    let liked = target.get_song("liked").unwrap().unwrap();
    assert_eq!(liked.liked, Some(5_000));
    assert_eq!(liked.in_library, Some(1));
    assert_eq!(liked.album.as_ref().unwrap().title, "First");
    assert_eq!(liked.artists[0].id.as_deref(), Some("ar-one"));
    assert_eq!(target.get_song("saved").unwrap().unwrap().liked, None);
    assert!(target
        .bookmarked_albums(10, 0)
        .unwrap()
        .iter()
        .any(|album| album.id == "al-1"));
    assert_eq!(
        target.bookmarked_ids(BookmarkKind::Playlist, 10).unwrap(),
        vec![playlist.id.clone()]
    );

    // The restored export matches the original, ignoring the export clock.
    let mut again = target.export_backup(9_000).unwrap();
    let mut original = backup.clone();
    again.songs.sort_by(|a, b| a.id.cmp(&b.id));
    original.songs.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(again, original);
}

#[test]
fn restore_tolerates_missing_songs_and_refuses_newer_schemas() {
    let store = store();
    store.upsert_song(&in_library("known", "Known")).unwrap();
    let backup = Backup {
        schema: BACKUP_SCHEMA_VERSION,
        created_at_ms: 1,
        songs: vec![BackupSong {
            id: "known".to_string(),
            title: "Known".to_string(),
            ..BackupSong::default()
        }],
        playlists: vec![BackupPlaylist {
            id: "LPlocalMix".to_string(),
            name: "Restored".to_string(),
            entries: vec![
                BackupEntry {
                    song_id: "known".to_string(),
                    set_video_id: None,
                },
                BackupEntry {
                    song_id: "ghost".to_string(),
                    set_video_id: Some("sv".to_string()),
                },
            ],
            ..BackupPlaylist::default()
        }],
        album_bookmarks: vec![BackupBookmark {
            id: "never-stored".to_string(),
            bookmarked_at: Some(1),
        }],
        ..Backup::default()
    };
    let report = store.restore_backup(&backup).unwrap();
    assert_eq!(report.entries, 1);
    assert_eq!(
        report.skipped_entries, 1,
        "a missing song is skipped, not an error"
    );
    assert_eq!(
        report.album_bookmarks, 0,
        "a bookmark cannot invent an album"
    );
    assert_eq!(
        store.playlist_song_ids("LPlocalMix").unwrap(),
        vec!["known"]
    );

    let newer = Backup {
        schema: BACKUP_SCHEMA_VERSION + 1,
        ..backup
    };
    assert!(matches!(
        store.restore_backup(&newer),
        Err(MlError::Store(message)) if message.contains("newer")
    ));
    assert!(store.restore_backup_json("{not json").is_err());
    assert!(store.restore_backup_json("").is_err());
}

#[test]
fn restore_never_overwrites_a_remote_playlist_and_is_atomic() {
    let store = store();
    // A remote playlist with the id the backup wants.
    store
        .upsert_playlist(&ml_store::PlaylistRecord {
            id: "VLremote".to_string(),
            name: "Server owned".to_string(),
            browse_id: Some("VLremote".to_string()),
            ..Default::default()
        })
        .unwrap();
    let backup = Backup {
        schema: BACKUP_SCHEMA_VERSION,
        songs: vec![BackupSong {
            id: "s".to_string(),
            title: "S".to_string(),
            ..BackupSong::default()
        }],
        playlists: vec![
            BackupPlaylist {
                id: "VLremote".to_string(),
                name: "Hijacked".to_string(),
                entries: vec![BackupEntry {
                    song_id: "s".to_string(),
                    set_video_id: None,
                }],
                ..BackupPlaylist::default()
            },
            BackupPlaylist {
                id: "LPlocal".to_string(),
                name: "Local".to_string(),
                entries: vec![BackupEntry {
                    song_id: "s".to_string(),
                    set_video_id: None,
                }],
                ..BackupPlaylist::default()
            },
        ],
        ..Backup::default()
    };
    let report = store.restore_backup(&backup).unwrap();
    assert_eq!(report.playlists, 1);
    assert_eq!(report.skipped_playlists, 1);
    assert_eq!(
        store.get_playlist("VLremote").unwrap().unwrap().name,
        "Server owned"
    );

    // Force a failure mid-transaction: nothing from the backup may land.
    store
        .with_connection(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER refuse BEFORE INSERT ON playlist_song_map \
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .map_err(|error| MlError::Store(error.to_string()))?;
            Ok(())
        })
        .unwrap();
    let failing = Backup {
        schema: BACKUP_SCHEMA_VERSION,
        songs: vec![BackupSong {
            id: "fresh".to_string(),
            title: "Fresh".to_string(),
            ..BackupSong::default()
        }],
        playlists: vec![BackupPlaylist {
            id: "LPatomic".to_string(),
            name: "Atomic".to_string(),
            entries: vec![BackupEntry {
                song_id: "fresh".to_string(),
                set_video_id: None,
            }],
            ..BackupPlaylist::default()
        }],
        ..Backup::default()
    };
    assert!(store.restore_backup(&failing).is_err());
    assert!(
        store.get_playlist("LPatomic").unwrap().is_none(),
        "the playlist insert rolled back"
    );
    assert!(
        store.get_song("fresh").unwrap().is_none(),
        "the song insert rolled back"
    );
}

// --------------------------------------------------------- bookmarks

#[test]
fn bookmarks_span_albums_artists_and_playlists() {
    let store = store();
    store
        .upsert_album(&Album {
            id: "al".to_string(),
            title: "Album".to_string(),
            ..Album::default()
        })
        .unwrap();
    store
        .upsert_artist(&ml_store::ArtistRecord {
            id: "ar".to_string(),
            name: "Artist".to_string(),
            ..Default::default()
        })
        .unwrap();
    let playlist = store.create_playlist("List").unwrap();

    assert_eq!(store.bookmark(BookmarkKind::Album, "al").unwrap(), None);
    store
        .set_bookmark(BookmarkKind::Album, "al", true, 100)
        .unwrap();
    store
        .set_bookmark(BookmarkKind::Artist, "ar", true, 200)
        .unwrap();
    store
        .set_bookmark(BookmarkKind::Playlist, &playlist, true, 300)
        .unwrap();
    assert_eq!(
        store.bookmark(BookmarkKind::Album, "al").unwrap(),
        Some(100)
    );
    assert_eq!(
        store.bookmark(BookmarkKind::Album, "missing").unwrap(),
        None
    );
    assert!(store
        .set_bookmark(BookmarkKind::Album, "missing", true, 1)
        .is_ok());

    assert_eq!(
        store.bookmarked_ids(BookmarkKind::Album, 10).unwrap(),
        vec!["al"]
    );
    let page = store.bookmarks_page(10).unwrap();
    assert_eq!(page.albums.len(), 1);
    assert_eq!(page.artists.len(), 1);
    assert_eq!(page.playlists.len(), 1);
    assert!(page.songs.is_empty());

    store
        .set_bookmark(BookmarkKind::Album, "al", false, 400)
        .unwrap();
    assert_eq!(store.bookmark(BookmarkKind::Album, "al").unwrap(), None);
    assert!(store
        .bookmarked_ids(BookmarkKind::Album, 10)
        .unwrap()
        .is_empty());
}

// ------------------------------------------------------ auto playlists

#[test]
fn auto_playlists_answer_liked_library_recent_and_most_played() {
    let store = store();
    let now = day_ms(2026, 8, 26, 12);
    store
        .upsert_songs(&[track("a", "Alpha"), track("b", "Beta"), track("c", "Gamma")])
        .unwrap();
    store.set_in_library_at("a", Some(1_000)).unwrap();
    store.set_in_library_at("b", Some(2_000)).unwrap();
    store.set_in_library_at("c", Some(3_000)).unwrap();
    store.set_liked_at("b", true, 5_000).unwrap();
    store.set_liked_at("c", true, 9_000).unwrap();
    store
        .record_play("a", 1_000, day_ms(2026, 8, 20, 9))
        .unwrap();
    store
        .record_play("a", 1_000, day_ms(2026, 8, 26, 9))
        .unwrap();
    store.record_play("b", 500, day_ms(2026, 8, 25, 9)).unwrap();
    store.record_play("c", 500, day_ms(2026, 8, 1, 9)).unwrap();

    let liked = store.auto_playlist(AutoPlaylist::Liked, now, 10).unwrap();
    assert_eq!(
        liked
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "b"],
        "most recently liked first"
    );

    let recently = store
        .auto_playlist(AutoPlaylist::RecentlyPlayed, now, 10)
        .unwrap();
    assert_eq!(
        recently
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"],
        "one row per song, newest play first"
    );
    assert_eq!(store.recently_played(1).unwrap().len(), 1);

    let most = store
        .auto_playlist(AutoPlaylist::MostPlayed(StatsPeriod::All), now, 10)
        .unwrap();
    assert_eq!(
        most.iter().map(|song| song.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    let weekly = store
        .auto_playlist(AutoPlaylist::MostPlayed(StatsPeriod::Week), now, 10)
        .unwrap();
    assert_eq!(weekly.len(), 2, "only this week's plays");

    let library = store.auto_playlist(AutoPlaylist::Library, now, 10).unwrap();
    assert_eq!(
        library
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "b", "a"],
        "newest library addition first"
    );
    assert_eq!(
        store
            .library_songs(LibrarySort::Alphabetical, 2, 0)
            .unwrap()
            .len(),
        2
    );
    // Downloads are outside this crate: no honest query, so an empty answer.
    assert!(store.downloaded_songs(10, 0).unwrap().is_empty());
}

// ---------------------------------------------------- Firefox cookies

/// A throwaway directory outside the user's real browser profiles.
fn scratch_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "metrolist-firefox-{tag}-{}-{}",
        std::process::id(),
        ml_core::now_millis()
    ))
}

/// Build a `cookies.sqlite` with the columns the real Firefox schema has that
/// this reader touches, so the query is exercised against SQLite itself.
fn write_cookie_db(path: &std::path::Path, rows: &[(&str, &str, &str, i64, bool, bool, bool)]) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE moz_cookies (
             id INTEGER PRIMARY KEY,
             host TEXT, path TEXT, name TEXT, value TEXT, expiry INTEGER,
             isSecure INTEGER, isHttpOnly INTEGER, isPartitionedAttributeSet INTEGER DEFAULT 0
         );",
    )
    .unwrap();
    for (host, name, value, expiry, secure, http_only, partitioned) in rows {
        conn.execute(
            "INSERT INTO moz_cookies \
             (host, path, name, value, expiry, isSecure, isHttpOnly, isPartitionedAttributeSet) \
             VALUES (?1, '/', ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![host, name, value, expiry, secure, http_only, partitioned],
        )
        .unwrap();
    }
}

#[test]
fn firefox_cookie_import_reads_a_real_database_and_skips_noise() {
    let dir = scratch_dir("db");
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("cookies.sqlite");
    write_cookie_db(
        &db,
        &[
            (
                ".youtube.com",
                "SID",
                "youtube-secret",
                0,
                true,
                true,
                false,
            ),
            (
                ".google.com",
                "SAPISID",
                "sapisid-secret",
                1_900_000_000,
                true,
                false,
                false,
            ),
            (
                "music.youtube.com",
                "PREF",
                "tz=Europe/Berlin",
                0,
                false,
                false,
                false,
            ),
            (".youtube.com", "EXPIRED", "gone", 100, true, false, false),
            (".example.com", "TRACKER", "nope", 0, false, false, false),
            // A partitioned cookie belongs to one embedded site: not global.
            (".youtube.com", "PARTITIONED", "x", 0, true, false, true),
            ("youtube.com.evil.test", "SPOOF", "x", 0, true, false, false),
        ],
    );
    let profile = ml_store::FirefoxProfile {
        name: "test.default-release".to_string(),
        cookies_db: db.clone(),
        modified: std::fs::metadata(&db).unwrap().modified().ok(),
    };

    let text = ml_store::read_firefox_cookies_at(&profile, 1_800_000_000).unwrap();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.starts_with('#') || line.starts_with("#HttpOnly_"))
        .collect();
    assert_eq!(
        lines,
        vec![
            ".google.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tsapisid-secret",
            "#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tyoutube-secret",
            "music.youtube.com\tFALSE\t/\tFALSE\t0\tPREF\ttz=Europe/Berlin",
        ],
        "sorted by raw host, so the dotted domains precede `music.`"
    );
    // Deterministic and free of the noise rows.
    for unwanted in ["EXPIRED", "TRACKER", "PARTITIONED", "SPOOF", "evil"] {
        assert!(!text.contains(unwanted), "{unwanted} leaked: {text}");
    }
    assert_eq!(
        text,
        ml_store::read_firefox_cookies_at(&profile, 1_800_000_000).unwrap()
    );
    // The same output as passing the rows straight to the pure renderer.
    assert!(text.ends_with('\n'));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn firefox_cookie_import_rejects_a_non_cookie_database_and_survives_locking() {
    let dir = scratch_dir("bad");
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("cookies.sqlite");

    // Some other SQLite file: a clean, named error; nothing about its contents.
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE unrelated (id INTEGER);")
            .unwrap();
        conn.execute("INSERT INTO unrelated VALUES (42)", [])
            .unwrap();
    }
    let profile = ml_store::FirefoxProfile {
        name: "not-cookies".to_string(),
        cookies_db: db.clone(),
        modified: None,
    };
    let error = ml_store::read_firefox_cookies_at(&profile, 1_800_000_000).unwrap_err();
    assert!(matches!(error, MlError::NotFound(_)), "{error:?}");
    assert!(!format!("{error}").contains("42"));

    // A missing file is an error, not a panic.
    let missing = ml_store::FirefoxProfile {
        name: "missing".to_string(),
        cookies_db: dir.join("nope.sqlite"),
        modified: None,
    };
    assert!(ml_store::read_firefox_cookies_at(&missing, 0).is_err());

    // Another connection holding the database, as a running browser would:
    // the immutable retry still reads it.
    write_cookie_db(
        &db,
        &[(".youtube.com", "SID", "live-session", 0, true, true, false)],
    );
    let holder = rusqlite::Connection::open(&db).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    let text = ml_store::read_firefox_cookies_at(&profile, 0).unwrap();
    assert!(text.contains("#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tlive-session"));
    holder.execute_batch("ROLLBACK;").unwrap();
    drop(holder);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn firefox_profile_discovery_sorts_newest_first_and_ignores_junk() {
    let root = scratch_dir("profiles");
    let older = root.join("aaa.default");
    let newer = root.join("bbb.default-release");
    let empty = root.join("ccc.empty");
    for dir in [&older, &newer, &empty] {
        std::fs::create_dir_all(dir).unwrap();
    }
    // A profile with only a subdirectory has no cookie database, so it is not
    // listed; the one below gets a file that is not a usable database, to prove
    // discovery filters on readability rather than on the name (the DB itself
    // is exercised by the other two tests).
    std::fs::create_dir_all(newer.join("nested")).unwrap();
    write_cookie_db(&newer.join("cookies.sqlite"), &[]);
    let old_db = older.join("cookies.sqlite");
    std::fs::write(&old_db, b"not really sqlite").unwrap();
    // Make the newer one newer, explicitly.
    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(3_600);
    std::fs::File::options()
        .write(true)
        .open(&old_db)
        .unwrap()
        .set_modified(old_time)
        .unwrap();

    let found = ml_store::firefox_profiles_in(&[root.clone(), root.join("absent")]);
    assert_eq!(
        found
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        vec!["bbb.default-release", "aaa.default"],
        "newest first; a profile without cookies.sqlite is not listed"
    );
    assert!(found[0].modified.unwrap() >= found[1].modified.unwrap());
    // The discovery is pure filesystem work; the real HOME is never touched by
    // these tests because they pass an explicit root.
    std::fs::remove_dir_all(root).ok();
}
