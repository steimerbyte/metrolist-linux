//! Opt-in browser-cookie import. Reads the local LibreWolf/Firefox cookie DB,
//! writes only under a temp data dir, and prints status/counts, never cookies.
//!
//! `cargo test -p ml-ui --test live_account -- --ignored --nocapture`

use std::time::{Duration, Instant};

use ml_core::Settings;
use ml_ui::services::{Services, UiEvent};
use ml_ui::settings_store::SettingsPaths;

#[test]
#[ignore = "reads the live browser cookie DB and makes one network request"]
fn browser_import_reports_the_live_account_status() {
    let root = std::env::temp_dir().join(format!("metrolist-account-live-{}", std::process::id()));
    let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
    let ctx = eframe::egui::Context::default();
    let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
    let Some(profile) = services.browser_profile_name() else {
        println!("account live: no browser profile found");
        return;
    };
    println!("account live: importing profile {profile}");
    services.import_browser_cookies();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut synced = false;
    while Instant::now() < deadline {
        while let Some(event) = services.try_recv() {
            match event {
                UiEvent::AccountUpdated { session, .. } => {
                    println!(
                        "account live: signed_in={} has_info={} cookies={}",
                        session.signed_in,
                        session.info.is_some(),
                        session.snapshot.cookie_count
                    );
                    if synced {
                        println!("account live: done");
                        drop(services);
                        let _ = std::fs::remove_dir_all(root);
                        return;
                    }
                    if session.signed_in && session.info.is_some() {
                        synced = true;
                        services.sync_library();
                    }
                }
                UiEvent::AccountError { message, cleared } => {
                    println!("account live: error cleared={cleared}: {message}");
                    assert!(!message.contains("SAPISID="), "cookie value in error");
                    drop(services);
                    let _ = std::fs::remove_dir_all(root);
                    return;
                }
                UiEvent::LibrarySynced { counts, .. } => {
                    println!(
                        "account live: synced songs={} albums={} artists={} playlists={}",
                        counts.songs, counts.albums, counts.artists, counts.playlists
                    );
                    drop(services);
                    let _ = std::fs::remove_dir_all(root);
                    return;
                }
                UiEvent::LibrarySyncFailed(message) => {
                    println!("account live: sync failed: {message}");
                    drop(services);
                    let _ = std::fs::remove_dir_all(root);
                    return;
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(services);
    let _ = std::fs::remove_dir_all(root);
    panic!("account live: no account/sync result within 30 s");
}
