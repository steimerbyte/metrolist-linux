//! Live tray test: registers a real StatusNotifierItem and checks the watcher
//! saw it, then drives the menu through D-Bus.
//!
//! Ignored by default; run with:
//! `ML_INTEGRATION_LIVE=1 cargo test -p ml-integrations --test tray_live -- --ignored --nocapture`

use std::path::Path;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver};
use ml_integrations::{spawn_tray, TrayAction, TrayHandle};

fn live_enabled() -> bool {
    std::env::var_os("ML_INTEGRATION_LIVE").is_some()
}

fn busctl_has_watcher() -> bool {
    std::process::Command::new("busctl")
        .args(["--user", "--no-pager", "list", "--no-legend"])
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout).contains("org.kde.StatusNotifierWatcher")
        })
        .unwrap_or(false)
}

/// `StatusNotifierItems` from the watcher, as busctl prints them.
fn registered_items() -> String {
    let output = std::process::Command::new("busctl")
        .args([
            "--user",
            "--no-pager",
            "get-property",
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
            "RegisteredStatusNotifierItems",
        ])
        .output()
        .expect("busctl");
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn wait_for(action: &Receiver<TrayAction>, expected: TrayAction) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match action.recv_timeout(Duration::from_millis(100)) {
            Ok(actual) if actual == expected => return true,
            Ok(other) => panic!("expected {expected:?}, got {other:?}"),
            Err(_) => continue,
        }
    }
    false
}

/// Invoke a dbusmenu item by its id on our own item.
///
/// `gdbus`, not `busctl`: busctl parses `-1`/`0` positionals as options.
fn activate_menu_item(service_name: &str, id: i32) -> bool {
    let output = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            service_name,
            "--object-path",
            "/MenuBar",
            "--method",
            "com.canonical.dbusmenu.Event",
            // `--`: the positionals start with a number, which gdbus would
            // otherwise parse as an option. Event takes (i, s, v, u).
            "--",
            &id.to_string(),
            "clicked",
            "<0>",
            "0",
        ])
        .output()
        .expect("gdbus");
    if !output.status.success() {
        eprintln!(
            "menu Event failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output.status.success()
}

/// Full dbusmenu layout of our item, as `gdbus` prints it.
fn menu_layout(service_name: &str) -> String {
    let output = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            service_name,
            "--object-path",
            "/MenuBar",
            "--method",
            "com.canonical.dbusmenu.GetLayout",
            "--",
            "0",
            "-1",
            "[]",
        ])
        .output()
        .expect("gdbus");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text
}

#[test]
#[ignore = "needs a StatusNotifierWatcher; run with ML_INTEGRATION_LIVE=1"]
fn tray_registers_and_menu_actions_reach_the_channel() {
    if !live_enabled() {
        eprintln!("skipping: ML_INTEGRATION_LIVE is not set");
        return;
    }
    if !busctl_has_watcher() {
        eprintln!("skipping: no org.kde.StatusNotifierWatcher on this session");
        return;
    }

    let (tx, rx) = bounded(32);
    let before = registered_items();
    let icon = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("assets/icons/metrolist-desktop-256.png"));
    let icon = icon.filter(|path| path.exists());

    let tray: TrayHandle =
        spawn_tray("Metrolist Desktop (test)", icon.as_deref(), tx).expect("tray must register");
    assert!(tray.is_running());
    println!("icon source: {:?}", tray.icon_source());

    let pid = std::process::id();
    let service_name = (0..8)
        .map(|index| format!("org.kde.StatusNotifierItem-{pid}-{index}"))
        .find(|name| {
            std::process::Command::new("busctl")
                .args(["--user", "status", name.as_str()])
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        })
        .expect("our SNI bus name must be owned");

    let after = registered_items();
    println!("RegisteredStatusNotifierItems before: {before}");
    println!("RegisteredStatusNotifierItems after:  {after}");
    assert!(
        after.contains(&pid.to_string()),
        "watcher did not register us: {after}"
    );
    assert_ne!(before, after, "registration must change the watcher list");

    // The watcher-visible properties.
    for (property, expected) in [
        ("Id", "org.metrolist.Metrolist"),
        ("Title", "Metrolist Desktop (test)"),
        ("Category", "ApplicationStatus"),
        ("Status", "Active"),
    ] {
        let value = std::process::Command::new("busctl")
            .args([
                "--user",
                "--no-pager",
                "get-property",
                service_name.as_str(),
                "/StatusNotifierItem",
                "org.kde.StatusNotifierItem",
                property,
            ])
            .output()
            .expect("busctl");
        let text = String::from_utf8_lossy(&value.stdout).trim().to_string();
        println!("SNI {property} -> {text}");
        assert!(text.contains(expected), "{property}: {text}");
    }

    // Menu layout: dbusmenu item ids are positional.
    let layout = menu_layout(&service_name);
    println!("menu layout -> {layout}");
    for label in ["Show", "Hide", "Play", "Next", "Previous", "Quit"] {
        assert!(layout.contains(label), "menu lost {label}: {layout}");
    }

    // dbusmenu ids come from the flattened menu; find the one labelled Next.
    let next_id = layout_id_for(&layout, "Next").expect("Next item id");
    assert!(activate_menu_item(&service_name, next_id));
    assert!(wait_for(&rx, TrayAction::Next), "Next click did not arrive");

    let quit_id = layout_id_for(&layout, "Quit").expect("Quit item id");
    assert!(activate_menu_item(&service_name, quit_id));
    assert!(wait_for(&rx, TrayAction::Quit), "Quit click did not arrive");

    // Play/Pause label follows playback state.
    tray.set_playing(true);
    std::thread::sleep(Duration::from_millis(200));
    let layout = menu_layout(&service_name);
    assert!(
        layout.contains("Pause"),
        "playing tray must offer Pause: {layout}"
    );

    drop(tray);
    // The tray thread owns the bus name; after shutdown it must be released.
    let released = {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let gone = !std::process::Command::new("busctl")
                .args(["--user", "status", service_name.as_str()])
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false);
            if gone || Instant::now() > deadline {
                break gone;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let after_drop = registered_items();
    println!("RegisteredStatusNotifierItems after drop: {after_drop}");
    assert!(released, "tray bus name still owned after drop");
}

/// `gdbus` renders each item as `<(5, {'label': <'Next'>, ...}, @av [])>`;
/// find the id that belongs to `label`.
fn layout_id_for(layout: &str, label: &str) -> Option<i32> {
    let needle = format!("'label': <'{label}'>");
    for group in layout.split("<(").skip(1) {
        if !group.contains(&needle) {
            continue;
        }
        let id = group
            .split(',')
            .next()?
            .trim()
            .trim_start_matches("uint32 ")
            .parse::<i32>()
            .ok()?;
        return Some(id);
    }
    None
}
