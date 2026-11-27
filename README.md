# Metrolist Linux

An **unofficial** native Linux port of
[Metrolist](https://github.com/MetrolistGroup/Metrolist), the GPL-3.0-only
YouTube Music client for Android: a Nothing-OS-styled egui/eframe desktop app with an
InnerTube (WEB_REMIX) client, SQLite library, offline downloads, mpv playback
and MPRIS/tray integration. Not affiliated with, endorsed by or sponsored by
the upstream project or Google.

The upstream Android source (version 13.7.0) is a separate, read-only checkout
and is **not** part of this crate graph.

## Features

- **Playback** — unauthenticated stream resolution over the VISIONOS client
  (no PO token needed) with ranged transfers, mpv JSON-IPC engine, gapless
  prefetch, shuffle/repeat/radio queue (radio button immediately right of
  Repeat, active dot while the queue is radio), an animated progress timeline
  with hover/drag seek preview and one seek on release, creator links in every
  song row (first artist with a channel id) and the player byline, 10-band EQ
  presets, speed, sleep timer, output device selection, Auto/Low/High quality.
- **Library** — liked songs, library membership, songs/albums/artists/
  playlists/downloads/history tabs, sort chips (Recently added/Title/Artist/
  Album/Duration) that order a display copy without rewriting the store,
  playlist and local-playlist display order (Default/Recently added/Title/
  Artist/Duration, also used by Play/Shuffle), Liked Songs Play All, local
  playlist editing (create, rename, delete, add, remove, reorder), local play
  history.
- **Discovery** — live home feed with chips and continuations, search with
  suggestions and filters behind a centred search pill (equal gutters, 341 px
  at 405, capped at 720 px when wide), album, artist and online playlist pages.
- **Catalog & stats** — charts, moods/genres, new releases, podcast browsing,
  song recognition from the search app bar, and Stats/Wrapped over local
  history.
- **Together** — Listen Together rooms: create/join by code, host approval,
  host-driven transport, guest sync that follows the room's current track.
- **Offline** — manifest-backed download queue with progress, retry, cancel,
  storage budget; a verified download plays without the network.
- **Lyrics** — LRCLIB (default) and KuGou providers, LRC/TTML parsing with word
  timings, agents, background lines and offsets; Cyrillic romanisation is
  implemented and tested, though the lyric view still renders provider text
  verbatim.
- **Desktop** — MPRIS, StatusNotifierItem tray, now-playing notifications,
  single instance with `metrolist://play?v=…` forwarding (`--smoke`/
  `--screenshot` skip the guard so batch runs work beside the installed app).
- **Integrations** — optional Last.fm scrobbling (session or username/password
  login, credentials in `${data_dir}/secrets.json`, mode 0600) and Discord Rich
  Presence (feature-gated, off by default). Account sign-in by importing a
  browser cookie file (Account → *Sign in with cookies.txt*), with library
  sync and a rotating `session-cookies.txt` (`0600`).
- **Theme** — one monochrome Nothing palette (`crates/ui/src/theme/scheme.rs`):
  OLED black/off-white, surfaces #111/#1a1a1a (dark) and #fff/#f0f0f0 (light),
  1-px hairline borders, radii 16 for cards and pills for chips, no shadows or
  blur. Space Grotesk body, Doto display ≥ 36 px, Space Mono UPPERCASE labels.
  System/Light/Dark/Pure black, responsive compact (`< 840 dp`) and expanded
  shells.
- **Motion** — one ease-out motion system (`crates/ui/src/motion.rs`, design
  and coverage in `docs/MOTION.md`): duration/easing tokens for route, player,
  dialog, mini player, toast, search, lyrics, skeleton and equalizer animation,
  plus hover/press/focus state layers, a Reduce motion setting and
  `METROLIST_REDUCE_MOTION=1`.

`docs/PARITY.md` is the honest per-feature matrix (Done / Partial / Not ported /
N/A), with the exact evidence for each row and the limits of every partial.

## Screenshots

Captured from the Nothing build at 405×900 (compact shell), signed out, dark and
light. Home is a compositor capture because the live feed's cover art needs
longer to load than the `--screenshot` settle window allows; the other nine are
framebuffer captures (`--screenshot`). Each thumbnail links to the
full-resolution frame.

| | | | |
|---|---|---|---|
| [<img src="docs/ui-home-dark.png" width="240" alt="Home, dark">](docs/ui-home-dark.png) | [<img src="docs/ui-home-light.png" width="240" alt="Home, light">](docs/ui-home-light.png) | [<img src="docs/ui-search-dark.png" width="240" alt="Search, dark">](docs/ui-search-dark.png) | [<img src="docs/ui-search-light.png" width="240" alt="Search, light">](docs/ui-search-light.png) |
| Home, dark | Home, light | Search, dark | Search, light |
| [<img src="docs/ui-library-dark.png" width="240" alt="Library, dark">](docs/ui-library-dark.png) | [<img src="docs/ui-library-light.png" width="240" alt="Library, light">](docs/ui-library-light.png) | [<img src="docs/ui-settings-dark.png" width="240" alt="Settings, dark">](docs/ui-settings-dark.png) | [<img src="docs/ui-settings-light.png" width="240" alt="Settings, light">](docs/ui-settings-light.png) |
| Library, dark | Library, light | Settings, dark | Settings, light |
| [<img src="docs/ui-player-dark.png" width="240" alt="Player, dark">](docs/ui-player-dark.png) | [<img src="docs/ui-player-light.png" width="240" alt="Player, light">](docs/ui-player-light.png) | | |
| Player, dark | Player, light | | |

Frames from the Material 3 releases (up to 0.1.3) remain in `docs/` under their
old names and in Git history; they no longer describe the current UI.

### Desktop icon

The icon master is
[`assets/icons/metrolist-desktop.svg`](assets/icons/metrolist-desktop.svg)
(1024×1024 viewBox) with RGBA PNGs at 16, 32, 48, 64, 128, 256, 512 and
1024 px in the same directory; `packaging/check-icon.sh` validates the SVG and
every size. Preview sheet: [`docs/icon-preview.png`](docs/icon-preview.png).
The `.desktop` entry keeps `Icon=metrolist-desktop`.

## Status

| Layer | State |
|---|---|
| `ml-core` | Models, queue state machine, settings, events, utilities + tests |
| `ml-ui` | Nothing theme + motion system, custom widgets, responsive shell, live Home/Search/Album/Artist/Playlist/Library/Settings/player/queue/lyrics screens plus catalog (charts, moods, new releases, podcasts), stats/Wrapped, recognition, Listen Together and integrations |
| `ml-innertube` | Live unauthenticated WEB_REMIX browse/search/album/artist/playlist/`next` client; VISIONOS player/stream path needs no PO token. Read-only browser-cookie session discovery exists; OAuth/WebView login and the PO-token (BotGuard) route are not implemented. |
| `ml-lyrics` | LRCLIB + KuGou wired; LyricsPlus/BetterLyrics/Paxsenix/Zemer refuse with typed errors; LRC/TTML parsing; Macedonian Cyrillic romanisation (other Cyrillic languages return `MlError::Unimplemented`), Japanese/Korean/Chinese/Hindi romanisation implemented |
| `ml-store` | SQLite schema v1 forward migrations, typed CRUD for library/likes/history/playlists/queue/settings |
| `ml-downloads` | Manifest-backed offline download manager with bounded workers, progress, retry/remove, storage budget |
| `ml-audio` | mpv JSON IPC engine + queue-aware controller; seek, volume, EQ, speed, device and sleep timer verified live |
| `ml-integrations` | MPRIS, tray, now-playing notifications, single-instance forwarding, optional Discord presence (feature-gated, off) |

No shipped path calls `todo!()`/`unimplemented!()`. Lyrics romanisation returns
`MlError::Unimplemented` only for Cyrillic languages other than Macedonian; the
AI translation stub returns `MlError::Unavailable`. Both fail loudly instead of
producing half-converted text. See `docs/ARCHITECTURE.md` for the frozen
interfaces and per-workstream ownership.

## AppImage

`dist/Metrolist-Desktop-0.1.4-x86_64.AppImage` — 120,285,688 bytes (114.7 MiB),
AppImage type 2 (runtime commit `fedfad5`), built 2026-11-27 in
`rust:1.99.1-bookworm` from the tree frozen in `docs/SOURCE-FREEZE.sha256`
(275 files, self-sha256 `78e714e6…`) with the Debian bookworm mpv
bundle (`build/mpv-bundle`). Published as
[v0.1.4](https://github.com/steimerbyte/metrolist-linux/releases/tag/v0.1.4).

```text
sha256  b31088c6b0aed5a42097d1e718878323bccbf55fc449601f0c6bf4c2e51317ee
```

```sh
cd dist && sha256sum -c SHA256SUMS
# Metrolist-Desktop-0.1.4-x86_64.AppImage: OK
```

```sh
chmod +x dist/Metrolist-Desktop-*-x86_64.AppImage
./dist/Metrolist-Desktop-*-x86_64.AppImage
# or install it for the desktop (the layout used for release verification):
mkdir -p ~/AppImages && cp dist/Metrolist-Desktop-0.1.4-x86_64.AppImage \
  ~/AppImages/metrolist_desktop.appimage && chmod 755 ~/AppImages/metrolist_desktop.appimage
```

The 0.1.3 artifact (121,031,160 bytes, sha256 `19307f8a6a8b…`) stays at its tag.

### What changed in 0.1.4

- **Boot sequence** — `ml-boot` registry with a start-up overlay (theme, fonts,
  settings, session, home) instead of silent first frames; every module logs
  `ok` or `degraded`.
- **Home feed** — Explore is the default `HomeView` (page + shelf prefixes);
  Settings chips list the rejected shelves, and a change reloads Home. The
  filter runs on `HomeLoaded`/`HomeMore`; the client keeps the full feed.
- **Streams** — innertube StreamShim serves mpv from loopback in 512 KiB
  chunks and re-mints on 403/410; a rejected URL is dropped so Play resolves
  again. The output mute is reasserted only while an output is open.
- **Queue thumbnails** — a 40 px cover per 56 dp row; missing or failed art
  falls back to the dot dither instead of a blank slot.
- **Player waveform** — 2 dp solid bars at a 3 dp pitch, played `on_surface`
  with 1 px white tips, unplayed `on_surface_variant`, a 2 pt 4x4 Bayer
  checker behind the band; one seek on release, hover time bubble.
- **Sessions** — signed-in requests prefer the jar `visitor_data` (PoToken,
  then config, then jar), read in its own scope so the future stays `Send`.

### What changed in 0.1.3

- **Library sort** — a compact sort chip on songs, albums, artists and
  playlists (Recently added, Title, Artist, Album, Duration); playlists and
  local playlists add Default (stored order) and feed the sorted copy to Play
  and Shuffle. Display-only: the stored order is never mutated, ties keep the
  store order, rows without a timestamp/artist/album/duration sort last, and
  the reorder controls are hidden while a sort is active.
- **Creator links** — the first artist with a channel id in every song row and
  the player byline opens the artist page; a row without one keeps its play
  action and plain text.
- **Player radio** — a radio button immediately right of Repeat starts the
  existing `next` radio queue, with an active dot while the queue source is
  radio.
- **Progress timeline** — frame-rate-independent smoothing, a hover/drag time
  bubble, a buffering pulse and exactly one seek on release.
- **Liked Songs Play All** — the header action plays the current snapshot
  immediately (mini player included) through a new `Intent::PlayAll`, so a
  one-song liked list is never turned into radio; tapping a row in that list
  still starts radio, as before.
- **Search pill** — centred with equal gutters at 405 and 1100 (rendered boxes
  341 px and 720 px), instead of following the search body width.
- **Motion** — `crates/ui/src/motion.rs` adds one Material 3 token/easing set
  used by the route, player, dialog, mini player, toast, search, lyrics,
  skeleton and equalizer animations and by hover/press/focus states; Reduce
  motion is a Settings toggle and `METROLIST_REDUCE_MOTION=1`. Idle-repaint
  tests keep finished animations from requesting frames. Design, coverage and
  the evaluated-but-rejected `egui_animate` 0.6 / `blinc_animation` 0.5.1 are
  documented in `docs/MOTION.md`.
- **Desktop icon** — new SVG master and 16–1024 px PNGs, validated by
  `packaging/check-icon.sh`; the `.desktop` `Icon=metrolist-desktop` name is
  unchanged.
- **Batch modes** — `--smoke` and `--screenshot` skip the single-instance
  guard, so they run beside the installed app instead of being forwarded.

### What changed in 0.1.2

- **Account login works.** `SAPISIDHASH` hashed the fields in the wrong order
  (`ts origin sapisid`), which YouTube answers with a signed-out 200; it now
  hashes `ts sapisid origin`. A quoted `SESSION_INDEX` (`"0"`) is parsed too.
- **Import via file**: Account → *Sign in with cookies.txt* (or
  `~/Downloads/cookies.txt`) accepts a Netscape/header/JSON cookie file,
  expands `~` and trims quotes/whitespace, keeps the session in
  `$XDG_DATA_HOME/metrolist-desktop/session-cookies.txt` with mode `0600`, and
  syncs the library (liked songs, albums, artists, playlists).
- Verified live with a masked account: 14 playlists / 24 songs / 1 album /
  25 artists; the Account screen shows signed in from the saved session.

### What changed in 0.1.1

- **Audio**: playback no longer starts muted because of a stale per-stream
  WirePlumber/session mute; the engine sets its own unmuted stream state. The
  v0.1.0 control stayed muted in the same test, the user "test mute" path still
  mutes on request.
- **Click stutter**: picking the entry that is already playing no longer
  reloads the stream (no gap on the current queue entry).
- **Settings** live in the sidebar (and the compact bottom nav), with a
  dedicated audio page instead of one long scrolling list.
- **Player**: rotating circular vinyl built from the artwork.
- **Build**: rustc capped at 2 concurrent processes (`.cargo/config.toml.example`
  sets `jobs = 2`, `scripts/rustc-single.sh` is a 2-slot semaphore).

AppImage type 2 normally needs FUSE. Without it:

```sh
./dist/Metrolist-Desktop-*-x86_64.AppImage --appimage-extract-and-run
# or
APPIMAGE_EXTRACT_AND_RUN=1 ./dist/Metrolist-Desktop-*-x86_64.AppImage
```

If Wayland/EGL is broken on a driver, force X11:
`env -u WAYLAND_DISPLAY ./dist/Metrolist-Desktop-*-x86_64.AppImage`.

Portability: the app binary needs at most **GLIBC_2.35** and the bundled mpv
0.35.1 at most **GLIBC_2.34**, both within the Debian 12 / Ubuntu 24.04
`GLIBC_2.36` floor. mpv and its non-system libraries are bundled; the host
graphics/audio stack is not. Only `assets/i18n/en.json` is shipped.

The full artifact record (container binary hash, mpv bundle, GLIBC floors,
host-library list) is in `dist/SNAPSHOT.txt`; build, bundling and runtime
dependency details are in `docs/BUILD.md` and `packaging/README.md`.

## Verification

| Check | Result |
|---|---|
| `cargo test --workspace --no-fail-fast --locked` | **744 passed, 0 failed, 31 ignored** |
| Verus proofs (`proofs/run-gates.sh`) | **12 proof sets, 296 functions verified, 0 errors** (frozen `17e030a7…`, gate `09f035e7…`); gate exit 0, `check-drift.sh` 21/21 |
| Independent proof review | **ACCEPT** of the full gate — 12/12 pairs, 296 functions, token-lexer `spec_guard` (`proofs/REVIEW.md`) |
| UI audit (`docs/UI-AUDIT-0.1.3.md`) | **PASS** — no blocker or should-fix left; nits only (16 px icon, sort-menu elevation, 900 px player composition, capture-state placeholders, narrow-row queue title edge) |
| `cargo fmt`/`check`/`clippy`/`build` | pass — `fmt --all -- --check`, `check --workspace --all-targets --locked`, clippy exit 0 with 16 warnings (all `ml-ui`), `build -p ml-ui --bin metrolist --locked` |
| `./proofs/check-drift.sh` | no source drift (21 declared sources match) |
| `metrolist --smoke 120` | `SMOKE_OK frames=120 backend=Vulkan home_sections=2` |
| AppImage smoke (`--smoke 120`) | `SMOKE_OK frames=120 backend=Vulkan home_sections=2` in two clean isolated runs, exit 0 both times, 0 `ERROR` lines, no `home_sections` variance, no attributable leftover mpv/metrolist/AppRun processes or sockets — real runtime, `METROLIST_TEST_MUTE=1`, `--appimage-extract-and-run` |
| Account login (live) | masked account signed in, `session-cookies.txt` mode `0600` with 27 cookies; library sync 14 playlists / 24 songs / 1 album / 25 artists |
| Audio mute fix | seeded WirePlumber stream mute cleared by the engine (v0.1.0 control stayed muted); explicit user "test mute" still mutes |
| Player / Settings | rotating vinyl screenshot (`docs/ui-player-vinyl-405.png`); Settings reachable from sidebar and compact bottom nav (`docs/ui-sidebar-settings-405.png`) |
| Shutdown / leak checks | 0 mpv processes, 0 engine sockets and 0 instance socket after smoke and SIGTERM; SIGKILL leaves 0 children and the next launch sweeps stale sockets |
| Installed copy (`~/AppImages/metrolist_desktop.appimage`, 755) | same 121,031,160 bytes and sha256; `metrolist_desktop.sha256` updated; two clean installed `--appimage-extract-and-run --smoke 120` runs (fresh isolated `XDG_*`, real runtime, `METROLIST_TEST_MUTE=1`) printed the exact `SMOKE_OK` line, both exit 0, 0 attributable leftovers; the pre-existing 0.1.2 instance was untouched |
| `sha256sum -c dist/SHA256SUMS` (from `dist/`) | `Metrolist-Desktop-0.1.3-x86_64.AppImage: OK` |

The 31 ignored tests are the live ones that need mpv, D-Bus/tray/notification
services, the single-instance socket, the network or a real account cookie file.

Evidence and full logs: `docs/RELEASE-CHECK.md` (gate commands, per-package
counts, binary hash), `docs/PROOFS.md` (what each proof covers, the independent
review and its honest limits), `docs/TEST-PARITY.md` (53 upstream Kotlin test
files / 235 tests mapped to Rust requirements), `proofs/logs/`, and
`dist/SNAPSHOT.txt` + `docs/BUILD.md` for the AppImage.

The proof gate proves the **frozen model**, not the binary: each area transcribes
a slice of the source into `original.rs`, and only `candidate.rs` is verified.
`REVIEW.md` remains an independent sign-off record, not a proof — a prover who
edits the gate scripts (including `review-check.sh` and the record) can make any
candidate pass and recompute both hashes, so the record only asserts that these
exact gate bytes and freeze manifest were present when it was signed. Each
`SOURCE.md` is a hand transcription: an internally consistent wrong
transcription would verify, because the drift hashes pin the revision, not the
semantic fidelity. The real controls are CI on the pinned toolchain and a fresh
reviewer after any gate change.

## Known gaps

Not a 100 % port. `docs/PARITY.md` is the per-feature matrix; the limits that
matter for a release are:

- **Account**: cookie-file import only (Netscape/header/JSON, or the newest
  LibreWolf/Firefox profile). It needs a *fresh* browser session — an expired
  cookie snapshot is rejected instead of half-loading a library. Login and
  library sync are verified live as of 0.1.2 (masked account, 14 playlists /
  24 songs / 1 album / 25 artists); there is no OAuth/WebView login and no
  PO-token provider, and remote history reporting stays unproven.
- **Integrations**: Last.fm scrobbling and Discord Rich Presence are covered by
  unit and fake-socket tests only; neither was run against a live account or a
  running Discord client.
- **Listen Together**: room core (create/join, host approval, host-driven
  transport, guest sync) works; guest queue suggestions and explicit host
  transfer are not ported.
- **i18n**: 55 translated overlays ship in `assets/i18n/` but no code reads
  them yet and only `en.json` is bundled in the AppImage — the UI is English.
- **Downloads** have no byte-level resume (an interrupted transfer restarts at
  byte 0) and offline cover art falls back to the scheme placeholder on a cold
  start.
- **Library sort** is display-only: the chips order a copy for rendering and
  for Play/Shuffle, the stored order is never rewritten, and the reorder
  controls are hidden while a sort is active. **Local library search** still
  has no UI box; the store query (songs/albums/artists/playlists in one page)
  is implemented and tested.
- Not ported: backup/import/export, updater/changelog, hide-explicit /
  hide-video-song filters, dynamic desktop accent colour, Automix, lyrics
  cache wiring, Android Auto/widgets/Cast/alarms (MPRIS + tray replace them).
- Partial by design: EQ surface, loudness (manual gain only), podcast
  subscriptions, generic YouTube browse.

## Toolchain

Pinned in `rust-toolchain.toml`: **Rust 1.99.1 stable**.

Why not 1.95.0: the workspace used to depend on `material-colors 0.5.0`
(`rust-version = 1.97.0`), so Cargo refused 1.95. That dependency is gone with
the Nothing redesign; the pin stays at **1.99.1** because it is the toolchain
every gate is run with. The AppImage container is
therefore `rust:1.99.1-bookworm` (glibc 2.36 floor); a host `stable` build is
1.99.1 and is what is tested here.

## Build, run, verify

```sh
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo run -p ml-ui --bin metrolist
```

For a single-core local build copy `.cargo/config.toml.example` to
`.cargo/config.toml` (gitignored). The repo ships no `rustc-wrapper`: that path
must be absolute and therefore host-specific (`docs/BUILD.md`). CI sets
`CARGO_BUILD_JOBS=1`.

The UI renders through wgpu/Vulkan (observed on the probe host:
`backend=Vulkan adapter="Intel(R) Iris(R) Xe Graphics (RPL-U)"`). A Wayland or
X11 session with a working GPU driver is required; this is not a headless app.

### CI / capture modes

```sh
# Open a window, render N frames (default 120), exit 0
cargo run -p ml-ui --bin metrolist -- --smoke
# -> SMOKE_OK frames=120 backend=Vulkan home_sections=2

# Framebuffer screenshot (window size defaults to 405x900)
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-home.png 405 900
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-wide.png 1100 760

# Deterministic demo state for a screen
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-player.png \
    405 900 --demo player

# Hover/click evidence: pointer is injected through eframe raw input (screenshot
# mode only, dev aid); syntax: --pointer X Y [--click]
cargo run -p ml-ui --bin metrolist -- --screenshot docs/ui-sort-open.png \
    405 900 --demo library --pointer 100 419 --click
```

`--demo` accepts `player|queue|lyrics|library|settings|history|liked|downloads`.
`--screenshot` requests an eframe framebuffer capture after fonts/textures
settle, writes a PNG and exits 0. Capture uses `Context::request_screenshot`,
not the compositor: GNOME Shell does not implement `wlr-screencopy`/grim on
this host.

Keyboard: `Space` play/pause, `Esc` back/close, `Ctrl+1..4` tabs, `/` search,
`←`/`→` seek 5 s, `↑`/`↓` volume, `M` mute, `S` shuffle, `R` repeat.

## Layout

```text
crates/boot          ml-boot      boot registry (`crates/*/module.meta`) + start-up timeline
crates/core          ml-core      models, queue, settings, events, utils
crates/innertube     ml-innertube WEB_REMIX client, parsers, stream resolution, POT seam
crates/lyrics        ml-lyrics    provider ladder + LRC/TTML/romanisation
crates/store         ml-store     SQLite schema v1 + typed CRUD
crates/downloads     ml-downloads manifest-backed offline download manager
crates/audio         ml-audio     mpv JSON IPC engine + playback controller
crates/integrations  ml-integrations MPRIS/tray/notifications/single-instance/Discord
crates/ui            ml-ui        eframe app + `metrolist` binary
crates/ui/assets/fonts            bundled Space Grotesk + Doto + Space Mono + Material Symbols
assets/i18n                       en.json + 55 locale overlays (not yet wired)
docs/PARITY.md                    feature parity matrix
docs/TEST-PARITY.md               upstream test → Rust requirement map
docs/ARCHITECTURE.md              ownership, frozen APIs, threads, data flow
docs/PROOFS.md, proofs/           Verus proofs, freeze manifest, review record
docs/BUILD.md, packaging/         AppImage, glibc floor, mpv bundle
```

## UI architecture summary

- **Theme** (`ui/src/theme/`): one monochrome Nothing palette (dark #000/#111/
  #1a1a1a, light #f5f5f5/#fff/#f0f0f0; success #4a9e5c, warning #d4a843, red
  #D71921 as the single interrupt), 1-px hairlines, radii 16/pill, no shadows.
  Space Grotesk 400/500, Doto ≥ 36 px and Space Mono UPPERCASE are compiled into
  the binary from `crates/ui/assets/fonts/` (swap those files to rebrand;
  notices in `NOTICE`).
- **Shell** (`ui/src/shell/`): compact `< 840 dp` = top bar + chips + content +
  mini player + bottom nav; expanded `>= 840 dp` = 88 dp nav rail + centered
  960 dp column + pinned mini player.
- **Screens** (`ui/src/screens/`): live Home/Search/Album/Artist/Playlist,
  Library (songs/albums/artists/playlists/downloads/history), Liked, local
  playlist editing, Settings, and the player/queue/lyrics views. Together is a
  placeholder.
- **State**: `ml_core::Queue` is the queue state machine;
  `ml_core::PlayerSnapshot` and `AppEvent` are the backend->UI channel types.
- **Position** is reported by the mpv engine over the audio event channel; the
  UI never advances it on its own.

## Settings and data paths

Resolved with `directories` 6 under the application name `metrolist-desktop`:

- config: `$XDG_CONFIG_HOME/metrolist-desktop/settings.json`
- data: `$XDG_DATA_HOME/metrolist-desktop/library.db`
- cache: `$XDG_CACHE_HOME/metrolist-desktop/art/`

`settings.json` is created with defaults on first run and rewritten whenever
settings change. Runtime playback extras (EQ preset, speed, output device,
notifications toggle) live in the SQLite `kv` table. `seed_color` and the
`dynamic_accent` flag are retained for settings compatibility only: Nothing is
monochrome, so the UI no longer derives colours from them.

## Dependency policy

eframe/egui 0.37, reqwest 0.13 (rustls, no OpenSSL),
tokio 1.54, rusqlite 0.41 (`bundled`), serde, thiserror. No `egui-elegance` at
runtime. Unused workspace dependencies stay declared so later agents need no
manifest churn; only inherited dependencies are compiled.

## License and attribution

GPL-3.0-only, no "or later" option. This is an unofficial port of Metrolist and
a derivative work of the upstream GPL-3.0-only Android source; it is not
affiliated with, endorsed by or sponsored by the Metrolist project, Google LLC
or YouTube. Upstream project, bundled fonts (Space Grotesk, Doto and Space Mono
under OFL-1.1, Material Symbols Apache-2.0), translations and the bundled
mpv/FFmpeg/libass/libplacebo licences
are recorded in `NOTICE`; the full GPL text is in `LICENSE`. "Metrolist",
"YouTube", "YouTube Music", "Google" and "Android" are trademarks of their
respective owners.
