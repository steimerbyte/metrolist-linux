//! Service bridge between the egui UI thread and the backend crates.
//!
//! The UI thread owns this value. Network work runs on a two-worker tokio
//! runtime and publishes [`UiEvent`]s over a crossbeam channel; the UI drains
//! them once per frame. The playback controller lives behind a mutex because it
//! is polled from the UI thread and fed resolved URLs from async tasks.
//!
//! Failures that would otherwise abort startup (mpv missing, database locked)
//! become startup notes the app shows as toasts.

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui;
use ml_audio::{
    AudioConfig, AudioDevice, ControllerEvent, MpvEngine, PlaybackController, QueueView,
    SleepTimer, StreamResolver,
};
use ml_core::{
    now_millis, AlbumPage, AppEvent, AudioQuality, Episode, HomeFeed, LibrarySort, LyricsDocument,
    LyricsLine, LyricsSettings, MlError, MlResult, PlaylistPage, Queue, QueuePage, QueueSource,
    SearchPage, Settings, Song,
};
use ml_downloads::{Download, DownloadConfig, DownloadEvent, DownloadManager};
use ml_innertube::auth::{AccountInfo, LibrarySnapshot};
use ml_innertube::cookies::CookieJar;
use ml_innertube::pages::browse::BrowsePage;
use ml_innertube::pages::moods::MoodsAndGenres;
use ml_innertube::pages::podcast::PodcastPage;
use ml_innertube::{ClientConfig, InnertubeClient, SearchFilter};
use ml_integrations::lastfm::HttpTransport;
use ml_integrations::{LastFmClient, LastFmConfig, LastFmTransport};
use ml_lyrics::{
    LyricsClient, LyricsProvider, LyricsQuery, ReqwestTransport, TranslationClient,
    TranslationConfig, TranslationEndpoint,
};
use ml_recognition::{capture_until, signature_uri_from_pcm16le, CaptureOptions, ShazamClient};
use ml_store::{
    newest_firefox_profile, read_firefox_cookies, ArtistRecord, LibraryFilter, PlaylistRecord,
    StatsPeriod, StatsReport, Store,
};
use ml_together::{queue_from_room, PlaybackAction, RoomState, TogetherClient, TogetherEvent};
use parking_lot::Mutex;

use crate::art::ArtImage;
use crate::integrations::{discord_app_id, queue_path, DiscordStatus, LastFmStatus};
use crate::integrations_worker::{DiscordWorker, LastFmWorker};
use crate::pure::{
    account_error_message, download_badge, parse_persisted_queue, radio_should_extend,
    recognition_error_message, redact_secrets, session_file_path, translation_endpoint_valid,
    DownloadBadge, TRANSLATION_ENDPOINT_MESSAGE, TRANSLATION_MISSING_KEY_MESSAGE,
};
use crate::settings_store::SettingsPaths;
use crate::state::{
    AccountCache, AccountSession, AuthIdentity, BrowseTarget, LibraryData, LocalPlaylistView,
    RecognitionMatch, SyncCounts, RECOGNITION_SECONDS,
};

/// Events produced by background work for the UI.
#[derive(Debug)]
pub enum UiEvent {
    /// Free-form events from the frozen `ml-core` vocabulary.
    Core(AppEvent),
    /// Continuation shelves/page sections (append, do not replace).
    HomeMore(HomeFeed),
    SearchMore(SearchPage),
    AlbumMore(AlbumPage),
    ArtistMore(HomeFeed),
    PlaylistMore(PlaylistPage),
    /// Search box suggestions for `query` (stale responses are dropped).
    Suggestions {
        query: String,
        items: Vec<String>,
    },
    /// Thumbnail fetch result keyed by URL.
    Art {
        url: String,
        image: Option<Arc<ArtImage>>,
    },
    /// A stream URL was submitted to the controller; repaint.
    Resolved,
    /// One or more radio continuation tracks were appended.
    RadioExtended {
        added: usize,
    },
    /// A `next` queue (radio) is ready to become the playback queue.
    RadioReady {
        origin: String,
        songs: Vec<Song>,
        index: usize,
        source: QueueSource,
    },
    /// The stream could not be resolved (often a 403/POT cap).
    PlaybackFailed {
        video_id: String,
        message: String,
    },
    /// Full Library snapshot (songs, albums, artists, playlists, history).
    LibraryLoaded(LibraryData),
    /// Liked songs only.
    LikedLoaded(Vec<Song>),
    /// One local playlist with its songs, in order.
    LocalPlaylistLoaded(LocalPlaylistView),
    /// Output devices from `mpv --audio-device=help`.
    Devices(Result<Vec<AudioDevice>, String>),
    /// One offline download lifecycle/progress event.
    Download(DownloadEvent),
    /// A DB mutation succeeded; reload depending on what is on screen.
    LibraryDirty {
        playlist: Option<String>,
    },
    PlaylistCreated {
        id: String,
        name: String,
    },
    PlaylistDeleted {
        id: String,
    },
    /// Imported/refreshed account session.
    AccountUpdated {
        session: AccountSession,
        cache: Option<AccountCache>,
    },
    /// Account request failed. `cleared` means the bad session was removed.
    AccountError {
        message: String,
        cleared: bool,
    },
    AccountSignedOut,
    /// Library sync completed and the Store was updated.
    LibrarySynced {
        counts: SyncCounts,
        synced_at: i64,
    },
    LibrarySyncFailed(String),
    /// Charts, new releases, mood results or a raw YouTube browse page.
    BrowseLoaded {
        target: BrowseTarget,
        result: Result<BrowsePage, String>,
    },
    /// More shelves for the active browse page.
    BrowseMore {
        target: BrowseTarget,
        result: Result<BrowsePage, String>,
    },
    MoodsLoaded(Result<MoodsAndGenres, String>),
    PodcastsLoaded(Result<HomeFeed, String>),
    PodcastLoaded {
        id: String,
        result: Result<PodcastPage, String>,
    },
    /// More episodes for a show page.
    PodcastMore {
        id: String,
        result: Result<(Vec<Episode>, Option<String>), String>,
    },
    StatsLoaded {
        period: StatsPeriod,
        result: Result<StatsReport, String>,
    },
    /// The capture finished and the signature is now being submitted.
    RecognitionProcessing {
        generation: u64,
    },
    /// One recognition run finished. `generation` drops results after cancel or
    /// a newer run; audio never leaves the capture thread.
    Recognized {
        generation: u64,
        result: Result<RecognitionMatch, String>,
    },
    /// Song search for a recognised match is ready to be selected and played.
    RecognitionPlayReady {
        generation: u64,
        songs: Result<Vec<Song>, String>,
    },
    /// Translation for the loaded lyrics document finished.
    LyricsTranslated {
        generation: u64,
        video_id: String,
        result: Result<Vec<LyricsLine>, String>,
    },
    /// `auth.getMobileSession` finished; session key only.
    LastFmConnected {
        result: Result<String, String>,
    },
    /// A worker-side integration problem worth a toast (already redacted).
    IntegrationNotice {
        message: String,
        lastfm_disabled: bool,
    },
}

/// Synchronous stream cache shared with `PlaybackController`.
struct UrlCache {
    client: InnertubeClient,
    quality: Mutex<AudioQuality>,
    urls: Mutex<HashMap<String, String>>,
}

impl StreamResolver for UrlCache {
    fn resolve(&self, video_id: &str) -> Option<String> {
        if let Some(url) = self.urls.lock().get(video_id) {
            return Some(url.clone());
        }
        let quality = *self.quality.lock();
        self.client
            .cached_stream(video_id, quality)
            .map(|info| info.url)
    }
}

#[derive(Clone)]
pub(crate) struct Publisher {
    tx: Sender<UiEvent>,
    ctx: egui::Context,
}

impl Publisher {
    pub(crate) fn send(&self, event: UiEvent) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }

    pub(crate) fn repaint(&self) {
        self.ctx.request_repaint();
    }

    #[cfg(test)]
    pub(crate) fn for_test(ctx: egui::Context) -> (Self, Receiver<UiEvent>) {
        let (tx, rx) = unbounded();
        (Self { tx, ctx }, rx)
    }
}

/// Map a `next` queue page to the radio event. The track the user picked wins
/// over the panel's own `selected` row; `current_index` is only a fallback.
fn radio_event(song: Song, page: QueuePage) -> UiEvent {
    let index = page
        .songs
        .iter()
        .position(|entry| entry.id == song.id)
        .or(page.current_index.filter(|index| *index < page.songs.len()))
        .unwrap_or(0);
    UiEvent::RadioReady {
        origin: song.id,
        songs: page.songs,
        index,
        source: QueueSource::Radio,
    }
}

fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Write the normalized session file. `CookieJar::save` handles 0600; the
/// parent directory is forced to 0700 here.
fn save_session_file(path: &Path, jar: &CookieJar) -> MlResult<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)
            .map_err(|error| MlError::Io(format!("session directory: {error}")))?;
    }
    jar.save(path)
}

/// Startup load only; never performs network I/O. Missing/corrupt/empty files
/// are `None`.
fn load_session_file(path: &Path) -> Option<CookieJar> {
    if !path.is_file() {
        return None;
    }
    CookieJar::load(path).ok().filter(|jar| !jar.is_empty())
}

fn remove_session_file(path: &Path) {
    let _ = fs::remove_file(path);
}

fn session_from_client(client: &InnertubeClient, auth: &AuthCache) -> AccountSession {
    let snapshot = client.auth_snapshot();
    let expires_at = if snapshot.authenticated {
        client
            .cookie_jar()
            .soonest_expiry((now_millis() / 1000) as i64)
            .or(auth.expires_at)
    } else {
        None
    };
    AccountSession {
        signed_in: snapshot.authenticated,
        info: auth.info.clone(),
        identity: auth.identity.clone(),
        snapshot,
        synced_at: auth.synced_at,
        counts: auth.counts,
        expires_at,
    }
}

fn parse_count_text(text: Option<&str>) -> Option<i64> {
    let token = text?.split_whitespace().next()?.replace(',', "");
    // Abbreviated counts ("1.2K") stay unknown rather than becoming "12".
    if token.is_empty() || !token.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

/// Upsert one signed-in library snapshot. IDs come only from the API data.
fn apply_library_snapshot(
    store: &Store,
    snapshot: &LibrarySnapshot,
    now_ms: i64,
) -> MlResult<SyncCounts> {
    for playlist in &snapshot.playlists {
        let remote_song_count = parse_count_text(playlist.song_count_text.as_deref());
        store.upsert_playlist(&PlaylistRecord {
            id: playlist.id.clone(),
            name: playlist.title.clone(),
            browse_id: Some(playlist.id.clone()),
            author: playlist.author.as_ref().map(|artist| artist.name.clone()),
            thumbnail_url: playlist.thumbnail_url.clone(),
            song_count: remote_song_count.unwrap_or(0),
            remote_song_count,
            created_at: None,
            updated_at: None,
            is_editable: false,
            is_auto_sync: false,
            play_endpoint_params: None,
            shuffle_endpoint_params: None,
            radio_endpoint_params: None,
            bookmarked_at: Some(now_ms),
        })?;
    }
    for album in &snapshot.albums {
        store.upsert_album(album)?;
        store.set_album_in_library_at(&album.id, Some(now_ms))?;
    }
    for artist in &snapshot.artists {
        store.upsert_artist(&ArtistRecord {
            id: artist.id.clone(),
            name: artist.name.clone(),
            thumbnail_url: artist.thumbnail_url.clone(),
            channel_id: artist.channel_id.clone(),
            subscribers: artist.subscribers.clone(),
            description: None,
            bookmarked_at: Some(now_ms),
            last_update: None,
            cached_page_json: None,
        })?;
    }
    let songs: Vec<Song> = snapshot
        .liked_songs
        .iter()
        .cloned()
        .map(|mut song| {
            song.liked = Some(now_ms);
            song.in_library = Some(now_ms);
            song
        })
        .collect();
    store.upsert_songs(&songs)?;
    Ok(SyncCounts {
        songs: songs.len(),
        albums: snapshot.albums.len(),
        artists: snapshot.artists.len(),
        playlists: snapshot.playlists.len(),
    })
}

/// Single-track fallback: the clicked song becomes the whole queue.
fn radio_fallback(song: Song, source: QueueSource) -> UiEvent {
    UiEvent::RadioReady {
        origin: song.id.clone(),
        songs: vec![song],
        index: 0,
        source,
    }
}

/// Non-secret account state cached in Services.
#[derive(Debug, Default)]
struct AuthCache {
    generation: u64,
    identity: AuthIdentity,
    info: Option<AccountInfo>,
    synced_at: Option<i64>,
    counts: SyncCounts,
    expires_at: Option<i64>,
}

impl AuthCache {
    fn apply_cache(&mut self, cache: AccountCache) {
        self.info = Some(cache.info);
        self.synced_at = cache.synced_at;
        self.counts = cache.counts;
        self.expires_at = cache.expires_at.or(self.expires_at);
    }
}

/// Continuation state for the current radio queue. `generation` invalidates
/// in-flight page fetches when another queue starts.
#[derive(Debug, Default)]
struct RadioState {
    generation: u64,
    page: Option<QueuePage>,
    loading: bool,
    exhausted: bool,
}

impl RadioState {
    /// Invalidate older fetches and forget the previous page.
    fn begin(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.page = None;
        self.loading = false;
        self.exhausted = false;
        self.generation
    }

    /// Install the page returned for `generation`; stale responses are dropped.
    fn install(&mut self, generation: u64, page: QueuePage) -> bool {
        if self.generation != generation {
            return false;
        }
        self.exhausted = page.continuation.is_none();
        self.page = Some(page);
        self.loading = false;
        true
    }

    fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.page = None;
        self.loading = false;
        self.exhausted = true;
    }

    fn can_extend(&self) -> bool {
        self.page
            .as_ref()
            .is_some_and(|page| page.continuation.is_some())
            && !self.loading
            && !self.exhausted
    }
}

pub struct Services {
    rt: tokio::runtime::Runtime,
    client: InnertubeClient,
    lyrics_settings: Mutex<LyricsSettings>,
    store: Option<Arc<Store>>,
    paths: SettingsPaths,
    publisher: Publisher,
    events_rx: Receiver<UiEvent>,
    urls: Arc<UrlCache>,
    controller: Arc<Mutex<PlaybackController>>,
    engine: Option<MpvEngine>,
    crossfade_engine: Option<MpvEngine>,
    downloads: Option<DownloadManager>,
    downloads_error: Option<String>,
    download_dir: std::path::PathBuf,
    download_limit: Mutex<Option<u64>>,
    download_active_limit: Option<u64>,
    download_quality: Option<AudioQuality>,
    download_badges: Arc<Mutex<HashMap<String, DownloadBadge>>>,
    radio: Arc<Mutex<RadioState>>,
    auth_client: Arc<Mutex<InnertubeClient>>,
    auth: Arc<Mutex<AuthCache>>,
    session_path: PathBuf,
    browser_profile: Option<String>,
    /// One catalog request at a time wins; older responses are dropped.
    catalog_generation: Arc<AtomicU64>,
    stats_generation: Arc<AtomicU64>,
    recognition_client: Arc<ShazamClient>,
    recognition_generation: Arc<AtomicU64>,
    recognition_play_generation: Arc<AtomicU64>,
    recognition_stop: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    lyrics_translation_generation: Arc<AtomicU64>,
    lyrics_translation_cache: Arc<Mutex<HashMap<String, Vec<LyricsLine>>>>,
    lastfm_transport: Arc<dyn LastFmTransport>,
    lastfm: LastFmWorker,
    discord: Mutex<Option<DiscordWorker>>,
    together: Mutex<Option<TogetherClient>>,
    together_generation: AtomicU64,
    together_saved_queue: Mutex<Option<ml_core::Queue>>,
}

impl Services {
    pub fn new(
        ctx: &egui::Context,
        paths: SettingsPaths,
        settings: &Settings,
    ) -> (Self, Vec<String>) {
        let mut notes = Vec::new();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ml-net")
            .enable_all()
            .build()
            .expect("start tokio runtime for ml-innertube/ml-lyrics");

        let mut config = ClientConfig {
            language: settings.content_language.clone(),
            country: settings.content_country.clone(),
            ..ClientConfig::default()
        };
        if settings.proxy.enabled && !settings.proxy.url.trim().is_empty() {
            config.proxy = Some(settings.proxy.clone());
        }
        let client = InnertubeClient::new(config);

        let quality = settings.audio_quality;
        let urls = Arc::new(UrlCache {
            client: client.clone(),
            quality: Mutex::new(quality),
            urls: Mutex::new(HashMap::new()),
        });

        let (engine, mut controller) = match MpvEngine::spawn(AudioConfig::default()) {
            Ok(engine) => {
                let controller = PlaybackController::new(
                    engine.command_sender().clone(),
                    engine.event_receiver().clone(),
                    urls.clone(),
                );
                (Some(engine), controller)
            }
            Err(error) => {
                notes.push(format!("Audio disabled: {error}"));
                let (commands, _) = unbounded();
                let (_events, receiver) = unbounded();
                (
                    None,
                    PlaybackController::new(commands, receiver, urls.clone()),
                )
            }
        };

        // A second mpv instance is required for audible overlaps. It is idle
        // until the controller prepares the next crossfade.
        let crossfade_engine = if engine.is_some() {
            match MpvEngine::spawn(AudioConfig::default()) {
                Ok(second) => {
                    if controller.attach_crossfade_engine(
                        second.command_sender().clone(),
                        second.event_receiver().clone(),
                    ) {
                        Some(second)
                    } else {
                        notes.push("Crossfade unavailable: second engine was rejected".to_string());
                        None
                    }
                }
                Err(error) => {
                    notes.push(format!("Crossfade unavailable: {error}"));
                    None
                }
            }
        } else {
            None
        };

        let store = match Store::open(&paths.database_file()) {
            Ok(store) => Some(Arc::new(store)),
            Err(error) => {
                notes.push(format!("Library database unavailable: {error}"));
                None
            }
        };

        let session_path = session_file_path(&paths.data_dir);
        let mut auth_client = client.clone();
        let mut auth_cache = AuthCache::default();
        if let Some(store) = store.as_ref() {
            if let Ok(Some(raw)) = store.kv_get("auth.identity") {
                if let Some(identity) = AuthIdentity::from_json(&raw) {
                    auth_cache.identity = identity;
                }
            }
            if let Ok(Some(raw)) = store.kv_get("account") {
                if let Some(cache) = AccountCache::from_json(&raw) {
                    auth_cache.apply_cache(cache);
                }
            }
        }
        if let Some(jar) = load_session_file(&session_path) {
            auth_client.set_cookie_jar(&jar);
            auth_client.set_session_path(Some(session_path.clone()));
            auth_cache.expires_at = jar
                .soonest_expiry((now_millis() / 1000) as i64)
                .or(auth_cache.expires_at);
        } else if session_path.exists() {
            notes.push("Session cookies ignored: file is unreadable or empty".to_string());
        }
        auth_client.set_login_identity(
            auth_cache.identity.data_sync_id.clone(),
            auth_cache.identity.auth_user.clone(),
        );
        let browser_profile = newest_firefox_profile().map(|profile| profile.name);

        // Restore the last queue without starting audio. Corrupt or legacy JSON
        // is ignored (and replaced on the next queue mutation).
        let restored = store
            .as_ref()
            .and_then(|store| store.kv_get("queue").ok().flatten())
            .and_then(|raw| parse_persisted_queue(&raw));
        if let Some(queue) = restored {
            eprintln!(
                "queue: restored {} tracks at {}",
                queue.len(),
                queue.position()
            );
            controller.restore_queue(queue);
        }

        let lyrics_settings = Mutex::new(settings.lyrics.clone());
        let (tx, events_rx) = unbounded();
        let publisher = Publisher {
            tx,
            ctx: ctx.clone(),
        };

        let download_limit_key = "downloads.limit_bytes";
        let download_limit = store
            .as_ref()
            .and_then(|store| store.kv_get(download_limit_key).ok().flatten())
            .and_then(|value| value.parse::<u64>().ok());
        let download_dir = settings
            .downloads
            .directory
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| paths.data_dir.join("downloads"));
        let download_dir = std::path::absolute(&download_dir).unwrap_or(download_dir);
        let config = DownloadConfig {
            dir: download_dir.clone(),
            max_concurrent: settings.downloads.concurrent_limit as usize,
            quality: settings.audio_quality,
            max_storage_bytes: download_limit,
        };
        let download_badges: Arc<Mutex<HashMap<String, DownloadBadge>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (downloads, downloads_error) =
            match DownloadManager::with_source(config, Arc::new(client.clone())) {
                Ok(manager) => {
                    {
                        let mut badges = download_badges.lock();
                        for entry in manager.list() {
                            badges.insert(entry.id.clone(), download_badge(Some(&entry)));
                        }
                    }
                    let receiver = manager.events();
                    let forward = publisher.clone();
                    let badges = download_badges.clone();
                    match std::thread::Builder::new()
                        .name("ml-downloads-ui".into())
                        .spawn(move || {
                            let mut last_percent: HashMap<String, u8> = HashMap::new();
                            while let Ok(event) = receiver.recv() {
                                match &event {
                                    DownloadEvent::Queued(id) => {
                                        badges.lock().insert(id.clone(), DownloadBadge::Queued);
                                        eprintln!("downloads: queued {id}");
                                    }
                                    DownloadEvent::Progress { id, bytes, total } => {
                                        let percent =
                                            ml_downloads::progress_percent(*bytes, *total);
                                        badges.lock().insert(
                                            id.clone(),
                                            DownloadBadge::Downloading(percent),
                                        );
                                        let previous = last_percent.get(id).copied().unwrap_or(0);
                                        if percent >= previous.saturating_add(10) || percent == 100
                                        {
                                            last_percent.insert(id.clone(), percent);
                                            eprintln!(
                                            "downloads: progress {id} {percent}% ({bytes}/{total})"
                                        );
                                        }
                                    }
                                    DownloadEvent::Completed { id, path } => {
                                        badges.lock().insert(id.clone(), DownloadBadge::Completed);
                                        eprintln!(
                                            "downloads: completed {id} -> {}",
                                            path.display()
                                        );
                                    }
                                    DownloadEvent::Failed { id, error } => {
                                        badges.lock().insert(id.clone(), DownloadBadge::Failed);
                                        eprintln!("downloads: failed {id}: {error}");
                                    }
                                    DownloadEvent::Cancelled(id) => {
                                        badges.lock().insert(id.clone(), DownloadBadge::Cancelled);
                                        eprintln!("downloads: cancelled {id}");
                                    }
                                    DownloadEvent::Removed(id) => {
                                        badges.lock().remove(id);
                                        eprintln!("downloads: removed {id}");
                                    }
                                    DownloadEvent::StorageChanged { used, limit } => {
                                        eprintln!("downloads: storage {used}/{limit:?}");
                                    }
                                }
                                forward.send(UiEvent::Download(event));
                            }
                        }) {
                        Ok(_) => {}
                        Err(error) => notes.push(format!("Download events unavailable: {error}")),
                    }
                    (Some(manager), None)
                }
                Err(error) => {
                    notes.push(format!("Downloads unavailable: {error}"));
                    (None, Some(error.to_string()))
                }
            };
        let download_quality = downloads.as_ref().map(|_| settings.audio_quality);
        let lastfm_transport: Arc<dyn LastFmTransport> = Arc::new(HttpTransport::with_defaults());
        let lastfm = LastFmWorker::new(
            rt.handle().clone(),
            queue_path(&paths.data_dir),
            lastfm_transport.clone(),
            publisher.clone(),
        );

        let services = Self {
            rt,
            client,
            lyrics_settings,
            store,
            paths,
            publisher,
            events_rx,
            urls,
            controller: Arc::new(Mutex::new(controller)),
            engine,
            crossfade_engine,
            downloads,
            downloads_error,
            download_dir,
            download_limit: Mutex::new(download_limit),
            download_active_limit: download_limit,
            download_quality,
            download_badges,
            radio: Arc::new(Mutex::new(RadioState::default())),
            auth_client: Arc::new(Mutex::new(auth_client)),
            auth: Arc::new(Mutex::new(auth_cache)),
            session_path,
            browser_profile,
            catalog_generation: Arc::new(AtomicU64::new(0)),
            stats_generation: Arc::new(AtomicU64::new(0)),
            recognition_client: Arc::new(ShazamClient::new()),
            recognition_generation: Arc::new(AtomicU64::new(0)),
            recognition_play_generation: Arc::new(AtomicU64::new(0)),
            recognition_stop: Arc::new(Mutex::new(None)),
            lyrics_translation_generation: Arc::new(AtomicU64::new(0)),
            lyrics_translation_cache: Arc::new(Mutex::new(HashMap::new())),
            lastfm_transport,
            lastfm,
            discord: Mutex::new(None),
            together: Mutex::new(None),
            together_generation: AtomicU64::new(0),
            together_saved_queue: Mutex::new(None),
        };
        (services, notes)
    }

    /// Stop playback engines, integration workers and download jobs.
    ///
    /// Idempotent and safe before `std::process::exit`, which skips `Drop`:
    /// this is what releases the two mpv children and their IPC sockets.
    pub fn shutdown(&mut self) {
        self.cancel_recognition();
        self.lastfm.shutdown();
        self.stop_discord();
        self.stop_together();
        self.controller.lock().shutdown();
        if let Some(mut engine) = self.engine.take() {
            engine.shutdown();
        }
        if let Some(mut engine) = self.crossfade_engine.take() {
            engine.shutdown();
        }
        if let Some(downloads) = self.downloads.take() {
            downloads.shutdown();
        }
    }

    pub fn try_recv(&self) -> Option<UiEvent> {
        self.events_rx.try_recv().ok()
    }

    fn publisher(&self) -> Publisher {
        self.publisher.clone()
    }

    /// Controller events are drained in `logic`; actions taken during `ui`
    /// need one more frame before widgets see the new state.
    fn wake(&self) {
        self.publisher.ctx.request_repaint();
    }

    // ----------------------------------------------------------- integrations

    pub fn lastfm_status(&self) -> LastFmStatus {
        self.lastfm.status()
    }

    pub fn configure_lastfm(
        &self,
        config: LastFmConfig,
        enabled: bool,
        username: Option<String>,
        flush: bool,
    ) {
        self.lastfm.configure(config, enabled, username, flush);
    }

    pub fn lastfm_snapshot(&self, snapshot: &ml_core::PlayerSnapshot) {
        self.lastfm.snapshot(snapshot.clone());
    }

    pub fn lastfm_sign_out(&self) {
        self.lastfm.sign_out();
    }

    pub fn lastfm_forget(&self) {
        self.lastfm.forget();
    }

    pub fn connect_lastfm(&self, config: LastFmConfig, username: String, password: String) {
        let transport = self.lastfm_transport.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let mut client = LastFmClient::new(config.clone(), transport);
            let result = client.authenticate(&username, &password).await;
            let result = match result {
                Ok(session) => Ok(session),
                Err(error) => Err(redact_secrets(
                    &error.to_string(),
                    &[
                        &config.api_key,
                        config.secret.as_deref().unwrap_or(""),
                        config.session.as_deref().unwrap_or(""),
                        &username,
                        &password,
                    ],
                )),
            };
            publisher.send(UiEvent::LastFmConnected { result });
        });
    }

    pub fn configure_discord(&self, enabled: bool, app_id: &str) {
        let app_id = discord_app_id(app_id);
        let mut slot = self.discord.lock();
        if !enabled {
            if let Some(worker) = slot.take() {
                worker.shutdown();
            }
            return;
        }
        if slot
            .as_ref()
            .is_some_and(|worker| worker.app_id() == app_id)
        {
            return;
        }
        if let Some(worker) = slot.take() {
            worker.shutdown();
        }
        *slot = Some(DiscordWorker::new(app_id, self.publisher.clone()));
    }

    pub fn discord_status(&self) -> DiscordStatus {
        self.discord
            .lock()
            .as_ref()
            .map(DiscordWorker::status)
            .unwrap_or_default()
    }

    pub fn discord_snapshot(&self, snapshot: &ml_core::PlayerSnapshot) {
        if let Some(worker) = self.discord.lock().as_ref() {
            worker.snapshot(snapshot.clone());
        }
    }

    pub fn stop_discord(&self) {
        if let Some(worker) = self.discord.lock().take() {
            worker.shutdown();
        }
    }

    // --------------------------------------------------------------- together

    /// Start a room client. No client exists before this call, so startup and
    /// demos never touch the network.
    fn begin_together(&self, server: &str, code: Option<&str>, username: &str) -> u64 {
        let generation = self.together_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut slot = self.together.lock();
        if let Some(previous) = slot.take() {
            previous.disconnect();
        }
        let client = TogetherClient::with_server(server);
        match code {
            Some(code) => client.join_room(code, username),
            None => client.create_room(username),
        }
        *slot = Some(client);
        generation
    }

    pub fn create_together_room(&self, server: &str, username: &str) -> u64 {
        self.begin_together(server, None, username)
    }

    pub fn join_together_room(&self, server: &str, code: &str, username: &str) -> u64 {
        self.begin_together(server, Some(code), username)
    }

    pub fn leave_together_room(&self) -> u64 {
        let generation = self.together_generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(client) = self.together.lock().take() {
            client.leave();
            client.disconnect();
        }
        self.restore_together_queue();
        generation
    }

    /// Drain connection/room events for this generation. Never blocks.
    pub fn poll_together(&self) -> (u64, Vec<TogetherEvent>) {
        let generation = self.together_generation.load(Ordering::SeqCst);
        let mut events = Vec::new();
        if let Some(client) = self.together.lock().as_ref() {
            while let Some(event) = client.try_recv_event() {
                events.push(event);
            }
        }
        (generation, events)
    }

    pub fn together_playback(&self, action: PlaybackAction) {
        if let Some(client) = self.together.lock().as_ref() {
            client.send_playback(action);
        }
    }

    pub fn together_request_sync(&self) {
        if let Some(client) = self.together.lock().as_ref() {
            client.request_sync();
        }
    }

    pub fn together_approve_join(&self, user_id: &str) {
        if let Some(client) = self.together.lock().as_ref() {
            client.approve_join(user_id);
        }
    }

    pub fn together_reject_join(&self, user_id: &str) {
        if let Some(client) = self.together.lock().as_ref() {
            client.reject_join(user_id, None);
        }
    }

    /// Install only the host's current track for a guest, so the local player
    /// cannot auto-advance into a stale queue. The room queue is shown in the
    /// Together screen. Returns false when the room has no current track.
    pub fn install_together_queue(&self, room: &RoomState) -> bool {
        let queue = queue_from_room(room.current_track.as_ref(), &[]);
        if queue.is_empty() {
            return false;
        }
        let mut saved = self.together_saved_queue.lock();
        if saved.is_none() {
            *saved = Some(self.controller.lock().queue().clone());
        }
        drop(saved);
        self.controller.lock().play_queue(queue, 0);
        true
    }

    fn restore_together_queue(&self) {
        let saved = self.together_saved_queue.lock().take();
        if let Some(queue) = saved {
            self.controller.lock().restore_queue(queue);
        }
    }

    fn stop_together(&self) {
        self.together_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(client) = self.together.lock().take() {
            client.disconnect();
        }
    }

    // ---------------------------------------------------------------- pages

    pub fn load_home(&self) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.home().await {
                Ok(feed) => UiEvent::Core(AppEvent::HomeLoaded(feed)),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn load_home_more(&self, token: String) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.home_continuation(&token).await {
                Ok(feed) => UiEvent::HomeMore(feed),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn search(&self, query: String, filter: Option<SearchFilter>) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.search(&query, filter).await {
                Ok(page) => UiEvent::Core(AppEvent::SearchLoaded(page)),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn search_more(&self, token: String) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.search_continuation(&token).await {
                Ok(page) => UiEvent::SearchMore(page),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn suggest(&self, query: String) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            // Suggestion failures are cosmetic; never surface them.
            if let Ok(items) = client.suggestions(&query).await {
                publisher.send(UiEvent::Suggestions { query, items });
            }
        });
    }

    pub fn album(&self, browse_id: String) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.album(&browse_id).await {
                Ok(page) => UiEvent::Core(AppEvent::AlbumLoaded(page)),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn album_more(&self, token: String) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.album_continuation(&token).await {
                Ok(page) => UiEvent::AlbumMore(page),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn artist(&self, channel_id: String) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.artist(&channel_id).await {
                Ok(page) => UiEvent::Core(AppEvent::ArtistLoaded(page)),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn artist_more(&self, token: String) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.artist_continuation(&token).await {
                Ok(feed) => UiEvent::ArtistMore(feed),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn playlist(&self, playlist_id: String) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.playlist(&playlist_id).await {
                Ok(page) => UiEvent::Core(AppEvent::PlaylistLoaded(page)),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn playlist_more(&self, token: String) {
        let client = self.client.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let event = match client.playlist_continuation(&token).await {
                Ok(page) => UiEvent::PlaylistMore(page),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    pub fn lyrics(&self, song: Song) {
        let providers = lyrics_providers(&self.lyrics_settings.lock().clone());
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let client = LyricsClient::new(providers);
            let mut query = LyricsQuery::new(song.title.clone(), song.artist_names());
            query.album = song.album.as_ref().map(|album| album.title.clone());
            query.duration = song.duration;
            let event = match client.fetch(&query).await {
                Ok(document) => UiEvent::Core(AppEvent::LyricsLoaded(document)),
                Err(error) => UiEvent::Core(AppEvent::Error(error)),
            };
            publisher.send(event);
        });
    }

    /// Translate the current document lines, cache by video id, and emit
    /// `LyricsTranslated`. A missing key/endpoint is refused before any request.
    pub fn translate_current_lyrics(
        &self,
        video_id: String,
        lines: Vec<LyricsLine>,
        config: TranslationConfig,
    ) -> u64 {
        let generation = next_generation(&self.lyrics_translation_generation);
        let refusal = if !config.is_configured() {
            Some(TRANSLATION_MISSING_KEY_MESSAGE.to_string())
        } else if matches!(
            &config.endpoint,
            TranslationEndpoint::OpenAiCompatible { url } if !translation_endpoint_valid(url)
        ) {
            Some(TRANSLATION_ENDPOINT_MESSAGE.to_string())
        } else {
            None
        };
        if let Some(message) = refusal {
            self.publisher().send(UiEvent::LyricsTranslated {
                generation,
                video_id,
                result: Err(message),
            });
            return generation;
        }
        if let Some(cached) = self.lyrics_translation_cache.lock().get(&video_id).cloned() {
            self.publisher().send(UiEvent::LyricsTranslated {
                generation,
                video_id,
                result: Ok(cached),
            });
            return generation;
        }

        let publisher = self.publisher();
        let cache = self.lyrics_translation_cache.clone();
        let generation_guard = self.lyrics_translation_generation.clone();
        self.rt.spawn(async move {
            let document = LyricsDocument {
                lines,
                ..LyricsDocument::default()
            };
            let client =
                TranslationClient::new(Arc::new(ReqwestTransport::with_defaults()), config);
            let result = client.translate_document(&document).await;
            if !generation_is_current(&generation_guard, generation) {
                return;
            }
            let result = result.map_err(|error| error.to_string());
            if let Ok(lines) = &result {
                cache.lock().insert(video_id.clone(), lines.clone());
            }
            publisher.send(UiEvent::LyricsTranslated {
                generation,
                video_id,
                result,
            });
        });
        generation
    }

    /// Invalidate an in-flight translation without clearing existing cache.
    pub fn cancel_lyrics_translation(&self) -> u64 {
        next_generation(&self.lyrics_translation_generation)
    }

    /// Provider/key/language changed: drop cached translations and invalidate.
    pub fn reset_lyrics_translation(&self) -> u64 {
        self.lyrics_translation_cache.lock().clear();
        next_generation(&self.lyrics_translation_generation)
    }

    /// Queue a thumbnail download; decoded pixels come back as `UiEvent::Art`.
    pub fn fetch_art(&self, url: String) {
        let http = self.client.http().clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let image = async {
                let response = http.get(&url).send().await.ok()?;
                if !response.status().is_success() {
                    return None;
                }
                let bytes = response.bytes().await.ok()?;
                let decoded = image::load_from_memory(&bytes).ok()?.thumbnail(512, 512);
                let rgba = decoded.to_rgba8();
                let (width, height) = rgba.dimensions();
                Some(Arc::new(ArtImage {
                    width: width as usize,
                    height: height as usize,
                    rgba: rgba.into_raw(),
                }))
            }
            .await;
            publisher.send(UiEvent::Art { url, image });
        });
    }

    // ------------------------------------------------------------- playback

    pub fn play(&self, songs: Vec<Song>, index: usize, source: QueueSource) {
        if songs.is_empty() {
            return;
        }
        let queue = Queue::new(songs, source);
        self.controller.lock().play_queue(queue, index);
        self.persist_queue();
        self.wake();
    }

    /// Play one song and continue with the InnerTube `next` radio queue.
    pub fn play_song(&self, song: Song) {
        self.play_radio(song, None);
    }

    /// Build the full `next` queue for `song` (optionally inside a playlist)
    /// and hand it back to the UI. Falls back to the single track on failure.
    /// The returned page is kept so `extend_radio` can append its continuation.
    pub fn play_radio(&self, song: Song, playlist_id: Option<String>) {
        let client = self.client.clone();
        let publisher = self.publisher();
        let radio = self.radio.clone();
        let generation = radio.lock().begin();
        self.rt.spawn(async move {
            let video_id = song.id.clone();
            match client.queue(&video_id, playlist_id.as_deref()).await {
                Ok(page) if !page.songs.is_empty() => {
                    let installed = radio.lock().install(generation, page.clone());
                    if installed {
                        eprintln!("radio: initial {} tracks for {video_id}", page.songs.len());
                        publisher.send(radio_event(song, page));
                    }
                }
                Ok(_) => {
                    let current = {
                        let mut state = radio.lock();
                        if state.generation == generation {
                            state.exhausted = true;
                            state.page = None;
                            state.loading = false;
                            true
                        } else {
                            false
                        }
                    };
                    if current {
                        publisher.send(radio_fallback(song, QueueSource::Radio));
                    }
                }
                Err(error) => {
                    let current = {
                        let mut state = radio.lock();
                        if state.generation == generation {
                            state.exhausted = true;
                            state.page = None;
                            state.loading = false;
                            true
                        } else {
                            false
                        }
                    };
                    if current {
                        publisher.send(UiEvent::Core(AppEvent::Toast(format!(
                            "Radio queue unavailable: {error}"
                        ))));
                        publisher.send(radio_fallback(song, QueueSource::List));
                    }
                }
            }
        });
    }

    /// Forget the radio continuation (non-radio playback or an exhausted page).
    /// A late response from the previous generation is ignored.
    pub fn clear_radio(&self) {
        self.radio.lock().clear();
    }

    /// Whether the current radio page still has a continuation to fetch.
    pub fn radio_can_extend(&self) -> bool {
        self.radio.lock().can_extend()
    }

    /// Fetch the next radio continuation and append only new video ids.
    ///
    /// `force` skips the near-end check for the first page after a radio
    /// install; automatic calls wait until the last five tracks.
    pub fn extend_radio(&self, force: bool) {
        let near_end = {
            let controller = self.controller.lock();
            radio_should_extend(
                (!controller.queue().is_empty()).then(|| controller.queue().position()),
                controller.queue().len(),
            )
        };
        if !force && !near_end {
            return;
        }
        let (page, generation) = {
            let mut state = self.radio.lock();
            if state.loading || state.exhausted {
                return;
            }
            let Some(page) = state.page.clone() else {
                return;
            };
            if page.continuation.is_none() {
                return;
            }
            state.loading = true;
            (page, state.generation)
        };
        let client = self.client.clone();
        let publisher = self.publisher();
        let controller = self.controller.clone();
        let radio = self.radio.clone();
        self.rt.spawn(async move {
            let mut page = page;
            let before = page.songs.len();
            match client.queue_append(&mut page).await {
                Ok(_) => {
                    let new_songs = page.songs[before..].to_vec();
                    let installed = {
                        let mut state = radio.lock();
                        if state.generation != generation {
                            None
                        } else {
                            let added = if new_songs.is_empty() {
                                0
                            } else {
                                controller.lock().append_queue(new_songs)
                            };
                            state.loading = false;
                            state.exhausted = page.continuation.is_none() || added == 0;
                            state.page = Some(page);
                            eprintln!(
                                "radio: appended {added} tracks (total {})",
                                controller.lock().queue().len()
                            );
                            Some(added)
                        }
                    };
                    if let Some(added) = installed {
                        publisher.send(UiEvent::RadioExtended { added });
                    }
                }
                Err(error) => {
                    let current = {
                        let mut state = radio.lock();
                        if state.generation == generation {
                            state.loading = false;
                            true
                        } else {
                            false
                        }
                    };
                    if current {
                        publisher.send(UiEvent::Core(AppEvent::Toast(format!(
                            "Radio continuation unavailable: {error}"
                        ))));
                    }
                }
            }
        });
    }

    pub fn play_shuffled(&self, songs: Vec<Song>, source: QueueSource) {
        if songs.is_empty() {
            return;
        }
        let mut queue = Queue::new(songs, source);
        queue.set_shuffle(true);
        self.controller.lock().play_queue(queue, 0);
        self.persist_queue();
        self.wake();
    }

    pub fn jump_queue(&self, position: usize) {
        let mut controller = self.controller.lock();
        let queue = controller.queue().clone();
        controller.play_queue(queue, position);
        drop(controller);
        self.persist_queue();
        self.wake();
    }

    pub fn toggle_play(&self) {
        self.controller.lock().play_pause();
        self.wake();
    }

    pub fn next(&self) {
        self.controller.lock().next();
        self.wake();
    }

    pub fn previous(&self) {
        self.controller.lock().previous();
        self.wake();
    }

    pub fn seek(&self, position_ms: u64) {
        self.controller.lock().seek(position_ms);
        self.wake();
    }

    pub fn set_volume(&self, volume: f32) {
        self.controller.lock().set_volume(volume);
        self.wake();
    }

    pub fn toggle_shuffle(&self) {
        self.controller.lock().toggle_shuffle();
        self.wake();
    }

    pub fn cycle_repeat(&self) {
        self.controller.lock().cycle_repeat();
        self.wake();
    }

    pub fn poll(&self) -> Vec<ControllerEvent> {
        self.controller.lock().poll()
    }

    pub fn stop(&self) {
        self.controller.lock().stop();
        self.wake();
    }

    pub fn snapshot(&self) -> ml_core::PlayerSnapshot {
        self.controller.lock().snapshot()
    }

    pub fn queue_view(&self) -> QueueView {
        self.controller.lock().queue_view()
    }

    /// Songs in playback order plus the active index (queue overlay order).
    pub fn queue_display(&self) -> (Vec<Song>, Option<usize>) {
        let controller = self.controller.lock();
        let position = if controller.queue().is_empty() {
            None
        } else {
            Some(controller.queue().position())
        };
        (controller.queue().iter().cloned().collect(), position)
    }

    pub fn move_queue_item(&self, from: usize, to: usize) -> bool {
        let changed = self.controller.lock().move_queue_item(from, to);
        if changed {
            self.persist_queue();
            self.wake();
        }
        changed
    }

    pub fn remove_queue_item(&self, position: usize) -> bool {
        let removed = self.controller.lock().remove_queue_item(position).is_some();
        if removed {
            self.persist_queue();
            self.wake();
        }
        removed
    }

    /// Asynchronously resolve a stream URL and hand it to the controller.
    /// A verified offline file wins over the network.
    pub fn resolve(&self, video_id: String) {
        if let Some(path) = self.download_path(&video_id) {
            let url = crate::pure::local_file_url(&path);
            eprintln!("playback: offline file {}", path.display());
            self.controller.lock().submit_resolved(video_id, url);
            self.publisher().send(UiEvent::Resolved);
            return;
        }
        let client = self.client.clone();
        let publisher = self.publisher();
        let controller = self.controller.clone();
        let urls = self.urls.clone();
        let quality = *urls.quality.lock();
        self.rt.spawn(async move {
            match client.stream_info(&video_id, quality).await {
                Ok(info) => {
                    urls.urls.lock().insert(video_id.clone(), info.url.clone());
                    controller.lock().submit_resolved(video_id, info.url);
                    publisher.send(UiEvent::Resolved);
                }
                Err(error) => publisher.send(UiEvent::PlaybackFailed {
                    video_id,
                    message: error.to_string(),
                }),
            }
        });
    }

    // ------------------------------------------------------------- downloads

    pub fn downloads_available(&self) -> bool {
        self.downloads.is_some()
    }

    pub fn downloads_error(&self) -> Option<&str> {
        self.downloads_error.as_deref()
    }

    pub fn download_dir(&self) -> &std::path::Path {
        &self.download_dir
    }

    pub fn download_quality(&self) -> Option<AudioQuality> {
        self.download_quality
    }

    /// Configured (next-launch) budget; the active manager keeps its own.
    pub fn download_limit(&self) -> Option<u64> {
        *self.download_limit.lock()
    }

    pub fn download_active_limit(&self) -> Option<u64> {
        self.download_active_limit
    }

    pub fn set_download_limit(&self, limit: Option<u64>) {
        *self.download_limit.lock() = limit;
        self.ui_kv_set(
            "downloads.limit_bytes",
            limit.map(|value| value.to_string()).unwrap_or_default(),
        );
    }

    pub fn download_list(&self) -> Vec<Download> {
        self.downloads
            .as_ref()
            .map(DownloadManager::list)
            .unwrap_or_default()
    }

    pub fn download_usage(&self) -> u64 {
        self.downloads
            .as_ref()
            .map(DownloadManager::storage_usage)
            .unwrap_or(0)
    }

    pub fn download_badge(&self, song_id: &str) -> DownloadBadge {
        let cached = self.download_badges.lock().get(song_id).copied();
        if cached.is_some_and(|badge| badge != DownloadBadge::Failed) {
            return cached.unwrap_or(DownloadBadge::None);
        }
        // Failed can also be emitted for a budget rejection with no manifest
        // entry, so confirm against the manager before offering retry.
        let entry = self
            .downloads
            .as_ref()
            .and_then(|manager| manager.get(song_id));
        let badge = download_badge(entry.as_ref());
        self.download_badges
            .lock()
            .insert(song_id.to_string(), badge);
        badge
    }

    pub fn download_path(&self, song_id: &str) -> Option<std::path::PathBuf> {
        self.downloads
            .as_ref()
            .and_then(|manager| manager.path_for(song_id))
    }

    pub fn is_downloaded(&self, song_id: &str) -> bool {
        self.download_path(song_id).is_some()
    }

    pub fn enqueue_downloads(&self, songs: &[Song]) -> Vec<String> {
        self.downloads
            .as_ref()
            .map(|manager| manager.enqueue(songs))
            .unwrap_or_default()
    }

    pub fn cancel_download(&self, id: &str) -> bool {
        self.downloads
            .as_ref()
            .is_some_and(|manager| manager.cancel(id))
    }

    pub fn retry_download(&self, id: &str) -> bool {
        self.downloads
            .as_ref()
            .is_some_and(|manager| manager.retry(id))
    }

    pub fn remove_download(&self, id: &str) -> bool {
        self.downloads
            .as_ref()
            .is_some_and(|manager| manager.remove(id))
    }

    pub fn clear_failed_downloads(&self) -> usize {
        self.downloads
            .as_ref()
            .map(DownloadManager::clear_failed)
            .unwrap_or(0)
    }

    // ----------------------------------------------------------------- store

    /// Search history, newest first. Empty while the store is unavailable.
    pub fn search_history(&self, limit: u32) -> Vec<String> {
        self.store
            .as_ref()
            .and_then(|store| store.search_history(limit).ok())
            .unwrap_or_default()
    }

    pub fn record_search(&self, query: String) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let publisher = self.publisher();
        self.rt.spawn_blocking(move || {
            if let Err(error) = store.add_search_history(&query) {
                publisher.send(UiEvent::Core(AppEvent::Toast(format!(
                    "Search history: {error}"
                ))));
            }
        });
    }

    /// Record a play event with the actual listened duration; the song row is
    /// upserted first for the FK. Historical rows are not backfilled.
    pub fn record_play(&self, song: Song, played_ms: u64) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let duration_ms = u64::from(song.duration.unwrap_or(0)).saturating_mul(1000);
        let played_ms = if duration_ms == 0 {
            played_ms
        } else {
            played_ms.min(duration_ms)
        };
        let played_ms = played_ms.min(i64::MAX as u64) as i64;
        self.rt.spawn_blocking(move || {
            if let Err(error) = store.upsert_song(&song) {
                println!("store: cannot save song: {error}");
                return;
            }
            if let Err(error) = store.record_play(&song.id, played_ms, now_millis() as i64) {
                println!("store: cannot record play: {error}");
            }
        });
    }

    pub fn set_liked(&self, song: Song, liked: bool) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let publisher = self.publisher();
        self.rt.spawn_blocking(move || {
            let result = store
                .upsert_song(&song)
                .and_then(|()| store.set_liked(&song.id, liked));
            match result {
                Ok(()) => publisher.send(UiEvent::LibraryDirty { playlist: None }),
                Err(error) => publisher.send(UiEvent::Core(AppEvent::Toast(error.to_string()))),
            }
        });
    }

    /// Persist the controller queue to `kv("queue")` for the next launch.
    pub fn persist_queue(&self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let json = {
            let controller = self.controller.lock();
            serde_json::to_string(controller.queue()).ok()
        };
        let Some(json) = json else {
            return;
        };
        self.rt.spawn_blocking(move || {
            if let Err(error) = store.kv_set("queue", &json) {
                println!("store: cannot persist queue: {error}");
            }
        });
    }

    // ------------------------------------------------------------- library

    /// Load every Library shelf plus history and the liked count in one pass.
    pub fn load_library(&self, sort: LibrarySort) {
        let Some(store) = self.store.clone() else {
            self.publisher.send(UiEvent::Core(AppEvent::Toast(
                "Library database unavailable".into(),
            )));
            return;
        };
        let publisher = self.publisher();
        self.rt.spawn_blocking(move || {
            let result = (|| -> ml_core::MlResult<LibraryData> {
                let songs = store
                    .library_page(LibraryFilter::Songs, sort, 500, 0)?
                    .songs;
                let albums = store
                    .library_page(LibraryFilter::Albums, sort, 200, 0)?
                    .albums;
                let artists = store
                    .library_page(LibraryFilter::Artists, sort, 200, 0)?
                    .artists;
                let playlists = store.playlists(500, 0)?;
                let history = store.history(300, 0)?;
                let liked = store.liked_songs(500, 0)?;
                Ok(LibraryData {
                    songs,
                    albums,
                    artists,
                    playlists,
                    history,
                    liked,
                })
            })();
            match result {
                Ok(data) => publisher.send(UiEvent::LibraryLoaded(data)),
                Err(error) => publisher.send(UiEvent::Core(AppEvent::Error(error))),
            }
        });
    }

    pub fn load_liked(&self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let publisher = self.publisher();
        self.rt
            .spawn_blocking(move || match store.liked_songs(500, 0) {
                Ok(songs) => publisher.send(UiEvent::LikedLoaded(songs)),
                Err(error) => publisher.send(UiEvent::Core(AppEvent::Error(error))),
            });
    }

    pub fn load_local_playlist(&self, id: String) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let publisher = self.publisher();
        self.rt.spawn_blocking(move || {
            let result = (|| -> ml_core::MlResult<LocalPlaylistView> {
                let record = store.get_playlist(&id)?;
                let songs = store
                    .playlist_page(&id, 1000, 0)?
                    .into_iter()
                    .map(|entry| entry.song)
                    .collect();
                Ok(LocalPlaylistView {
                    id: id.clone(),
                    name: record
                        .map(|record| record.name)
                        .unwrap_or_else(|| "Playlist".into()),
                    songs,
                })
            })();
            match result {
                Ok(view) => publisher.send(UiEvent::LocalPlaylistLoaded(view)),
                Err(error) => publisher.send(UiEvent::Core(AppEvent::Error(error))),
            }
        });
    }

    /// Run one playlist mutation off the UI thread and report success/error.
    fn mutate_playlist(
        &self,
        playlist: Option<String>,
        op: impl FnOnce(&Store) -> ml_core::MlResult<()> + Send + 'static,
    ) {
        let Some(store) = self.store.clone() else {
            self.publisher.send(UiEvent::Core(AppEvent::Toast(
                "Library database unavailable".into(),
            )));
            return;
        };
        let publisher = self.publisher();
        self.rt.spawn_blocking(move || match op(&store) {
            Ok(()) => publisher.send(UiEvent::LibraryDirty { playlist }),
            Err(error) => publisher.send(UiEvent::Core(AppEvent::Toast(error.to_string()))),
        });
    }

    pub fn create_local_playlist(&self, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        let publisher = self.publisher();
        self.rt
            .spawn_blocking(move || match store.create_playlist(&name) {
                Ok(id) => {
                    publisher.send(UiEvent::PlaylistCreated { id, name });
                    publisher.send(UiEvent::LibraryDirty { playlist: None });
                }
                Err(error) => publisher.send(UiEvent::Core(AppEvent::Toast(error.to_string()))),
            });
    }

    pub fn rename_local_playlist(&self, id: String, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let id_for_event = id.clone();
        self.mutate_playlist(Some(id_for_event), move |store| {
            store.rename_playlist(&id, &name)
        });
    }

    pub fn delete_local_playlist(&self, id: String) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let publisher = self.publisher();
        let event_id = id.clone();
        self.rt
            .spawn_blocking(move || match store.delete_playlist(&id) {
                Ok(()) => publisher.send(UiEvent::PlaylistDeleted { id: event_id }),
                Err(error) => publisher.send(UiEvent::Core(AppEvent::Toast(error.to_string()))),
            });
    }

    pub fn add_song_to_playlist(&self, playlist_id: String, song: Song) {
        let event_id = playlist_id.clone();
        self.mutate_playlist(Some(event_id), move |store| {
            store.upsert_song(&song)?;
            store.add_songs(&playlist_id, &[song]).map(|_| ())
        });
    }

    pub fn remove_song_from_playlist(&self, playlist_id: String, song_id: String) {
        let event_id = playlist_id.clone();
        self.mutate_playlist(Some(event_id), move |store| {
            store.remove_song(&playlist_id, &song_id)
        });
    }

    pub fn move_playlist_song(&self, playlist_id: String, from: usize, to: usize) {
        let event_id = playlist_id.clone();
        self.mutate_playlist(Some(event_id), move |store| {
            store.move_song(&playlist_id, from as i64, to as i64)
        });
    }

    pub fn set_in_library(&self, song: Song, in_library: bool) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let publisher = self.publisher();
        self.rt.spawn_blocking(move || {
            let result = store
                .upsert_song(&song)
                .and_then(|()| store.set_in_library(&song.id, in_library));
            match result {
                Ok(()) => publisher.send(UiEvent::LibraryDirty { playlist: None }),
                Err(error) => publisher.send(UiEvent::Core(AppEvent::Toast(error.to_string()))),
            }
        });
    }

    /// Shared controller handle for the MPRIS adapter.
    pub(crate) fn controller_handle(&self) -> Arc<Mutex<PlaybackController>> {
        self.controller.clone()
    }

    /// Synchronous KV read for startup-only UI preferences.
    pub fn ui_kv_get(&self, key: &str) -> Option<String> {
        let store = self.store.as_ref()?;
        match store.kv_get(key) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("store: cannot read {key}: {error}");
                None
            }
        }
    }

    /// Persist a UI-owned preference (EQ, speed, device, notifications).
    pub fn ui_kv_set(&self, key: &str, value: String) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let key = key.to_string();
        self.rt.spawn_blocking(move || {
            if let Err(error) = store.kv_set(&key, &value) {
                eprintln!("store: cannot save {key}: {error}");
            }
        });
    }

    /// Apply 10-band EQ gains (empty/zero bands disable it in mpv).
    pub fn set_eq(&self, bands: &[f32]) {
        self.controller.lock().set_eq(bands);
        self.wake();
    }

    pub fn set_sleep_timer(&self, timer: SleepTimer) {
        self.controller.lock().set_sleep_timer(timer);
        self.wake();
    }

    pub fn sleep_timer(&self) -> SleepTimer {
        self.controller.lock().sleep_timer()
    }

    /// Apply speed to both engines (pitch is preserved by mpv).
    pub fn set_speed(&self, speed: f32) {
        self.controller.lock().set_speed(speed);
        self.wake();
    }

    pub fn set_muted(&self, muted: bool) {
        self.controller.lock().set_muted(muted);
        self.wake();
    }

    /// `None`/`"auto"` selects mpv's automatic output device.
    pub fn set_device(&self, device: Option<String>) {
        self.controller.lock().set_device(device.as_deref());
        self.wake();
    }

    /// Crossfade duration in seconds (`0` disables it) plus album gapless mode.
    pub fn set_crossfade(&self, seconds: u32, gapless: bool) {
        let mut controller = self.controller.lock();
        controller.set_crossfade(std::time::Duration::from_secs(seconds.min(30).into()));
        controller.set_crossfade_gapless(gapless);
        drop(controller);
        self.wake();
    }

    pub fn crossfade(&self) -> (std::time::Duration, bool) {
        self.controller.lock().crossfade()
    }

    pub fn crossfade_available(&self) -> bool {
        self.engine.is_some() && self.crossfade_engine.is_some()
    }

    pub fn set_skip_silence(&self, enabled: bool) {
        self.controller.lock().set_skip_silence(enabled);
        self.wake();
    }

    pub fn set_normalization(&self, enabled: bool, gain_db: Option<f32>) {
        self.controller.lock().set_normalization(enabled, gain_db);
        self.wake();
    }

    pub fn list_devices(&self) {
        let publisher = self.publisher();
        self.rt.spawn_blocking(move || {
            let result =
                ml_audio::list_devices(&AudioConfig::default()).map_err(|error| error.to_string());
            publisher.send(UiEvent::Devices(result));
        });
    }

    /// Apply the audio quality preference to future stream requests.
    pub fn set_quality(&self, quality: AudioQuality) {
        *self.urls.quality.lock() = quality;
    }

    /// Apply lyrics provider toggles to future lyric fetches.
    pub fn set_lyrics_settings(&self, settings: LyricsSettings) {
        *self.lyrics_settings.lock() = settings;
    }

    // ------------------------------------------------------------------ stats

    /// Load one stats window off the UI thread. The newest request wins.
    pub fn load_stats(&self, period: StatsPeriod) {
        let generation = self.stats_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation_guard = self.stats_generation.clone();
        let publisher = self.publisher();
        let Some(store) = self.store.clone() else {
            publisher.send(UiEvent::StatsLoaded {
                period,
                result: Err("The local library database is unavailable.".to_string()),
            });
            return;
        };
        let now_ms = now_millis() as i64;
        self.rt.spawn(async move {
            let result =
                tokio::task::spawn_blocking(move || store.stats_report(period, now_ms, 10))
                    .await
                    .map_err(|error| format!("Stats task failed: {error}"))
                    .and_then(|result| result.map_err(|error| error.to_string()));
            if generation_guard.load(Ordering::SeqCst) == generation {
                publisher.send(UiEvent::StatsLoaded { period, result });
            }
        });
    }

    // ------------------------------------------------------------- recognition

    /// Start a 12 s capture, fingerprint it off the UI thread, then ask Shazam.
    /// Returns the run generation; only the newest generation is published.
    pub fn start_recognition(&self) -> u64 {
        let generation = next_generation(&self.recognition_generation);
        if let Some(stop) = self.recognition_stop.lock().take() {
            stop.store(true, Ordering::SeqCst);
        }
        self.recognition_play_generation
            .fetch_add(1, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        *self.recognition_stop.lock() = Some(stop.clone());

        let publisher = self.publisher();
        let client = self.recognition_client.clone();
        let generation_guard = self.recognition_generation.clone();
        let started = Instant::now();
        self.rt.spawn(async move {
            let captured = tokio::task::spawn_blocking(move || capture_fingerprint(&stop)).await;
            if !generation_is_current(&generation_guard, generation) {
                return;
            }
            let (signature, sample_ms) = match captured {
                Ok(Ok(captured)) => captured,
                Ok(Err(RecognitionCapture::Cancelled)) => return,
                Ok(Err(RecognitionCapture::Failed(message))) => {
                    publisher.send(UiEvent::Recognized {
                        generation,
                        result: Err(message),
                    });
                    return;
                }
                Err(error) => {
                    publisher.send(UiEvent::Recognized {
                        generation,
                        result: Err(format!("Recognition task failed: {error}")),
                    });
                    return;
                }
            };
            publisher.send(UiEvent::RecognitionProcessing { generation });
            let result = client.recognize(&signature, sample_ms).await;
            if !generation_is_current(&generation_guard, generation) {
                return;
            }
            let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let result = result
                .map(|result| RecognitionMatch {
                    result,
                    sample_ms,
                    elapsed_ms,
                })
                .map_err(|error| recognition_error_message(&error));
            publisher.send(UiEvent::Recognized { generation, result });
        });
        generation
    }

    /// Stop an in-flight capture and invalidate its result.
    pub fn cancel_recognition(&self) -> u64 {
        let generation = next_generation(&self.recognition_generation);
        self.recognition_play_generation
            .fetch_add(1, Ordering::SeqCst);
        if let Some(stop) = self.recognition_stop.lock().take() {
            stop.store(true, Ordering::SeqCst);
        }
        generation
    }

    /// Search YouTube Music for a recognised match; `RecognitionPlayReady`
    /// carries the songs. Never installs a radio queue.
    pub fn find_recognition_song(&self, generation: u64, query: String) {
        if !generation_is_current(&self.recognition_generation, generation) {
            return;
        }
        let play_generation = next_generation(&self.recognition_play_generation);
        let client = self.client.clone();
        let publisher = self.publisher();
        let generation_guard = self.recognition_generation.clone();
        let play_guard = self.recognition_play_generation.clone();
        self.rt.spawn(async move {
            let songs = client
                .search(&query, Some(SearchFilter::Songs))
                .await
                .map(|page| page.songs)
                .map_err(|error| error.to_string());
            if generation_is_current(&generation_guard, generation)
                && generation_is_current(&play_guard, play_generation)
            {
                publisher.send(UiEvent::RecognitionPlayReady { generation, songs });
            }
        });
    }

    // ---------------------------------------------------------------- catalog

    fn begin_catalog(&self) -> u64 {
        self.catalog_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn spawn_browse<F>(&self, target: BrowseTarget, request: F)
    where
        F: Future<Output = MlResult<BrowsePage>> + Send + 'static,
    {
        let generation = self.begin_catalog();
        let generation_guard = self.catalog_generation.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let result = request.await.map_err(|error| error.to_string());
            if generation_guard.load(Ordering::SeqCst) == generation {
                publisher.send(UiEvent::BrowseLoaded { target, result });
            }
        });
    }

    pub fn load_charts(&self) {
        let client = self.client.clone();
        self.spawn_browse(BrowseTarget::Charts, async move { client.charts().await });
    }

    pub fn load_new_releases(&self) {
        let client = self.client.clone();
        self.spawn_browse(BrowseTarget::NewReleases, async move {
            client.new_releases().await
        });
    }

    pub fn load_mood_results(&self, params: Option<String>) {
        let client = self.client.clone();
        let target = BrowseTarget::MoodResults {
            params: params.clone(),
        };
        self.spawn_browse(target, async move {
            client.mood_genre_page(params.as_deref()).await
        });
    }

    pub fn load_youtube_browse(&self, id: Option<String>, params: Option<String>) {
        let client = self.client.clone();
        let target = BrowseTarget::YoutubeBrowse {
            id: id.clone().unwrap_or_default(),
            params: params.clone(),
        };
        self.spawn_browse(target, async move {
            let Some(id) = id else {
                return Err(MlError::Unavailable(
                    "no YouTube browse id was provided".to_string(),
                ));
            };
            client.browse(&id, params.as_deref()).await
        });
    }

    pub fn load_browse_continuation(&self, target: BrowseTarget, token: String) {
        let generation = self.begin_catalog();
        let generation_guard = self.catalog_generation.clone();
        let publisher = self.publisher();
        let client = self.client.clone();
        self.rt.spawn(async move {
            let result = client
                .browse_continuation(&token)
                .await
                .map_err(|error| error.to_string());
            if generation_guard.load(Ordering::SeqCst) == generation {
                publisher.send(UiEvent::BrowseMore { target, result });
            }
        });
    }

    pub fn load_moods(&self) {
        let generation = self.begin_catalog();
        let generation_guard = self.catalog_generation.clone();
        let publisher = self.publisher();
        let client = self.client.clone();
        self.rt.spawn(async move {
            let result = client
                .moods_and_genres()
                .await
                .map_err(|error| error.to_string());
            if generation_guard.load(Ordering::SeqCst) == generation {
                publisher.send(UiEvent::MoodsLoaded(result));
            }
        });
    }

    pub fn load_podcasts(&self) {
        let generation = self.begin_catalog();
        let generation_guard = self.catalog_generation.clone();
        let publisher = self.publisher();
        let client = self.client.clone();
        self.rt.spawn(async move {
            let result = client.podcasts().await.map_err(|error| error.to_string());
            if generation_guard.load(Ordering::SeqCst) == generation {
                publisher.send(UiEvent::PodcastsLoaded(result));
            }
        });
    }

    pub fn load_podcast(&self, id: String) {
        let generation = self.begin_catalog();
        let generation_guard = self.catalog_generation.clone();
        let publisher = self.publisher();
        let client = self.client.clone();
        self.rt.spawn(async move {
            let result = client.podcast(&id).await.map_err(|error| error.to_string());
            if generation_guard.load(Ordering::SeqCst) == generation {
                publisher.send(UiEvent::PodcastLoaded { id, result });
            }
        });
    }

    pub fn load_podcast_more(&self, id: String, token: String) {
        let generation = self.begin_catalog();
        let generation_guard = self.catalog_generation.clone();
        let publisher = self.publisher();
        let client = self.client.clone();
        self.rt.spawn(async move {
            let result = client
                .podcast_continuation(&token)
                .await
                .map_err(|error| error.to_string());
            if generation_guard.load(Ordering::SeqCst) == generation {
                publisher.send(UiEvent::PodcastMore { id, result });
            }
        });
    }

    // --------------------------------------------------------------- account

    fn auth_client(&self) -> InnertubeClient {
        let mut client = self.auth_client.lock().clone();
        let identity = self.auth.lock().identity.clone();
        client.set_login_identity(identity.data_sync_id, identity.auth_user);
        client
    }

    pub fn account_session(&self) -> AccountSession {
        let client = self.auth_client.lock();
        let auth = self.auth.lock();
        session_from_client(&client, &auth)
    }

    pub fn cached_account(&self) -> Option<AccountCache> {
        let auth = self.auth.lock();
        auth.info.as_ref().map(|info| AccountCache {
            info: info.clone(),
            synced_at: auth.synced_at,
            counts: auth.counts,
            expires_at: auth.expires_at,
        })
    }

    pub fn account_startup(&self) -> (AccountSession, Option<AccountCache>) {
        (self.account_session(), self.cached_account())
    }

    pub fn browser_profile_name(&self) -> Option<String> {
        self.browser_profile.clone()
    }

    pub fn import_cookie_file(&self, path: PathBuf) {
        let result = (|| {
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
            let format = crate::pure::cookie_import_format(&text);
            if format == crate::pure::CookieImportFormat::Unknown {
                return Err(format!(
                    "{} is not a recognized Netscape, header or JSON cookie file.",
                    path.display()
                ));
            }
            let jar = CookieJar::load(&path)
                .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
            if jar.sapisid().is_none() {
                return Err(format!(
                    "{} is a valid cookie file but has no SAPISID; export a signed-in YouTube Music session.",
                    path.display()
                ));
            }
            Ok(jar)
        })();
        match result {
            Ok(jar) => self.begin_import(jar),
            Err(message) => self.publisher().send(UiEvent::AccountError {
                message,
                cleared: false,
            }),
        }
    }

    pub fn import_browser_cookies(&self) {
        let Some(profile) = newest_firefox_profile() else {
            self.publisher().send(UiEvent::AccountError {
                message: "No LibreWolf/Firefox profile with a readable cookie database was found."
                    .to_string(),
                cleared: false,
            });
            return;
        };
        let result = read_firefox_cookies(&profile).map_err(|error| {
            format!(
                "Could not read the cookie database for profile “{}”: {error}",
                profile.name
            )
        });
        let jar = result.and_then(|text| {
            let jar = CookieJar::parse(&text);
            if jar.sapisid().is_none() {
                Err(format!(
                    "LibreWolf profile “{}” has no YouTube session cookies; sign in there and retry.",
                    profile.name
                ))
            } else {
                Ok(jar)
            }
        });
        match jar {
            Ok(jar) => self.begin_import(jar),
            Err(message) => self.publisher().send(UiEvent::AccountError {
                message,
                cleared: false,
            }),
        }
    }

    fn begin_import(&self, jar: CookieJar) {
        if let Err(error) = save_session_file(&self.session_path, &jar) {
            self.publisher().send(UiEvent::AccountError {
                message: format!("Could not save the session file: {error}"),
                cleared: false,
            });
            return;
        }
        {
            let mut client = self.auth_client.lock();
            client.set_cookie_jar(&jar);
            client.set_session_path(Some(self.session_path.clone()));
            client.set_login_identity(None, None);
        }
        {
            let mut auth = self.auth.lock();
            auth.generation = auth.generation.wrapping_add(1);
            auth.identity = AuthIdentity::default();
            auth.info = None;
            auth.synced_at = None;
            auth.counts = SyncCounts::default();
            auth.expires_at = jar.soonest_expiry((now_millis() / 1000) as i64);
        }
        self.delete_account_cache();
        self.publisher().send(UiEvent::AccountUpdated {
            session: self.account_session(),
            cache: None,
        });
        self.spawn_account_refresh();
    }

    pub fn refresh_account(&self) {
        if !self.auth_client.lock().is_authenticated() {
            self.publisher().send(UiEvent::AccountError {
                message: "No cookie session is loaded. Import a cookies.txt file first."
                    .to_string(),
                cleared: false,
            });
            return;
        }
        self.spawn_account_refresh();
    }

    fn spawn_account_refresh(&self) {
        let mut client = self.auth_client.lock().clone();
        let shared_client = self.auth_client.clone();
        let auth = self.auth.clone();
        let generation = auth.lock().generation;
        let store = self.store.clone();
        let session_path = self.session_path.clone();
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let result = async {
                let identity = client.discover_session().await?;
                let info = client.account_info().await?;
                Ok::<_, ml_core::MlError>((identity, info))
            }
            .await;
            let current = auth.lock().generation;
            if current != generation {
                return;
            }
            match result {
                Ok((identity, info)) => {
                    let identity = AuthIdentity {
                        data_sync_id: identity.data_sync_id,
                        auth_user: identity.auth_user,
                    };
                    let expires_at = client
                        .cookie_jar()
                        .soonest_expiry((now_millis() / 1000) as i64);
                    let cache = {
                        let mut auth = auth.lock();
                        auth.identity = identity.clone();
                        auth.info = Some(info.clone());
                        auth.synced_at = None;
                        auth.counts = SyncCounts::default();
                        auth.expires_at = expires_at;
                        AccountCache {
                            info,
                            synced_at: None,
                            counts: SyncCounts::default(),
                            expires_at,
                        }
                    };
                    if let Some(store) = store.as_ref() {
                        let _ = store.kv_set("auth.identity", &identity.to_json());
                        let _ = store.kv_set("account", &cache.to_json());
                    }
                    let session = {
                        let auth = auth.lock();
                        session_from_client(&client, &auth)
                    };
                    publisher.send(UiEvent::AccountUpdated {
                        session,
                        cache: Some(cache),
                    });
                }
                Err(error) => {
                    let clear = matches!(error, ml_core::MlError::AuthRequired)
                        || matches!(
                            &error,
                            ml_core::MlError::Unavailable(message) if message.contains("signed-out")
                        );
                    if clear {
                        client.clear_auth();
                        shared_client.lock().clear_auth();
                        remove_session_file(&session_path);
                        let mut auth = auth.lock();
                        auth.identity = AuthIdentity::default();
                        auth.info = None;
                        auth.synced_at = None;
                        auth.counts = SyncCounts::default();
                        auth.expires_at = None;
                        drop(auth);
                        if let Some(store) = store.as_ref() {
                            let _ = store.kv_delete("auth.identity");
                            let _ = store.kv_delete("account");
                        }
                    }
                    publisher.send(UiEvent::AccountError {
                        message: account_error_message(&error),
                        cleared: clear,
                    });
                }
            }
        });
    }

    pub fn sign_out(&self) {
        {
            let mut client = self.auth_client.lock();
            client.clear_auth();
        }
        remove_session_file(&self.session_path);
        {
            let mut auth = self.auth.lock();
            auth.generation = auth.generation.wrapping_add(1);
            auth.identity = AuthIdentity::default();
            auth.info = None;
            auth.synced_at = None;
            auth.counts = SyncCounts::default();
            auth.expires_at = None;
        }
        self.delete_account_cache();
        self.publisher().send(UiEvent::AccountSignedOut);
    }

    fn delete_account_cache(&self) {
        if let Some(store) = self.store.as_ref() {
            let _ = store.kv_delete("auth.identity");
            let _ = store.kv_delete("account");
        }
    }

    pub fn sync_library(&self) {
        let client = self.auth_client();
        if !client.is_authenticated() {
            self.publisher().send(UiEvent::LibrarySyncFailed(
                "Import a cookie session before syncing.".to_string(),
            ));
            return;
        }
        let Some(store) = self.store.clone() else {
            self.publisher().send(UiEvent::LibrarySyncFailed(
                "Library database is unavailable.".to_string(),
            ));
            return;
        };
        let store_for_cache = self.store.clone();
        let auth = self.auth.clone();
        let generation = auth.lock().generation;
        let publisher = self.publisher();
        self.rt.spawn(async move {
            let snapshot = match client.library_snapshot().await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    publisher.send(UiEvent::LibrarySyncFailed(format!(
                        "Library sync failed: {}",
                        account_error_message(&error)
                    )));
                    return;
                }
            };
            if auth.lock().generation != generation {
                return;
            }
            let synced_at = now_millis() as i64;
            let counts = match tokio::task::spawn_blocking(move || {
                apply_library_snapshot(&store, &snapshot, synced_at)
            })
            .await
            {
                Ok(Ok(counts)) => counts,
                Ok(Err(error)) => {
                    publisher.send(UiEvent::LibrarySyncFailed(format!(
                        "Library sync could not be saved: {error}"
                    )));
                    return;
                }
                Err(error) => {
                    publisher.send(UiEvent::LibrarySyncFailed(format!(
                        "Library sync task failed: {error}"
                    )));
                    return;
                }
            };
            if auth.lock().generation != generation {
                return;
            }
            let cache = {
                let mut auth = auth.lock();
                auth.synced_at = Some(synced_at);
                auth.counts = counts;
                auth.info.as_ref().map(|info| AccountCache {
                    info: info.clone(),
                    synced_at: auth.synced_at,
                    counts: auth.counts,
                    expires_at: auth.expires_at,
                })
            };
            if let (Some(store), Some(cache)) = (store_for_cache.as_ref(), cache.as_ref()) {
                let _ = store.kv_set("account", &cache.to_json());
            }
            publisher.send(UiEvent::LibrarySynced { counts, synced_at });
            publisher.send(UiEvent::LibraryDirty { playlist: None });
        });
    }

    pub fn save_settings(&self, settings: &Settings) {
        if let Err(error) = self.paths.save(settings) {
            self.publisher
                .send(UiEvent::Core(AppEvent::Toast(format!("Settings: {error}"))));
        }
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn lyrics_providers(settings: &ml_core::LyricsSettings) -> Vec<LyricsProvider> {
    let mut providers = Vec::new();
    if settings.lrclib {
        providers.push(LyricsProvider::LrcLib);
    }
    if settings.lyrics_plus {
        providers.push(LyricsProvider::LyricsPlus);
    }
    if settings.kugou {
        providers.push(LyricsProvider::KuGou);
    }
    if settings.better_lyrics {
        providers.push(LyricsProvider::BetterLyrics);
    }
    if settings.paxsenix {
        providers.push(LyricsProvider::Paxsenix);
    }
    if settings.zemer {
        providers.push(LyricsProvider::Zemer);
    }
    providers
}

fn next_generation(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::SeqCst) + 1
}

fn generation_is_current(counter: &AtomicU64, generation: u64) -> bool {
    counter.load(Ordering::SeqCst) == generation
}

enum RecognitionCapture {
    Cancelled,
    Failed(String),
}

/// Runs on the blocking pool. Audio and PCM buffers stay inside this function;
/// only the signature URI and sample duration cross to the network task.
fn capture_fingerprint(stop: &AtomicBool) -> Result<(String, u64), RecognitionCapture> {
    if stop.load(Ordering::SeqCst) {
        return Err(RecognitionCapture::Cancelled);
    }
    let options = CaptureOptions {
        seconds: RECOGNITION_SECONDS,
        device: None,
    };
    let audio = capture_until(&options, stop).map_err(|error| {
        if stop.load(Ordering::SeqCst) {
            RecognitionCapture::Cancelled
        } else {
            RecognitionCapture::Failed(recognition_error_message(&error))
        }
    })?;
    if stop.load(Ordering::SeqCst) {
        return Err(RecognitionCapture::Cancelled);
    }
    let sample_ms = audio.duration_ms();
    let pcm = audio.to_16k_mono_pcm16();
    drop(audio);
    let signature = signature_uri_from_pcm16le(&pcm)
        .map_err(|error| RecognitionCapture::Failed(recognition_error_message(&error)))?;
    drop(pcm);
    if stop.load(Ordering::SeqCst) {
        return Err(RecognitionCapture::Cancelled);
    }
    Ok((signature, sample_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Song {
        Song {
            id: id.to_string(),
            title: id.to_string(),
            ..Song::default()
        }
    }

    fn page(ids: &[&str], current: Option<usize>) -> QueuePage {
        QueuePage {
            title: Some("Radio".into()),
            songs: ids.iter().map(|id| song(id)).collect(),
            current_index: current,
            continuation: Some("token".into()),
        }
    }

    #[test]
    fn recognition_generation_drops_cancelled_or_replaced_runs() {
        let counter = AtomicU64::new(0);
        let first = next_generation(&counter);
        assert!(generation_is_current(&counter, first));
        let second = next_generation(&counter);
        assert!(!generation_is_current(&counter, first));
        assert!(generation_is_current(&counter, second));
        let cancelled = next_generation(&counter);
        assert!(!generation_is_current(&counter, second));
        assert!(generation_is_current(&counter, cancelled));
    }

    #[test]
    fn lyrics_translation_refuses_without_a_key_and_caches_per_video() {
        let root = std::env::temp_dir().join(format!(
            "metrolist-lyrics-translation-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let paths = crate::settings_store::SettingsPaths::new(
            root.join("config"),
            root.join("data"),
            root.join("cache"),
        );
        let ctx = egui::Context::default();
        let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
        let line = LyricsLine {
            start_ms: 0,
            text: "origin".to_string(),
            ..LyricsLine::default()
        };
        let config = TranslationConfig::new(TranslationEndpoint::DeepL, "", "en");
        services.translate_current_lyrics("v1".to_string(), vec![line.clone()], config);
        let event = services.try_recv().expect("typed missing-key event");
        match event {
            UiEvent::LyricsTranslated { result, .. } => {
                assert_eq!(result.unwrap_err(), TRANSLATION_MISSING_KEY_MESSAGE);
            }
            other => panic!("unexpected event {other:?}"),
        }

        let cached = LyricsLine {
            text: "translated".to_string(),
            ..line
        };
        services
            .lyrics_translation_cache
            .lock()
            .insert("v1".to_string(), vec![cached.clone()]);
        let config = TranslationConfig::new(TranslationEndpoint::DeepL, "test-key", "en");
        services.translate_current_lyrics("v1".to_string(), vec![cached.clone()], config.clone());
        let event = services.try_recv().expect("cached event");
        match event {
            UiEvent::LyricsTranslated {
                result, video_id, ..
            } => {
                assert_eq!(video_id, "v1");
                assert_eq!(result.expect("cached lines")[0].text, "translated");
            }
            other => panic!("unexpected event {other:?}"),
        }
        // Another video id must not reuse the cached document; without a key it
        // refuses before any HTTP client is built.
        let missing = TranslationConfig::new(TranslationEndpoint::DeepL, "", "en");
        services.translate_current_lyrics("v2".to_string(), vec![cached], missing);
        let event = services.try_recv().expect("cache miss event");
        assert!(matches!(
            event,
            UiEvent::LyricsTranslated { result: Err(message), .. }
                if message == TRANSLATION_MISSING_KEY_MESSAGE
        ));
        drop(services);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lyrics_offset_survives_a_services_restart() {
        use std::time::{Duration, Instant};
        let root = std::env::temp_dir().join(format!(
            "metrolist-lyrics-offset-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let paths = crate::settings_store::SettingsPaths::new(
            root.join("config"),
            root.join("data"),
            root.join("cache"),
        );
        let ctx = egui::Context::default();
        {
            let (services, _notes) = Services::new(&ctx, paths.clone(), &Settings::default());
            services.ui_kv_set("lyrics.offset.video123", "-500".to_string());
            let deadline = Instant::now() + Duration::from_secs(3);
            while services.ui_kv_get("lyrics.offset.video123").as_deref() != Some("-500") {
                assert!(Instant::now() < deadline, "offset write timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
        assert_eq!(
            services.ui_kv_get("lyrics.offset.video123").as_deref(),
            Some("-500")
        );
        drop(services);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lyrics_translation_generation_drops_stale_runs() {
        let counter = AtomicU64::new(0);
        let first = next_generation(&counter);
        let second = next_generation(&counter);
        assert!(!generation_is_current(&counter, first));
        assert!(generation_is_current(&counter, second));
    }

    #[test]
    fn import_rejects_unrecognized_files_before_writing_a_session() {
        use std::time::{Duration, Instant};
        let root = std::env::temp_dir().join(format!(
            "metrolist-import-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let paths = crate::settings_store::SettingsPaths::new(
            root.join("config"),
            root.join("data"),
            root.join("cache"),
        );
        let session = session_file_path(&paths.data_dir);
        let ctx = egui::Context::default();
        let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
        let bad = root.join("not-cookies.txt");
        std::fs::write(&bad, "hello world\n").expect("write bad file");
        services.import_cookie_file(bad.clone());
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut error = None;
        while Instant::now() < deadline {
            while let Some(event) = services.try_recv() {
                if let UiEvent::AccountError { message, cleared } = event {
                    assert!(!cleared);
                    error = Some(message);
                }
            }
            if error.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let error = error.expect("import error event");
        assert!(error.contains("not-cookies.txt"), "{error}");
        assert!(error.contains("recognized"), "{error}");
        assert!(!session.exists(), "invalid import must not write cookies");
        drop(services);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sign_out_deletes_the_loaded_session_file() {
        let root = std::env::temp_dir().join(format!(
            "metrolist-signout-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let paths = crate::settings_store::SettingsPaths::new(
            root.join("config"),
            root.join("data"),
            root.join("cache"),
        );
        let session = session_file_path(&paths.data_dir);
        save_session_file(
            &session,
            &CookieJar::parse("SAPISID=secret-value; Domain=.youtube.com; Path=/"),
        )
        .expect("seed session");
        let ctx = egui::Context::default();
        let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
        assert!(services.account_session().signed_in);
        assert!(session.exists());
        services.sign_out();
        assert!(!session.exists(), "sign-out must delete session cookies");
        let session = services.account_session();
        assert!(!session.signed_in);
        assert!(session.info.is_none());
        assert!(session.counts.total() == 0);
        drop(services);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn account_snapshot_upserts_the_signed_in_library() {
        use ml_core::{Album, ArtistItem, Playlist};
        let root = std::env::temp_dir().join(format!(
            "metrolist-account-sync-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let db = root.join("library.db");
        let store = Store::open(&db).expect("open store");
        let snapshot = LibrarySnapshot {
            playlists: vec![Playlist {
                id: "VLPLsync".to_string(),
                title: "Synced playlist".to_string(),
                author: Some(ml_core::Artist::new("Author", None)),
                song_count_text: Some("42 songs".to_string()),
                ..Playlist::default()
            }],
            albums: vec![Album {
                id: "MPREsync".to_string(),
                playlist_id: "OLAKsync".to_string(),
                title: "Synced album".to_string(),
                artists: vec![ml_core::Artist::new("Artist", None)],
                year: Some(2024),
                thumbnail_url: None,
                explicit: false,
                song_count: Some(1),
                duration: Some(60),
            }],
            artists: vec![ArtistItem {
                id: "UCsync".to_string(),
                name: "Synced artist".to_string(),
                thumbnail_url: None,
                channel_id: Some("UCsync".to_string()),
                subscribers: None,
            }],
            liked_songs: vec![Song {
                id: "syncedsong1".to_string(),
                title: "Synced song".to_string(),
                album: Some(
                    Album {
                        id: "MPREsync".to_string(),
                        playlist_id: "OLAKsync".to_string(),
                        title: "Synced album".to_string(),
                        artists: Vec::new(),
                        year: None,
                        thumbnail_url: None,
                        explicit: false,
                        song_count: None,
                        duration: None,
                    }
                    .as_ref(),
                ),
                ..Song::default()
            }],
        };
        let now = 1_700_000_000_000;
        let counts = apply_library_snapshot(&store, &snapshot, now).expect("apply snapshot");
        assert_eq!(
            counts,
            SyncCounts {
                songs: 1,
                albums: 1,
                artists: 1,
                playlists: 1
            }
        );
        let song = store
            .get_song("syncedsong1")
            .expect("read song")
            .expect("song exists");
        assert_eq!(song.liked, Some(now));
        assert_eq!(song.in_library, Some(now));
        let artist = store
            .get_artist("UCsync")
            .expect("read artist")
            .expect("artist exists");
        assert_eq!(artist.bookmarked_at, Some(now));
        let playlist = store
            .get_playlist("VLPLsync")
            .expect("read playlist")
            .expect("playlist exists");
        assert_eq!(playlist.bookmarked_at, Some(now));
        assert_eq!(playlist.remote_song_count, Some(42));
        assert_eq!(parse_count_text(Some("1,234 songs")), Some(1234));
        assert_eq!(parse_count_text(Some("1.2K songs")), None);
        assert_eq!(parse_count_text(Some("no count")), None);
        assert_eq!(parse_count_text(None), None);
        let albums = store
            .library_albums(ml_core::LibrarySort::RecentlyAdded, 10, 0)
            .expect("library albums");
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].id, "MPREsync");
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn session_and_corrupt_files_are_private_and_ignored() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "metrolist-session-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let data = root.join("data");
        let path = session_file_path(&data);
        let jar = CookieJar::parse("SAPISID=secret-value; Domain=.youtube.com; Path=/");
        assert!(jar.sapisid().is_some());
        save_session_file(&path, &jar).expect("save session");
        let dir_mode = std::fs::metadata(&data)
            .expect("dir meta")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = std::fs::metadata(&path)
            .expect("file meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
        let loaded = load_session_file(&path).expect("load session");
        assert!(loaded.sapisid().is_some());
        let corrupt = root.join("corrupt.txt");
        std::fs::write(&corrupt, "not a cookie file\n").expect("write corrupt");
        assert!(load_session_file(&corrupt).is_none());
        assert!(load_session_file(&root.join("missing.txt")).is_none());
        remove_session_file(&path);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn radio_state_installs_one_generation_and_clears() {
        let mut state = RadioState::default();
        assert!(!state.can_extend());
        let generation = state.begin();
        assert!(!state.can_extend(), "no page yet");
        assert!(state.install(generation, page(&["a", "b"], Some(0))));
        assert!(state.can_extend());
        state.loading = true;
        assert!(!state.can_extend(), "one fetch at a time");
        state.loading = false;

        let stale = generation;
        let current = state.begin();
        assert!(!state.install(stale, page(&["c"], Some(0))));
        assert!(!state.can_extend(), "stale page rejected");
        assert!(state.install(current, page(&["d"], Some(0))));
        assert!(state.can_extend());
        state.clear();
        assert!(!state.can_extend());
        assert!(state.page.is_none());
    }

    #[test]
    fn radio_event_uses_the_clicked_track_not_the_panel_marker() {
        let UiEvent::RadioReady {
            origin,
            songs,
            index,
            source,
        } = radio_event(song("b"), page(&["a", "b", "c"], Some(2)))
        else {
            panic!("expected radio event");
        };
        assert_eq!(origin, "b");
        assert_eq!(songs.len(), 3);
        assert_eq!(index, 1);
        assert_eq!(source, QueueSource::Radio);
    }

    #[test]
    fn radio_event_falls_back_to_the_panel_index() {
        let UiEvent::RadioReady { index, .. } =
            radio_event(song("missing"), page(&["a", "b", "c"], Some(2)))
        else {
            panic!("expected radio event");
        };
        assert_eq!(index, 2);
    }

    #[test]
    fn radio_event_ignores_an_out_of_range_panel_index() {
        let UiEvent::RadioReady { index, .. } =
            radio_event(song("missing"), page(&["a", "b"], Some(99)))
        else {
            panic!("expected radio event");
        };
        assert_eq!(index, 0);
    }

    #[test]
    fn radio_failure_is_a_single_track_that_does_not_retry() {
        let UiEvent::RadioReady {
            origin,
            songs,
            index,
            source,
        } = radio_fallback(song("kJQP7kiw5Fk"), QueueSource::List)
        else {
            panic!("expected radio event");
        };
        assert_eq!(origin, "kJQP7kiw5Fk");
        assert_eq!(songs, vec![song("kJQP7kiw5Fk")]);
        assert_eq!(index, 0);
        assert_eq!(source, QueueSource::List);
    }
}
