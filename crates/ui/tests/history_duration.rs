//! Playback history stores the listened duration, not a hardcoded zero.

use std::time::{Duration, Instant};

use ml_core::{now_millis, Artist, Settings, Song};
use ml_store::{StatsPeriod, Store};
use ml_ui::services::Services;
use ml_ui::settings_store::SettingsPaths;

#[test]
fn thirty_second_play_reaches_stats_total() {
    let root = std::env::temp_dir().join(format!(
        "metrolist-history-duration-{}-{}",
        std::process::id(),
        now_millis()
    ));
    let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
    let song = Song {
        id: "historyduration".to_string(),
        title: "Thirty seconds".to_string(),
        artists: vec![Artist::new("Test artist", None)],
        duration: Some(200),
        ..Song::default()
    };
    {
        let store = Store::open(&paths.database_file()).expect("seed store");
        store.upsert_song(&song).expect("seed song");
    }

    let ctx = eframe::egui::Context::default();
    let (services, _notes) = Services::new(&ctx, paths.clone(), &Settings::default());
    services.record_play(song.clone(), 30_000);

    let store = Store::open(&paths.database_file()).expect("read store");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut entry = None;
    while Instant::now() < deadline {
        entry = store
            .history(10, 0)
            .expect("read history")
            .into_iter()
            .find(|entry| entry.song.id == song.id && entry.play_time > 0);
        if entry.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let entry = entry.expect("play event with a non-zero play_time");
    assert_eq!(entry.play_time, 30_000);

    let report = store
        .stats_report(StatsPeriod::All, now_millis() as i64, 10)
        .expect("stats report");
    assert_eq!(report.summary.event_count, 1);
    assert_eq!(report.summary.total_ms, 30_000);
    println!(
        "history duration: play_time={} total_ms={}",
        entry.play_time, report.summary.total_ms
    );
    drop(store);
    drop(services);
    let _ = std::fs::remove_dir_all(root);
}
