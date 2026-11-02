//! LRCLIB (primary, keyless) and KuGou (opt-in). Other provider ids are honest
//! `Unavailable`/`AuthRequired` answers — no undocumented endpoints, no API keys.
//!
//! All HTTP goes through [`HttpTransport`] so the match ladder is tested with fixtures.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ml_core::{LyricsDocument, MlError, MlResult};
use serde::Deserialize;

use crate::lrc::{decode_kugou, parse_lrc_with_meta};
use crate::LyricsQuery;

/// User-Agent identifying the port; LRCLIB asks clients to send one.
pub const USER_AGENT: &str = concat!(
    "metrolist-linux/",
    env!("CARGO_PKG_VERSION"),
    " (+https://example.invalid/metrolist-linux)"
);

/// Exact duration tolerance, seconds.
pub const DURATION_TOLERANCE_S: u32 = 2;
/// Relaxed duration tolerance, seconds.
pub const DURATION_TOLERANCE_RELAXED_S: u32 = 5;
/// KuGou's own tolerance is wider than LRCLIB's.
pub const KUGOU_TOLERANCE_S: u32 = 8;

const LRCLIB_BASE: &str = "https://lrclib.net";
const KUGOU_SEARCH: &str = "https://lyrics.kugou.com/search";
const KUGOU_DOWNLOAD: &str = "https://lyrics.kugou.com/download";

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Minimal HTTP surface; the real one wraps `reqwest`, tests return fixtures.
///
/// `post` is only used by translation; its default answers [`MlError::Unavailable`] so a
/// GET-only test transport cannot silently reach the network.
pub trait HttpTransport: Send + Sync {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, MlResult<String>>;

    fn post<'a>(
        &'a self,
        url: &'a str,
        _headers: Vec<(String, String)>,
        _body: String,
    ) -> BoxFuture<'a, MlResult<String>> {
        let _ = url;
        Box::pin(async move {
            Err(MlError::Unavailable(
                "transport does not support POST (translation)".into(),
            ))
        })
    }
}

/// `reqwest`-backed transport with the timeout every provider call uses.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub fn with_defaults() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

impl HttpTransport for ReqwestTransport {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, MlResult<String>> {
        Box::pin(async move {
            let response = self
                .client
                .get(url)
                .send()
                .await
                .map_err(|e| MlError::Network(e.to_string()))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|e| MlError::Network(e.to_string()))?;
            if !status.is_success() {
                return Err(MlError::Http {
                    status: status.as_u16(),
                    message: body.chars().take(200).collect(),
                });
            }
            Ok(body)
        })
    }

    fn post<'a>(
        &'a self,
        url: &'a str,
        headers: Vec<(String, String)>,
        body: String,
    ) -> BoxFuture<'a, MlResult<String>> {
        Box::pin(async move {
            let mut request = self
                .client
                .post(url)
                .header("Content-Type", "application/json")
                .body(body);
            for (name, value) in headers {
                request = request.header(name, value);
            }
            let response = request
                .send()
                .await
                .map_err(|e| MlError::Network(e.to_string()))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|e| MlError::Network(e.to_string()))?;
            if !status.is_success() {
                return Err(MlError::Http {
                    status: status.as_u16(),
                    message: body.chars().take(200).collect(),
                });
            }
            Ok(body)
        })
    }
}

/// One LRCLIB search hit. Unknown fields are ignored, missing ones default.
#[derive(Debug, Clone, Default, Deserialize)]
struct LrcLibTrack {
    #[serde(default, rename = "trackName")]
    track_name: String,
    #[serde(default, rename = "artistName")]
    artist_name: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default, rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(default, rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(default)]
    instrumental: bool,
}

impl LrcLibTrack {
    /// Synced LRC when present, else plain text.
    fn lyrics(&self) -> Option<&str> {
        self.synced_lyrics
            .as_deref()
            .or(self.plain_lyrics.as_deref())
            .filter(|t| !t.trim().is_empty())
    }
}

/// Fetch from LRCLIB: the five-strategy ladder, then duration matching.
pub async fn fetch_lrclib(
    http: &dyn HttpTransport,
    query: &LyricsQuery,
) -> MlResult<LyricsDocument> {
    if query.title.trim().is_empty() {
        return Err(MlError::Lyrics("lrclib: empty title".into()));
    }
    let title = clean_title(&query.title);
    let artist = clean_artist(&query.artist);

    // A failed strategy is skipped, not fatal: LRCLIB answers 503 under load and the
    // next strategy may still succeed. The first error is kept for the final report.
    let mut tracks = Vec::new();
    let mut first_error: Option<MlError> = None;
    for url in ladder_urls(&title, &artist, query) {
        match search_lrclib(http, &url).await {
            Ok(found) => {
                tracks = found;
                tracks.retain(|t| t.lyrics().is_some() && !t.instrumental);
                if !tracks.is_empty() {
                    break;
                }
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if tracks.is_empty() {
        if let Some(error) = first_error {
            return Err(error);
        }
        return Err(MlError::NotFound(format!(
            "lrclib: no lyrics for {} — {}",
            query.title, query.artist
        )));
    }

    let track = pick_track(&tracks, query.duration, &title, &artist).ok_or_else(|| {
        MlError::NotFound(format!("lrclib: no duration match for {}", query.title))
    })?;
    to_document("lrclib", track)
}

/// The five search strategies, in the order the app tries them.
fn ladder_urls(title: &str, artist: &str, query: &LyricsQuery) -> Vec<String> {
    let mut urls = vec![
        search_url(&[
            ("track_name", title),
            ("artist_name", artist),
            ("album_name", query.album.as_deref().unwrap_or_default()),
        ]),
        search_url(&[("track_name", title)]),
        search_url(&[("q", &format!("{artist} {title}"))]),
        search_url(&[("q", title)]),
    ];
    let original = query.title.trim();
    if !original.is_empty() && original != title {
        urls.push(search_url(&[
            ("track_name", original),
            ("artist_name", query.artist.trim()),
        ]));
    }
    urls
}

fn search_url(params: &[(&str, &str)]) -> String {
    query_url(&format!("{LRCLIB_BASE}/api/search"), params)
}

/// Percent-encode `params` onto `base`; empty values are dropped. Infallible.
fn query_url(base: &str, params: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in params {
        if !value.is_empty() {
            serializer.append_pair(key, value);
        }
    }
    format!("{base}?{}", serializer.finish())
}

async fn search_lrclib(http: &dyn HttpTransport, url: &str) -> MlResult<Vec<LrcLibTrack>> {
    let body = http.get(url).await?;
    serde_json::from_str(&body).map_err(|e| MlError::Parse(format!("lrclib search: {e}")))
}

/// Narrow a result list to the best duration match, preferring synced lyrics.
///
/// `None` duration means the query carried no length: fall back to name similarity.
fn pick_track<'a>(
    tracks: &'a [LrcLibTrack],
    duration: Option<u32>,
    title: &str,
    artist: &str,
) -> Option<&'a LrcLibTrack> {
    let Some(duration) = duration else {
        return tracks
            .iter()
            .max_by(|a, b| score(a, title, artist).total_cmp(&score(b, title, artist)));
    };
    let tolerance = |track: &LrcLibTrack, limit: u32| {
        track
            .duration
            .map(|d| d.round().max(0.0) as u32)
            .is_some_and(|d| d.abs_diff(duration) <= limit)
    };
    tracks
        .iter()
        .filter(|t| t.synced_lyrics.is_some() && tolerance(t, DURATION_TOLERANCE_S))
        .min_by_key(|t| duration_diff(t, duration))
        .or_else(|| {
            tracks
                .iter()
                .filter(|t| t.synced_lyrics.is_some() && tolerance(t, DURATION_TOLERANCE_RELAXED_S))
                .min_by_key(|t| duration_diff(t, duration))
        })
        .or_else(|| {
            tracks
                .iter()
                .filter(|t| tolerance(t, DURATION_TOLERANCE_RELAXED_S))
                .min_by_key(|t| duration_diff(t, duration))
        })
}

fn duration_diff(track: &LrcLibTrack, duration: u32) -> u32 {
    track
        .duration
        .map(|d| (d.round().max(0.0) as u32).abs_diff(duration))
        .unwrap_or(u32::MAX)
}

/// Title/artist similarity, used when the query has no duration.
fn score(track: &LrcLibTrack, title: &str, artist: &str) -> f64 {
    let title_similarity = similarity(title, &track.track_name);
    let artist_similarity = similarity(artist, &track.artist_name);
    let bonus = if track.synced_lyrics.is_some() {
        0.1
    } else {
        0.0
    };
    (title_similarity + artist_similarity) / 2.0 + bonus
}

fn similarity(a: &str, b: &str) -> f64 {
    let a = a.trim().to_lowercase();
    let b = b.trim().to_lowercase();
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let contains: f64 = if a.contains(&b) || b.contains(&a) {
        0.8
    } else {
        0.0
    };
    let longest = a.chars().count().max(b.chars().count());
    let distance = levenshtein(&a, &b);
    contains.max(1.0 - distance as f64 / longest as f64)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

fn to_document(provider: &str, track: &LrcLibTrack) -> MlResult<LyricsDocument> {
    let Some(lyrics) = track.lyrics() else {
        return Err(MlError::Unavailable("lrclib: track has no lyrics".into()));
    };
    if track.synced_lyrics.is_some() {
        let mut doc = parse_lrc_with_meta(lyrics)?.doc;
        doc.provider = provider.to_string();
        if doc.lines.is_empty() {
            doc.plain = Some(lyrics.to_string());
        }
        Ok(doc)
    } else {
        Ok(plain_document(provider, lyrics))
    }
}

/// A document with no timings: `plain` only, `lines` empty.
pub fn plain_document(provider: &str, text: &str) -> LyricsDocument {
    LyricsDocument {
        provider: provider.to_string(),
        lines: Vec::new(),
        plain: Some(text.to_string()),
        translated: None,
        language: None,
    }
}

/// Strip `(Official Video)`, `[Lyrics]`, `feat. X` and similar from a title, using the
/// same patterns as `LrcLib.cleanTitle`.
pub fn clean_title(title: &str) -> String {
    static PATTERNS: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            r"(?i)\s*\(.*?(official|video|audio|lyrics|lyric|visualizer|hd|hq|4k|remaster|remix|live|acoustic|version|edit|extended|radio|clean|explicit).*?\)",
            r"(?i)\s*\[.*?(official|video|audio|lyrics|lyric|visualizer|hd|hq|4k|remaster|remix|live|acoustic|version|edit|extended|radio|clean|explicit).*?\]",
            r"\s*【.*?】",
            r"\s*\|.*$",
            r"(?i)\s*-\s*(official|video|audio|lyrics|lyric|visualizer).*$",
            r"(?i)\s*\(feat\..*?\)",
            r"(?i)\s*\(ft\..*?\)",
            r"(?i)\s*feat\..*$",
            r"(?i)\s*ft\..*$",
        ]
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect()
    });
    let mut cleaned = title.trim().to_string();
    for pattern in patterns {
        cleaned = pattern.replace_all(&cleaned, "").into_owned();
    }
    cleaned.trim().to_string()
}

/// Primary artist: everything before the first collaborator separator.
pub fn clean_artist(artist: &str) -> String {
    let lower = artist.to_lowercase();
    let mut cut = artist.len();
    for separator in [
        " & ",
        " and ",
        ", ",
        " x ",
        " feat. ",
        " feat ",
        " ft. ",
        " ft ",
        " featuring ",
        " with ",
    ] {
        if let Some(pos) = lower.find(separator) {
            cut = cut.min(pos);
        }
    }
    artist[..cut].trim().to_string()
}

/// One KuGou search hit.
#[derive(Debug, Clone, Default, Deserialize)]
struct KuGouCandidate {
    #[serde(default)]
    id: String,
    #[serde(default)]
    accesskey: String,
    #[serde(default)]
    duration: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct KuGouSearchResponse {
    #[serde(default)]
    candidates: Vec<KuGouCandidate>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct KuGouDownloadResponse {
    #[serde(default)]
    content: String,
}

/// Fetch from KuGou (off by default). Base64 LRC download, widened tolerance.
pub async fn fetch_kugou(
    http: &dyn HttpTransport,
    query: &LyricsQuery,
) -> MlResult<LyricsDocument> {
    if query.title.trim().is_empty() || query.artist.trim().is_empty() {
        return Err(MlError::Lyrics("kugou: title and artist required".into()));
    }
    // KuGou's keyword search only matches `Artist - Title` (verified against the live
    // API: `Title - Artist` returns zero candidates).
    let keyword = format!(
        "{} - {}{}",
        clean_artist(&query.artist),
        clean_title(&query.title),
        query
            .album
            .as_deref()
            .map(|a| format!(" {a}"))
            .unwrap_or_default()
    );
    let duration = query.duration.map(|d| (d * 1000).to_string());
    let url = query_url(
        KUGOU_SEARCH,
        &[
            ("ver", "1"),
            ("man", "yes"),
            ("client", "pc"),
            ("keyword", &keyword),
            ("duration", duration.as_deref().unwrap_or_default()),
        ],
    );

    let body = http.get(&url).await?;
    let search: KuGouSearchResponse =
        serde_json::from_str(&body).map_err(|e| MlError::Parse(format!("kugou search: {e}")))?;

    let candidate = search
        .candidates
        .iter()
        .filter(|c| !c.id.is_empty() && !c.accesskey.is_empty())
        .filter(|c| match (c.duration, query.duration) {
            (Some(ms), Some(want)) => {
                (ms / 1000).abs_diff(u64::from(want)) <= u64::from(KUGOU_TOLERANCE_S)
            }
            _ => true,
        })
        .min_by_key(|c| {
            c.duration
                .map(|ms| (ms / 1000).abs_diff(u64::from(query.duration.unwrap_or(0))))
                .unwrap_or(u64::MAX)
        })
        .ok_or_else(|| MlError::NotFound("kugou: no candidate".into()))?;

    let url = query_url(
        KUGOU_DOWNLOAD,
        &[
            ("fmt", "lrc"),
            ("charset", "utf8"),
            ("client", "pc"),
            ("ver", "1"),
            ("id", &candidate.id),
            ("accesskey", &candidate.accesskey),
        ],
    );
    let body = http.get(&url).await?;
    let download: KuGouDownloadResponse =
        serde_json::from_str(&body).map_err(|e| MlError::Parse(format!("kugou download: {e}")))?;
    let lrc = decode_kugou(&download.content)?;
    let mut doc = parse_lrc_with_meta(&lrc)?.doc;
    // KuGou emits its own `Title - Artist` header as a timed line; it is not sung.
    // (The *search keyword* needs `Artist - Title`, the header is the other way round.)
    let header = format!(
        "{} - {}",
        clean_title(&query.title),
        clean_artist(&query.artist)
    );
    let header = header.trim().to_lowercase();
    doc.lines
        .retain(|line| line.text.trim().to_lowercase() != header);
    crate::lrc::filter_credits(&mut doc.lines);
    doc.provider = "kugou".to_string();
    Ok(doc)
}

/// Provider id used by [`crate::LyricsClient`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    LrcLib,
    KuGou,
    /// Refuses: BetterLyrics needs an API key (HTTP 401 without one).
    BetterLyrics,
    /// Refuses: Apple-Music scraper, no documented contract.
    Paxsenix,
    /// Refuses: no documented contract.
    LyricsPlus,
    /// Refuses: Zemer proxies LRCLIB; call LRCLIB directly instead.
    Zemer,
}

impl ProviderKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::LrcLib => "lrclib",
            Self::KuGou => "kugou",
            Self::BetterLyrics => "betterlyrics",
            Self::Paxsenix => "paxsenix",
            Self::LyricsPlus => "lyricsplus",
            Self::Zemer => "zemer",
        }
    }

    /// Only LRCLIB is on by default: keyless, verified, and the app's own fallback.
    pub fn enabled_by_default(self) -> bool {
        matches!(self, Self::LrcLib)
    }
}

/// Why an unported provider refuses, without touching the network.
pub fn refusal(kind: ProviderKind) -> MlError {
    match kind {
        ProviderKind::BetterLyrics => MlError::AuthRequired,
        ProviderKind::Paxsenix => {
            MlError::Unavailable("paxsenix: Apple Music scraper, not ported".into())
        }
        ProviderKind::LyricsPlus => {
            MlError::Unavailable("lyricsplus: endpoints not documented; disabled".into())
        }
        ProviderKind::Zemer => {
            MlError::Unavailable("zemer: proxies LRCLIB; use the lrclib provider".into())
        }
        ProviderKind::LrcLib | ProviderKind::KuGou => {
            MlError::Unavailable(format!("{}: not wired", kind.name()))
        }
    }
}

/// One recorded POST: `(url, headers, body)`.
pub type RecordedRequest = (String, Vec<(String, String)>, String);

/// Shared by tests: a transport that answers from a fixed URL→body map.
///
/// URLs are matched by prefix, so one route covers a whole ladder. POST writes go to
/// [`FakeTransport::requests`], which the translation tests assert on.
#[derive(Debug, Default)]
pub struct FakeTransport {
    pub routes: Vec<(String, String)>,
    pub calls: std::sync::Mutex<Vec<String>>,
    /// `(url, headers, body)` for every POST, in order.
    pub requests: std::sync::Mutex<Vec<RecordedRequest>>,
}

impl FakeTransport {
    pub fn new(routes: impl IntoIterator<Item = (String, String)>) -> Arc<Self> {
        Arc::new(Self {
            routes: routes.into_iter().collect(),
            ..Self::default()
        })
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }

    fn answer(&self, url: &str) -> MlResult<String> {
        for (prefix, body) in &self.routes {
            if url.starts_with(prefix.as_str()) {
                return Ok(body.clone());
            }
        }
        Err(MlError::Http {
            status: 404,
            message: format!("no fixture for {url}"),
        })
    }
}

impl HttpTransport for FakeTransport {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, MlResult<String>> {
        Box::pin(async move {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(url.to_string());
            }
            self.answer(url)
        })
    }

    fn post<'a>(
        &'a self,
        url: &'a str,
        headers: Vec<(String, String)>,
        body: String,
    ) -> BoxFuture<'a, MlResult<String>> {
        Box::pin(async move {
            if let Ok(mut requests) = self.requests.lock() {
                requests.push((url.to_string(), headers, body));
            }
            self.answer(url)
        })
    }
}
