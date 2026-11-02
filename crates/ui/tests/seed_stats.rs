//! Seeded temp-DB check for the UI stats service; no network.

use std::time::{Duration, Instant};

use ml_core::{now_millis, Artist, Settings, Song};
use ml_store::{StatsPeriod, Store};
use ml_ui::services::{Services, UiEvent};
use ml_ui::settings_store::SettingsPaths;

#[test]
fn seeded_plays_reach_the_stats_event() {
    let root = std::env::temp_dir().join(format!(
        "metrolist-stats-seed-{}-{}",
        std::process::id(),
        now_millis()
    ));
    let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
    let now = now_millis() as i64;
    {
        let store = Store::open(&paths.database_file()).expect("open seed store");
        let songs = vec![
            Song {
                id: "seedstatsa".to_string(),
                title: "Seed A".to_string(),
                artists: vec![Artist::new("Seed artist", None)],
                duration: Some(200),
                ..Song::default()
            },
            Song {
                id: "seedstatsb".to_string(),
                title: "Seed B".to_string(),
                artists: vec![Artist::new("Other artist", None)],
                duration: Some(240),
                ..Song::default()
            },
        ];
        store.upsert_songs(&songs).expect("upsert songs");
        store
            .record_play("seedstatsa", 120_000, now)
            .expect("play a");
        store
            .record_play("seedstatsa", 120_000, now - 86_400_000)
            .expect("play a again");
        store
            .record_play("seedstatsb", 180_000, now - 2 * 86_400_000)
            .expect("play b");
    }

    let ctx = eframe::egui::Context::default();
    let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
    services.load_stats(StatsPeriod::Week);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut report = None;
    while Instant::now() < deadline {
        while let Some(event) = services.try_recv() {
            if let UiEvent::StatsLoaded { period, result } = event {
                assert_eq!(period, StatsPeriod::Week);
                report = Some(result.expect("stats report"));
            }
        }
        if report.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let report = report.expect("stats event");
    assert_eq!(report.summary.event_count, 3);
    assert_eq!(report.summary.total_ms, 420_000);
    assert_eq!(report.summary.unique_songs, 2);
    assert_eq!(report.summary.unique_artists, 2);
    assert_eq!(report.top_songs.len(), 2);
    assert_eq!(report.top_songs[0].song.id, "seedstatsa");
    assert_eq!(report.top_songs[0].plays, 2);
    assert_eq!(
        report.monthly.iter().map(|month| month.plays).sum::<u64>(),
        3
    );
    println!(
        "stats seed: plays={} total_ms={} songs={} artists={} monthly={}",
        report.summary.event_count,
        report.summary.total_ms,
        report.summary.unique_songs,
        report.summary.unique_artists,
        report.monthly.iter().map(|month| month.plays).sum::<u64>()
    );
    drop(services);
    let _ = std::fs::remove_dir_all(root);
}
