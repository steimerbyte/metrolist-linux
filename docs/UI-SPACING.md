# UI spacing & density

Spacing-only pass over the Material 3 egui shell. The original pass made no
behaviour, navigation, colour or type changes; the Settings entry noted under
Shell is a later navigation addition.

## Method

- Captures: `metrolist --screenshot <png> <w> <h> --demo <screen>` at
  **405×900** (compact) and **1100×760** (wide). `home` has no demo route, so it
  is the live feed; its rhythm is stable but its content differs between runs.
- The capture build is a copy of `target/debug/metrolist` with the app-id string
  patched to the same length, so the single-instance lock stays with the user's
  session. All captures are therefore toast-free, matching `docs/audit/`.
- Measurement is pixel-scanline based on the PNGs, not on the code constants:
  page margin = leftmost non-background pixel of body rows; column right =
  rightmost top-bar avatar ink + its 8 dp inset; vertical gaps = runs of
  background-coloured scanlines between ink bands inside the content column.
- Before: `docs/spacing-before/`, after: `docs/spacing-after/`.
- Album/artist/playlist pages are not reachable from `--screenshot` (no
  deep-link navigation for them), so their deltas below are declared-layout
  values, not pixels.

## Scale (`crates/ui/src/theme/spacing.rs`)

| Token | dp | Used for |
|---|---|---|
| `DP4` | 4 | title↔subtitle, icon↔label inside a control |
| `DP8` | 8 | chips, buttons, list rows, action rows |
| `DP12` | 12 | art↔text, card grid gap (`GRID_GAP`) |
| `DP16` | 16 | page/panel inset, section header↔content |
| `DP20` | 20 | nav-rail inset |
| `DP24` | 24 | section gap, page bottom |
| `DP32` | 32 | button body height, reserve |
| `DP48` | 48 | section-header band, empty-state top |
| `TOUCH` | 40 | minimum hit target (`style.spacing.interact_size`) |

## Content column (main delta)

`center_column` in `app.rs` allocated the column inside a top-down `Ui`, so the
`add_space` spacer added *height*, not a left margin: content started at x = 0
and the right side kept a dead strip. The column is now placed by a horizontal
row with a `(available - width) / 2` spacer (and the column height is captured
before that row, so the player art keeps its size).

| capture | before left / right | after left / right | delta |
|---|---|---|---|
| 405 compact | 0 / 32 | 16 / 16 | +16 / −16 |
| 1100 wide | 88 (rail edge) / 52 | 114 / 26 | +26 / −26 |

Rail (88 dp) and the intended 560 / 960 dp column widths are unchanged. The
player is centred by the same fix: compact column x 0→24, wide 0→270.
The top-bar title and section headers no longer carry their ad-hoc `+4` offset,
so ink starts on the 16 dp page margin (section header ink: 4 → 16 px).

## Per screen (measured)

- **Home** — margins 0/32 → 16/16; vertical rhythm unchanged: chips→section
  header 16 dp, header→cards 20 dp, song pitch 72 dp (64 row + 8). Card-grid row
  gap 10 → 12 dp (`GRID_GAP`); today's live feed only has song shelves, so that
  gap is not in the before/after PNGs.
- **Library** — column 0..372 → 16..388. Liked tile→chips 36 dp, chip lines
  16 dp, header→first row 20 dp (unchanged, density preserved). Album/playlist
  card rows 10 → 12 dp.
- **Settings** — section title→first control 12 → 10 dp (ad-hoc 2 dp replaced by
  the 8 dp item spacing); chip block→caption 20 → 22 dp (6 → 8); every other
  6 dp group gap is 8 dp; section gap 24 + 30 dp unchanged.
- **Player** — column centred (art x 12..345 → 36..369 at 405); cover letterbox
  removed (see UI-04); art 333 dp; declared stack title→artist→album 4 / 4 dp
  (was 2 / 1), album→seek 12 (was 10); transport gaps 16 → 8 dp; the play FAB is
  on the art centre line (wide: 303.5 → 549.5 px against a 550 px window centre,
  compact 202.0 → 202.0 by coincidence of the old offset).
- **Queue** — 360×380 modal at 20 dp frame insets (360+40 = 400 fits the
  405 window; a 24 dp inset would clip), 56 dp rows unchanged. Trailing
  32 dp controls (up / down / remove / download) leave 208 dp for title and
  subtitle; duration moved into the subtitle. A Load more button sits below
  the scroll area while a radio continuation is available.
- **Lyrics** — line pitch 28 → 34 px (2 → 8 dp extra between lines).
- **Downloads** — heading→stats 11 → 9 px (26 → 24 dp); following rows shift up
  2 px; internal gaps unchanged.
- **Liked / detail action rows** — outlined buttons 32 → 40 dp, matching the
  filled button and pill radius; gaps 16 → 8 dp. Verified on Liked: Shuffle
  32 → 40 px, Play stays 40 px. Detail's row is centred with the shared width
  helpers.
- **Album/artist/playlist (code-level)** — art→title 16 dp; title stack 4/4 dp;
  action gaps 16 → 8 dp; bio clamped to 3 lines with a Show more/less pill
  (UI-05), state in egui memory.

## Shell / widget fixes

- **UI-03 top bar** — the title is elided to `width − title_x − 138` dp (the
  history/sync/avatar hit areas start 130 dp before the right edge). Compact ink
  limit 235 dp (was unbounded and ran under the icons), wide 822 dp.
- **UI-04 cover crop** — `cover_uv` trims the baked 12.5 % bars of ytimg 4:3
  thumbnails, then centre-crops to the target aspect. Before: 20 pure-black rows
  at the art centre (letterbox inside the square); after: 0. Unit-tested.
- **UI-06** — see action rows above.
- **Settings entry** — the wide rail pins a utility Settings item to the bottom
  under an inset divider; compact uses a fifth bottom-nav item after a vertical
  divider. Both use the existing 72 dp nav item geometry and label, highlight
  Settings when `Route::Settings` is open, and suppress the primary-tab
  highlight. The compact five-item row is 74.6 dp per item at 405 px.
- **UI-10 toasts** — identical texts refresh the visible toast instead of
  stacking, and the anchor moved from 100 to 152 dp above the bottom edge, so
  the toast ends 8–16 dp above the mini player instead of overlapping it.
  Shortcut: fixed inset; exact on both window classes needs the measured
  bottom-panel height passed from `app.rs`.
- **Rows/cards** — album card art→title 6 → 8 dp, subtitle +26 → +28 dp;
  skeleton rows match the real rows. Song rows reserve a 28 dp radio action
  before the download icon (72 dp trailing total); compact local-playlist rows
  hide it and keep 44 dp.

## Remaining / needs other owners

- **UI-02 (app.rs)** — `player_route` uses `(screen.width() − 48).min(560)` and
  the art is `(width − 24).min(380).min(available_height × 0.48)`: the wide art
  is 333 dp inside a 560 dp column. Needs the player's own centred max width
  (≈ 520) and art scaling.
- **UI-12 (app.rs)** — wide content width 960; long song rows put actions 870 dp
  from the text. Cap reading rows at ≈ 640–720 dp.
- **UI-13** — settings inline label+chip rows need one row widget (label left,
  control right) and 48 dp chip hit areas; that is a component change, not a
  gap change. Chips remain 32 dp visual (M3 spec) on a pointer-first UI.
- **UI-14 / UI-15** — section-scoped filter capsule and copy-to-playlist flow are
  navigation changes.
- **UI-16** — scrollbar width/colour is theme `visuals()`, untouched here.
- `screens/search.rs` is not tokenised while `search-radio` owns it.

## Verification

```
cargo fmt -p ml-ui
cargo check -p ml-ui
cargo test -p ml-ui --lib      # 26 passed, incl. 3 cover_uv cases
```

Pure function for the proof lead: `widgets::cover_uv(egui::Vec2, egui::Vec2) ->
egui::Rect` (total, no panics, unit-tested); `filled_button_width` /
`outlined_button_width` are pure given the painter's font metrics.
