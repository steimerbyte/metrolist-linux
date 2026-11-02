//! `ml-downloads` — offline download manager.
//!
//! One directory, one `manifest.json`, one bounded worker pool:
//!
//! ```no_run
//! # fn main() -> ml_core::MlResult<()> {
//! use ml_downloads::{DownloadConfig, DownloadManager};
//!
//! let manager = DownloadManager::new(DownloadConfig::new("/music/Metrolist"))?;
//! let events = manager.events();
//! # let songs: &[ml_core::Song] = &[];
//! let _ids = manager.enqueue(songs);
//! for event in events.try_iter() {
//!     println!("{event:?}");
//! }
//! manager.shutdown();
//! # Ok(())
//! # }
//! ```
//!
//! Design notes:
//!
//! - **No resume.** A failed transfer deletes its `.part` file and `retry`
//!   starts from byte 0; `InnertubeClient::download_audio` already restarts on a
//!   googlevideo 403, and a half-trusted prefix is worse than a re-download.
//! - **Manifest is the source of truth**, not an index of the directory. A
//!   corrupt manifest is moved to `manifest.json.bak` and the manager starts
//!   empty (warning, never a panic). `Queued` entries resume on the next start.
//! - **Progress is coalesced** to one event per 100 ms per transfer; the entry
//!   itself is updated on every callback.
//! - **Budget is soft**: checked at enqueue and before each transfer starts.

mod manager;
mod manifest;
mod model;
mod naming;
mod source;

pub use manager::DownloadManager;
pub use model::{
    clamp_concurrency, occupied_bytes, progress_percent, storage_sum, Download, DownloadConfig,
    DownloadError, DownloadEvent, DownloadId, DownloadState, MAX_CONCURRENT,
};
pub use naming::{extension_for_mime, file_name, sanitize, MAX_FILE_NAME_BYTES};
pub use source::{AudioSource, BoxFuture, ProgressFn};
