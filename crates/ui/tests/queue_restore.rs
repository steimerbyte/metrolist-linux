//! Persisted queue restore and mutation persistence through the UI bridge.

use std::time::{Duration, Instant};

use ml_core::{Queue, QueueSource, Settings, Song};
use ml_store::Store;
use ml_ui::services::Services;
use ml_ui::settings_store::SettingsPaths;

fn song(id: &str) -> Song {
    Song {
        id: id.to_string(),
        title: id.to_string(),
        duration: Some(60),
        ..Song::default()
    }
}

fn paths(name: &str) -> SettingsPaths {
    let root = std::env::temp_dir().join(format!("metrolist-queue-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"))
}

fn seed(paths: &SettingsPaths, raw: &str) {
    let store = Store::open(&paths.database_file()).expect("open store");
    store.kv_set("queue", raw).expect("seed queue");
}

fn persisted(paths: &SettingsPaths) -> Option<Queue> {
    let store = Store::open(&paths.database_file()).ok()?;
    store
        .kv_get("queue")
        .ok()
        .flatten()
        .and_then(|raw| ml_ui::pure::parse_persisted_queue(&raw))
}

#[test]
fn queue_is_restored_without_autoplay_and_mutations_persist() {
    let ctx = eframe::egui::Context::default();
    let paths = paths("restore");
    let mut queue = Queue::new(
        vec![
            song("aaaaaaaaaaa"),
            song("bbbbbbbbbbb"),
            song("ccccccccccc"),
        ],
        QueueSource::Album,
    );
    queue.jump_to(1);
    seed(&paths, &serde_json::to_string(&queue).unwrap());

    let (services, _notes) = Services::new(&ctx, paths.clone(), &Settings::default());
    assert_eq!(services.queue_display().0.len(), 3);
    assert_eq!(services.queue_display().1, Some(1));
    let mut stopped = false;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && !stopped {
        for event in services.poll() {
            match event {
                ml_audio::ControllerEvent::Snapshot(snapshot) => {
                    assert!(matches!(
                        snapshot.status,
                        ml_core::PlaybackStatus::Stopped | ml_core::PlaybackStatus::Idle
                    ));
                    assert_eq!(
                        snapshot.current.as_ref().map(|song| song.id.as_str()),
                        Some("bbbbbbbbbbb")
                    );
                    stopped = true;
                }
                ml_audio::ControllerEvent::NeedUrl { .. } => {
                    panic!("restored queue must not resolve or autoplay")
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(stopped, "restore snapshot missing");

    assert!(services.move_queue_item(1, 2));
    let (songs, position) = services.queue_display();
    assert_eq!(
        songs
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["aaaaaaaaaaa", "ccccccccccc", "bbbbbbbbbbb"]
    );
    assert_eq!(position, Some(2));

    assert!(services.remove_queue_item(0));
    assert_eq!(services.queue_display().1, Some(1));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(queue) = persisted(&paths) {
            if queue.len() == 2
                && queue.position() == 1
                && queue
                    .iter()
                    .map(|song| song.id.as_str())
                    .collect::<Vec<_>>()
                    == vec!["ccccccccccc", "bbbbbbbbbbb"]
            {
                break;
            }
        }
        assert!(Instant::now() < deadline, "mutated queue was not persisted");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn corrupt_or_duplicate_persisted_queue_starts_empty() {
    for (name, raw) in [
        ("corrupt", "{not json".to_string()),
        (
            "duplicate",
            serde_json::to_string(&Queue::new(
                vec![song("aaaaaaaaaaa"), song("aaaaaaaaaaa")],
                QueueSource::List,
            ))
            .unwrap(),
        ),
    ] {
        let ctx = eframe::egui::Context::default();
        let paths = paths(name);
        seed(&paths, &raw);
        let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
        assert!(services.queue_display().0.is_empty(), "{name}");
        assert_eq!(services.queue_display().1, None, "{name}");
    }
}
