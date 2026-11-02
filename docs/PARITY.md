# Feature parity: Android → Linux

Status snapshot: 2026-11-02. This is a matrix of what the Rust port actually
does, read out of the implementation and its tests — not a roadmap. If a row
says **Done**, the code path exists and is exercised by the named test; if it
says **Partial**, the note states the limit.

Android source inspected read-only in a separate checkout (version 13.7.0, 384
Kotlin files under `app/src/main/kotlin`). Paths in the *Android source* column
are relative to `app/src/main/kotlin/com/metrolist/music/`, except `innertube/`
which is `innertube/src/main/kotlin/com/metrolist/innertube/`.

Status vocabulary:

- **Done** — shipped and covered by a listed test or live check.
- **Partial** — shipped with a named limit.
- **Not ported** — absent; nothing pretends otherwise in the UI.
- **N/A** — Android platform surface with no honest desktop equivalent; the
  Linux replacement is named.

## Playback and audio

| Feature | Android source | Linux status | Evidence | Notes |
|---|---|---|---|---|
| Stream resolution, format pick | `playback/MusicService.kt`, `playback/StreamUrlCache.kt`, `innertube/InnerTube.kt` | **Done** | `crates/innertube/src/{lib,player,stream,pot}.rs`; `crates/innertube/src/player.rs` tests; `crates/innertube/tests/parser_regression.rs` | Unauthenticated. `player` goes through VISIONOS (no PO token) with IOS as fallback; prefers itag 251 then 140, `Low` prefers 139; auto-dubbed formats are skipped. |
| Ranged transfers | `innertube/` DASH/segment fetch | **Done** | `crates/innertube/src/stream.rs` (`chunk_ranges`); Verus `proofs/stream` (19 functions) | Every request is a ≤512 KiB ranged GET that must answer 206. No DASH manifest or segment assembly. |
| Play/pause, seek, volume, next/prev | `playback/MusicService.kt` | **Done** | `crates/audio/src/{controller,engine,ipc}.rs`; `crates/audio/tests/live.rs` (ignored live) | mpv subprocess over JSON IPC; position is reported by mpv, never advanced by the UI. |
| Gapless playback / prefetch | `playback/MusicService.kt` | **Done** | `crates/audio/src/engine.rs` (`--gapless-audio`, `--prefetch-playlist`), `controller.rs::prefetch_next`; `controller.rs` tests | End-of-file advances the queue pointer without reloading. |
| Stream quality selection | `AudioQualityKey`, `FormatEntity` | **Partial** | `crates/core/src/settings.rs`, `crates/innertube/src/player.rs::select_format` | Three levels (Auto/Low/High). No per-itag or per-codec picker. |
| URL cache and expiry | `playback/StreamUrlCache.kt` | **Done** | `crates/innertube/src/stream.rs`; Verus `proofs/media` (`is_expired`) | In-memory per `(video_id, quality)` with a 300 s safety margin. Not persisted to disk. |
| Playback error report | `ui/player/PlaybackError.kt` | **Partial** | `crates/ui/src/app.rs` (toast + `playback_error`), `crates/ui/src/screens/player.rs` | Error text is shown in-app; there is no copyable diagnostics dialog. |
| Background playback | `MusicService` foreground service | **Partial** | `crates/ui/src/desktop.rs`; `crates/ui/src/screens/settings.rs` | Hiding the window from the tray keeps audio playing; closing/quitting stops playback. No separate daemon. |
| Bluetooth resume, audio focus, offload, keep-screen-on | `MusicService`, prefs | **N/A** | — | Desktop audio stack (PipeWire/PulseAudio) owns routing and focus. |

## Queue

| Feature | Android source | Linux status | Evidence | Notes |
|---|---|---|---|---|
| Shuffle, repeat off/all/one, add-next, append, remove, reorder, clear | `playback/queues/*` | **Done** | `crates/core/src/queue.rs`; Verus `proofs/queue` (35 functions); `queue.rs` tests | State machine is pure and total; shuffle is a permutation that keeps the current track first. |
| Radio / autoplay continuation | `playback/queues/YouTubeQueue.kt`, `YouTubeAlbumRadio.kt` | **Done** | `crates/ui/src/services.rs` (`play_radio`, `extend_radio`), `crates/innertube/src/lib.rs::queue_append`, `crates/audio/src/controller.rs::append_queue`; radio/queue tests | Starting a radio from a song row (or a search tap) installs the 50-track `next` queue, fetches one continuation immediately, and appends further pages when five tracks remain. Appends deduplicate video ids and keep the current index/position; non-radio playback clears the radio. |
| Persistent queue across launches | `MusicService` queue restore | **Done** | `crates/ui/src/services.rs` (`kv("queue")` read at startup, `controller.restore_queue`), `crates/ui/src/pure.rs::parse_persisted_queue`; `crates/ui/tests/queue_restore.rs` | Restores songs, index, source, shuffle and repeat without autoplay (first play resolves the current track). Corrupt JSON, duplicate ids or an invalid position start empty. |
| Queue view | `ui/player/Queue.kt` | **Done** | `crates/ui/src/screens/queue.rs`; `crates/ui/tests/queue_restore.rs` | Modal list with jump-to-track, now-playing/track badges, download state, move up/down, remove and Load more; each mutation is persisted through the controller. |
| Automix | `playback/MusicService.kt` | **Not ported** | — | No beat-matched transition. |

## Downloads and offline

| Feature | Android source | Linux status | Evidence | Notes |
|---|---|---|---|---|
| Download queue, concurrency, retry/cancel/remove, clear failed | `playback/DownloadUtil.kt`, `playback/ExoDownloadService.kt` | **Done** | `crates/downloads/src/manager.rs`; `crates/downloads/tests/manager.rs`; `src/manifest.rs` tests | Manifest-backed, JSON-persisted, deduplicated, bounded worker pool. |
| Offline playback | `ExoDownloadService`, cache playlist | **Done** | `crates/ui/src/services.rs::resolve` (`download_path` first), `crates/downloads/src/manager.rs::path_for` — `Completed` + size re-checked before use (`manager.rs` test `path_for_detects_a_truncated_file`); real download in `crates/downloads/tests/live.rs` (env-gated `ML_DOWNLOAD_LIVE=1`) | A verified file on disk short-circuits the network resolver; if the file is missing or resized the network path is used. |
| Byte-level resume | media3 `DownloadService` | **Not ported** | `crates/downloads/src/manager.rs` module docs | An interrupted transfer restarts at byte 0; one file per track, never a partially trusted file. |
| Storage budget and usage | storage prefs | **Done** | `crates/downloads/src/manager.rs` (`max_storage_bytes`, soft budget), `crates/ui/src/screens/downloads.rs`, `screens/settings.rs`, `services.rs::set_download_limit` | Limit is persisted in `kv("downloads.limit_bytes")` and enforced at enqueue and before each transfer. A changed directory applies on the next launch. |
| Downloaded album/playlist bulk action | `ui/menu/*` | **Done** | `crates/ui/src/screens/mod.rs::Intent::DownloadAll` | Bulk enqueue from album, artist and playlist headers. |
| Offline cover art | — | **Partial** | `crates/downloads/src/manager.rs` | Covers still come from the network cache, so a fully offline cold start falls back to the scheme placeholder. |

## Library, playlists, history

| Feature | Android source | Linux status | Evidence | Notes |
|---|---|---|---|---|
| Liked songs | `db/`, `ui/screens/library/*` | **Done** | `crates/store/src/library.rs`; `crates/store/tests/store.rs`; `crates/ui/src/screens/liked.rs` | Toggle from the player; `Liked` route and Library tab. |
| Library membership ("add to library") | `db/entities/*` | **Done** | `crates/store/src/song.rs`, `crates/ui/src/app.rs::ToggleLibrary` | Separate from the like flag, as upstream. |
| Library tabs: songs/albums/artists/playlists/downloads/history | `ui/screens/library/*` | **Done** | `crates/ui/src/screens/library.rs`, `state.rs::LibTab` | Albums/artists are derived from songs in the library. A header row above the tabs opens Your stats. |
| Library sort | `LibrarySortKind` prefs | **Partial** | `crates/core/src/settings.rs::LibrarySort`, `crates/ui/src/app.rs` (`load_library(self.settings.library_sort)`) | Four modes (recently added, alphabetical, most played, artist) are applied to every library load, but no screen exposes a control, so only the default is reachable. |
| Local library search/filter | `ui/screens/search/LocalSearchScreen.kt` | **Partial** | `crates/store/src/{library,song,album,artist,playlist}.rs` (`search_library`); `crates/store/tests/parity.rs` | Store side complete: songs/albums/artists/playlists in one `LibraryPage`, Unicode case folding (`ml_fold`, so `MÜLLER` finds `müller`), literal `%`/`_`, deterministic order. No UI search box yet. |
| Local playlists: create, rename, delete, add/remove, move, play, download | `ui/screens/playlist/LocalPlaylistScreen.kt`, `ui/menu/PlaylistMenu.kt` | **Done** | `crates/store/src/playlist.rs`; `crates/ui/src/screens/local_playlist.rs`; `crates/ui/tests/playlist_crud.rs` | Move up/down and remove per row; positions stay dense (`store.rs` test). |
| Playlist bookmarks | `db/entities/PlaylistEntity.kt` | **Partial** | `crates/store/src/library.rs` (`set_bookmark`, `bookmark`, `bookmarked_ids`, `bookmarks_page`); `playlist.rs` | Store side covers albums, artists and playlists uniformly; no UI. |
| Auto / Top / Cache / Uploaded playlists | `AutoPlaylistScreen.kt`, `TopPlaylistScreen.kt`, `CachePlaylistScreen.kt` | **Partial** | `crates/store/src/library.rs` (`AutoPlaylist`, `auto_playlist`, `recently_played`) | Liked / Recently played / Most played / Library are queries, not materialised playlists (upstream's weekly/monthly sync needs no scheduler this way). Downloaded is not offered: the manifest lives in `ml-downloads`, outside this crate. No UI. |
| History (local) | `ui/screens/HistoryScreen.kt`, `db/entities/Event.kt` | **Partial** | `crates/store/src/history.rs`; `crates/ui/src/screens/history.rs`; `crates/ui/src/state.rs::HistoryPending`; `crates/ui/tests/history_duration.rs` | Plays are recorded on track change/end with accumulated playback duration (pause-aware, seek-jump filtered, clamped to track length), grouped by period bucket. Historical rows written before this fix carry `play_time=0` and cannot be backfilled. No clear/export action. |
| History (remote YouTube reporting) | `playback/YouTubeHistoryReporter.kt` | **Not ported** | `crates/innertube/src/lib.rs::register_playback` exists | The client method exists; nothing calls it because there is no account. |
| Stats (top songs/artists, periods) | `ui/screens/StatsScreen.kt` | **Done** | `crates/ui/src/screens/stats.rs`; `crates/ui/src/services.rs::load_stats`; `crates/ui/tests/seed_stats.rs`, `crates/ui/tests/live_stats.rs`; `crates/store/src/stats.rs` | Week/Month/Year/All chips, summary cards (plays, listening time, unique songs/artists/albums, streak, first/last play), 12-month bar chart with an accessible summary, and top songs/artists/albums with artwork and play counts. Loads through `spawn_blocking` with a generation guard; empty/loading/error/retry states. Entry from the Library header. |
| Wrapped | `ui/screens/wrapped/*` | **Done** | `crates/ui/src/screens/stats.rs::wrapped`; `crates/ui/src/pure.rs::wrapped_summary` | Story cards for listening time, plays, longest streak, unique artists and top song/artist/album, plus a copyable text summary. No share sheet or image export. |
| Backup/restore, CSV/M3U import-export, YouTube playlist import | `viewmodels/BackupRestoreViewModel.kt`, `utils/PlaylistExporter.kt`, `ui/menu/ImportPlaylistDialog.kt` | **Partial** | `crates/store/src/transfer.rs`; `crates/store/tests/parity.rs` | M3U (extended) and CSV (`Title,Artist,Album,YouTube Video ID`, as `PlaylistExporter.kt`) export/import per playlist; JSON backup (schema v1) of local playlists, likes, library membership, album/artist metadata and bookmarks, restored in one transaction tolerating missing songs. **Android's ZIP/Room backup is deliberately not readable or written**, and YouTube-playlist import (needs the account/cookie) is absent. No UI. |

## Discovery

| Feature | Android source | Linux status | Evidence | Notes |
|---|---|---|---|---|
| Home feed | `ui/screens/HomeScreen.kt`, `viewmodels/HomeViewModel.kt` | **Done** | `crates/innertube/src/pages/home.rs`, `crates/ui/src/screens/home.rs` | `FEmusic_home` shelves, skeleton/error/empty states, continuations. |
| Home chips | `HomeScreen.kt` | **Partial** | `crates/ui/src/screens/home.rs::CHIPS` | 13 curated labels run a search; server-supplied dynamic chips are not rendered. |
| Online search: query, filters, continuations | `ui/screens/search/OnlineSearchResult.kt` | **Done** | `crates/innertube/src/pages/search.rs`; `crates/ui/src/screens/search.rs` | Filters: All/Songs/Albums/Artists/Playlists/Videos; Enter submits the literal text; pasted YouTube links route to the player/page. No Podcasts filter. |
| Search suggestions | `SearchSuggestionPage.kt`, `OnlineSearchSuggestionViewModel.kt` | **Done** | `crates/innertube/src/lib.rs::suggestions`, `crates/innertube/src/pages/search.rs::suggestions` | Debounced popover with the first 6 suggestions. |
| Search history | `SearchScreen.kt` | **Partial** | `crates/store/src/misc.rs` (`add_search_history`, `search_history`) | Recorded and shown (10 entries); no clear action. |
| Album pages | `ui/screens/AlbumScreen.kt` | **Done** | `crates/innertube/src/pages/album.rs`; `crates/ui/src/screens/detail.rs` | Header, play/shuffle/download, tracks, continuations, uploaded-release shape. |
| Artist pages | `ui/screens/artist/ArtistScreen.kt` | **Done** | `crates/innertube/src/pages/artist.rs`, `detail.rs` | Top songs, carousel sections, "see all" browse, description, subscriber count, continuations. |
| Online playlist pages | `ui/screens/playlist/OnlinePlaylistScreen.kt` | **Done** | `crates/innertube/src/pages/playlist.rs`, `detail.rs` | Lazy continuation loading. |
| Podcasts: browse, subscribe, episodes | `ui/screens/podcast/OnlinePodcastScreen.kt` | **Partial** | `crates/ui/src/screens/catalog.rs`; `crates/innertube/src/pages/podcast.rs`; `crates/ui/tests/live_catalog.rs` | Anonymous episode shelves, show page (header/description/episodes), episode play/download and continuation. No subscriptions, no show grid: anonymous `podcasts()` exposes only episode shelves, and `podcast_channels()` needs auth. |
| Charts | `ui/screens/ChartsScreen.kt` | **Done** | `crates/ui/src/screens/catalog.rs`; `crates/innertube/src/pages/browse.rs` | Live charts via `charts()`, sections with counts, artist/playlist cards, direct catalog playback, browse continuation when returned. |
| Moods and genres | `ui/screens/MoodAndGenresScreen.kt` | **Done** | `crates/ui/src/screens/catalog.rs`; `crates/innertube/src/pages/moods.rs` | Grouped category chips and mood-result browse pages (playlists/albums). |
| New releases | `ui/screens/NewReleaseScreen.kt` | **Done** | `crates/ui/src/screens/catalog.rs`; `crates/innertube/src/pages/browse.rs` | Top-level `new_releases()` shelves rendered as cards; album cards open detail, video cards play directly (no radio). |
| YouTube browse | `ui/screens/YouTubeBrowseScreen.kt`, `BrowseScreen.kt` | **Partial** | `crates/ui/src/services.rs::load_youtube_browse`; `crates/innertube/src/pages/browse.rs` | The generic browse loader exists and is tested through the catalog screens, but there is no arbitrary-browse route/UI or URL entry. |
| Account login, library sync | `ui/screens/LoginScreen.kt`, `viewmodels/AccountViewModel.kt`, `utils/potoken/*` | **Partial** | `crates/ui/src/screens/account.rs`, `crates/ui/src/services.rs`; `crates/innertube/src/auth.rs` (`discover_session`, `account_info`, `library_snapshot`); `crates/store/src/browser.rs` (`newest_firefox_profile`, `read_firefox_cookies`) | Cookie import from a Netscape/header/JSON file or the newest LibreWolf/Firefox profile, a private `session-cookies.txt` (`0600`) with automatic rotation, Account screen (avatar, masked contact, expiry, refresh, sync, sign out), and `library_snapshot` upserts for liked songs/albums/artists/playlists. Errors carry paths/status only. No OAuth/portal login and no PO-token provider; the only live session available on the verification host was rejected by Google as stale, so live login is not claimed. |

## Lyrics

| Feature | Android source | Linux status | Evidence | Notes |
|---|---|---|---|---|
| LRC parsing (agents, background, offset, credits) | `lyrics/LyricsUtils.kt` | **Done** | `crates/lyrics/src/lrc.rs`; `crates/lyrics/tests/lrc.rs` | Offset clamps at zero; `{agent}`/`{bg}` markers and `[bg:]` handled. |
| TTML parsing (word timings, background, dedup, offset) | `betterlyrics/TTMLParser.kt` | **Done** | `crates/lyrics/src/ttml.rs`; `crates/lyrics/tests/ttml.rs`; Verus `proofs/util` | Sequential background de-duplication matches upstream; `x-roman`/`x-translation` are skipped. |
| Provider ladder | `lyrics/LyricsProviderRegistry.kt` | **Partial** | `crates/lyrics/src/{lib,providers}.rs`; `crates/lyrics/tests/providers.rs` | LRCLIB (default, scored matching) and KuGou are wired, with fixture tests for both. Order comes from stored ids (`LyricsClient::from_names`, upstream `deserializeProviderOrder`): unknown ids ignored, duplicates collapsed, empty list falls back to LRCLIB. `LyricsProvider::availability()` reports usable/disabled + reason for Settings. BetterLyrics returns `AuthRequired`; Paxsenix, LyricsPlus and Zemer refuse with typed `Unavailable`. |
| Synced display, active-line highlight, offset | `ui/component/Lyrics*`, `LyricsLine.kt` | **Done (no fullscreen/share)** | `crates/ui/src/screens/lyrics.rs`, `app.rs` | Scrollable line list with the current line (or latest passed line) in the accent colour and strong weight. `−0.5 s`/`+0.5 s`/Reset controls shift the display; offsets persist per video id in `lyrics.offset.<id>` and restore on the next load. Original-line highlight stays authoritative when a translation is shown. Fullscreen and share-image output are not ported. |
| Romanization | `lyrics/` + `ui/screens/settings/RomanizationSettings.kt` | **Done (script coverage gap)** | `crates/lyrics/src/{romanize,korean,hindi}.rs`; `crates/lyrics/tests/romanize.rs`; `crates/ui/src/screens/{settings,lyrics}.rs` | Japanese (`kakasi` 0.1, MIT), Korean (Revised Romanization, upstream's table), Chinese (`pinyin` 0.11, MIT, tone-free) and Hindi (upstream's own Devanagari table) plus Macedonian Cyrillic. Detection is once per document over `ALL_SCRIPTS`; `romanized()` keeps timings/words/agents. The Settings toggle applies live to the lyrics screen and the screen exposes the same toggle. Cyrillic languages other than Macedonian still answer `Unimplemented`. |
| Translation | `lyrics/LyricsTranslationHelper.kt`, `api/OpenRouter*` | **Partial (no streaming/prompt modes)** | `crates/lyrics/src/translate.rs`; `crates/lyrics/tests/translate.rs`; `crates/ui/src/{screens/settings,screens/lyrics,services}.rs` | Settings select Off/DeepL/OpenAI-compatible, an OpenAI endpoint, target language (default `en`) and a session-only API key. The lyrics Translate action sends the whole line list once, caches the result per video id in memory, drops stale generations and renders translations under the original lines. Missing key or invalid endpoint produces a typed explanation without an HTTP request; keys and lyrics are never logged or persisted. Streaming and the Romanized/Transcribed prompt modes remain unported. |
| Offset, translated/romanized document fields | `SongEntity.lyricsOffset`, `LyricsLine.kt` | **Done** | `crates/lyrics/src/lib.rs::{apply_offset,translated,romanized}`; `crates/lyrics/tests/romanize.rs`; `crates/ui/src/{screens/lyrics,app}.rs` | Offset shifts lines *and* words, clamps at zero and is capped at ±3 s in the UI. `translated` pads/truncates to the line count so the timeline stays aligned; the UI shows a translated line under each original while highlight stays on the original. Per-video offsets live in `lyrics.offset.<video_id>` and survive restart. |
| Lyrics cache | `db/entities/LyricsEntity.kt` | **Not ported** | `crates/store/src/misc.rs` (`cached_lyrics`/`put_lyrics`), `schema.rs` (`lyrics_cache`) | Table and DAO exist but no caller reads or writes it; wiring it belongs to the lyrics crate, outside `ml-store`. |

## Audio settings and DSP

| Feature | Android source | Linux status | Evidence | Notes |
|---|---|---|---|---|
| Equalizer | `eq/EqualizerService.kt`, `ui/screens/equalizer/EqScreen.kt` | **Partial** | `crates/audio/src/eq.rs`; `crates/ui/src/screens/settings.rs`; `crates/ui/src/state.rs::EqPreset` | 10-band lavfi chain, on/off, 5 presets, ±12 dB clamp. No per-band sliders, no parametric/APO import, no AutoEQ. |
| Crossfade | `playback/MusicService.kt`, `PreferenceKeys.kt` | **Done** | `crates/audio/src/controller.rs` (`attach_crossfade_engine`, `set_crossfade`, `set_crossfade_gapless`); `crates/ui/src/screens/settings.rs`; live `crossfade_overlaps_two_mpv_engines` | 0–30 s slider (0 = off), same-album gapless switch, second mpv engine attached at startup. Live 5 s run: 2/2 muted sink inputs during the fade, then queue advanced. |
| Sleep timer | `playback/SleepTimer.kt` | **Done** | `crates/audio/src/controller.rs::SleepTimer`; `crates/ui/src/screens/settings.rs`; `crates/ui/tests/playback_extras.rs` | Off/15/30/60 min/End of track/Fade out 30 s. Non-fading selections persist in `kv("player.sleep_timer")` and re-arm at launch; fade-out is immediate, session-only and writes Off so a restart cannot resume a ramp. |
| Loudness normalization | `playback/audio/VolumeNormalizationAudioProcessor.kt` | **Partial** | `crates/audio/src/engine.rs` (`SetNormalization`, `SetGainDb`); `crates/ui/src/screens/settings.rs` | Switch plus manual gain slider (−15…+3 dB, 0.5 dB steps) wired to `set_normalization(enabled, gain)`. No per-track loudness metadata is fetched; the UI says so. |
| Speed | `MusicService` `PlaybackParameters` | **Done** | `crates/audio/src/engine.rs` (`speed`, clamped 0.25–4.0); `crates/ui/src/screens/settings.rs` | 0.5–2.0× slider in 0.05× steps, percent label, pitch preserved by mpv; routed through the controller to both engines. |
| Skip silence | `playback/audio/SilenceDetectorAudioProcessor.kt` | **Done** | `crates/audio/src/engine.rs` (`SetSkipSilence`, `silenceremove`); `crates/ui/src/screens/settings.rs` | Switch uses `silenceremove=start_periods=1:start_duration=0.5:start_threshold=-50dB:stop_periods=-1:stop_duration=0.5:stop_threshold=-50dB`; instant skip only. |
| Output device | `MusicService` device prefs | **Done** | `crates/audio/src/engine.rs::list_devices` (`mpv --audio-device=help`), `screens/settings.rs`, `ControllerEvent::DeviceChanged` | Choice persists in `kv("player.device")`; Automatic is `auto`. A requested device that disappears falls back to auto, persists `auto`, and toasts the fallback. |

## Appearance, settings, i18n

| Feature | Android source | Linux status | Evidence | Notes |
|---|---|---|---|---|
| Theme modes and seed colour | `ui/theme/*`, `ui/screens/settings/ThemeScreen.kt` | **Partial** | `crates/ui/src/theme/*`, `screens/settings.rs`, `crates/core/src/settings.rs::ThemeMode` | System/Light/Dark/Pure black, 6 seed presets plus an sRGB picker; M3 roles from `material-colors` SPEC_2025/TonalSpot. No density, player/mini-player style or slider style options. |
| Dynamic colour | `ThemeScreen.kt` (Android 12+) | **Not ported** | `crates/core/src/settings.rs::dynamic_accent` | Flag and toggle exist; the portal reader is not implemented, the UI says so, and the seed colour is used. No album-art accent. |
| Settings surface | `ui/screens/settings/*` | **Partial** | `crates/ui/src/screens/settings.rs` | Appearance, playback (quality/timer/EQ/device/speed), content (language/country), network (proxy), lyrics (providers/romanize/translation/offset), integrations (Last.fm credentials/queue, Discord IPC), desktop (notifications, MPRIS/tray status), storage, about. Android-only groups and AI/backup pages are absent. |
| Content language/country | `ContentSettings.kt` | **Done** | `screens/settings.rs`, `crates/ui/src/services.rs` | Applied to the InnerTube context; changes take effect on the next launch. |
| Proxy | `PrivacySettings.kt` | **Done** | `crates/core/src/settings.rs::ProxyConfig`, `screens/settings.rs` | Applied on the next launch; the password is stored in plain JSON until a keyring milestone. |
| Hide explicit / hide video songs | `ContentSettings.kt` | **Not ported** | `crates/core/src/settings.rs` | Fields exist; nothing reads them. |
| i18n | `app/src/main/res/values-*/` (55 translated locales) | **Not ported** | `assets/i18n/` — 55 locale overlays + `en.json`, generated by `packaging/tools/extract-i18n.py`; counts in `assets/i18n/INDEX.md` | `en.json`: 1110 keys, 15 of them `<plurals>` groups. Overlays hold translated keys only and fall back to English; 22 of 55 are ≥90 % covered, mean 63.7 %. **No code reads them** — the UI is English and only `en.json` is bundled in the AppImage. |
| Updater, changelog | `utils/Updater.kt`, `ui/screens/settings/UpdaterSettings.kt` | **Not ported** | — | — |

## Desktop integration and platform

| Feature | Android source | Linux status | Evidence | Notes |
|---|---|---|---|---|
| MPRIS | media3 session | **Done** | `crates/integrations/src/{mpris,mapping}.rs`; `crates/integrations/tests/mpris_live.rs` (ignored live); Verus `proofs/integrations` (13 functions) | `org.mpris.MediaPlayer2.metrolist`: play/pause/stop/next/prev, seek, set position, volume, shuffle, loop, raise, quit, metadata. Verified with `busctl` (`playerctl` was not installed). |
| Media keys | media3 session | **Partial** | `crates/integrations/src/mpris.rs` | Keys work through MPRIS; the app registers no global shortcuts of its own. |
| Tray | — | **Done** | `crates/integrations/src/tray.rs`, `crates/integrations/tests/tray_live.rs` (ignored live) | StatusNotifierItem with Show/Hide, Play/Pause, Next, Previous, Quit. Tray unavailable → startup toast, app keeps running. |
| Single instance, deep links | `MainActivity` intent filters | **Done** | `crates/integrations/src/instance.rs`, `crates/ui/src/{desktop,pure}.rs`; `instance.rs` tests | `$XDG_RUNTIME_DIR/<app_id>.sock`; a second launch forwards argv/`metrolist://play?v=…` and exits 0. `.desktop` declares `x-scheme-handler/metrolist`. |
| Now-playing notifications | media3 notification | **Done** | `crates/integrations/src/notify.rs`, `tests/notify_live.rs` (ignored live) | One notification per track change with art; seeks/repeats/volume never re-notify. |
| Discord Rich Presence | `discord/*` | **Done** | `crates/integrations/src/discord.rs`; `crates/ui/src/integrations_worker.rs`; `crates/ui/tests/discord_presence.rs`; feature `discord` enabled in `crates/ui/Cargo.toml` | Settings toggle and optional app ID (blank uses `DISCORD_CLIENT_ID`). A dedicated thread handshakes over the local IPC socket, sends the current track/position and clears on pause/stop; position updates are throttled to 15 s. No token/OAuth. Missing socket shows “Discord not running — retrying” and retries with the crate's backoff; a fake Unix-socket test covers handshake/set/clear. No live Discord client/socket existed on this host during verification. |
| Last.fm scrobbling | `com/metrolist/lastfm/*`, `utils/ScrobbleManager.kt` | **Done (live account unverified)** | `crates/integrations/src/lastfm.rs`; `crates/ui/src/integrations*.rs`; `crates/ui/src/services.rs`; `crates/ui/tests/{lastfm_scrobble,integrations_secrets}.rs` | Settings supports enable, API key, shared secret, session key or username/password (`auth.getMobileSession` only when all four are present), password masked/session-only, Connect/Test, Sign out, Forget credentials, connected user, sent/pending counts. Credentials live in `${data_dir}/secrets.json` 0600 under a 0700 parent; secrets never enter PlayerExtras/kv/logs and errors are redacted. A dedicated worker runs `playback_transition`/hooks, now-playing, threshold/end scrobbles, dedupes `<song>@<started_at>`, persists `${data_dir}/lastfm-queue.jsonl`, flushes on startup/periodically, and disables scrobbling with a toast on session invalid. Live auth/scrobble not attempted without user env credentials. |
| Song recognition | `recognition/*`, `com/metrolist/shazamkit/*` | **Done** | `crates/recognition/**`; `crates/ui/src/screens/recognize.rs`; `crates/ui/src/services.rs::start_recognition`; `crates/ui/tests/recognition_fake.rs` | Search app-bar mic opens a dedicated screen. Explicit Start opens a 12 s cpal capture on the blocking pool, computes the Kotlin-exact Shazam signature, and posts it to the public discovery endpoint; no key or endpoint setting is needed. Listening progress/cancel, processing spinner/cancel, match card with artwork/title/artist/album/genre/date/label and sample/match timings, Play (YouTube Music song search, non-radio), Open in YouTube Music, Copy, and retryable empty/no-match/network errors. Generation guards drop results after cancel/repeat. Raw audio is never logged or written; Shazam does not return a confidence score, so none is shown. |
| Quick Settings recognition tile | `quicksettings/MusicRecognizerTileService.kt` | **N/A** | `crates/ui/src/screens/recognize.rs` | No QS on desktop; recognition is reachable from the Search app bar. |
| Listen Together | `listentogether/*`, `ui/screens/ListenTogetherScreen.kt` | **Done (sync core)** | `crates/together/**`; `crates/ui/src/screens/together.rs`; `crates/ui/src/services.rs`; `crates/ui/src/state.rs` | Lobby with `SERVERS`/`DEFAULT_SERVER_URL`, display name, create/join code, host approval; room code copy, server/role badge, users with crown/you, join/leave notices, reconnect status, leave; room queue with added-by; host play/pause/previous/next/seek/sync now; guest transport locked; automatic host queue/playback publish and guest queue install with `position_at`/`drift_decision`; guest local player follows only the room's current track so it cannot auto-advance into a stale queue; background `TogetherClient` with generation guards; no token persistence. Gaps: guest queue suggestions/add, explicit host transfer; no live two-client room. |
| Home-screen widgets | `widget/*` | **N/A** | — | Linux equivalent: tray menu + MPRIS (any desktop media widget reads MPRIS). |
| Android Auto / Automotive | `playback/MediaLibrarySessionCallback.kt` | **N/A** | — | Linux equivalent: MPRIS; no Automotive protocol. |
| Google Cast | `gms/.../cast/*` | **N/A** | — | Not in a GPL build; no DLNA/Snapcast replacement either. |
| Music alarm | `playback/alarm/*` | **N/A** | — | No Android alarm plumbing; the OS timer/scheduler is the user's tool. |
| Foreground service, exact alarms, battery prompts, screenshot blocking, Bluetooth resume | manifest, `MusicService`, prefs | **N/A** | — | Desktop process + tray replace all of it. |

## Summary

- **Done**: streaming and playback, gapless queue, downloads/offline (minus
  byte-resume), library/likes/playlists, local history, LRC/TTML parsing,
  LRCLIB/KuGou lyrics, synced lyrics display/romanization/translation/offset,
  home/search/album/artist/playlist pages, crossfade,
  skip silence, speed, sleep timer/fade, output device with auto fallback,
  MPRIS/tray/single-instance/notifications, charts/moods/new releases,
  anonymous podcast episode browsing, stats/Wrapped, song recognition,
  Last.fm scrobbling (live account unverified), Discord Rich Presence, and
  Listen Together rooms with host/guest sync.
- **Partial**: account login/library sync (cookie import only; live session not verified),
  lyrics cache, translation streaming/prompt modes, EQ surface, loudness
  normalization (manual gain only), theme options, library sort/search,
  podcast subscriptions/show grid, arbitrary YouTube browse, remote history.
- **Not ported**: Listen Together guest queue suggestions and explicit host transfer,
  backup/import/export, runtime i18n, updater.
- **N/A**: widgets, Android Auto, QS tile, Cast, alarms, Android service
  plumbing.
