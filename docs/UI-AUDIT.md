# Metrolist Linux UI audit (2026-11-02)

Fresh capture and visual review of the native egui/M3 port in this repository.
Source was not modified.
Reference: Android fastlane screenshots (1080x2400, 3x -> 360 dp) and the
upstream Compose sources under `metrolist/app/src/main/kotlin/com/metrolist/music/ui/`
(the upstream Android tree is a separate, read-only checkout).
Captures taken while signed in (`docs/audit/live-*`, `docs/ui-*-live*.png`) were
used for measurement but are not published; the evidence column lists the
signed-out captures in `docs/audit/`.

## Severity summary

| Severity | Count |
|---|---|
| P0 | 0 |
| P1 | 6 |
| P2 | 11 |

P1 items are systematic layout defects visible on every screen or on the
primary artist/album/player surfaces. Nothing blocks interaction or loses data.

## Findings

| ID | Screen | Severity | Issue | File / line | Expected fix | Screenshot |
|---|---|---|---|---|---|---|
| UI-01 | All compact + wide shell + player | P1 | `center_column` never centers: it allocates the fixed column with `Align::Min`. Compact 405: content is 373 px at x=0 with a 32 px right gutter; wide 1100: content is 960 px at x=88 (rail edge) with a 52 px right gutter. Player compact: art/title center at x=178 vs window center 202.5 (24.5 px left). | `crates/ui/src/app.rs:1857-1877`; widths `:1532`, `:1581`; player `:1657` | Add the horizontal offset inside `center_column` (`add_space((available-width)/2)` or `Layout::top_down(Align::Center)` around a fixed-width UI) once, for every call site. | `docs/audit/compact-home.png`, `docs/audit/wide-home.png`, `docs/audit/wide-settings.png`, `docs/audit/compact-player.png` |
| UI-02 | Player (wide) | P1 | Full player is a 560 px column anchored at x=0: art 333 px at x=116-448 (center 282 vs 550), transport ends x=419, top-bar controls end x=545. The right half of a 1100 px window (540 px) is empty; on 1080 the same layout leaves ~500 px unused. | `crates/ui/src/app.rs:1657` (`.min(560.0)`), `:1857`; `crates/ui/src/screens/player.rs:186-190` | Center the 560 column in the window and scale the art/transport for expanded width (or a two-column wide player: art left, controls right). | `docs/audit/wide-player.png` |
| UI-03 | Album / Playlist top bar (long titles) | P1 | App-bar title is painted with `layout_no_wrap` and no clip width. "Random Access Memories" ink runs x=49-310; the history icon occupies x=254-267 and the sync icon x=305-310, so the glyphs are drawn through the title. "Every Daft Punk Song" overlaps the history icon. | `crates/ui/src/shell/mod.rs:51-62` (title), `:83-93` (icons) | Elide to `rect.width() - title_x - 96.0` and reserve the action cluster; show the full title in a tooltip. | `docs/audit/wide-playlist.png` |
| UI-04 | Player / mini player artwork | P1 | 333x333 art frame shows a 333x251 image with 41 px black bars top and bottom (12.3% each). `thumb(480)` resolves to ytimg `hqdefault.jpg`, which is a 480x360 4:3 frame with 45 px baked bars; the square UV crop removes side pixels but keeps the bars. Mini player uses the same 4:3 tier (`thumb(96)` -> `default.jpg`). | `crates/ui/src/screens/player.rs:186-190`; `crates/ui/src/app.rs:1284-1292`; `crates/core/src/util.rs:33-38`; `crates/ui/src/widgets/mod.rs:65-87` | Use a 16:9 asset at >=480 (`maxresdefault`/`hq720`) and crop to square, or crop the known 12.5% top/bottom bands out of 4:3 ytimg assets before the square UV crop. | `docs/audit/compact-player.png`, `docs/audit/wide-player.png`, `docs/audit/compact-lyrics.png` |
| UI-05 | Album / Artist / Playlist | P1 | Full Wiki description is rendered unbounded, before the track list, with no heading or expand control. Artist screen: the 900 px viewport ends inside the bio; "Top songs" is below the fold. Upstream clamps to 3 lines with `ExpandableText` and an "About artist" heading. | `crates/ui/src/screens/detail.rs:68-77` (album), `:104-113` (playlist), `:164-173` (artist) | Clamp to 3 lines (or a 96 px block) with Expand, place after the first song section, and add the section heading. | `docs/audit/wide-playlist.png` |
| UI-06 | Liked / detail header action row | P1 | "Play" is 40 px high (`filled_button`), "Shuffle" 32 px (`outlined_button`) on the same row; the row is left-aligned at x=4 under centered artwork/title. Download icon is a 40 px square. | `crates/ui/src/widgets/controls.rs:124` vs `:153`; `crates/ui/src/screens/detail.rs:309-316`; liked equivalent | Make both 40 px high with matching radius, center the row inside the column. | `docs/audit/compact-liked.png`, `docs/audit/wide-playlist.png` |
| UI-07 | All screens | P2 | No single inset system: top-bar title layout x=4 (ink 5), section headings +4 (`x=4`), settings headings x=0, song covers x=0, chip rows x=0/8. Android reference uses 16 dp (49 px at 3x) for title, chips and covers. | `crates/ui/src/shell/mod.rs:51`; `crates/ui/src/widgets/controls.rs:190-193`; `crates/ui/src/widgets/rows.rs:45-47`; `crates/ui/src/screens/settings.rs:572-580` | After UI-01, standardize to one 16 dp gutter and delete the ad-hoc +4 offsets. | `docs/audit/compact-home.png`, `docs/audit/compact-settings.png` |
| UI-08 | Search suggestions | P2 | Suggestion list is six borderless, transparent, center-aligned buttons (36 px) between the field and the chips, with no popover surface, no leading icon and no hover fill; it also stays visible after Enter while "No results" is shown. | `crates/ui/src/screens/search.rs:74-93` (suggestions), `:97-108` (filters) | Draw a `surface_container_high` rounded dropdown with 48 px left-aligned rows, icon and divider; close it on submit. | — |
| UI-09 | Queue modal | P2 | Rows are 56 px with only a 12 px index/bars at x=20 and no cover thumbnail; title width is hard-capped at 190 px. Upstream uses `MediaMetadataListItem` (cover + title + artist). | `crates/ui/src/screens/queue.rs:106-155` (row), `:36-40` (360 px modal) | Add a 40 px cover (or 48) and let the title use the available width minus actions. | `docs/audit/compact-queue.png`, `docs/audit/wide-queue.png` |
| UI-10 | All screens with mini player | P2 | Toast is anchored `CENTER_BOTTOM, -100`: on 900 px it occupies y=754-790 while the mini player starts at y=764, overlapping 26 px and covering its text. Duplicate error toasts stack over the mini player; on the player/lyrics they sit inside the transport/content area. | `crates/ui/src/widgets/feedback.rs:53-58`; mini player y from `crates/ui/src/shell/mod.rs:154-157` | Anchor above the measured bottom panel height (mini + nav) and collapse duplicate messages. | `docs/audit/compact-lyrics.png`, `docs/audit/wide-lyrics.png` |
| UI-11 | Lyrics | P2 | Plain left-aligned list at x=0, 15 px regular with 2 px line gaps, no current-line highlight, no centering and no artwork/backdrop treatment used by the Android lyrics screen. | `crates/ui/src/screens/lyrics.rs:90-105` | Use a readable 400-560 px centered column (or 32 px gutter), 18-20 px lines with 8-10 px gaps, and highlight the active line when timed lyrics exist. | `docs/audit/compact-lyrics.png`, `docs/audit/wide-lyrics.png` |
| UI-12 | History / Liked / Downloads / detail lists (wide) | P2 | Rows stretch to the full 960 px content width: title at x=140, download/dots at x=1010-1035 (~870 px gap); one row spans nearly the whole window. Detail header uses only the left ~500 px of 960. | `crates/ui/src/app.rs:1532`; `crates/ui/src/screens/history.rs:96-104`; `crates/ui/src/screens/mod.rs:383-410` | Cap list rows at 640-720 px (or use two columns) and keep the trailing actions near the text. | `docs/audit/wide-library.png`, `docs/audit/wide-history.png`, `docs/audit/wide-downloads.png` |
| UI-13 | Settings | P2 | Chips are 32 px and seed swatches 34 px (M3 interactive minimum 48); `10-band equaliser`/other label-plus-chip rows break the label-above-control rhythm; "Dynamic colour" says it follows the desktop accent and the next line says detection is unavailable (duplicate/contradictory copy); raw mpv device descriptions are truncated into chips. | `crates/ui/src/widgets/controls.rs:88-89`; `crates/ui/src/screens/settings.rs:106` (swatches), `:134-146` (dynamic), `:209-214` (inline chips), `:248-262` (devices) | 48 px hit area around chips/swatches; use switch/list rows with supporting text; remove or disable the unsupported dynamic accent until implemented; device rows in a list with ellipsis + tooltip. | `docs/audit/compact-settings.png`, `docs/audit/wide-settings.png` |
| UI-14 | Library / Search filters | P2 | `horizontal_wrapped` breaks six chips into two ragged rows ("Videos" alone on row two; Library uses Songs/Albums/Artists/Playlists then Downloads/History) instead of the upstream horizontally scrolling row. | `crates/ui/src/screens/search.rs:97-108`; `crates/ui/src/screens/library.rs:37-46` | Use one horizontally scrolling row with hidden scrollbar and 16 px leading inset. | `docs/audit/compact-library.png` |
| UI-15 | Wide shell | P2 | Content touches the rail divider at x=88 with no gutter; title ink x=93, settings heading x=88, song rows start x=88, while the right gutter is 52 px. | `crates/ui/src/app.rs:1532`, `:1543`; `crates/ui/src/shell/mod.rs:301-303` | Include the 16-24 px gutter before centering the 960 column (same fix as UI-01). | `docs/audit/wide-settings.png`, `docs/audit/wide-playlist.png` |
| UI-16 | Home (scrolling) | P2 | Hover scrollbar renders as a 9 px wide, near-white (`#F8E0E0`) bar at x=396-404, flush against the window edge and separated from the content by the 32 px dead gutter. It is the only vertical element there and reads as a stray line. | `crates/ui/src/app.rs:1590`, `:1635`; scrollbar left unthemed in `crates/ui/src/theme/mod.rs:116-153` | Style a 4 px rounded `outline_variant` scrollbar, inset inside the content column, or use `ScrollBarVisibility::VisibleWhenNeeded` + hover. | `docs/audit/compact-home.png` |
| UI-17 | Top bar avatar | P2 | Avatar is two fixed RGB gray circles (`#3A4A5A`, `#7A8A9A`) regardless of theme/account, with no image; at 28 px it reads as a placeholder. | `crates/ui/src/shell/mod.rs:71-79` | Use `on_surface_variant`/`surface_container_high` roles and an initial or silhouette, or hide until an account is connected. | `docs/audit/compact-home.png` |

## Top 10 fixes (priority order)

1. **UI-01 - center the content column.** Compact 405: 373 px column at x=0,
   32 px empty right; wide 1100: 960 px at x=88, 52 px empty right; compact
   player center x=178 vs window 202.5. One change in
   `crates/ui/src/app.rs:1857-1877` fixes home, search, library, settings,
   detail, queue-adjacent chrome and the compact player.
2. **UI-02 - wide player uses the whole window.** On 1100x760 the player
   column is 560 px at x=0, art 333 px at x=116-448, controls end x=419, top
   bar ends x=545: 540 px of the window is unused. Center the column and grow
   art/transport (or use a two-column wide player).
3. **UI-03 - elide the app-bar title.** "Random Access Memories" ink runs to
   x=310; history icon x=254-267 and sync icon x=305-310 are painted through
   it. Same for "Every Daft Punk Song" (history icon). Reserve the action
   cluster and ellipsize at `width - title_x - 96`.
4. **UI-04 - crop player art to square without black bars.** 333x333 frame
   contains a 333x251 image with 41 px bars top/bottom. `thumb(480)` returns
   ytimg `hqdefault` (480x360, 45 px baked bars); `thumb(96)` returns
   `default.jpg` (120x90, ~11 px bars). The square UV crop preserves them.
5. **UI-05 - clamp detail descriptions and move songs up.** The artist page
   at 405x900 ends inside the Wiki bio; "Top songs" is below the fold.
   Upstream uses `ExpandableText(collapsedMaxLines=3)` with an "About artist"
   heading. Put the first song section before the bio block.
6. **UI-06 - align header action heights.** "Play" 40 px vs "Shuffle" 32 px
   on the same row; both start at x=4 under centered art/title. Make both
   40 px and center the row.
7. **UI-07 - one 16 dp inset.** Title layout x=4 (ink 5), section headings
   x=4, settings headings x=0, covers x=0; Android uses 49 px / 3 = 16 dp.
   Remove the ad-hoc +4 painter offsets after UI-01.
8. **UI-12 - cap wide list rows.** Rows are 960 px: text starts x=140,
   download/dots at x=1010-1035, ~870 px apart. Cap at 640-720 px or use two
   columns; keep trailing actions near the text.
9. **UI-08 - give search suggestions a surface.** Six 36 px borderless
   transparent center-aligned rows with no icon/hover, still visible over
   "No results" after Enter. Use a `surface_container_high` dropdown with
   48 px left-aligned rows and close it on submit.
10. **UI-10 - stop the toast covering the mini player.** On 900 px the toast
    is y=754-790 and the mini player starts y=764: 26 px overlap plus stacked
    duplicates. Anchor above the measured mini + nav height.

## Keep as-is

- Bottom navigation: 72 dp items with the 56x32 selected pill, 24 dp icons and
  12 px labels matches the Android look; the 4-tab set and order are correct.
- Home filter chips: 32 dp height, 8 dp shape, 8 px gaps and horizontal scroll
  (`filter_chips`) are the closest match to the reference.
- Song rows: 56 dp square cover, 15 px medium title, 13 px variant subtitle,
  64 dp row height, 40 px icon hit areas, truncation via `elided`.
- Album/playlist card grid: square art, 12 px gaps, 3 columns at 405, play
  badge with 48 px hit rect; titles/subtitles truncate cleanly.
- Mini player: 64 dp bar, 44 dp art, 64 dp filled play control, 40 px skip
  targets; its visual construction is right (only its position is affected by
  UI-01 and the toast by UI-10).
- Queue modal: 360 px wide, centered, 32 px radius, scrim `#000` 180 alpha;
  active row uses `surface_container_high`, 56 px rows, readable 14/12 px text.
- Search empty and no-result states: centered icon, 22 px title, 13 px body,
  copy is specific and not placeholder-like.
- Color roles and contrast: sampled scheme (primary `#F8B6B7`, background
  `#000`, surface container family from SPEC_2025) showed no contrast failure
  in the captures;
  selected chips use `secondary_container`/`on_secondary_container` correctly.
- Typography sizes themselves (22 title, 16/14 body, 12 label) follow the M3
  scale; weights are consistent (medium titles, regular body).

## Missing demo routes / coverage limits

- `--demo` has no route for Search, Home feed variants, Album, Artist,
  Playlist or Together. Search/album/artist/playlist captures were obtained
  by running the release binary interactively under X11 and driving it with
  `xdotool`; no source hook was added.
- Search/radio correctness was not evaluated (owned by a parallel agent).
  `live-compact-search-results2.png` is a visual capture only.
- Loading skeletons and network-error states were not captured: screenshot
  mode waits for idle (45 settled frames), and there is no flag to force an
  error. The player error toast occurred naturally (mpv) and is visible in the
  player captures.
- Only six Android reference screenshots exist (home, artist, recognize,
  together, player, lyrics); Settings, Library, Queue, Downloads and History
  were compared against M3 and the upstream Compose sources only.
- The wide interactive capture is 1080x760, not 1100x760: the WM clamped the
  requested width to the display. All other wide captures are 1100x760 from
  `--screenshot`.
- Android screenshots are 1080x2400 at density 3, so dp comparisons use
  `px/3`; desktop egui uses 1 logical px here (no HiDPI scaling).

## Capture verification

Screenshot mode (release binary `target/release/metrolist`, built 14:05;
no `.rs` file newer than the binary at capture time). No cargo build or
rebuild was run for this audit; all captures used the existing binary, and
the debug binary was left untouched:

```
cd metrolist-linux   # repo root
XDG_RUNTIME_DIR=/tmp/ml-audit-rt LD_PRELOAD=/tmp/ml-audit-shim.so \
  ./target/release/metrolist --screenshot docs/audit/<name>.png 405 900 [--demo <route>]
```

Each run logged `screenshot written: docs/audit/<name>.png (405x900)` or
`(1100x760)`. The installed AppImage held the abstract single-instance lock,
so the shim appends `.audit` to that one abstract socket name and a private
`XDG_RUNTIME_DIR` was used; it changes no app behavior and no source file was
touched. Window size came from `--screenshot WIDTH HEIGHT` and matches every
file's dimensions.

Live navigation (X11 backend, same shim/private runtime):

```
env -u WAYLAND_DISPLAY WINIT_UNIX_BACKEND=x11 DISPLAY=:0 \
  XDG_RUNTIME_DIR=/tmp/ml-audit-rt LD_PRELOAD=/tmp/ml-audit-shim.so \
  setsid ./target/release/metrolist &
xdotool windowsize 33554436 405 900     # later 1100 760 (WM gave 1080x760)
xdotool mousemove --window 33554436 <x> <y> click 1
xdotool type --delay 55 'daft punk'; xdotool key Return
import -window 33554436 docs/audit/<name>.png
```

Observed: 30 PNGs, 20 at 405x900 and 10 wide (nine 1100x760, one 1080x760),
total 4.0 MiB.

| Batch | Files | Timestamps (local) | Size range |
|---|---|---|---|
| Compact, screenshot mode | 9 | 14:41:55-14:42:24 | 40,670-257,909 B |
| Wide, screenshot mode | 9 | 14:42:35-14:43:03 | 50,309-410,368 B |
| Live compact navigation | 11 | 14:48:27-14:53:30 | 28,835-213,220 B |
| Live wide playlist | 1 | 14:51:11 | 177,164 B |

The live batches are private review evidence and are not published (see the
note at the top); the published set is the compact/wide signed-out captures.

Every file is a real window capture; no empty-state image was fabricated. The
first search attempt was overwritten by the successful capture
(`live-compact-search-empty.png`, 55,361 B, 14:48:49), and the failed
click-through `live-compact-artist.png` from the album page was replaced by
the live artist capture (166,331 B, 14:50:36).
