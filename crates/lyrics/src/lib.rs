//! `ml-lyrics` — LRC/TTML parsing, playback helpers, romanization and the provider
//! ladder for the Metrolist Linux port.
//!
//! * [`parse_lrc`] / [`parse_ttml`] turn provider payloads into [`LyricsDocument`]
//!   (plain LRC, rich sync with `<mm:ss.xx>word` markers, and TTML from Apple/BetterLyrics).
//! * [`active_line_at`] / [`active_word_at`] answer "what is being sung now".
//! * [`romanize_document`]: one script per song, never re-detected per line, covering
//!   Japanese, Korean, Chinese, Hindi and Macedonian Cyrillic; [`apply_offset`] shifts
//!   a whole document when the user taps "sync".
//! * [`TranslationClient`]: DeepL or any OpenAI-compatible endpoint with the user's own
//!   key, refused with a typed error when no key is configured.
//! * [`LyricsClient::fetch`] walks the enabled providers; LRCLIB is the only default
//!   because it is keyless and verified, while BetterLyrics (401 without a key),
//!   Paxsenix/LyricsPlus/Zemer (undocumented) and KuGou (opt-in) are refused with a
//!   typed error instead of a hidden request.
//!
//! Matching ladder (as upstream): title+artist+album → title → `artist title` →
//! `title` → raw, with a ±2 s duration match relaxed to ±5 s.

#![forbid(unsafe_code)]

pub mod hindi;
pub mod invariants;
pub mod korean;
pub mod lrc;
pub mod providers;
pub mod romanize;
pub mod timing;
pub mod translate;
pub mod ttml;

use std::sync::Arc;

pub use lrc::{decode_kugou, parse_lrc, parse_lrc_with_meta, Lrc, LrcMeta};
pub use providers::{
    clean_artist, clean_title, fetch_kugou, fetch_lrclib, plain_document, refusal, FakeTransport,
    HttpTransport, ProviderKind, ReqwestTransport, USER_AGENT,
};
pub use romanize::{
    detect_cyrillic_language, detect_script, is_macedonian, romanize_chinese, romanize_detected,
    romanize_document, romanize_document_enabled, romanize_hindi, romanize_japanese,
    romanize_korean, romanize_line, romanize_macedonian, CyrillicLanguage, Romanization, Script,
    ALL_LANGUAGES, ALL_SCRIPTS,
};
pub use timing::{
    active_line_at, active_line_index, active_line_indices, active_lines_at, active_word_at,
    active_word_index, ACTIVE_THRESHOLD_MS,
};
pub use translate::{
    parse_translation_content, TranslationClient, TranslationConfig, TranslationEndpoint,
};
pub use ttml::{parse_ttml, to_lrc};

use ml_core::{LyricsDocument, LyricsLine, MlError, MlResult};

/// Supported providers, in fallback order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsProvider {
    LrcLib,
    LyricsPlus,
    KuGou,
    BetterLyrics,
    Paxsenix,
    Zemer,
}

impl LyricsProvider {
    pub fn name(self) -> &'static str {
        match self {
            Self::LrcLib => "lrclib",
            Self::LyricsPlus => "lyricsplus",
            Self::KuGou => "kugou",
            Self::BetterLyrics => "betterlyrics",
            Self::Paxsenix => "paxsenix",
            Self::Zemer => "zemer",
        }
    }

    /// Every provider id, in upstream's default fallback order.
    pub const ALL: [LyricsProvider; 6] = [
        LyricsProvider::LrcLib,
        LyricsProvider::LyricsPlus,
        LyricsProvider::KuGou,
        LyricsProvider::BetterLyrics,
        LyricsProvider::Paxsenix,
        LyricsProvider::Zemer,
    ];

    /// Parse a stored provider id (`"lrclib"`, `"kugou"`, …); case-insensitive.
    pub fn from_name(name: &str) -> Option<Self> {
        let wanted = name.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|p| p.name() == wanted)
    }

    /// Whether the provider is actually wired (its endpoint is documented and keyless
    /// or already covered by another provider).
    pub fn is_available(self) -> bool {
        matches!(self, Self::LrcLib | Self::KuGou)
    }

    /// Whether this provider is on when nothing is configured (upstream: LRCLIB only).
    pub fn enabled_by_default(self) -> bool {
        matches!(self, Self::LrcLib)
    }

    /// Why a provider cannot be used, or `None` when it is wired.
    pub fn disabled_reason(self) -> Option<&'static str> {
        match self {
            Self::LrcLib | Self::KuGou => None,
            Self::BetterLyrics => Some("needs an API key (HTTP 401 without one)"),
            Self::Paxsenix => Some("Apple Music scraper, no documented contract"),
            Self::LyricsPlus => Some("endpoints not documented"),
            Self::Zemer => Some("proxies LRCLIB; enable lrclib instead"),
        }
    }

    /// Availability report for the settings UI: `(provider, usable, reason)`.
    pub fn availability() -> [(LyricsProvider, bool, Option<&'static str>); 6] {
        Self::ALL.map(|provider| {
            (
                provider,
                provider.is_available(),
                provider.disabled_reason(),
            )
        })
    }

    pub fn kind(self) -> ProviderKind {
        match self {
            Self::LrcLib => ProviderKind::LrcLib,
            Self::LyricsPlus => ProviderKind::LyricsPlus,
            Self::KuGou => ProviderKind::KuGou,
            Self::BetterLyrics => ProviderKind::BetterLyrics,
            Self::Paxsenix => ProviderKind::Paxsenix,
            Self::Zemer => ProviderKind::Zemer,
        }
    }
}

/// Everything a provider needs to match a track.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LyricsQuery {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Option<u32>,
}

impl LyricsQuery {
    pub fn new(title: impl Into<String>, artist: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            artist: artist.into(),
            ..Self::default()
        }
    }

    pub fn with_album(mut self, album: impl Into<String>) -> Self {
        self.album = Some(album.into());
        self
    }

    pub fn with_duration(mut self, seconds: u32) -> Self {
        self.duration = Some(seconds);
        self
    }
}

/// Tries each enabled provider in order and returns the first synced result.
///
/// No caching here — the store owns the lyrics cache. Errors from a provider are only
/// reported when every provider has failed.
#[derive(Clone)]
pub struct LyricsClient {
    providers: Vec<LyricsProvider>,
    http: Arc<dyn HttpTransport>,
}

impl std::fmt::Debug for LyricsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LyricsClient")
            .field("providers", &self.providers)
            .finish_non_exhaustive()
    }
}

impl LyricsClient {
    /// Build a client over the real HTTP transport.
    pub fn new(providers: Vec<LyricsProvider>) -> Self {
        Self {
            providers,
            http: Arc::new(ReqwestTransport::with_defaults()),
        }
    }

    /// Build a client over a caller-supplied transport (tests, or a shared `reqwest`).
    pub fn with_transport(providers: Vec<LyricsProvider>, http: Arc<dyn HttpTransport>) -> Self {
        Self { providers, http }
    }

    /// The default ladder: LRCLIB only, off the network until `fetch` is called.
    pub fn default_providers() -> Self {
        Self::new(vec![LyricsProvider::LrcLib])
    }

    /// Build a ladder from user-ordered ids, dropping duplicates and empty entries.
    ///
    /// Unavailable providers are **kept** (fetch answers their typed refusal), so the
    /// settings UI can show why a chosen provider produced nothing.
    pub fn from_names(names: &[&str]) -> Self {
        let mut providers: Vec<LyricsProvider> = Vec::new();
        for name in names {
            if let Some(provider) = LyricsProvider::from_name(name) {
                if !providers.contains(&provider) {
                    providers.push(provider);
                }
            }
        }
        if providers.is_empty() {
            providers.push(LyricsProvider::LrcLib);
        }
        Self::new(providers)
    }

    pub fn providers(&self) -> &[LyricsProvider] {
        &self.providers
    }

    /// Try every enabled provider in order and return the first synced result.
    ///
    /// A synced document wins; an unsynced (plain) result is kept as a fallback so a
    /// caller still gets lyrics when no provider has timings.
    pub async fn fetch(&self, query: &LyricsQuery) -> MlResult<LyricsDocument> {
        let mut plain_fallback: Option<LyricsDocument> = None;
        let mut last_error = None;

        for provider in &self.providers {
            let result = match provider {
                LyricsProvider::LrcLib => fetch_lrclib(self.http.as_ref(), query).await,
                LyricsProvider::KuGou => fetch_kugou(self.http.as_ref(), query).await,
                other => Err(refusal(other.kind())),
            };
            match result {
                Ok(doc) if doc.is_synced() => return Ok(doc),
                Ok(doc) => {
                    plain_fallback.get_or_insert(doc);
                }
                Err(error) => {
                    if last_error.is_none() {
                        last_error = Some(error);
                    }
                }
            }
        }

        match plain_fallback {
            Some(doc) => Ok(doc),
            None => Err(last_error.unwrap_or_else(|| MlError::NotFound(query.title.clone()))),
        }
    }
}

// ------------------------------------------------------------------- display helpers

/// Shift every line and word by `offset_ms`, in place.
///
/// Sign matches upstream's `SongEntity.lyricsOffset`: a **positive** offset moves the
/// text *later* (`start_ms + offset`), i.e. the lyrics lag behind the audio. Timestamps
/// clamp at zero, so an offset larger than the first line cannot wrap around, and the
/// line order never changes.
pub fn apply_offset(doc: &mut LyricsDocument, offset_ms: i64) {
    for line in &mut doc.lines {
        line.start_ms = shift(line.start_ms, offset_ms);
        if let Some(end) = line.end_ms {
            line.end_ms = Some(shift(end, offset_ms));
        }
        for word in &mut line.words {
            word.start_ms = shift(word.start_ms, offset_ms);
            word.end_ms = shift(word.end_ms, offset_ms);
        }
    }
    if let Some(translated) = &mut doc.translated {
        for line in translated {
            line.start_ms = shift(line.start_ms, offset_ms);
            if let Some(end) = line.end_ms {
                line.end_ms = Some(shift(end, offset_ms));
            }
            for word in &mut line.words {
                word.start_ms = shift(word.start_ms, offset_ms);
                word.end_ms = shift(word.end_ms, offset_ms);
            }
        }
    }
}

/// Saturating shift of one timestamp; the result never goes below zero.
fn shift(ms: u64, offset_ms: i64) -> u64 {
    if offset_ms >= 0 {
        ms.saturating_add(offset_ms as u64)
    } else {
        ms.saturating_sub(offset_ms.unsigned_abs())
    }
}

/// The document with [`romanize_document`] applied, or `None` when the script is not
/// ported (the caller keeps showing the original lines).
///
/// Word timings are preserved, so karaoke rendering keeps working; only `text` changes.
pub fn romanized(doc: &LyricsDocument) -> Option<LyricsDocument> {
    let lines = romanize_document(doc)?;
    Some(LyricsDocument {
        lines,
        ..doc.clone()
    })
}

/// Same as [`romanized`] over an explicit script allow-list.
pub fn romanized_enabled(doc: &LyricsDocument, enabled: &[Script]) -> Option<LyricsDocument> {
    let romanization = romanize_document_enabled(doc, enabled)?;
    Some(LyricsDocument {
        lines: romanization.lines,
        ..doc.clone()
    })
}

/// The document with `translated` filled from a finished translation, keeping timings.
///
/// `translated` has one entry per line (a shorter list is padded with `""`), so both
/// tracks stay aligned with the timeline.
pub fn translated(doc: &mut LyricsDocument, translated: Vec<String>) {
    let mut translated = translated;
    translated.resize(doc.lines.len(), String::new());
    translated.truncate(doc.lines.len());
    doc.translated = Some(
        doc.lines
            .iter()
            .zip(translated)
            .map(|(line, text)| LyricsLine {
                text,
                ..line.clone()
            })
            .collect(),
    );
}
