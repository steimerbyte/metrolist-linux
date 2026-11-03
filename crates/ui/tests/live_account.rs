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

/// Opt-in file import through the same `Services` path the Account screen
/// uses. Without `ML_LIVE_DATA_DIR` the session copy lives in a private dir
/// under the app data root (never `/tmp`) and is removed afterwards; when set,
/// the real app data dir is used and kept (only a scratch config/cache dir is
/// deleted), so the account cache written here is what a later app start shows.
///
/// `ML_LIVE_COOKIES=~/Downloads/cookies.txt cargo test -p ml-ui --test live_account -- --ignored --nocapture`
#[test]
#[ignore = "reads a live cookies file and makes network requests"]
fn cookie_file_import_signs_in_and_syncs() {
    use std::os::unix::fs::PermissionsExt;

    let Some(source) = std::env::var_os("ML_LIVE_COOKIES") else {
        println!("account file live: skipped, ML_LIVE_COOKIES is not set");
        return;
    };
    let source = std::path::PathBuf::from(source);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    // The import itself receives the path as typed (possibly `~/…`); this copy
    // is only for the post-run rotation comparison.
    let source_abs = ml_ui::pure::expand_import_path(&source.to_string_lossy(), home.as_deref());
    let keep_data = std::env::var_os("ML_LIVE_DATA_DIR").is_some();
    let root = home
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".local/share/metrolist-desktop")
        .join(format!("live-test-{}", std::process::id()));
    let data_dir = match std::env::var_os("ML_LIVE_DATA_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => root.join("data"),
    };
    let scratch = if keep_data {
        root.clone()
    } else {
        root.join("scratch")
    };
    let _ = std::fs::remove_dir_all(&root);
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&root);
    };
    let paths = SettingsPaths::new(
        scratch.join("config"),
        data_dir.clone(),
        scratch.join("cache"),
    );
    let ctx = eframe::egui::Context::default();
    let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
    let session_file = data_dir.join("session-cookies.txt");

    services.import_cookie_file(source.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut synced = false;
    let mut signed_in = false;
    while Instant::now() < deadline {
        while let Some(event) = services.try_recv() {
            match event {
                UiEvent::AccountUpdated { session, .. } => {
                    println!(
                        "account file live: signed_in={} has_info={} cookies={}",
                        session.signed_in,
                        session.info.is_some(),
                        session.snapshot.cookie_count
                    );
                    assert!(session.signed_in);
                    let Some(info) = session.info else {
                        continue;
                    };
                    assert!(!info.name.is_empty());
                    println!(
                        "account file live: name={} email={:?} handle={:?}",
                        info.masked().name,
                        info.masked().email,
                        info.masked().channel_handle
                    );
                    let mode = std::fs::metadata(&session_file)
                        .expect("session file written")
                        .permissions()
                        .mode()
                        & 0o777;
                    println!("account file live: session mode={mode:04o}");
                    assert_eq!(mode, 0o600);
                    if !signed_in {
                        signed_in = true;
                        services.sync_library();
                    }
                }
                UiEvent::AccountError { message, cleared } => {
                    println!("account file live: error cleared={cleared}: {message}");
                    cleanup();
                    panic!("account file live: account error: {message}");
                }
                UiEvent::LibrarySynced { counts, .. } => {
                    println!(
                        "account file live: synced songs={} albums={} artists={} playlists={}",
                        counts.songs, counts.albums, counts.artists, counts.playlists
                    );
                    assert!(counts.songs > 0 && counts.playlists > 0);
                    synced = signed_in;
                }
                UiEvent::LibrarySyncFailed(message) => {
                    println!("account file live: sync failed: {message}");
                    cleanup();
                    panic!("account file live: sync failed: {message}");
                }
                _ => {}
            }
        }
        if synced {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(services);
    let session_copy = std::fs::read(&session_file).ok();
    let rotated = session_copy
        .as_deref()
        .map(|bytes| bytes != std::fs::read(&source_abs).unwrap_or_default())
        .unwrap_or(false);
    println!("account file live: session rotated={rotated}");
    cleanup();
    assert!(
        synced,
        "account file live: no account/sync result within 30 s"
    );
}
