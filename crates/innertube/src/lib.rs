//! `ml-innertube` — YouTube Music InnerTube client.
//!
//! Transport, contexts and request envelopes live in [`context`]; page shapes in
//! [`parse`] + [`pages`]; stream URL selection/caching in [`player`] +
//! [`stream`]. Browse/search use WEB_REMIX; playback URLs use VISIONOS, the measured
//! no-POT full-track path (`arch/pot-notes.md` §1). The [`pot`] seam defaults to
//! [`pot::NoPot`]; tokens are not wired by default.

pub mod auth;
pub mod context;
pub mod cookies;
pub mod error;
pub mod pages;
pub mod parse;
pub mod player;
pub mod pot;
pub mod stream;

use std::collections::HashMap;
use std::fmt;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ml_core::{
    Album, AlbumPage, ArtistPage, AudioQuality, Episode, HomeFeed, MediaInfo, MlError, MlResult,
    PlaylistPage, ProxyConfig, SearchPage, StreamInfo,
};
use serde_json::{json, Value};

use context::{ClientKind, ClientVersions, PreparedRequest};
use pages::artist::ArtistDetails;
use pot::{NoPot, PoTokens, TokenProvider};
use stream::StreamCache;

/// Cache and observability knobs shared by all requests.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// `hl` in the InnerTube context, e.g. `"en"`.
    pub language: String,
    /// `gl` in the InnerTube context, e.g. `"US"`.
    pub country: String,
    /// Optional visitor id; not required for browse/player per the probe.
    pub visitor_data: Option<String>,
    /// Raw `Cookie:` header for authenticated library requests (SAPISID …),
    /// either `name=value; …` or a Netscape `cookies.txt` blob.
    pub cookie: Option<String>,
    /// `context.user.onBehalfOfUser` (`dataSyncId` upstream); requires cookies.
    pub data_sync_id: Option<String>,
    /// `X-Goog-AuthUser` (the selected channel index).
    pub auth_user: Option<String>,
    /// Per-request timeout in seconds.
    pub timeout_secs: u64,
    /// Optional HTTP proxy.
    pub proxy: Option<ProxyConfig>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            language: "en".to_string(),
            country: "US".to_string(),
            visitor_data: None,
            cookie: None,
            data_sync_id: None,
            auth_user: None,
            timeout_secs: 30,
            proxy: None,
        }
    }
}

/// Search filter `params` values copied from the measured WEB_REMIX requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFilter {
    Songs,
    Albums,
    Artists,
    Playlists,
    Videos,
}

impl SearchFilter {
    pub fn params(self) -> &'static str {
        match self {
            Self::Songs => "EgWKAQIIAWoMEA4QChADEAQQCRAF",
            Self::Albums => "EgWKAQIYAWoMEA4QChADEAQQCRAF",
            Self::Artists => "EgWKAQIgAWoMEA4QChADEAQQCRAF",
            Self::Playlists => "EgWKAQIoAWoMEA4QChADEAQQCRAF",
            Self::Videos => "EgWKAQIQAWoMEA4QChADEAQQCRAF",
        }
    }
}

/// `search` request body. Kept free-standing so the query text and filter
/// `params` mapping are unit-testable without a network round-trip.
pub(crate) fn search_body(query: &str, filter: Option<SearchFilter>) -> Value {
    let mut body = json!({ "query": query });
    if let Some(filter) = filter {
        body["params"] = Value::String(filter.params().to_string());
    }
    body
}

/// `FEmusic_charts` params copied from the upstream client (country menu preset).
const CHART_PARAMS: &str = "ggMGCgQIgAQ%3D";
/// Every mood/genre button browses this id with its own `params`.
const MOOD_CATEGORY_BROWSE_ID: &str = "FEmusic_moods_and_genres_category";

/// InnerTube client facade. Cloning shares the HTTP client and caches.
#[derive(Clone)]
pub struct InnertubeClient {
    config: ClientConfig,
    http: reqwest::Client,
    versions: Arc<Mutex<ClientVersions>>,
    streams: Arc<StreamCache>,
    /// Album context per continuation token, so paginated rows keep the album
    /// artist fallback and `OLAK`/upload performer rules.
    album_contexts: Arc<Mutex<HashMap<String, Album>>>,
    tokens: Option<Arc<dyn TokenProvider>>,
    /// Live cookie state. Shared across clones so a rotation captured by one
    /// request is sent by the next one on every clone.
    cookies: Arc<Mutex<crate::cookies::CookieJar>>,
    /// When set, the jar is written back (0600) after every `Set-Cookie` merge.
    session_path: Arc<Mutex<Option<PathBuf>>>,
}

impl fmt::Debug for InnertubeClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `ClientConfig` carries the cookie header; never render it.
        f.debug_struct("InnertubeClient")
            .field("language", &self.config.language)
            .field("country", &self.config.country)
            .field("authenticated", &self.is_authenticated())
            .field("token_provider", &self.tokens.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for InnertubeClient {
    fn default() -> Self {
        Self::new(ClientConfig::default())
    }
}

impl InnertubeClient {
    pub fn new(config: ClientConfig) -> Self {
        let http = context::build_http_client(&config);
        let cookies = crate::cookies::CookieJar::parse(config.cookie.as_deref().unwrap_or(""));
        Self {
            config,
            http,
            versions: Arc::new(Mutex::new(ClientVersions::default())),
            streams: Arc::new(StreamCache::default()),
            album_contexts: Arc::new(Mutex::new(HashMap::new())),
            tokens: Some(Arc::new(NoPot)),
            cookies: Arc::new(Mutex::new(cookies)),
            session_path: Arc::new(Mutex::new(None)),
        }
    }

    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Swap credentials without rebuilding the shared HTTP client. The header
    /// seeds the live jar, so rotations continue from it.
    pub fn set_cookie(&mut self, cookie: Option<String>) {
        let jar = crate::cookies::CookieJar::parse(cookie.as_deref().unwrap_or(""));
        self.replace_cookie_jar(jar);
        self.config.cookie = cookie;
    }

    /// Install the in-process PO-token minter.
    pub fn set_token_provider(&mut self, provider: Arc<dyn TokenProvider>) {
        self.tokens = Some(provider);
    }

    pub fn clear_token_provider(&mut self) {
        self.tokens = Some(Arc::new(NoPot));
    }

    // ---------------------------------------------------------------- transport

    fn versions_snapshot(&self) -> ClientVersions {
        self.versions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    async fn ensure_versions(&self) -> ClientVersions {
        let current = self.versions_snapshot();
        if !current.is_stale() {
            return current;
        }
        match context::scrape_versions(&self.http).await {
            Ok(fresh) => {
                *self
                    .versions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = fresh.clone();
                fresh
            }
            Err(_) => current,
        }
    }

    /// POST an InnerTube endpoint. A 404 means a stale client version: refresh
    /// `ytcfg` once and retry the same request.
    async fn post(
        &self,
        kind: ClientKind,
        endpoint: &str,
        body: Value,
        set_login: bool,
        integrity: Option<&PoTokens>,
    ) -> MlResult<Value> {
        let mut versions = self.ensure_versions().await;
        let mut retried = false;
        loop {
            // The live jar owns the cookie header; `config.cookie` is only its
            // seed. A cloned config keeps `prepare` the single header builder.
            let mut config = self.config.clone();
            config.cookie = self.cookie_header();
            let request_versions = crate::auth::request_versions(&versions, set_login);
            let prepared = context::prepare(
                &config,
                &request_versions,
                kind,
                endpoint,
                body.clone(),
                set_login,
                integrity,
            );
            let response = self.send(&prepared).await?;
            self.absorb_response_cookies(
                response.headers(),
                kind.host(),
                &format!("/youtubei/v1/{endpoint}"),
            );
            let status = response.status();
            if status.is_success() {
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| MlError::Network(error.to_string()))?;
                return serde_json::from_slice(&bytes)
                    .map_err(|error| MlError::Parse(format!("InnerTube {endpoint}: {error}")));
            }
            let code = status.as_u16();
            let text = response.text().await.unwrap_or_default();
            if code == 404 && !retried {
                retried = true;
                if let Ok(fresh) = context::scrape_versions(&self.http).await {
                    versions = fresh.clone();
                    *self
                        .versions
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = fresh;
                }
                continue;
            }
            return Err(error::api_error(code, &text));
        }
    }

    async fn send(&self, prepared: &PreparedRequest) -> MlResult<reqwest::Response> {
        let mut request = self.http.post(&prepared.url);
        for (name, value) in &prepared.headers {
            request = request.header(name, value);
        }
        request
            .json(&prepared.body)
            .send()
            .await
            .map_err(|error| MlError::Network(error.to_string()))
    }

    /// Refresh the WEB_REMIX/IOS client versions and API key from `ytcfg`.
    pub async fn refresh_client_config(&mut self) -> MlResult<()> {
        let fresh = context::scrape_versions(&self.http).await?;
        *self
            .versions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = fresh;
        Ok(())
    }

    // ------------------------------------------------------------------- browse

    pub async fn home(&self) -> MlResult<HomeFeed> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "browseId": "FEmusic_home" }),
                false,
                None,
            )
            .await?;
        Ok(pages::home::from_browse(&response))
    }

    pub async fn home_continuation(&self, token: &str) -> MlResult<HomeFeed> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "continuation": token }),
                false,
                None,
            )
            .await?;
        Ok(pages::home::from_continuation(&response))
    }

    pub async fn search(&self, query: &str, filter: Option<SearchFilter>) -> MlResult<SearchPage> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "search",
                search_body(query, filter),
                false,
                None,
            )
            .await?;
        Ok(pages::search::from_response(&response, query))
    }

    pub async fn search_continuation(&self, token: &str) -> MlResult<SearchPage> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "search",
                json!({ "continuation": token }),
                false,
                None,
            )
            .await?;
        Ok(pages::search::from_continuation(&response))
    }

    pub async fn suggestions(&self, query: &str) -> MlResult<Vec<String>> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "music/get_search_suggestions",
                json!({ "input": query }),
                false,
                None,
            )
            .await?;
        Ok(pages::search::suggestions(&response))
    }

    pub async fn album(&self, browse_id: &str) -> MlResult<AlbumPage> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "browseId": browse_id }),
                false,
                None,
            )
            .await?;
        let mut page = pages::album::from_browse(&response, browse_id);

        if !page.album.playlist_id.is_empty() {
            let playlist_browse = format!("VL{}", page.album.playlist_id);
            if let Ok(tracks_response) = self
                .post(
                    ClientKind::WebRemix,
                    "browse",
                    json!({ "browseId": playlist_browse }),
                    false,
                    None,
                )
                .await
            {
                let (songs, continuation) =
                    pages::album::tracks_from_browse(&tracks_response, Some(&page.album));
                if !songs.is_empty() {
                    page.songs = songs;
                    page.continuation = continuation;
                }
            }
        }

        // If every track credits one performer but the header credits a label,
        // surface the performer instead (`YouTube.album` rule).
        if let Some(first) = page
            .songs
            .first()
            .and_then(|song| song.artists.first())
            .filter(|artist| !artist.name.is_empty())
        {
            if page
                .songs
                .iter()
                .all(|song| song.artists.first().map(|a| &a.name) == Some(&first.name))
                && !page
                    .album
                    .artists
                    .iter()
                    .any(|artist| artist.name == first.name)
            {
                page.album.artists = vec![first.clone()];
            }
        }

        if let Some(token) = page.continuation.clone() {
            self.album_contexts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(token, page.album.clone());
        }
        Ok(page)
    }

    pub async fn album_continuation(&self, token: &str) -> MlResult<AlbumPage> {
        let album = self
            .album_contexts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(token)
            .cloned();
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "continuation": token }),
                false,
                None,
            )
            .await?;
        let (songs, continuation) =
            pages::album::tracks_from_continuation(&response, album.as_ref());
        if let (Some(album), Some(next)) = (album.as_ref(), continuation.clone()) {
            self.album_contexts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(next, album.clone());
        }
        Ok(AlbumPage {
            album: album.unwrap_or_default(),
            songs,
            description: None,
            continuation,
        })
    }

    pub async fn artist(&self, channel_id: &str) -> MlResult<ArtistPage> {
        Ok(self.artist_details(channel_id).await?.page)
    }

    /// Artist page plus monthly listeners / see-all endpoints absent from the
    /// frozen core model.
    pub async fn artist_details(&self, channel_id: &str) -> MlResult<ArtistDetails> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "browseId": channel_id }),
                false,
                None,
            )
            .await?;
        Ok(pages::artist::from_browse(&response, channel_id))
    }

    pub async fn artist_continuation(&self, token: &str) -> MlResult<HomeFeed> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "continuation": token }),
                false,
                None,
            )
            .await?;
        Ok(pages::artist::from_continuation(&response))
    }

    /// Artist "see all" (`browseId` + `params` from a carousel header).
    pub async fn artist_items(&self, browse_id: &str, params: Option<&str>) -> MlResult<HomeFeed> {
        let mut body = json!({ "browseId": browse_id });
        if let Some(params) = params {
            body["params"] = Value::String(params.to_string());
        }
        let response = self
            .post(ClientKind::WebRemix, "browse", body, false, None)
            .await?;
        Ok(pages::artist::items_from_browse(&response))
    }

    // ------------------------------------------------------------- catalog pages

    /// Any `browse {browseId, params}` page: mood/genre categories, the YouTube
    /// browse screen, library lists. Sections reuse Home's shelf parser.
    pub async fn browse(
        &self,
        browse_id: &str,
        params: Option<&str>,
    ) -> MlResult<pages::browse::BrowsePage> {
        Ok(pages::browse::from_browse(
            &self.browse_json(browse_id, params).await?,
        ))
    }

    async fn browse_json(&self, browse_id: &str, params: Option<&str>) -> MlResult<Value> {
        let mut body = json!({ "browseId": browse_id });
        if let Some(params) = params {
            body["params"] = Value::String(params.to_string());
        }
        self.post(ClientKind::WebRemix, "browse", body, false, None)
            .await
    }

    /// More shelves of a generic browse page (`page.continuation`).
    pub async fn browse_continuation(&self, token: &str) -> MlResult<pages::browse::BrowsePage> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "continuation": token }),
                false,
                None,
            )
            .await?;
        Ok(pages::browse::from_continuation(&response))
    }

    /// `FEmusic_charts`: country-filter shelf plus "Video charts" and
    /// "Top artists" carousels. Follow `page.continuation` with
    /// [`Self::browse_continuation`].
    pub async fn charts(&self) -> MlResult<pages::browse::BrowsePage> {
        self.browse("FEmusic_charts", Some(CHART_PARAMS)).await
    }

    /// `FEmusic_moods_and_genres`: category buttons carrying browse params.
    pub async fn moods_and_genres(&self) -> MlResult<pages::moods::MoodsAndGenres> {
        let response = self.browse_json("FEmusic_moods_and_genres", None).await?;
        Ok(pages::moods::from_browse(&response))
    }

    /// One mood/genre category: the landed button's `params`.
    pub async fn mood_genre_page(
        &self,
        params: Option<&str>,
    ) -> MlResult<pages::browse::BrowsePage> {
        self.browse(MOOD_CATEGORY_BROWSE_ID, params).await
    }

    /// `FEmusic_new_releases`: "Albums & singles" + "Music videos" carousels.
    pub async fn new_releases(&self) -> MlResult<pages::browse::BrowsePage> {
        self.browse("FEmusic_new_releases", None).await
    }

    /// Flat album grid of `FEmusic_new_releases_albums` (upstream NewRelease screen).
    pub async fn new_release_albums(&self) -> MlResult<Vec<Album>> {
        let page = self.browse("FEmusic_new_releases_albums", None).await?;
        Ok(pages::browse::albums(page))
    }

    /// Podcast landing: Home reloaded through its "Podcasts" chip, which answers
    /// with episode shelves (`FEmusic_podcasts` is a 404).
    pub async fn podcasts(&self) -> MlResult<HomeFeed> {
        let home = self.browse_json("FEmusic_home", None).await?;
        let Some(params) = pages::home::chip_params(&home, "podcast") else {
            return Ok(HomeFeed::default());
        };
        Ok(pages::home::from_browse(
            &self.browse_json("FEmusic_home", Some(&params)).await?,
        ))
    }

    /// Subscribed podcast shows (`FEmusic_library_non_music_audio_channels_list`).
    /// Needs a cookie session; anonymous answers with a sign-in notice.
    pub async fn podcast_channels(&self) -> MlResult<pages::browse::BrowsePage> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "browseId": "FEmusic_library_non_music_audio_channels_list" }),
                true,
                None,
            )
            .await?;
        Ok(pages::browse::from_browse(&response))
    }

    /// Podcast show page (`MPSP…`): header, first 20 episodes, continuation.
    pub async fn podcast(&self, show_id: &str) -> MlResult<pages::podcast::PodcastPage> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "browseId": show_id }),
                true,
                None,
            )
            .await?;
        Ok(pages::podcast::from_show_browse(&response, show_id))
    }

    /// Later episodes of a show; the show header is not repeated.
    pub async fn podcast_continuation(
        &self,
        token: &str,
    ) -> MlResult<(Vec<Episode>, Option<String>)> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "continuation": token }),
                false,
                None,
            )
            .await?;
        Ok(pages::podcast::from_show_continuation(&response))
    }

    /// Fetch and append the next radio/queue page in place (dedup by video id,
    /// order and `current_index` preserved). Returns how many songs were added.
    pub async fn queue_append(&self, page: &mut ml_core::QueuePage) -> MlResult<usize> {
        let Some(token) = page.continuation.clone() else {
            return Ok(0);
        };
        let next = self.queue_continuation(&token).await?;
        Ok(pages::next::append(page, next))
    }

    pub async fn playlist(&self, playlist_id: &str) -> MlResult<PlaylistPage> {
        let id = playlist_id.trim_start_matches("VL");
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "browseId": format!("VL{id}") }),
                true,
                None,
            )
            .await?;
        Ok(pages::playlist::from_browse(&response, id))
    }

    pub async fn playlist_continuation(&self, token: &str) -> MlResult<PlaylistPage> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "continuation": token }),
                true,
                None,
            )
            .await?;
        Ok(pages::playlist::from_continuation(&response))
    }

    /// Build the `next` radio/album/playlist queue.
    pub async fn queue(
        &self,
        video_id: &str,
        playlist_id: Option<&str>,
    ) -> MlResult<ml_core::QueuePage> {
        let radio = playlist_id.is_none();
        let playlist_id = playlist_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("RDAMVM{video_id}"));
        let mut body = json!({
            "videoId": video_id,
            "playlistId": playlist_id,
            "isAudioOnly": true,
        });
        if radio {
            body["params"] = Value::String("wAEB".to_string());
        }
        let response = self
            .post(ClientKind::WebRemix, "next", body, false, None)
            .await?;
        Ok(pages::next::from_response(&response))
    }

    pub async fn queue_continuation(&self, token: &str) -> MlResult<ml_core::QueuePage> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "next",
                json!({ "continuation": token }),
                false,
                None,
            )
            .await?;
        Ok(pages::next::from_continuation(&response))
    }

    // ------------------------------------------------------------------- player

    fn now_epoch_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0)
    }

    async fn mint_tokens(&self, video_id: &str) -> MlResult<Option<PoTokens>> {
        match self.tokens.as_ref() {
            Some(provider) => provider.po_tokens(video_id).await,
            None => Ok(None),
        }
    }

    /// Player request for one client kind.
    async fn fetch_player_with(
        &self,
        video_id: &str,
        kind: ClientKind,
    ) -> MlResult<(player::PlayerData, Option<PoTokens>)> {
        let tokens = self.mint_tokens(video_id).await?;
        let set_login = self.config.cookie.is_some();
        let value = self
            .post(
                kind,
                "player",
                player::player_body(video_id),
                set_login,
                tokens.as_ref(),
            )
            .await?;
        Ok((player::parse_player(&value)?, tokens))
    }

    /// Player metadata request: VISIONOS primary, IOS fallback (IOS alone caps
    /// at 512 KiB/1 MiB, but metadata does not download bytes).
    async fn fetch_player(
        &self,
        video_id: &str,
    ) -> MlResult<(player::PlayerData, Option<PoTokens>)> {
        let mut last_error = None;
        for kind in [ClientKind::VisionOs, ClientKind::Ios] {
            match self.fetch_player_with(video_id, kind).await {
                Ok(result) => return Ok(result),
                Err(error) => {
                    let retryable = !matches!(error, MlError::Network(_) | MlError::Http { .. });
                    last_error = Some(error);
                    if !retryable {
                        break;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| MlError::Unavailable("player failed".to_string())))
    }

    pub async fn media_info(&self, video_id: &str) -> MlResult<MediaInfo> {
        let (data, _) = self.fetch_player(video_id).await?;
        Ok(data.media)
    }

    /// Resolve one audio format. VISIONOS is the measured POT-less path for
    /// full tracks (`arch/pot-notes.md` §1); IOS is only an explicit fallback
    /// and hard-caps at 512 KiB (251) / 1 MiB (140) before 403. Prefers itag
    /// 251 then 140 and respects [`AudioQuality`].
    pub async fn stream_info(&self, video_id: &str, quality: AudioQuality) -> MlResult<StreamInfo> {
        let now = Self::now_epoch_secs();
        if let Some(info) = stream::cached(&self.streams, video_id, quality, now) {
            return Ok(info);
        }
        let info = match self
            .stream_info_via(video_id, quality, ClientKind::VisionOs)
            .await
        {
            Ok(info) => info,
            Err(vision_error) => self
                .stream_info_via(video_id, quality, ClientKind::Ios)
                .await
                .map_err(|_| vision_error)?,
        };
        stream::store(&self.streams, video_id, quality, info.clone());
        Ok(info)
    }

    /// Mint a URL from one specific client. Not cached: the download loop uses
    /// it to re-mint behind the fallback client after a googlevideo 403.
    async fn stream_info_via(
        &self,
        video_id: &str,
        quality: AudioQuality,
        kind: ClientKind,
    ) -> MlResult<StreamInfo> {
        let (data, tokens) = self.fetch_player_with(video_id, kind).await?;
        let format = player::select_format(&data.formats, quality)
            .ok_or_else(|| MlError::Unavailable(format!("no audio format for {video_id}")))?;
        let gvs = tokens.as_ref().and_then(|tokens| tokens.gvs.as_deref());
        player::stream_info(format, data.expires_in_seconds, Self::now_epoch_secs(), gvs)
    }

    /// Drop a cached stream URL (e.g. after a UI-level 403).
    pub fn invalidate_stream(&self, video_id: &str, quality: AudioQuality) {
        stream::invalidate(&self.streams, video_id, quality);
    }

    pub fn clear_stream_cache(&self) {
        stream::clear(&self.streams);
    }

    pub fn cached_stream(&self, video_id: &str, quality: AudioQuality) -> Option<StreamInfo> {
        stream::cached(&self.streams, video_id, quality, Self::now_epoch_secs())
    }

    /// Download a whole track with ≤512 KiB ranged GETs. A googlevideo auth
    /// failure switches to the fallback client and restarts from byte 0.
    /// `progress(done, total)` runs per chunk; `total` is 0 while unknown.
    pub async fn download_audio(
        &self,
        video_id: &str,
        quality: AudioQuality,
        path: impl AsRef<Path>,
        mut progress: impl FnMut(u64, u64) + Send,
    ) -> MlResult<()> {
        let mut info = self.stream_info(video_id, quality).await?;
        let mut total = info.content_length.unwrap_or(0);
        let mut file = std::fs::File::create(path).map_err(MlError::from)?;
        progress(0, total);
        let mut written = 0u64;
        // Primary is VISIONOS (measured POT-less full track); IOS is the
        // explicit fallback, hard-capped at 512 KiB (251) / 1 MiB (140).
        let order = [ClientKind::VisionOs, ClientKind::Ios];
        let mut client = 0usize;
        let mut reminted = false;

        while total == 0 || written < total {
            let requested_end = written.saturating_add(stream::RANGE_CHUNK - 1);
            let range_end = if total > 0 {
                requested_end.min(total - 1)
            } else {
                requested_end
            };
            let mut response =
                stream::send_range(&self.http, &info.url, written, range_end).await?;
            let status = response.status().as_u16();

            if stream::is_auth_failure(status) {
                // One fresh player URL for this client, then the other client,
                // then a typed error (pot-notes §9).
                if !reminted {
                    reminted = true;
                    if let Some(provider) = self.tokens.as_ref() {
                        provider.invalidate(video_id);
                    }
                    info = self
                        .stream_info_via(video_id, quality, order[client])
                        .await?;
                    stream::store(&self.streams, video_id, quality, info.clone());
                    file.set_len(0).map_err(MlError::from)?;
                    file.seek(SeekFrom::Start(0)).map_err(MlError::from)?;
                    written = 0;
                    total = info.content_length.unwrap_or(0);
                    progress(0, total);
                    continue;
                }
                if client + 1 >= order.len() {
                    return Err(error::http_error(
                        status,
                        format!(
                            "googlevideo rejected {video_id} with every client at byte {written} (no POT available)"
                        ),
                    ));
                }
                client += 1;
                reminted = false;
                if let Some(provider) = self.tokens.as_ref() {
                    provider.invalidate(video_id);
                }
                // The fallback mints a different URL; restart to keep the file
                // consistent with one client's byte stream.
                info = self
                    .stream_info_via(video_id, quality, order[client])
                    .await?;
                stream::store(&self.streams, video_id, quality, info.clone());
                file.set_len(0).map_err(MlError::from)?;
                file.seek(SeekFrom::Start(0)).map_err(MlError::from)?;
                written = 0;
                total = info.content_length.unwrap_or(0);
                progress(0, total);
                continue;
            }

            if status == 416 {
                let header = response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                if let Some(parsed) = header
                    .as_deref()
                    .and_then(stream::parse_content_range_total)
                {
                    total = parsed;
                }
                if total > 0 && written >= total {
                    break;
                }
                return Err(error::http_error(416, "googlevideo rejected the range"));
            }
            // Ranged playback requires 206; a 200 would be an unbounded body.
            if status != 206 {
                return Err(error::http_error(
                    status,
                    format!("googlevideo did not answer 206 at byte {written}"),
                ));
            }

            let content_range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
                .ok_or_else(|| error::http_error(206, "206 response carried no Content-Range"))?;
            let (start, end, range_total) = stream::parse_content_range(&content_range)
                .ok_or_else(|| {
                    error::http_error(206, format!("malformed Content-Range: {content_range}"))
                })?;
            if start != written || end > range_end {
                return Err(error::http_error(
                    206,
                    format!(
                        "unexpected Content-Range {content_range} for bytes {written}-{range_end}"
                    ),
                ));
            }
            if let Some(player_total) = info.content_length {
                if range_total.is_some_and(|total| total != player_total) {
                    return Err(error::http_error(
                        206,
                        format!(
                            "Content-Range total {range_total:?} disagrees with contentLength {player_total}"
                        ),
                    ));
                }
            }
            if let Some(parsed_total) = range_total {
                total = parsed_total;
            }
            let expected = end - start + 1;
            if let Some(length) = response.content_length() {
                if length != expected {
                    return Err(error::http_error(
                        206,
                        format!("Content-Length {length} != range length {expected}"),
                    ));
                }
            }

            let mut received = 0u64;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| MlError::Network(error.to_string()))?
            {
                file.write_all(&chunk).map_err(MlError::from)?;
                written = written.saturating_add(chunk.len() as u64);
                received = received.saturating_add(chunk.len() as u64);
                progress(written, total.max(written));
            }
            if received != expected {
                return Err(MlError::Unavailable(format!(
                    "206 body ended after {received}/{expected} bytes at offset {start}"
                )));
            }
            // Unknown total: a short body is the end of the resource.
            if total == 0 && received < stream::RANGE_CHUNK {
                break;
            }
        }

        file.flush().map_err(MlError::from)?;
        if total > 0 && written < total {
            return Err(MlError::Unavailable(format!(
                "download stopped at {written}/{total} bytes"
            )));
        }
        Ok(())
    }

    // --------------------------------------------------------------- playback

    /// Fetch a fresh player response and register a playback start (`cpn`).
    /// Tracking URLs are allowlisted to `s.youtube.com` before the GET.
    pub async fn register_playback(&self, video_id: &str, cpn: &str) -> MlResult<()> {
        let (data, _) = self.fetch_player(video_id).await?;
        let tracking = data
            .playback_url
            .ok_or_else(|| MlError::Unavailable("no playback tracking URL".to_string()))?;
        let url = player::validate_tracking_url(&tracking)?;
        let url = {
            let mut url = url;
            url.query_pairs_mut().append_pair("cpn", cpn);
            url
        };
        let mut request = self.http.get(url);
        if let Some(cookie) = self.config.cookie.as_deref() {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        let response = request
            .send()
            .await
            .map_err(|error| MlError::Network(error.to_string()))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(error::http_error(
                response.status().as_u16(),
                "playback registration failed",
            ))
        }
    }

    /// Alias kept for callers that only need the validated tracking URL.
    pub async fn playback_tracking_url(&self, video_id: &str) -> MlResult<String> {
        let (data, _) = self.fetch_player(video_id).await?;
        let url = data
            .playback_url
            .ok_or_else(|| MlError::Unavailable("no playback tracking URL".to_string()))?;
        player::validate_tracking_url(&url)?;
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_body_preserves_literal_query_text() {
        for query in [
            "Shape of You",
            "Björk – Jóga",
            "\"don't stop\" me now!",
            "AC/DC & friends?",
            "%2F literal",
        ] {
            assert_eq!(search_body(query, None)["query"], query);
        }
    }

    #[test]
    fn every_filter_maps_to_its_measured_params() {
        let filters = [
            (SearchFilter::Songs, "EgWKAQIIAWoMEA4QChADEAQQCRAF"),
            (SearchFilter::Albums, "EgWKAQIYAWoMEA4QChADEAQQCRAF"),
            (SearchFilter::Artists, "EgWKAQIgAWoMEA4QChADEAQQCRAF"),
            (SearchFilter::Playlists, "EgWKAQIoAWoMEA4QChADEAQQCRAF"),
            (SearchFilter::Videos, "EgWKAQIQAWoMEA4QChADEAQQCRAF"),
        ];
        for (filter, params) in filters {
            assert_eq!(search_body("Shape of You", Some(filter))["params"], params);
        }
        assert!(search_body("Shape of You", None).get("params").is_none());
    }
}
