//! PO-token seam.
//!
//! Measured 2026-11-02 (`arch/pot-notes.md` §1): VISIONOS serves full tracks
//! without a PO token; IOS hard-caps at 512 KiB (251) / 1 MiB (140) then 403,
//! and every tested token route was rejected. The seam remains for a future
//! accepted token source and defaults to [`NoPot`], so the crate never depends
//! on a JS engine. A real provider would send `player` in
//! `serviceIntegrityDimensions.poToken` and `gvs` as `pot=` on the media URL,
//! both bound to `visitor_data`.

use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use ml_core::MlResult;

/// Tokens minted for one video by a [`TokenProvider`].
#[derive(Debug, Clone, Default)]
pub struct PoTokens {
    /// `serviceIntegrityDimensions.poToken` on the player request.
    pub player: Option<String>,
    /// `pot` query parameter on the resolved googlevideo URL.
    pub gvs: Option<String>,
    /// Visitor id the tokens are bound to; sent in context and `X-Goog-Visitor-Id`.
    pub visitor_data: String,
    pub expires_at: Option<SystemTime>,
}

impl PoTokens {
    pub fn is_expired(&self, now: SystemTime) -> bool {
        self.expires_at.map(|at| now >= at).unwrap_or(false)
    }
}

/// In-process BotGuard minter (or any other provider).
///
/// The future is boxed because async trait methods are not dyn-compatible on
/// stable Rust; the box is one per player/stream request, not per byte.
pub trait TokenProvider: Send + Sync {
    fn po_tokens<'a>(
        &'a self,
        video_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = MlResult<Option<PoTokens>>> + Send + 'a>>;

    /// Drop any cached token for `video_id`; called after a googlevideo 403.
    fn invalidate(&self, _video_id: &str) {}
}

/// Default provider: PO tokens are not required on the VISIONOS path
/// (`arch/pot-notes.md` §1, §4). Never wire a JS engine by default.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoPot;

impl TokenProvider for NoPot {
    fn po_tokens<'a>(
        &'a self,
        _video_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = MlResult<Option<PoTokens>>> + Send + 'a>> {
        Box::pin(async { Ok(None) })
    }
}
