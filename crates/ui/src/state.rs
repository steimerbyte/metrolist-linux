//! Screen state shared by the app shell and the routed screens.

use std::time::Instant;

use ml_audio::{AudioDevice, SleepTimer};
use ml_core::{
    Album, AlbumPage, ArtistItem, ArtistPage, HomeFeed, HomeSection, LibrarySort, PlaylistPage,
    SearchPage, Song, YtItem,
};
use ml_downloads::Download;
use ml_innertube::auth::{AccountInfo, AuthSnapshot};
use ml_innertube::SearchFilter;
use ml_recognition::RecognitionResult;
use ml_store::{HistoryEntry, PlaylistRecord};
use ml_store::{StatsPeriod, StatsReport};
use ml_together::{
    Role, RoomState, SessionEvent, SessionSnapshot, TogetherEvent, DEFAULT_SERVER_URL,
    MAX_RECONNECT_ATTEMPTS,
};

use crate::routes::Route;

/// One asynchronously loaded value with loading/more/error bookkeeping.
#[derive(Debug)]
pub struct Load<T> {
    pub data: Option<T>,
    pub loading: bool,
    pub error: Option<String>,
    pub more: Option<String>,
    pub more_loading: bool,
}

// Manual impl: `T` need not implement `Default` (page data types do not).
impl<T> Default for Load<T> {
    fn default() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
            more: None,
            more_loading: false,
        }
    }
}

impl<T> Load<T> {
    pub fn begin(&mut self) {
        self.loading = true;
        self.more_loading = false;
        self.error = None;
    }

    pub fn begin_more(&mut self) {
        self.more_loading = true;
        self.error = None;
    }

    pub fn finish(&mut self, data: T) {
        self.data = Some(data);
        self.loading = false;
        self.more_loading = false;
        self.error = None;
    }

    pub fn fail(&mut self, message: impl Into<String>) {
        self.loading = false;
        self.more_loading = false;
        self.error = Some(message.into());
    }

    pub fn initial_loading(&self) -> bool {
        self.loading && self.data.is_none()
    }

    pub fn loaded(&self) -> Option<&T> {
        self.data.as_ref()
    }
}

/// Album/artist/playlist detail currently on screen.
#[derive(Debug)]
pub enum PageData {
    Album(AlbumPage),
    Artist(ArtistPage),
    Playlist(PlaylistPage),
}

impl PageData {
    pub fn title(&self) -> &str {
        match self {
            Self::Album(page) => &page.album.title,
            Self::Artist(page) => &page.artist.name,
            Self::Playlist(page) => &page.playlist.title,
        }
    }

    pub fn continuation(&self) -> Option<String> {
        match self {
            Self::Album(page) => page.continuation.clone(),
            Self::Artist(page) => page.continuation.clone(),
            Self::Playlist(page) => page.continuation.clone(),
        }
    }

    /// Playable songs in page order (used by Play all / Shuffle).
    pub fn songs(&self) -> Vec<Song> {
        match self {
            Self::Album(page) => page.songs.clone(),
            Self::Artist(page) => page.top_songs.clone(),
            Self::Playlist(page) => page.songs.clone(),
        }
    }
}

/// Detail page state; `route` is the page the data belongs to.
#[derive(Debug, Default)]
pub struct PageState {
    pub route: Option<Route>,
    pub data: Load<PageData>,
}

/// Search filter row, matching the Android app's six chips.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FilterChoice {
    #[default]
    All,
    Songs,
    Albums,
    Artists,
    Playlists,
    Videos,
}

impl FilterChoice {
    pub const ALL: [FilterChoice; 6] = [
        Self::All,
        Self::Songs,
        Self::Albums,
        Self::Artists,
        Self::Playlists,
        Self::Videos,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Songs => "Songs",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Playlists => "Playlists",
            Self::Videos => "Videos",
        }
    }

    pub fn filter(self) -> Option<SearchFilter> {
        match self {
            Self::All => None,
            Self::Songs => Some(SearchFilter::Songs),
            Self::Albums => Some(SearchFilter::Albums),
            Self::Artists => Some(SearchFilter::Artists),
            Self::Playlists => Some(SearchFilter::Playlists),
            Self::Videos => Some(SearchFilter::Videos),
        }
    }
}

#[derive(Debug)]
pub struct SearchState {
    pub query: String,
    /// The query whose results are on screen; used to drop stale responses.
    pub submitted: String,
    pub filter: FilterChoice,
    pub focus: bool,
    pub suggestions: Vec<String>,
    pub suggestions_open: bool,
    pub last_edit: Option<Instant>,
    pub results: Load<SearchPage>,
    pub history: Vec<String>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            query: String::new(),
            submitted: String::new(),
            filter: FilterChoice::All,
            focus: false,
            suggestions: Vec::new(),
            suggestions_open: false,
            last_edit: None,
            results: Load::default(),
            history: Vec::new(),
        }
    }
}

/// Everything the Library destination shows, loaded in one DB pass.
#[derive(Debug, Default)]
pub struct LibraryData {
    pub songs: Vec<Song>,
    pub albums: Vec<Album>,
    pub artists: Vec<ArtistItem>,
    pub playlists: Vec<PlaylistRecord>,
    pub history: Vec<HistoryEntry>,
    pub liked: Vec<Song>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LibTab {
    #[default]
    Songs,
    Albums,
    Artists,
    Playlists,
    Downloads,
    History,
}

impl LibTab {
    pub const ALL: [LibTab; 6] = [
        Self::Songs,
        Self::Albums,
        Self::Artists,
        Self::Playlists,
        Self::Downloads,
        Self::History,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Songs => "Songs",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Playlists => "Playlists",
            Self::Downloads => "Downloads",
            Self::History => "History",
        }
    }
}

#[derive(Debug)]
pub struct LibraryState {
    pub tab: LibTab,
    pub data: Load<LibraryData>,
    pub sort: LibrarySort,
    /// Song waiting for a playlist choice in the add-to-playlist modal.
    pub add_song: Option<Song>,
    pub new_playlist: String,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            tab: LibTab::Songs,
            data: Load::default(),
            sort: LibrarySort::default(),
            add_song: None,
            new_playlist: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalPlaylistView {
    pub id: String,
    pub name: String,
    pub songs: Vec<Song>,
}

#[derive(Debug, Default)]
pub struct LocalPlaylistState {
    pub id: Option<String>,
    pub data: Load<LocalPlaylistView>,
    pub rename: Option<String>,
    pub confirm_delete: bool,
}

/// Downloads tab: a snapshot refreshed from `DownloadManager` events.
#[derive(Debug, Default)]
pub struct DownloadsState {
    pub items: Vec<Download>,
    pub used: u64,
    /// Active manager budget; `None` means the manager is unlimited.
    pub limit: Option<u64>,
    /// Download manager could not start (missing directory/runtime).
    pub error: Option<String>,
    /// Completed entry awaiting a delete confirmation.
    pub confirm_remove: Option<Download>,
}

impl DownloadsState {
    pub fn failed_count(&self) -> usize {
        self.items
            .iter()
            .filter(|entry| entry.state == ml_downloads::DownloadState::Failed)
            .count()
    }
}

/// Stats and Wrapped share one selected period and one loaded report.
#[derive(Debug)]
pub struct StatsState {
    pub period: StatsPeriod,
    pub load: Load<StatsReport>,
    pub copied: bool,
}

impl Default for StatsState {
    fn default() -> Self {
        Self {
            period: StatsPeriod::Month,
            load: Load::default(),
            copied: false,
        }
    }
}

/// Recognition captures for twelve seconds, matching the Android pipeline.
pub const RECOGNITION_SECONDS: f32 = 12.0;

/// Accumulates positive player-position deltas while playing. History is
/// written when the track leaves the player; pauses add nothing and a seek jump
/// larger than [`MAX_POSITION_STEP_MS`] is treated as a jump, not listening.
#[derive(Debug, Clone)]
pub struct HistoryPending {
    song: Song,
    played_ms: u64,
    last_position_ms: u64,
    was_playing: bool,
}

/// Snapshots arrive around 10 Hz; anything larger is a seek, not playback.
const MAX_POSITION_STEP_MS: u64 = 2_000;

impl HistoryPending {
    pub fn new(song: &Song, position_ms: u64) -> Self {
        Self {
            song: song.clone(),
            played_ms: 0,
            last_position_ms: position_ms,
            was_playing: false,
        }
    }

    pub fn observe(&mut self, position_ms: u64, playing: bool) {
        if playing && self.was_playing {
            let delta = position_ms.saturating_sub(self.last_position_ms);
            if delta <= MAX_POSITION_STEP_MS {
                self.played_ms = self.played_ms.saturating_add(delta);
            }
        }
        self.last_position_ms = position_ms;
        self.was_playing = playing;
    }

    pub fn finish(self) -> (Song, u64) {
        let duration_ms = u64::from(self.song.duration.unwrap_or(0)).saturating_mul(1000);
        let played_ms = if duration_ms == 0 {
            self.played_ms
        } else {
            self.played_ms.min(duration_ms)
        };
        (self.song, played_ms)
    }
}

/// One successful Shazam match plus the timings the UI shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecognitionMatch {
    pub result: RecognitionResult,
    pub sample_ms: u64,
    pub elapsed_ms: u64,
}

/// Explicit user-driven recognition lifecycle. No microphone is touched until
/// [`RecognitionState::Listening`] is entered by the app.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RecognitionState {
    #[default]
    Idle,
    Listening {
        started: Instant,
    },
    Processing,
    Success {
        matched: Box<RecognitionMatch>,
    },
    Error {
        message: String,
    },
}

impl RecognitionState {
    pub fn start(&mut self, started: Instant) {
        *self = Self::Listening { started };
    }

    pub fn processing(&mut self) {
        *self = Self::Processing;
    }

    pub fn succeed(&mut self, matched: RecognitionMatch) {
        *self = Self::Success {
            matched: Box::new(matched),
        };
    }

    pub fn fail(&mut self, message: String) {
        *self = Self::Error { message };
    }

    pub fn cancel(&mut self) {
        *self = Self::Idle;
    }

    pub fn is_listening(&self) -> bool {
        matches!(self, Self::Listening { .. })
    }

    pub fn busy(&self) -> bool {
        matches!(self, Self::Listening { .. } | Self::Processing)
    }
}

/// Which generic browse request a [`BrowsePage`] belongs to. The UI compares
/// it with the active route before applying an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseTarget {
    Charts,
    NewReleases,
    MoodResults { params: Option<String> },
    YoutubeBrowse { id: String, params: Option<String> },
}
impl BrowseTarget {
    /// Target for a route that owns a generic browse page.
    pub fn from_route(route: &Route) -> Option<Self> {
        match route {
            Route::Charts => Some(Self::Charts),
            Route::NewReleases => Some(Self::NewReleases),
            Route::MoodResults { params, .. } => Some(Self::MoodResults {
                params: params.clone(),
            }),
            _ => None,
        }
    }

    pub fn matches_route(&self, route: &Route) -> bool {
        match (self, route) {
            (Self::Charts, Route::Charts) => true,
            (Self::NewReleases, Route::NewReleases) => true,
            (
                Self::MoodResults { params },
                Route::MoodResults {
                    params: current, ..
                },
            ) => params == current,
            (Self::YoutubeBrowse { .. }, _) => false,
            _ => false,
        }
    }
}

/// Maps a mixed InnerTube item to a playable song (videos and episodes use the
/// same player path as songs).
pub fn item_song(item: &YtItem) -> Option<Song> {
    match item {
        YtItem::Song(song) => Some(song.clone()),
        YtItem::Episode(episode) => Some(episode.as_song()),
        YtItem::Video(video) => Some(Song {
            id: video.id.clone(),
            title: video.title.clone(),
            artists: video.artists.clone(),
            duration: video.duration,
            thumbnail_url: video.thumbnail_url.clone(),
            explicit: video.explicit,
            ..Song::default()
        }),
        YtItem::Album(_) | YtItem::Artist(_) | YtItem::Playlist(_) => None,
    }
}

/// Where clicking a mixed item should navigate.
pub fn item_route(item: &YtItem) -> Option<Route> {
    match item {
        YtItem::Album(album) => Some(Route::Album {
            id: album.id.clone(),
        }),
        YtItem::Artist(artist) => Some(Route::Artist {
            id: artist
                .channel_id
                .clone()
                .unwrap_or_else(|| artist.id.clone()),
        }),
        YtItem::Playlist(playlist) => Some(Route::Playlist {
            id: playlist.id.clone(),
        }),
        YtItem::Song(_) | YtItem::Episode(_) | YtItem::Video(_) => None,
    }
}

/// Resized thumbnail URL for a card.
///
/// Only playable items may fall back to the video-id thumbnail pattern; a
/// playlist/album/artist id is not a video id, so those return `""` and the
/// card keeps its scheme placeholder instead of requesting a 404.
pub fn item_thumb(item: &YtItem, size: u32) -> String {
    match item.thumbnail() {
        Some(url) if !url.is_empty() => ml_core::resize_thumbnail_url(url, size),
        _ => match item {
            YtItem::Song(_) | YtItem::Episode(_) | YtItem::Video(_) => {
                ml_core::thumbnail_url(item.id(), size)
            }
            YtItem::Album(_) | YtItem::Artist(_) | YtItem::Playlist(_) => String::new(),
        },
    }
}

/// Card subtitle for any item kind.
pub fn item_subtitle(item: &YtItem) -> String {
    match item {
        YtItem::Song(song) => song.artist_names(),
        YtItem::Album(album) => album.artist_names(),
        YtItem::Artist(artist) => artist.subscribers.clone().unwrap_or_default(),
        YtItem::Playlist(playlist) => playlist
            .author
            .as_ref()
            .map(|author| author.name.clone())
            .or_else(|| playlist.song_count_text.clone())
            .unwrap_or_default(),
        YtItem::Episode(episode) => episode
            .author
            .as_ref()
            .map(|author| author.name.clone())
            .unwrap_or_default(),
        YtItem::Video(video) => video
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// First song-bearing item of a section (Play all button).
pub fn section_song_id(section: &HomeSection) -> Option<String> {
    section
        .items
        .iter()
        .find_map(|item| item_song(item).map(|song| song.id))
}

/// All playable songs in a section, in shelf order.
pub fn section_songs(section: &HomeSection) -> Vec<Song> {
    section.items.iter().filter_map(item_song).collect()
}

/// Merge a continuation feed into the existing home shelves.
pub fn merge_sections(sections: &mut Vec<HomeSection>, extra: Vec<HomeSection>) {
    for section in extra {
        match sections
            .iter_mut()
            .find(|existing| !existing.title.is_empty() && existing.title == section.title)
        {
            Some(existing) => existing.items.extend(section.items),
            None => sections.push(section),
        }
    }
}

pub fn merge_feed(home: &mut HomeFeed, extra: HomeFeed) {
    merge_sections(&mut home.sections, extra.sections);
    home.continuation = extra.continuation;
}

/// Translation provider selected in Lyrics settings. `Off` hides Translate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TranslationProvider {
    #[default]
    Off,
    DeepL,
    OpenAiCompatible,
}

impl TranslationProvider {
    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::DeepL => "deepl",
            Self::OpenAiCompatible => "openai",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "deepl" => Some(Self::DeepL),
            "openai" | "openai-compatible" => Some(Self::OpenAiCompatible),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::DeepL => "DeepL",
            Self::OpenAiCompatible => "OpenAI-compatible",
        }
    }

    pub fn enabled(self) -> bool {
        self != Self::Off
    }
}

/// UI-owned playback options that have no field in `ml_core::Settings`.
///
/// Persisted in the store's `kv` table so Settings survives restarts without
/// changing the frozen core settings shape.
#[derive(Debug)]
pub struct PlayerExtras {
    pub eq_enabled: bool,
    pub eq_preset: EqPreset,
    pub speed: f32,
    pub device: Option<String>,
    pub notifications: bool,
    pub crossfade_seconds: u32,
    pub crossfade_gapless: bool,
    pub skip_silence: bool,
    pub normalization: bool,
    pub normalization_gain_db: f32,
    pub sleep: SleepTimer,
    pub devices: Load<Vec<AudioDevice>>,
    pub translation_enabled: bool,
    pub translation_provider: TranslationProvider,
    pub translation_endpoint: String,
    pub translation_target: String,
    /// Integration toggles; credentials never live here.
    pub lastfm_enabled: bool,
    pub discord_enabled: bool,
    pub discord_app_id: String,
}

impl Default for PlayerExtras {
    fn default() -> Self {
        Self {
            eq_enabled: false,
            eq_preset: EqPreset::Flat,
            speed: 1.0,
            device: None,
            notifications: true,
            crossfade_seconds: 0,
            crossfade_gapless: true,
            skip_silence: false,
            normalization: false,
            normalization_gain_db: 0.0,
            sleep: SleepTimer::Off,
            devices: Load::default(),
            translation_enabled: false,
            translation_provider: TranslationProvider::Off,
            translation_endpoint: String::new(),
            translation_target: "en".to_string(),
            lastfm_enabled: false,
            discord_enabled: false,
            discord_app_id: String::new(),
        }
    }
}

impl PlayerExtras {
    pub fn from_kv(get: impl Fn(&str) -> Option<String>) -> Self {
        let mut extras = Self::default();
        extras.eq_enabled = get("player.eq_enabled").as_deref() == Some("1");
        extras.eq_preset = get("player.eq_preset")
            .and_then(|value| EqPreset::parse(&value))
            .unwrap_or_default();
        extras.speed = get("player.speed")
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|speed| speed.is_finite() && (0.25..=4.0).contains(speed))
            .unwrap_or(1.0);
        extras.device = get("player.device").filter(|id| !id.trim().is_empty());
        extras.notifications = get("player.notifications")
            .map(|value| value != "0")
            .unwrap_or(true);
        extras.crossfade_seconds = get("player.crossfade_seconds")
            .and_then(|value| value.parse::<f32>().ok())
            .map(crate::pure::crossfade_seconds)
            .unwrap_or(0);
        extras.crossfade_gapless = get("player.crossfade_gapless")
            .map(|value| value != "0")
            .unwrap_or(true);
        extras.skip_silence = get("player.skip_silence").as_deref() == Some("1");
        extras.normalization = get("player.normalization").as_deref() == Some("1");
        extras.normalization_gain_db = get("player.normalization_gain_db")
            .and_then(|value| value.parse::<f32>().ok())
            .map(crate::pure::normalization_gain)
            .unwrap_or(0.0);
        extras.sleep = get("player.sleep_timer")
            .as_deref()
            .map(parse_sleep_timer)
            .unwrap_or(SleepTimer::Off);
        extras.translation_enabled = get("lyrics.translation").as_deref() == Some("1");
        extras.translation_provider = get("lyrics.translation_provider")
            .as_deref()
            .and_then(TranslationProvider::from_key)
            .unwrap_or_default();
        extras.translation_endpoint = get("lyrics.translation_endpoint")
            .filter(|value| value.len() <= 2_048)
            .unwrap_or_default();
        extras.translation_target = get("lyrics.translation_target")
            .filter(|value| !value.trim().is_empty() && value.len() <= 64)
            .unwrap_or_else(|| "en".to_string());
        extras.lastfm_enabled = get("integrations.lastfm").as_deref() == Some("1");
        extras.discord_enabled = get("integrations.discord").as_deref() == Some("1");
        extras.discord_app_id = get("integrations.discord_app_id")
            .filter(|value| value.len() <= 64)
            .unwrap_or_default();
        extras
    }

    /// `(key, value)` pairs for `Store::kv_set`.
    pub fn persisted(&self) -> [(&'static str, String); 18] {
        [
            ("player.eq_enabled", (self.eq_enabled as u8).to_string()),
            ("player.eq_preset", self.eq_preset.key().to_string()),
            ("player.speed", format!("{:.2}", self.speed)),
            (
                "player.device",
                self.device.clone().unwrap_or_else(|| "auto".to_string()),
            ),
            (
                "player.notifications",
                (self.notifications as u8).to_string(),
            ),
            (
                "player.crossfade_seconds",
                self.crossfade_seconds.to_string(),
            ),
            (
                "player.crossfade_gapless",
                (self.crossfade_gapless as u8).to_string(),
            ),
            ("player.skip_silence", (self.skip_silence as u8).to_string()),
            (
                "player.normalization",
                (self.normalization as u8).to_string(),
            ),
            (
                "player.normalization_gain_db",
                format!("{:.1}", self.normalization_gain_db),
            ),
            ("player.sleep_timer", sleep_timer_value(self.sleep)),
            (
                "lyrics.translation",
                (self.translation_enabled as u8).to_string(),
            ),
            (
                "lyrics.translation_provider",
                self.translation_provider.key().to_string(),
            ),
            (
                "lyrics.translation_endpoint",
                self.translation_endpoint.clone(),
            ),
            ("lyrics.translation_target", self.translation_target.clone()),
            (
                "integrations.lastfm",
                (self.lastfm_enabled as u8).to_string(),
            ),
            (
                "integrations.discord",
                (self.discord_enabled as u8).to_string(),
            ),
            ("integrations.discord_app_id", self.discord_app_id.clone()),
        ]
    }
}

/// Non-secret session identity, safe to cache in `kv("auth.identity")`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthIdentity {
    pub data_sync_id: Option<String>,
    pub auth_user: Option<String>,
}

impl AuthIdentity {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "data_sync_id": self.data_sync_id,
            "auth_user": self.auth_user,
        })
        .to_string()
    }

    pub fn from_json(raw: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(raw).ok()?;
        if value.as_object().is_none_or(|object| object.is_empty()) {
            return None;
        }
        Some(Self {
            data_sync_id: value
                .get("data_sync_id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
            auth_user: value
                .get("auth_user")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        })
    }
}

/// Counts from one completed library sync.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncCounts {
    pub songs: usize,
    pub albums: usize,
    pub artists: usize,
    pub playlists: usize,
}

impl SyncCounts {
    pub fn total(self) -> usize {
        self.songs + self.albums + self.artists + self.playlists
    }
}

/// Cached account metadata in `kv("account")`. No cookies or secrets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountCache {
    pub info: AccountInfo,
    pub synced_at: Option<i64>,
    pub counts: SyncCounts,
    pub expires_at: Option<i64>,
}

impl AccountCache {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "name": self.info.name,
            "email": self.info.email,
            "channel_handle": self.info.channel_handle,
            "thumbnail_url": self.info.thumbnail_url,
            "channel_id": self.info.channel_id,
            "synced_at": self.synced_at,
            "counts": {
                "songs": self.counts.songs,
                "albums": self.counts.albums,
                "artists": self.counts.artists,
                "playlists": self.counts.playlists,
            },
            "expires_at": self.expires_at,
        })
        .to_string()
    }

    pub fn from_json(raw: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(raw).ok()?;
        let name = value.get("name")?.as_str()?.trim();
        if name.is_empty() {
            return None;
        }
        let text = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        let count = |key: &str| {
            value
                .get("counts")
                .and_then(|counts| counts.get(key))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize
        };
        Some(Self {
            info: AccountInfo {
                name: name.to_string(),
                email: text("email"),
                channel_handle: text("channel_handle"),
                thumbnail_url: text("thumbnail_url"),
                channel_id: text("channel_id"),
            },
            synced_at: value.get("synced_at").and_then(serde_json::Value::as_i64),
            counts: SyncCounts {
                songs: count("songs"),
                albums: count("albums"),
                artists: count("artists"),
                playlists: count("playlists"),
            },
            expires_at: value.get("expires_at").and_then(serde_json::Value::as_i64),
        })
    }
}

/// Credential/account state shown by the Account screen.
#[derive(Debug, Clone, Default)]
pub struct AccountSession {
    pub signed_in: bool,
    pub info: Option<AccountInfo>,
    pub identity: AuthIdentity,
    pub snapshot: AuthSnapshot,
    pub synced_at: Option<i64>,
    pub counts: SyncCounts,
    pub expires_at: Option<i64>,
}

/// UI-only account/import state.
#[derive(Debug, Clone, Default)]
pub struct AccountState {
    pub session: AccountSession,
    pub cache: Option<AccountCache>,
    pub imported_path: String,
    pub browser_profile: Option<String>,
    pub loading: bool,
    pub syncing: bool,
    pub error: Option<String>,
}

fn parse_sleep_timer(value: &str) -> SleepTimer {
    match value.trim() {
        "end" => SleepTimer::EndOfTrack,
        "off" | "" => SleepTimer::Off,
        other => other
            .parse::<u32>()
            .ok()
            .filter(|minutes| *minutes > 0 && *minutes <= 24 * 60)
            .map(SleepTimer::Minutes)
            .unwrap_or(SleepTimer::Off),
    }
}

fn sleep_timer_value(timer: SleepTimer) -> String {
    match timer {
        // Fade-out starts immediately and must not be re-armed on launch.
        SleepTimer::Off | SleepTimer::FadeOut(_) => "off".to_string(),
        SleepTimer::Minutes(minutes) => minutes.to_string(),
        SleepTimer::EndOfTrack => "end".to_string(),
    }
}

/// Preset 10-band curves in dB; actual application clamps via pure::clamp_eq_bands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EqPreset {
    #[default]
    Flat,
    Bass,
    Vocal,
    Rock,
    Electronic,
}

impl EqPreset {
    pub const ALL: [Self; 5] = [
        Self::Flat,
        Self::Bass,
        Self::Vocal,
        Self::Rock,
        Self::Electronic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Bass => "Bass",
            Self::Vocal => "Vocal",
            Self::Rock => "Rock",
            Self::Electronic => "Electronic",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Bass => "bass",
            Self::Vocal => "vocal",
            Self::Rock => "rock",
            Self::Electronic => "electronic",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.key() == value)
    }

    pub fn bands(self) -> [f32; 10] {
        match self {
            Self::Flat => [0.0; 10],
            Self::Bass => [6.0, 5.0, 4.0, 2.0, 0.0, -1.0, -2.0, -2.0, -1.0, 0.0],
            Self::Vocal => [-2.0, -1.0, 0.0, 2.0, 4.0, 5.0, 4.0, 2.0, 0.0, -1.0],
            Self::Rock => [4.0, 3.0, 2.0, 0.0, -1.0, 0.0, 2.0, 3.0, 4.0, 4.0],
            Self::Electronic => [5.0, 4.0, 2.0, 0.0, -1.0, 1.0, 3.0, 4.0, 5.0, 4.0],
        }
    }
}

/// Connection state of the Listen Together screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TogetherPhase {
    #[default]
    Idle,
    Connecting,
    Connected,
    Reconnecting {
        attempt: u32,
        max: u32,
    },
    Error,
}

impl TogetherPhase {
    pub fn label(self) -> String {
        match self {
            Self::Idle => "Not connected".to_string(),
            Self::Connecting => "Connecting…".to_string(),
            Self::Connected => "Connected".to_string(),
            Self::Reconnecting { attempt, max } => {
                format!("Reconnecting {attempt}/{max}…")
            }
            Self::Error => "Connection failed".to_string(),
        }
    }
}

/// UI-side Listen Together state. The client and its session token stay in
/// `Services`; this never persists a token.
#[derive(Debug)]
pub struct TogetherState {
    pub server: String,
    pub display_name: String,
    pub join_code: String,
    pub phase: TogetherPhase,
    pub session: Option<SessionSnapshot>,
    pub notices: Vec<String>,
    pub error: Option<String>,
    /// Events stamped with another generation are ignored after leave/rejoin.
    pub generation: u64,
    /// Screenshot fixture: do not feed room playback into the local player.
    pub demo: bool,
}

impl Default for TogetherState {
    fn default() -> Self {
        Self {
            server: DEFAULT_SERVER_URL.to_string(),
            display_name: "Listener".to_string(),
            join_code: String::new(),
            phase: TogetherPhase::Idle,
            session: None,
            notices: Vec::new(),
            error: None,
            generation: 0,
            demo: false,
        }
    }
}

impl TogetherState {
    pub fn new(display_name: impl Into<String>) -> Self {
        let mut state = Self::default();
        let name = display_name.into();
        if !name.trim().is_empty() {
            state.display_name = name;
        }
        state
    }

    pub fn begin(&mut self, generation: u64) {
        self.generation = generation;
        self.phase = TogetherPhase::Connecting;
        self.session = None;
        self.notices.clear();
        self.error = None;
        self.join_code = self.join_code.trim().to_ascii_uppercase();
    }

    pub fn leave(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.phase = TogetherPhase::Idle;
        self.session = None;
        self.notices.clear();
        self.error = None;
    }

    pub fn room(&self) -> Option<&RoomState> {
        self.session.as_ref()?.room.as_ref()
    }

    pub fn role(&self) -> Role {
        self.session
            .as_ref()
            .map_or(Role::None, |session| session.role)
    }

    pub fn is_host(&self) -> bool {
        self.role() == Role::Host && self.room().is_some()
    }

    pub fn is_guest(&self) -> bool {
        self.role() == Role::Guest && self.room().is_some()
    }

    pub fn room_code(&self) -> Option<&str> {
        self.session
            .as_ref()
            .and_then(|session| session.room_code.as_deref())
            .filter(|code| !code.is_empty())
            .or_else(|| self.room().map(|room| room.room_code.as_str()))
            .filter(|code| !code.is_empty())
    }

    pub fn user_id(&self) -> Option<&str> {
        self.session.as_ref()?.user_id.as_deref()
    }

    fn notice(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.notices.last() == Some(&text) {
            return;
        }
        self.notices.push(text);
        if self.notices.len() > 4 {
            self.notices.remove(0);
        }
    }

    /// Fold one client event. Returns false for events from an older client
    /// generation, which the screen must ignore after leave/rejoin.
    pub fn apply_event(&mut self, generation: u64, event: TogetherEvent) -> bool {
        if generation != self.generation {
            return false;
        }
        match event {
            TogetherEvent::Connected => {
                self.phase = TogetherPhase::Connected;
                self.notice("Connected to the room server");
            }
            TogetherEvent::Disconnected {
                will_reconnect: true,
            } => {
                if !matches!(self.phase, TogetherPhase::Reconnecting { .. }) {
                    self.phase = TogetherPhase::Reconnecting {
                        attempt: 1,
                        max: MAX_RECONNECT_ATTEMPTS,
                    };
                }
                self.notice("Connection lost; reconnecting");
            }
            TogetherEvent::Disconnected {
                will_reconnect: false,
            } => {
                if self.phase != TogetherPhase::Idle {
                    self.phase = TogetherPhase::Error;
                    self.session = None;
                    self.error
                        .get_or_insert_with(|| "Disconnected from the room server".to_string());
                }
            }
            TogetherEvent::Reconnecting {
                attempt,
                max_attempts,
            } => {
                self.phase = TogetherPhase::Reconnecting {
                    attempt,
                    max: max_attempts,
                };
                self.notice(format!("Reconnecting ({attempt}/{max_attempts})"));
            }
            TogetherEvent::ConnectionError(message) => {
                self.error = Some(message);
                if self.session.is_none() {
                    self.phase = TogetherPhase::Error;
                }
            }
            TogetherEvent::Session(event) => match *event {
                SessionEvent::Snapshot(snapshot) => {
                    let joined = snapshot.room.is_some();
                    self.session = Some(snapshot);
                    if joined {
                        self.phase = TogetherPhase::Connected;
                        self.error = None;
                    }
                }
                SessionEvent::RoomCreated { room_code, .. } => {
                    self.notice(format!("Room {room_code} created"));
                }
                SessionEvent::JoinApproved { room_code, .. } => {
                    self.join_code = room_code.clone();
                    self.notice(format!("Joined room {room_code}"));
                }
                SessionEvent::JoinRequest { username, .. } => {
                    self.notice(format!("{username} wants to join"));
                }
                SessionEvent::JoinRejected { reason } => {
                    self.phase = TogetherPhase::Error;
                    self.session = None;
                    self.error = Some(if reason.trim().is_empty() {
                        "The host rejected the join request".to_string()
                    } else {
                        format!("Join rejected: {reason}")
                    });
                }
                SessionEvent::UserJoined { username, .. } => {
                    self.notice(format!("{username} joined"));
                }
                SessionEvent::UserLeft { username, .. } => {
                    self.notice(format!("{username} left"));
                }
                SessionEvent::HostChanged { new_host_name, .. } => {
                    self.notice(format!("{new_host_name} is now the host"));
                }
                SessionEvent::Kicked { reason } => {
                    self.phase = TogetherPhase::Error;
                    self.session = None;
                    self.error = Some(if reason.trim().is_empty() {
                        "You were removed from the room".to_string()
                    } else {
                        format!("Removed from the room: {reason}")
                    });
                }
                SessionEvent::Reconnected { room_code, .. } => {
                    self.phase = TogetherPhase::Connected;
                    self.notice(format!("Reconnected to room {room_code}"));
                }
                SessionEvent::UserReconnected { username, .. } => {
                    self.notice(format!("{username} reconnected"));
                }
                SessionEvent::UserDisconnected { username, .. } => {
                    self.notice(format!("{username} disconnected"));
                }
                SessionEvent::SuggestionReceived(suggestion) => {
                    self.notice(format!(
                        "{} suggested {}",
                        suggestion.from_username,
                        suggestion
                            .track_info
                            .as_ref()
                            .map_or("a track", |track| track.title.as_str())
                    ));
                }
                SessionEvent::SuggestionApproved(approved) => {
                    let title = approved
                        .track_info
                        .as_ref()
                        .map_or("a track", |track| track.title.as_str());
                    self.notice(format!("Suggestion approved: {title}"));
                }
                SessionEvent::SuggestionRejected(payload) => {
                    let reason = payload.reason;
                    self.notice(if reason.trim().is_empty() {
                        "Suggestion rejected".to_string()
                    } else {
                        format!("Suggestion rejected: {reason}")
                    });
                }
                SessionEvent::ServerError { code, message } => {
                    self.error = Some(if message.trim().is_empty() {
                        format!("Server error: {code}")
                    } else {
                        format!("Server error ({code}): {message}")
                    });
                }
                SessionEvent::PlaybackSync(_)
                | SessionEvent::BufferWait { .. }
                | SessionEvent::BufferComplete { .. }
                | SessionEvent::SyncStateReceived(_) => {}
            },
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn read(extras: &PlayerExtras) -> PlayerExtras {
        let values: HashMap<&str, String> = extras.persisted().into_iter().collect();
        PlayerExtras::from_kv(|key| values.get(key).cloned())
    }

    #[test]
    fn translation_preferences_round_trip_without_secrets() {
        let extras = PlayerExtras {
            translation_enabled: true,
            translation_provider: TranslationProvider::OpenAiCompatible,
            translation_endpoint: "https://llm.example/v1/chat/completions".to_string(),
            translation_target: "de".to_string(),
            lastfm_enabled: true,
            discord_enabled: true,
            discord_app_id: "123456789012345678".to_string(),
            ..PlayerExtras::default()
        };
        let persisted: std::collections::HashMap<String, String> = extras
            .persisted()
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();
        assert_eq!(
            persisted.get("lyrics.translation").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            persisted
                .get("lyrics.translation_provider")
                .map(String::as_str),
            Some("openai")
        );
        assert!(!persisted.keys().any(|key| key.contains("key")));
        let decoded = PlayerExtras::from_kv(|key| persisted.get(key).cloned());
        assert!(decoded.translation_enabled);
        assert_eq!(
            decoded.translation_provider,
            TranslationProvider::OpenAiCompatible
        );
        assert_eq!(
            decoded.translation_endpoint,
            "https://llm.example/v1/chat/completions"
        );
        assert_eq!(decoded.translation_target, "de");
        assert!(decoded.lastfm_enabled);
        assert!(decoded.discord_enabled);
        assert_eq!(decoded.discord_app_id, "123456789012345678");
        assert!(!persisted.keys().any(|key| key.contains("api")));
        assert!(!persisted.keys().any(|key| key.contains("secret")));
        assert!(!persisted.keys().any(|key| key.contains("session")));
        assert_eq!(TranslationProvider::from_key("bogus"), None);
    }

    #[test]
    fn recognition_state_is_explicit_and_cancelable() {
        let mut state = RecognitionState::default();
        assert_eq!(state, RecognitionState::Idle);
        assert!(!state.busy());
        state.start(Instant::now());
        assert!(state.is_listening());
        assert!(state.busy());
        state.processing();
        assert_eq!(state, RecognitionState::Processing);
        assert!(state.busy());
        state.succeed(RecognitionMatch {
            result: RecognitionResult::default(),
            sample_ms: 12_000,
            elapsed_ms: 1_200,
        });
        assert!(matches!(state, RecognitionState::Success { .. }));
        assert!(!state.busy());
        state.fail("No match".to_string());
        assert_eq!(
            state,
            RecognitionState::Error {
                message: "No match".to_string()
            }
        );
        state.cancel();
        assert_eq!(state, RecognitionState::Idle);
    }

    #[test]
    fn pending_history_clamps_to_track_duration() {
        let song = Song {
            duration: Some(30),
            ..Song::default()
        };
        let mut pending = HistoryPending::new(&song, 0);
        for second in 0..=30 {
            pending.observe(second as u64 * 1_000, true);
        }
        let (_, played) = pending.finish();
        assert_eq!(played, 30_000);
        // Pause adds nothing; a seek jump larger than the step cap is ignored.
        let mut pending = HistoryPending::new(&song, 0);
        pending.observe(1_000, true);
        pending.observe(5_000, false);
        pending.observe(5_000, true);
        pending.observe(9_000, true);
        pending.observe(10_000, true);
        let (_, played) = pending.finish();
        assert_eq!(played, 1_000);
        let mut pending = HistoryPending::new(&Song::default(), 0);
        for second in 0..=45 {
            pending.observe(second as u64 * 1_000, true);
        }
        let (_, played) = pending.finish();
        assert_eq!(played, 45_000);
    }

    #[test]
    fn browse_targets_match_only_their_route() {
        assert_eq!(
            BrowseTarget::from_route(&Route::Charts),
            Some(BrowseTarget::Charts)
        );
        assert!(BrowseTarget::Charts.matches_route(&Route::Charts));
        assert!(!BrowseTarget::Charts.matches_route(&Route::NewReleases));
        assert_eq!(
            BrowseTarget::from_route(&Route::NewReleases),
            Some(BrowseTarget::NewReleases)
        );
        let moods = Route::MoodResults {
            params: Some("ggMP".to_string()),
            title: Some("Autumn".to_string()),
        };
        assert_eq!(
            BrowseTarget::from_route(&moods),
            Some(BrowseTarget::MoodResults {
                params: Some("ggMP".to_string())
            })
        );
        assert!(BrowseTarget::MoodResults {
            params: Some("ggMP".to_string())
        }
        .matches_route(&moods));
        assert!(!BrowseTarget::MoodResults {
            params: Some("ggMC".to_string())
        }
        .matches_route(&moods));
        assert_eq!(BrowseTarget::from_route(&Route::Moods), None);
        assert_eq!(
            BrowseTarget::from_route(&Route::Podcast {
                id: "MPSP".to_string()
            }),
            None
        );
        assert!(!BrowseTarget::YoutubeBrowse {
            id: "FEmusic_home".to_string(),
            params: None
        }
        .matches_route(&Route::Charts));
    }

    #[test]
    fn account_cache_round_trip_keeps_optional_fields() {
        let cache = AccountCache {
            info: ml_innertube::auth::AccountInfo {
                name: "Rahul".to_string(),
                email: Some("rahul@example.com".to_string()),
                channel_handle: Some("@rahul".to_string()),
                thumbnail_url: Some("https://example.test/avatar.jpg".to_string()),
                channel_id: Some("UCxyz".to_string()),
            },
            synced_at: Some(1_700_000_000_000),
            counts: SyncCounts {
                songs: 12,
                albums: 3,
                artists: 2,
                playlists: 1,
            },
            expires_at: Some(1_800_000_000),
        };
        let json = cache.to_json();
        assert!(!json.to_ascii_lowercase().contains("sapisid"));
        assert!(!json.to_ascii_lowercase().contains("cookie"));
        let restored = AccountCache::from_json(&json).expect("restore cache");
        assert_eq!(restored.info.name, "Rahul");
        assert_eq!(restored.synced_at, Some(1_700_000_000_000));
        assert_eq!(restored.counts.total(), 18);
        assert_eq!(restored.expires_at, Some(1_800_000_000));
        assert!(AccountCache::from_json("not json").is_none());
        assert!(AccountCache::from_json("{}").is_none());
        assert_eq!(SyncCounts::default().total(), 0);

        let identity = AuthIdentity {
            data_sync_id: Some("UCabc||UCdef".to_string()),
            auth_user: Some("2".to_string()),
        };
        assert_eq!(
            AuthIdentity::from_json(&identity.to_json()).expect("identity"),
            identity
        );
        assert!(AuthIdentity::from_json("[]").is_none());
    }

    #[test]
    fn player_extras_round_trip() {
        let mut extras = PlayerExtras {
            eq_enabled: true,
            eq_preset: EqPreset::Rock,
            speed: 1.25,
            device: Some("auto".to_string()),
            notifications: false,
            crossfade_seconds: 17,
            crossfade_gapless: false,
            skip_silence: true,
            normalization: true,
            normalization_gain_db: -6.5,
            sleep: SleepTimer::Minutes(30),
            devices: Load::default(),
            ..PlayerExtras::default()
        };
        let restored = read(&extras);
        assert!(restored.eq_enabled);
        assert_eq!(restored.eq_preset, EqPreset::Rock);
        assert_eq!(restored.speed, 1.25);
        assert_eq!(restored.device.as_deref(), Some("auto"));
        assert!(!restored.notifications);
        assert_eq!(restored.crossfade_seconds, 17);
        assert!(!restored.crossfade_gapless);
        assert!(restored.skip_silence);
        assert!(restored.normalization);
        assert_eq!(restored.normalization_gain_db, -6.5);
        assert_eq!(restored.sleep, SleepTimer::Minutes(30));

        extras.sleep = SleepTimer::FadeOut(std::time::Duration::from_secs(30));
        let values: HashMap<&str, String> = extras.persisted().into_iter().collect();
        assert_eq!(values.get("player.sleep_timer"), Some(&"off".to_string()));
        assert_eq!(values.get("player.device"), Some(&"auto".to_string()));

        let values = HashMap::from([
            ("player.crossfade_seconds", "99".to_string()),
            ("player.normalization_gain_db", "-99".to_string()),
            ("player.speed", "NaN".to_string()),
            ("player.sleep_timer", "bogus".to_string()),
        ]);
        let clamped = PlayerExtras::from_kv(|key| values.get(key).cloned());
        assert_eq!(clamped.crossfade_seconds, 30);
        assert_eq!(clamped.normalization_gain_db, -15.0);
        assert_eq!(clamped.speed, 1.0);
        assert_eq!(clamped.sleep, SleepTimer::Off);
    }

    fn together_snapshot(role: Role) -> SessionSnapshot {
        SessionSnapshot {
            room: Some(RoomState {
                room_code: "ROOM1234".to_string(),
                host_id: "u-host".to_string(),
                ..RoomState::default()
            }),
            role,
            user_id: Some(
                if role == Role::Host {
                    "u-host"
                } else {
                    "u-guest"
                }
                .to_string(),
            ),
            room_code: Some("ROOM1234".to_string()),
            username: Some(if role == Role::Host { "Nyx" } else { "Ava" }.to_string()),
            pending_join_requests: Vec::new(),
            pending_suggestions: Vec::new(),
            buffering_users: Vec::new(),
        }
    }

    #[test]
    fn together_stale_events_are_ignored_and_leave_clears_the_room() {
        let mut state = TogetherState::new("Nyx");
        state.begin(9);
        assert!(state.apply_event(
            9,
            TogetherEvent::Session(Box::new(SessionEvent::Snapshot(together_snapshot(
                Role::Host
            ))))
        ));
        assert!(state.is_host());
        assert_eq!(state.room_code(), Some("ROOM1234"));

        assert!(!state.apply_event(
            8,
            TogetherEvent::ConnectionError("stale failure".to_string())
        ));
        assert!(state.error.is_none());

        state.leave();
        assert!(state.room().is_none());
        assert_eq!(state.phase, TogetherPhase::Idle);
        assert!(!state.is_host());
    }

    #[test]
    fn together_join_rejection_and_guest_role_are_mapped() {
        let mut guest = TogetherState::new("Ava");
        guest.begin(3);
        assert!(!guest.is_guest());
        guest.apply_event(
            3,
            TogetherEvent::Session(Box::new(SessionEvent::Snapshot(together_snapshot(
                Role::Guest,
            )))),
        );
        assert!(guest.is_guest());
        assert_eq!(guest.phase, TogetherPhase::Connected);

        let mut rejected = TogetherState::new("Ava");
        rejected.begin(4);
        rejected.apply_event(
            4,
            TogetherEvent::Session(Box::new(SessionEvent::JoinRejected {
                reason: "Room is full".to_string(),
            })),
        );
        assert_eq!(rejected.phase, TogetherPhase::Error);
        assert!(rejected
            .error
            .as_deref()
            .is_some_and(|error| error.contains("Room is full")));
    }
}
