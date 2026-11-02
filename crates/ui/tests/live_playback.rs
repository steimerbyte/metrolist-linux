//! Live playback smoke test: requires mpv, PipeWire and YouTube Music access.
//!
//! Run with `cargo test -p ml-ui -- --ignored --nocapture`. Normal CI runs skip
//! it; it exists to produce sink evidence (position advancing) on the host.

use std::time::{Duration, Instant};

use ml_core::{PlaybackStatus, Settings, Song};
use ml_ui::services::Services;
use ml_ui::settings_store::SettingsPaths;

#[test]
#[ignore = "requires mpv + audio sink + network"]
fn plays_one_stream() {
    let ctx = eframe::egui::Context::default();
    let root = std::env::temp_dir().join(format!("metrolist-live-{}", std::process::id()));
    let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
    let db_path = paths.database_file();
    {
        let (services, notes) = Services::new(&ctx, paths.clone(), &Settings::default());
        for note in &notes {
            eprintln!("note: {note}");
        }
        // Verified full-track sample from the audio backend report.
        let song = Song {
            id: "kJQP7kiw5Fk".to_string(),
            title: "Despacito".to_string(),
            duration: Some(282),
            ..Song::default()
        };
        services.play_song(song.clone());

        let deadline = Instant::now() + Duration::from_secs(25);
        let mut last = PlaybackStatus::Idle;
        while Instant::now() < deadline {
            // The app drains `UiEvent`s on its frame loop; the test has to do
            // the same so the radio queue reaches the controller.
            while let Some(event) = services.try_recv() {
                match event {
                    ml_ui::services::UiEvent::RadioReady {
                        songs,
                        index,
                        source,
                        ..
                    } => {
                        eprintln!("radio queue: {} tracks", songs.len());
                        services.play(songs, index, source);
                    }
                    ml_ui::services::UiEvent::PlaybackFailed { message, .. } => {
                        eprintln!("playback failed: {message}");
                    }
                    _ => {}
                }
            }
            for event in services.poll() {
                match event {
                    ml_audio::ControllerEvent::Snapshot(snapshot) => {
                        if snapshot.status != last {
                            eprintln!(
                                "status={:?} pos={}ms dur={}ms",
                                snapshot.status, snapshot.position_ms, snapshot.duration_ms
                            );
                        }
                        last = snapshot.status;
                    }
                    ml_audio::ControllerEvent::Error(error) => eprintln!("controller: {error}"),
                    ml_audio::ControllerEvent::NeedUrl { video_id, .. } => {
                        services.resolve(video_id);
                    }
                    _ => {}
                }
            }
            if matches!(
                last,
                PlaybackStatus::Playing | PlaybackStatus::Paused | PlaybackStatus::Buffering
            ) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            matches!(
                last,
                PlaybackStatus::Playing | PlaybackStatus::Paused | PlaybackStatus::Buffering
            ),
            "audio never started (last status {last:?})"
        );

        // Poll for a few seconds: position advances only while `poll` drains the
        // engine's 10 Hz Position events.
        let mut max_position = 0u64;
        let listen_secs = std::env::var("ML_LIVE_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(4);
        let listen_until = Instant::now() + Duration::from_secs(listen_secs);
        while Instant::now() < listen_until {
            while let Some(event) = services.try_recv() {
                if let ml_ui::services::UiEvent::RadioReady {
                    songs,
                    index,
                    source,
                    ..
                } = event
                {
                    eprintln!("radio queue: {} tracks", songs.len());
                    services.play(songs, index, source);
                }
            }
            for event in services.poll() {
                if let ml_audio::ControllerEvent::NeedUrl { video_id, .. } = event {
                    services.resolve(video_id);
                }
            }
            let snapshot = services.snapshot();
            max_position = max_position.max(snapshot.position_ms);
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!(
            "max position observed = {max_position}ms, queue tracks = {}",
            services.queue_view().songs.len()
        );
        assert!(
            services.queue_view().songs.len() > 1,
            "next/radio queue was not installed"
        );
        assert!(
            max_position > 500,
            "sink position did not advance: {max_position}ms"
        );

        // Store round-trip: search history, play event and queue snapshot are
        // written on the DB thread; a second connection reads them back.
        services.record_search("metrolist live probe".to_string());
        services.record_play(song.clone(), max_position);
        services.persist_queue();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut found = false;
        while Instant::now() < deadline && !found {
            found = services
                .search_history(10)
                .iter()
                .any(|query| query == "metrolist live probe");
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(found, "search history was not persisted");
        let store = ml_store::Store::open(&db_path).expect("open library db");
        let history = store.history(20, 0).expect("read play history");
        assert!(
            history
                .iter()
                .any(|entry| entry.song.id == song.id && entry.play_time > 0),
            "play event was not recorded with its listened duration"
        );
        let queue_json = store
            .kv_get("queue")
            .expect("read queue snapshot")
            .unwrap_or_default();
        assert!(
            queue_json.contains(&song.id),
            "queue snapshot was not persisted"
        );

        // Transport parity: seek, volume, jump, shuffle and repeat all reach
        // the controller while audio is live.
        services.seek(30_000);
        let seek_deadline = Instant::now() + Duration::from_secs(4);
        let mut seek_applied = false;
        while Instant::now() < seek_deadline && !seek_applied {
            for event in services.poll() {
                if let ml_audio::ControllerEvent::NeedUrl { video_id, .. } = event {
                    services.resolve(video_id);
                }
            }
            let snapshot = services.snapshot();
            seek_applied = snapshot.position_ms >= 30_100;
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            seek_applied,
            "seek did not reach the sink (position {}ms)",
            services.snapshot().position_ms
        );

        let seek_position = services.snapshot().position_ms;
        services.set_volume(0.25);
        let volume = services.snapshot().volume;
        assert!((volume - 0.25).abs() < 0.01, "volume not applied: {volume}");

        let before = services.queue_view();
        assert!(before.songs.len() > 1, "need a multi-track queue to jump");
        services.jump_queue(1);
        let jumped = services.queue_view();
        assert_eq!(
            jumped.position,
            Some(1),
            "jump did not move the queue index"
        );

        services.toggle_shuffle();
        assert!(
            services.queue_view().shuffle,
            "shuffle was not enabled on the queue"
        );
        let repeat_before = services.queue_view().repeat;
        services.cycle_repeat();
        assert_ne!(
            services.queue_view().repeat,
            repeat_before,
            "repeat did not cycle"
        );
        eprintln!(
            "controls verified: seek={seek_position}ms jump position=1 volume={volume} shuffle={} repeat={:?}",
            services.queue_view().shuffle,
            services.queue_view().repeat
        );
    }
    std::fs::remove_dir_all(root).ok();
}
