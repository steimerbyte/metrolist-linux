//! Manager behaviour with a fake [`AudioSource`] — no network, no mpv.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ml_core::{AudioQuality, MlError, Song, StreamInfo};
use ml_downloads::{
    BoxFuture, DownloadConfig, DownloadError, DownloadEvent, DownloadManager, DownloadState,
    ProgressFn,
};
use parking_lot::Mutex;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn manager_is_send_and_sync() {
    assert_send_sync::<DownloadManager>();
}

// ---------------------------------------------------------------- fake source

/// Scripted behaviour for one fake transfer.
#[derive(Clone)]
struct Behavior {
    /// Bytes written in `chunks` pieces, each reported as progress.
    bytes: u64,
    /// Chunks per transfer (at least one).
    chunks: u64,
    /// Total reported to the progress callback (defaults to `bytes`).
    total: u64,
    /// Error returned after writing the file.
    error: Option<String>,
    /// Resolve error instead of a stream.
    resolve_error: Option<String>,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            bytes: 1024,
            chunks: 4,
            total: 1024,
            error: None,
            resolve_error: None,
        }
    }
}

struct FakeSource {
    behavior: Mutex<Behavior>,
    /// Per-video overrides, keyed by video id.
    overrides: Mutex<HashMap<String, Behavior>>,
    /// Maximum number of transfers that were inside `fetch` at the same time.
    concurrent: AtomicUsize,
    active: AtomicUsize,
    fetches: AtomicUsize,
    /// Delay inside `fetch`, so concurrency is observable.
    hold: Duration,
}

impl FakeSource {
    fn new(behavior: Behavior) -> Self {
        Self {
            behavior: Mutex::new(behavior),
            overrides: Mutex::new(HashMap::new()),
            concurrent: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            fetches: AtomicUsize::new(0),
            hold: Duration::from_millis(60),
        }
    }

    fn with_hold(mut self, hold: Duration) -> Self {
        self.hold = hold;
        self
    }

    fn set(&self, id: &str, behavior: Behavior) {
        self.overrides.lock().insert(id.to_string(), behavior);
    }

    fn behavior_for(&self, id: &str) -> Behavior {
        self.overrides
            .lock()
            .get(id)
            .cloned()
            .unwrap_or_else(|| self.behavior.lock().clone())
    }

    fn max_concurrent(&self) -> usize {
        self.concurrent.load(Ordering::SeqCst)
    }

    fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

impl ml_downloads::AudioSource for FakeSource {
    fn resolve<'a>(
        &'a self,
        video_id: &'a str,
        _quality: AudioQuality,
    ) -> BoxFuture<'a, ml_core::MlResult<StreamInfo>> {
        let behavior = self.behavior_for(video_id);
        Box::pin(async move {
            if let Some(error) = behavior.resolve_error {
                return Err(MlError::Unavailable(error));
            }
            Ok(StreamInfo {
                url: format!("fake://{video_id}"),
                mime: "audio/webm; codecs=\"opus\"".into(),
                bitrate: None,
                itag: 251,
                content_length: Some(behavior.bytes),
                expires_at: None,
            })
        })
    }

    fn fetch<'a>(
        &'a self,
        video_id: &'a str,
        _quality: AudioQuality,
        path: &'a Path,
        mut progress: ProgressFn<'a>,
    ) -> BoxFuture<'a, ml_core::MlResult<()>> {
        let behavior = self.behavior_for(video_id);
        let active = &self.active;
        let concurrent = &self.concurrent;
        self.fetches.fetch_add(1, Ordering::SeqCst);
        let hold = self.hold;
        Box::pin(async move {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            concurrent.fetch_max(now, Ordering::SeqCst);
            let result = write_body(path, &behavior, &mut progress, hold).await;
            active.fetch_sub(1, Ordering::SeqCst);
            result
        })
    }
}

async fn write_body(
    path: &Path,
    behavior: &Behavior,
    progress: &mut ProgressFn<'_>,
    hold: Duration,
) -> ml_core::MlResult<()> {
    tokio::time::sleep(hold).await;
    let chunks = behavior.chunks.max(1);
    let per = behavior.bytes / chunks + u64::from(!behavior.bytes.is_multiple_of(chunks));
    let mut written = 0u64;
    {
        let mut file = fs::File::create(path).map_err(MlError::from)?;
        for _ in 0..chunks {
            if written >= behavior.bytes {
                break;
            }
            let step = per.min(behavior.bytes - written);
            file.write_all(&vec![0xABu8; step as usize])
                .map_err(MlError::from)?;
            written += step;
            progress(written, behavior.total.max(behavior.bytes));
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        file.flush().map_err(MlError::from)?;
    }
    if let Some(error) = &behavior.error {
        return Err(MlError::Unavailable(error.clone()));
    }
    Ok(())
}

// -------------------------------------------------------------------- helpers

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        // A counter keeps parallel tests (same process, same millisecond) apart.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let unique = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "ml-downloads-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn song(id: &str, title: &str, artist: &str) -> Song {
    Song {
        id: id.to_string(),
        title: title.to_string(),
        artists: vec![ml_core::Artist::new(artist, None)],
        ..Song::default()
    }
}

fn config(dir: &Path, max_concurrent: usize) -> DownloadConfig {
    DownloadConfig {
        dir: dir.to_path_buf(),
        max_concurrent,
        quality: AudioQuality::High,
        max_storage_bytes: None,
    }
}

fn manager(dir: &Path, source: Arc<FakeSource>, max_concurrent: usize) -> DownloadManager {
    DownloadManager::with_source(config(dir, max_concurrent), source).expect("manager starts")
}

/// Drain events until `done` returns `Some`, or panic after `timeout`.
fn wait_for<T>(
    events: &crossbeam_channel::Receiver<DownloadEvent>,
    timeout: Duration,
    mut done: impl FnMut(&DownloadEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(event) => {
                if let Some(value) = done(&event) {
                    return value;
                }
            }
            Err(_) => panic!("timed out waiting for an event"),
        }
    }
}

// ---------------------------------------------------------------------- tests

#[test]
fn enqueue_downloads_and_reports_completion() {
    let dir = TempDir::new("enqueue");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    let manager = manager(dir.path(), source.clone(), 3);
    let events = manager.events();

    let accepted = manager.enqueue(&[song("abc12345678", "Song", "Artist")]);
    assert_eq!(accepted, vec!["abc12345678".to_string()]);

    let path = wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Completed { id, path } if id == "abc12345678" => Some(path.clone()),
        _ => None,
    });
    assert_eq!(path, dir.path().join("Song - Artist [abc12345678].webm"));
    assert_eq!(fs::metadata(&path).unwrap().len(), 1024);
    assert_eq!(manager.storage_usage(), 1024);
    assert_eq!(manager.path_for("abc12345678"), Some(path));

    let entry = manager.get("abc12345678").unwrap();
    assert_eq!(entry.state, DownloadState::Completed);
    assert_eq!(entry.size_bytes, 1024);
    assert_eq!(entry.song.title, "Song");
    assert_eq!(entry.percent(), 100);
}

#[test]
fn manifest_round_trips_full_song_metadata() {
    let dir = TempDir::new("roundtrip");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    let manager = manager(dir.path(), source, 2);
    let events = manager.events();
    let track = song("vid00000001", "Title", "Composer");
    manager.enqueue(std::slice::from_ref(&track));
    wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Completed { .. } => Some(()),
        _ => None,
    });
    let before = manager.get("vid00000001").unwrap();
    manager.shutdown();

    let reopened = DownloadManager::with_source(
        config(dir.path(), 2),
        Arc::new(FakeSource::new(Behavior::default())),
    )
    .unwrap();
    let after = reopened.get("vid00000001").expect("entry survives restart");
    assert_eq!(after.state, DownloadState::Completed);
    assert_eq!(after.file_name, before.file_name);
    assert_eq!(after.size_bytes, 1024);
    assert_eq!(after.song, track, "full song metadata is persisted");
    assert_eq!(after.song.artists[0].name, "Composer");
    assert!(reopened.path_for("vid00000001").is_some());
}

#[test]
fn corrupt_manifest_is_backed_up_and_the_manager_still_starts() {
    let dir = TempDir::new("corrupt");
    fs::write(dir.path().join("manifest.json"), b"{not json at all").unwrap();
    let source = Arc::new(FakeSource::new(Behavior::default()));
    let manager = manager(dir.path(), source, 1);
    assert!(manager.list().is_empty());
    assert!(dir.path().join("manifest.json.bak").exists());
    assert!(!dir.path().join("manifest.json").exists());
    manager.shutdown();
}

#[test]
fn duplicate_enqueue_is_deduped() {
    let dir = TempDir::new("dedupe");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    let manager = manager(dir.path(), source.clone(), 4);
    let track = song("dupe0000001", "S", "A");
    let first = manager.enqueue(std::slice::from_ref(&track));
    let second = manager.enqueue(std::slice::from_ref(&track));
    assert_eq!(first.len(), 1);
    assert!(second.is_empty(), "queued entries are not re-queued");
    // The same id twice in one batch is also deduped.
    let batch = manager.enqueue(&[track.clone(), track.clone()]);
    assert!(batch.is_empty(), "a batch cannot queue one id twice");

    let events = manager.events();
    wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Completed { id, .. } if id == "dupe0000001" => Some(()),
        _ => None,
    });
    assert!(
        manager.enqueue(std::slice::from_ref(&track)).is_empty(),
        "completed entries are never prepared again"
    );
    assert_eq!(source.fetches(), 1);
}

#[test]
fn concurrency_never_exceeds_the_bound() {
    let dir = TempDir::new("bound");
    let source =
        Arc::new(FakeSource::new(Behavior::default()).with_hold(Duration::from_millis(80)));
    let manager = manager(dir.path(), source.clone(), 2);
    let songs: Vec<Song> = (0..8)
        .map(|i| song(&format!("vid{i:08}"), "T", "A"))
        .collect();
    let events = manager.events();
    assert_eq!(manager.enqueue(&songs).len(), 8);
    let mut done = 0;
    while done < 8 {
        wait_for(&events, Duration::from_secs(20), |event| match event {
            DownloadEvent::Completed { .. } => Some(()),
            _ => None,
        });
        done += 1;
    }
    assert_eq!(done, 8);
    assert_eq!(source.max_concurrent(), 2, "semaphore bounds the pool");
    assert_eq!(manager.list().len(), 8);
}

#[test]
fn concurrency_config_is_clamped() {
    let dir = TempDir::new("clamp");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    // 0 and 99 must both be accepted (clamped), never panic.
    let zero = manager(dir.path(), source.clone(), 0);
    assert_eq!(zero.enqueue(&[song("clamp000001", "T", "A")]).len(), 1);
    zero.shutdown();
    let big = manager(dir.path(), source, 99);
    assert!(big.list().len() == 1);
    big.shutdown();
}

#[test]
fn cancel_delete_and_retry_transitions() {
    let dir = TempDir::new("cancel");
    let source =
        Arc::new(FakeSource::new(Behavior::default()).with_hold(Duration::from_millis(300)));
    let manager = manager(dir.path(), source.clone(), 1);
    let events = manager.events();
    let track = song("cancel00001", "T", "A");
    manager.enqueue(std::slice::from_ref(&track));
    // Wait until the worker is actually inside the transfer.
    wait_for(&events, Duration::from_secs(5), |event| match event {
        DownloadEvent::Progress { .. } => Some(()),
        _ => None,
    });
    assert_eq!(
        manager.get("cancel00001").unwrap().state,
        DownloadState::Downloading
    );
    assert!(manager.cancel("cancel00001"));
    assert!(!manager.cancel("cancel00001"), "double cancel is a no-op");
    wait_for(&events, Duration::from_secs(5), |event| match event {
        DownloadEvent::Cancelled(id) if id == "cancel00001" => Some(()),
        _ => None,
    });
    assert_eq!(
        manager.get("cancel00001").unwrap().state,
        DownloadState::Cancelled
    );
    assert!(manager.path_for("cancel00001").is_none());

    // Retry starts from zero and completes.
    let behavior = Behavior::default();
    source.set("cancel00001", behavior);
    assert!(manager.retry("cancel00001"));
    assert!(
        !manager.retry("cancel00001"),
        "retry of a queued entry is refused"
    );
    wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Completed { id, .. } if id == "cancel00001" => Some(()),
        _ => None,
    });
    let path = manager.path_for("cancel00001").expect("file exists");
    assert_eq!(fs::metadata(&path).unwrap().len(), 1024);
    assert!(!manager.retry("cancel00001"), "completed is terminal");
}

#[test]
fn failing_transfer_deletes_the_part_file_and_publishes_a_typed_error() {
    let dir = TempDir::new("fail");
    let source = Arc::new(FakeSource::new(Behavior {
        error: Some("googlevideo said no".into()),
        ..Behavior::default()
    }));
    let manager = manager(dir.path(), source, 1);
    let events = manager.events();
    manager.enqueue(&[song("fail0000001", "T", "A")]);
    let error = wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Failed { id, error } if id == "fail0000001" => Some(error.clone()),
        _ => None,
    });
    assert!(matches!(error, DownloadError::Download(_)));
    assert_eq!(
        manager.get("fail0000001").unwrap().state,
        DownloadState::Failed
    );
    assert!(manager.path_for("fail0000001").is_none());
    assert!(
        !dir.path().join("dl_fail0000001.part").exists(),
        "the partial file is removed on failure"
    );
    assert_eq!(manager.clear_failed(), 1);
    assert!(manager.get("fail0000001").is_none());
}

#[test]
fn resolve_errors_are_reported_not_panicked() {
    let dir = TempDir::new("resolve");
    let source = Arc::new(FakeSource::new(Behavior {
        resolve_error: Some("no audio format".into()),
        ..Behavior::default()
    }));
    let manager = manager(dir.path(), source, 1);
    let events = manager.events();
    manager.enqueue(&[song("resolve0001", "T", "A")]);
    let error = wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Failed { id, error } if id == "resolve0001" => Some(error.clone()),
        _ => None,
    });
    assert!(error.to_string().contains("no audio format"));
}

#[test]
fn storage_budget_rejects_at_enqueue_and_before_transfer() {
    let dir = TempDir::new("budget");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    // A 1024-byte track exactly fills the 1024-byte budget; the next enqueue is
    // rejected at the door.
    let mut cfg = config(dir.path(), 1);
    cfg.max_storage_bytes = Some(1024);
    let manager = DownloadManager::with_source(cfg, source).unwrap();
    let events = manager.events();
    assert_eq!(manager.enqueue(&[song("budget00001", "T", "A")]).len(), 1);
    wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Completed { .. } => Some(()),
        _ => None,
    });
    let usage = manager.storage_usage();
    assert_eq!(usage, 1024);

    let accepted = manager.enqueue(&[song("budget00002", "T", "A")]);
    assert!(accepted.is_empty(), "budget is full");
    let error = wait_for(&events, Duration::from_secs(5), |event| match event {
        DownloadEvent::Failed { id, error } if id == "budget00002" => Some(error.clone()),
        _ => None,
    });
    assert_eq!(
        error,
        DownloadError::StorageFull {
            used: usage,
            limit: 1024
        }
    );
    assert!(
        manager.get("budget00002").is_none(),
        "no manifest entry is created"
    );
}

#[test]
fn streaming_budget_check_uses_the_resolved_size() {
    let dir = TempDir::new("budget-stream");
    let source = Arc::new(FakeSource::new(Behavior {
        bytes: 4096,
        ..Behavior::default()
    }));
    let mut cfg = config(dir.path(), 1);
    cfg.max_storage_bytes = Some(2000);
    let manager = DownloadManager::with_source(cfg, source).unwrap();
    let events = manager.events();
    // Enqueue passes (nothing used yet), the pre-transfer check rejects.
    assert_eq!(manager.enqueue(&[song("budget00003", "T", "A")]).len(), 1);
    let error = wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Failed { id, error } if id == "budget00003" => Some(error.clone()),
        _ => None,
    });
    assert!(matches!(error, DownloadError::StorageFull { .. }));
    assert_eq!(manager.storage_usage(), 0);
}

#[test]
fn remove_deletes_the_file_and_the_entry() {
    let dir = TempDir::new("remove");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    let manager = manager(dir.path(), source, 1);
    let events = manager.events();
    manager.enqueue(&[song("remove00001", "T", "A")]);
    let path = wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Completed { path, .. } => Some(path.clone()),
        _ => None,
    });
    assert!(path.exists());
    assert!(manager.remove("remove00001"));
    assert!(!path.exists(), "remove deletes the audio file");
    assert!(manager.get("remove00001").is_none());
    assert_eq!(manager.storage_usage(), 0);
    assert!(!manager.remove("remove00001"), "double remove is a no-op");
    wait_for(&events, Duration::from_secs(5), |event| match event {
        DownloadEvent::Removed(id) if id == "remove00001" => Some(()),
        _ => None,
    });
}

#[test]
fn remove_while_downloading_aborts_and_cleans_up() {
    let dir = TempDir::new("remove-inflight");
    let source =
        Arc::new(FakeSource::new(Behavior::default()).with_hold(Duration::from_millis(300)));
    let manager = manager(dir.path(), source, 1);
    let events = manager.events();
    manager.enqueue(&[song("removeinflight", "T", "A")]);
    wait_for(&events, Duration::from_secs(5), |event| match event {
        DownloadEvent::Progress { .. } => Some(()),
        _ => None,
    });
    assert!(manager.remove("removeinflight"));
    assert!(manager.get("removeinflight").is_none());
    wait_for(&events, Duration::from_secs(5), |event| match event {
        DownloadEvent::Removed(id) if id == "removeinflight" => Some(()),
        _ => None,
    });
    // Give the aborted task a moment to land, then assert no file survived.
    std::thread::sleep(Duration::from_millis(400));
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "manifest.json" && name != "manifest.json.bak")
        .collect();
    assert!(leftovers.is_empty(), "stray files: {leftovers:?}");
}

#[test]
fn path_for_detects_a_truncated_file() {
    let dir = TempDir::new("truncate");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    let manager = manager(dir.path(), source, 1);
    let events = manager.events();
    manager.enqueue(&[song("trunc000001", "T", "A")]);
    let path = wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Completed { path, .. } => Some(path.clone()),
        _ => None,
    });
    fs::write(&path, b"short").unwrap();
    assert_eq!(
        manager.path_for("trunc000001"),
        None,
        "size mismatch is not playable"
    );
    fs::remove_file(&path).unwrap();
    assert_eq!(manager.path_for("trunc000001"), None);
}

#[test]
fn progress_is_coalesced_to_one_event_per_100ms() {
    let dir = TempDir::new("progress");
    // 40 chunks, one progress callback every ~1 ms: far more callbacks than
    // events, so coalescing must hide almost all of them.
    let source = Arc::new(FakeSource::new(Behavior {
        bytes: 40_000,
        chunks: 40,
        total: 40_000,
        ..Behavior::default()
    }));
    let manager = manager(dir.path(), source, 1);
    let events = manager.events();
    manager.enqueue(&[song("progress001", "T", "A")]);
    let mut progress_events = 0;
    while wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Progress { .. } => Some(true),
        DownloadEvent::Completed { .. } => Some(false),
        _ => None,
    }) {
        progress_events += 1;
    }
    assert!(progress_events >= 1, "the first callback always emits");
    assert!(
        progress_events <= 3,
        "expected <= 3 coalesced events, got {progress_events}"
    );
    let entry = manager.get("progress001").unwrap();
    assert_eq!(
        entry.bytes_downloaded, 40_000,
        "the entry itself is never coalesced"
    );
    assert_eq!(entry.total_bytes, Some(40_000));
}

#[test]
fn shutdown_is_idempotent_and_leaves_no_downloading_entry() {
    let dir = TempDir::new("shutdown");
    let source =
        Arc::new(FakeSource::new(Behavior::default()).with_hold(Duration::from_millis(400)));
    let manager = manager(dir.path(), source, 2);
    let events = manager.events();
    manager.enqueue(&[song("shut0000001", "T", "A"), song("shut0000002", "T", "B")]);
    wait_for(&events, Duration::from_secs(5), |event| match event {
        DownloadEvent::Progress { .. } => Some(()),
        _ => None,
    });
    manager.shutdown();

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("manifest.json")).unwrap()).unwrap();
    for entry in manifest["entries"].as_array().unwrap() {
        assert_ne!(entry["state"], "downloading");
    }
}

#[test]
fn shutdown_flushes_the_manifest_for_the_next_manager() {
    let dir = TempDir::new("after-shutdown");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    let first = manager(dir.path(), source, 1);
    first.shutdown();
    // `shutdown` consumed it; a second manager over the same dir must still work
    // (proves the manifest was flushed and nothing holds the directory).
    let second = manager(
        dir.path(),
        Arc::new(FakeSource::new(Behavior::default())),
        1,
    );
    assert!(!second.enqueue(&[song("post0000001", "T", "A")]).is_empty());
    second.shutdown();
}

#[test]
fn dropping_a_manager_does_not_panic_mid_transfer() {
    let dir = TempDir::new("drop");
    let source =
        Arc::new(FakeSource::new(Behavior::default()).with_hold(Duration::from_millis(300)));
    let manager = manager(dir.path(), source, 1);
    let events = manager.events();
    manager.enqueue(&[song("drop0000001", "T", "A")]);
    wait_for(&events, Duration::from_secs(5), |event| match event {
        DownloadEvent::Progress { .. } => Some(()),
        _ => None,
    });
    drop(manager);
    drop(events);
}

#[test]
fn event_order_is_queued_progress_completed() {
    let dir = TempDir::new("order");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    let manager = manager(dir.path(), source, 1);
    let events = manager.events();
    manager.enqueue(&[song("order000001", "T", "A")]);

    let mut order = Vec::new();
    loop {
        match wait_for(&events, Duration::from_secs(10), |event| match event {
            DownloadEvent::Progress { .. } | DownloadEvent::Completed { .. } => Some(event.clone()),
            _ => None,
        }) {
            DownloadEvent::Completed { .. } => {
                order.push("completed");
                break;
            }
            DownloadEvent::Progress { .. } => order.push("progress"),
            _ => unreachable!(),
        }
    }
    assert_eq!(order.first(), Some(&"progress"));
    assert_eq!(order.last(), Some(&"completed"));
    assert!(order.len() >= 2);
}

#[test]
fn queued_entries_resume_when_the_manager_starts() {
    let dir = TempDir::new("resume");
    // 1. Produce a persisted entry in `failed` state.
    let failing = Arc::new(FakeSource::new(Behavior {
        error: Some("first attempt".into()),
        ..Behavior::default()
    }));
    let first = manager(dir.path(), failing, 1);
    let events = first.events();
    first.enqueue(&[song("resume00001", "T", "A")]);
    wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Failed { id, .. } if id == "resume00001" => Some(()),
        _ => None,
    });
    first.shutdown();

    // 2. Flip it to `queued`, as a shutdown mid-transfer would leave it.
    let manifest_path = dir.path().join("manifest.json");
    let json = fs::read_to_string(&manifest_path).unwrap();
    assert!(
        json.contains("\"failed\""),
        "expected a failed entry: {json}"
    );
    fs::write(&manifest_path, json.replace("\"failed\"", "\"queued\"")).unwrap();

    // 3. A new manager resumes it without any `enqueue` call.
    let working = Arc::new(FakeSource::new(Behavior::default()));
    let second = manager(dir.path(), working.clone(), 1);
    let events = second.events();
    wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Completed { id, .. } if id == "resume00001" => Some(()),
        _ => None,
    });
    assert!(second.path_for("resume00001").is_some());
    assert_eq!(working.fetches(), 1);
    second.shutdown();
}

#[test]
fn manifest_write_is_atomic() {
    let dir = TempDir::new("atomic");
    let source = Arc::new(FakeSource::new(Behavior::default()));
    let manager = manager(dir.path(), source, 1);
    let events = manager.events();
    manager.enqueue(&[song("atomic00001", "T", "A")]);
    wait_for(&events, Duration::from_secs(10), |event| match event {
        DownloadEvent::Completed { .. } => Some(()),
        _ => None,
    });
    // The temp file is renamed away, never left behind.
    assert!(!dir.path().join("manifest.json.tmp").exists());
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["entries"][0]["state"], "completed");
}
