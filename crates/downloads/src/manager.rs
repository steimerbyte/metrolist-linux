//! The download queue: one manifest, one bounded worker pool, one event stream.
//!
//! Concurrency model: [`DownloadManager`] owns a small tokio runtime and one
//! `Semaphore(max_concurrent)`. Every accepted enqueue spawns a task that first
//! acquires a permit, so at most N transfers touch the network at once and the
//! rest wait in FIFO order. Cancelling aborts the task, which drops the in-flight
//! HTTP response and the write handle.
//!
//! Resume semantics: none at the byte level. A failure (including a googlevideo
//! 403 that makes the innertube loop restart at byte 0) deletes the `.part` file,
//! and `retry` starts from zero — one file per track, never a partially trusted
//! prefix. Persisted `Queued` entries are restarted when a manager is created,
//! so a clean shutdown or a crash does not strand the queue.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use ml_core::{AudioQuality, MlError, MlResult, Song};
use parking_lot::Mutex;
use tokio::runtime::Runtime;
use tokio::sync::Semaphore;
use tokio::task::AbortHandle;
use tracing::warn;

use crate::manifest;
use crate::model::{
    clamp_concurrency, usage_of, Download, DownloadConfig, DownloadError, DownloadEvent,
    DownloadId, DownloadState,
};
use crate::naming;
use crate::source::{AudioSource, ProgressFn};

/// Progress events are coalesced to one per transfer per this interval.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// How long `shutdown()` waits for running transfers before dropping them.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Shared {
    entries: BTreeMap<DownloadId, Download>,
    running: HashMap<DownloadId, AbortHandle>,
}

impl Shared {
    fn usage(&self) -> u64 {
        usage_of(self.entries.values())
    }
}

struct Inner {
    dir: PathBuf,
    quality: AudioQuality,
    max_storage_bytes: Option<u64>,
    source: Arc<dyn AudioSource>,
    events: Sender<DownloadEvent>,
    permits: Arc<Semaphore>,
    shared: Mutex<Shared>,
    shutdown: AtomicBool,
}

impl Inner {
    fn emit(&self, event: DownloadEvent) {
        let _ = self.events.send(event);
    }

    fn persist(&self, shared: &Shared) {
        if let Err(error) = manifest::save(&self.dir, &shared.entries) {
            warn!(error = %error, "cannot write download manifest");
        }
    }

    fn storage_event(&self, shared: &Shared) {
        self.emit(DownloadEvent::StorageChanged {
            used: shared.usage(),
            limit: self.max_storage_bytes,
        });
    }

    fn part_path(&self, id: &str) -> PathBuf {
        naming::part_path(&self.dir, id)
    }

    fn fail(&self, id: &str, error: DownloadError) {
        let _ = fs::remove_file(self.part_path(id));
        let mut shared = self.shared.lock();
        shared.running.remove(id);
        let matched = match shared.entries.get_mut(id) {
            Some(entry) if entry.state == DownloadState::Downloading => {
                entry.state = DownloadState::Failed;
                entry.error = Some(error.to_string());
                true
            }
            _ => false,
        };
        if matched {
            self.persist(&shared);
            self.emit(DownloadEvent::Failed {
                id: id.to_string(),
                error,
            });
            self.storage_event(&shared);
        }
    }

    /// Move the finished `.part` into its final name. A cancel that won the
    /// race has already set a non-`Downloading` state, and then this deletes.
    fn complete(&self, id: &str, part: &Path, mime: &str) {
        let mut shared = self.shared.lock();
        let running = shared.entries.get(id).is_some_and(|entry| {
            entry.state == DownloadState::Downloading && !self.shutdown.load(Ordering::SeqCst)
        });
        if !running {
            drop(shared);
            let _ = fs::remove_file(part);
            return;
        }
        let name = {
            let entry = match shared.entries.get(id) {
                Some(entry) => entry,
                None => {
                    drop(shared);
                    let _ = fs::remove_file(part);
                    return;
                }
            };
            naming::file_name(
                &entry.song.title,
                &entry.song.artist_names(),
                id,
                naming::extension_for_mime(mime),
            )
        };
        let target = self.dir.join(&name);
        let size = match fs::metadata(part) {
            Ok(meta) => meta.len(),
            Err(error) => {
                drop(shared);
                self.fail(id, DownloadError::from(MlError::from(error)));
                return;
            }
        };
        if size == 0 {
            drop(shared);
            self.fail(
                id,
                DownloadError::Download("downloaded file is empty".to_string()),
            );
            return;
        }
        if let Err(error) = fs::rename(part, &target) {
            drop(shared);
            self.fail(id, DownloadError::from(MlError::from(error)));
            return;
        }
        let Some(entry) = shared.entries.get_mut(id) else {
            drop(shared);
            let _ = fs::remove_file(&target);
            return;
        };
        entry.state = DownloadState::Completed;
        entry.file_name = Some(name);
        entry.size_bytes = size;
        entry.bytes_downloaded = size;
        entry.total_bytes = Some(size);
        entry.error = None;
        shared.running.remove(id);
        self.persist(&shared);
        self.emit(DownloadEvent::Completed {
            id: id.to_string(),
            path: target,
        });
        self.storage_event(&shared);
    }
}

/// Offline download manager. Every method takes `&self`, so share it behind an
/// `Arc`; only [`DownloadManager::shutdown`] consumes it.
pub struct DownloadManager {
    inner: Arc<Inner>,
    runtime: Option<Runtime>,
    events_rx: Receiver<DownloadEvent>,
}

fn build_runtime(workers: usize) -> MlResult<Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(clamp_concurrency(workers))
        .thread_name("ml-downloads")
        .enable_all()
        .build()
        .map_err(|error| MlError::Unavailable(format!("download runtime: {error}")))
}

impl DownloadManager {
    /// Create the directory, load the manifest and start the worker pool.
    /// Tracks come from a default [`ml_innertube::InnertubeClient`].
    pub fn new(config: DownloadConfig) -> MlResult<Self> {
        Self::with_source(config, Arc::new(ml_innertube::InnertubeClient::default()))
    }

    /// [`DownloadManager::new`] with an injected byte source (tests, or a caller
    /// that already owns a configured InnerTube client).
    pub fn with_source(config: DownloadConfig, source: Arc<dyn AudioSource>) -> MlResult<Self> {
        fs::create_dir_all(&config.dir).map_err(MlError::from)?;
        // No transfer can be live here, so every `dl_*.part` is a crash leftover.
        let swept = manifest::sweep_part_files(&config.dir);
        if swept > 0 {
            warn!(files = swept, "swept incomplete download files");
        }
        let entries = manifest::load(&config.dir);
        let workers = clamp_concurrency(config.max_concurrent);
        let (events, events_rx) = crossbeam_channel::unbounded();
        let runtime = build_runtime(workers)?;
        let manager = Self {
            inner: Arc::new(Inner {
                dir: config.dir,
                quality: config.quality,
                max_storage_bytes: config.max_storage_bytes,
                source,
                events,
                permits: Arc::new(Semaphore::new(workers)),
                shared: Mutex::new(Shared {
                    entries,
                    running: HashMap::new(),
                }),
                shutdown: AtomicBool::new(false),
            }),
            runtime: Some(runtime),
            events_rx,
        };
        manager.start_queued();
        Ok(manager)
    }

    /// Resume the queue after a restart: every persisted `Queued` entry starts
    /// again from byte 0 (there is no byte-level resume). This mirrors media3's
    /// `DownloadService`, which resumes pending downloads when it starts.
    fn start_queued(&self) {
        let queued: Vec<DownloadId> = self
            .inner
            .shared
            .lock()
            .entries
            .iter()
            .filter(|(_, entry)| entry.state == DownloadState::Queued)
            .map(|(id, _)| id.clone())
            .collect();
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        for id in queued {
            self.register(runtime, id);
        }
    }

    /// Download directory (holds `manifest.json` and the audio files).
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// Event stream. `Receiver` is cloneable, so split the stream with care and
    /// drain it (the channel is unbounded, like `ml-audio`'s).
    pub fn events(&self) -> Receiver<DownloadEvent> {
        self.events_rx.clone()
    }

    /// Queue `songs`, returning the ids that were started.
    ///
    /// Completed entries and entries already queued/in flight are skipped (the
    /// latter is the dedupe: a second enqueue never double-downloads). Failed or
    /// cancelled entries are retried. When the storage budget is already used
    /// up, every song is rejected with a [`DownloadEvent::Failed`] carrying
    /// [`DownloadError::StorageFull`] and gets no manifest entry.
    pub fn enqueue(&self, songs: &[Song]) -> Vec<DownloadId> {
        let mut accepted = Vec::new();
        if self.inner.shutdown.load(Ordering::SeqCst) {
            return accepted;
        }
        let mut spawn = Vec::new();
        {
            let mut shared = self.inner.shared.lock();
            // Queued entries occupy nothing, so the usage snapshot stays valid
            // for the whole batch.
            let used = shared.usage();
            let full = self.inner.max_storage_bytes.filter(|limit| used >= *limit);
            for song in songs {
                if song.id.is_empty() {
                    continue;
                }
                match shared.entries.get(&song.id).map(|entry| entry.state) {
                    Some(DownloadState::Completed)
                    | Some(DownloadState::Queued)
                    | Some(DownloadState::Downloading) => continue,
                    _ => {}
                }
                if let Some(limit) = full {
                    self.inner.emit(DownloadEvent::Failed {
                        id: song.id.clone(),
                        error: DownloadError::StorageFull { used, limit },
                    });
                    continue;
                }
                match shared.entries.get_mut(&song.id) {
                    Some(entry) => {
                        entry.song = song.clone();
                        entry.state = DownloadState::Queued;
                        entry.error = None;
                        entry.bytes_downloaded = 0;
                        entry.total_bytes = None;
                        entry.file_name = None;
                        entry.size_bytes = 0;
                    }
                    None => {
                        let entry =
                            Download::new(song.clone(), self.inner.quality, ml_core::now_millis());
                        shared.entries.insert(song.id.clone(), entry);
                    }
                }
                spawn.push(song.id.clone());
                accepted.push(song.id.clone());
            }
            if !accepted.is_empty() {
                self.inner.persist(&shared);
            }
        }
        let Some(runtime) = self.runtime.as_ref() else {
            return Vec::new();
        };
        for id in spawn {
            // Announce before spawning so a fast transfer cannot report
            // `Completed` ahead of its own `Queued`.
            self.inner.emit(DownloadEvent::Queued(id.clone()));
            self.register(runtime, id);
        }
        accepted
    }

    /// All entries, ordered by song id (`BTreeMap` order, deterministic).
    pub fn list(&self) -> Vec<Download> {
        self.inner.shared.lock().entries.values().cloned().collect()
    }

    pub fn get(&self, song_id: &str) -> Option<Download> {
        self.inner.shared.lock().entries.get(song_id).cloned()
    }

    /// Path of a finished, verified file: `Completed`, safe file name, the file
    /// exists and its size still matches the recorded one.
    pub fn path_for(&self, song_id: &str) -> Option<PathBuf> {
        let entry = self.inner.shared.lock().entries.get(song_id).cloned()?;
        if entry.state != DownloadState::Completed {
            return None;
        }
        let name = entry.file_name.as_deref()?;
        if !manifest::is_plain_file_name(name) {
            return None;
        }
        let path = self.inner.dir.join(name);
        let size = fs::metadata(&path).ok()?.len();
        (size == entry.size_bytes).then_some(path)
    }

    /// Bytes occupied by completed files plus bytes written by in-flight ones.
    pub fn storage_usage(&self) -> u64 {
        self.inner.shared.lock().usage()
    }

    /// Stop a queued or running transfer; the `.part` file is deleted. The entry
    /// stays as `Cancelled` until `retry` or `remove`.
    pub fn cancel(&self, id: &str) -> bool {
        let mut shared = self.inner.shared.lock();
        let Some(entry) = shared.entries.get_mut(id) else {
            return false;
        };
        if !DownloadState::can_transition(entry.state, DownloadState::Cancelled)
            || entry.state == DownloadState::Cancelled
        {
            return false;
        }
        entry.state = DownloadState::Cancelled;
        entry.error = None;
        entry.bytes_downloaded = 0;
        let abort = shared.running.remove(id);
        self.inner.persist(&shared);
        drop(shared);
        // Abort after unlocking: the task needs this lock to finish.
        if let Some(abort) = abort {
            abort.abort();
        }
        let _ = fs::remove_file(self.inner.part_path(id));
        // shortcut: `abort` is asynchronous, so a transfer that had not reached
        // an await point yet may recreate the `.part` after this delete. It is
        // named `dl_<id>.part`, is ignored by the library, and is swept on the
        // next startup/attempt. Upgrade if orphaned parts ever cost real space.
        self.inner.emit(DownloadEvent::Cancelled(id.to_string()));
        let usage = self.inner.shared.lock().usage();
        self.inner.emit(DownloadEvent::StorageChanged {
            used: usage,
            limit: self.inner.max_storage_bytes,
        });
        true
    }

    /// Re-queue a failed or cancelled entry. Completed entries are final
    /// (upstream parity: a completed download is never prepared again).
    pub fn retry(&self, id: &str) -> bool {
        {
            let mut shared = self.inner.shared.lock();
            let Some(entry) = shared.entries.get_mut(id) else {
                return false;
            };
            if !matches!(
                entry.state,
                DownloadState::Failed | DownloadState::Cancelled
            ) {
                return false;
            }
            entry.state = DownloadState::Queued;
            entry.error = None;
            entry.bytes_downloaded = 0;
            entry.total_bytes = None;
            entry.file_name = None;
            entry.size_bytes = 0;
            self.inner.persist(&shared);
        }
        let _ = fs::remove_file(self.inner.part_path(id));
        let Some(runtime) = self.runtime.as_ref() else {
            return false;
        };
        self.inner.emit(DownloadEvent::Queued(id.to_string()));
        self.register(runtime, id.to_string());
        true
    }

    /// Delete the file (if any) and the entry.
    pub fn remove(&self, id: &str) -> bool {
        let abort = {
            let mut shared = self.inner.shared.lock();
            let Some(entry) = shared.entries.remove(id) else {
                return false;
            };
            if let Some(name) = entry.file_name.as_deref() {
                if manifest::is_plain_file_name(name) {
                    let _ = fs::remove_file(self.inner.dir.join(name));
                }
            }
            let abort = shared.running.remove(id);
            self.inner.persist(&shared);
            drop(shared);
            self.inner.emit(DownloadEvent::Removed(id.to_string()));
            let usage = self.inner.shared.lock().usage();
            self.inner.emit(DownloadEvent::StorageChanged {
                used: usage,
                limit: self.inner.max_storage_bytes,
            });
            abort
        };
        if let Some(abort) = abort {
            abort.abort();
        }
        let _ = fs::remove_file(self.inner.part_path(id));
        true
    }

    /// Spawn one job and record its abort handle. A `remove` between the spawn
    /// and this insert leaves no entry, so the handle is dropped unused.
    fn register(&self, runtime: &Runtime, id: DownloadId) {
        let handle = runtime.spawn(run_job(self.inner.clone(), id.clone()));
        let abort = handle.abort_handle();
        drop(handle);
        let mut shared = self.inner.shared.lock();
        if shared.entries.contains_key(&id) {
            shared.running.insert(id, abort);
        }
    }

    /// Drop every `Failed` entry (and its stray `.part` file). Returns the count.
    pub fn clear_failed(&self) -> usize {
        let failed: Vec<DownloadId> = {
            let mut shared = self.inner.shared.lock();
            let failed: Vec<DownloadId> = shared
                .entries
                .iter()
                .filter(|(_, entry)| entry.state == DownloadState::Failed)
                .map(|(id, _)| id.clone())
                .collect();
            if failed.is_empty() {
                return 0;
            }
            for id in &failed {
                shared.entries.remove(id);
            }
            self.inner.persist(&shared);
            failed
        };
        for id in &failed {
            let _ = fs::remove_file(self.inner.part_path(id));
            self.inner.emit(DownloadEvent::Removed(id.clone()));
        }
        let usage = self.inner.shared.lock().usage();
        self.inner.emit(DownloadEvent::StorageChanged {
            used: usage,
            limit: self.inner.max_storage_bytes,
        });
        failed.len()
    }

    /// Cancel every worker and join the pool. Never panics, also called by `Drop`.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        // `shutdown()` consumes `self`, so `Drop` runs right after it; the swap
        // makes the second pass a no-op instead of a second manifest write.
        if self.inner.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let mut shared = self.inner.shared.lock();
            for (_, abort) in shared.running.drain() {
                abort.abort();
            }
            // Nothing is in flight; a reload must not see `Downloading`.
            for entry in shared.entries.values_mut() {
                if entry.state == DownloadState::Downloading {
                    entry.state = DownloadState::Queued;
                    entry.bytes_downloaded = 0;
                }
            }
            self.inner.persist(&shared);
        }
        if let Some(runtime) = self.runtime.take() {
            // Dropping a runtime inside another runtime panics; that must not
            // happen in `Drop`, so fall back to a background shutdown there.
            if tokio::runtime::Handle::try_current().is_ok() {
                runtime.shutdown_background();
            } else {
                runtime.shutdown_timeout(SHUTDOWN_GRACE);
            }
        }
        // Tasks are gone now, so a `.part` here is debris from an abort.
        let swept = manifest::sweep_part_files(&self.inner.dir);
        if swept > 0 {
            warn!(
                files = swept,
                "removed incomplete download files on shutdown"
            );
        }
    }
}

impl Drop for DownloadManager {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn run_job(inner: Arc<Inner>, id: DownloadId) {
    let Ok(_permit) = inner.permits.clone().acquire_owned().await else {
        return;
    };
    if inner.shutdown.load(Ordering::SeqCst) {
        return;
    }
    {
        let mut shared = inner.shared.lock();
        match shared.entries.get_mut(&id) {
            Some(entry) if entry.state == DownloadState::Queued => {
                entry.state = DownloadState::Downloading;
                entry.error = None;
                entry.bytes_downloaded = 0;
                entry.total_bytes = None;
            }
            // Cancelled or removed while waiting for a permit.
            _ => return,
        }
        inner.persist(&shared);
    }
    let part = inner.part_path(&id);
    let _ = fs::remove_file(&part);

    let stream = match inner.source.resolve(&id, inner.quality).await {
        Ok(stream) => stream,
        Err(error) => {
            inner.fail(&id, DownloadError::from(error));
            return;
        }
    };
    if let Some(limit) = inner.max_storage_bytes {
        let need = stream.content_length.unwrap_or(0);
        let used = inner.shared.lock().usage();
        if used.saturating_add(need) > limit {
            inner.fail(&id, DownloadError::StorageFull { used, limit });
            return;
        }
    }

    let progress = progress_callback(inner.clone(), id.clone());
    let result = inner
        .source
        .fetch(&id, inner.quality, &part, progress)
        .await;
    // Release the permit before committing so a slow rename cannot idle the pool.
    drop(_permit);
    match result {
        Ok(()) => inner.complete(&id, &part, &stream.mime),
        Err(error) => inner.fail(&id, DownloadError::from(error)),
    }
}

/// Progress sink: every callback updates the entry, events are coalesced to one
/// per [`PROGRESS_INTERVAL`] (the first callback always emits).
fn progress_callback(inner: Arc<Inner>, id: DownloadId) -> ProgressFn<'static> {
    let mut last: Option<Instant> = None;
    Box::new(move |bytes: u64, total: u64| {
        {
            let mut shared = inner.shared.lock();
            match shared.entries.get_mut(&id) {
                Some(entry) if entry.state == DownloadState::Downloading => {
                    entry.bytes_downloaded = bytes;
                    if total > 0 {
                        entry.total_bytes = Some(total);
                    }
                }
                _ => return,
            }
        }
        let due = last.is_none_or(|instant| instant.elapsed() >= PROGRESS_INTERVAL);
        if due {
            last = Some(Instant::now());
            inner.emit(DownloadEvent::Progress {
                id: id.clone(),
                bytes,
                total,
            });
        }
    })
}
