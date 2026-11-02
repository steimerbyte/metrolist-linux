//! Live probe: `recognize-probe --input file.wav` or `--record [seconds] [device]`.
//!
//! Prints the device list, captures or loads audio, builds the Shazam
//! fingerprint, then asks the live API. `--offline` stops after the fingerprint
//! (prints the signature URI), `--signature-out PATH` dumps the raw bytes.
//!
//! Exit codes: 0 match (or offline success), 1 error, 2 usage, 3 no match.

use std::path::PathBuf;
use std::process::ExitCode;

use ml_recognition::capture::MAX_RECORD_SECONDS;
use ml_recognition::{
    capture, list_input_devices, read_wav, signature_bytes, signature_uri_from_pcm16le,
    CaptureOptions, PcmAudio, RecognitionError, ShazamClient, SAMPLE_RATE,
};

fn usage() -> ! {
    eprintln!(
        "usage:\n  \
         recognize-probe --list-devices\n  \
         recognize-probe --input file.wav [--signature-out sig.bin] [--offline]\n  \
         recognize-probe --record [seconds] [device] [--signature-out sig.bin] [--offline]\n\
         \n  \
         --device NAME   pick an input device (exact name or substring)\n  \
         seconds         recording length, clamped to 0.1..=12 (default 12)"
    );
    std::process::exit(2)
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut record = false;
    let mut list_devices = false;
    let mut signature_out: Option<PathBuf> = None;
    let mut offline = false;
    let mut seconds = MAX_RECORD_SECONDS;
    let mut device: Option<String> = None;

    let mut args = std::env::args().skip(1).peekable();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--list-devices" | "--devices" => list_devices = true,
            "--input" | "-i" => input = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--record" | "-r" => {
                record = true;
                let numeric = args
                    .peek()
                    .map(|value| value.parse::<f32>().is_ok())
                    .unwrap_or(false);
                if numeric {
                    seconds = args.next().unwrap().parse().unwrap();
                }
                let positional_device = args
                    .peek()
                    .map(|value| !value.starts_with('-'))
                    .unwrap_or(false);
                if positional_device && device.is_none() {
                    device = args.next();
                }
            }
            "--device" | "-d" => device = Some(args.next().unwrap_or_else(|| usage())),
            "--signature-out" => {
                signature_out = Some(PathBuf::from(args.next().unwrap_or_else(|| usage())))
            }
            "--offline" => offline = true,
            "--help" | "-h" => usage(),
            other => {
                eprintln!("unknown argument: {other}");
                usage();
            }
        }
    }

    if list_devices {
        match list_input_devices() {
            Ok(devices) if devices.is_empty() => println!("no input devices found"),
            Ok(devices) => {
                for (index, device) in devices.iter().enumerate() {
                    let default = if device.is_default { " [default]" } else { "" };
                    println!(
                        "{index}: {}{default} ({} ch, {} Hz)",
                        device.name, device.channels, device.sample_rate
                    );
                }
            }
            Err(error) => {
                eprintln!("device listing failed: {error}");
                return ExitCode::from(1);
            }
        }
        return ExitCode::SUCCESS;
    }

    if input.is_some() && record {
        eprintln!("choose either --input or --record, not both");
        usage();
    }

    let audio: PcmAudio = match &input {
        Some(path) => match read_wav(path) {
            Ok(audio) => audio,
            Err(error) => {
                eprintln!("cannot read {}: {error}", path.display());
                return ExitCode::from(1);
            }
        },
        None if record => {
            let options = CaptureOptions { seconds, device };
            match capture(&options) {
                Ok(audio) => audio,
                Err(error) => {
                    eprintln!("capture failed: {error}");
                    return ExitCode::from(1);
                }
            }
        }
        None => usage(),
    };

    let frames = audio.samples.len() / usize::from(audio.channels.max(1));
    println!(
        "audio: {} Hz, {} ch, {} frames, {} ms",
        audio.sample_rate,
        audio.channels,
        frames,
        audio.duration_ms()
    );

    let pcm16 = audio.to_16k_mono_pcm16();
    let fingerprint_ms = (pcm16.len() as u64 / 2) * 1000 / u64::from(SAMPLE_RATE);
    println!(
        "fingerprint input: {} samples @ {SAMPLE_RATE} Hz, {fingerprint_ms} ms",
        pcm16.len() / 2
    );

    let signature = match signature_uri_from_pcm16le(&pcm16) {
        Ok(signature) => signature,
        Err(error) => {
            eprintln!("fingerprint failed: {error}");
            return ExitCode::from(1);
        }
    };
    println!(
        "signature: {} bytes, URI {} bytes",
        (signature.len() - "data:audio/vnd.shazam.sig;base64,".len()) * 3 / 4,
        signature.len()
    );

    if let Some(path) = &signature_out {
        let raw = signature_bytes(&ml_recognition::pcm16le_to_samples(&pcm16));
        if let Err(error) = std::fs::write(path, &raw) {
            eprintln!("cannot write {}: {error}", path.display());
            return ExitCode::from(1);
        }
        println!(
            "wrote {} raw signature bytes to {}",
            raw.len(),
            path.display()
        );
    }

    if offline {
        println!("{signature}");
        return ExitCode::SUCCESS;
    }

    match ShazamClient::new()
        .recognize(&signature, fingerprint_ms)
        .await
    {
        Ok(result) => {
            println!("MATCH: {} — {}", result.title, result.artist);
            println!("track key: {}", result.track_key);
            if let Some(album) = &result.album {
                println!("album: {album}");
            }
            if let Some(artwork) = result
                .artwork_hq_url
                .as_ref()
                .or(result.artwork_url.as_ref())
            {
                println!("artwork: {artwork}");
            }
            if let Some(genre) = &result.genre {
                println!("genre: {genre}");
            }
            if let Some(released) = &result.release_date {
                println!("released: {released}");
            }
            if let Some(label) = &result.label {
                println!("label: {label}");
            }
            if let Some(isrc) = &result.isrc {
                println!("isrc: {isrc}");
            }
            if let Some(video_id) = &result.youtube_video_id {
                println!("youtube: https://www.youtube.com/watch?v={video_id}");
            }
            if let Some(url) = &result.shazam_url {
                println!("shazam: {url}");
            }
            if let Some(url) = &result.apple_music_url {
                println!("apple music: {url}");
            }
            if let Some(url) = &result.spotify_url {
                println!("spotify: {url}");
            }
            if let Some(lyrics) = &result.lyrics {
                println!("lyrics: {} lines", lyrics.len());
            }
            ExitCode::SUCCESS
        }
        Err(RecognitionError::NoMatch) => {
            eprintln!("no match found");
            ExitCode::from(3)
        }
        Err(error) => {
            eprintln!("recognition failed: {error}");
            ExitCode::from(1)
        }
    }
}
