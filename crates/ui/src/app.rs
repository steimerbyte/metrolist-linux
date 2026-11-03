//! Application state, shell layout, routing and CLI capture modes.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{
    self, Align, CentralPanel, Context, Frame, Key, Layout, Panel, ScrollArea, Ui, Vec2,
};
use ml_core::{
    AppEvent, HomeFeed, LyricsDocument, PlaybackStatus, PlayerSnapshot, QueueSource, Song,
};
use ml_downloads::{DownloadEvent, DownloadState};
use ml_innertube::parse::url::{parse_url, ParsedUrl};

use crate::art::ArtCache;
use crate::desktop::{self, Desktop, DesktopEvent};
use crate::integrations::{self, DiscordStatus, LastFmStatus, LastFmUiState};
use crate::pure;
use crate::routes::{Route, Tab};
use crate::screens::settings::{SettingsAction, SettingsStatus};
use crate::screens::{
    account, catalog, detail, history, home, library, liked, local_playlist, lyrics, player,
    queue as queue_screen, recognize, search, settings, stats, together, Intent,
};
use crate::services::{Services, UiEvent};
use crate::settings_store::SettingsPaths;
use crate::shell::{self, MiniAction, NavAction, TopAction};
use crate::state::{
    merge_feed, AccountState, BrowseTarget, DownloadsState, FilterChoice, HistoryPending, LibTab,
    LibraryState, Load, LocalPlaylistState, PageData, PageState, PlayerExtras, RecognitionState,
    SearchState, StatsState, TogetherState,
};
use crate::theme::{install_fonts, Theme};
use crate::widgets::Toasts;
use crate::{DemoMode, RunMode};
use ml_audio::SleepTimer;
use ml_integrations::InstanceGuard;
use ml_together::{
    action, drift_decision, position_at, song_to_track_info, DriftDecision, PlaybackAction,
    RoomState, SessionEvent, TogetherEvent, TrackInfo,
};

/// Default CLI window size (desktop / expanded class).
pub const DEFAULT_WIDTH: u32 = 1100;
pub const DEFAULT_HEIGHT: u32 = 760;

#[derive(Debug, Default)]
struct TogetherRemote {
    room_code: Option<String>,
    track: Option<String>,
    queue: Vec<String>,
    playing: bool,
    received_at: Option<Instant>,
}

#[derive(Debug, Default)]
struct TogetherHost {
    room_code: Option<String>,
    track: Option<String>,
    queue: Vec<String>,
    playing: bool,
    position: u64,
    at: Option<Instant>,
    was_host: bool,
}

pub struct App {
    settings: ml_core::Settings,
    paths: SettingsPaths,
    theme: Theme,
    desktop: Desktop,
    services: Services,
    art: ArtCache,
    route: Route,
    back: Vec<Route>,
    tab: Tab,
    home: Load<HomeFeed>,
    home_chip: Option<usize>,
    search: SearchState,
    page: PageState,
    lyrics: Load<LyricsDocument>,
    lyrics_display: Option<LyricsDocument>,
    lyrics_for: Option<String>,
    lyrics_offset_ms: i64,
    lyrics_translation: Load<()>,
    lyrics_translation_generation: u64,
    translation_key: String,
    lastfm: LastFmUiState,
    lastfm_demo_status: Option<LastFmStatus>,
    discord_demo_status: Option<DiscordStatus>,
    together: TogetherState,
    together_remote: TogetherRemote,
    together_host: TogetherHost,
    together_force_sync: bool,
    snapshot: PlayerSnapshot,
    playback_error: Option<String>,
    player_open: bool,
    queue_open: bool,
    liked: bool,
    in_library: bool,
    library: LibraryState,
    downloads: DownloadsState,
    account: AccountState,
    download_limit: Option<u64>,
    liked_songs: Load<Vec<Song>>,
    local_playlist: LocalPlaylistState,
    browse: Load<ml_innertube::pages::browse::BrowsePage>,
    browse_target: Option<BrowseTarget>,
    moods: Load<ml_innertube::pages::moods::MoodsAndGenres>,
    podcasts: Load<HomeFeed>,
    podcast: Load<ml_innertube::pages::podcast::PodcastPage>,
    podcast_id: Option<String>,
    stats: StatsState,
    recognition: RecognitionState,
    recognition_generation: u64,
    history_pending: Option<HistoryPending>,
    open_play_local: bool,
    theme_key: (ml_core::ThemeMode, u32),
    settings_dirty_at: Option<Instant>,
    extras: PlayerExtras,
    muted: bool,
    window_visible: bool,
    demo: Option<DemoMode>,
    demo_lyrics_pending: bool,
    toasts: Toasts,
    mode: RunMode,
    frames: u32,
    backend: String,
    screenshot_requested: bool,
    screenshot_frame: u32,
    screenshot_idle_frames: u32,
    smoke_reported: bool,
    open_play: bool,
    last_played_id: Option<String>,
    radio_for: Option<String>,
    suggest_for: String,
    /// Exit code recorded by the screenshot callback, which cannot borrow the
    /// app; `after_frame` performs the shutdown and exit.
    cli_exit: Arc<AtomicI32>,
}

/// Destination for a YouTube link pasted into the search box. `None` means the
/// link is played (video URLs start that video's radio), matching upstream
/// `handleSearch`. Album `list=` links carry a playlist id, so they open the
/// playlist page unless the id is already an `MPRE…` album browse id.
fn url_route(parsed: &ParsedUrl) -> Option<Route> {
    Some(match parsed {
        ParsedUrl::Video(_) => return None,
        ParsedUrl::Playlist(id) => Route::Playlist { id: id.clone() },
        ParsedUrl::Album(id) if id.starts_with("MPRE") => Route::Album { id: id.clone() },
        ParsedUrl::Album(id) => Route::Playlist { id: id.clone() },
        ParsedUrl::Artist(id) => Route::Artist { id: id.clone() },
    })
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        options: crate::RunOptions,
        instance: Option<InstanceGuard>,
    ) -> Self {
        // Graceful SIGTERM/SIGINT: without it mpv children and both socket
        // files outlive the app.
        crate::signals::install(cc.egui_ctx.clone());
        install_fonts(&cc.egui_ctx);
        let paths = SettingsPaths::from_env();
        let settings = paths.load();
        let theme = Theme::from_settings(&settings);
        theme.install(&cc.egui_ctx);
        if let Some(path) = paths.ensure_settings_file() {
            println!("settings: wrote defaults to {}", path.display());
        }

        let backend = if let Some(render_state) = &cc.wgpu_render_state {
            let info = render_state.adapter.get_info();
            println!(
                "renderer: wgpu backend={:?} adapter={:?} driver={:?}",
                info.backend, info.name, info.driver_info
            );
            format!("{:?}", info.backend)
        } else {
            println!("renderer: wgpu render state unavailable");
            "unknown".to_string()
        };
        println!(
            "theme: {} seed=#{:06X} surface=#{:02X}{:02X}{:02X} primary=#{:02X}{:02X}{:02X}",
            if theme.dark { "dark" } else { "light" },
            settings.seed_color & 0xFFFFFF,
            theme.scheme.surface.r(),
            theme.scheme.surface.g(),
            theme.scheme.surface.b(),
            theme.scheme.primary.r(),
            theme.scheme.primary.g(),
            theme.scheme.primary.b(),
        );
        if settings.dynamic_accent {
            eprintln!("note: dynamic accent is reserved for the portal reader; using seed #ED5564");
        }

        let (services, mut notes) = Services::new(&cc.egui_ctx, paths.clone(), &settings);
        let downloads = DownloadsState {
            items: services.download_list(),
            used: services.download_usage(),
            limit: services.download_active_limit(),
            error: services.downloads_error().map(str::to_owned),
            confirm_remove: None,
        };
        let download_limit = services.download_limit();
        let account = {
            let (session, cache) = services.account_startup();
            AccountState {
                session,
                cache,
                browser_profile: services.browser_profile_name(),
                ..AccountState::default()
            }
        };
        let mut extras = PlayerExtras::from_kv(|key| services.ui_kv_get(key));
        // One-time migration from the old settings.json sleep field.
        if services.ui_kv_get("player.sleep_timer").is_none() {
            if let Some(minutes) = settings.sleep_timer_minutes {
                extras.sleep = SleepTimer::Minutes(minutes.min(24 * 60));
            }
        }
        let lastfm_path = integrations::secrets_path(&paths.data_dir);
        let lastfm = match integrations::load_secrets(&lastfm_path) {
            Ok(secrets) => LastFmUiState::from_secrets(secrets, extras.lastfm_enabled),
            Err(error) => {
                notes.push(format!("Last.fm credentials unavailable: {error}"));
                LastFmUiState {
                    enabled: extras.lastfm_enabled,
                    ..LastFmUiState::default()
                }
            }
        };
        let eq_bands = if extras.eq_enabled {
            crate::pure::clamp_eq_bands(&extras.eq_preset.bands())
        } else {
            [0.0; 10]
        };
        services.set_eq(&eq_bands);
        services.set_crossfade(extras.crossfade_seconds, extras.crossfade_gapless);
        services.set_skip_silence(extras.skip_silence);
        services.set_normalization(extras.normalization, Some(extras.normalization_gain_db));
        services.set_speed(extras.speed);
        services.set_device(extras.device.clone());
        if !matches!(extras.sleep, SleepTimer::Off) {
            services.set_sleep_timer(extras.sleep);
        }
        let (desktop, integration_notes) = Desktop::start(
            services.controller_handle(),
            instance,
            extras.notifications,
            cc.egui_ctx.clone(),
        );
        notes.extend(integration_notes);
        let toasts = Toasts::new();
        let mut search = SearchState {
            history: services.search_history(12),
            ..SearchState::default()
        };
        let home = Load {
            loading: true,
            ..Load::default()
        };
        let together_name = account
            .session
            .info
            .as_ref()
            .map(|info| info.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Listener".to_string());
        let theme_key = (settings.theme_mode, settings.seed_color);
        let demo = options.demo;
        services.load_home();
        if search.query.is_empty() {
            search.submitted = String::new();
        }

        let mut app = Self {
            settings,
            paths,
            theme,
            desktop,
            services,
            art: ArtCache::new(),
            route: Route::Home,
            back: Vec::new(),
            tab: Tab::Home,
            home,
            home_chip: None,
            search,
            page: PageState::default(),
            lyrics: Load::default(),
            lyrics_display: None,
            lyrics_for: None,
            lyrics_offset_ms: 0,
            lyrics_translation: Load::default(),
            lyrics_translation_generation: 0,
            translation_key: String::new(),
            lastfm,
            lastfm_demo_status: None,
            discord_demo_status: None,
            together: TogetherState::new(together_name),
            together_remote: TogetherRemote::default(),
            together_host: TogetherHost::default(),
            together_force_sync: false,
            snapshot: PlayerSnapshot::default(),
            playback_error: None,
            player_open: false,
            queue_open: false,
            liked: false,
            in_library: false,
            library: LibraryState::default(),
            downloads,
            account,
            download_limit,
            liked_songs: Load::default(),
            local_playlist: LocalPlaylistState::default(),
            browse: Load::default(),
            browse_target: None,
            moods: Load::default(),
            podcasts: Load::default(),
            podcast: Load::default(),
            podcast_id: None,
            stats: StatsState::default(),
            recognition: RecognitionState::default(),
            recognition_generation: 0,
            history_pending: None,
            open_play_local: false,
            theme_key,
            settings_dirty_at: None,
            extras,
            muted: false,
            window_visible: true,
            demo,
            demo_lyrics_pending: false,
            toasts,
            mode: options.mode,
            frames: 0,
            backend,
            screenshot_requested: false,
            screenshot_frame: 0,
            screenshot_idle_frames: 0,
            smoke_reported: false,
            open_play: false,
            last_played_id: None,
            radio_for: None,
            suggest_for: String::new(),
            cli_exit: Arc::new(AtomicI32::new(-1)),
        };
        for note in notes {
            app.toasts.push(note);
        }
        match demo {
            Some(DemoMode::Player) => {
                app.player_open = true;
                app.start_player_demo();
            }
            Some(DemoMode::Queue) => {
                app.player_open = true;
                app.queue_open = true;
                app.start_demo();
            }
            Some(DemoMode::Lyrics) => {
                app.start_demo();
                app.demo_lyrics_pending = true;
            }
            Some(DemoMode::Library) => app.switch_tab(Tab::Library),
            Some(DemoMode::Settings) => app.navigate(Route::Settings),
            Some(DemoMode::SettingsAudio) => app.navigate(Route::Settings),
            Some(DemoMode::History) => app.navigate(Route::History),
            Some(DemoMode::Liked) => app.navigate(Route::Liked),
            Some(DemoMode::Downloads) => app.start_download_demo(),
            Some(DemoMode::Account) => app.navigate(Route::Account),
            Some(DemoMode::Charts) => app.navigate(Route::Charts),
            Some(DemoMode::Moods) => app.navigate(Route::Moods),
            Some(DemoMode::NewReleases) => app.navigate(Route::NewReleases),
            Some(DemoMode::Podcasts) => app.navigate(Route::Podcasts),
            Some(DemoMode::Stats) => app.navigate(Route::Stats),
            Some(DemoMode::Wrapped) => app.navigate(Route::Wrapped),
            Some(DemoMode::Recognize) => app.navigate(Route::Recognize),
            Some(DemoMode::RecognizeListening) => {
                app.recognition
                    .start(Instant::now() - Duration::from_millis(4200));
                app.navigate(Route::Recognize);
            }
            Some(DemoMode::RecognizeSuccess) => {
                app.recognition.succeed(recognize::demo_match());
                app.navigate(Route::Recognize);
            }
            Some(DemoMode::RecognizeError) => {
                app.recognition.fail(recognize::demo_error());
                app.navigate(Route::Recognize);
            }
            Some(
                DemoMode::Integrations
                | DemoMode::IntegrationsConnected
                | DemoMode::IntegrationsError,
            ) => {
                app.navigate(Route::Settings);
                app.setup_integrations_demo(demo.expect("integration demo"));
            }
            Some(DemoMode::Together) => app.switch_tab(Tab::Together),
            Some(DemoMode::TogetherConnected) => {
                app.switch_tab(Tab::Together);
                app.together = together::demo_state(true);
                app.together_host.was_host = true;
            }
            Some(DemoMode::TogetherGuest) => {
                app.switch_tab(Tab::Together);
                app.together = together::demo_state(false);
            }
            None => {}
        }
        if !matches!(
            demo,
            Some(
                DemoMode::Integrations
                    | DemoMode::IntegrationsConnected
                    | DemoMode::IntegrationsError
            )
        ) {
            app.configure_integrations(true);
        }
        let argv: Vec<String> = std::env::args().skip(1).collect();
        if let Some(video_id) = desktop::deep_link_video(&argv) {
            app.play_shared(&video_id);
        }
        app
    }

    fn setup_integrations_demo(&mut self, demo: DemoMode) {
        self.lastfm.enabled = true;
        self.extras.lastfm_enabled = true;
        self.extras.discord_enabled = true;
        self.lastfm_demo_status = Some(LastFmStatus::default());
        self.discord_demo_status = Some(DiscordStatus {
            enabled: true,
            waiting: true,
            ..DiscordStatus::default()
        });
        if demo == DemoMode::IntegrationsConnected || demo == DemoMode::IntegrationsError {
            self.lastfm.api_key = "demo-api-key-not-real".to_string();
            self.lastfm.secret = "demo-shared-secret-not-real".to_string();
            self.lastfm.username = "demo-listener".to_string();
        }
        if demo == DemoMode::IntegrationsConnected {
            self.lastfm.session = "demo-session-not-real".to_string();
            self.lastfm_demo_status = Some(LastFmStatus {
                signed_in: true,
                user: Some("demo-listener".to_string()),
                scrobbles_sent: 128,
                queued: 3,
                ..LastFmStatus::default()
            });
            self.discord_demo_status = Some(DiscordStatus {
                enabled: true,
                connected: true,
                ..DiscordStatus::default()
            });
        } else if demo == DemoMode::IntegrationsError {
            self.lastfm.error =
                Some("Last.fm session rejected. Reconnect to continue.".to_string());
            self.lastfm_demo_status = Some(LastFmStatus {
                scrobbles_sent: 42,
                queued: 7,
                ..LastFmStatus::default()
            });
            self.discord_demo_status = Some(DiscordStatus {
                enabled: true,
                waiting: true,
                ..DiscordStatus::default()
            });
        }
    }

    /// CLI demo: the only allowed screenshot track. The `next` radio queue
    /// replaces it with full metadata once resolved.
    const DEMO_SONG_ID: &str = "oWqnJdGy_84";

    fn demo_song() -> Song {
        Song {
            id: Self::DEMO_SONG_ID.to_string(),
            title: "Empty Streets".to_string(),
            artists: vec![ml_core::Artist::new("LONOWN", None)],
            duration: Some(232),
            ..Song::default()
        }
    }

    fn start_demo(&mut self) {
        let song = Self::demo_song();
        self.radio_for = Some(song.id.clone());
        self.services.play_song(song);
    }

    /// Player screenshots must not enqueue or play anything besides the one
    /// approved fixture, so start a single-track queue without radio.
    fn start_player_demo(&mut self) {
        // Screenshot playback stays muted; an unmuted null sink can reject the
        // stream-unmute property and abort the track before it renders.
        self.muted = true;
        self.services.set_muted(true);
        let song = Self::demo_song();
        self.play_songs(vec![song], 0, QueueSource::List);
    }

    fn start_download_demo(&mut self) {
        self.library.tab = LibTab::Downloads;
        self.switch_tab(Tab::Library);
        let complete = self
            .services
            .download_list()
            .iter()
            .any(|entry| entry.state == DownloadState::Completed);
        if !complete {
            self.services.enqueue_downloads(&[Self::demo_song()]);
        }
        self.reload_downloads();
    }

    pub fn settings(&self) -> &ml_core::Settings {
        &self.settings
    }

    pub fn paths(&self) -> &SettingsPaths {
        &self.paths
    }

    // ------------------------------------------------------------- services

    fn drain_events(&mut self, ctx: &Context) {
        while let Some(event) = self.services.try_recv() {
            match event {
                UiEvent::Core(event) => self.apply_core_event(event),
                UiEvent::HomeMore(feed) => {
                    if !self.home.more_loading {
                        continue;
                    }
                    let continuation = feed.continuation.clone();
                    if let Some(home) = self.home.data.as_mut() {
                        merge_feed(home, feed);
                    } else {
                        self.home.data = Some(feed);
                    }
                    self.home.more = continuation;
                    self.home.more_loading = false;
                    self.home.loading = false;
                }
                UiEvent::SearchMore(page) => {
                    if !self.search.results.more_loading {
                        continue;
                    }
                    if let Some(results) = self.search.results.data.as_mut() {
                        let known: std::collections::HashSet<String> = results
                            .items
                            .iter()
                            .map(|item| item.id().to_owned())
                            .collect();
                        results.items.extend(
                            page.items
                                .into_iter()
                                .filter(|item| !known.contains(item.id())),
                        );
                        results.songs.extend(page.songs);
                        results.continuation = page.continuation.clone();
                    }
                    self.search.results.more = page.continuation;
                    self.search.results.more_loading = false;
                }
                UiEvent::AlbumMore(page) => {
                    if !self.page.data.more_loading {
                        continue;
                    }
                    if let Some(PageData::Album(current)) = self.page.data.data.as_mut() {
                        current.songs.extend(page.songs);
                        current.continuation = page.continuation.clone();
                    }
                    self.page.data.more = page.continuation;
                    self.page.data.more_loading = false;
                }
                UiEvent::ArtistMore(feed) => {
                    if !self.page.data.more_loading {
                        continue;
                    }
                    let continuation = feed.continuation.clone();
                    if let Some(PageData::Artist(current)) = self.page.data.data.as_mut() {
                        for section in feed.sections {
                            current.sections.push(section);
                        }
                        current.continuation = continuation.clone();
                    }
                    self.page.data.more = continuation;
                    self.page.data.more_loading = false;
                }
                UiEvent::PlaylistMore(page) => {
                    if !self.page.data.more_loading {
                        continue;
                    }
                    if let Some(PageData::Playlist(current)) = self.page.data.data.as_mut() {
                        current.songs.extend(page.songs);
                        current.continuation = page.continuation.clone();
                    }
                    self.page.data.more = page.continuation;
                    self.page.data.more_loading = false;
                }
                UiEvent::LibraryLoaded(data) => self.library.data.finish(data),
                UiEvent::LikedLoaded(songs) => self.liked_songs.finish(songs),
                UiEvent::LocalPlaylistLoaded(view) => {
                    if self.local_playlist.id.as_deref() == Some(view.id.as_str()) {
                        self.local_playlist.data.finish(view.clone());
                        if self.open_play_local {
                            self.open_play_local = false;
                            if view.songs.is_empty() {
                                self.toasts.push("Playlist is empty");
                            } else {
                                self.play_songs(view.songs, 0, QueueSource::List);
                            }
                        }
                    }
                }
                UiEvent::LibraryDirty { playlist } => {
                    self.library.data.loading = true;
                    self.services.load_library(self.settings.library_sort);
                    if let Some(id) = playlist {
                        if self.local_playlist.id.as_deref() == Some(id.as_str()) {
                            self.local_playlist.data.loading = true;
                            self.services.load_local_playlist(id);
                        }
                    }
                    if matches!(self.route, Route::Liked) {
                        self.liked_songs.loading = true;
                        self.services.load_liked();
                    }
                    if matches!(self.route, Route::Stats | Route::Wrapped)
                        && !matches!(self.demo, Some(DemoMode::Stats | DemoMode::Wrapped))
                    {
                        self.stats.copied = false;
                        self.stats.load.begin();
                        self.services.load_stats(self.stats.period);
                    }
                }
                UiEvent::AccountUpdated { session, cache } => {
                    let signed_in = session.signed_in;
                    let details_pending = signed_in && session.info.is_none();
                    self.account.session = session;
                    self.account.cache = cache;
                    self.account.loading = details_pending;
                    if signed_in && !details_pending {
                        self.account.error = None;
                    }
                }
                UiEvent::AccountError { message, cleared } => {
                    self.account.loading = false;
                    self.account.error = Some(message.clone());
                    if cleared {
                        self.account.session = self.services.account_session();
                        self.account.cache = None;
                    }
                    self.toasts.push(message);
                }
                UiEvent::AccountSignedOut => {
                    self.account.session = self.services.account_session();
                    self.account.cache = None;
                    self.account.error = None;
                    self.account.loading = false;
                    self.toasts.push("Signed out");
                }
                UiEvent::LibrarySynced { counts, synced_at } => {
                    self.account.syncing = false;
                    self.account.error = None;
                    self.account.session.counts = counts;
                    self.account.session.synced_at = Some(synced_at);
                    if let Some(cache) = self.account.cache.as_mut() {
                        cache.counts = counts;
                        cache.synced_at = Some(synced_at);
                    }
                    self.toasts.push(format!(
                        "Library synced: {} songs, {} albums, {} artists, {} playlists",
                        counts.songs, counts.albums, counts.artists, counts.playlists
                    ));
                    // The service also emits `LibraryDirty`, which reloads the
                    // open library screen exactly once.
                }
                UiEvent::LibrarySyncFailed(message) => {
                    self.account.syncing = false;
                    self.account.error = Some(message.clone());
                    self.toasts.push(message);
                }
                UiEvent::StatsLoaded { period, result } => {
                    if period != self.stats.period {
                        continue;
                    }
                    self.stats.copied = false;
                    match result {
                        Ok(report) => self.stats.load.finish(report),
                        Err(error) => self.stats.load.fail(error),
                    }
                }
                UiEvent::RecognitionProcessing { generation } => {
                    if generation == self.recognition_generation {
                        self.recognition.processing();
                    }
                }
                UiEvent::Recognized { generation, result } => {
                    if generation != self.recognition_generation {
                        continue;
                    }
                    match result {
                        Ok(matched) => self.recognition.succeed(matched),
                        Err(message) => self.recognition.fail(message),
                    }
                }
                UiEvent::RecognitionPlayReady { generation, songs } => {
                    if generation != self.recognition_generation {
                        continue;
                    }
                    let RecognitionState::Success { matched } = &self.recognition else {
                        continue;
                    };
                    let song = match songs {
                        Ok(songs) => pure::best_recognition_match(&matched.result, &songs)
                            .and_then(|index| songs.get(index).cloned())
                            .or_else(|| pure::recognition_fallback_song(&matched.result)),
                        Err(error) => {
                            self.toasts
                                .push(format!("Recognition search failed: {error}"));
                            pure::recognition_fallback_song(&matched.result)
                        }
                    };
                    match song {
                        Some(song) => self.play_songs(vec![song], 0, QueueSource::List),
                        None => self
                            .toasts
                            .push("No playable YouTube Music match was found.".to_string()),
                    }
                }
                UiEvent::LyricsTranslated {
                    generation,
                    video_id,
                    result,
                } => {
                    if generation != self.lyrics_translation_generation
                        || self.lyrics_for.as_deref() != Some(video_id.as_str())
                    {
                        continue;
                    }
                    match result {
                        Ok(lines) => {
                            self.lyrics_translation.finish(());
                            if let Some(document) = self.lyrics.data.as_mut() {
                                document.translated = Some(lines);
                                self.rebuild_lyrics_display();
                            }
                        }
                        Err(message) => self.lyrics_translation.fail(message),
                    }
                }
                UiEvent::LastFmConnected { result } => {
                    if !self.lastfm.connecting {
                        continue;
                    }
                    self.lastfm.connecting = false;
                    match result {
                        Ok(session) => {
                            self.lastfm.session = session;
                            self.lastfm.password.clear();
                            self.lastfm.error = None;
                            self.persist_lastfm();
                            self.configure_integrations(true);
                            let user = self.lastfm.username.trim();
                            self.toasts.push(if user.is_empty() {
                                "Connected to Last.fm".to_string()
                            } else {
                                format!("Connected to Last.fm as {user}")
                            });
                        }
                        Err(message) => self.lastfm.error = Some(message),
                    }
                }
                UiEvent::IntegrationNotice {
                    message,
                    lastfm_disabled,
                } => {
                    if lastfm_disabled {
                        self.lastfm.error = Some(message.clone());
                    }
                    self.toasts.push(message);
                    if lastfm_disabled {
                        self.lastfm.enabled = false;
                        self.lastfm.session.clear();
                        self.lastfm.password.clear();
                        self.lastfm.connecting = false;
                        self.extras.lastfm_enabled = false;
                        self.persist_extras();
                        self.persist_lastfm();
                        self.services.lastfm_sign_out();
                    }
                }
                UiEvent::BrowseLoaded { target, result } => {
                    if self.browse_target.as_ref() != Some(&target)
                        || !target.matches_route(&self.route)
                    {
                        continue;
                    }
                    match result {
                        Ok(page) => {
                            let continuation = page.continuation.clone();
                            self.browse.finish(page);
                            self.browse.more = continuation;
                        }
                        Err(error) => self.browse.fail(error),
                    }
                }
                UiEvent::BrowseMore { target, result } => {
                    if self.browse_target.as_ref() != Some(&target)
                        || !target.matches_route(&self.route)
                        || !self.browse.more_loading
                    {
                        continue;
                    }
                    match result {
                        Ok(page) => {
                            let continuation = page.continuation;
                            if let Some(current) = self.browse.data.as_mut() {
                                crate::state::merge_sections(&mut current.sections, page.sections);
                                current.continuation = continuation.clone();
                            }
                            self.browse.more = continuation;
                            self.browse.more_loading = false;
                        }
                        Err(error) => {
                            self.browse.more_loading = false;
                            self.toasts.push(error);
                        }
                    }
                }
                UiEvent::MoodsLoaded(result) => {
                    if self.route != Route::Moods {
                        continue;
                    }
                    match result {
                        Ok(moods) => self.moods.finish(moods),
                        Err(error) => self.moods.fail(error),
                    }
                }
                UiEvent::PodcastsLoaded(result) => {
                    if self.route != Route::Podcasts {
                        continue;
                    }
                    match result {
                        Ok(feed) => self.podcasts.finish(feed),
                        Err(error) => self.podcasts.fail(error),
                    }
                }
                UiEvent::PodcastLoaded { id, result } => {
                    if self.podcast_id.as_deref() != Some(id.as_str())
                        || !matches!(&self.route, Route::Podcast { id: current } if current == &id)
                    {
                        continue;
                    }
                    match result {
                        Ok(page) => {
                            let continuation = page.continuation.clone();
                            self.podcast.finish(page);
                            self.podcast.more = continuation;
                        }
                        Err(error) => self.podcast.fail(error),
                    }
                }
                UiEvent::PodcastMore { id, result } => {
                    if self.podcast_id.as_deref() != Some(id.as_str())
                        || !matches!(&self.route, Route::Podcast { id: current } if current == &id)
                        || !self.podcast.more_loading
                    {
                        continue;
                    }
                    match result {
                        Ok((episodes, continuation)) => {
                            if let Some(page) = self.podcast.data.as_mut() {
                                let known: std::collections::HashSet<String> = page
                                    .episodes
                                    .iter()
                                    .map(|episode| episode.id.clone())
                                    .collect();
                                page.episodes.extend(
                                    episodes
                                        .into_iter()
                                        .filter(|episode| !known.contains(&episode.id)),
                                );
                                page.continuation = continuation.clone();
                            }
                            self.podcast.more = continuation;
                            self.podcast.more_loading = false;
                        }
                        Err(error) => {
                            self.podcast.more_loading = false;
                            self.toasts.push(error);
                        }
                    }
                }
                UiEvent::Devices(result) => match result {
                    Ok(devices) => self.extras.devices.finish(devices),
                    Err(error) => self.extras.devices.fail(error),
                },
                UiEvent::Download(event) => self.apply_download_event(event),
                UiEvent::PlaylistCreated { id, name } => {
                    self.toasts.push(format!("Created “{name}”"));
                    self.library.tab = LibTab::Playlists;
                    self.navigate(Route::LocalPlaylist { id });
                }
                UiEvent::PlaylistDeleted { id } => {
                    self.toasts.push("Playlist deleted");
                    if matches!(&self.route, Route::LocalPlaylist { id: current } if current == &id)
                    {
                        self.switch_tab(Tab::Library);
                    }
                    if self.local_playlist.id.as_deref() == Some(id.as_str()) {
                        self.local_playlist = LocalPlaylistState::default();
                    }
                }
                UiEvent::Suggestions { query, items } => {
                    if query == self.search.query.trim() && self.search.suggestions_open {
                        self.search.suggestions = items;
                    }
                }
                UiEvent::Art { url, image } => self.art.insert(ctx, &url, image),
                UiEvent::Resolved => {}
                UiEvent::RadioReady {
                    origin,
                    songs,
                    index,
                    source,
                } => {
                    // Ignore a radio response for a song the user already replaced.
                    if self.radio_for.as_deref() == Some(origin.as_str()) {
                        self.playback_error = None;
                        let radio = source == QueueSource::Radio;
                        if !radio {
                            self.radio_for = None;
                            self.services.clear_radio();
                        }
                        self.services.play(songs, index, source);
                        if radio {
                            // Fetch one page immediately; later pages follow
                            // automatically as the queue approaches its end.
                            self.services.extend_radio(true);
                        }
                    }
                }
                UiEvent::RadioExtended { .. } => {}
                UiEvent::PlaybackFailed { video_id, message } => {
                    let current = self.snapshot.current.as_ref().map(|song| song.id.as_str());
                    if current == Some(video_id.as_str()) {
                        self.playback_error = Some(message.clone());
                        // Stop so the transport stops claiming "loading"; Play retries.
                        self.services.stop();
                    }
                    self.toasts.push(message);
                }
            }
        }
    }

    fn apply_core_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::HomeLoaded(feed) => {
                let continuation = feed.continuation.clone();
                self.home.finish(feed);
                self.home.more = continuation;
            }
            AppEvent::SearchLoaded(page) => {
                let matches = page
                    .query
                    .trim()
                    .eq_ignore_ascii_case(&self.search.submitted);
                if !matches {
                    return;
                }
                let continuation = page.continuation.clone();
                self.search.results.finish(page);
                self.search.results.more = continuation;
            }
            AppEvent::AlbumLoaded(page) => {
                let Route::Album { id } = &self.route else {
                    return;
                };
                if &page.album.id != id && &page.album.playlist_id != id {
                    return;
                }
                let continuation = page.continuation.clone();
                let songs = page.songs.clone();
                self.page.data.finish(PageData::Album(page));
                self.page.data.more = continuation;
                if self.open_play && !songs.is_empty() {
                    self.open_play = false;
                    self.play_songs(songs, 0, QueueSource::Album);
                }
            }
            AppEvent::ArtistLoaded(page) => {
                let Route::Artist { id } = &self.route else {
                    return;
                };
                let same = &page.artist.id == id || page.artist.channel_id.as_deref() == Some(id);
                if !same {
                    return;
                }
                let continuation = page.continuation.clone();
                let songs = page.top_songs.clone();
                self.page.data.finish(PageData::Artist(page));
                self.page.data.more = continuation;
                if self.open_play && !songs.is_empty() {
                    self.open_play = false;
                    self.play_songs(songs, 0, QueueSource::Artist);
                }
            }
            AppEvent::PlaylistLoaded(page) => {
                let Route::Playlist { id } = &self.route else {
                    return;
                };
                let wanted = id.trim_start_matches("VL");
                let got = page.playlist.id.trim_start_matches("VL");
                if wanted != got {
                    return;
                }
                let continuation = page.continuation.clone();
                let songs = page.songs.clone();
                self.page.data.finish(PageData::Playlist(page));
                self.page.data.more = continuation;
                if self.open_play && !songs.is_empty() {
                    self.open_play = false;
                    self.play_songs(songs, 0, QueueSource::Playlist);
                }
            }
            AppEvent::LyricsLoaded(document) => {
                self.lyrics.finish(document);
            }
            AppEvent::PlayerState(snapshot) => {
                self.snapshot = snapshot;
            }
            AppEvent::Error(error) => {
                let message = error.to_string();
                if self.page.data.loading {
                    self.open_play = false;
                }
                if self.home.loading || self.home.more_loading {
                    self.home.fail(message.clone());
                }
                if self.search.results.loading || self.search.results.more_loading {
                    self.search.results.fail(message.clone());
                }
                if self.page.data.loading || self.page.data.more_loading {
                    self.page.data.fail(message.clone());
                }
                if self.lyrics.loading {
                    self.lyrics.fail(message.clone());
                }
                if self.library.data.loading {
                    self.library.data.fail(message.clone());
                }
                if self.liked_songs.loading {
                    self.liked_songs.fail(message.clone());
                }
                if self.local_playlist.data.loading {
                    self.local_playlist.data.fail(message.clone());
                }
                self.toasts.push(message);
            }
            AppEvent::Toast(message) => self.toasts.push(message),
            AppEvent::QueueLoaded(_) | AppEvent::LibraryLoaded(_) | AppEvent::StreamResolved(_) => {
            }
        }
    }

    fn poll_player(&mut self) {
        for event in self.services.poll() {
            match event {
                ml_audio::ControllerEvent::Snapshot(snapshot) => {
                    self.snapshot = snapshot;
                }
                ml_audio::ControllerEvent::QueueChanged(_) => {
                    self.services.persist_queue();
                }
                ml_audio::ControllerEvent::NeedUrl { video_id, .. } => {
                    self.services.resolve(video_id);
                }
                ml_audio::ControllerEvent::Error(message) => {
                    self.playback_error = Some(message.clone());
                    self.toasts.push(message);
                }
                ml_audio::ControllerEvent::DeviceChanged {
                    fallback_from: Some(from),
                    ..
                } => {
                    self.extras.device = Some("auto".to_string());
                    self.persist_extras();
                    self.toasts.push(format!(
                        "Audio device \"{from}\" unavailable; using Automatic."
                    ));
                }
                ml_audio::ControllerEvent::DeviceChanged {
                    fallback_from: None,
                    ..
                } => {}
                ml_audio::ControllerEvent::EndOfQueue => {
                    self.toasts.push("Queue finished");
                }
            }
        }
        let sleep = self.services.sleep_timer();
        if self.extras.sleep != sleep {
            self.extras.sleep = sleep;
            self.persist_extras();
        }
        if self.radio_for.is_some() && self.services.radio_can_extend() {
            self.services.extend_radio(false);
        }
        self.services.lastfm_snapshot(&self.snapshot);
        self.services.discord_snapshot(&self.snapshot);
    }

    fn poll_together(&mut self, ctx: &Context) {
        let (generation, events) = self.services.poll_together();
        for event in events {
            let snapshot = matches!(
                &event,
                TogetherEvent::Session(session) if matches!(session.as_ref(), SessionEvent::Snapshot(_))
            );
            let terminal = matches!(
                &event,
                TogetherEvent::Session(session)
                    if matches!(
                        session.as_ref(),
                        SessionEvent::JoinRejected { .. } | SessionEvent::Kicked { .. }
                    )
            ) || matches!(
                &event,
                TogetherEvent::Disconnected {
                    will_reconnect: false
                }
            );
            if self.together.apply_event(generation, event) {
                if snapshot {
                    self.together_remote.received_at = Some(Instant::now());
                }
                if terminal {
                    let new_generation = self.services.leave_together_room();
                    self.together.generation = new_generation;
                    self.together_host = TogetherHost::default();
                    self.together_remote = TogetherRemote::default();
                    self.together_force_sync = false;
                }
                ctx.request_repaint();
            }
        }
        self.apply_together_guest();
        self.sync_together_host();
    }

    fn together_target_position(&self, room: &RoomState) -> u64 {
        let elapsed = self
            .together_remote
            .received_at
            .map(|at| at.elapsed().as_millis() as i64)
            .unwrap_or(0);
        let effective = (room.last_update > 0).then_some(room.last_update);
        // The real server stamps wall-clock epoch milliseconds. Use the local
        // wall clock for late joins; synthetic/demo timestamps fall back to
        // elapsed time since this snapshot arrived.
        let now = if room.last_update > 1_000_000_000_000 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as i64)
                .ok()
        } else {
            None
        };
        let now = now.or_else(|| effective.map(|time| time.saturating_add(elapsed)));
        position_at(room.position, effective, room.is_playing, now).max(0) as u64
    }

    fn apply_together_guest(&mut self) {
        if self.demo.is_some() {
            return;
        }
        if !self.together.is_guest() {
            self.together_remote = TogetherRemote::default();
            return;
        }
        let Some(room) = self.together.room().cloned() else {
            return;
        };
        let Some(code) = self.together.room_code().map(str::to_owned) else {
            return;
        };
        if self.together_remote.room_code.as_deref() != Some(code.as_str()) {
            self.together_remote = TogetherRemote {
                room_code: Some(code),
                received_at: self.together_remote.received_at,
                ..TogetherRemote::default()
            };
        }
        let track = room.current_track.as_ref().map(|track| track.id.clone());
        let queue_ids: Vec<String> = room.queue.iter().map(|track| track.id.clone()).collect();
        if track != self.together_remote.track {
            if room.current_track.is_some() {
                self.services.install_together_queue(&room);
                let target = self.together_target_position(&room);
                if target > 0 {
                    self.services.seek(target);
                }
                if !room.is_playing {
                    self.services.toggle_play();
                }
            } else {
                self.services.stop();
            }
            self.together_remote.track = track;
            self.together_remote.queue = queue_ids;
            self.together_remote.playing = room.is_playing;
            return;
        }
        self.together_remote.queue = queue_ids;
        if room.is_playing != self.together_remote.playing {
            self.services.toggle_play();
            self.together_remote.playing = room.is_playing;
        }
        let target = self.together_target_position(&room);
        let decision = drift_decision(
            target as i64,
            self.snapshot.position_ms as i64,
            self.snapshot.current.is_some(),
        );
        if let DriftDecision::Seek(position) = decision {
            self.services.seek(position.max(0) as u64);
        }
    }

    fn sync_together_host(&mut self) {
        if self.demo.is_some() {
            return;
        }
        if !self.together.is_host() {
            self.together_host.was_host = false;
            return;
        }
        let Some(code) = self.together.room_code().map(str::to_owned) else {
            return;
        };
        let name = self.together.display_name.trim().to_string();
        let (songs, position) = self.services.queue_display();
        let current = self.snapshot.current.as_ref().map(|song| {
            let mut info = song_to_track_info(song);
            info.suggested_by = name.clone();
            info
        });
        let upcoming: Vec<TrackInfo> = songs
            .iter()
            .skip(position.map_or(0, |index| index + 1))
            .map(|song| {
                let mut info = song_to_track_info(song);
                info.suggested_by = name.clone();
                info
            })
            .collect();
        let queue_ids: Vec<String> = upcoming.iter().map(|track| track.id.clone()).collect();
        let track_id = current.as_ref().map(|track| track.id.clone());
        let playing = self.snapshot.is_playing();
        let player_position = self.snapshot.position_ms;
        let force = self.together_force_sync
            || !self.together_host.was_host
            || self.together_host.room_code.as_deref() != Some(code.as_str());
        if force || track_id != self.together_host.track {
            if let Some(track) = current.clone() {
                let mut change = PlaybackAction::new(action::CHANGE_TRACK);
                change.track_info = Some(track);
                change.queue = Some(upcoming.clone());
                self.services.together_playback(change);
                self.publish_together_playback(track_id.clone(), playing, player_position);
            } else {
                let mut sync = PlaybackAction::new(action::SYNC_QUEUE);
                sync.queue = Some(upcoming.clone());
                self.services.together_playback(sync);
            }
            self.together_host.room_code = Some(code);
            self.together_host.track = track_id;
            self.together_host.queue = queue_ids;
            self.together_host.playing = playing;
            self.together_host.position = player_position;
            self.together_host.at = Some(Instant::now());
            self.together_host.was_host = true;
            self.together_force_sync = false;
            return;
        }
        if queue_ids != self.together_host.queue {
            let mut sync = PlaybackAction::new(action::SYNC_QUEUE);
            sync.queue = Some(upcoming);
            self.services.together_playback(sync);
            self.together_host.queue = queue_ids;
        }
        if playing != self.together_host.playing {
            self.publish_together_playback(track_id.clone(), playing, player_position);
            self.together_host.playing = playing;
            self.together_host.position = player_position;
            self.together_host.at = Some(Instant::now());
        } else if let Some(at) = self.together_host.at {
            let expected = if self.together_host.playing {
                self.together_host
                    .position
                    .saturating_add(at.elapsed().as_millis() as u64)
            } else {
                self.together_host.position
            };
            if player_position.abs_diff(expected) > 2_000 {
                self.publish_together_playback(track_id, playing, player_position);
                self.together_host.position = player_position;
                self.together_host.at = Some(Instant::now());
            }
        }
        self.together_force_sync = false;
    }

    fn publish_together_playback(&self, track_id: Option<String>, playing: bool, position: u64) {
        let mut action = PlaybackAction::new(if playing { action::PLAY } else { action::PAUSE });
        action.track_id = track_id;
        action.position = Some(position as i64);
        self.services.together_playback(action);
    }

    fn apply_together_actions(&mut self, actions: Vec<together::TogetherAction>) {
        for action in actions {
            match action {
                together::TogetherAction::Create => {
                    let name = self.together.display_name.trim().to_string();
                    if name.is_empty() {
                        self.toasts.push("Enter a display name first");
                        continue;
                    }
                    let generation = self
                        .services
                        .create_together_room(&self.together.server, &name);
                    self.together.begin(generation);
                    self.together_host = TogetherHost::default();
                    self.together_remote = TogetherRemote::default();
                    self.together_force_sync = true;
                }
                together::TogetherAction::Join => {
                    let name = self.together.display_name.trim().to_string();
                    let code = self.together.join_code.trim().to_string();
                    if name.is_empty() || code.is_empty() {
                        self.toasts.push("Enter your name and a room code");
                        continue;
                    }
                    let generation =
                        self.services
                            .join_together_room(&self.together.server, &code, &name);
                    self.together.begin(generation);
                    self.together_host = TogetherHost::default();
                    self.together_remote = TogetherRemote::default();
                    self.together_force_sync = false;
                }
                together::TogetherAction::Retry => {
                    if self.together.join_code.trim().is_empty() {
                        let name = self.together.display_name.trim().to_string();
                        if name.is_empty() {
                            self.toasts.push("Enter a display name first");
                            continue;
                        }
                        let generation = self
                            .services
                            .create_together_room(&self.together.server, &name);
                        self.together.begin(generation);
                        self.together_force_sync = true;
                    } else {
                        let name = self.together.display_name.trim().to_string();
                        let code = self.together.join_code.trim().to_string();
                        let generation =
                            self.services
                                .join_together_room(&self.together.server, &code, &name);
                        self.together.begin(generation);
                    }
                    self.together_host = TogetherHost::default();
                    self.together_remote = TogetherRemote::default();
                }
                together::TogetherAction::Leave => {
                    let generation = self.services.leave_together_room();
                    self.together.leave();
                    self.together.generation = generation;
                    self.together_host = TogetherHost::default();
                    self.together_remote = TogetherRemote::default();
                    self.together_force_sync = false;
                    self.toasts.push("Left the room");
                }
                together::TogetherAction::Copied => {
                    self.toasts.push("Room code copied");
                }
                together::TogetherAction::ApproveJoin(user_id) => {
                    self.services.together_approve_join(&user_id);
                }
                together::TogetherAction::RejectJoin(user_id) => {
                    self.services.together_reject_join(&user_id);
                }
                together::TogetherAction::HostPlayPause => {
                    if self.together.is_host() {
                        self.playback_error = None;
                        self.services.toggle_play();
                    }
                }
                together::TogetherAction::HostNext => {
                    if self.together.is_host() {
                        self.services.next();
                    }
                }
                together::TogetherAction::HostPrevious => {
                    if self.together.is_host() {
                        self.services.previous();
                    }
                }
                together::TogetherAction::HostSeek(ms) => {
                    if self.together.is_host() {
                        self.services.seek(ms);
                    }
                }
                together::TogetherAction::HostSync => {
                    if self.together.is_host() {
                        self.together_force_sync = true;
                    }
                }
                together::TogetherAction::PlayRoomTrack(id) => {
                    if self.together.is_host() {
                        let (songs, _) = self.services.queue_display();
                        if let Some(index) = songs.iter().position(|song| song.id == id) {
                            self.services.jump_queue(index);
                        }
                    }
                }
            }
        }
    }

    fn together_guest(&self) -> bool {
        self.together.is_guest()
    }

    fn block_together_guest_control(&mut self) -> bool {
        if self.together_guest() {
            self.toasts
                .push("Host controls playback. Leave the room to control this device.");
            true
        } else {
            false
        }
    }

    fn track_play(&mut self) {
        let current = self.snapshot.current.clone();
        let current_id = current.as_ref().map(|song| song.id.clone());
        let changed = current_id != self.last_played_id;
        if changed {
            // The previous track just left the player: record the last position
            // it reached, not a start-of-track zero.
            self.flush_history();
            self.last_played_id = current_id;
            self.playback_error = None;
            match current {
                Some(song) => {
                    self.liked = song.liked.is_some();
                    self.in_library = song.in_library.is_some();
                    self.history_pending = Some(crate::state::HistoryPending::new(
                        &song,
                        self.snapshot.position_ms,
                    ));
                }
                None => {
                    self.liked = false;
                    self.in_library = false;
                }
            }
        } else if let Some(pending) = self.history_pending.as_mut() {
            pending.observe(self.snapshot.position_ms, self.snapshot.is_playing());
        }
        // End of queue and Stop leave the current song in the snapshot; close
        // the pending play so the final track is not lost.
        if !changed
            && matches!(
                self.snapshot.status,
                PlaybackStatus::Ended | PlaybackStatus::Stopped
            )
        {
            self.flush_history();
        }
    }

    fn flush_history(&mut self) {
        let Some(pending) = self.history_pending.take() else {
            return;
        };
        let (song, played_ms) = pending.finish();
        if played_ms > 0 {
            self.services.record_play(song, played_ms);
        }
    }

    fn maybe_suggest(&mut self) {
        let Some(edited) = self.search.last_edit else {
            return;
        };
        if edited.elapsed() < Duration::from_millis(280) {
            return;
        }
        self.search.last_edit = None;
        let Some(query) = pure::valid_search_query(&self.search.query) else {
            self.search.suggestions.clear();
            return;
        };
        if query == self.suggest_for {
            return;
        }
        self.suggest_for = query.to_owned();
        self.services.suggest(query.to_owned());
    }

    fn ensure_library(&mut self) {
        if !self.library.data.loading {
            // Keep the current rows on screen while the refresh lands.
            self.library.data.loading = true;
            self.services.load_library(self.settings.library_sort);
        }
    }

    fn reload_liked(&mut self) {
        self.liked_songs = Load {
            loading: true,
            ..Load::default()
        };
        self.services.load_liked();
    }

    fn reload_local_playlist(&mut self, id: String) {
        self.local_playlist.id = Some(id.clone());
        self.local_playlist.data = Load {
            loading: true,
            ..Load::default()
        };
        self.services.load_local_playlist(id);
    }

    fn reload_downloads(&mut self) {
        self.downloads.items = self.services.download_list();
        self.downloads.used = self.services.download_usage();
        self.downloads.limit = self.services.download_active_limit();
        self.downloads.error = self.services.downloads_error().map(str::to_owned);
    }

    fn apply_download_event(&mut self, event: DownloadEvent) {
        match &event {
            DownloadEvent::Completed { id, .. } => {
                let title = self
                    .services
                    .download_list()
                    .into_iter()
                    .find(|entry| &entry.id == id)
                    .map(|entry| entry.song.title)
                    .unwrap_or_else(|| id.clone());
                self.toasts.push(format!("Downloaded “{title}”"));
            }
            DownloadEvent::Failed { error, .. } => self.toasts.push(error.to_string()),
            DownloadEvent::StorageChanged { .. }
            | DownloadEvent::Queued(_)
            | DownloadEvent::Progress { .. }
            | DownloadEvent::Cancelled(_)
            | DownloadEvent::Removed(_) => {}
        }
        self.reload_downloads();
    }

    // -------------------------------------------------------------- actions

    fn play_songs(&mut self, songs: Vec<Song>, index: usize, source: QueueSource) {
        self.playback_error = None;
        self.radio_for = None;
        self.services.clear_radio();
        self.services.play(songs, index, source);
    }

    fn submit_search(&mut self) {
        if self.search.query.trim().is_empty() && !self.search.submitted.is_empty() {
            self.search.query = self.search.submitted.clone();
        }
        let Some(query) = pure::valid_search_query(&self.search.query).map(str::to_owned) else {
            return;
        };
        // A pasted YouTube link routes to its page (or player) instead of being
        // searched as literal text, matching upstream `handleSearch`.
        if let Some(parsed) = parse_url(&query) {
            self.search.focus = false;
            self.search.suggestions_open = false;
            self.search.suggestions.clear();
            match url_route(&parsed) {
                Some(route) => self.navigate(route),
                None => self.play_shared(parsed.id()),
            }
            return;
        }
        self.search.submitted = query.clone();
        self.search.focus = false;
        self.search.suggestions_open = false;
        self.search.results = Load {
            loading: true,
            ..Load::default()
        };
        self.services
            .search(query.clone(), self.search.filter.filter());
        self.services.record_search(query.clone());
        self.search.history.retain(|entry| entry != &query);
        self.search.history.insert(0, query);
        self.search.history.truncate(10);
        self.search.suggestions.clear();
        self.route = Route::Search;
        self.tab = Tab::Search;
        self.back.clear();
    }

    fn start_search(&mut self, query: String) {
        self.search.query = query;
        self.search.filter = FilterChoice::All;
        self.submit_search();
    }

    fn navigate(&mut self, target: Route) {
        if target == self.route {
            return;
        }
        self.back.push(self.route.clone());
        self.set_route(target);
    }

    fn set_route(&mut self, target: Route) {
        if self.route == Route::Recognize && target != Route::Recognize && self.recognition.busy() {
            self.recognition_generation = self.services.cancel_recognition();
            self.recognition.cancel();
        }
        self.route = target.clone();
        self.player_open = false;
        self.queue_open = false;
        self.open_play_local = false;
        if let Some(tab) = target.tab() {
            self.tab = tab;
        }
        match &target {
            Route::Album { id } => {
                let id = id.clone();
                self.page = PageState {
                    route: Some(target),
                    data: Load {
                        loading: true,
                        ..Load::default()
                    },
                };
                self.services.album(id);
            }
            Route::Artist { id } => {
                let id = id.clone();
                self.page = PageState {
                    route: Some(target),
                    data: Load {
                        loading: true,
                        ..Load::default()
                    },
                };
                self.services.artist(id);
            }
            Route::Playlist { id } => {
                let id = id.clone();
                self.page = PageState {
                    route: Some(target),
                    data: Load {
                        loading: true,
                        ..Load::default()
                    },
                };
                self.services.playlist(id);
            }
            Route::Settings => {
                if self.extras.devices.data.is_none() && !self.extras.devices.loading {
                    self.extras.devices.begin();
                    self.services.list_devices();
                }
            }
            Route::Library | Route::History => self.ensure_library(),
            Route::Liked => {
                if !self.liked_songs.loading {
                    self.reload_liked();
                }
            }
            Route::LocalPlaylist { id } => {
                let id = id.clone();
                if self.local_playlist.id.as_deref() != Some(id.as_str())
                    || self.local_playlist.data.data.is_none()
                {
                    self.reload_local_playlist(id);
                }
            }
            Route::Charts => {
                self.browse = Load {
                    loading: true,
                    ..Load::default()
                };
                self.browse_target = Some(BrowseTarget::Charts);
                self.services.load_charts();
            }
            Route::NewReleases => {
                self.browse = Load {
                    loading: true,
                    ..Load::default()
                };
                self.browse_target = Some(BrowseTarget::NewReleases);
                self.services.load_new_releases();
            }
            Route::MoodResults { params, .. } => {
                self.browse = Load {
                    loading: true,
                    ..Load::default()
                };
                self.browse_target = Some(BrowseTarget::MoodResults {
                    params: params.clone(),
                });
                self.services.load_mood_results(params.clone());
            }
            Route::Moods => {
                self.moods = Load {
                    loading: true,
                    ..Load::default()
                };
                self.services.load_moods();
            }
            Route::Podcasts => {
                self.podcasts = Load {
                    loading: true,
                    ..Load::default()
                };
                self.services.load_podcasts();
            }
            Route::Podcast { id } => {
                self.podcast = Load {
                    loading: true,
                    ..Load::default()
                };
                self.podcast_id = Some(id.clone());
                self.services.load_podcast(id.clone());
            }
            Route::Stats | Route::Wrapped => {
                self.stats.copied = false;
                if matches!(self.demo, Some(DemoMode::Stats | DemoMode::Wrapped)) {
                    self.stats.period = ml_store::StatsPeriod::Month;
                    self.stats.load.finish(stats::demo_report());
                } else {
                    self.stats.load.begin();
                    self.services.load_stats(self.stats.period);
                }
            }
            Route::Lyrics => {
                if matches!(self.demo, Some(DemoMode::Lyrics)) {
                    self.install_lyrics_demo();
                } else {
                    let song = self.snapshot.current.clone();
                    if let Some(song) = song {
                        if self.lyrics_for.as_deref() != Some(song.id.as_str()) {
                            self.request_lyrics(song);
                        } else {
                            self.rebuild_lyrics_display();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn switch_tab(&mut self, tab: Tab) {
        self.back.clear();
        self.open_play = false;
        self.set_route(Route::from(tab));
    }

    fn go_back(&mut self) {
        self.open_play = false;
        if let Some(previous) = pure::back_route(&mut self.back) {
            self.set_route(previous);
        } else if self.route != Route::Home {
            self.set_route(Route::Home);
        }
    }

    fn request_lyrics(&mut self, song: Song) {
        self.lyrics_for = Some(song.id.clone());
        self.lyrics = Load {
            loading: true,
            ..Load::default()
        };
        self.lyrics_display = None;
        self.lyrics_translation = Load::default();
        self.lyrics_translation_generation = self.services.cancel_lyrics_translation();
        self.lyrics_offset_ms = self
            .services
            .ui_kv_get(&lyrics_offset_key(&song.id))
            .and_then(|value| value.parse::<i64>().ok())
            .map(|value| pure::lyrics_offset_after(value, 0))
            .unwrap_or(0);
        self.services.lyrics(song);
    }

    /// Synthetic lyrics for `--demo lyrics`; never persisted and no network.
    fn install_lyrics_demo(&mut self) {
        let Some(song) = self.snapshot.current.clone() else {
            return;
        };
        self.lyrics_for = Some(song.id);
        self.lyrics = Load {
            data: Some(lyrics::demo_document()),
            ..Load::default()
        };
        self.lyrics_translation = Load::default();
        self.settings.lyrics.romanize = true;
        self.extras.translation_provider = crate::state::TranslationProvider::DeepL;
        self.extras.translation_enabled = true;
        self.lyrics_offset_ms = -500;
        self.rebuild_lyrics_display();
    }

    fn set_lyrics_offset(&mut self, offset_ms: i64) {
        let offset_ms = pure::lyrics_offset_after(offset_ms, 0);
        if offset_ms == self.lyrics_offset_ms {
            return;
        }
        self.lyrics_offset_ms = offset_ms;
        if let Some(video_id) = self.lyrics_for.as_deref() {
            self.services
                .ui_kv_set(&lyrics_offset_key(video_id), offset_ms.to_string());
        }
        self.rebuild_lyrics_display();
    }

    fn rebuild_lyrics_display(&mut self) {
        let Some(document) = self.lyrics.data.as_ref() else {
            self.lyrics_display = None;
            return;
        };
        let mut display = if self.settings.lyrics.romanize {
            ml_lyrics::romanized(document).unwrap_or_else(|| document.clone())
        } else {
            document.clone()
        };
        ml_lyrics::apply_offset(&mut display, self.lyrics_offset_ms);
        self.lyrics_display = Some(display);
    }

    fn start_lyrics_translation(&mut self) {
        let provider = self.extras.translation_provider;
        if !provider.enabled() {
            return;
        }
        let Some(song) = self.snapshot.current.clone() else {
            return;
        };
        let Some(document) = self.lyrics.data.as_ref() else {
            return;
        };
        let endpoint = match provider {
            crate::state::TranslationProvider::DeepL => ml_lyrics::TranslationEndpoint::DeepL,
            crate::state::TranslationProvider::OpenAiCompatible => {
                ml_lyrics::TranslationEndpoint::OpenAiCompatible {
                    url: self.extras.translation_endpoint.trim().to_string(),
                }
            }
            crate::state::TranslationProvider::Off => return,
        };
        let target = self.extras.translation_target.trim();
        let config = ml_lyrics::TranslationConfig::new(
            endpoint,
            self.translation_key.trim(),
            if target.is_empty() { "en" } else { target },
        );
        self.lyrics_translation = Load {
            loading: true,
            ..Load::default()
        };
        self.lyrics_translation_generation =
            self.services
                .translate_current_lyrics(song.id, document.lines.clone(), config);
    }

    fn refresh_current(&mut self) {
        match self.route.clone() {
            Route::Home => {
                self.home.begin();
                if let Some(data) = self.home.data.as_ref() {
                    let _ = data.sections.len();
                }
                self.services.load_home();
            }
            Route::Search => self.submit_search(),
            Route::Album { id } => {
                self.page.data = Load {
                    loading: true,
                    ..Load::default()
                };
                self.services.album(id);
            }
            Route::Artist { id } => {
                self.page.data = Load {
                    loading: true,
                    ..Load::default()
                };
                self.services.artist(id);
            }
            Route::Playlist { id } => {
                self.page.data = Load {
                    loading: true,
                    ..Load::default()
                };
                self.services.playlist(id);
            }
            Route::Library | Route::History => {
                self.library.data = Load {
                    loading: true,
                    ..Load::default()
                };
                self.services.load_library(self.settings.library_sort);
            }
            Route::Liked => self.reload_liked(),
            Route::LocalPlaylist { id } => self.reload_local_playlist(id),
            Route::Charts => {
                self.browse = Load {
                    loading: true,
                    ..Load::default()
                };
                self.browse_target = Some(BrowseTarget::Charts);
                self.services.load_charts();
            }
            Route::NewReleases => {
                self.browse = Load {
                    loading: true,
                    ..Load::default()
                };
                self.browse_target = Some(BrowseTarget::NewReleases);
                self.services.load_new_releases();
            }
            Route::MoodResults { params, .. } => {
                self.browse = Load {
                    loading: true,
                    ..Load::default()
                };
                self.browse_target = Some(BrowseTarget::MoodResults {
                    params: params.clone(),
                });
                self.services.load_mood_results(params.clone());
            }
            Route::Moods => {
                self.moods = Load {
                    loading: true,
                    ..Load::default()
                };
                self.services.load_moods();
            }
            Route::Podcasts => {
                self.podcasts = Load {
                    loading: true,
                    ..Load::default()
                };
                self.services.load_podcasts();
            }
            Route::Podcast { id } => {
                self.podcast = Load {
                    loading: true,
                    ..Load::default()
                };
                self.podcast_id = Some(id.clone());
                self.services.load_podcast(id.clone());
            }
            Route::Stats | Route::Wrapped => {
                self.stats.copied = false;
                if matches!(self.demo, Some(DemoMode::Stats | DemoMode::Wrapped)) {
                    self.stats.period = ml_store::StatsPeriod::Month;
                    self.stats.load.finish(stats::demo_report());
                } else {
                    self.stats.load.begin();
                    self.services.load_stats(self.stats.period);
                }
            }
            Route::Lyrics => {
                if let Some(song) = self.snapshot.current.clone() {
                    self.request_lyrics(song);
                }
            }
            Route::Account => {
                self.account.loading = true;
                self.account.error = None;
                self.services.refresh_account();
                self.account.loading = true;
            }
            _ => {}
        }
    }

    fn load_more_current(&mut self) {
        match self.route.clone() {
            Route::Home => {
                if let Some(token) = self.home.more.clone() {
                    self.home.begin_more();
                    self.services.load_home_more(token);
                }
            }
            Route::Search => {
                if let Some(token) = self.search.results.more.clone() {
                    self.search.results.begin_more();
                    self.services.search_more(token);
                }
            }
            Route::Album { .. } => {
                if let Some(token) = self.page.data.more.clone() {
                    self.page.data.begin_more();
                    self.services.album_more(token);
                }
            }
            Route::Artist { .. } => {
                if let Some(token) = self.page.data.more.clone() {
                    self.page.data.begin_more();
                    self.services.artist_more(token);
                }
            }
            Route::Playlist { .. } => {
                if let Some(token) = self.page.data.more.clone() {
                    self.page.data.begin_more();
                    self.services.playlist_more(token);
                }
            }
            Route::Charts | Route::NewReleases | Route::MoodResults { .. } => {
                if let (Some(target), Some(token)) = (
                    BrowseTarget::from_route(&self.route),
                    self.browse.more.clone(),
                ) {
                    self.browse.begin_more();
                    self.browse_target = Some(target.clone());
                    self.services.load_browse_continuation(target, token);
                }
            }
            Route::Podcast { id } => {
                if let Some(token) = self.podcast.more.clone() {
                    self.podcast.begin_more();
                    self.services.load_podcast_more(id, token);
                }
            }
            _ => {}
        }
    }

    fn apply_intents(&mut self, intents: Vec<Intent>) {
        for intent in intents {
            if self.together_guest()
                && matches!(
                    &intent,
                    Intent::Play { .. }
                        | Intent::PlayCatalog { .. }
                        | Intent::PlayShuffled { .. }
                        | Intent::OpenPlay(_)
                        | Intent::StartRadio(_)
                        | Intent::PlayLocalPlaylist(_)
                        | Intent::RecognitionPlay { .. }
                )
            {
                self.block_together_guest_control();
                continue;
            }
            match intent {
                Intent::Play {
                    songs,
                    index,
                    source,
                } => {
                    if songs.len() == 1 && source == QueueSource::List {
                        self.playback_error = None;
                        self.radio_for = Some(songs[0].id.clone());
                        self.services.play_song(songs[0].clone());
                    } else {
                        self.play_songs(songs, index, source);
                    }
                }
                Intent::PlayCatalog { songs, index } => {
                    self.play_songs(songs, index, QueueSource::List);
                }
                Intent::StatsPeriod(period) => {
                    if period != self.stats.period {
                        self.stats.period = period;
                        self.stats.load = Load {
                            loading: true,
                            ..Load::default()
                        };
                        self.stats.copied = false;
                        self.services.load_stats(period);
                    }
                }
                Intent::StartRecognition => {
                    if !self.recognition.busy() {
                        self.recognition.start(Instant::now());
                        self.recognition_generation = self.services.start_recognition();
                    }
                }
                Intent::CancelRecognition => {
                    self.recognition_generation = self.services.cancel_recognition();
                    self.recognition.cancel();
                }
                Intent::RecognitionPlay { query } => {
                    self.services
                        .find_recognition_song(self.recognition_generation, query);
                }
                Intent::ToggleLyricsRomanize => {
                    self.settings.lyrics.romanize = !self.settings.lyrics.romanize;
                    self.settings_dirty_at = Some(Instant::now());
                    self.rebuild_lyrics_display();
                }
                Intent::AdjustLyricsOffset(delta) => {
                    let next = pure::lyrics_offset_after(self.lyrics_offset_ms, delta);
                    self.set_lyrics_offset(next);
                }
                Intent::ResetLyricsOffset => self.set_lyrics_offset(0),
                Intent::TranslateLyrics => self.start_lyrics_translation(),
                Intent::PlayShuffled { songs, source } => {
                    self.playback_error = None;
                    self.radio_for = None;
                    self.services.clear_radio();
                    self.services.play_shuffled(songs, source);
                }
                Intent::StartRadio(song) => {
                    self.playback_error = None;
                    self.radio_for = Some(song.id.clone());
                    self.services.play_song(song);
                }
                Intent::Open(route) => {
                    self.open_play = false;
                    self.navigate(route);
                }
                Intent::OpenPlay(route) => {
                    self.open_play = true;
                    self.navigate(route);
                }
                Intent::Search(query) => self.start_search(query),
                Intent::SubmitSearch => self.submit_search(),
                Intent::Refresh => self.refresh_current(),
                Intent::LoadMore => self.load_more_current(),
                Intent::StartLyrics(song) => self.request_lyrics(song),
                Intent::CreatePlaylist(name) => self.services.create_local_playlist(name),
                Intent::AddToPlaylist { playlist_id, song } => {
                    self.services.add_song_to_playlist(playlist_id, song);
                    self.toasts.push("Added to playlist");
                }
                Intent::PlayLocalPlaylist(id) => {
                    self.radio_for = None;
                    if self.local_playlist.id.as_deref() == Some(id.as_str()) {
                        let songs = self
                            .local_playlist
                            .data
                            .data
                            .as_ref()
                            .map(|view| view.songs.clone());
                        match songs {
                            Some(songs) if songs.is_empty() => {
                                self.toasts.push("Playlist is empty");
                                continue;
                            }
                            Some(songs) => {
                                self.play_songs(songs, 0, QueueSource::List);
                                continue;
                            }
                            None => {}
                        }
                    }
                    self.open_play_local = true;
                    self.reload_local_playlist(id);
                }
                Intent::AddCurrentToPlaylist { playlist_id } => {
                    match self.snapshot.current.clone() {
                        Some(song) => {
                            self.services.add_song_to_playlist(playlist_id, song);
                            self.toasts.push("Added current track");
                        }
                        None => self.toasts.push("Nothing is playing"),
                    }
                }
                Intent::RenameLocalPlaylist { id, name } => {
                    self.services.rename_local_playlist(id, name);
                }
                Intent::DeleteLocalPlaylist { id } => {
                    self.services.delete_local_playlist(id);
                }
                Intent::RemoveFromPlaylist { id, song_id } => {
                    self.services.remove_song_from_playlist(id, song_id);
                }
                Intent::MovePlaylistSong { id, from, to } => {
                    self.services.move_playlist_song(id, from, to);
                }
                Intent::Download { song, action } => match action {
                    pure::DownloadClick::Download => {
                        let ids = self.services.enqueue_downloads(&[song]);
                        if ids.is_empty() {
                            self.toasts.push("Already downloaded or queued".to_string());
                        }
                    }
                    pure::DownloadClick::Cancel => {
                        self.services.cancel_download(&song.id);
                    }
                    pure::DownloadClick::Retry => {
                        self.services.retry_download(&song.id);
                    }
                    pure::DownloadClick::None => {}
                },
                Intent::DownloadAll(songs) => {
                    let count = songs.len();
                    let ids = self.services.enqueue_downloads(&songs);
                    if ids.is_empty() {
                        self.toasts.push("Nothing new to download".to_string());
                    } else {
                        self.toasts
                            .push(format!("Queued {} of {} for download", ids.len(), count));
                    }
                }
                Intent::CancelDownload(id) => {
                    self.services.cancel_download(&id);
                }
                Intent::RetryDownload(id) => {
                    self.services.retry_download(&id);
                }
                Intent::RemoveDownload(id) => {
                    if self.services.remove_download(&id) {
                        self.toasts.push("Download removed".to_string());
                    }
                }
                Intent::ClearFailedDownloads => {
                    let removed = self.services.clear_failed_downloads();
                    self.toasts
                        .push(format!("Cleared {removed} failed downloads"));
                }
            }
        }
    }

    fn apply_mini(&mut self, action: MiniAction) {
        if matches!(
            action,
            MiniAction::TogglePlay | MiniAction::Next | MiniAction::Previous
        ) && self.block_together_guest_control()
        {
            return;
        }
        match action {
            MiniAction::TogglePlay => {
                self.playback_error = None;
                self.services.toggle_play();
            }
            MiniAction::Next => self.services.next(),
            MiniAction::Previous => self.services.previous(),
            MiniAction::OpenPlayer => {
                self.player_open = true;
                self.queue_open = false;
            }
        }
    }

    fn apply_player(&mut self, action: player::PlayerAction) {
        if matches!(
            action,
            player::PlayerAction::TogglePlay
                | player::PlayerAction::Next
                | player::PlayerAction::Previous
                | player::PlayerAction::Seek(_)
                | player::PlayerAction::ToggleShuffle
                | player::PlayerAction::CycleRepeat
        ) && self.block_together_guest_control()
        {
            return;
        }
        match action {
            player::PlayerAction::None => {}
            player::PlayerAction::TogglePlay => {
                self.playback_error = None;
                self.services.toggle_play();
            }
            player::PlayerAction::Next => self.services.next(),
            player::PlayerAction::Previous => self.services.previous(),
            player::PlayerAction::Seek(ms) => self.services.seek(ms),
            player::PlayerAction::ToggleShuffle => {
                self.services.toggle_shuffle();
                let enabled = !self.snapshot.shuffle;
                self.toasts
                    .push(if enabled { "Shuffle on" } else { "Shuffle off" });
            }
            player::PlayerAction::CycleRepeat => {
                self.services.cycle_repeat();
                self.toasts.push(format!("Repeat: {}", {
                    use ml_core::RepeatMode;
                    match self.snapshot.repeat {
                        RepeatMode::Off => "all",
                        RepeatMode::All => "one",
                        RepeatMode::One => "off",
                    }
                }));
            }
            player::PlayerAction::OpenQueue => self.queue_open = true,
            player::PlayerAction::OpenLyrics => self.navigate(Route::Lyrics),
            player::PlayerAction::ToggleMute => self.toggle_mute(),
            player::PlayerAction::ToggleLibrary => {
                if let Some(song) = self.snapshot.current.clone() {
                    self.in_library = !self.in_library;
                    let in_library = self.in_library;
                    self.services.set_in_library(song, in_library);
                    self.toasts.push(if in_library {
                        "Added to library"
                    } else {
                        "Removed from library"
                    });
                }
            }
            player::PlayerAction::ToggleLike => {
                if let Some(song) = self.snapshot.current.clone() {
                    self.liked = !self.liked;
                    let liked = self.liked;
                    self.services.set_liked(song, liked);
                    self.toasts.push(if liked {
                        "Added to liked songs"
                    } else {
                        "Removed from liked songs"
                    });
                }
            }
            player::PlayerAction::SetVolume(volume) => {
                self.services.set_volume(pure::volume_target(volume));
            }
            player::PlayerAction::Close => self.player_open = false,
        }
    }

    fn apply_nav(&mut self, action: NavAction) {
        match action {
            NavAction::Tab(tab) => self.switch_tab(tab),
            NavAction::Settings => self.navigate(action.route()),
        }
    }

    fn apply_top(&mut self, action: TopAction) {
        match action {
            TopAction::Back => self.go_back(),
            TopAction::History => self.navigate(Route::History),
            TopAction::Sync => self.refresh_current(),
            TopAction::Avatar => self.navigate(Route::Account),
        }
    }

    fn handle_keys(&mut self, ctx: &Context) {
        if ctx.memory(|memory| memory.focused().is_some()) {
            return;
        }
        let (space, escape, slash, digit, left, right, up, down, mute, shuffle, repeat) = ctx
            .input(|input| {
                let digit = if input.modifiers.command {
                    (1usize..=4).find(|number| {
                        input.key_pressed(match number {
                            1 => Key::Num1,
                            2 => Key::Num2,
                            3 => Key::Num3,
                            _ => Key::Num4,
                        })
                    })
                } else {
                    None
                };
                (
                    input.key_pressed(Key::Space),
                    input.key_pressed(Key::Escape),
                    input.key_pressed(Key::Slash),
                    digit,
                    input.key_pressed(Key::ArrowLeft),
                    input.key_pressed(Key::ArrowRight),
                    input.key_pressed(Key::ArrowUp),
                    input.key_pressed(Key::ArrowDown),
                    input.key_pressed(Key::M),
                    input.key_pressed(Key::S),
                    input.key_pressed(Key::R),
                )
            });

        if let Some(number) = digit {
            if let Some(tab) = Tab::from_shortcut(number - 1) {
                self.switch_tab(tab);
            }
        }
        if escape {
            if self.queue_open {
                self.queue_open = false;
            } else if self.player_open {
                self.player_open = false;
            } else {
                self.go_back();
            }
        }
        if slash {
            self.switch_tab(Tab::Search);
            self.search.focus = true;
        }
        let guest = self.together_guest();
        if !guest && space && self.snapshot.current.is_some() {
            self.services.toggle_play();
        }
        if !guest && (left || right) && self.snapshot.current.is_some() {
            let offset = if left { -5_000 } else { 5_000 };
            let target = crate::pure::seek_relative(
                self.snapshot.position_ms,
                self.snapshot.duration_ms,
                offset,
            );
            self.services.seek(target);
        }
        if up || down {
            let delta = if up { 0.05 } else { -0.05 };
            let target = crate::pure::volume_step(self.snapshot.volume, delta);
            self.services.set_volume(target);
            if self.muted {
                self.muted = false;
                self.services.set_muted(false);
            }
        }
        if mute {
            self.toggle_mute();
        }
        if !guest && shuffle {
            self.apply_player(player::PlayerAction::ToggleShuffle);
        }
        if !guest && repeat {
            self.apply_player(player::PlayerAction::CycleRepeat);
        }
    }

    fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.services.set_muted(self.muted);
        self.toasts
            .push(if self.muted { "Muted" } else { "Unmuted" });
    }

    fn apply_desktop_events(&mut self, ctx: &Context) {
        let events = self.desktop.poll(&self.snapshot);
        for event in events {
            match event {
                DesktopEvent::ToggleWindow => {
                    if self.window_visible {
                        self.set_window_visible(ctx, false);
                    } else {
                        self.set_window_visible(ctx, true);
                    }
                }
                DesktopEvent::Show => self.set_window_visible(ctx, true),
                DesktopEvent::Hide => self.set_window_visible(ctx, false),
                DesktopEvent::PlayPause => {
                    if self.snapshot.current.is_some() && !self.block_together_guest_control() {
                        self.services.toggle_play();
                    }
                }
                DesktopEvent::Next => {
                    if !self.block_together_guest_control() {
                        self.services.next();
                    }
                }
                DesktopEvent::Previous => {
                    if !self.block_together_guest_control() {
                        self.services.previous();
                    }
                }
                DesktopEvent::Quit => {
                    self.services.stop();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                DesktopEvent::OpenUri(uri) => {
                    if let Some(video_id) = crate::pure::parse_deep_link(&uri) {
                        self.play_shared(&video_id);
                    } else {
                        self.toasts.push(format!("Unsupported link: {uri}"));
                    }
                    self.set_window_visible(ctx, true);
                }
                DesktopEvent::Forwarded(args) => {
                    println!("instance: forwarded argv: {args:?}");
                    if let Some(video_id) = desktop::deep_link_video(&args) {
                        self.play_shared(&video_id);
                    }
                    self.set_window_visible(ctx, true);
                }
                DesktopEvent::Error(message) => self.toasts.push(message),
            }
        }
    }

    fn set_window_visible(&mut self, ctx: &Context, visible: bool) {
        self.window_visible = visible;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
        if visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn play_shared(&mut self, video_id: &str) {
        if video_id.is_empty() {
            return;
        }
        if self.block_together_guest_control() {
            return;
        }
        let song = Song {
            id: video_id.to_string(),
            title: "Shared track".to_string(),
            ..Song::default()
        };
        self.radio_for = Some(song.id.clone());
        self.services.play_song(song);
        self.toasts.push(format!("Opening shared track {video_id}"));
    }

    fn current_texture(&mut self, ctx: &Context, size: u32) -> Option<egui::TextureHandle> {
        let url = self
            .snapshot
            .current
            .as_ref()
            .map(|song| song.thumb(size))
            .unwrap_or_default();
        self.art.texture(ctx, &self.services, &url)
    }

    // --------------------------------------------------------------- render

    fn route_content(&mut self, ui: &mut Ui, width: f32) {
        let route = self.route.clone();
        let current = self.snapshot.current.clone();
        let intents = match route {
            Route::Home => home::show(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.home,
                current.as_ref(),
                width,
            ),
            Route::Search => search::show(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &mut self.search,
                current.as_ref(),
                width,
            ),
            Route::Album { .. } | Route::Artist { .. } | Route::Playlist { .. } => detail::show(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.page.data,
                current.as_ref(),
                width,
            ),
            Route::Lyrics => lyrics::show(
                ui,
                &self.theme,
                current.as_ref(),
                &self.lyrics,
                lyrics::LyricsOptions {
                    display: self.lyrics_display.as_ref(),
                    translation: &self.lyrics_translation,
                    active_ms: self.snapshot.position_ms,
                    offset_ms: self.lyrics_offset_ms,
                    romanize: self.settings.lyrics.romanize,
                    translation_available: self.extras.translation_provider.enabled(),
                    show_translation: self.extras.translation_enabled
                        && self.extras.translation_provider.enabled(),
                },
            ),
            Route::Together => {
                let actions =
                    together::show(ui, &self.theme, &mut self.together, &self.snapshot, width);
                self.apply_together_actions(actions);
                Vec::new()
            }
            Route::Library => library::show(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &mut self.library,
                &mut self.downloads,
                current.as_ref(),
                width,
            ),
            Route::Liked => liked::show(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.liked_songs,
                current.as_ref(),
                width,
            ),
            Route::History => history::show(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.library.data,
                current.as_ref(),
                width,
            ),
            Route::LocalPlaylist { .. } => local_playlist::show(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &mut self.local_playlist,
                current.as_ref(),
                width,
            ),
            Route::Recognize => recognize::show(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.recognition,
                width,
            ),
            Route::Stats => stats::show(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &mut self.stats,
                current.as_ref(),
                width,
            ),
            Route::Wrapped => stats::wrapped(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &mut self.stats,
                width,
            ),
            Route::Charts => catalog::charts(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.browse,
                current.as_ref(),
                width,
            ),
            Route::NewReleases => catalog::new_releases(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.browse,
                current.as_ref(),
                width,
            ),
            Route::MoodResults { .. } => catalog::mood_results(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.browse,
                current.as_ref(),
                width,
            ),
            Route::Moods => catalog::moods(ui, &self.theme, &self.moods, width),
            Route::Podcasts => catalog::podcasts(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.podcasts,
                current.as_ref(),
                width,
            ),
            Route::Podcast { .. } => catalog::podcast(
                ui,
                &self.theme,
                &self.services,
                &mut self.art,
                &self.podcast,
                current.as_ref(),
                width,
            ),
            Route::Account => {
                let actions = account::show(
                    ui,
                    &self.theme,
                    &self.services,
                    &mut self.art,
                    &mut self.account,
                    width,
                );
                self.apply_account_actions(actions);
                Vec::new()
            }
            Route::Settings => {
                let cache = self.art.ready_count();
                let cache_bytes = self.art.memory_bytes();
                let db_path = self.paths.database_file();
                let desktop = self.desktop.status();
                let download_dir = self.services.download_dir().to_path_buf();
                let download_used = self.services.download_usage();
                let download_limit = self.download_limit;
                let download_active_limit = self.services.download_active_limit();
                let download_failed = self.downloads.failed_count();
                let download_quality = self.services.download_quality();
                let downloads_available = self.services.downloads_available();
                let download_error = self.services.downloads_error().map(str::to_owned);
                let crossfade_available = self.services.crossfade_available();
                let account_signed_in = self.account.session.signed_in;
                let account_name = self
                    .account
                    .session
                    .info
                    .as_ref()
                    .map(|info| info.name.clone());
                let demo_settings = matches!(self.demo, Some(DemoMode::Settings));
                let focus_playback = demo_settings && width < 800.0;
                let focus_sleep = demo_settings && !focus_playback;
                let focus_audio = matches!(self.demo, Some(DemoMode::SettingsAudio));
                let focus_integrations = self.frames < 2
                    && matches!(
                        self.demo,
                        Some(
                            DemoMode::Integrations
                                | DemoMode::IntegrationsConnected
                                | DemoMode::IntegrationsError
                        )
                    );
                let lastfm_status = self
                    .lastfm_demo_status
                    .clone()
                    .unwrap_or_else(|| self.services.lastfm_status());
                let discord_status = self
                    .discord_demo_status
                    .clone()
                    .unwrap_or_else(|| self.services.discord_status());
                let actions = settings::show(
                    ui,
                    &self.theme,
                    &mut self.settings,
                    SettingsStatus {
                        extras: &mut self.extras,
                        crossfade_available,
                        focus_playback,
                        focus_sleep,
                        focus_audio,
                        image_cache: cache,
                        image_cache_bytes: cache_bytes,
                        db_path: &db_path,
                        desktop,
                        download_dir: &download_dir,
                        download_used,
                        download_limit,
                        download_active_limit,
                        download_failed,
                        download_quality,
                        downloads_available,
                        download_error: download_error.as_deref(),
                        account_signed_in,
                        account_name: account_name.as_deref(),
                        translation_key: &mut self.translation_key,
                        lastfm: &mut self.lastfm,
                        lastfm_status,
                        discord_status,
                        focus_integrations,
                    },
                    width,
                );
                self.apply_settings_actions(ui.ctx(), actions);
                Vec::new()
            }
        };
        self.apply_intents(intents);
    }

    fn apply_account_actions(&mut self, actions: Vec<account::AccountAction>) {
        for action in actions {
            match action {
                account::AccountAction::ImportFile => {
                    let path = self.account.imported_path.trim().to_string();
                    if path.is_empty() {
                        self.account.error =
                            Some("Enter the path to a cookies.txt file first.".to_string());
                        continue;
                    }
                    self.account.loading = true;
                    self.account.error = None;
                    self.services
                        .import_cookie_file(std::path::PathBuf::from(path));
                    self.account.loading = true;
                }
                account::AccountAction::ImportBrowser => {
                    self.account.loading = true;
                    self.account.error = None;
                    self.services.import_browser_cookies();
                    self.account.loading = true;
                }
                account::AccountAction::Refresh => {
                    self.account.loading = true;
                    self.account.error = None;
                    self.services.refresh_account();
                    self.account.loading = true;
                }
                account::AccountAction::Sync => {
                    self.account.syncing = true;
                    self.account.error = None;
                    self.services.sync_library();
                }
                account::AccountAction::SignOut => {
                    self.services.sign_out();
                }
            }
        }
    }

    fn apply_settings_actions(&mut self, ctx: &Context, actions: Vec<SettingsAction>) {
        let mut changed = false;
        for action in actions {
            match action {
                SettingsAction::Changed => changed = true,
                SettingsAction::ClearImageCache => {
                    self.art.clear();
                    self.toasts.push("Image cache cleared");
                }
                SettingsAction::SleepTimer(timer) => {
                    self.extras.sleep = timer;
                    self.services.set_sleep_timer(timer);
                    self.persist_extras();
                }
                SettingsAction::EqChanged => {
                    let bands = if self.extras.eq_enabled {
                        crate::pure::clamp_eq_bands(&self.extras.eq_preset.bands())
                    } else {
                        [0.0; 10]
                    };
                    self.services.set_eq(&bands);
                    self.persist_extras();
                }
                SettingsAction::CrossfadeChanged => {
                    self.services.set_crossfade(
                        self.extras.crossfade_seconds,
                        self.extras.crossfade_gapless,
                    );
                    self.persist_extras();
                }
                SettingsAction::SkipSilenceChanged => {
                    self.services.set_skip_silence(self.extras.skip_silence);
                    self.persist_extras();
                }
                SettingsAction::NormalizationChanged => {
                    self.services.set_normalization(
                        self.extras.normalization,
                        Some(self.extras.normalization_gain_db),
                    );
                    self.persist_extras();
                }
                SettingsAction::SpeedChanged(speed) => {
                    self.extras.speed = speed;
                    self.services.set_speed(speed);
                    self.persist_extras();
                }
                SettingsAction::DeviceChanged(device) => {
                    let device = device.unwrap_or_else(|| "auto".to_string());
                    self.services.set_device(Some(device.clone()));
                    self.extras.device = Some(device);
                    self.persist_extras();
                }
                SettingsAction::NotificationsChanged(enabled) => {
                    self.desktop.set_notifications(enabled);
                    self.persist_extras();
                }
                SettingsAction::ListDevices => {
                    self.extras.devices.begin();
                    self.services.list_devices();
                }
                SettingsAction::DownloadLimitChanged(limit) => {
                    self.download_limit = limit;
                    self.services.set_download_limit(limit);
                }
                SettingsAction::ClearFailedDownloads => {
                    let removed = self.services.clear_failed_downloads();
                    self.toasts
                        .push(format!("Cleared {removed} failed downloads"));
                }
                SettingsAction::OpenDownloads => {
                    self.library.tab = LibTab::Downloads;
                    self.switch_tab(Tab::Library);
                }
                SettingsAction::OpenAccount => self.navigate(Route::Account),
                SettingsAction::LyricsTranslationChanged => {
                    self.persist_extras();
                    self.lyrics_translation = Load::default();
                    self.lyrics_translation_generation = self.services.reset_lyrics_translation();
                }
                SettingsAction::LastFmChanged => {
                    if self.integrations_demo() {
                        continue;
                    }
                    self.extras.lastfm_enabled = self.lastfm.enabled;
                    self.persist_extras();
                    self.persist_lastfm();
                    self.configure_integrations(false);
                }
                SettingsAction::LastFmConnect => self.connect_lastfm(),
                SettingsAction::LastFmSignOut => {
                    if self.integrations_demo() {
                        self.lastfm.clear_credentials();
                        continue;
                    }
                    self.lastfm.session.clear();
                    self.lastfm.password.clear();
                    self.lastfm.enabled = false;
                    self.lastfm.connecting = false;
                    self.lastfm.error = None;
                    self.extras.lastfm_enabled = false;
                    self.persist_extras();
                    self.persist_lastfm();
                    self.services.lastfm_sign_out();
                    self.toasts.push("Signed out of Last.fm");
                }
                SettingsAction::LastFmForget => {
                    if self.integrations_demo() {
                        self.lastfm.clear_credentials();
                        continue;
                    }
                    self.lastfm.clear_credentials();
                    self.extras.lastfm_enabled = false;
                    self.persist_extras();
                    let path = integrations::secrets_path(&self.paths.data_dir);
                    if let Err(error) = integrations::delete_secrets(&path) {
                        self.lastfm.error = Some(error);
                    }
                    self.services.lastfm_forget();
                    self.toasts.push("Last.fm credentials forgotten");
                }
                SettingsAction::DiscordChanged => {
                    if self.integrations_demo() {
                        continue;
                    }
                    self.persist_extras();
                    self.services.configure_discord(
                        self.extras.discord_enabled,
                        &self.extras.discord_app_id,
                    );
                }
            }
        }
        if !changed {
            return;
        }
        let key = (self.settings.theme_mode, self.settings.seed_color);
        if key != self.theme_key {
            self.theme = Theme::from_settings(&self.settings);
            self.theme.install(ctx);
            self.theme_key = key;
        }
        self.services.set_quality(self.settings.audio_quality);
        self.services
            .set_lyrics_settings(self.settings.lyrics.clone());
        self.settings_dirty_at = Some(Instant::now());
        if self.route == Route::Lyrics {
            self.rebuild_lyrics_display();
        }
        ctx.request_repaint();
    }

    fn persist_extras(&mut self) {
        for (key, value) in self.extras.persisted() {
            self.services.ui_kv_set(key, value);
        }
    }

    fn integrations_demo(&self) -> bool {
        matches!(
            self.demo,
            Some(
                DemoMode::Integrations
                    | DemoMode::IntegrationsConnected
                    | DemoMode::IntegrationsError
            )
        )
    }

    fn connect_lastfm(&mut self) {
        if self.integrations_demo() {
            self.lastfm.error = Some("Demo mode does not contact Last.fm.".to_string());
            return;
        }
        if self.lastfm.connecting {
            return;
        }
        let mode = pure::lastfm_connect_mode(
            &self.lastfm.api_key,
            &self.lastfm.secret,
            &self.lastfm.username,
            &self.lastfm.password,
            &self.lastfm.session,
        );
        match mode {
            pure::LastFmConnectMode::Authenticate => {
                let password = std::mem::take(&mut self.lastfm.password);
                let username = self.lastfm.username.trim().to_string();
                let mut config = self.lastfm.config();
                config.session = None;
                self.lastfm.connecting = true;
                self.lastfm.error = None;
                self.persist_lastfm();
                self.services.connect_lastfm(config, username, password);
            }
            pure::LastFmConnectMode::AcceptSession => {
                self.lastfm.password.clear();
                self.lastfm.connecting = false;
                self.lastfm.error = None;
                self.persist_lastfm();
                self.configure_integrations(true);
                self.toasts.push("Last.fm session key saved".to_string());
            }
            pure::LastFmConnectMode::Missing => {
                self.lastfm.error = Some(
                    "Enter an API key, shared secret and session key, or API key, shared secret, username and password."
                        .to_string(),
                );
            }
        }
    }

    fn configure_integrations(&self, flush: bool) {
        self.services.configure_lastfm(
            self.lastfm.config(),
            self.lastfm.enabled,
            Some(self.lastfm.username.trim().to_string()).filter(|name| !name.is_empty()),
            flush,
        );
        self.services
            .configure_discord(self.extras.discord_enabled, &self.extras.discord_app_id);
    }

    fn persist_lastfm(&mut self) {
        let path = integrations::secrets_path(&self.paths.data_dir);
        let secrets = self.lastfm.secrets();
        let result = if secrets.is_empty() {
            integrations::delete_secrets(&path)
        } else {
            integrations::save_secrets(&path, &secrets)
        };
        if let Err(error) = result {
            self.lastfm.error = Some(error);
        }
    }

    fn flush_settings(&mut self, ctx: &Context) {
        let Some(at) = self.settings_dirty_at else {
            return;
        };
        if at.elapsed() >= Duration::from_millis(400) {
            self.services.save_settings(&self.settings);
            self.settings_dirty_at = None;
        } else {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    fn title(&self) -> &str {
        match &self.route {
            Route::Album { .. } | Route::Artist { .. } | Route::Playlist { .. } => self
                .page
                .data
                .data
                .as_ref()
                .map(|page| page.title())
                .unwrap_or(self.route.title()),
            Route::LocalPlaylist { id } => self
                .local_playlist
                .data
                .data
                .as_ref()
                .filter(|view| &view.id == id)
                .map(|view| view.name.as_str())
                .unwrap_or(self.route.title()),
            Route::MoodResults { title, .. } => title.as_deref().unwrap_or(self.route.title()),
            Route::Podcast { id } => self
                .podcast
                .data
                .as_ref()
                .filter(|_| self.podcast_id.as_deref() == Some(id.as_str()))
                .map(|page| page.show.title.as_str())
                .unwrap_or(self.route.title()),
            route => route.title(),
        }
    }

    fn shell(&mut self, ui: &mut Ui, screen_width: f32, expanded: bool) {
        let theme = self.theme.clone();
        let bg = theme.bg();
        let panel_frame = Frame::new().fill(bg);
        let title = self.title().to_owned();
        let back = self.route.tab().is_none();
        if expanded {
            let content_width = (screen_width - 88.0 - 32.0).min(960.0);
            Panel::left("nav_rail")
                .exact_size(88.0)
                .frame(panel_frame)
                .show(ui, |ui| {
                    if let Some(action) = shell::nav_rail(
                        ui,
                        &theme,
                        shell::active_tab(&self.route, self.tab),
                        shell::settings_active(&self.route),
                    ) {
                        self.apply_nav(action);
                    }
                });
            Panel::top("wide_top").frame(panel_frame).show(ui, |ui| {
                ui.add_space(8.0);
                center_column(ui, content_width, |ui| {
                    if let Some(action) = shell::top_bar(ui, &theme, &title, content_width, back) {
                        self.apply_top(action);
                    }
                    if self.route == Route::Home {
                        ui.add_space(2.0);
                        let changed =
                            shell::filter_chips(ui, &theme, &home::CHIPS, &mut self.home_chip);
                        if changed {
                            self.chip_search();
                        }
                    }
                });
            });
            if self.snapshot.current.is_some() {
                Panel::bottom("wide_mini")
                    .frame(panel_frame)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        center_column(ui, content_width, |ui| {
                            self.mini_player(ui, content_width);
                        });
                        ui.add_space(8.0);
                    });
            }
            CentralPanel::default()
                .frame(Frame::new().fill(bg))
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            center_column(ui, content_width, |ui| {
                                ui.add_space(4.0);
                                self.route_content(ui, content_width);
                            });
                        });
                });
        } else {
            let content_width = (screen_width - 32.0).min(560.0);
            Panel::top("compact_top").frame(panel_frame).show(ui, |ui| {
                ui.add_space(6.0);
                center_column(ui, content_width, |ui| {
                    if let Some(action) = shell::top_bar(ui, &theme, &title, content_width, back) {
                        self.apply_top(action);
                    }
                    if self.route == Route::Home {
                        ui.add_space(2.0);
                        let changed =
                            shell::filter_chips(ui, &theme, &home::CHIPS, &mut self.home_chip);
                        if changed {
                            self.chip_search();
                        }
                    }
                });
            });
            Panel::bottom("compact_bottom")
                .frame(panel_frame)
                .show(ui, |ui| {
                    if self.snapshot.current.is_some() {
                        ui.add_space(4.0);
                        center_column(ui, content_width, |ui| {
                            self.mini_player(ui, content_width);
                        });
                    }
                    center_column(ui, content_width, |ui| {
                        if let Some(action) = shell::bottom_nav(
                            ui,
                            &theme,
                            shell::active_tab(&self.route, self.tab),
                            shell::settings_active(&self.route),
                            content_width,
                        ) {
                            self.apply_nav(action);
                        }
                    });
                });
            CentralPanel::default()
                .frame(Frame::new().fill(bg))
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            center_column(ui, content_width, |ui| {
                                ui.add_space(4.0);
                                self.route_content(ui, content_width);
                            });
                        });
                });
        }
    }

    fn mini_player(&mut self, ui: &mut Ui, width: f32) {
        let Some(song) = self.snapshot.current.clone() else {
            return;
        };
        let texture = self.art.texture(ui.ctx(), &self.services, &song.thumb(96));
        let action = shell::mini_player(
            ui,
            &self.theme,
            &song,
            self.snapshot.is_playing(),
            texture.as_ref(),
            width,
        );
        if let Some(action) = action {
            self.apply_mini(action);
        }
    }

    fn chip_search(&mut self) {
        if let Some(index) = self.home_chip {
            if let Some(label) = home::CHIPS.get(index) {
                self.start_search((*label).to_owned());
            }
        }
    }

    fn player_route(&mut self, ui: &mut Ui, screen_width: f32) {
        let texture = self.current_texture(ui.ctx(), 480);
        player::backdrop(ui, &self.theme, texture.as_ref());
        let width = (screen_width - 48.0).min(560.0).max(240.0);
        let theme = self.theme.clone();
        let current = self.snapshot.current.clone();
        let status = if self.playback_error.is_some() {
            PlaybackStatus::Error
        } else {
            self.snapshot.status
        };
        let snapshot = self.snapshot.clone();
        let liked = self.liked;
        let mut action = player::PlayerAction::None;
        center_column(ui, width, |ui| {
            let view = player::PlayerView {
                song: current.as_ref(),
                playing: snapshot.is_playing(),
                status,
                position_ms: snapshot.position_ms,
                duration_ms: snapshot.duration_ms,
                shuffle: snapshot.shuffle,
                repeat: snapshot.repeat,
                texture: texture.as_ref(),
                volume: snapshot.volume,
                liked,
                in_library: self.in_library,
                muted: self.muted,
                offline: current
                    .as_ref()
                    .is_some_and(|song| self.services.is_downloaded(&song.id)),
            };
            action = player::show(ui, &theme, &view, width);
        });
        self.apply_player(action);
    }

    fn queue_overlay(&mut self, ctx: &Context) {
        let (songs, position) = self.services.queue_display();
        let action = queue_screen::show_modal(ctx, &self.theme, &self.services, &songs, position);
        if self.together_guest()
            && matches!(
                action,
                queue_screen::QueueAction::Play(_)
                    | queue_screen::QueueAction::Move { .. }
                    | queue_screen::QueueAction::Remove(_)
                    | queue_screen::QueueAction::LoadMore
            )
        {
            self.block_together_guest_control();
            return;
        }
        match action {
            queue_screen::QueueAction::None => {}
            queue_screen::QueueAction::Play(position) => {
                self.services.jump_queue(position);
            }
            queue_screen::QueueAction::Move { from, to } => {
                self.services.move_queue_item(from, to);
            }
            queue_screen::QueueAction::Remove(position) => {
                self.services.remove_queue_item(position);
            }
            queue_screen::QueueAction::LoadMore => self.services.extend_radio(true),
            queue_screen::QueueAction::Download { song, action } => {
                self.apply_intents(vec![Intent::Download { song, action }]);
            }
            queue_screen::QueueAction::Close => self.queue_open = false,
        }
    }

    /// Release playback engines, workers and the single-instance socket.
    ///
    /// `std::process::exit` skips `Drop`, so every exit path calls this.
    fn shutdown(&mut self) {
        self.services.shutdown();
        self.desktop.shutdown();
    }

    /// Clean up, flush the CLI output and terminate the process.
    fn exit(&mut self, code: i32) -> ! {
        let _ = std::io::Write::flush(&mut std::io::stdout());
        self.shutdown();
        std::process::exit(code)
    }

    fn after_frame(&mut self, ctx: &Context) {
        // The screenshot callback runs before this frame's UI and cannot
        // borrow the app, so it records the exit code here.
        let cli_exit = self.cli_exit.swap(-1, Ordering::SeqCst);
        if cli_exit >= 0 {
            self.exit(cli_exit);
        }
        self.frames = self.frames.saturating_add(1);
        let mode = self.mode.clone();
        match mode {
            RunMode::Screenshot { path } => {
                let demo_ready = match self.demo {
                    None => true,
                    Some(DemoMode::Lyrics) => {
                        self.route == Route::Lyrics && self.lyrics.data.is_some()
                    }
                    Some(DemoMode::Player) => {
                        self.snapshot
                            .current
                            .as_ref()
                            .is_some_and(|song| song.id == Self::DEMO_SONG_ID)
                            && self.snapshot.is_playing()
                            && self.snapshot.position_ms > 1500
                            && self.toasts.is_empty()
                    }
                    Some(DemoMode::Queue) => {
                        self.snapshot.current.is_some() && self.services.queue_display().0.len() > 1
                    }
                    Some(DemoMode::Library | DemoMode::History) => self.library.data.data.is_some(),
                    Some(DemoMode::Liked) => self.liked_songs.data.is_some(),
                    Some(
                        DemoMode::Settings
                        | DemoMode::SettingsAudio
                        | DemoMode::Account
                        | DemoMode::Integrations
                        | DemoMode::IntegrationsConnected
                        | DemoMode::IntegrationsError
                        | DemoMode::Together
                        | DemoMode::TogetherConnected
                        | DemoMode::TogetherGuest,
                    ) => true,
                    Some(DemoMode::Charts | DemoMode::NewReleases) => self.browse.data.is_some(),
                    Some(DemoMode::Moods) => self.moods.data.is_some(),
                    Some(DemoMode::Podcasts) => self.podcasts.data.is_some(),
                    Some(DemoMode::Stats | DemoMode::Wrapped) => self.stats.load.data.is_some(),
                    Some(
                        DemoMode::Recognize
                        | DemoMode::RecognizeListening
                        | DemoMode::RecognizeSuccess
                        | DemoMode::RecognizeError,
                    ) => true,
                    Some(DemoMode::Downloads) => self
                        .downloads
                        .items
                        .iter()
                        .any(|entry| entry.state == DownloadState::Completed),
                };
                // A demo renders its own screen, so the home feed settling is
                // irrelevant; waiting for it made screenshot runs hang when
                // the network feed was slow or unavailable.
                let idle = (self.demo.is_some() || !self.home.loading)
                    && !self.page.data.loading
                    && demo_ready;
                if idle {
                    self.screenshot_idle_frames = self.screenshot_idle_frames.saturating_add(1);
                }
                // Give thumbnails a moment to download and upload after the feed
                // lands. Demo screens render local fixtures and only need a
                // couple of frames; waiting longer let occluded windows stall
                // forever because wgpu stopped delivering frames.
                let settled = self.frames >= 6
                    && demo_ready
                    && (self.demo.is_some() || self.screenshot_idle_frames >= 45);
                if !self.screenshot_requested && (settled || self.frames >= 360) {
                    self.screenshot_requested = true;
                    self.screenshot_frame = self.frames;
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    let cli_exit = self.cli_exit.clone();
                    ctx.request_screenshot(move |image| {
                        let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
                        for pixel in &image.pixels {
                            rgba.extend_from_slice(&pixel.to_array());
                        }
                        let Some(buffer) = image::RgbaImage::from_raw(
                            image.size[0] as u32,
                            image.size[1] as u32,
                            rgba,
                        ) else {
                            eprintln!("screenshot failed: buffer size mismatch");
                            cli_exit.store(1, Ordering::SeqCst);
                            return;
                        };
                        if let Err(error) = buffer.save(&path) {
                            eprintln!("screenshot failed: {error}");
                            cli_exit.store(1, Ordering::SeqCst);
                            return;
                        }
                        println!(
                            "screenshot written: {} ({}x{})",
                            path.display(),
                            image.size[0],
                            image.size[1]
                        );
                        // eframe may stop calling `ui` while the window is occluded,
                        // leaving the screenshot CLI alive forever; the image is
                        // already on disk, so exit instead of waiting for Close.
                        cli_exit.store(0, Ordering::SeqCst);
                    });
                }
                if self.screenshot_requested && self.frames >= self.screenshot_frame + 20 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                ctx.request_repaint();
            }
            RunMode::Smoke { frames: max_frames } => {
                if self.frames >= max_frames {
                    if !self.smoke_reported {
                        self.smoke_reported = true;
                        let sections = self
                            .home
                            .data
                            .as_ref()
                            .map(|feed| feed.sections.len())
                            .unwrap_or(0);
                        println!(
                            "SMOKE_OK frames={} backend={} home_sections={}",
                            self.frames, self.backend, sections
                        );
                        self.exit(0);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    ctx.request_repaint();
                }
            }
            RunMode::Normal => {}
        }
    }
}

fn lyrics_offset_key(video_id: &str) -> String {
    format!("lyrics.offset.{video_id}")
}

impl eframe::App for App {
    fn on_exit(&mut self) {
        if self.recognition.busy() {
            self.services.cancel_recognition();
        }
        self.flush_history();
        self.shutdown();
    }

    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // SIGTERM/SIGINT: the handler only wakes this loop, so cleanup still
        // runs on the thread that owns the engines.
        if crate::signals::terminate_requested() {
            self.exit(0);
        }
        self.drain_events(ctx);
        self.poll_player();
        self.poll_together(ctx);
        self.track_play();
        self.apply_desktop_events(ctx);
        if self.demo_lyrics_pending && self.snapshot.current.is_some() {
            self.demo_lyrics_pending = false;
            self.navigate(Route::Lyrics);
        }
        self.maybe_suggest();
        self.handle_keys(ctx);
        self.flush_settings(ctx);
        // Keep integration/tray/instance events flowing even when the window is
        // hidden or playback is paused (MPRIS already pokes us directly).
        ctx.request_repaint_after(Duration::from_millis(250));
        let busy = self.home.loading
            || self.page.data.loading
            || self.search.results.loading
            || self.library.data.loading
            || self.liked_songs.loading
            || self.local_playlist.data.loading
            || self.lyrics.loading
            || self.snapshot.is_playing()
            || matches!(
                self.snapshot.status,
                PlaybackStatus::Loading | PlaybackStatus::Buffering
            );
        if busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        let expanded = shell::is_expanded(screen.width());
        let bg = self.theme.bg();
        Frame::central_panel(ui.style())
            .fill(bg)
            .inner_margin(0)
            .show(ui, |ui| {
                ui.set_min_size(screen.size());
                if self.player_open {
                    self.player_route(ui, screen.width());
                } else {
                    self.shell(ui, screen.width(), expanded);
                }
            });
        if self.queue_open {
            self.queue_overlay(&ctx);
        }
        self.toasts.show(&ctx, &self.theme);
        self.after_frame(&ctx);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // eframe can tear down without `on_exit`; shutdown is idempotent.
        self.shutdown();
    }
}

/// Center a fixed-width content column inside a full-width Ui.
///
/// The spacer lives in a horizontal row: `add_space` follows the current
/// layout, so in the top-down body it would add height instead of a margin.
fn center_column<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let available = ui.available_width();
    let height = ui.available_height();
    let width = width.min(available).max(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(((available - width) / 2.0).max(0.0));
        ui.allocate_ui_with_layout(
            Vec2::new(width, height),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(width);
                add(ui)
            },
        )
        .inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_offset_key_is_scoped_by_video() {
        assert_eq!(
            lyrics_offset_key("kJQP7kiw5Fk"),
            "lyrics.offset.kJQP7kiw5Fk"
        );
    }

    #[test]
    fn pasted_urls_route_like_upstream() {
        assert_eq!(
            url_route(&ParsedUrl::Video("kJQP7kiw5Fk".into())),
            None,
            "video links play"
        );
        assert_eq!(
            url_route(&ParsedUrl::Playlist("PL123".into())),
            Some(Route::Playlist { id: "PL123".into() })
        );
        assert_eq!(
            url_route(&ParsedUrl::Album("MPREb_album".into())),
            Some(Route::Album {
                id: "MPREb_album".into()
            })
        );
        assert_eq!(
            url_route(&ParsedUrl::Album("OLAK5uy_album".into())),
            Some(Route::Playlist {
                id: "OLAK5uy_album".into()
            })
        );
        assert_eq!(
            url_route(&ParsedUrl::Artist("UCartist".into())),
            Some(Route::Artist {
                id: "UCartist".into()
            })
        );
    }
}
