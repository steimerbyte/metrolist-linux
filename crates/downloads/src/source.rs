//! The byte source the manager downloads through.
//!
//! Production uses [`ml_innertube::InnertubeClient`] (VISIONOS ranged 206
//! requests, `arch/pot-notes.md` §2). Tests implement this trait with a fake,
//! so the queue/state machine is exercised without a network.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use ml_core::{AudioQuality, MlResult, StreamInfo};
use ml_innertube::InnertubeClient;

/// Boxed future alias (the trait must stay object-safe for `Arc<dyn AudioSource>`).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Write progress callback: `(bytes_written, total_bytes)`, `total == 0` while unknown.
pub type ProgressFn<'a> = Box<dyn FnMut(u64, u64) + Send + 'a>;

/// Fetch whole tracks from YouTube Music.
pub trait AudioSource: Send + Sync {
    /// Resolve the stream for `video_id`; the mime type decides the file
    /// extension and `content_length` (when present) is the size budget check.
    fn resolve<'a>(
        &'a self,
        video_id: &'a str,
        quality: AudioQuality,
    ) -> BoxFuture<'a, MlResult<StreamInfo>>;

    /// Write the complete track to `path`, reporting progress as it goes.
    ///
    /// The manager guarantees `path` is a `*.part` file inside its directory and
    /// deletes it whenever this returns an error or the future is dropped.
    fn fetch<'a>(
        &'a self,
        video_id: &'a str,
        quality: AudioQuality,
        path: &'a Path,
        progress: ProgressFn<'a>,
    ) -> BoxFuture<'a, MlResult<()>>;
}

impl AudioSource for InnertubeClient {
    fn resolve<'a>(
        &'a self,
        video_id: &'a str,
        quality: AudioQuality,
    ) -> BoxFuture<'a, MlResult<StreamInfo>> {
        Box::pin(async move { self.stream_info(video_id, quality).await })
    }

    fn fetch<'a>(
        &'a self,
        video_id: &'a str,
        quality: AudioQuality,
        path: &'a Path,
        progress: ProgressFn<'a>,
    ) -> BoxFuture<'a, MlResult<()>> {
        Box::pin(async move { self.download_audio(video_id, quality, path, progress).await })
    }
}
