//! Behaviour ported from the app's Room/SQLite tests, plus the store-specific
//! invariants (migrations, FKs, positions, history buckets, WAL, concurrency).
//!
//! Sources under `arch/upstream-tests/app/src/test/kotlin/com/metrolist/music/db/`:
//! AddSongsToPlaylistTest, PlaylistDuplicatesBatchedTest,
//! HistoryDatabaseDeduplicationTest, SongMetadataEditingTest,
//! ArtistPageCacheTest, entities/SongEntityTest, entities/LocalIdTest.

use std::sync::Arc;

use ml_core::{Album, AlbumRef, Artist, LibrarySort, MlError, Song};
use ml_store::{
    civil_from_days, days_from_civil, local_id, CachedLyrics, LibraryFilter, PeriodBounds,
    PeriodBucket, PlaylistRecord, SpeedDial, Store, SCHEMA_VERSION,
};

const MS_PER_DAY: i64 = 86_400_000;

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn song(id: &str) -> Song {
    Song {
        id: id.to_string(),
        title: format!("Song {id}"),
        ..Song::default()
    }
}

fn titled(id: &str, title: &str) -> Song {
    Song {
        id: id.to_string(),
        title: title.to_string(),
        ..Song::default()
    }
}

// ---------------------------------------------------------------- songs

#[test]
fn upsert_keeps_local_state_and_accepts_explicit_liked_timestamps() {
    let store = store();
    store.upsert_song(&song("s")).unwrap();
    store.set_liked_at("s", true, 1_700_000_000_000).unwrap();
    store
        .set_in_library_at("s", Some(1_700_000_000_500))
        .unwrap();

    let mut refreshed = song("s");
    refreshed.duration = Some(200);
    store.upsert_song(&refreshed).unwrap();

    let stored = store.get_song("s").unwrap().unwrap();
    assert_eq!(stored.duration, Some(200));
    assert_eq!(stored.liked, Some(1_700_000_000_000));
    assert_eq!(stored.in_library, Some(1_700_000_000_500));
}

#[test]
fn local_only_like_uses_the_same_state_transition() {
    // SongEntityTest.localOnlyLikeUsesTheSameStateTransition
    let store = store();
    store.upsert_song(&song("video-id")).unwrap();

    store
        .set_liked_at("video-id", true, 1_700_000_000_000)
        .unwrap();
    let liked = store.get_song("video-id").unwrap().unwrap();
    assert_eq!(liked.liked, Some(1_700_000_000_000));
    assert!(liked.in_library.is_some(), "liking adds to the library");

    store
        .set_liked_at("video-id", false, 1_700_000_001_000)
        .unwrap();
    let unliked = store.get_song("video-id").unwrap().unwrap();
    assert_eq!(unliked.liked, None);
    assert!(
        unliked.in_library.is_some(),
        "unliking keeps library membership"
    );
}

#[test]
fn library_membership_sync_does_not_change_like_state() {
    // SongEntityTest.libraryMembershipSyncDoesNotChangeLikeState
    let store = store();
    store.upsert_song(&song("video-id")).unwrap();
    store
        .set_liked_at("video-id", true, 1_756_000_000_000)
        .unwrap();

    store.set_in_library_at("video-id", None).unwrap();
    let removed = store.get_song("video-id").unwrap().unwrap();
    assert_eq!(removed.liked, Some(1_756_000_000_000));
    assert_eq!(removed.in_library, None);

    store
        .set_in_library_at("video-id", Some(1_756_000_003_600))
        .unwrap();
    let restored = store.get_song("video-id").unwrap().unwrap();
    assert_eq!(restored.liked, Some(1_756_000_000_000));
    assert_eq!(restored.in_library, Some(1_756_000_003_600));
}

#[test]
fn album_refresh_preserves_edited_song_metadata() {
    // SongMetadataEditingTest.album refresh preserves edited song metadata
    let store = store();
    let artist = Artist::new("Edited artist", Some("edited-artist".to_string()));
    let edited = Song {
        id: "song".to_string(),
        title: "Edited title".to_string(),
        duration: Some(100),
        artists: vec![artist.clone()],
        album: Some(AlbumRef {
            id: "album".to_string(),
            title: "Album".to_string(),
        }),
        ..Song::default()
    };
    store.upsert_song(&edited).unwrap();

    // A remote refresh carries a different title/artist but a new duration.
    let mut remote = Song {
        id: "song".to_string(),
        title: "Remote title".to_string(),
        duration: Some(200),
        artists: vec![Artist::new(
            "Remote artist",
            Some("remote-artist".to_string()),
        )],
        ..Song::default()
    };
    store.refresh_song(&remote).unwrap();

    let refreshed = store.get_song("song").unwrap().unwrap();
    assert_eq!(refreshed.title, "Edited title");
    assert_eq!(refreshed.duration, Some(200));
    assert_eq!(
        refreshed.artists,
        vec![Artist::new(
            "Edited artist",
            Some("edited-artist".to_string())
        )]
    );

    // A full upsert does adopt the new title (that is the "overwrite" path).
    remote.title = "Second title".to_string();
    store.upsert_song(&remote).unwrap();
    assert_eq!(
        store.get_song("song").unwrap().unwrap().title,
        "Second title"
    );

    // Editing the credit list is per song: the old artist row survives.
    store
        .replace_song_artists(
            "song",
            &[Artist::new("New artist", Some("new-artist".to_string()))],
        )
        .unwrap();
    assert_eq!(
        store.song_artists("song").unwrap(),
        vec!["new-artist".to_string()]
    );
    assert_eq!(
        store.get_artist("edited-artist").unwrap().unwrap().name,
        "Edited artist"
    );
    let _ = edited.artists;
}

#[test]
fn untrusted_credit_lists_do_not_erase_known_artists() {
    // SongMetadataEditingTest: artists = null must not drop links.
    let store = store();
    let mut song = titled("song", "Song");
    song.artists = vec![Artist::new(
        "Saved artist",
        Some("saved-artist".to_string()),
    )];
    store.upsert_song(&song).unwrap();

    let mut no_credits = titled("song", "Song");
    no_credits.artists = Vec::new();
    store.upsert_song(&no_credits).unwrap();
    store.replace_song_artists("song", &[]).unwrap();

    assert_eq!(store.song_artists("song").unwrap(), vec!["saved-artist"]);
    assert_eq!(
        store.get_artist("saved-artist").unwrap().unwrap().name,
        "Saved artist"
    );
}

#[test]
fn unknown_artist_ids_get_a_stable_local_artist() {
    // LocalIdTest: generateLocalId("LA") shape, and reuse by name.
    let store = store();
    let mut first = titled("a", "A");
    first.artists = vec![Artist::new("Nameless", None)];
    let mut second = titled("b", "B");
    second.artists = vec![Artist::new("Nameless", None)];
    store.upsert_songs(&[first, second]).unwrap();

    let a = store.song_artists("a").unwrap();
    let b = store.song_artists("b").unwrap();
    assert_eq!(a, b, "the same name resolves to one artist row");
    let id = &a[0];
    assert!(id.starts_with("LA"));
    assert_eq!(id.len(), 10);
    assert!(id[2..].bytes().all(|byte| byte.is_ascii_alphabetic()));
}

#[test]
fn local_ids_are_prefixed_and_random_enough() {
    let id = local_id("LP");
    assert!(id.starts_with("LP"));
    assert_eq!(id.len(), 10);
    assert!(id[2..].bytes().all(|byte| byte.is_ascii_alphabetic()));
    assert_ne!(id, local_id("LP"));
    assert_ne!(local_id("LP"), local_id("LP"));
}

#[test]
fn missing_song_is_none_and_batches_keep_request_order() {
    let store = store();
    store
        .upsert_songs(&[song("a"), song("b"), song("c")])
        .unwrap();
    assert!(store.get_song("nope").unwrap().is_none());
    let ids = vec!["c".to_string(), "a".to_string(), "ghost".to_string()];
    let found = store.songs_by_ids(&ids).unwrap();
    assert_eq!(
        found
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "a"]
    );
    assert!(store.songs_by_ids(&[]).unwrap().is_empty());
}

#[test]
fn library_and_liked_queries_filter_and_order() {
    let store = store();
    store
        .upsert_songs(&[
            titled("a", "Zebra"),
            titled("b", "apple"),
            titled("c", "Mango"),
        ])
        .unwrap();
    store.set_in_library_at("a", Some(2_000)).unwrap();
    store.set_in_library_at("b", Some(1_000)).unwrap();
    store.set_liked_at("c", true, 3_000).unwrap();

    let library = store
        .library_songs(LibrarySort::RecentlyAdded, 10, 0)
        .unwrap();
    assert_eq!(
        library
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "a", "b"]
    );
    let alphabetical = store
        .library_songs(LibrarySort::Alphabetical, 10, 0)
        .unwrap();
    assert_eq!(
        alphabetical
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "c", "a"]
    );
    let liked = store.liked_songs(10, 0).unwrap();
    assert_eq!(
        liked
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["c"]
    );

    // Pagination tiles: no repeats, no gaps.
    let page_one = store
        .library_songs(LibrarySort::Alphabetical, 2, 0)
        .unwrap();
    let page_two = store
        .library_songs(LibrarySort::Alphabetical, 2, 2)
        .unwrap();
    assert_eq!(page_one.len(), 2);
    assert_eq!(page_two.len(), 1);
    assert!(page_one.iter().all(|song| song.id != page_two[0].id));
}

#[test]
fn search_matches_title_artist_and_album_and_escapes_wildcards() {
    let store = store();
    let mut track = Song {
        id: "s".to_string(),
        title: "100% Pure".to_string(),
        artists: vec![Artist::new("Kavinsky", Some("kav".to_string()))],
        album: Some(AlbumRef {
            id: "al".to_string(),
            title: "Outrun".to_string(),
        }),
        ..Song::default()
    };
    track.in_library = Some(1);
    store.upsert_song(&track).unwrap();
    let other = titled("t", "Nightcall");
    store.upsert_song(&other).unwrap();

    let hits = store.search_songs("kavin", 10).unwrap();
    assert_eq!(
        hits.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["s"]
    );
    let by_album = store.search_songs("outr", 10).unwrap();
    assert_eq!(by_album.len(), 1);
    // A bare `%` matches only the literal title, not every song.
    let literal = store.search_songs("%", 10).unwrap();
    assert_eq!(
        literal.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["s"]
    );
    // Not in the library => not a local search hit.
    assert!(store.search_songs("nightcall", 10).unwrap().is_empty());
}

// ------------------------------------------------------------ playlists

#[test]
fn bulk_add_works_and_returns_the_song_ids() {
    // AddSongsToPlaylistTest.bulk add is safe when called from main thread
    let store = store();
    store.upsert_song(&song("song")).unwrap();
    let playlist = store.create_playlist("Playlist").unwrap();

    assert_eq!(store.add_songs(&playlist, &[song("song")]).unwrap(), 1);
    assert_eq!(store.playlist_song_ids(&playlist).unwrap(), vec!["song"]);
    // Appending the same song again is a no-op.
    assert_eq!(store.add_songs(&playlist, &[song("song")]).unwrap(), 0);
    assert_eq!(store.playlist_song_ids(&playlist).unwrap(), vec!["song"]);
}

#[test]
fn append_inserts_after_sparse_playlist_positions() {
    // AddSongsToPlaylistTest.append inserts after sparse playlist positions
    let store = store();
    store
        .upsert_songs(&[song("first"), song("last"), song("new")])
        .unwrap();
    let playlist = store.create_playlist("Sparse").unwrap();
    store
        .upsert_playlist(&PlaylistRecord {
            id: playlist.clone(),
            name: "Sparse".to_string(),
            remote_song_count: Some(2),
            ..PlaylistRecord::default()
        })
        .unwrap();
    // Seed positions 0 and 5 the way a remote playlist would arrive.
    store
        .replace_playlist_songs(
            &playlist,
            &[
                ("first".to_string(), Some("sv-first".to_string())),
                ("last".to_string(), None),
            ],
        )
        .unwrap();
    store
        .with_connection(|store| {
            store
                .execute(
                    "UPDATE playlist_song_map SET position = 5 WHERE song_id = 'last'",
                    [],
                )
                .map_err(|error| MlError::Store(error.to_string()))?;
            Ok(())
        })
        .unwrap();

    store.add_songs(&playlist, &[song("new")]).unwrap();
    assert_eq!(
        store.playlist_song_ids(&playlist).unwrap(),
        vec!["first", "last", "new"]
    );
    let positions: Vec<i64> = store
        .playlist_songs(&playlist)
        .unwrap()
        .iter()
        .map(|entry| entry.position)
        .collect();
    assert_eq!(
        positions,
        vec![0, 5, 6],
        "append uses max(position)+1, not count"
    );
}

#[test]
fn add_carries_set_video_id_and_joins_the_library() {
    let store = store();
    store.upsert_song(&song("s")).unwrap();
    let playlist = store.create_playlist("Mix").unwrap();
    let mut entry = song("s");
    entry.set_video_id = Some("sv123".to_string());
    store.add_songs(&playlist, &[entry]).unwrap();

    let songs = store.playlist_songs(&playlist).unwrap();
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].set_video_id.as_deref(), Some("sv123"));
    assert!(
        songs[0].song.in_library.is_some(),
        "playlist adds join the library"
    );
}

#[test]
fn duplicate_lookup_is_correct_across_bind_variable_batches() {
    // PlaylistDuplicatesBatchedTest
    let store = store();
    let full = store.create_playlist("Full").unwrap();
    let partial = store.create_playlist("Partial").unwrap();
    let ids: Vec<String> = (0..1_200).map(|index| format!("song-{index}")).collect();
    let songs: Vec<Song> = ids.iter().map(|id| song(id)).collect();
    store.upsert_songs(&songs).unwrap();
    store.add_songs(&full, &songs).unwrap();
    // Every other song, so the partial playlist holds the even indices.
    let evens: Vec<Song> = songs.iter().step_by(2).cloned().collect();
    store.add_songs(&partial, &evens).unwrap();

    let full_duplicates = store.playlist_duplicates(&full, &ids).unwrap();
    assert_eq!(full_duplicates.len(), 1_200);
    assert_eq!(
        full_duplicates
            .into_iter()
            .collect::<std::collections::HashSet<_>>(),
        ids.iter().cloned().collect()
    );

    let partial_duplicates = store.playlist_duplicates(&partial, &ids).unwrap();
    assert_eq!(partial_duplicates.len(), 600);
    assert!(partial_duplicates.iter().all(|id| {
        let index: usize = id.trim_start_matches("song-").parse().unwrap();
        index.is_multiple_of(2)
    }));
    assert!(store.playlist_duplicates(&full, &[]).unwrap().is_empty());
}

#[test]
fn playlist_positions_stay_dense_after_remove_move_and_reorder() {
    let store = store();
    let playlist = store.create_playlist("Mix").unwrap();
    let songs: Vec<Song> = ["a", "b", "c", "d"].iter().map(|id| song(id)).collect();
    store.upsert_songs(&songs).unwrap();
    store.add_songs(&playlist, &songs).unwrap();
    assert_eq!(
        store.playlist_song_ids(&playlist).unwrap(),
        vec!["a", "b", "c", "d"]
    );

    // Move b (1) to position 3 and c (2) up to 0.
    store.move_song(&playlist, 1, 3).unwrap();
    assert_eq!(
        store.playlist_song_ids(&playlist).unwrap(),
        vec!["a", "c", "d", "b"]
    );
    store.move_song(&playlist, 2, 0).unwrap();
    assert_eq!(
        store.playlist_song_ids(&playlist).unwrap(),
        vec!["d", "a", "c", "b"]
    );

    // Out-of-range merges are no-ops.
    store.move_song(&playlist, 0, 99).unwrap();
    store.move_song(&playlist, 0, 0).unwrap();
    assert_eq!(
        store.playlist_song_ids(&playlist).unwrap(),
        vec!["d", "a", "c", "b"]
    );
    let positions: Vec<i64> = store
        .playlist_songs(&playlist)
        .unwrap()
        .iter()
        .map(|entry| entry.position)
        .collect();
    assert_eq!(positions, vec![0, 1, 2, 3]);

    store.remove_song(&playlist, "a").unwrap();
    assert_eq!(
        store.playlist_song_ids(&playlist).unwrap(),
        vec!["d", "c", "b"]
    );
    let positions: Vec<i64> = store
        .playlist_songs(&playlist)
        .unwrap()
        .iter()
        .map(|entry| entry.position)
        .collect();
    assert_eq!(positions, vec![0, 1, 2], "removal closes the gap");

    let reorder = ["b".to_string(), "d".to_string(), "c".to_string()];
    store.reorder(&playlist, &reorder).unwrap();
    assert_eq!(
        store.playlist_song_ids(&playlist).unwrap(),
        vec!["b", "d", "c"]
    );

    store.clear_playlist(&playlist).unwrap();
    assert!(store.playlist_songs(&playlist).unwrap().is_empty());
    // The header survives clearing.
    assert!(store.get_playlist(&playlist).unwrap().is_some());
}

#[test]
fn playlist_rename_delete_and_bookmark() {
    let store = store();
    let playlist = store.create_playlist("Before").unwrap();
    store.rename_playlist(&playlist, "After").unwrap();
    assert_eq!(
        store.get_playlist(&playlist).unwrap().unwrap().name,
        "After"
    );

    store
        .set_playlist_bookmarked_at(&playlist, Some(1_234))
        .unwrap();
    let bookmarked = store.bookmarked_playlists(10, 0).unwrap();
    assert_eq!(bookmarked.len(), 1);
    assert_eq!(bookmarked[0].id, playlist);

    store.delete_playlist(&playlist).unwrap();
    assert!(store.get_playlist(&playlist).unwrap().is_none());
    assert!(!store
        .playlists(10, 0)
        .unwrap()
        .iter()
        .any(|record| record.id == playlist));
}

#[test]
fn deleting_a_song_removes_it_from_playlists() {
    let store = store();
    store.upsert_songs(&[song("a"), song("b")]).unwrap();
    let playlist = store.create_playlist("Mix").unwrap();
    store.add_songs(&playlist, &[song("a"), song("b")]).unwrap();
    store.delete_song("a").unwrap();
    assert_eq!(store.playlist_song_ids(&playlist).unwrap(), vec!["b"]);
}

// -------------------------------------------------------------- history

/// 2026-08-26 is a Wednesday; the week's Monday is 2026-08-24.
fn aug_2026(day: u32, hour: u32) -> i64 {
    days_from_civil(2026, 8, day) * MS_PER_DAY + i64::from(hour) * 3_600_000
}

const NOW: i64 = 25 + days_from_civil(2026, 8, 26) * MS_PER_DAY + 12 * 3_600_000;

fn history_ids(store: &Store, now: i64) -> Vec<i64> {
    store
        .history_at(now, 100, 0)
        .unwrap()
        .iter()
        .map(|entry| entry.id)
        .collect()
}

#[test]
fn history_hydrates_only_the_latest_play_per_song_in_each_period() {
    // HistoryDatabaseDeduplicationTest, same calendar: TODAY = 2026-08-26.
    let store = store();
    store
        .upsert_songs(&[titled("song", "Song"), titled("other-song", "Other song")])
        .unwrap();
    let plays = [
        (1, "song", aug_2026(26, 9)),  // today
        (2, "song", aug_2026(26, 10)), // today (newer)
        (3, "song", aug_2026(25, 9)),  // yesterday
        (4, "song", aug_2026(25, 10)), // yesterday (newer)
        (5, "song", aug_2026(24, 9)),  // this week (Monday)
        (6, "song", aug_2026(18, 9)),  // last week (Tuesday after last Monday)
        (
            7,
            "song",
            days_from_civil(2026, 7, 1) * MS_PER_DAY + 9 * 3_600_000,
        ),
        (
            8,
            "song",
            days_from_civil(2026, 7, 20) * MS_PER_DAY + 9 * 3_600_000,
        ),
        (9, "other-song", aug_2026(26, 11)), // today, other song
    ];
    for (_, song_id, timestamp) in plays {
        store.record_play(song_id, 30_000, timestamp).unwrap();
    }

    assert_eq!(history_ids(&store, NOW), vec![9, 8, 6, 5, 4, 2]);
}

#[test]
fn repeated_plays_collapse_to_one_row() {
    // HistoryDatabaseDeduplicationTest.repeated plays do not create repeated graphs
    let store = store();
    store.upsert_song(&song("song")).unwrap();
    let base = aug_2026(26, 12);
    for index in 0..2_000 {
        store.record_play("song", 30_000, base + index).unwrap();
    }
    let entries = store.history_at(NOW, 100, 0).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, 2_000);
}

#[test]
fn latest_event_is_the_newest_row_whatever_the_period() {
    // HistoryDatabaseDeduplicationTest.latest event lookup stays bounded
    let store = store();
    store
        .upsert_songs(&[titled("song", "Song"), titled("other-song", "Other")])
        .unwrap();
    store.record_play("song", 30_000, aug_2026(26, 9)).unwrap();
    store
        .record_play("other-song", 30_000, aug_2026(26, 10))
        .unwrap();
    let latest = store.latest_event().unwrap().unwrap();
    assert_eq!(latest.id, 2);
    assert_eq!(latest.song.id, "other-song");
}

#[test]
fn history_buckets_respect_period_boundaries() {
    let bounds = PeriodBounds::from_millis(NOW);
    let cases = [
        (bounds.tomorrow_start, PeriodBucket::ThisWeek),
        (bounds.today_start, PeriodBucket::Today),
        (bounds.yesterday_start, PeriodBucket::Yesterday),
        (bounds.this_monday_start, PeriodBucket::ThisWeek),
        (bounds.last_monday_start, PeriodBucket::LastWeek),
        // 2026-07-15 is after the previous month's boundaries, so it lands in
        // that calendar month's bucket.
        (
            days_from_civil(2026, 7, 15) * MS_PER_DAY,
            PeriodBucket::Month {
                year: 2026,
                month: 7,
            },
        ),
    ];
    for (timestamp, expected) in cases {
        assert_eq!(
            bounds.bucket_of(timestamp),
            expected,
            "timestamp {timestamp}"
        );
    }
    assert_eq!(
        PeriodBucket::Month {
            year: 2026,
            month: 7
        }
        .label(),
        "2026-07"
    );
    assert_eq!(PeriodBucket::Today.label(), "today");
}

#[test]
fn period_bounds_are_ordered_and_contain_now() {
    // Every value a real clock produces: the strict invariants hold.
    for now in [0, 1, NOW, 1_700_000_000_000, -86_400_000, i64::MAX / 2] {
        let bounds = PeriodBounds::from_millis(now);
        assert!(
            bounds.last_monday_start < bounds.this_monday_start,
            "now={now}"
        );
        assert!(
            bounds.this_monday_start <= bounds.yesterday_start,
            "now={now}"
        );
        assert!(bounds.yesterday_start < bounds.today_start, "now={now}");
        assert!(bounds.today_start <= now, "now={now}");
        assert!(now < bounds.tomorrow_start, "now={now}");
        assert_eq!(
            bounds.this_monday_start - bounds.last_monday_start,
            7 * MS_PER_DAY
        );
        assert_eq!(bounds.bucket_of(now), PeriodBucket::Today);
    }

    // The documented domain's edges: ±100 years around the epoch, plus a
    // million years out, all still exact.
    for now in [
        -3_155_760_000_000,
        -86_400_000,
        0,
        3_155_760_000_000,
        31_556_952_000_000_000,
    ] {
        let bounds = PeriodBounds::from_millis(now);
        assert!(
            bounds.last_monday_start < bounds.this_monday_start,
            "now={now}"
        );
        assert!(bounds.today_start <= now, "now={now}");
        assert!(now < bounds.tomorrow_start, "now={now}");
        assert_eq!(bounds.bucket_of(now), PeriodBucket::Today);
    }

    // Outside the domain the function is still total: no panic, no overflow.
    for now in [i64::MAX, i64::MIN, i64::MIN + 1, i64::MAX - 1] {
        let bounds = PeriodBounds::from_millis(now);
        let _ = bounds.bucket_of(now);
        let _ = bounds.bucket_of(i64::MIN);
        let _ = PeriodBounds::from_millis_with_offset(now, i64::MIN);
        let _ = PeriodBounds::from_millis_with_offset(now, i64::MAX);
    }
}

#[test]
fn period_bounds_honour_a_timezone_offset() {
    // 2026-08-26 23:30 UTC is already the 27th in UTC+2.
    let late = days_from_civil(2026, 8, 26) * MS_PER_DAY + 23 * 3_600_000 + 30 * 60_000;
    let utc = PeriodBounds::from_millis(late);
    let plus_two = PeriodBounds::from_millis_with_offset(late, 2 * 3_600_000);
    assert_eq!(utc.today_start, days_from_civil(2026, 8, 26) * MS_PER_DAY);
    assert_eq!(
        plus_two.today_start,
        days_from_civil(2026, 8, 27) * MS_PER_DAY - 2 * 3_600_000
    );
    assert_eq!(plus_two.bucket_of(late), PeriodBucket::Today);
}

#[test]
fn civil_day_conversion_round_trips() {
    for (year, month, day) in [
        (1970, 1, 1),
        (2000, 2, 29),
        (2026, 8, 26),
        (1969, 12, 31),
        (2100, 12, 31),
    ] {
        assert_eq!(
            civil_from_days(days_from_civil(year, month, day)),
            (year, month, day)
        );
    }
    assert_eq!(days_from_civil(1970, 1, 1), 0);
}

#[test]
fn play_counts_track_events_per_month() {
    let store = store();
    store.upsert_song(&song("s")).unwrap();
    store.record_play("s", 30_000, aug_2026(1, 9)).unwrap();
    store.record_play("s", 30_000, aug_2026(2, 9)).unwrap();
    store
        .record_play("s", 10_000, days_from_civil(2026, 7, 3) * MS_PER_DAY)
        .unwrap();

    let august = store.play_counts(2026, 8).unwrap();
    assert_eq!(august.len(), 1);
    assert_eq!(august[0].count, 2);
    assert_eq!(store.play_count("s", 2026, 8).unwrap(), 2);
    assert_eq!(store.play_count("s", 2026, 7).unwrap(), 1);
    assert_eq!(store.play_count("s", 2026, 6).unwrap(), 0);
    assert_eq!(store.total_play_time().unwrap(), 70_000);
    assert_eq!(store.event_count().unwrap(), 3);

    store.clear_history().unwrap();
    assert_eq!(store.event_count().unwrap(), 0);
    // The monthly tally survives, as upstream's clearListenHistory only clears events.
    assert_eq!(store.play_count("s", 2026, 8).unwrap(), 2);
}

#[test]
fn history_pages_tile_without_repeats() {
    let store = store();
    store
        .upsert_songs(&[song("a"), song("b"), song("c")])
        .unwrap();
    store.record_play("a", 1_000, aug_2026(1, 9)).unwrap();
    store
        .record_play("b", 1_000, days_from_civil(2026, 6, 1) * MS_PER_DAY)
        .unwrap();
    store
        .record_play("c", 1_000, days_from_civil(2026, 5, 1) * MS_PER_DAY)
        .unwrap();

    let all = store.history_at(NOW, 100, 0).unwrap();
    assert_eq!(all.len(), 3);
    let first = store.history_at(NOW, 2, 0).unwrap();
    let second = store.history_at(NOW, 2, 2).unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 1);
    let ids: std::collections::HashSet<i64> = first.iter().chain(&second).map(|e| e.id).collect();
    assert_eq!(ids.len(), 3);
}

// ---------------------------------------------- albums, artists, misc

#[test]
fn album_refresh_keeps_song_order_and_untrusted_credits() {
    let store = store();
    let album = Album {
        id: "album".to_string(),
        playlist_id: "playlist".to_string(),
        title: "Album".to_string(),
        year: Some(2026),
        song_count: Some(2),
        duration: Some(300),
        ..Album::default()
    };
    let first = titled("s1", "One");
    let second = titled("s2", "Two");
    store
        .set_album_songs(&album, &[second.clone(), first.clone()], Some(&[]), 1_000)
        .unwrap();

    assert_eq!(
        store
            .album_songs("album")
            .unwrap()
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["s2", "s1"]
    );
    // Untrusted refresh (empty artist list) must not clear the album's credits
    // or its song associations.
    let credited = Album {
        artists: vec![Artist::new("Primary", Some("primary".to_string()))],
        ..album.clone()
    };
    store
        .set_album_songs(&credited, &[], Some(&[]), 2_000)
        .unwrap();
    assert_eq!(store.album_artists("album").unwrap(), vec!["primary"]);
    assert_eq!(store.album_songs("album").unwrap().len(), 2);
    assert_eq!(store.get_album("album").unwrap().unwrap().title, "Album");
}

#[test]
fn partial_album_credits_do_not_delete_shared_artists() {
    // SongMetadataEditingTest.partial album credits do not delete shared artists
    let store = store();
    let primary = Artist::new("Primary", Some("primary".to_string()));
    let featured = Artist::new("Featured", Some("featured".to_string()));
    let album = Album {
        id: "album".to_string(),
        title: "Album".to_string(),
        ..Album::default()
    };
    let other = Album {
        id: "other-album".to_string(),
        title: "Other album".to_string(),
        ..Album::default()
    };
    store.upsert_album(&album).unwrap();
    store.upsert_album(&other).unwrap();
    store
        .set_album_artists("album", &[primary.clone(), featured.clone()])
        .unwrap();
    store
        .set_album_artists("other-album", std::slice::from_ref(&featured))
        .unwrap();
    let mut shared = titled("shared-song", "Shared");
    shared.artists = vec![featured.clone()];
    store.upsert_song(&shared).unwrap();

    store
        .set_album_artists("album", std::slice::from_ref(&primary))
        .unwrap();

    assert_eq!(
        store.get_artist("featured").unwrap().unwrap().name,
        "Featured"
    );
    assert_eq!(store.album_artists("album").unwrap(), vec!["primary"]);
    assert_eq!(
        store.album_artists("other-album").unwrap(),
        vec!["featured"]
    );
    assert_eq!(store.song_artists("shared-song").unwrap(), vec!["featured"]);
}

#[test]
fn reading_a_song_leaves_the_cached_artist_page_on_disk() {
    // ArtistPageCacheTest
    let store = store();
    store
        .upsert_artist(&ml_store::ArtistRecord {
            id: "UCartist".to_string(),
            name: "Artist".to_string(),
            thumbnail_url: Some("https://example.invalid/artist.jpg".to_string()),
            channel_id: Some("UCchannel".to_string()),
            subscribers: Some("1M".to_string()),
            description: Some("About".to_string()),
            bookmarked_at: Some(1_000),
            last_update: Some(2_000),
            cached_page_json: Some(r#"{"sections":[{"title":"Songs"}]}"#.to_string()),
        })
        .unwrap();
    let mut track = titled("song", "Song");
    track.artists = vec![Artist::new("Artist", Some("UCartist".to_string()))];
    store.upsert_song(&track).unwrap();

    let hydrated = store.get_song("song").unwrap().unwrap();
    assert_eq!(hydrated.artists.len(), 1);
    assert_eq!(hydrated.artists[0].name, "Artist");

    let bulk = store.get_artist("UCartist").unwrap().unwrap();
    assert_eq!(
        bulk.cached_page_json, None,
        "bulk projection drops the page"
    );
    assert_eq!(bulk.id, "UCartist");
    assert_eq!(bulk.channel_id.as_deref(), Some("UCchannel"));
    assert_eq!(bulk.bookmarked_at, Some(1_000));
    assert_eq!(bulk.last_update, Some(2_000));

    let detail = store.artist_detail("UCartist").unwrap().unwrap();
    assert_eq!(
        detail.cached_page_json.as_deref(),
        Some(r#"{"sections":[{"title":"Songs"}]}"#)
    );

    // A thumbnail found while playing replaces only the thumbnail.
    store
        .update_artist_thumbnail("UCartist", "https://example.invalid/fetched.jpg")
        .unwrap();
    let detail = store.artist_detail("UCartist").unwrap().unwrap();
    assert_eq!(
        detail.thumbnail_url.as_deref(),
        Some("https://example.invalid/fetched.jpg")
    );
    assert!(detail.cached_page_json.is_some());

    // And the bulk projection still carries every other column.
    let all = store.artists(10, 0).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].subscribers.as_deref(), Some("1M"));
    assert_eq!(all[0].description.as_deref(), Some("About"));
    assert_eq!(all[0].cached_page_json, None);
}

#[test]
fn artist_bookmarks_and_song_lists() {
    let store = store();
    store
        .upsert_artist(&ml_store::ArtistRecord {
            id: "a1".to_string(),
            name: "Alpha".to_string(),
            ..Default::default()
        })
        .unwrap();
    store.set_artist_bookmarked_at("a1", Some(5_000)).unwrap();
    assert_eq!(store.bookmarked_artists(10, 0).unwrap().len(), 1);

    let mut track = titled("s", "Song");
    track.artists = vec![Artist::new("Alpha", Some("a1".to_string()))];
    store.upsert_song(&track).unwrap();
    assert!(
        store.artist_songs("a1", 10, 0).unwrap().is_empty(),
        "not in library yet"
    );
    store.set_in_library_at("s", Some(1)).unwrap();
    assert_eq!(store.artist_songs("a1", 10, 0).unwrap().len(), 1);
    assert_eq!(store.artist_song_count("a1").unwrap(), 1);
    assert_eq!(store.search_artists("alph", 10).unwrap().len(), 1);
}

#[test]
fn lyrics_cache_round_trips_and_replaces() {
    let store = store();
    let lyrics = CachedLyrics {
        song_id: "s".to_string(),
        provider: "lrclib".to_string(),
        synced: true,
        body: "[00:01.00]Hello".to_string(),
        translated: None,
        translation_lang: None,
        updated_at: 1,
    };
    store.put_lyrics_at(&lyrics, 1_000).unwrap();
    let stored = store.cached_lyrics("s").unwrap().unwrap();
    assert_eq!(stored.body, "[00:01.00]Hello");
    assert!(stored.synced);
    assert_eq!(stored.updated_at, 1_000);

    store
        .put_lyrics_at(
            &CachedLyrics {
                body: "[00:02.00]Bye".to_string(),
                translated: Some("[00:02.00]Tschüss".to_string()),
                translation_lang: Some("de".to_string()),
                ..lyrics
            },
            2_000,
        )
        .unwrap();
    let stored = store.cached_lyrics("s").unwrap().unwrap();
    assert_eq!(stored.body, "[00:02.00]Bye");
    assert_eq!(stored.translation_lang.as_deref(), Some("de"));
    assert_eq!(stored.updated_at, 2_000);

    store.delete_lyrics("s").unwrap();
    assert!(store.cached_lyrics("s").unwrap().is_none());
}

#[test]
fn search_history_moves_requeries_to_the_top() {
    let store = store();
    store.add_search_history_at("alpha", 1).unwrap();
    store.add_search_history_at("beta", 2).unwrap();
    store.add_search_history_at("alpha", 3).unwrap();
    assert_eq!(store.search_history(10).unwrap(), vec!["alpha", "beta"]);

    store.add_search_history("   ").unwrap();
    assert_eq!(
        store.search_history(10).unwrap().len(),
        2,
        "blank queries are ignored"
    );

    assert_eq!(store.search_history(1).unwrap(), vec!["alpha"]);
    store.clear_search_history().unwrap();
    assert!(store.search_history(10).unwrap().is_empty());
}

#[test]
fn speed_dial_keeps_order_when_deleting() {
    let store = store();
    for id in ["a", "b", "c"] {
        store
            .put_speed_dial_at(
                &SpeedDial {
                    id: id.to_string(),
                    title: id.to_uppercase(),
                    item_type: "song".to_string(),
                    item_json: "{}".to_string(),
                    ..SpeedDial::default()
                },
                1,
            )
            .unwrap();
    }
    let positions: Vec<i64> = store
        .speed_dial()
        .unwrap()
        .iter()
        .map(|item| item.position)
        .collect();
    assert_eq!(positions, vec![0, 1, 2]);

    store.delete_speed_dial("b").unwrap();
    let tiles = store.speed_dial().unwrap();
    assert_eq!(
        tiles
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "c"]
    );
    assert_eq!(
        tiles.iter().map(|item| item.position).collect::<Vec<_>>(),
        vec![0, 1]
    );

    store
        .reorder_speed_dial(&["c".to_string(), "a".to_string()])
        .unwrap();
    assert_eq!(
        store
            .speed_dial()
            .unwrap()
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "a"]
    );
}

#[test]
fn kv_stores_the_queue_snapshot() {
    let store = store();
    assert_eq!(store.kv_get("queue").unwrap(), None);
    store.kv_set("queue", r#"{"songs":["a"]}"#).unwrap();
    assert_eq!(
        store.kv_get("queue").unwrap().unwrap(),
        r#"{"songs":["a"]}"#
    );
    store.kv_set("queue", "{}").unwrap();
    assert_eq!(store.kv_get("queue").unwrap().unwrap(), "{}");
    store.kv_delete("queue").unwrap();
    assert_eq!(store.kv_get("queue").unwrap(), None);
}

#[test]
fn library_page_and_local_search() {
    let store = store();
    let mut track = titled("s", "Song");
    track.artists = vec![Artist::new("Artist", Some("ar".to_string()))];
    track.album = Some(AlbumRef {
        id: "al".to_string(),
        title: "Album".to_string(),
    });
    track.in_library = Some(1);
    store.upsert_song(&track).unwrap();
    store
        .upsert_album(&Album {
            id: "al".to_string(),
            title: "Album".to_string(),
            ..Album::default()
        })
        .unwrap();
    let playlist = store.create_playlist("Road trip").unwrap();

    let page = store
        .library_page(LibraryFilter::All, LibrarySort::Alphabetical, 10, 0)
        .unwrap();
    assert_eq!(page.songs.len(), 1);
    assert_eq!(page.albums.len(), 1, "album has a library song");
    assert_eq!(page.playlists.len(), 1);
    assert_eq!(page.playlists[0].id, playlist);

    let liked = store
        .library_page(LibraryFilter::Liked, LibrarySort::RecentlyAdded, 10, 0)
        .unwrap();
    assert!(liked.songs.is_empty());

    let found = store.search_library("son", 10).unwrap();
    assert_eq!(found.songs.len(), 1);
    assert_eq!(found.playlists.len(), 0, "the playlist is named Road trip");
    let by_album = store.search_library("albu", 10).unwrap();
    assert_eq!(by_album.albums.len(), 1, "album title search");
    assert_eq!(store.search_library("nothing", 10).unwrap().songs.len(), 0);
    assert!(store.downloaded_songs(10, 0).unwrap().is_empty());
}

#[test]
fn library_albums_answers_every_sort_arm() {
    // Regression: `ORDER BY` cannot see an alias declared inside the WHERE's
    // EXISTS subquery, so every sort arm must be evaluated against a real
    // schema (this used to fail for RecentlyAdded on a v1 DB).
    let store = store();
    // Three library albums with distinguishable timing, playtime and artist.
    let setups = [
        ("al-old", "Zeta", 1_000, 10_000, "Zed"),
        ("al-new", "Beta", 3_000, 1_000, "Abe"),
        ("al-mid", "Alpha", 2_000, 5_000, "Moe"),
    ];
    for (album_id, title, in_library_at, play_time, artist_name) in setups {
        store
            .upsert_album(&Album {
                id: album_id.to_string(),
                title: title.to_string(),
                ..Album::default()
            })
            .unwrap();
        let artist_id = format!("ar-{album_id}");
        let mut track = Song {
            id: format!("s-{album_id}"),
            title: title.to_string(),
            artists: vec![Artist::new(artist_name, Some(artist_id))],
            album: Some(AlbumRef {
                id: album_id.to_string(),
                title: title.to_string(),
            }),
            ..Song::default()
        };
        track.in_library = Some(in_library_at);
        store.upsert_song(&track).unwrap();
        store.record_play(&track.id, play_time, NOW).unwrap();
    }
    // A non-library album must never appear.
    store
        .upsert_album(&Album {
            id: "al-remote".to_string(),
            title: "AAA remote only".to_string(),
            ..Album::default()
        })
        .unwrap();

    let ids = |sort| {
        store
            .library_albums(sort, 50, 0)
            .unwrap_or_else(|error| panic!("{sort:?} failed: {error:?}"))
            .iter()
            .map(|album| album.id.clone())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        ids(LibrarySort::RecentlyAdded),
        vec!["al-new", "al-mid", "al-old"],
        "newest library addition first",
    );
    assert_eq!(
        ids(LibrarySort::Alphabetical),
        vec!["al-mid", "al-new", "al-old"]
    );
    assert_eq!(
        ids(LibrarySort::MostPlayed),
        vec!["al-old", "al-mid", "al-new"],
        "most listening time first",
    );
    assert_eq!(
        ids(LibrarySort::Artist),
        vec!["al-new", "al-mid", "al-old"],
        "ordered by first credited artist",
    );

    // The same four arms through the library page, which is what the UI calls.
    for sort in [
        LibrarySort::RecentlyAdded,
        LibrarySort::Alphabetical,
        LibrarySort::MostPlayed,
        LibrarySort::Artist,
    ] {
        let page = store
            .library_page(LibraryFilter::Albums, sort, 50, 0)
            .unwrap_or_else(|error| panic!("library_page({sort:?}) failed: {error:?}"));
        assert_eq!(page.albums.len(), 3, "{sort:?} leaked a non-library album");
    }
}

// ------------------------------------------------- schema and plumbing

#[test]
fn schema_is_versioned_and_migrate_is_idempotent() {
    let store = store();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    store.migrate().unwrap();
    store.migrate().unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
}

#[test]
fn every_expected_table_exists() {
    let store = store();
    let expected = [
        "song",
        "artist",
        "album",
        "playlist",
        "playlist_song_map",
        "song_artist_map",
        "song_album_map",
        "album_artist_map",
        "event",
        "play_count",
        "lyrics_cache",
        "search_history",
        "speed_dial",
        "kv",
    ];
    for table in expected {
        let found: Option<String> = store
            .with_connection(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
                        [table],
                        |row| row.get(0),
                    )
                    .ok())
            })
            .unwrap();
        assert_eq!(found.as_deref(), Some(table), "missing table {table}");
    }
}

#[test]
fn unique_constraints_reject_duplicates() {
    let store = store();
    store.upsert_song(&song("s")).unwrap();
    let playlist = store.create_playlist("Mix").unwrap();
    store.add_songs(&playlist, &[song("s")]).unwrap();

    let duplicate_map = store
        .with_connection(|conn| {
            Ok(conn.execute(
                "INSERT INTO playlist_song_map (playlist_id, song_id, position) VALUES (?1, 's', 0)",
                [playlist.as_str()],
            ))
        })
        .unwrap();
    assert!(
        duplicate_map.is_err(),
        "UNIQUE(playlist_id, song_id, position)"
    );

    let duplicate_query = store
        .with_connection(|conn| {
            Ok(conn.execute(
                "INSERT INTO search_history (query, timestamp) VALUES ('q', 1)",
                [],
            ))
        })
        .unwrap();
    assert!(duplicate_query.is_ok());
    let duplicate_query = store
        .with_connection(|conn| {
            Ok(conn.execute(
                "INSERT INTO search_history (query, timestamp) VALUES ('q', 2)",
                [],
            ))
        })
        .unwrap();
    assert!(duplicate_query.is_err(), "UNIQUE(query)");
}

#[test]
fn errors_surface_as_store_errors_not_panics() {
    let store = store();
    // Playing an unknown song violates the FK and must be an MlError.
    let error = store.record_play("ghost", 1_000, NOW).unwrap_err();
    assert!(matches!(error, MlError::Store(_)), "got {error:?}");
}

#[test]
fn file_database_is_wal_and_survives_reopen() {
    let dir = std::env::temp_dir().join(format!(
        "metrolist-store-integration-{}-{}",
        std::process::id(),
        ml_core::now_millis()
    ));
    let path = dir.join("library.db");
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        store.upsert_song(&titled("s", "Kept")).unwrap();
        store.set_liked_at("s", true, 42).unwrap();
        let journal: String = store
            .with_connection(|conn| {
                conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))
                    .map_err(|error| MlError::Store(error.to_string()))
            })
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
        let fk: i64 = store
            .with_connection(|conn| {
                conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                    .map_err(|error| MlError::Store(error.to_string()))
            })
            .unwrap();
        assert_eq!(fk, 1);
    }
    let reopened = Store::open(&path).unwrap();
    let song = reopened.get_song("s").unwrap().unwrap();
    assert_eq!(song.title, "Kept");
    assert_eq!(song.liked, Some(42));
    assert_eq!(reopened.schema_version().unwrap(), SCHEMA_VERSION);
    drop(reopened);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_newer_database_is_refused() {
    let dir = std::env::temp_dir().join(format!(
        "metrolist-store-future-{}-{}",
        std::process::id(),
        ml_core::now_millis()
    ));
    let path = dir.join("library.db");
    let store = Store::open(&path).unwrap();
    store
        .with_connection(|conn| {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
                .unwrap();
            Ok(())
        })
        .unwrap();
    assert!(store.migrate().is_err());
    drop(store);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_shared_store_serves_two_threads_concurrently() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let playlist = store.create_playlist("Shared").unwrap();
    store.upsert_songs(&[song("a"), song("b")]).unwrap();

    let writers: Vec<_> = (0..2)
        .map(|worker| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                for index in 0..40 {
                    let id = format!("w{worker}-{index}");
                    store.upsert_song(&song(&id)).unwrap();
                    store
                        .record_play(&id, 1_000, NOW + i64::from(index))
                        .unwrap();
                    assert!(store.get_song(&id).unwrap().is_some());
                }
            })
        })
        .collect();
    let reader = {
        let store = Arc::clone(&store);
        let playlist = playlist.clone();
        std::thread::spawn(move || {
            for _ in 0..40 {
                let _ = store.playlist_songs(&playlist).unwrap();
                let _ = store
                    .library_page(LibraryFilter::All, LibrarySort::Artist, 50, 0)
                    .unwrap();
            }
        })
    };

    for writer in writers {
        writer.join().unwrap();
    }
    reader.join().unwrap();
    assert_eq!(
        store
            .all_songs(LibrarySort::RecentlyAdded, 500, 0)
            .unwrap()
            .len(),
        82
    );
    assert_eq!(store.event_count().unwrap(), 80);
}

/// Panic-free surface: an empty store answers every read with an empty result
/// rather than an error.
#[test]
fn empty_store_answers_every_read() {
    let store = store();
    assert!(store
        .all_songs(LibrarySort::Alphabetical, 10, 0)
        .unwrap()
        .is_empty());
    assert!(store.liked_songs(10, 0).unwrap().is_empty());
    assert!(store.history(10, 0).unwrap().is_empty());
    assert!(store.latest_event().unwrap().is_none());
    assert!(store.playlists(10, 0).unwrap().is_empty());
    assert!(store.artists(10, 0).unwrap().is_empty());
    assert!(store.albums(10, 0).unwrap().is_empty());
    assert!(store.speed_dial().unwrap().is_empty());
    assert_eq!(store.total_play_time().unwrap(), 0);
    assert!(store.songs_by_ids(&[]).unwrap().is_empty());
}
