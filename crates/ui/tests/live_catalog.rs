//! Opt-in live catalog check: counts, muted direct playback, no radio queue.
//!
//! `cargo test -p ml-ui --test live_catalog -- --ignored --nocapture`

use std::time::{Duration, Instant};

use ml_core::{QueueSource, Settings, Song};
use ml_ui::services::{Services, UiEvent};
use ml_ui::settings_store::SettingsPaths;

fn wait_event(
    services: &Services,
    deadline: Instant,
    mut accept: impl FnMut(&UiEvent) -> bool,
) -> UiEvent {
    loop {
        while let Some(event) = services.try_recv() {
            if accept(&event) {
                return event;
            }
        }
        assert!(Instant::now() < deadline, "catalog event timed out");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "live network and audio output"]
fn live_catalog_counts_and_muted_direct_playback() {
    let root = std::env::temp_dir().join(format!("metrolist-catalog-live-{}", std::process::id()));
    let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
    let ctx = eframe::egui::Context::default();
    let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
    let deadline = Instant::now() + Duration::from_secs(45);

    services.load_charts();
    let charts = wait_event(&services, deadline, |event| {
        matches!(event, UiEvent::BrowseLoaded { .. })
    });
    let UiEvent::BrowseLoaded { result, .. } = charts else {
        unreachable!()
    };
    let charts = result.expect("charts page");
    println!(
        "catalog live: charts sections={} items={}",
        charts.sections.len(),
        charts.sections.iter().map(|s| s.items.len()).sum::<usize>()
    );

    services.load_moods();
    let moods = wait_event(&services, deadline, |event| {
        matches!(event, UiEvent::MoodsLoaded(_))
    });
    let UiEvent::MoodsLoaded(result) = moods else {
        unreachable!()
    };
    let moods = result.expect("moods page");
    println!(
        "catalog live: mood groups={} categories={}",
        moods.groups.len(),
        moods.groups.iter().map(|g| g.items.len()).sum::<usize>()
    );

    services.load_new_releases();
    let releases = wait_event(&services, deadline, |event| {
        matches!(event, UiEvent::BrowseLoaded { .. })
    });
    let UiEvent::BrowseLoaded { result, .. } = releases else {
        unreachable!()
    };
    let releases = result.expect("new releases page");
    println!(
        "catalog live: new release sections={} items={}",
        releases.sections.len(),
        releases
            .sections
            .iter()
            .map(|s| s.items.len())
            .sum::<usize>()
    );

    services.load_podcasts();
    let podcasts = wait_event(&services, deadline, |event| {
        matches!(event, UiEvent::PodcastsLoaded(_))
    });
    let UiEvent::PodcastsLoaded(result) = podcasts else {
        unreachable!()
    };
    let podcasts = result.expect("podcasts page");
    let episodes: Vec<Song> = podcasts
        .sections
        .iter()
        .flat_map(|section| section.items.iter())
        .filter_map(|item| match item {
            ml_core::YtItem::Episode(episode) => Some(episode.as_song()),
            _ => None,
        })
        .collect();
    println!(
        "catalog live: podcast sections={} episodes={}",
        podcasts.sections.len(),
        episodes.len()
    );
    assert!(!episodes.is_empty(), "podcasts returned no episodes");

    let section_len = podcasts
        .sections
        .iter()
        .find(|section| !section.items.is_empty())
        .map(|section| section.items.len())
        .unwrap_or(1);
    let songs: Vec<Song> = episodes.into_iter().take(section_len).collect();
    services.set_muted(true);
    services.play(songs.clone(), 0, QueueSource::List);
    let started = Instant::now();
    let mut position = 0;
    while started.elapsed() < Duration::from_secs(25) {
        for event in services.poll() {
            match event {
                ml_audio::ControllerEvent::NeedUrl { video_id, .. } => {
                    services.resolve(video_id);
                }
                ml_audio::ControllerEvent::Error(error) => eprintln!("controller: {error}"),
                _ => {}
            }
        }
        let snapshot = services.snapshot();
        position = snapshot.position_ms;
        if position > 1_500 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let (queue, _) = services.queue_display();
    let muted = services.snapshot().muted;
    println!(
        "catalog live: playback position_ms={position} muted={muted} queue={} radio_extendable={}",
        queue.len(),
        services.radio_can_extend()
    );
    assert!(position > 1_500, "playback did not advance");
    assert!(muted, "playback was not muted");
    assert_eq!(queue.len(), songs.len(), "catalog play added radio tracks");
    assert!(
        !services.radio_can_extend(),
        "catalog play started a radio queue"
    );
    drop(services);
    let _ = std::fs::remove_dir_all(root);
}
