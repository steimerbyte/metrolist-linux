# UI audit — v0.1.3

Verdict: **PASS** — final post-audit: all four should-fixes and the evidence blocker are verified; no blocker or should-fix remains. Nits only: 16 px icon, sort-menu elevation, 900 px player composition, capture-state placeholders / 1 s queue duration, and a narrow-row queue text/hit edge.

Read-only review of the shipped PNGs plus the player/row/queue layout code. No build, no code or image changes.

## Verified without finding

- **Search pill is truly centred.** 405: `x=32..372`, `y=67..126` (341x60) in a 405 px window — 32 px gutter on both sides. 1100: `x=234..953`, `y=69..128` (720x60) in the 88..1100 content area — 146 px gutter on both sides; centre 593.5 vs content centre 594. The rounded chip row is not part of the pill's frame.
- **Nav/settings are consistent.** 1100 rail is 88 px wide; selected pills are 56x32 at `x=16..71`; the Settings group is separated by a 1 px divider and pinned 14 px above the bottom. 405 bottom nav uses 16 px side insets and five 74.6 px cells, with the Settings divider centred between cells 4 and 5.
- **Transport order is correct.** Like, shuffle, previous, play, next, repeat, radio at both widths; play is 63–64 px, the others 22 px glyphs in 40 px hit cells. (The row's horizontal centre is the finding below; the top-bar queue icon is correctly 24 px inside the content edge.)
- **Sort chip/menu anchor correctly.** Chip 178x32 at `(16,403)` / `(114,357)`; menu 196x250 opens 1 px below, flush left, selected row 178x40; no clipping at either width.
- **Creator hover affordance exists.** `ui-creator-hover-405.png` draws a 1 px primary underline under "Aster Vale" only (`x=84..142`, `y=492`), not under the duration.
- **Progress preview is anchored correctly.** Track `x=57..347` / `x=329..770`; the 1:01 thumb is at 26.3 % of the track at both widths. The hover bubble (38x20, `x=201..238`) is anchored to the hovered 2:09 position, not the current thumb, which is correct for a seek preview.
- **Queue modal geometry.** 405: 373x262 at `x=16..388, y=319..580`; 1100: 402x262 at `x=349..750, y=249..510`; three 56 px rows with ~28 px bottom padding, centred; header/count/close and scrim layer correctly. (The pre-fix fixed-height 474 finding is resolved; see Fix verification.)
- **No layout jump from state layers/press scale in the captured endpoints.** Search pill x/y and player block positions are identical between the static and motion captures; the only state-layer difference is the selected-chip radius (nit). The 405 motion player was recaptured at 15:22 and now matches the progress capture (play centre 202.0).
- **Contrast and text fit.** on_surface 248,224,224; on_surface_variant 188,166,166; outline 165,146,146 on #000 (about 10:1 / 8:1 / 7.8:1). Times at 11 px are legible. No clipped labels; the only intentional overflow is the horizontally scrollable chip row at 405.

## Findings

### Blocker (evidence) — progress label and thumb disagree in `ui-motion-player-1100.png` — **fixed post-audit**

At 1100x760 the label reads `1:22` (`x=321..339`, `y=560..567`) but the thumb centre is at `x≈451` on the `x=329..770` track and the fill ends at `x=457`. That is `(451-329)/441 = 27.7 %`, i.e. about `1:05`, roughly 18 s behind the label. The 900-high `ui-player-progress-1100.png` is consistent (1:01, thumb centre 445), so this is the 760-high capture taken mid `smooth_progress`, not a static-state disagreement.

Per the evidence rule (a still with a stale/inconsistent playhead is a blocker) the capture must be retaken after the smoothing settles, or the demo seek must snap the animation. No product change is implied: the label is current and the thumb eases toward it by design.

### Should-fix — player transport row is 13–15 px left of the content centre — **fixed post-audit**

Measured icon centres at 405: `47.5, 93, 139, 188.5, 243, 289.5, 333.5` — play centre 188.5 vs the art/title/progress centre 202.5. At 1100: `394.5, 439, 486.5, 535.5, 590, 636, 681.5` — play centre 535.5 vs 550. The first cell is correct, then each slot loses width, so the row compresses left; the play button and the whole row sit visibly off the centre line in `ui-player-transport-405.png`, `ui-player-progress-405.png`, `ui-player-progress-1100.png` and `ui-motion-player-1100.png`.

Root cause: `crates/ui/src/screens/player.rs:511-526` — `transport_slot` calls `allocate_ui_with_layout(Vec2::new(slot_width, 64.0), ...)`, but the child advances by its content's min width (about 44.7 px at 405 instead of the 48.8 px slot), so the six cells collapse and the 64 px play cell lands early. `transport_geometry` (503-507) and the test `transport_slots_keep_play_centred_at_every_width` (751) pass because they only test the arithmetic, not the rendered rects. Reserve `slot_width` in each cell (e.g. set the child's min width) and assert the rendered play centre equals `rect.center().x` in the test.

### Should-fix — song-row overflow "⋮" is decorative; clicking it plays the track — **fixed post-audit; all flagged captures recaptured**

Every `song_row` paints a `more_vert` dots at `rect.right()-16` (`crates/ui/src/widgets/rows.rs:65-67, 128-134`) but creates no interaction for it. The only row responses are the whole-row click (play) and the radio/download buttons; the library appends a playlist-add after the row. In the captures the dots is at `x=328.5` (405) / `1013.5` (1100) inside the row hit rect, so a user aiming at an overflow menu starts playback instead. Visible in every row of `ui-library-sort-405.png`, `ui-library-sort-1100.png`, `ui-motion-mini-405.png` and the home quick picks.

Either wire the dots to the row's menu/actions or remove the glyph; leaving a dead menu affordance in the list is worse than the missing action.

### Should-fix — trailing action hit rects overlap — **fixed post-audit**

`rows.rs:66-68, 136-176`: radio/download centres are 28 px apart, but each hit rect is 40x40 (`TOUCH=40`), so the targets overlap by 12 px; the same pattern is used in the queue at 32x32 with 28 px centres (`crates/ui/src/screens/queue.rs:167, 195, 219-237`; 4 px overlap). Measured centres 405: `272.5, 300.5, 328.5`; 1100: `957.5, 985.5, 1013.5`. Clicks in the gap can hit the neighbouring action.

Space the hit rects at least as wide as themselves (or consolidate the actions behind the row menu). Separately: `TOUCH=40` (`crates/ui/src/theme/spacing.rs:25`) also gives the transport and top-bar icon buttons 40x40 targets, below the M3 48 dp minimum; acceptable for a desktop pointer, but should be a conscious token choice rather than an accident.

### Should-fix — queue modal is a fixed 474 px tall for three rows — **fixed post-audit**

The 405 modal is `x=16..388`, `y=213..686` (373x474); the 1100 modal is `x=349..750`, `y=143..616` (402x474). The three 56 px rows end around y≈466 / y≈396, leaving about 220 px of empty surface below the list. The cause is the explicit `Vec2::new(width, 380.0)` viewport in `crates/ui/src/screens/queue.rs:75-96`; the comment explains it avoids a one-row clip, but the result over-allocates for short queues. Size the scroll viewport from the row count up to 380 px (or otherwise clamp the content height) so a three-track queue does not look half-empty.

### Nit — 16 px app icon loses the play glyph

In `assets/icons/metrolist-desktop-16.png` and `docs/icon-preview.png` the 16 px mark is a red squircle with a 10–11 px dark disc; the play triangle is a 4x4 salmon shape on that disc and reads as a bright dot. The silhouette survives, and 32/48/64/128/256 are legible, but the 16 px is marginal in a taskbar. The preview also omits the 64 and 256 assets that ship in `assets/icons/`; the files themselves are correct sizes.

### Nit — disabled queue reorder arrows barely differ from enabled ones — **fixed post-audit**

`queue.rs:232, 239` uses `outline` (165,146,146) for the disabled up (row 1) and disabled down (last row) versus `on_surface_variant` (188,166,166) for enabled. In `ui-motion-queue-405.png` / `ui-motion-queue-1100.png` the ends look clickable; only a 23-step colour difference separates them. Use a clearly dimmed colour or omit the end arrows.

### Nit — long queue titles can reach under the up-arrow hit rect

`crates/ui/src/screens/queue.rs:191-193` elides the title to `up_x - 8`, but the 32 px up target starts at `up_x - 16`, so a title that fills the width ends 8 px inside the up hit rect. At the test's 240 px row the 80 px text floor overlaps it by 26 px. The 331/360 px rows and the current 3× fixture titles do not reach the edge; synthetic/long-title only.

### Nit — sort menu surface is page-black with only a 1 px outline

The popup fill is `#000` (theme `window_fill = background`) with an 84,68,68 1 px stroke; against the black page the menu boundary is only that hairline, and the selected row is the only filled item (87,64,64, 178x40). It reads on top of the track rows because the fill is opaque, but it has no tonal elevation as an M3 menu should. A `surface_container` fill (or a shadow) would separate it from the page.

### Nit — evidence-state hygiene

- `ui-search-pill-405.png` / `-1100.png` were captured mid-SELECT: the "All" chip's top row is 24 px wide vs 8 px in the settled `ui-motion-search-*` captures (radius ~10 vs the 16 px pill), and the motion captures show the Search tab's hover fill (94,68,69 vs 80,52,53). The field's centring is unaffected, but the static shots are not the settled chip state.
- `ui-motion-reduced-home-405.png` has unloaded quick-pick thumbnails (music-note placeholders) and a different row set, while `ui-motion-home-405.png` has loaded art; the reduced-motion layout itself is fine.
- The replacement `ui-library-sort-{405,1100}.png` show every cover as a music-note placeholder and no mini player; the replacement queue captures show a placeholder player backdrop. The older `ui-library-sort-menu-1100.png` mini-player placeholder is the same category; capture-time loading states, not layout defects.
- The `ui-motion-player-1100.png` playhead blocker above was retaken (1:01 / 3:51); all previously flagged stale captures were recaptured at 15:22 (see Fix verification).

### Nit — player composition at 900 px height is top-heavy

At 405x900 the volume row ends at y≈701, leaving about 199 px of empty backdrop below; at 1100x900 it ends at y≈748 (152 px empty). The 1100x760 capture fills the window correctly, so this is a tall-window composition issue rather than a break; a flexible spacer before the volume row would balance it if 900 px is a supported size.

## Fix verification (post-audit)

Re-checked against the replacement captures; no code or images changed.

- **Transport centring — fixed.** 405 play centre 202.0 vs content centre 202.5 (`ui-player-transport-405.png` / `ui-player-progress-405.png`); 1100 play centre 550.0 vs 550.0 (`ui-player-progress-1100.png`); icons run 47.5→356.5 at 405 and 394.5→704.5 at 1100. The new rendered test scans the actual hit rects at content widths 357/560/400.
- **Progress label/thumb — fixed.** `ui-motion-player-1100.png` now reads `1:01` / `3:51`; thumb centre `x=446.5` on the `329..770` track = 26.6 % ≈ 1:01. The 900-high `ui-player-progress-1100.png` agrees (thumb 446.5, 1:01 / 3:51), as do the 405, paused and hover captures. The blocker is cleared.
- **Dead ⋮ — fixed.** `ui-library-sort-{405,1100}.png` show only radio, download and playlist-add at a 40 px pitch; the row's 40x40 rects are disjoint (new test), and playlist_add remains interactive. The replacement sort pair is the post-fix song-row evidence used here.
- **Queue height — fixed.** 405 `x=16..388, y=319..580` (373x262); 1100 `x=349..750, y=249..510` (402x262); three 56 px rows, ~28 px bottom padding, centred (449.5 / 379.5). The 50-track `ui-queue-live-1100.png` still caps at 380 body / 474 total, so the cap holds. Disabled end arrows are now 38 % on_surface and consume clicks (nit resolved).
- **Queue action hit rects — fixed.** 32x32 rects at a 36 px pitch: button centres right−20/−56/−92/−128 (4 px gaps), text edge right−128; the replacement queue captures show the 36 px glyph pitch (405: 240/276/312/348; 1100: 602/638/674/710). The new test covers 240/331/360 and asserts each rect is inside the row and pairwise disjoint; the only residual is the long-title text edge noted above.
- **Evidence recapture — resolved.** All previously flagged stale captures were recaptured from the current binary at 15:22 (home/mini/reduced-home/search/creator/sort-menu/library, motion-player-405, vinyl-405/1100); the current player captures show the 202.0/550.0 transport and no dead ⋮. Remaining capture-state nits: the replacement sort pair still has placeholder covers and no mini player; the replacement queue captures still have a placeholder player backdrop and 3:52 vs the player/library 3:51.

## Notes on method

All coordinates are device pixels from the named PNGs at their native 405 or 1100 width. The 1100 player captures are 1100x900 for the progress/paused sets and 1100x760 for the motion set, which is why the vertical layout differs between them. Code was cross-checked only for the decisions above (`screens/player.rs`, `widgets/controls.rs`, `widgets/rows.rs`, `screens/queue.rs`, `theme/`, `shell/`); nothing was run or rebuilt.
