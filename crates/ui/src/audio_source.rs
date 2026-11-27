//! The production [`AudioSource`]: the InnerTube client behind the downloads
//! contract.
//!
//! `ml-downloads` owns the trait and must not import `ml-innertube`; this
//! newtype is the composition root's implementation and is injected through
//! `DownloadManager::with_source`.

use std::path::Path;

use ml_core::{AudioQuality, MlResult, StreamInfo};
use ml_downloads::{AudioSource, BoxFuture, ProgressFn};
use ml_innertube::InnertubeClient;

/// InnerTube as the download manager's byte source (VISIONOS ranged 206).
pub struct InnerTubeAudioSource(pub InnertubeClient);

impl Default for InnerTubeAudioSource {
    fn default() -> Self {
        Self(InnertubeClient::default())
    }
}

impl AudioSource for InnerTubeAudioSource {
    fn resolve<'a>(
        &'a self,
        video_id: &'a str,
        quality: AudioQuality,
    ) -> BoxFuture<'a, MlResult<StreamInfo>> {
        Box::pin(async move { self.0.stream_info(video_id, quality).await })
    }

    fn fetch<'a>(
        &'a self,
        video_id: &'a str,
        quality: AudioQuality,
        path: &'a Path,
        progress: ProgressFn<'a>,
    ) -> BoxFuture<'a, MlResult<()>> {
        Box::pin(async move {
            self.0
                .download_audio(video_id, quality, path, progress)
                .await
        })
    }
}
