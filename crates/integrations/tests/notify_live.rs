//! Live notification test: posts through the real
//! `org.freedesktop.Notifications` daemon and checks `Notify` was accepted.
//!
//! Run with:
//! `ML_INTEGRATION_LIVE=1 cargo test -p ml-integrations --test notify_live -- --ignored --nocapture`

use ml_core::{PlaybackStatus, PlayerSnapshot, Song};
use ml_integrations::{notify, notify_with, NowPlayingNotifier};

fn live_enabled() -> bool {
    std::env::var_os("ML_INTEGRATION_LIVE").is_some()
}

fn daemon_present() -> bool {
    std::process::Command::new("busctl")
        .args(["--user", "status", "org.freedesktop.Notifications"])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Subscribe to `Notify` calls and return the captured text.
fn capture_notify(action: impl FnOnce()) -> String {
    let mut monitor = std::process::Command::new("busctl")
        .args(["--user", "monitor", "org.freedesktop.Notifications"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("busctl monitor");
    std::thread::sleep(std::time::Duration::from_millis(350));
    action();
    std::thread::sleep(std::time::Duration::from_millis(600));
    let _ = monitor.kill();
    monitor
        .wait_with_output()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default()
}

#[test]
#[ignore = "needs org.freedesktop.Notifications; run with ML_INTEGRATION_LIVE=1"]
fn notifications_reach_the_desktop_daemon() {
    if !live_enabled() {
        eprintln!("skipping: ML_INTEGRATION_LIVE is not set");
        return;
    }
    assert!(
        daemon_present(),
        "no org.freedesktop.Notifications on this bus"
    );

    // Plain notification.
    let log = capture_notify(|| {
        notify("Metrolist Desktop", "integration test notification")
            .expect("daemon must accept the notification");
    });
    println!("--- Notify traffic\n{log}");
    assert!(log.contains("Notify"), "no Notify call observed:\n{log}");

    // Now-playing: first snapshot notifies, a repeat does not.
    let song = Song {
        id: "dQw4w9WgXcQ".into(),
        title: "Never Gonna Give You Up".into(),
        artists: vec![ml_core::Artist::new("Rick Astley", None)],
        duration: Some(213),
        thumbnail_url: Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg".into()),
        ..Song::default()
    };
    let snapshot = PlayerSnapshot {
        status: PlaybackStatus::Playing,
        current: Some(song),
        ..PlayerSnapshot::default()
    };
    let mut notifier = NowPlayingNotifier::new();
    notifier.set_enabled(true);
    let log = capture_notify(|| {
        notifier.on_snapshot(&snapshot).expect("now playing");
        notifier
            .on_snapshot(&snapshot)
            .expect("repeat must be a no-op");
    });
    println!("--- now-playing traffic\n{log}");
    assert!(
        log.contains("Never Gonna Give You Up"),
        "title missing:\n{log}"
    );
    assert!(log.contains("Rick Astley"), "artist missing:\n{log}");
    // Count client -> daemon calls only: gnome-shell re-relays each accepted
    // notification to a private connection, which is a second `Notify`.
    let notify_calls = log
        .lines()
        .filter(|line| {
            line.contains("Member=Notify")
                && line.contains("Destination=org.freedesktop.Notifications")
        })
        .count();
    println!("client Notify calls: {notify_calls}");
    assert_eq!(notify_calls, 1, "exactly one bubble per track:\n{log}");

    // An explicit icon path is passed through as the app icon.
    let log = capture_notify(|| {
        notify_with(
            "Metrolist Desktop",
            "icon override",
            Some("metrolist-desktop"),
        )
        .expect("icon notification");
    });
    assert!(log.contains("metrolist-desktop"), "icon missing:\n{log}");
}
