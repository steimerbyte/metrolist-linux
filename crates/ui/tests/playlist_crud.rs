//! Local playlist CRUD through the UI service bridge (no audio/network).

use std::time::{Duration, Instant};

use ml_core::{Settings, Song};
use ml_store::Store;
use ml_ui::services::{Services, UiEvent};
use ml_ui::settings_store::SettingsPaths;

fn wait_for<T>(services: &Services, mut pick: impl FnMut(UiEvent) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        while let Some(event) = services.try_recv() {
            if let Some(value) = pick(event) {
                return value;
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("service event was not delivered");
}

/// Wait until the mutation for `id` has committed on the DB thread.
fn wait_dirty(services: &Services, id: &str) {
    wait_for(services, |event| match event {
        UiEvent::LibraryDirty {
            playlist: Some(playlist),
        } if playlist == id => Some(()),
        _ => None,
    });
}

fn load(services: &Services, id: &str) -> ml_ui::state::LocalPlaylistView {
    services.load_local_playlist(id.to_string());
    wait_for(services, |event| match event {
        UiEvent::LocalPlaylistLoaded(view) if view.id == id => Some(view),
        _ => None,
    })
}

#[test]
fn playlist_crud_round_trip() {
    let ctx = eframe::egui::Context::default();
    let root = std::env::temp_dir().join(format!("metrolist-playlist-{}", std::process::id()));
    let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
    let db_path = paths.database_file();
    {
        let (services, _notes) = Services::new(&ctx, paths.clone(), &Settings::default());

        services.create_local_playlist("Road trip".to_string());
        let (id, name) = wait_for(&services, |event| match event {
            UiEvent::PlaylistCreated { id, name } => Some((id, name)),
            _ => None,
        });
        assert_eq!(name, "Road trip");
        wait_for(&services, |event| match event {
            UiEvent::LibraryDirty { playlist: None } => Some(()),
            _ => None,
        });

        let first = Song {
            id: "aaaaaaaaaaa".to_string(),
            title: "First".to_string(),
            duration: Some(60),
            ..Song::default()
        };
        let second = Song {
            id: "bbbbbbbbbbb".to_string(),
            title: "Second".to_string(),
            duration: Some(90),
            ..Song::default()
        };
        services.add_song_to_playlist(id.clone(), first.clone());
        wait_dirty(&services, &id);
        services.add_song_to_playlist(id.clone(), second.clone());
        wait_dirty(&services, &id);
        let view = load(&services, &id);
        assert_eq!(
            view.songs.iter().map(|song| &song.id).collect::<Vec<_>>(),
            vec![&first.id, &second.id],
            "insertion order"
        );

        services.move_playlist_song(id.clone(), 0, 1);
        wait_dirty(&services, &id);
        let view = load(&services, &id);
        assert_eq!(
            view.songs.iter().map(|song| &song.id).collect::<Vec<_>>(),
            vec![&second.id, &first.id],
            "reorder"
        );

        services.remove_song_from_playlist(id.clone(), first.id.clone());
        wait_dirty(&services, &id);
        let view = load(&services, &id);
        assert_eq!(view.songs.len(), 1);
        assert_eq!(view.songs[0].id, second.id);

        services.rename_local_playlist(id.clone(), "Renamed".to_string());
        wait_dirty(&services, &id);
        let view = load(&services, &id);
        assert_eq!(view.name, "Renamed");

        services.delete_local_playlist(id.clone());
        let deleted = wait_for(&services, |event| match event {
            UiEvent::PlaylistDeleted { id } => Some(id),
            _ => None,
        });
        assert_eq!(deleted, id);

        // Player library/like toggles go through the same bridge and mark the
        // library dirty so open screens refresh.
        services.set_in_library(second.clone(), true);
        wait_for(&services, |event| match event {
            UiEvent::LibraryDirty { playlist: None } => Some(()),
            _ => None,
        });
        services.set_liked(second.clone(), true);
        wait_for(&services, |event| match event {
            UiEvent::LibraryDirty { playlist: None } => Some(()),
            _ => None,
        });
        let store = Store::open(&db_path).expect("open library db");
        let stored = store
            .get_song(&second.id)
            .expect("read song")
            .expect("song persisted");
        assert!(stored.in_library.is_some(), "library flag not stored");
        assert!(stored.liked.is_some(), "like flag not stored");
    }
    std::fs::remove_dir_all(root).ok();
}
