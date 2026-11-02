//! Live MPRIS integration test: registers the real service with a fake
//! [`PlayerControl`] on the session bus and drives it with `busctl`.
//!
//! Ignored by default because it needs a running session bus. Run with:
//!
//! ```text
//! ML_INTEGRATION_LIVE=1 cargo test -p ml-integrations --test mpris_live -- --ignored --nocapture
//! ```
//!
//! It fails (rather than skips) if the bus is reachable but the checks do not
//! pass; a machine with no session bus reports that explicitly.

use std::process::Command;
use std::sync::{Arc, Mutex};

use ml_core::{AlbumRef, Artist, MlResult, PlaybackStatus, PlayerSnapshot, RepeatMode, Song};
use ml_integrations::{MprisService, PlayerControl};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.metrolist";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Default)]
struct FakeControl {
    snapshot: Mutex<PlayerSnapshot>,
    log: Mutex<Vec<String>>,
}

impl FakeControl {
    fn new() -> Arc<Self> {
        let mut control = FakeControl::default();
        let mut snapshot = PlayerSnapshot {
            status: PlaybackStatus::Playing,
            position_ms: 42_000,
            duration_ms: 213_000,
            volume: 0.75,
            shuffle: false,
            repeat: RepeatMode::Off,
            queue_len: 3,
            queue_index: Some(0),
            ..PlayerSnapshot::default()
        };
        let song = Song {
            id: "dQw4w9WgXcQ".into(),
            title: "Never Gonna Give You Up".into(),
            artists: vec![Artist::new("Rick Astley", None)],
            duration: Some(213),
            thumbnail_url: Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg".into()),
            album: Some(AlbumRef {
                id: "al".into(),
                title: "Whenever You Need Somebody".into(),
            }),
            ..Song::default()
        };
        snapshot.current = Some(song.clone());
        *control.snapshot.get_mut().unwrap() = snapshot;
        Arc::new(control)
    }

    fn record(&self, what: impl Into<String>) {
        self.log.lock().unwrap().push(what.into());
    }

    fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn set_status(&self, status: PlaybackStatus) {
        self.snapshot.lock().unwrap().status = status;
    }

    fn snapshot_now(&self) -> PlayerSnapshot {
        self.snapshot.lock().unwrap().clone()
    }

    /// Move the fake player's own state, as the real audio engine would.
    fn set_snapshot(&self, snapshot: PlayerSnapshot) {
        *self.snapshot.lock().unwrap() = snapshot;
    }
}

impl PlayerControl for FakeControl {
    fn play(&self) -> MlResult<()> {
        self.record("play");
        self.set_status(PlaybackStatus::Playing);
        Ok(())
    }
    fn pause(&self) -> MlResult<()> {
        self.record("pause");
        self.set_status(PlaybackStatus::Paused);
        Ok(())
    }
    fn play_pause(&self) -> MlResult<()> {
        self.record("play_pause");
        let next = if self.snapshot.lock().unwrap().status == PlaybackStatus::Playing {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        };
        self.set_status(next);
        Ok(())
    }
    fn stop(&self) -> MlResult<()> {
        self.record("stop");
        self.set_status(PlaybackStatus::Stopped);
        Ok(())
    }
    fn next(&self) -> MlResult<()> {
        self.record("next");
        self.snapshot.lock().unwrap().queue_index = Some(1);
        Ok(())
    }
    fn previous(&self) -> MlResult<()> {
        self.record("previous");
        Ok(())
    }
    fn seek_by_ms(&self, offset_ms: i64) -> MlResult<()> {
        self.record(format!("seek_by_ms:{offset_ms}"));
        let mut snapshot = self.snapshot.lock().unwrap();
        snapshot.position_ms = snapshot
            .position_ms
            .saturating_add_signed(offset_ms)
            .min(snapshot.duration_ms);
        Ok(())
    }
    fn set_position_ms(&self, position_ms: u64) -> MlResult<()> {
        self.record(format!("set_position_ms:{position_ms}"));
        self.snapshot.lock().unwrap().position_ms = position_ms;
        Ok(())
    }
    fn set_volume(&self, volume: f64) -> MlResult<()> {
        self.record(format!("set_volume:{volume:.3}"));
        self.snapshot.lock().unwrap().volume = volume as f32;
        Ok(())
    }
    fn set_shuffle(&self, shuffle: bool) -> MlResult<()> {
        self.record(format!("set_shuffle:{shuffle}"));
        self.snapshot.lock().unwrap().shuffle = shuffle;
        Ok(())
    }
    fn set_repeat(&self, repeat: RepeatMode) -> MlResult<()> {
        self.record(format!("set_repeat:{}", repeat.as_str()));
        self.snapshot.lock().unwrap().repeat = repeat;
        Ok(())
    }
    fn state(&self) -> PlayerSnapshot {
        self.snapshot_now()
    }
    fn current_song(&self) -> Option<Song> {
        self.snapshot.lock().unwrap().current.clone()
    }
    fn current_art_url(&self) -> Option<String> {
        Some("file:///tmp/metrolist-cover.png".into())
    }
    fn raise(&self) -> MlResult<()> {
        self.record("raise");
        Ok(())
    }
    fn open_uri(&self, uri: &str) -> MlResult<()> {
        self.record(format!("open_uri:{uri}"));
        Ok(())
    }
    fn quit(&self) -> MlResult<()> {
        self.record("quit");
        Ok(())
    }
}

fn live_enabled() -> bool {
    std::env::var_os("ML_INTEGRATION_LIVE").is_some()
}

fn bus_available() -> bool {
    Command::new("busctl")
        .args(["--user", "status"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// `busctl --user call ... Get` for one property, as raw stdout text.
fn get_property(interface: &str, property: &str) -> String {
    let output = Command::new("busctl")
        .args([
            "--user",
            "call",
            BUS_NAME,
            OBJECT_PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            "ss",
            interface,
            property,
        ])
        .output()
        .expect("busctl must run");
    assert!(
        output.status.success(),
        "Get {interface}.{property} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn call_method(interface: &str, method: &str) -> String {
    let output = Command::new("busctl")
        .args(["--user", "call", BUS_NAME, OBJECT_PATH, interface, method])
        .output()
        .expect("busctl must run");
    assert!(
        output.status.success(),
        "{method} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn wait_until(timeout_ms: u128, mut check: impl FnMut() -> bool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < timeout_ms {
        if check() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    check()
}

#[test]
#[ignore = "needs a live D-Bus session; run with ML_INTEGRATION_LIVE=1"]
fn mpris_service_answers_busctl_calls() {
    if !live_enabled() {
        eprintln!("skipping: ML_INTEGRATION_LIVE is not set");
        return;
    }
    assert!(bus_available(), "ML_INTEGRATION_LIVE=1 but no session bus");

    let control = FakeControl::new();
    let service = MprisService::spawn(control.clone()).expect("MPRIS bus name must register");
    assert!(service.is_running());

    // --- introspection -----------------------------------------------------
    let introspect = Command::new("busctl")
        .args(["--user", "introspect", BUS_NAME, OBJECT_PATH])
        .output()
        .expect("busctl must run");
    assert!(introspect.status.success());
    let tree = String::from_utf8_lossy(&introspect.stdout).to_string();
    println!("--- busctl introspect {BUS_NAME} {OBJECT_PATH}\n{tree}");
    for expected in [
        ROOT_IFACE,
        PLAYER_IFACE,
        ".Play",
        ".Pause",
        ".PlayPause",
        ".Next",
        ".Previous",
        ".Stop",
        ".Seek",
        ".SetPosition",
        ".OpenUri",
        // `busctl` renders properties as `<indent>.Name<indent>property`.
        ".Identity",
        ".DesktopEntry",
        ".SupportedUriSchemes",
        ".PlaybackStatus",
        ".Metadata",
        ".Position",
        ".Volume",
        ".LoopStatus",
        ".Shuffle",
        ".Seeked",
    ] {
        assert!(
            tree.contains(expected),
            "introspection lost {expected}:\n{tree}"
        );
    }

    // --- root properties ---------------------------------------------------
    assert!(get_property(ROOT_IFACE, "Identity").contains("Metrolist Desktop"));
    assert!(get_property(ROOT_IFACE, "DesktopEntry").contains("Metrolist-Desktop"));
    let schemes = get_property(ROOT_IFACE, "SupportedUriSchemes");
    assert!(schemes.contains("metrolist"), "{schemes}");

    // --- player properties -------------------------------------------------
    // `Properties.Get` returns a variant, so `busctl` prints `v <sig> <value>`.
    assert_eq!(
        get_property(PLAYER_IFACE, "PlaybackStatus"),
        "v s \"Playing\""
    );
    assert_eq!(get_property(PLAYER_IFACE, "Volume"), "v d 0.75");
    assert_eq!(get_property(PLAYER_IFACE, "LoopStatus"), "v s \"None\"");
    assert_eq!(get_property(PLAYER_IFACE, "Shuffle"), "v b false");

    // Position is microseconds: 42_000 ms -> 42_000_000 us.
    let position = get_property(PLAYER_IFACE, "Position");
    println!("Position -> {position}");
    assert!(
        position.contains("42000000"),
        "position not in µs: {position}"
    );

    // Metadata: title/artist/album/artUrl/trackid/length.
    let metadata = get_property(PLAYER_IFACE, "Metadata");
    println!("Metadata -> {metadata}");
    for expected in [
        "Never Gonna Give You Up",
        "Rick Astley",
        "Whenever You Need Somebody",
        "file:///tmp/metrolist-cover.png",
        "/com/metrolist/track/",
        "213000000",
    ] {
        assert!(
            metadata.contains(expected),
            "metadata lost {expected}: {metadata}"
        );
    }

    // --- methods reach the control ----------------------------------------
    let _ = call_method(PLAYER_IFACE, "PlayPause");
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"play_pause".to_string())));
    assert!(
        wait_until(2_000, || get_property(PLAYER_IFACE, "PlaybackStatus")
            .contains("Paused")),
        "PlayPause must flip the published status"
    );

    let _ = call_method(PLAYER_IFACE, "Next");
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"next".to_string())));

    let _ = call_method(PLAYER_IFACE, "Previous");
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"previous".to_string())));

    // Seek takes a signed microsecond offset: +5_000_000 us == +5000 ms.
    let seek = Command::new("busctl")
        .args([
            "--user",
            "call",
            BUS_NAME,
            OBJECT_PATH,
            PLAYER_IFACE,
            "Seek",
            "x",
            "5000000",
        ])
        .output()
        .expect("busctl must run");
    assert!(
        seek.status.success(),
        "{}",
        String::from_utf8_lossy(&seek.stderr)
    );
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"seek_by_ms:5000".to_string())));

    // SetPosition: path must match the current track, position in µs.
    let set_position = Command::new("busctl")
        .args([
            "--user",
            "call",
            BUS_NAME,
            OBJECT_PATH,
            PLAYER_IFACE,
            "SetPosition",
            "ox",
            "/com/metrolist/track/6451773477395767586351",
            "90450000",
        ])
        .output()
        .expect("busctl must run");
    assert!(
        set_position.status.success(),
        "{}",
        String::from_utf8_lossy(&set_position.stderr)
    );
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"set_position_ms:90450".to_string())));
    assert!(
        wait_until(2_000, || get_property(PLAYER_IFACE, "Position")
            .contains("90450000")),
        "Position must follow SetPosition"
    );

    // A stale track id is ignored, not applied.
    let stale = Command::new("busctl")
        .args([
            "--user",
            "call",
            BUS_NAME,
            OBJECT_PATH,
            PLAYER_IFACE,
            "SetPosition",
            "ox",
            "/com/metrolist/track/deadbeef",
            "1000000",
        ])
        .output()
        .expect("busctl must run");
    assert!(stale.status.success());
    assert!(!control
        .calls()
        .iter()
        .any(|call| call == "set_position_ms:1000"));

    // --- writable properties ----------------------------------------------
    let volume = Command::new("busctl")
        .args([
            "--user",
            "set-property",
            BUS_NAME,
            OBJECT_PATH,
            PLAYER_IFACE,
            "Volume",
            "d",
            "0.25",
        ])
        .output()
        .expect("busctl must run");
    assert!(
        volume.status.success(),
        "{}",
        String::from_utf8_lossy(&volume.stderr)
    );
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"set_volume:0.250".to_string())));

    let loop_status = Command::new("busctl")
        .args([
            "--user",
            "set-property",
            BUS_NAME,
            OBJECT_PATH,
            PLAYER_IFACE,
            "LoopStatus",
            "s",
            "Playlist",
        ])
        .output()
        .expect("busctl must run");
    assert!(loop_status.status.success());
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"set_repeat:all".to_string())));

    let shuffle = Command::new("busctl")
        .args([
            "--user",
            "set-property",
            BUS_NAME,
            OBJECT_PATH,
            PLAYER_IFACE,
            "Shuffle",
            "b",
            "true",
        ])
        .output()
        .expect("busctl must run");
    assert!(shuffle.status.success());
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"set_shuffle:true".to_string())));

    // OpenUri reaches the deep-link handler.
    let open = Command::new("busctl")
        .args([
            "--user",
            "call",
            BUS_NAME,
            OBJECT_PATH,
            PLAYER_IFACE,
            "OpenUri",
            "s",
            "metrolist://album/MPREb_4pL8H9dBh1c",
        ])
        .output()
        .expect("busctl must run");
    assert!(
        open.status.success(),
        "{}",
        String::from_utf8_lossy(&open.stderr)
    );
    assert!(wait_until(2_000, || control
        .calls()
        .iter()
        .any(|call| call.starts_with("open_uri:metrolist://album/"))));

    // --- update() emits PropertiesChanged ---------------------------------
    // Subscribe first, then move the state through `update`.
    let mut monitor = Command::new("busctl")
        .args(["--user", "monitor", BUS_NAME])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("busctl monitor must start");
    std::thread::sleep(std::time::Duration::from_millis(400));

    // A relative seek must raise `Seeked` on the Properties object path.
    let seek = Command::new("busctl")
        .args([
            // `--`: the offset starts with '-', which busctl would parse as an option.
            "--user",
            "call",
            BUS_NAME,
            OBJECT_PATH,
            PLAYER_IFACE,
            "Seek",
            "x",
            "--",
            "-2000000",
        ])
        .output()
        .expect("busctl must run");
    assert!(
        seek.status.success(),
        "{}",
        String::from_utf8_lossy(&seek.stderr)
    );
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"seek_by_ms:-2000".to_string())));

    // First publish after a track change: metadata must be announced.
    let mut snapshot = control.snapshot_now();
    snapshot.status = PlaybackStatus::Playing;
    snapshot.repeat = RepeatMode::One;
    snapshot.shuffle = true;
    snapshot.volume = 0.5;
    snapshot.current = Some(Song {
        id: "otherTrackId".into(),
        title: "Second Track".into(),
        duration: Some(120),
        ..Song::default()
    });
    let second_track = snapshot.current.clone();
    control.set_snapshot(snapshot.clone());
    service.update(&snapshot);
    // Then clearing the track announces an empty metadata map (NoTrack).
    let mut cleared = snapshot;
    cleared.current = None;
    control.set_snapshot(cleared.clone());
    service.update(&cleared);

    std::thread::sleep(std::time::Duration::from_millis(700));
    let _ = monitor.kill();
    let output = monitor.wait_with_output().expect("monitor output");
    let log = String::from_utf8_lossy(&output.stdout).to_string();
    println!("--- busctl monitor excerpt\n{log}");
    assert!(
        log.contains("PropertiesChanged"),
        "update() must emit PropertiesChanged:\n{log}"
    );
    for property in [
        "PlaybackStatus",
        "LoopStatus",
        "Shuffle",
        "Volume",
        "Metadata",
    ] {
        assert!(
            log.contains(property),
            "PropertiesChanged lost {property}:\n{log}"
        );
    }
    assert!(
        log.contains("Member=Seeked"),
        "Seek must emit the Seeked signal:\n{log}"
    );
    assert!(
        !log.contains("STRING \"Position\""),
        "Position must never be announced via PropertiesChanged:\n{log}"
    );

    // The live getters read the control's current state.
    assert!(wait_until(2_000, || get_property(PLAYER_IFACE, "Volume") == "v d 0.5"));
    assert!(wait_until(2_000, || get_property(
        PLAYER_IFACE,
        "LoopStatus"
    )
    .contains("Track")));
    assert!(wait_until(2_000, || get_property(PLAYER_IFACE, "Shuffle")
        == "v b true"));
    let cleared_metadata = get_property(PLAYER_IFACE, "Metadata");
    assert!(
        cleared_metadata.contains("NoTrack"),
        "cleared track metadata must be NoTrack: {cleared_metadata}"
    );

    // The second track was published while it existed, then NoTrack after.
    let log_track = log.contains("Second Track");
    assert!(
        log_track,
        "PropertiesChanged never carried the second track:\n{log}"
    );
    let _ = second_track;

    println!("--- control log\n{:?}", control.calls());
}

// `MprisService::drop` unregisters the bus name; a failed assertion unwinds
// the test thread and drops the service, so no extra cleanup guard is needed.

#[test]
#[ignore = "needs playerctl installed; run with ML_INTEGRATION_LIVE=1"]
fn mpris_service_is_visible_to_playerctl() {
    if !live_enabled() {
        eprintln!("skipping: ML_INTEGRATION_LIVE is not set");
        return;
    }
    let playerctl = Command::new("playerctl")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !playerctl {
        eprintln!("playerctl not installed: skipping (see busctl test for live evidence)");
        return;
    }

    let control = FakeControl::new();
    let _service = MprisService::spawn(control.clone()).expect("register");

    let run = |args: &[&str]| -> String {
        let output = Command::new("playerctl")
            .args(["--player", "metrolist"])
            .args(args)
            .output()
            .expect("playerctl must run");
        assert!(
            output.status.success(),
            "playerctl {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    let listed = Command::new("playerctl")
        .arg("-l")
        .output()
        .expect("playerctl");
    let listed = String::from_utf8_lossy(&listed.stdout).to_string();
    println!("playerctl -l -> {listed}");
    assert!(listed.contains("metrolist"), "service not listed: {listed}");

    assert_eq!(run(&["status"]), "Playing");
    assert_eq!(run(&["metadata", "title"]), "Never Gonna Give You Up");
    assert_eq!(run(&["metadata", "artist"]), "Rick Astley");
    assert_eq!(run(&["metadata", "album"]), "Whenever You Need Somebody");
    assert_eq!(
        run(&["metadata", "mpris:artUrl"]),
        "file:///tmp/metrolist-cover.png"
    );

    run(&["play-pause"]);
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"play_pause".to_string())));
    assert!(wait_until(2_000, || run(&["status"]) == "Paused"));

    run(&["next"]);
    assert!(wait_until(2_000, || control
        .calls()
        .contains(&"next".to_string())));
    println!("playerctl ok; control log {:?}", control.calls());
}
