//! Live audio tests. They start a real mpv process and need a working audio
//! server (PipeWire/PulseAudio/ALSA), so they are ignored by default:
//!
//! `cargo test -p ml-audio --test live -- --ignored --nocapture`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ml_audio::{
    AudioCommand, AudioConfig, AudioEvent, MpvEngine, PlaybackController, SleepTimer,
    StreamResolver,
};
use ml_core::{PlaybackStatus, Queue, QueueSource, Song};

/// 16-bit mono 8 kHz WAV built from `(frequency_hz, milliseconds)` segments;
/// frequency 0 is silence. Small enough for fast fixtures, long enough for
/// position/fade assertions.
fn write_wav(path: &Path, segments: &[(f32, u32)]) {
    const SAMPLE_RATE: u32 = 8_000;
    let frames: u32 = segments
        .iter()
        .map(|(_, millis)| SAMPLE_RATE * millis / 1000)
        .sum();
    let data_len = frames * 2;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for (frequency, millis) in segments {
        let count = SAMPLE_RATE * millis / 1000;
        for frame in 0..count {
            let seconds = frame as f32 / SAMPLE_RATE as f32;
            let sample = if *frequency <= 0.0 {
                0
            } else {
                (f32::sin(std::f32::consts::TAU * frequency * seconds) * 12_000.0) as i16
            };
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    std::fs::write(path, bytes).expect("write wav fixture");
}

fn fixture(name: &str) -> PathBuf {
    fixture_secs(name, 20)
}

fn fixture_secs(name: &str, seconds: u32) -> PathBuf {
    fixture_segments(name, &[(0.0, seconds * 1000)])
}

fn fixture_segments(name: &str, segments: &[(f32, u32)]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ml-audio-live-{}-{name}", std::process::id()));
    write_wav(&path, segments);
    path
}

/// Resolver over fixture paths keyed by queue id.
struct Files(HashMap<String, String>);

impl StreamResolver for Files {
    fn resolve(&self, video_id: &str) -> Option<String> {
        self.0.get(video_id).cloned()
    }
}

fn files(entries: &[(&str, &Path)]) -> Arc<Files> {
    Arc::new(Files(
        entries
            .iter()
            .map(|(id, path)| (id.to_string(), path.to_string_lossy().into_owned()))
            .collect(),
    ))
}

fn song(id: &str, seconds: u32) -> Song {
    Song {
        id: id.to_string(),
        title: id.to_string(),
        duration: Some(seconds),
        ..Song::default()
    }
}

/// `(streams, muted streams)` for the muted test client id as seen by pactl.
/// Native `--ao=pipewire` streams still appear as sink inputs on this host.
fn sink_inputs() -> (usize, usize) {
    let Ok(output) = std::process::Command::new("pactl")
        .args(["list", "sink-inputs"])
        .output()
    else {
        return (0, 0);
    };
    if !output.status.success() {
        return (0, 0);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut total = 0;
    let mut muted = 0;
    for block in text.split("Sink Input #").skip(1) {
        if block.contains(r#"application.name = "Metrolist Desktop""#) {
            total += 1;
            if block.lines().any(|line| line.trim() == "Mute: yes") {
                muted += 1;
            }
        }
    }
    (total, muted)
}

fn sink_input_count() -> usize {
    sink_inputs().0
}

fn wait_for_playing_muted(engine: &MpvEngine, timeout: Duration) -> bool {
    wait_for(
        engine,
        |event| {
            matches!(
                event,
                AudioEvent::Snapshot(snapshot)
                    if snapshot.status == PlaybackStatus::Playing && snapshot.muted
            )
        },
        timeout,
    )
}

/// Every live test is silent: test mode passes `--mute=yes` and the legacy
/// client id, whose host stream-restore entry is muted too.
fn engine() -> MpvEngine {
    MpvEngine::spawn(AudioConfig {
        muted: Some(true),
        ..AudioConfig::default()
    })
    .expect("mpv with a working audio output")
}

/// True while the process exists; a reaped child leaves no `/proc` entry.
fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

fn wait_for(engine: &MpvEngine, accepted: impl Fn(&AudioEvent) -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        while let Some(event) = engine.try_recv() {
            if let AudioEvent::Error(error) = &event {
                panic!("engine error: {error}");
            }
            if accepted(&event) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn position_in(engine: &MpvEngine, range: std::ops::Range<u64>, timeout: Duration) -> Option<u64> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        while let Some(event) = engine.try_recv() {
            if let AudioEvent::Error(error) = &event {
                panic!("engine error: {error}");
            }
            if let AudioEvent::Position { position_ms, .. } = event {
                if range.contains(&position_ms) {
                    return Some(position_ms);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn shutdown_reaps_both_engines_and_removes_their_sockets() {
    let mut main = engine();
    let mut crossfade = engine();
    let (main_pid, crossfade_pid) = (main.pid(), crossfade.pid());
    let (main_socket, crossfade_socket) = (
        main.socket_path().to_path_buf(),
        crossfade.socket_path().to_path_buf(),
    );
    assert!(process_alive(main_pid) && process_alive(crossfade_pid));
    assert!(main_socket.exists() && crossfade_socket.exists());

    main.shutdown();
    crossfade.shutdown();

    assert!(!process_alive(main_pid), "main mpv child survived shutdown");
    assert!(
        !process_alive(crossfade_pid),
        "crossfade mpv child survived shutdown"
    );
    assert!(
        !main_socket.exists() && !crossfade_socket.exists(),
        "IPC sockets survived shutdown"
    );
    // Idempotent: Drop runs a second shutdown after this call.
    main.shutdown();
    assert!(!process_alive(main_pid));
    assert!(!main_socket.exists());
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn drop_reaps_the_mpv_child_and_removes_the_socket() {
    let (pid, socket) = {
        let engine = engine();
        let (pid, socket) = (engine.pid(), engine.socket_path().to_path_buf());
        assert!(process_alive(pid) && socket.exists());
        (pid, socket)
    };
    assert!(!process_alive(pid), "mpv child survived Drop");
    assert!(!socket.exists(), "IPC socket survived Drop");
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn local_file_plays_and_seeks() {
    let path = fixture("seek.wav");
    let mut engine = engine();
    engine
        .send(AudioCommand::LoadUrl {
            url: path.to_string_lossy().into_owned(),
            start_ms: None,
        })
        .unwrap();

    assert!(
        wait_for(
            &engine,
            |event| matches!(
                event,
                AudioEvent::Snapshot(snapshot) if snapshot.status == PlaybackStatus::Playing
            ),
            Duration::from_secs(10)
        ),
        "playback never reached Playing"
    );
    assert!(
        wait_for(
            &engine,
            |event| matches!(
                event,
                AudioEvent::Position { position_ms, .. } if *position_ms > 500
            ),
            Duration::from_secs(5)
        ),
        "position never advanced"
    );

    engine.send(AudioCommand::SeekAbsolute(10_000)).unwrap();
    let landed = position_in(&engine, 9_000..12_000, Duration::from_secs(5));
    engine.shutdown();
    let _ = std::fs::remove_file(path);
    assert!(landed.is_some(), "seek did not land near 10 s: {landed:?}");
}

#[test]
#[ignore = "requires mpv, a working audio output and network access"]
fn remote_https_url_plays() {
    let mut engine = engine();
    engine
        .send(AudioCommand::LoadUrl {
            url: "https://www.soundhelix.com/examples/mp3/SoundHelix-Song-1.mp3".to_string(),
            start_ms: None,
        })
        .unwrap();
    let playing = wait_for(
        &engine,
        |event| {
            matches!(
                event,
                AudioEvent::Snapshot(snapshot)
                    if snapshot.status == PlaybackStatus::Playing && snapshot.duration_ms > 0
            )
        },
        Duration::from_secs(30),
    );
    let advanced = playing
        && wait_for(
            &engine,
            |event| {
                matches!(
                    event,
                    AudioEvent::Position { position_ms, .. } if *position_ms > 500
                )
            },
            Duration::from_secs(10),
        );
    engine.shutdown();
    assert!(advanced, "remote track did not play");
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn load_starts_at_requested_position() {
    let path = fixture_secs("start.wav", 20);
    let mut engine = engine();
    engine
        .send(AudioCommand::LoadUrl {
            url: path.to_string_lossy().into_owned(),
            start_ms: Some(10_000),
        })
        .unwrap();
    let landed = position_in(&engine, 9_000..12_000, Duration::from_secs(10));
    engine.shutdown();
    let _ = std::fs::remove_file(path);
    assert!(
        landed.is_some(),
        "load did not start at the requested position: {landed:?}"
    );
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn enqueue_next_advances_gaplessly() {
    let first = fixture_secs("gapless-a.wav", 2);
    let second = fixture_secs("gapless-b.wav", 2);
    let second_url = second.to_string_lossy().into_owned();
    let mut engine = engine();
    engine
        .send(AudioCommand::LoadUrl {
            url: first.to_string_lossy().into_owned(),
            start_ms: None,
        })
        .unwrap();
    assert!(wait_for(
        &engine,
        |event| matches!(
            event,
            AudioEvent::Snapshot(snapshot) if snapshot.status == PlaybackStatus::Playing
        ),
        Duration::from_secs(10)
    ));
    engine
        .send(AudioCommand::EnqueueNext {
            url: second_url.clone(),
        })
        .unwrap();

    // mpv emits end-file when the first entry finishes and continues with the
    // appended entry; the engine reports the new path without a reload.
    let ended = wait_for(
        &engine,
        |event| matches!(event, AudioEvent::EndFile),
        Duration::from_secs(10),
    );
    let switched = wait_for(
        &engine,
        |event| matches!(event, AudioEvent::Path(path) if *path == second_url),
        Duration::from_secs(5),
    );
    let advanced = position_in(&engine, 1..2_000, Duration::from_secs(5));
    engine.shutdown();
    let _ = std::fs::remove_file(first);
    let _ = std::fs::remove_file(second);
    assert!(ended, "first file never ended");
    assert!(switched, "mpv did not advance to the appended file");
    assert!(advanced.is_some(), "appended file did not play");
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn eq_gain_and_device_switch_are_accepted_while_playing() {
    let path = fixture("eq.wav");
    let mut engine = engine();
    engine
        .send(AudioCommand::LoadUrl {
            url: path.to_string_lossy().into_owned(),
            start_ms: None,
        })
        .unwrap();
    assert!(
        wait_for(
            &engine,
            |event| matches!(
                event,
                AudioEvent::Snapshot(snapshot) if snapshot.status == PlaybackStatus::Playing
            ),
            Duration::from_secs(10)
        ),
        "playback never reached Playing"
    );

    engine
        .send(AudioCommand::SetEqBands(vec![
            3.0, 0.0, -2.0, 0.0, 1.5, 0.0, 0.0, -1.0, 0.0, 2.0,
        ]))
        .unwrap();
    engine.send(AudioCommand::SetGainDb(-3.0)).unwrap();
    engine.send(AudioCommand::SetVolume(0.5)).unwrap();
    engine.send(AudioCommand::SetSpeed(1.25)).unwrap();
    engine.send(AudioCommand::SetMuted(true)).unwrap();
    engine.send(AudioCommand::SetMuted(false)).unwrap();

    let devices = engine.list_devices().expect("device listing");
    assert!(!devices.is_empty());
    let target = devices
        .iter()
        .find(|device| device.id.starts_with("pipewire/"))
        .expect("a pipewire sink");
    engine
        .send(AudioCommand::SetDevice(target.id.clone()))
        .unwrap();

    let position = position_in(&engine, 1..60_000, Duration::from_secs(5));
    engine.shutdown();
    let _ = std::fs::remove_file(path);
    assert!(position.is_some(), "audio stopped after EQ/device changes");
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn skip_silence_drops_mid_stream_gaps() {
    let path = fixture_segments(
        "skip-silence.wav",
        &[(440.0, 2_000), (0.0, 3_000), (660.0, 2_000)],
    );
    let url = path.to_string_lossy().into_owned();
    let mut engine = engine();

    // Baseline: the 7 s timeline plays in ~7 s.
    engine
        .send(AudioCommand::LoadUrl {
            url: url.clone(),
            start_ms: None,
        })
        .unwrap();
    assert!(wait_for(
        &engine,
        |event| matches!(
            event,
            AudioEvent::Snapshot(snapshot) if snapshot.status == PlaybackStatus::Playing
        ),
        Duration::from_secs(10)
    ));
    let plain_start = Instant::now();
    assert!(
        wait_for(
            &engine,
            |event| matches!(event, AudioEvent::EndFile),
            Duration::from_secs(15)
        ),
        "baseline playback never ended"
    );
    let plain = plain_start.elapsed();

    // With silenceremove the 3 s gap is dropped; the file must end sooner.
    engine.send(AudioCommand::SetSkipSilence(true)).unwrap();
    engine
        .send(AudioCommand::LoadUrl {
            url,
            start_ms: None,
        })
        .unwrap();
    assert!(wait_for(
        &engine,
        |event| matches!(
            event,
            AudioEvent::Snapshot(snapshot) if snapshot.status == PlaybackStatus::Playing
        ),
        Duration::from_secs(10)
    ));
    let skipped_start = Instant::now();
    assert!(
        wait_for(
            &engine,
            |event| matches!(event, AudioEvent::EndFile),
            Duration::from_secs(15)
        ),
        "skip-silence playback never ended"
    );
    let skipped = skipped_start.elapsed();
    engine.shutdown();
    let _ = std::fs::remove_file(path);

    eprintln!("plain={plain:?} skipped={skipped:?}");
    assert!(
        plain >= Duration::from_secs(6),
        "baseline was shorter than the 7 s file: {plain:?}"
    );
    assert!(
        skipped <= Duration::from_millis(5_500),
        "silence was not removed: {skipped:?} (plain {plain:?})"
    );
    assert!(
        plain.saturating_sub(skipped) >= Duration::from_secs(1),
        "expected at least 1 s to be skipped: plain {plain:?} skipped {skipped:?}"
    );
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn normalization_gain_keeps_playing_with_a_live_sink() {
    let path = fixture_segments("normalize.wav", &[(440.0, 8_000)]);
    let url = path.to_string_lossy().into_owned();
    let mut engine = engine();
    engine
        .send(AudioCommand::LoadUrl {
            url,
            start_ms: None,
        })
        .unwrap();
    assert!(
        wait_for_playing_muted(&engine, Duration::from_secs(10)),
        "engine never reached muted playback"
    );
    engine.send(AudioCommand::SetGainDb(-6.0)).unwrap();
    engine.send(AudioCommand::SetNormalization(true)).unwrap();
    engine
        .send(AudioCommand::SetEqBands(vec![2.0; 10]))
        .unwrap();
    let advanced = position_in(&engine, 1_000..8_000, Duration::from_secs(5));
    let (sinks, muted_sinks) = sink_inputs();
    eprintln!("sink inputs: {sinks}, muted: {muted_sinks}");
    engine.shutdown();
    let _ = std::fs::remove_file(path);

    assert!(
        advanced.is_some(),
        "audio stopped after the normalization gain was applied"
    );
    assert!(
        sinks >= 1,
        "no 'Metrolist Desktop' sink input while normalization was active"
    );
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn sleep_fade_out_pauses_and_restores_volume() {
    let path = fixture_secs("sleep-fade.wav", 20);
    let mut engine = engine();
    let files = files(&[("sleep", &path)]);
    let mut controller = PlaybackController::new(
        engine.command_sender().clone(),
        engine.event_receiver().clone(),
        files,
    );
    controller.play_queue(Queue::new(vec![song("sleep", 20)], QueueSource::List), 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && controller.snapshot().status != PlaybackStatus::Playing {
        controller.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        controller.snapshot().status,
        PlaybackStatus::Playing,
        "playback never started"
    );
    assert!(
        controller.snapshot().muted,
        "sleep-fade engine was not muted"
    );

    let started = Instant::now();
    controller.set_sleep_timer(SleepTimer::FadeOut(Duration::from_millis(500)));
    while controller.snapshot().status != PlaybackStatus::Paused
        && started.elapsed() < Duration::from_secs(5)
    {
        controller.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    let elapsed = started.elapsed();
    engine.shutdown();
    let _ = std::fs::remove_file(path);

    eprintln!("sleep fade paused after {elapsed:?}");
    assert_eq!(controller.snapshot().status, PlaybackStatus::Paused);
    assert!(
        elapsed >= Duration::from_millis(400),
        "pause before the fade finished: {elapsed:?}"
    );
    assert!(
        elapsed <= Duration::from_secs(3),
        "fade took too long: {elapsed:?}"
    );
    assert_eq!(controller.sleep_timer(), SleepTimer::Off);
    assert_eq!(controller.snapshot().volume, 1.0);
}

#[test]
#[ignore = "requires mpv, a working audio output and pactl"]
fn crossfade_overlaps_two_mpv_engines() {
    let first = fixture_segments("xfade-a.wav", &[(440.0, 8_000)]);
    let second = fixture_segments("xfade-b.wav", &[(660.0, 8_000)]);
    let mut primary = engine();
    let mut secondary = engine();
    let files = files(&[("a", &first), ("b", &second)]);
    let mut controller = PlaybackController::new(
        primary.command_sender().clone(),
        primary.event_receiver().clone(),
        files,
    );
    assert!(controller.attach_crossfade_engine(
        secondary.command_sender().clone(),
        secondary.event_receiver().clone()
    ));
    controller.set_crossfade(Duration::from_secs(5));
    controller.play_queue(
        Queue::new(vec![song("a", 8), song("b", 8)], QueueSource::List),
        0,
    );

    let deadline = Instant::now() + Duration::from_secs(22);
    while Instant::now() < deadline && !controller.snapshot().muted {
        controller.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        controller.snapshot().muted,
        "crossfade engines were not muted"
    );
    let mut overlap_sinks = 0;
    while Instant::now() < deadline {
        controller.poll();
        if controller.is_crossfading() {
            // Give the incoming engine's stream a moment to register.
            let count_deadline = Instant::now() + Duration::from_millis(600);
            while Instant::now() < count_deadline {
                overlap_sinks = overlap_sinks.max(sink_input_count());
                if overlap_sinks >= 2 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        controller.is_crossfading(),
        "crossfade never started (status {:?})",
        controller.snapshot().status
    );
    let (sinks, muted_sinks) = sink_inputs();
    eprintln!(
        "sink inputs during crossfade: {sinks} total, {muted_sinks} muted (peak {overlap_sinks})"
    );
    assert!(
        overlap_sinks >= 2,
        "expected two 'Metrolist Desktop' sink inputs during crossfade, saw {overlap_sinks}"
    );

    while controller.is_crossfading() && Instant::now() < deadline {
        controller.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!controller.is_crossfading(), "crossfade never finished");
    assert_eq!(controller.queue().position(), 1);
    assert_eq!(
        controller
            .snapshot()
            .current
            .as_ref()
            .map(|song| song.id.as_str()),
        Some("b")
    );
    let advanced = Instant::now() + Duration::from_secs(4);
    let mut position = 0;
    while Instant::now() < advanced && position == 0 {
        controller.poll();
        position = controller.snapshot().position_ms;
        std::thread::sleep(Duration::from_millis(20));
    }
    primary.shutdown();
    secondary.shutdown();
    let _ = std::fs::remove_file(first);
    let _ = std::fs::remove_file(second);
    assert!(position > 0, "incoming track did not play after the swap");
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn device_switch_and_unknown_device_fallback() {
    let path = fixture_secs("device.wav", 20);
    let mut engine = engine();
    engine
        .send(AudioCommand::LoadUrl {
            url: path.to_string_lossy().into_owned(),
            start_ms: None,
        })
        .unwrap();
    assert!(wait_for(
        &engine,
        |event| matches!(
            event,
            AudioEvent::Snapshot(snapshot) if snapshot.status == PlaybackStatus::Playing
        ),
        Duration::from_secs(10)
    ));

    let devices = engine.list_devices().expect("device listing");
    let target = devices
        .iter()
        .find(|device| device.id.starts_with("pipewire/") && !device.id.contains("nope"))
        .map(|device| device.id.clone())
        .or_else(|| {
            devices
                .iter()
                .find(|device| device.id == "pipewire")
                .map(|device| device.id.clone())
        })
        .expect("a usable pipewire device");

    engine.set_device(Some(&target)).unwrap();
    assert!(
        wait_for(
            &engine,
            |event| matches!(
                event,
                AudioEvent::DeviceChanged { id, fallback_from: None } if id == &target
            ),
            Duration::from_secs(5)
        ),
        "valid device switch was not reported"
    );
    assert_eq!(engine.device().as_deref(), Some(target.as_str()));

    let bogus = "pipewire/definitely-not-a-device";
    engine.set_device(Some(bogus)).unwrap();
    assert!(
        wait_for(
            &engine,
            |event| matches!(
                event,
                AudioEvent::DeviceChanged { id, fallback_from: Some(requested) }
                    if id == "auto" && requested.as_str() == bogus
            ),
            Duration::from_secs(5)
        ),
        "unknown device was not reported as a fallback"
    );
    assert_eq!(engine.device(), None);
    let advanced = position_in(&engine, 1..20_000, Duration::from_secs(5));
    engine.shutdown();
    assert!(
        advanced.is_some(),
        "playback stopped after the device fallback"
    );

    // A persisted device in the config is used on the next spawn.
    let mut engine = MpvEngine::spawn(AudioConfig {
        device: Some(target.clone()),
        muted: Some(true),
        ..AudioConfig::default()
    })
    .expect("mpv with a persisted device");
    assert_eq!(engine.device().as_deref(), Some(target.as_str()));
    engine.shutdown();
    let _ = std::fs::remove_file(path);
}

#[test]
#[ignore = "requires mpv and a working audio output"]
fn speed_is_pitch_corrected_and_faster_than_realtime() {
    let path = fixture_secs("speed.wav", 30);
    let mut engine = engine();
    engine
        .send(AudioCommand::LoadUrl {
            url: path.to_string_lossy().into_owned(),
            start_ms: None,
        })
        .unwrap();
    assert!(wait_for(
        &engine,
        |event| matches!(
            event,
            AudioEvent::Snapshot(snapshot) if snapshot.status == PlaybackStatus::Playing
        ),
        Duration::from_secs(10)
    ));
    engine.send(AudioCommand::SetSpeed(1.5)).unwrap();
    std::thread::sleep(Duration::from_millis(400));

    let mut samples: Vec<(Instant, u64)> = Vec::new();
    let window = Instant::now() + Duration::from_secs(2);
    while Instant::now() < window {
        while let Some(event) = engine.try_recv() {
            if let AudioEvent::Position { position_ms, .. } = event {
                samples.push((Instant::now(), position_ms));
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    engine.shutdown();
    let _ = std::fs::remove_file(path);

    assert!(samples.len() >= 5, "not enough position samples");
    let (first_at, first_pos) = samples.first().copied().unwrap();
    let (last_at, last_pos) = samples.last().copied().unwrap();
    let speed = (last_pos - first_pos) as f64 / 1000.0 / (last_at - first_at).as_secs_f64();
    eprintln!("measured media speed {speed:.2}x (requested 1.5x)");
    assert!(
        (1.15..1.9).contains(&speed),
        "position advanced at {speed:.2}x, not ~1.5x"
    );
}
