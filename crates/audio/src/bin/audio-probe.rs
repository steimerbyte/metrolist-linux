//! Live smoke test: `audio-probe <url-or-file> [seek_ms] [seconds]`.
//!
//! Starts the real mpv engine, plays the source, optionally seeks after
//! 1.5 s, prints coalesced snapshots/positions, and exits after `seconds`
//! (default 10). Set `ML_AUDIO_DEVICE` to switch the output device before
//! loading; set `METROLIST_TEST_MUTE=1` to run muted (`--mute=yes`, test client
//! id) for silent probes.

use std::time::{Duration, Instant};

use ml_audio::{AudioCommand, AudioConfig, AudioEvent, MpvEngine};

fn main() {
    let mut args = std::env::args().skip(1);
    let source = match args.next() {
        Some(source) => source,
        None => {
            eprintln!("usage: audio-probe <url-or-file> [seek_ms] [seconds]");
            std::process::exit(2);
        }
    };
    let seek_ms: Option<u64> = match args.next() {
        Some(value) => match value.parse() {
            Ok(value) => Some(value),
            Err(_) => {
                eprintln!("seek_ms must be an integer: {value}");
                std::process::exit(2);
            }
        },
        None => None,
    };
    let seconds: u64 = match args.next() {
        Some(value) => match value.parse() {
            Ok(value) => value,
            Err(_) => {
                eprintln!("seconds must be an integer: {value}");
                std::process::exit(2);
            }
        },
        None => 10,
    };

    let config = AudioConfig::default();
    match ml_audio::list_devices(&config) {
        Ok(devices) => {
            let defaults: Vec<_> = devices.iter().filter(|device| device.is_default).collect();
            println!(
                "devices: {} total, defaults: {}",
                devices.len(),
                defaults
                    .iter()
                    .map(|device| device.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Err(error) => println!("device listing failed: {error}"),
    }

    let mut engine = match MpvEngine::spawn(config) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("failed to start mpv engine: {error}");
            std::process::exit(1);
        }
    };
    if let Ok(device) = std::env::var("ML_AUDIO_DEVICE") {
        println!("switching output device to {device}");
        send(&engine, AudioCommand::SetDevice(device));
    }
    send(
        &engine,
        AudioCommand::LoadUrl {
            url: source.clone(),
            start_ms: None,
        },
    );
    println!("loading {source}");

    let started = Instant::now();
    let mut seek_done = seek_ms.is_none();
    while started.elapsed() < Duration::from_secs(seconds) {
        if !seek_done && started.elapsed() >= Duration::from_millis(1500) {
            let target = seek_ms.unwrap_or_default();
            send(&engine, AudioCommand::SeekAbsolute(target));
            println!("seek -> {target} ms");
            seek_done = true;
        }
        while let Some(event) = engine.try_recv() {
            match event {
                AudioEvent::Snapshot(snapshot) => println!(
                    "[{} ms] status={:?} pos={} dur={} vol={:.2} muted={} buffering={}",
                    started.elapsed().as_millis(),
                    snapshot.status,
                    snapshot.position_ms,
                    snapshot.duration_ms,
                    snapshot.volume,
                    snapshot.muted,
                    snapshot.buffering
                ),
                AudioEvent::Position {
                    position_ms,
                    duration_ms,
                } => println!(
                    "[{} ms] position {position_ms} / {duration_ms}",
                    started.elapsed().as_millis()
                ),
                AudioEvent::Buffering(buffering) => println!("buffering {buffering}"),
                AudioEvent::EndFile => println!("end-file"),
                AudioEvent::Error(error) => println!("error: {error}"),
                AudioEvent::Devices(devices) => println!("devices: {}", devices.len()),
                AudioEvent::Path(path) => println!("path: {path}"),
                AudioEvent::AudioParams(params) => println!(
                    "audio-params: {:?} {} Hz {} ch",
                    params.format,
                    params.sample_rate.unwrap_or(0),
                    params.channels.unwrap_or(0)
                ),
                AudioEvent::Metadata(metadata) => println!(
                    "metadata: {} / {}",
                    metadata.title.unwrap_or_default(),
                    metadata.artist.unwrap_or_default()
                ),
                AudioEvent::DeviceChanged { id, fallback_from } => match fallback_from {
                    Some(requested) => {
                        println!("device: {requested} unavailable, fell back to {id}")
                    }
                    None => println!("device: {id}"),
                },
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    engine.shutdown();
    println!("done");
}

fn send(engine: &MpvEngine, command: AudioCommand) {
    if let Err(error) = engine.send(command) {
        eprintln!("engine stopped: {error}");
        std::process::exit(1);
    }
}
