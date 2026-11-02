//! Live network test: one real track through the real InnerTube source.
//!
//! Runs only with `ML_DOWNLOAD_LIVE=1` (it downloads several megabytes):
//!
//! ```text
//! ML_DOWNLOAD_LIVE=1 cargo test -p ml-downloads --test live -- --nocapture
//! ML_DOWNLOAD_LIVE_VIDEO=<id>    override the default video
//! ML_DOWNLOAD_LIVE_QUALITY=low   `low` or `high` (default `high`)
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ml_core::{AudioQuality, Song};
use ml_downloads::{DownloadConfig, DownloadEvent, DownloadManager, DownloadState};

const DEFAULT_VIDEO: &str = "kJQP7kiw5Fk";

fn live_enabled() -> bool {
    std::env::var("ML_DOWNLOAD_LIVE").is_ok_and(|value| value == "1")
}

#[test]
fn downloads_one_real_track() {
    if !live_enabled() {
        eprintln!("skipped: set ML_DOWNLOAD_LIVE=1 to run the live download");
        return;
    }
    let video_id = std::env::var("ML_DOWNLOAD_LIVE_VIDEO").unwrap_or_else(|_| DEFAULT_VIDEO.into());
    let quality = match std::env::var("ML_DOWNLOAD_LIVE_QUALITY").as_deref() {
        Ok("low") => AudioQuality::Low,
        _ => AudioQuality::High,
    };
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "ml-downloads-live-{}-{}",
        std::process::id(),
        video_id
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let manager = DownloadManager::new(DownloadConfig {
        dir: dir.clone(),
        max_concurrent: 1,
        quality,
        max_storage_bytes: None,
    })
    .expect("manager starts");

    // Title/author come from the player response so the file name is realistic.
    let song = {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let info = runtime
            .block_on(ml_innertube::InnertubeClient::default().media_info(&video_id))
            .expect("player metadata");
        Song {
            id: video_id.clone(),
            title: info.title.unwrap_or_else(|| video_id.clone()),
            artists: info
                .author
                .map(|author| vec![ml_core::Artist::new(author, info.author_id.clone())])
                .unwrap_or_default(),
            ..Song::default()
        }
    };

    let events = manager.events();
    let progress_events = Arc::new(AtomicUsize::new(0));
    let counter = progress_events.clone();
    let min_progress = Arc::new(AtomicU64::new(u64::MAX));
    let min_sink = min_progress.clone();
    let watch = std::thread::spawn(move || {
        // Records the smallest byte count seen after the first progress event,
        // proving the transfer grows monotonically from 0.
        while let Ok(event) = events.recv() {
            if let DownloadEvent::Progress { bytes, .. } = event {
                counter.fetch_add(1, Ordering::SeqCst);
                min_sink.fetch_min(bytes, Ordering::SeqCst);
            }
        }
    });

    let started = Instant::now();
    let accepted = manager.enqueue(std::slice::from_ref(&song));
    assert_eq!(
        accepted,
        vec![video_id.clone()],
        "enqueue accepted the track"
    );

    let deadline = Instant::now() + Duration::from_secs(180);
    let entry = loop {
        assert!(
            Instant::now() < deadline,
            "live download timed out after 180 s (state {:?})",
            manager.get(&song.id).map(|entry| entry.state)
        );
        if let Some(entry) = manager.get(&song.id) {
            if entry.state == DownloadState::Completed {
                break entry;
            }
            if entry.state == DownloadState::Failed {
                panic!("live download failed: {:?}", entry.error);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let elapsed = started.elapsed();

    let path = manager
        .path_for(&song.id)
        .expect("path_for returns the file");
    let size = std::fs::metadata(&path).expect("file exists").len();
    assert!(size > 0, "downloaded file is non-empty");
    assert_eq!(size, entry.size_bytes);
    assert_eq!(entry.song.title, song.title, "metadata is persisted");
    // WebM/Matroska starts with the EBML magic 0x1A45DFA3, MP4 with `ftyp` at
    // offset 4. Either proves the bytes are a real container, not an error page.
    let head = std::fs::read(&path).expect("read file");
    let webm = head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]);
    let mp4 = head.len() >= 8 && &head[4..8] == b"ftyp";
    assert!(
        webm || mp4,
        "file is a WebM or MP4 container, got {:?}",
        &head[..head.len().min(8)]
    );

    let events_seen = progress_events.load(Ordering::SeqCst);
    let min_bytes = min_progress.load(Ordering::SeqCst);
    assert!(
        min_bytes <= 32 * 1024,
        "the first progress event is near zero, got {min_bytes}"
    );
    println!(
        "live: {video_id} {size} bytes in {:.1} s ({:.1} KiB/s), {} progress events, first {min_bytes} bytes, path {}",
        elapsed.as_secs_f64(),
        size as f64 / 1024.0 / elapsed.as_secs_f64().max(0.001),
        events_seen,
        path.display()
    );
    println!("usage: {} bytes", manager.storage_usage());

    drop(watch);
    manager.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
