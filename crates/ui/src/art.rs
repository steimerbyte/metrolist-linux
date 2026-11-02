//! Asynchronous thumbnail textures keyed by URL.
//!
//! `image::load_from_memory` runs on the tokio runtime; the decoded RGBA buffer
//! crosses to the UI thread, which uploads it with `Context::load_texture`.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

use crate::services::Services;

/// Decoded thumbnail ready for upload.
#[derive(Debug)]
pub struct ArtImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

enum Entry {
    Pending,
    Ready(TextureHandle),
    Failed,
}

#[derive(Default)]
pub struct ArtCache {
    entries: HashMap<String, Entry>,
}

impl ArtCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Texture for `url`, starting a background fetch on first use.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        services: &Services,
        url: &str,
    ) -> Option<TextureHandle> {
        if url.is_empty() {
            return None;
        }
        match self.entries.get(url) {
            Some(Entry::Ready(texture)) => return Some(texture.clone()),
            Some(Entry::Pending) | Some(Entry::Failed) => return None,
            None => {}
        }
        self.entries.insert(url.to_owned(), Entry::Pending);
        services.fetch_art(url.to_owned());
        let _ = ctx;
        None
    }

    /// Number of decoded thumbnails currently held.
    pub fn ready_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry, Entry::Ready(_)))
            .count()
    }

    /// Approximate decoded size of the ready textures (RGBA, GPU copy).
    pub fn memory_bytes(&self) -> usize {
        self.entries
            .values()
            .filter_map(|entry| match entry {
                Entry::Ready(texture) => {
                    let [width, height] = texture.size();
                    Some(width.saturating_mul(height).saturating_mul(4))
                }
                _ => None,
            })
            .sum()
    }

    /// Drop every texture; pending fetches finish into a fresh cache.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Apply a fetch result delivered as `UiEvent::Art`.
    pub fn insert(&mut self, ctx: &egui::Context, url: &str, image: Option<Arc<ArtImage>>) {
        let Some(image) = image else {
            self.entries.insert(url.to_owned(), Entry::Failed);
            return;
        };
        let expected = image
            .width
            .checked_mul(image.height)
            .and_then(|pixels| pixels.checked_mul(4));
        if image.width == 0 || image.height == 0 || expected != Some(image.rgba.len()) {
            self.entries.insert(url.to_owned(), Entry::Failed);
            return;
        }
        let color = ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba);
        let texture = ctx.load_texture(url, color, TextureOptions::LINEAR);
        self.entries.insert(url.to_owned(), Entry::Ready(texture));
    }
}
