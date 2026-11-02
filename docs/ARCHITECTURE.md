# Architecture — Metrolist Linux

Status: release candidate, 2026-11-02. This document describes the code that
exists in this workspace, not the target design. Sections marked **planned** do
not exist yet; §8 lists the known gaps.

Upstream Android source (separate read-only checkout, not vendored) at version 13.7.0, GPL-3.0-only.
This port is unofficial; see `NOTICE`.

## 1. Crate ownership map

Cargo workspace, resolver 2, Rust 1.99.1 (`rust-toolchain.toml`).

| Crate | Package | Path | Owner / state |
|---|---|---|---|
| `core` | `ml-core` | `crates/core/` | **Frozen** foundation. Models, queue, settings, events, media types, errors, utils. |
| `innertube` | `ml-innertube` | `crates/innertube/` | InnerTube workstream. Live unauthenticated WEB_REMIX client, parsers and stream resolution. The player/stream path uses VISIONOS and needs no PO token; cookie-session account auth and library snapshots are implemented, while a PO-token provider is not. |
| `lyrics` | `ml-lyrics` | `crates/lyrics/` | Lyrics workstream. Provider ladder, LRC/TTML parsing, Japanese/Korean/Chinese/Hindi/Macedonian romanisation and key-gated translation. |
| `store` | `ml-store` | `crates/store/` | Store workstream. SQLite schema v1 forward migrations and typed CRUD for library, likes, history, playlists, queue and settings. |
| `downloads` | `ml-downloads` | `crates/downloads/` | Downloads workstream. Manifest-backed queue/worker pool, ranged VISIONOS fetches, progress, retry/remove/clear, storage budget. |
| `audio` | `ml-audio` | `crates/audio/` | Audio workstream. mpv JSON IPC engine, controller, EQ, device listing. |
| `integrations` | `ml-integrations` | `crates/integrations/` | MPRIS, tray, now-playing notifications, single-instance forwarding, optional Discord presence. |
| `ui` | `ml-ui` | `crates/ui/` | M3 theme, widgets, shell, live screens, `metrolist` binary. |

Other owned paths:

- `crates/core/src/` — `lib.rs`, `models.rs`, `media.rs`, `queue.rs`, `player.rs`,
  `settings.rs`, `events.rs`, `error.rs`, `util.rs`.
- `crates/ui/src/` — `lib.rs`, `main.rs`, `app.rs`, `routes.rs`, `services.rs`,
  `state.rs`, `art.rs`, `desktop.rs`, `pure.rs`, `mock.rs`, `settings_store.rs`,
  `theme/{mod,scheme,fonts,type_scale}.rs`,
  `widgets/{mod,controls,rows,feedback}.rs`, `shell/mod.rs`,
  `screens/{mod,home,search,detail,library,liked,local_playlist,downloads,history,
  lyrics,player,queue,settings,account,catalog,stats,recognize,placeholder}.rs`.
- `crates/ui/assets/fonts/` — fonts compiled into the binary with
  `include_bytes!` (provenance and hashes in `assets/fonts/SOURCES.md`).
- Root: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.github/workflows/ci.yml`,
  `README.md`, `LICENSE`, `NOTICE`.
- Assets/packaging (separate owner): `assets/`, `packaging/`, `.github/`,
  `docs/PARITY.md`, `docs/TEST-PARITY.md`.
- Parity/test maps: `docs/PARITY.md` (feature matrix, done/partial/not-ported),
  `docs/TEST-PARITY.md` (upstream test → Rust requirement).

## 2. Frozen public interfaces

Treat these as the integration contracts. Changes must be additive and
serde-defaulted; removing/renaming a field or method is a breaking change.
`ml-core` is the only crate every other crate depends on.

### 2.1 `ml-core`

```rust
pub type MlResult<T> = Result<T, MlError>;

pub enum MlError {
    Unimplemented(String), Network(String), Http { status: u16, message: String },
    Parse(String), NotFound(String), Unavailable(String), AuthRequired,
    Io(String), Store(String), Playback(String), Lyrics(String), Cancelled,
}
impl MlError { pub fn unimplemented(what: impl Into<String>) -> Self; }
```

Errors are `Clone + PartialEq + Eq + thiserror::Error`; `From<std::io::Error>`
maps to `Io`.

Models (`crates/core/src/models.rs`, all serde):

```rust
pub struct Artist { pub id: Option<String>, pub name: String }
pub struct AlbumRef { pub id: String, pub title: String }
pub struct Song {
    pub id: String, pub title: String, pub artists: Vec<Artist>,
    pub duration: Option<u32>, pub thumbnail_url: Option<String>,
    pub album: Option<AlbumRef>, pub explicit: bool,
    pub set_video_id: Option<String>, pub music_video_type: Option<String>,
    pub is_episode: bool, pub liked: Option<i64>, pub in_library: Option<i64>,
    pub library_add_token: Option<String>, pub library_remove_token: Option<String>,
}
pub struct Album { pub id: String, pub playlist_id: String, pub title: String,
    pub artists: Vec<Artist>, pub year: Option<u16>, pub thumbnail_url: Option<String>,
    pub explicit: bool, pub song_count: Option<u32>, pub duration: Option<u32> }
pub struct ArtistItem { pub id: String, pub name: String,
    pub thumbnail_url: Option<String>, pub channel_id: Option<String>,
    pub subscribers: Option<String> }
pub struct Playlist { pub id: String, pub title: String, pub author: Option<Artist>,
    pub song_count_text: Option<String>, pub thumbnail_url: Option<String>,
    pub description: Option<String>, pub is_editable: bool, pub is_podcast: bool,
    pub explicit: bool }
pub struct Episode { pub id: String, pub title: String, pub author: Option<Artist>,
    pub podcast: Option<AlbumRef>, pub duration: Option<u32>,
    pub publish_date_text: Option<String>, pub thumbnail_url: Option<String>,
    pub explicit: bool }
pub struct Video { pub id: String, pub title: String, pub artists: Vec<Artist>,
    pub duration: Option<u32>, pub thumbnail_url: Option<String>, pub explicit: bool }
pub enum YtItem { Song(Song), Album(Album), Artist(ArtistItem),
                  Playlist(Playlist), Episode(Episode), Video(Video) }
pub enum SectionKind { Unknown, QuickPicks, KeepListening, NewReleases, Songs,
                       Albums, Artists, Playlists, Videos, Episodes, Related }
pub struct HomeSection { pub title: String, pub items: Vec<YtItem>,
                         pub continuation: Option<String>, pub kind: SectionKind }
pub struct HomeFeed { pub sections: Vec<HomeSection>, pub continuation: Option<String> }
pub struct SearchPage { pub query: String, pub items: Vec<YtItem>,
                        pub songs: Vec<Song>, pub continuation: Option<String> }
pub struct AlbumPage { pub album: Album, pub songs: Vec<Song>,
                       pub description: Option<String>, pub continuation: Option<String> }
pub struct ArtistPage { pub artist: ArtistItem, pub top_songs: Vec<Song>,
                        pub sections: Vec<HomeSection>, pub description: Option<String>,
                        pub continuation: Option<String> }
pub struct PlaylistPage { pub playlist: Playlist, pub songs: Vec<Song>,
                          pub continuation: Option<String> }
pub struct QueuePage { pub title: Option<String>, pub songs: Vec<Song>,
                       pub current_index: Option<usize>, pub continuation: Option<String> }
pub struct LibraryPage { pub songs: Vec<Song>, pub albums: Vec<Album>,
                         pub playlists: Vec<Playlist>, pub artists: Vec<ArtistItem>,
                         pub continuation: Option<String> }
pub struct LyricsWord { pub start_ms: u64, pub end_ms: u64, pub text: String }
pub struct LyricsLine {
    pub start_ms: u64, pub end_ms: Option<u64>, pub text: String,
    #[serde(default)] pub words: Vec<LyricsWord>,       // word/segment timings
    #[serde(default)] pub background: bool,             // TTML background vocals
    #[serde(default)] pub agent: Option<String>,        // duet voice id
}
pub struct LyricsDocument { pub provider: String, pub lines: Vec<LyricsLine>,
    pub plain: Option<String>, pub translated: Option<Vec<LyricsLine>>,
    pub language: Option<String> }
impl LyricsDocument { pub fn is_synced(&self) -> bool; } // any start_ms > 0
```

Helpers: `Song::thumb(size)`, `artist_names()`, `duration_text()`,
`is_video_song()`, `share_url()`, and equivalents on `Album`, `Playlist`,
`ArtistItem`, `Episode`, `Video`; `YtItem::{id,title,thumbnail,explicit,as_song}`;
`Episode::as_song()`.

Queue (`crates/core/src/queue.rs`) — real state machine, unit-tested:

```rust
pub enum RepeatMode { Off, All, One }   // cycle(): Off -> All -> One -> Off
pub enum QueueSource { List, Album, Playlist, Radio, Artist, Local }
pub struct Queue { /* private: songs, order, position, shuffle, repeat, source, seed */ }

impl Queue {
    pub fn new(songs: Vec<Song>, source: QueueSource) -> Self;
    pub fn len(&self) -> usize;            pub fn is_empty(&self) -> bool;
    pub fn position(&self) -> usize;       pub fn source(&self) -> QueueSource;
    pub fn set_source(&mut self, source: QueueSource);
    pub fn is_shuffled(&self) -> bool;     pub fn repeat_mode(&self) -> RepeatMode;
    pub fn set_repeat(&mut self, repeat: RepeatMode);
    pub fn cycle_repeat(&mut self) -> RepeatMode;
    pub fn current(&self) -> Option<&Song>;
    pub fn song_at(&self, position: usize) -> Option<&Song>;
    pub fn songs(&self) -> &[Song];        // insertion order
    pub fn iter(&self) -> impl Iterator<Item = &Song>; // playback order
    pub fn jump_to(&mut self, position: usize) -> Option<&Song>;
    pub fn append(&mut self, song: Song);
    pub fn insert_next(&mut self, song: Song);
    pub fn remove(&mut self, position: usize) -> Option<Song>;
    pub fn move_item(&mut self, from: usize, to: usize) -> bool;
    pub fn clear(&mut self);
    pub fn replace_songs(&mut self, songs: Vec<Song>);
    pub fn set_shuffle(&mut self, enabled: bool);
    pub fn reshuffle(&mut self);
    pub fn next(&mut self) -> Option<&Song>;
    pub fn previous(&mut self) -> Option<&Song>;
}
```

Invariants: `order` is a permutation of `0..songs.len()`, `position` indexes
`order`, shuffle keeps the current track first and is a deterministic LCG
permutation, repeat `Off` stops at both ends without moving, `One` stays put.

Player/media/events:

```rust
pub enum PlaybackStatus { Idle, Loading, Buffering, Playing, Paused, Stopped, Ended, Error }
pub struct PlayerSnapshot { pub status: PlaybackStatus, pub position_ms: u64,
    pub duration_ms: u64, pub volume: f32, pub muted: bool, pub shuffle: bool,
    pub repeat: RepeatMode, pub current: Option<Song>, pub queue_len: usize,
    pub queue_index: Option<usize>, pub buffering: bool }
impl PlayerSnapshot { pub fn is_playing(&self) -> bool; }

pub struct MediaInfo { pub video_id: String, pub title: Option<String>,
    pub author: Option<String>, pub author_id: Option<String>,
    pub author_thumbnail: Option<String>, pub description: Option<String>,
    pub upload_date: Option<String>, pub subscribers: Option<String>,
    pub view_count: Option<u64>, pub like: Option<u64> }
pub struct StreamInfo { pub url: String, pub mime: String, pub bitrate: Option<u32>,
    pub itag: u32, pub content_length: Option<u64>, pub expires_at: Option<i64> }
impl StreamInfo { pub fn is_expired(&self, now_epoch_secs: i64) -> bool; }

pub enum AppEvent { HomeLoaded(HomeFeed), SearchLoaded(SearchPage),
    AlbumLoaded(AlbumPage), ArtistLoaded(ArtistPage), PlaylistLoaded(PlaylistPage),
    QueueLoaded(QueuePage), LyricsLoaded(LyricsDocument), LibraryLoaded(LibraryPage),
    PlayerState(PlayerSnapshot), StreamResolved(StreamInfo), Error(MlError),
    Toast(String) }
```

`AppEvent` is the UI-facing result vocabulary. The UI owns all state; backend
tasks only produce these values. Adding a variant is additive.

Settings (`crates/core/src/settings.rs`, `#[serde(default)]` on the struct and
nested structs):

```rust
pub enum ThemeMode { System, Light, Dark, PureBlack }
pub enum AudioQuality { Auto, Low, High }
pub enum NavStyle { Auto, Bottom, Rail }
pub enum LibrarySort { RecentlyAdded, Alphabetical, MostPlayed, Artist }
pub struct ProxyConfig { enabled: bool, url: String, username: String, password: String }
pub struct LyricsSettings { lrclib, lyrics_plus, kugou, better_lyrics, paxsenix, zemer, romanize: bool }
pub struct DownloadSettings { directory: Option<String>, concurrent_limit: u8, normalize: bool }
pub struct Settings { theme_mode, seed_color: u32 /* default 0xED5564 */,
    dynamic_accent: bool, audio_quality, content_language, content_country,
    proxy, lyrics, downloads, audio_normalization, crossfade_enabled,
    crossfade_seconds, sleep_timer_minutes, sleep_timer_fade, library_sort,
    nav_style, hide_explicit, hide_video_songs }
impl Settings { pub fn seed_rgb(&self) -> (u8, u8, u8); }
```

Missing JSON fields fall back to defaults. Path resolution is **not** in core;
it lives in `ml-ui`.

Utilities (`crates/core/src/util.rs`):

```rust
format_duration(u32) -> String;                    // m:ss or h:mm:ss
format_duration_opt(Option<u32>) -> String;        // "--:--" when None
format_position(u64) -> String;                    // ms -> m:ss
thumbnail_url(video_id: &str, size: u32) -> String;
resize_thumbnail_url(url: &str, size: u32) -> String;
parse_video_id(input: &str) -> Option<String>;
is_video_id(&str) -> bool; now_millis() -> u64; millis_to_seconds(u64) -> u64;
```

### 2.2 `ml-innertube`

```rust
pub struct ClientConfig { pub language: String, pub country: String,
    pub visitor_data: Option<String>, pub cookie: Option<String>,
    pub data_sync_id: Option<String>, pub auth_user: Option<String>,
    pub timeout_secs: u64, pub proxy: Option<ProxyConfig> }
pub enum SearchFilter { Songs, Albums, Artists, Playlists, Videos }
impl SearchFilter { pub fn params(self) -> &'static str; }

pub struct InnertubeClient; // Clone; shares HTTP client and caches

impl InnertubeClient {
    pub fn new(config: ClientConfig) -> Self;
    pub fn config(&self) -> &ClientConfig;
    pub fn http(&self) -> &reqwest::Client;
    pub fn set_cookie(&mut self, cookie: Option<String>);
    pub fn set_cookie_jar(&mut self, jar: &CookieJar);
    pub fn cookie_jar(&self) -> CookieJar;
    pub fn set_session_path(&mut self, path: Option<PathBuf>); // 0600 rotation
    pub fn set_login_identity(&mut self, data_sync_id: Option<String>, auth_user: Option<String>);
    pub fn clear_auth(&mut self);
    pub fn is_authenticated(&self) -> bool;
    pub fn auth_snapshot(&self) -> AuthSnapshot; // no cookie values
    pub async fn discover_session(&mut self) -> MlResult<SessionIdentity>;
    pub async fn account_info(&self) -> MlResult<AccountInfo>;
    pub async fn library_snapshot(&self) -> MlResult<LibrarySnapshot>;
    pub fn set_token_provider(&mut self, provider: Arc<dyn TokenProvider>);
    pub fn clear_token_provider(&mut self);
    pub async fn refresh_client_config(&mut self) -> MlResult<()>;

    pub async fn home(&self) -> MlResult<HomeFeed>;
    pub async fn home_continuation(&self, token: &str) -> MlResult<HomeFeed>;
    pub async fn search(&self, query: &str, filter: Option<SearchFilter>) -> MlResult<SearchPage>;
    pub async fn search_continuation(&self, token: &str) -> MlResult<SearchPage>;
    pub async fn suggestions(&self, query: &str) -> MlResult<Vec<String>>;
    pub async fn album(&self, browse_id: &str) -> MlResult<AlbumPage>;
    pub async fn album_continuation(&self, token: &str) -> MlResult<AlbumPage>;
    pub async fn artist(&self, channel_id: &str) -> MlResult<ArtistPage>;
    pub async fn artist_details(&self, channel_id: &str) -> MlResult<ArtistDetails>;
    pub async fn artist_continuation(&self, token: &str) -> MlResult<HomeFeed>;
    pub async fn artist_items(&self, browse_id: &str, params: Option<&str>) -> MlResult<HomeFeed>;
    pub async fn playlist(&self, playlist_id: &str) -> MlResult<PlaylistPage>;
    pub async fn playlist_continuation(&self, token: &str) -> MlResult<PlaylistPage>;
    pub async fn queue(&self, video_id: &str, playlist_id: Option<&str>) -> MlResult<QueuePage>;
    pub async fn queue_continuation(&self, token: &str) -> MlResult<QueuePage>;
    pub async fn charts(&self) -> MlResult<BrowsePage>;
    pub async fn new_releases(&self) -> MlResult<BrowsePage>;
    pub async fn moods_and_genres(&self) -> MlResult<MoodsAndGenres>;
    pub async fn mood_genre_page(&self, params: Option<&str>) -> MlResult<BrowsePage>;
    pub async fn podcasts(&self) -> MlResult<HomeFeed>;
    pub async fn podcast(&self, show_id: &str) -> MlResult<PodcastPage>;
    pub async fn podcast_continuation(&self, token: &str)
        -> MlResult<(Vec<Episode>, Option<String>)>;
    pub async fn browse(&self, browse_id: &str, params: Option<&str>) -> MlResult<BrowsePage>;
    pub async fn browse_continuation(&self, token: &str) -> MlResult<BrowsePage>;

    pub async fn media_info(&self, video_id: &str) -> MlResult<MediaInfo>;
    pub async fn stream_info(&self, video_id: &str, quality: AudioQuality) -> MlResult<StreamInfo>;
    pub fn invalidate_stream(&self, video_id: &str, quality: AudioQuality);
    pub fn clear_stream_cache(&self);
    pub fn cached_stream(&self, video_id: &str, quality: AudioQuality) -> Option<StreamInfo>;
    pub async fn download_audio(&self, video_id: &str, quality: AudioQuality,
        path: impl AsRef<Path>, progress: impl Fn(u64, u64) + Send) -> MlResult<()>;
    pub async fn register_playback(&self, video_id: &str, cpn: &str) -> MlResult<()>;
    pub async fn playback_tracking_url(&self, video_id: &str) -> MlResult<String>;
}

pub trait TokenProvider: Send + Sync { /* po_tokens / invalidate, see pot.rs */ }
```

Implementation modules are private-ish but public: `context`, `error`, `pages`,
`parse`, `player`, `pot`, `stream`. Only the facade above and the core return
types are frozen; parser helpers may change. Stream URLs are cached per
`(video_id, quality)` and expire from `StreamInfo::expires_at`.

### 2.3 `ml-lyrics`

```rust
pub enum LyricsProvider { LrcLib, LyricsPlus, KuGou, BetterLyrics, Paxsenix, Zemer }
impl LyricsProvider { pub fn name(self) -> &'static str; }
pub struct LyricsQuery { pub title: String, pub artist: String,
    pub album: Option<String>, pub duration: Option<u32> }
impl LyricsQuery { pub fn new(title: impl Into<String>, artist: impl Into<String>) -> Self; }

pub struct LyricsClient;
impl LyricsClient {
    pub fn new(providers: Vec<LyricsProvider>) -> Self;
    pub fn providers(&self) -> &[LyricsProvider];
    pub fn http(&self) -> &reqwest::Client;
    pub async fn fetch(&self, query: &LyricsQuery) -> MlResult<LyricsDocument>;
}
pub fn parse_lrc(text: &str) -> MlResult<LyricsDocument>;
pub fn parse_ttml(text: &str) -> MlResult<LyricsDocument>;
```

`fetch` tries the configured providers in order (LRCLIB only by default; `KuGou`
is opt-in), respects `Settings::lyrics`, and populates
`LyricsLine::words/background/agent` when the provider supplies them (TTML).
`LyricsDocument::is_synced` stays `lines.iter().any(|line| line.start_ms > 0)`.

Additive API on the same crate (nothing above changed shape):

```rust
pub enum Script { Japanese, Korean, Chinese, Hindi, Cyrillic(CyrillicLanguage) }
impl LyricsProvider { pub const ALL: [Self; 6]; pub fn from_name(&str) -> Option<Self>;
    pub fn is_available(self) -> bool; pub fn enabled_by_default(self) -> bool;
    pub fn disabled_reason(self) -> Option<&'static str>;
    pub fn availability() -> [(Self, bool, Option<&'static str>); 6]; }
impl LyricsClient { pub fn from_names(names: &[&str]) -> Self; }   // stored order
pub fn detect_script(text: &str, enabled: &[Script]) -> Option<Script>;
pub fn romanize_detected(script: Script, line: &str) -> String;
pub fn romanize_document/romanize_document_enabled(..) -> Option<..>;
pub fn romanized(&LyricsDocument) -> Option<LyricsDocument>;
pub fn apply_offset(&mut LyricsDocument, offset_ms: i64);          // lines + words
pub fn translated(&mut LyricsDocument, translated: Vec<String>);   // pads/truncates
pub struct TranslationClient/TranslationConfig/TranslationEndpoint; // user key
```

Romanisation covers Japanese (`kakasi`, MIT), Korean (Revised Romanization from
upstream's table), Chinese (`pinyin`, MIT, tone-free) and Hindi (upstream's
Devanagari table) plus Macedonian Cyrillic; script detection runs **once per
document**. Other Cyrillic languages still return `MlError::Unimplemented` rather
than half-romanising. Translation needs a user-supplied DeepL or
OpenAI-compatible key and answers `MlError::Unavailable` without one: no key is
compiled in, and no undocumented endpoint is called.

### 2.4 `ml-store`

```rust
pub struct Store; // owns one rusqlite::Connection; !Sync by design
impl Store {
    pub fn open(path: &Path) -> MlResult<Self>;       // WAL, foreign_keys, synchronous=NORMAL
    pub fn open_in_memory() -> MlResult<Self>;
    pub fn schema_version(&self) -> MlResult<u32>;    // user_version
    pub fn connection(&self) -> &Connection;
}
pub const SCHEMA_VERSION: u32 = 1; // schema.rs
```

`schema.rs` defines v1 tables (`song`, `artist`, `album`, `playlist`,
`playlist_song_map`, `song_artist_map`, `song_album_map`, `event`, `play_count`,
`lyrics_cache`, `search_history`, `speed_dial`, `kv`). **Current gap:** `open`
does not yet execute the schema/migrations, so `schema_version()` returns 0 and
the store test asserts that. The store owner must wire the migration before
relying on tables. `rows.rs` holds query row projections (`ArtistRecord`,
`PlaylistRecord`, `HistoryEntry`, `CachedLyrics`, `SpeedDial`, `PlayCount`,
`LibraryFilter`).

### 2.5 `ml-audio`

```rust
pub struct AudioConfig { pub mpv_binary: Option<PathBuf>, pub extra_args: Vec<String> }
pub enum AudioCommand { Load{urls,index}, LoadUrl{url,start_ms}, EnqueueNext{url},
    Play, Pause, TogglePause, Stop, SeekAbsolute(u64), SeekRelative(i64),
    SetVolume(f32), SetMuted(bool), SetSpeed(f32), SetNormalization(bool),
    SetGainDb(f32), SetEqBands(Vec<f32>), SetDevice(String), ListDevices,
    Next, Previous, Shutdown }
pub enum AudioEvent { Snapshot(PlayerSnapshot), Position{position_ms,duration_ms},
    Buffering(bool), EndFile, Error(String), Devices(Vec<AudioDevice>),
    Path(String), AudioParams(AudioParams), Metadata(TrackMetadata) }

pub struct MpvEngine;
impl MpvEngine {
    pub fn spawn(config: AudioConfig) -> MlResult<Self>;
    pub fn send(&self, command: AudioCommand) -> MlResult<()>;
    pub fn try_recv(&self) -> Option<AudioEvent>;
    pub fn event_receiver(&self) -> &crossbeam_channel::Receiver<AudioEvent>;
    pub fn command_sender(&self) -> &crossbeam_channel::Sender<AudioCommand>;
    pub fn list_devices(&self) -> MlResult<Vec<AudioDevice>>;
    pub fn pid(&self) -> u32;
    pub fn socket_path(&self) -> &Path;
    /// Kills/reaps the mpv child and removes its IPC socket; idempotent, also
    /// called from `Drop`. `PR_SET_PDEATHSIG(SIGTERM)` covers SIGKILL.
    pub fn shutdown(&mut self);
}
pub fn list_devices(config: &AudioConfig) -> MlResult<Vec<AudioDevice>>;
/// Removes `metrolist-audio-<pid>-<nanos>.sock` files whose owning pid is dead.
pub fn sweep_stale_sockets() -> usize;
pub const EQ_BANDS_HZ: [f32; 10];
pub fn build_af(eq_bands: &[f32], gain_db: f32) -> Option<String>;

pub trait StreamResolver: Send + Sync { fn resolve(&self, video_id: &str) -> Option<String>; }
pub enum SleepTimer { Off, Minutes(u32), EndOfTrack, FadeOut(Duration) }
pub struct QueueView { songs: Vec<Song>, position: Option<usize>, shuffle: bool,
                       repeat: RepeatMode, source: QueueSource }
pub enum ControllerEvent { Snapshot(PlayerSnapshot), QueueChanged(QueueView),
                           Error(String), EndOfQueue, NeedUrl{video_id,prefetch},
                           DeviceChanged{id,fallback_from} }
pub struct PlaybackController;
impl PlaybackController {
    pub fn new(commands: Sender<AudioCommand>, events: Receiver<AudioEvent>,
               resolver: Arc<dyn StreamResolver>) -> Self;
    pub fn play_queue(&mut self, queue: Queue, start_index: usize);
    pub fn play_pause(&mut self); pub fn next(&mut self); pub fn previous(&mut self);
    pub fn seek(&mut self, position_ms: u64); pub fn set_volume(&mut self, volume: f32);
    pub fn toggle_shuffle(&mut self); pub fn cycle_repeat(&mut self);
    pub fn set_eq(&mut self, bands: &[f32]);
    pub fn set_crossfade(&mut self, duration: Duration);
    pub fn set_crossfade_gapless(&mut self, enabled: bool);
    pub fn set_skip_silence(&mut self, enabled: bool);
    pub fn set_normalization(&mut self, enabled: bool, gain_db: Option<f32>);
    pub fn set_speed(&mut self, speed: f32); pub fn set_device(&mut self, id: Option<&str>);
    pub fn set_sleep_timer(&mut self, timer: SleepTimer);
    pub fn sleep_timer(&self) -> SleepTimer; pub fn stop(&mut self);
    /// Asks both engines to stop; the owning `MpvEngine`s reap the children.
    pub fn shutdown(&mut self);
    pub fn submit_resolved(&mut self, video_id: String, url: String);
    pub fn poll(&mut self) -> Vec<ControllerEvent>;
    pub fn snapshot(&self) -> PlayerSnapshot;
    pub fn queue(&self) -> &Queue; pub fn queue_view(&self) -> QueueView;
}
```

`MpvEngine::spawn` starts `mpv --input-ipc-server` on a unique Unix socket
(`metrolist-audio-<app pid>-<nanos>.sock`) with `PR_SET_PDEATHSIG(SIGTERM)`, so
the child dies even when the app is SIGKILLed; `sweep_stale_sockets` removes
socket files whose owner pid is dead at startup. The controller owns the
queue, preloads the next resolved URL for gapless playback, honours repeat,
and emits `NeedUrl` for missing/expired URLs. `ml-audio/tests/live.rs` is
`#[ignore]`d (requires mpv + audio output); `src/bin/audio-probe.rs` drives a
live engine.

### 2.6 `ml-integrations`

```rust
pub const APP_ID: &str = "org.metrolist.Metrolist";
pub const APP_NAME: &str = "Metrolist Desktop";
pub const MPRIS_BUS_NAME: &str = "org.mpris.MediaPlayer2.metrolist";
pub trait PlayerControl: Send + Sync { /* play/pause/next/previous/seek/volume/shuffle/repeat/state/open_uri/quit */ }
pub struct MprisService;
impl MprisService { pub fn spawn(control: Arc<dyn PlayerControl>) -> MlResult<Self>; pub fn update(&self, &PlayerSnapshot); pub fn is_running(&self) -> bool; }
pub struct TrayHandle;
pub fn spawn_tray(name: &str, icon: Option<&Path>, actions: Sender<TrayAction>) -> MlResult<TrayHandle>;
pub struct InstanceGuard; // acquire(APP_ID) -> Ok(Some(primary)) | Ok(None: argv forwarded)
pub fn acquire_single_instance(app_id: &str) -> MlResult<Option<InstanceGuard>>;
pub struct NowPlayingNotifier; // set_enabled / should_notify / on_snapshot
```

Implemented with `mpris-server 0.11` (zbus 6 + tokio thread), `ksni 0.3` and
`notify-rust 4` (zbus 5 + async-io threads), and `single-instance 0.3` over an
`AF_UNIX` socket. `mapping` is the pure, total core (status/loop/shuffle
mapping, µs↔ms, track ids, metadata projection, property diffs). Discord
presence stays behind the optional `discord` feature.

### 2.7 `ml-ui`

Public library surface (`crates/ui/src/lib.rs`):

```rust
pub enum RunMode { Normal, Smoke { frames: u32 }, Screenshot { path: PathBuf } }
pub enum DemoMode { Player, Queue, Lyrics, Library, Settings, History, Liked, Downloads, Account,
                     Charts, Moods, NewReleases, Podcasts, Stats, Wrapped, Recognize,
                     RecognizeListening, RecognizeSuccess, RecognizeError }
pub struct RunOptions { pub mode: RunMode, pub width: u32, pub height: u32, pub demo: Option<DemoMode> }
pub fn run(options: RunOptions) -> eframe::Result;
pub struct App; // eframe::App; App::new(cc, options, instance_guard)
```

Internal module contract (UI workstream may extend):

- `theme::Theme::{from_settings, install, bg, card}`, `ColorScheme::from_seed`,
  M3 type scale and shape constants, bundled `fonts`.
- `widgets::{chip, filled_button, outlined_button, icon_button, icon_button_at,
  filled_icon_button, section_header, nav_item, slider, song_row, album_card,
  modal, Toasts, shimmer, empty_state}`. `song_row` returns `SongRowResponse`
  with the row click, download-state icon and the badge-derived action.
- `shell::{top_bar, filter_chips, mini_player, bottom_nav, nav_rail, is_expanded}`;
  `is_expanded` is `width >= 840 dp`; below 840 the compact layout (top bar,
  bottom nav, centered column max 560) is used.
- `screens::home::show(...) -> Option<String>`, `search::show(...) -> Option<String>`,
  `player::show(...) -> PlayerAction`, `queue::show_modal(...) -> QueueAction`,
  `downloads::show(...) -> Vec<Intent>`, `account::show(...) -> Vec<AccountAction>`,
  `catalog::{charts,moods,new_releases,podcasts,podcast,mood_results}(...) -> Vec<Intent>`,
  `stats::{show,wrapped}(...) -> Vec<Intent>`,
  `recognize::show(...) -> Vec<Intent>`, `placeholder::coming_soon`.
- `routes::Tab { Home, Search, Together, Library }`, shortcuts Ctrl+1..4.
- `settings_store::SettingsPaths::{from_env,new,settings_file,database_file,
  art_cache_dir,load,save,ensure_settings_file}`.
- `desktop::Desktop::{start,poll,status,set_notifications}` adapts the
  controller to `PlayerControl` and drains MPRIS/tray/instance signals.
- `pure::*` holds the deterministic helpers (`seek_target`, `volume_target`,
  `seek_relative`, `volume_step`, `clamp_eq_bands`, `sleep_seconds`,
  `parse_deep_link`, `first_deep_link`, `tray_command`, `download_badge`,
  `download_icon`/`download_click`, `download_progress_text`, `bytes_text`,
  `storage_percent`, `local_file_url`, `session_file_path`, `cookie_import_format`,
  `account_error_message`, `expiry_hint`, `recognition_*` (progress, timings,
  subtitle/meta, search query/URL, copy text, best search match, error text),
  `played_duration_ms`, …), each with unit tests and no panicking paths.
- `mock::MockData` (deterministic songs/albums/art textures) is no longer used
  by the shipped UI; live pages arrive over the `AppEvent` contract.

Keyboard: Space play/pause, ←/→ seek 5 s, ↑/↓ volume, M mute, S shuffle,
R repeat, Esc closes queue → player → tab, `/` search, Ctrl+1..4 tabs.
Screenshot capture uses `Context::request_screenshot` once demo/artwork/playback
readiness settles; smoke mode prints `SMOKE_OK frames=N backend=…`.

## 3. Data flow

```
UI intent (egui widget)                      backend result
        │                                          ▲
        ▼                                          │
  App state / route  ── commands ──▶  async task ──┘  AppEvent
        │                              (tokio)
        │ StreamResolver cache
        ▼
  PlaybackController ── AudioCommand ──▶ MpvEngine thread ──▶ mpv
        ▲                                   │
        └──── ControllerEvent::* ◀── AudioEvent ◀┘
```

- The UI thread never blocks. It drains `PlaybackController::poll()` and
  `AppEvent`s once per frame and redraws.
- Account flow: `Services` keeps a cloned `InnertubeClient` with the shared
  cookie jar and private `session-cookies.txt`. Import parses Netscape/header/
  JSON, seeds the jar, saves `0600`, runs one `discover_session` +
  `account_info`, and caches metadata in `kv("account")`. Sync calls
  `library_snapshot` on tokio, then `spawn_blocking` upserts into `Store` and
  emits `LibrarySynced` so the Library reloads. A signed-out menu clears the
  jar/cache without retrying.
- Stats/Wrapped call `Store::stats_report(period, now, 10)` on `spawn_blocking`
  and return `UiEvent::StatsLoaded`; `stats_generation` drops responses for a
  period the user already left. A `LibraryDirty` event reloads the open stats
  route. Demo modes use the same screen with a synthetic report.
- Recognition is opt-in: the Search mic opens `Route::Recognize`, which stays
  idle until Start. `Services::start_recognition` opens a 12 s cpal capture on
  `spawn_blocking`, fingerprints it on the same blocking thread, then submits
  the URI to the public Shazam discovery endpoint on tokio. `recognition_generation`
  drops cancelled/replaced runs, `RecognitionProcessing` flips the UI after
  capture, and `Recognized` carries the match plus sample/match timings. Raw
  audio and PCM never leave the blocking closure and are never logged or
  written to disk. Cancel stops capture through an `AtomicBool`. Playing a
  match searches songs and uses `QueueSource::List` (no radio); the other
  actions open a YouTube Music search or copy text.
- Page requests (home/search/album/…) run on the tokio runtime and send
  `AppEvent`s plus UI-local continuations (`UiEvent::HomeMore`, `SearchMore`,
  `AlbumMore`, …) back over one crossbeam channel; the UI drains them once per
  frame. `MockData` is retained but unreferenced by the shipped UI.
- Stream URLs: `PlaybackController::poll` may emit `NeedUrl`; the UI first asks
  `DownloadManager::path_for`. A verified offline file is submitted as a
  percent-encoded `file://` URL; otherwise `InnertubeClient::stream_info`
  resolves on the runtime. Already-known network URLs come from the
  `StreamResolver` trait backed by `cached_stream` plus a local map.
- Downloads: the manager's event stream is forwarded as `UiEvent::Download`;
  the UI refreshes the manifest/storage snapshot and a per-row badge cache,
  then repaints. `DownloadEvent::Completed` also sends a toast.
- Lyrics: `LyricsClient::fetch` result becomes `AppEvent::LyricsLoaded`.
- Store: the UI holds `Arc<Store>` and issues short typed calls directly;
  mutations publish `UiEvent::LibraryDirty` so the visible library refreshes,
  while queue/history persistence is written from the event drain. Startup
  reads `kv("queue")` through `parse_persisted_queue` and installs it with
  `restore_queue` (no autoplay); queue edits persist immediately.
- Radio: `play_radio` installs the first `next` page and retains its
  continuation. `extend_radio` appends deduplicated songs in the background
  once five queue slots remain; `RadioExtended` only repaints.

## 4. Threading model

- **UI thread** — `eframe`/winit; owns `App`, `Services`, theme, textures and
  `SettingsPaths`. Reads/writes `settings.json` on startup.
- **mpv engine thread** — created by `MpvEngine::spawn`; owns the IPC socket
  and mpv child process. A second `ml-audio-ipc` thread reads JSON lines.
  Commands and events use unbounded `crossbeam-channel`s; property updates are
  coalesced into ~10 Hz `AudioEvent::Position`/`Snapshot` values.
- **Async runtime** — tokio multi-thread runtime, 2 worker threads, for
  `ml-innertube` and `ml-lyrics` futures. `InnertubeClient` is `Clone`; its
  caches use `Arc<Mutex<…>>` internally. Do not create a runtime per request.
- **DB access** — `Store` wraps `rusqlite::Connection` in its own mutex and is
  shared as `Arc<Store>`. UI and async tasks issue short serialized calls; no
  separate DB thread/channel exists.
- **Downloads** — `DownloadManager` owns a multi-thread tokio runtime sized by
  `settings.downloads.concurrent_limit` plus a UI forwarding thread that turns
  `DownloadEvent`s into `UiEvent::Download` and requests repaints.
- **Integrations** — MPRIS owns a thread with a current-thread tokio runtime;
  tray and notifications own zbus 5/async-io threads; single-instance runs an
  `AF_UNIX` listener. All expose crossbeam channels. The UI thread calls
  `Desktop::poll` once per frame, publishes each changed snapshot with
  `MprisService::update`, and applies raised signals there. Integration code
  never touches UI state or the controller from D-Bus callbacks; those post
  commands/signals back to the UI thread.
- No shared mutable UI state crosses threads. Crossing values are `Send` core
  types (`Song`, `PlayerSnapshot`, `AppEvent`, `StreamInfo`, …).

## 5. Settings and data paths

`SettingsPaths::from_env()` uses `directories::ProjectDirs` with
organization `""`, application `"metrolist-desktop"`:

| Purpose | Path |
|---|---|
| Settings | `$XDG_CONFIG_HOME/metrolist-desktop/settings.json` (fallback `~/.config/…`) |
| Database | `$XDG_DATA_HOME/metrolist-desktop/library.db` (fallback `~/.local/share/…`) |
| Account session cookies | `$XDG_DATA_HOME/metrolist-desktop/session-cookies.txt`, mode `0600`, parent `0700` |
| Art cache | `$XDG_CACHE_HOME/metrolist-desktop/art/` (fallback `~/.cache/…`) |

`settings.json` is created with defaults on first run and written atomically
(temp file + rename). A missing or corrupt file loads `Settings::default()`.
The download manager defaults to `$XDG_DATA_HOME/metrolist-desktop/downloads`
(fallback `~/.local/share/metrolist-desktop/downloads`);
`Settings::downloads.directory` overrides it. Its soft byte budget is stored in
`kv("downloads.limit_bytes")` (empty means unlimited).

Runtime playback extras are separate from the JSON `Settings`: EQ on/off and
preset, speed, output device, notifications, crossfade (seconds and same-album
gapless), skip silence, normalization plus manual gain, and sleep selection
live in the `kv` table (`player.eq_enabled`, `player.eq_preset`,
`player.speed`, `player.device`, `player.notifications`,
`player.crossfade_seconds`, `player.crossfade_gapless`,
`player.skip_silence`, `player.normalization`,
`player.normalization_gain_db`, `player.sleep_timer`) and are applied at
startup. `auto`/empty device means Automatic. Off/minutes/end-of-track sleep
selections persist; fade-out is immediate and writes Off, so restart cannot
resume a ramp. A second `MpvEngine` is spawned at startup and attached as the
crossfade engine; failure degrades to gapless and is reported in Settings.
Account metadata (`auth.identity`, `account`) is cached in the same `kv` table
without cookie values; the session file is the only cookie store.
Play history accumulates positive player-position deltas only while playing,
ignores seek jumps, and clamps to the track duration (`HistoryPending` in
`state.rs`, then `Services::record_play`). Events written before this fix have
`play_time=0`; they are left untouched and cannot be backfilled.

## 6. Build, run, verify

**Testing:** Live playback tests must be muted (`METROLIST_TEST_MUTE=1` / mpv `mute=yes`); never produce audible output.

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace                 # ignored live audio tests are skipped
cargo run -p ml-ui --bin metrolist
cargo run -p ml-ui --bin metrolist -- --smoke            # prints SMOKE_OK, exits 0
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-home.png 405 900
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-wide.png 1100 760
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-player-live.png 405 900 --demo player
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-queue-live.png 405 900 --demo queue
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-lyrics-live.png 405 900 --demo lyrics
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-downloads-live.png 405 900 --demo downloads
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-stats-405.png 405 900 --demo stats
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-wrapped-405.png 405 900 --demo wrapped
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-recognition-idle-405.png 405 900 --demo recognize
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-recognition-listening-405.png 405 900 --demo recognize-listening
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-recognition-success-405.png 405 900 --demo recognize-success
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-recognition-error-405.png 405 900 --demo recognize-error
cargo test -p ml-ui --test playback_extras   # sleep/EQ/speed/device through the controller
cargo test -p ml-ui -- --ignored --nocapture   # live audio/network/account checks
cargo test -p ml-ui --test live_catalog -- --ignored --nocapture  # catalog counts + muted direct play
cargo test -p ml-ui --test live_stats -- --ignored --nocapture    # stats counts over the real library DB
cargo test -p ml-ui --test history_duration                        # recorded play_time reaches stats
cargo test -p ml-ui --test recognition_fake                         # local fake Shazam + synthetic signature
```

A second `metrolist` launch (optionally with `metrolist://play?v=<id>`) exits 0
after forwarding its argv and URL to the primary instance. Desktop integration
is verified on the host with `busctl`/`dbus-monitor`; `playerctl` is not
installed there.

`--demo player|queue|lyrics|library|settings|history|liked|downloads` starts the app in a
deterministic state. The player demo plays the sample track and waits for the sink;
the downloads demo enqueues the sample track and waits for a completed download
(network required on the first run).

The UI needs a Wayland/X11 session with a working wgpu backend (Vulkan on the
reference host). `--screenshot` captures the framebuffer via
`Context::request_screenshot`, so no compositor screencopy support is needed.
Release packaging (AppImage, glibc floor) is documented in `docs/BUILD.md`,
which uses the `rust:1.99.1-bookworm` container: 1.95 cannot build
`material-colors 0.5` (it requires rustc ≥ 1.97).

## 7. Workstream ownership and status

| Workstream | Files | Status |
|---|---|---|
| InnerTube | `crates/innertube/**` | Live WEB_REMIX home/search/album/artist/playlist/`next`/stream URLs and cookie-session auth (`discover_session`, `account_info`, `library_snapshot`). Streams resolve through VISIONOS (no PO token); no PO-token provider. |
| Lyrics | `crates/lyrics/**` | lrclib ladder plus LyricsPlus/other providers, LRC/TTML parsing, provider toggles and romanisation settings. |
| Store | `crates/store/**` | Schema v1 migrations, typed CRUD, library/liked/history queries, playlist CRUD + reorder, queue/settings KV. |
| Downloads | `crates/downloads/**`, `crates/ui/src/screens/downloads.rs` | Manifest-backed download workers with progress/retry/remove/clear, Library > Downloads UI, download icons on song rows and bulk album/playlist actions. Offline files bypass the network stream resolver. |
| Recognition | `crates/recognition/**`, `crates/ui/src/screens/recognize.rs`, `crates/ui/src/services.rs` | Kotlin-exact Shazam signature, cpal capture, typed discovery client and the explicit UI flow. Public endpoint, no key. Fake local transport/signature tests; no microphone in automation. Confidence is not part of the API response. |
| Audio | `crates/audio/**` | mpv subprocess + JSON IPC behind `PlaybackController`; two engines for crossfade, seek, volume, shuffle, repeat, radio queues, 10-band EQ, skip silence, speed, manual-gain normalization, sleep timer/fade and device fallback verified on the host. Per-track loudness metadata remains unavailable. |
| Integrations | `crates/integrations/**`, `crates/ui/src/desktop.rs` | MPRIS (snapshot updates, seek/volume/shuffle/repeat/raise/quit), tray Show/Hide/Play-Pause/Next/Previous/Quit, now-playing notifications, single-instance argv/deep-link forwarding and optional Discord presence are driven by `App`. |
| UI screens | `crates/ui/src/screens/**`, `app.rs`, `shell/` | Live Home/Search/Album/Artist/Playlist, Charts, Moods & genres, New releases, anonymous podcast shelves/show pages, Library (songs/albums/artists/playlists/downloads/history), Liked, local playlist editing, Account (cookie import, masked profile, library sync/sign-out), Stats and Wrapped (period chips, totals/chart/top lists, story cards), song recognition (mic entry, listening/cancel, processing, match actions), Settings (appearance, playback, desktop, storage, about), player with EQ/speed/device/sleep controls, queue and lyrics screens. Together is a placeholder. |
| Packaging/parity | `assets/`, `packaging/`, `.github/`, `docs/PARITY.md`, `docs/TEST-PARITY.md` | AppImage, icons/i18n, CI container, parity tracking. |

Frozen foundation (change only additively): `crates/core/**`,
`crates/ui/src/{theme,widgets,routes,settings_store,mock}.rs`, `Cargo.toml`,
`rust-toolchain.toml`. New backend deps go in `[workspace.dependencies]`; do not
add `egui-elegance`.

## 8. Known gaps in the current tree

- Account sign-in accepts imported browser cookies (Netscape/header/JSON) and
  syncs the liked library into SQLite; the session file rotates under
  `${data_dir}/session-cookies.txt` (`0600`). There is no OAuth/portal login or
  PO-token provider, and the only live cookie snapshot on the verification host
  was rejected by Google as stale, so live login remains unverified. Remote
  playback history reporting is still not wired; Together is a placeholder.
- Catalog browse is anonymous-first: charts, moods, new releases and podcast
  episode shelves are live. Anonymous `podcasts()` returns episode shelves, not
  a show grid, and `podcast_channels()` needs auth, so the Podcasts landing has
  no show tiles; show pages exist behind `Route::Podcast` but no anonymous
  navigation exposes them. `load_youtube_browse` is wired at the service layer
  without an arbitrary-browse route. All catalog playback uses
  `Intent::PlayCatalog`/`QueueSource::List`, so it never installs a radio queue.
- Stats/Wrapped are read-only views over local play history: no share sheet,
  image export, remote stats or year-in-review persistence. Historical rows
  written with `play_time=0` cannot be backfilled.
- Recognition uses the default microphone only (no device picker), requires a
  network round-trip to the public Shazam endpoint, and cannot show confidence
  because the API response does not include one. No live microphone capture is
  part of automated verification; the local fake-transport test covers the
  signature/response path.
- Downloads have no byte-level resume: a failed/interrupted transfer restarts
  from zero. Changing the download folder or limit applies on the next launch,
  and the active manager keeps the launch quality. Download covers still come
  from the network when available, so a fully offline cold start falls back to
  the scheme placeholder. The in-memory thumbnail count/estimated size can be
  read and cleared.
- Desktop integrations are best-effort: missing D-Bus/tray/notification
  services become startup toasts. Hiding via the tray keeps audio playing;
  Quit stops playback and closes the window. Tray menu actions are not covered
  by CI and `playerctl` was unavailable on the verification host, so MPRIS was
  exercised with `busctl`.
- EQ/speed/device changes are not carried into downloaded files or exports,
  and speed changes apply live only while mpv owns the stream. Loudness
  normalization has no per-track metadata, so Settings exposes a manual
  −15…+3 dB gain instead. Crossfade needs the second mpv engine; if it cannot
  start, Settings says so and playback stays gapless. Non-fading sleep choices
  re-arm after restart; fade-out is session-only.
- Proxy settings only apply on the next launch; passwords are plain JSON until
  the keyring milestone.
- Radio continuations append; album/playlist page continuations do not extend
  an existing queue. Home/search parsing occasionally yields items without
  duration or artwork.
- InnerTube is unauthenticated: streams are served by the VISIONOS client,
  which needs no PO token, but every request is a ≤512 KiB ranged GET because
  unbounded transfers are what googlevideo caps (measured: IOS 403s after
  512 KiB/1 MiB). A token provider seam exists and defaults to no tokens. There are no live
  network tests in CI; `ml-audio` live tests are `#[ignore]`d and need mpv.
- Dynamic desktop accent detection is not implemented; the Settings screen
  labels it unavailable and uses the seed colour.
- The UI screenshot/smoke harness needs a working wgpu backend and a
  Wayland/X11 session; it is not part of `cargo test`.
