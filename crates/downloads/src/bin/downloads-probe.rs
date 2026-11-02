//! Live probe: `downloads-probe <videoId> [--dir PATH] [--quality low|high|auto]`.
//!
//! Runs one real download through `DownloadManager` + `InnertubeClient`
//! (VISIONOS ranged 206 requests), prints every event, and exits non-zero unless
//! the file exists with a non-zero size.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ml_core::{AudioQuality, Song};
use ml_downloads::{DownloadConfig, DownloadEvent, DownloadManager};

fn usage() -> ! {
    eprintln!("usage: downloads-probe <videoId> [--dir PATH] [--quality low|high|auto]");
    std::process::exit(2)
}

fn parse_quality(value: &str) -> AudioQuality {
    match value {
        "low" => AudioQuality::Low,
        "high" => AudioQuality::High,
        "auto" => AudioQuality::Auto,
        other => {
            eprintln!("unknown quality: {other} (expected low|high|auto)");
            std::process::exit(2)
        }
    }
}

fn metadata(video_id: &str) -> Song {
    let mut song = Song {
        id: video_id.to_string(),
        title: video_id.to_string(),
        ..Song::default()
    };
    // The player response carries the title/author; a failure here only means a
    // less pretty file name, never a failed download.
    let client = ml_innertube::InnertubeClient::default();
    if let Ok(info) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
        .and_then(|runtime| {
            runtime
                .block_on(client.media_info(video_id))
                .map_err(|error| error.to_string())
        })
    {
        if let Some(title) = info.title.filter(|title| !title.is_empty()) {
            song.title = title;
        }
        if let Some(author) = info.author.filter(|author| !author.is_empty()) {
            song.artists = vec![ml_core::Artist::new(author, info.author_id.clone())];
        }
    }
    song
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let video_id = match args.next() {
        Some(id) if !id.starts_with("--") => id,
        _ => usage(),
    };
    let mut dir: Option<PathBuf> = None;
    let mut quality = AudioQuality::High;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from),
            "--quality" => {
                quality = args
                    .next()
                    .map(|value| parse_quality(&value))
                    .unwrap_or_else(|| usage())
            }
            other => {
                eprintln!("unknown flag: {other}");
                usage()
            }
        }
    }
    let dir = dir.unwrap_or_else(|| {
        env::temp_dir().join(format!("ml-downloads-probe-{}", std::process::id()))
    });

    let song = metadata(&video_id);
    println!("dir: {}", dir.display());
    println!("quality: {quality:?}");
    println!("video: {video_id}");
    println!(
        "song: {} — {} ({})",
        song.title,
        song.artist_names(),
        song.id
    );

    let manager = match DownloadManager::new(DownloadConfig {
        dir: dir.clone(),
        max_concurrent: 1,
        quality,
        max_storage_bytes: None,
    }) {
        Ok(manager) => manager,
        Err(error) => {
            eprintln!("cannot start download manager: {error}");
            return ExitCode::FAILURE;
        }
    };
    let events = manager.events();
    if manager.enqueue(&[song]).is_empty() {
        eprintln!("enqueue was refused (already complete?)");
        if let Some(path) = manager.path_for(&video_id) {
            println!("already downloaded: {}", path.display());
        }
        return ExitCode::FAILURE;
    }

    let deadline = Instant::now() + Duration::from_secs(180);
    let mut final_path: Option<PathBuf> = None;
    let mut failure: Option<String> = None;
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(500)) {
            Ok(DownloadEvent::Queued(id)) => println!("event: queued {id}"),
            Ok(DownloadEvent::Progress { id, bytes, total }) => println!(
                "event: progress {id} {bytes}/{total} bytes ({}%)",
                ml_downloads::progress_percent(bytes, total)
            ),
            Ok(DownloadEvent::Completed { id, path }) => {
                let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                println!("event: completed {id} -> {} ({size} bytes)", path.display());
                final_path = Some(path);
                break;
            }
            Ok(DownloadEvent::Failed { id, error }) => {
                println!("event: failed {id}: {error}");
                failure = Some(format!("{id}: {error}"));
                break;
            }
            Ok(DownloadEvent::Cancelled(id)) => println!("event: cancelled {id}"),
            Ok(DownloadEvent::Removed(id)) => println!("event: removed {id}"),
            Ok(DownloadEvent::StorageChanged { used, limit }) => {
                println!("event: storage {used} bytes used, limit {limit:?}")
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    let Some(path) = final_path else {
        println!(
            "FAILED: {}",
            failure.unwrap_or_else(|| "timed out after 180 s".to_string())
        );
        return ExitCode::FAILURE;
    };
    let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let verified = manager.path_for(&video_id);
    println!("final: {size} bytes at {}", path.display());
    println!("path_for: {verified:?}");
    println!(
        "usage: {} bytes; manifest: {}",
        manager.storage_usage(),
        dir.join("manifest.json").display()
    );
    if size == 0 || verified.is_none() {
        println!("FAILED: size 0 or path_for rejected the file");
        return ExitCode::FAILURE;
    }
    println!("OK");
    ExitCode::SUCCESS
}
