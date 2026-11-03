# Motion & animation design

**Status: implemented (2026-11-03).** Phase A added `crates/ui/src/motion.rs`;
Phase B wired it through the theme, widgets, shell, app and screens. Tokens,
coverage and rationale below remain the design reference; where the shipped
code differs, "Implementation record" at the end wins.

## Why a system instead of effects

The shell paints almost everything by hand (`widgets/*`, `shell/mod.rs`,
`screens/*`), so per-widget `if hovered { … }` branches would multiply and each
screen would invent its own timing. Instead:

- one token set (durations, easings, scales) in a new `crates/ui/src/motion.rs`;
- one state-layer helper that all interactive widgets call, replacing the
  boolean `theme::state_layer` (12 call sites today);
- one route-enter animation in `App` instead of per-screen fades;
- egui's own `Context::animate_bool_with_time_and_easing` as the state store —
  it is frame-rate independent, reverses from the current value, and requests a
  repaint only while a value is between 0 and 1.

### Non-goals

Blur/glass, parallax, confetti, spring physics, per-glyph geometry morphing of
Material Symbols, exit animations that require keeping an unmounted route/modal/
player alive, and animation of layout height (mini player, panels). Each of
those costs far more than it reads on screen; the exceptions are called out in
the coverage table.

## Tokens

Durations are seconds (`f32`), matching egui. They are the Material 3 duration
tokens we use; unused M3 steps are not declared.

| Token | Value | M3 name | Used for |
|---|---|---|---|
| `HOVER` | 0.10 | short2 | state layer, press scale, thumb, focus ring |
| `PRESS` | 0.10 | short2 | alias of `HOVER`, names the press intent |
| `SELECT` | 0.15 | short3 | chip/nav selection morph, skeleton fade, icon swap |
| `ENTER` | 0.20 | short4 | route enter, lyrics fill |
| `EXIT` | 0.20 | short4 | toast fade-out (the only exit in this pass) |
| `MODAL` | 0.25 | medium1 | queue dialog, player open, mini player |
| `SHIMMER_SWEEP` | 1.8 | — | shimmer band loop (existing 0.55/s) |
| `SHIMMER_PULSE` | 3.93 | — | shimmer brightness loop (existing 1.6 rad/s) |
| `BARS_PERIOD` | 1.1 | — | queue playing-bars loop |
| `VINYL` | existing | — | 10°/s, 36 s per turn (unchanged) |

No long token is declared: the M3 long steps exist for full-screen shared-axis
transitions, and this UI's longest single motion (dialog/player) is 250 ms.
Adding an unused 450 ms constant would invite copy-paste of a duration nothing
on screen needs.

Easings are exact cubic-bezier solves (`motion::bezier`, 12 bisection steps,
~40 flops per call, only while a value animates):

| Name | Curve | Use |
|---|---|---|
| `emphasized` | (0.2, 0, 0, 1) | default: size, position, selection, route |
| `standard` | (0.2, 0, 0, 1) | alias with the same curve — M3 defines Standard and Emphasized identically; emphasis comes from duration, not curvature |
| `decelerate` | (0, 0, 0, 1) | M3 standard decelerate; one-shot entrances from outside (toast, mini player) |
| `accelerate` | (0.3, 0, 1, 1) | M3 standard accelerate; reserved for exits — no exit animation ships in this pass |

Source: the Material Web motion tokens vendored from the M3 spec
(`duration-short2/3/4`, `medium1`; `easing-standard`, `emphasized`,
`standard-decelerate`, `standard-accelerate`). The curves are implemented by
solving the cubic bezier, not by sampling `emath`'s polynomial approximations.

Geometry and tint tokens:

| Token | Value | Meaning |
|---|---|---|
| `HOVER_TINT` | 0.08 | white overlay on hover |
| `PRESS_TINT` | 0.12 | white overlay while pressed |
| `PRESS_SCALE` | 0.98 | button press |
| `PLAYER_SCALE` | 0.985 | player open zoom |
| `ROUTE_SLIDE` | 12 dp | route enter offset |
| `MODAL_RISE` | 12 dp | dialog rise |
| `TOAST_RISE` | 8 dp | snackbar rise |
| `BADGE_SCALE` | 1.06 | card play-badge hover scale |

No colour roles are added: overlays use the existing white state-layer, selected
containers use `secondary_container`/`on_secondary_container`, focus uses
`primary`. Contrast is unchanged at rest; the focus ring ends at full opacity.

## Runtime model (`crates/ui/src/motion.rs`)

```rust
pub const HOVER: f32 = 0.10;
pub const PRESS: f32 = 0.10;
pub const SELECT: f32 = 0.15;
pub const ENTER: f32 = 0.20;
pub const EXIT: f32 = 0.20;
pub const MODAL: f32 = 0.25;
// …tints, scales, offsets

pub fn emphasized(t: f32) -> f32;      // cubic-bezier(0.2, 0, 0, 1)
pub fn standard(t: f32) -> f32;        // same curve; intent alias
pub fn decelerate(t: f32) -> f32;      // cubic-bezier(0, 0, 0, 1)
pub fn accelerate(t: f32) -> f32;      // cubic-bezier(0.3, 0, 1, 1)
pub fn lerp(a: f32, b: f32, t: f32) -> f32;

pub fn set_reduced(ctx: &Context, reduced: bool);
pub fn reduced(ctx: &Context) -> bool; // default false when unset

pub fn t(ui: &Ui, id: Id, on: bool, secs: f32) -> f32; // 0 duration when reduced
pub fn state_layer(ui: &Ui, base: Color32, response: &Response) -> Color32;
pub fn state_layer_at(base: Color32, hover_t: f32, press_t: f32) -> Color32;
pub fn press_scale(ui: &Ui, response: &Response) -> f32;
pub fn press_scale_at(t: f32) -> f32;
pub fn nav_pill(t: f32) -> Vec2;       // 32×28 → 56×32
pub fn chip_radius(t: f32) -> f32;     // 8 → 16
pub fn focus_ring(ui: &Ui, rect: Rect, radius: u8, response: &Response);
pub fn appear(ctx: &Context, id: Id, secs: f32) -> f32; // one-shot 0→1
pub fn progress(now: f64, start: f64, secs: f32) -> f32; // pure route helper
```

Semantics verified in egui 0.37 (`context.rs:3709-3800`,
`animation_manager.rs:29-112`):

- `animate_bool_with_time_and_easing` is **rate based**: it moves from the
  current value toward the target at `1/duration` per second. A reversed
  animation continues from where it is; nothing restarts from 0 and nothing
  queues. Easing is mirrored when the target flips.
- `animate_value_with_time` interpolates linearly in wall-clock time and a
  re-target runs the full duration again (a small correction is as slow as a
  large one). It is therefore not used for interaction states; bool animations
  cover those. A continuous displayed value exists only in the seek slider,
  which ui-functional already owns via `progress_slider`.
- `animation_time == 0.0` snaps to the target (division guarded), so reduced
  motion needs no separate code path, only a duration of 0 passed to egui.
- Both `animate_bool*` and `animate_value_with_time` call `request_repaint()`
  **only while** `0 < value < 1`; at rest they request nothing. The first call
  for an id returns the target immediately (no fade-in), which is why one-shot
  entrances (route, dialog, skeleton) use `appear`/`progress` with a stored
  start time instead.

  shortcut: `appear` fades once per id per app run; a screen that reloads later
  does not replay the fade. Upgrade by keying the id with a load generation if
  that ever looks wrong.
- `ui.set_opacity(p)` multiplies the alpha of everything painted by that Ui and
  its children afterwards; it is the cheapest way to fade a block and does not
  move hitboxes.
- `ctx.transform_layer_shapes_after_rounding(layer, TSTransform)` is visual
  only. Used once, for the player open zoom; hitboxes stay at their final
  positions (≤9 dp mismatch at the window edge on a 1100 dp window, for the
  250 ms of the animation only).

`motion::set_reduced` stores the flag in `ctx.data`; `motion::reduced` reads it.
The single source is a `reduce_motion: bool` field on `Theme` (colours stay
cheap to clone): `Theme::from_settings` copies `settings.reduce_motion`, while
`App::new` applies the `METROLIST_REDUCE_MOTION` env override once before
building the theme (keeping `Theme::from_settings` hermetic for tests).
`Theme::install` calls `motion::set_reduced`, sets
`style.animation_time = HOVER` and a reduced-motion-aware
`style.scroll_animation`. Widget helpers only need `ui` because they read the
flag from the context. `app.rs`'s `theme_key` must gain the flag so toggling the
setting re-installs the style. No egui 0.37 `Options::reduce_motion` exists;
this is the app's own switch.

Route enter needs hand-held state in `App` because it is a one-shot, not a
state animation:

```rust
enter_route: Route,   // route this enter belongs to
enter_start: f64,     // ctx.input(|i| i.time) when it began
enter_from: Vec2,     // signed start offset in dp (x); settles to zero
```

`set_route` computes `enter_from.x`: tab switches use
`+12 * sign(new_tab_index - old_tab_index)`, pushes (album/artist/playlist/
settings) use `+12`, back uses `-12`. `start_search` (which assigns
`self.route` directly) must go through the same helper. On a second route change
mid-flight the animation restarts at 0 — an explicit snap, allowed by the spec.

## Coverage

At rest every row lands on the state the existing screenshots already show, with
one intended exception: a **selected chip** becomes a full pill (radius 16), so
the Home/Search chip captures must be re-shot. `docs/motion/*` (P2) is the only
place hover/pressed states are captured.

| Element (owner file) | Effect | Duration / easing | Repaint policy |
|---|---|---|---|
| Route & tab enter (`app.rs` `route_content`, `shell`) | new screen fades 0→1; content column slides `ROUTE_SLIDE` from the travel side (X for tabs/push/back) | 200 ms `emphasized` | 60 fps for 200 ms, then none |
| Player open (`app.rs` `ui`) | full-window fade + scale `PLAYER_SCALE`→1 about the screen centre; close snaps | 250 ms `emphasized` | 60 fps for 250 ms |
| Icon button (`controls.rs`) | state layer 8 %/12 %, press scale 0.98, focus ring 2 dp `primary` | 100 ms `emphasized` | 60 fps for 100 ms |
| Filled / outlined / "Play all" buttons | same state layer + press scale; hover border stays 1 dp | 100 ms | 60 fps for 100 ms |
| FAB play/pause (`controls.rs`, `player.rs`) | state layer + press scale; glyph crossfade (see icon swap) | 100 ms / 150 ms | 60 fps for 150 ms |
| Chip (`controls.rs` `chip`) | hover/press layer; selected fill + radius morph 8→16 dp (full pill); label colour lerp | 150 ms `emphasized` | 60 fps for 150 ms |
| Nav item (`controls.rs` `nav_item`) | selected pill grows 32×28→56×32 dp, fill fades in, icon/label colour lerp; hover layer on the unselected pill | 150 ms `emphasized` | 60 fps for 150 ms |
| Song row (`rows.rs` `song_row`) | hover 8 % / press 12 % fill fade; active-row fill fade | 100 ms | 60 fps for 100 ms |
| Album card (`rows.rs` `album_card`) | hover border alpha 0→1; play badge scale 1→`BADGE_SCALE`, black alpha 160→210 | 100 ms | 60 fps for 100 ms |
| Mini player (`shell/mod.rs`) | card fades in and scales 0.98→1 inside its slot when the first song appears. Panel height is **not** animated (see below) | 250 ms `decelerate` | 60 fps for 250 ms |
| Toast (`feedback.rs` `Toasts`) | in: fade + 8 dp rise; out: fade over the last 200 ms of `remaining` (no state, derived from the timer) | 150 ms in / 200 ms out, linear | 10 fps countdown; 60 fps only inside a fade window |
| Queue dialog (`feedback.rs` `modal`) | backdrop alpha 0→180; dialog fade + 12 dp rise via `Area::anchor` offset; close snaps | 250 ms `emphasized` | 60 fps for 250 ms |
| Search (`search.rs`) | suggestion popover fade + 4 dp slide; suggestion rows get the 8 % hover layer (they have none today) | 150 ms | 60 fps for 150 ms |
| Skeletons (`feedback.rs`, `screens/mod.rs`) | fade-in; existing shimmer sweep/pulse kept; while visible the shimmer now schedules its own frames | 150 ms in; 1.8 s / 3.9 s loops | 30 fps while visible (today it only advances on the app's 250 ms poll, i.e. ~4 fps) |
| Play/pause glyph (`shell` mini, `player.rs`) | both ligatures painted, crossfade 0→1, incoming scales 0.9→1; no geometry morph | 150 ms `emphasized` | 60 fps for 150 ms |
| Lyrics (`lyrics.rs`) | active line background fill fades in/out; colour + weight swap at the midpoint (avoids re-laying out a galley every frame); auto-scroll to the active line on change | 200 ms; scroll uses egui's 0.1–0.3 s scroll animation | 60 fps while a fade/scroll runs |
| Slider (`controls.rs`) | thumb radius 6→7.5 dp on hover/drag; press ring; **seek display smoothing stays with ui-functional** (`progress_slider`) | 100 ms | 60 fps for 100 ms |
| Vinyl (`player.rs`) | unchanged 10°/s spin; pause freezes; reduced motion keeps it still | continuous | 60 fps while playing (existing) |
| Playing bars (`rows.rs`) | three phase-shifted sine heights (8/14/10 ±3 dp) instead of static bars | 1.1 s loop | 60 fps while the queue dialog is open and playing |
| Focus ring (all custom widgets) | 2 dp `primary` ring, 100 ms fade to solid; also on pressed | 100 ms | 60 fps for 100 ms |

Deliberate omissions, with reasons:

- **No exit animations** for route, player, queue or mini player. egui destroys
  the unmounted Ui, so an exit would need a retained snapshot of the previous
  frame. Closing is instant; every enter starts from a known state.
- **No animated panel height.** Animating `Panel::bottom` re-sizes the central
  scroll area every frame; scroll offsets jump and it violates the no-per-frame
  layout rule. The mini player keeps its fixed 64 dp slot and animates opacity
  and a 2 % inset scale inside it, so nothing reflows.
- **No card art zoom on hover.** It would repaint a texture every frame for a
  1–2 % effect; border + badge carry the state.
- **No icon geometry morph.** Crossfading two Material ligatures is 2 draw calls;
  a real triangle↔bars morph means hand-drawn paths that would no longer match
  the icon font.

## Interruption & reversal rules

1. Every hover/press/selection animation is a bool target. A new target while
   the old one runs continues from the current value (rate based), so
   hover→press→release never jumps or queues.
2. No effect owns a queue. Repeating an interaction re-targets that element's
   single animation; it never stacks or replays from zero.
3. Route enter is time-based and snaps to fully on at 200 ms; a new route during
   an enter restarts it with the new direction. No cross-fade between two live
   routes (not possible without retaining the old Ui).
4. Seek and volume changes are not animated by motion. The seek thumb renders
   the value it is given; `progress_slider`'s display smoothing (ui-functional)
   is the only smoothing and it snaps on a seek jump.
5. Motion never changes `Sense`, `interactable`, hit rects, focus order or
   cursor icons. A click during a transition goes to the destination widget.
6. Visual-only transforms (player open) are applied after layout; the maximum
   hit-test mismatch is 0.75 % of the window width for 250 ms.
7. Reduced motion sets every duration to 0: animation helpers snap, loops are
   frozen/static, route slide is 0, auto-scroll jumps. Nothing is "nearly
   instant" or left half-animated.
8. No flash risk: the fastest loop is the equalizer at 1.1 s (0.9 Hz); the
   shimmer is 1.8–3.9 s. Nothing approaches the 3 Hz threshold, and no
   animation inverts full-screen luminance.

## Accessibility

- **Reduced motion** is a real setting: `ml_core::Settings::reduce_motion`
  (serde default false, so old settings files load), exposed in Settings →
  Appearance, plus `METROLIST_REDUCE_MOTION=1` for tests and users whose desktop
  has no toggle. The flag is honoured per effect, not by disabling hover
  feedback: state layers and focus rings still appear, just instantly.
- **Degrade table** under reduced motion: state layers/scale/ring instant;
  route/player/dialog/mini player/toast appear instantly; shimmer static at mid
  tone; equalizer bars static; vinyl frozen; lyrics scroll jumps to centre.
- **No flashing** (WCAG 2.3.1): no loop above 1 Hz, no full-screen luminance
  inversion; the dialog backdrop is a single 250 ms fade to 70 % black.
- **Contrast**: no colour changes at rest; the selected chip keeps
  `secondary_container`/`on_secondary_container`; the focus ring is solid
  `primary` (2 dp) and therefore meets the 3:1 non-text requirement in both
  schemes. Overlays only darken/lighten the existing role.
- **Keyboard**: in egui 0.37 `Sense::click()`/`Sense::click_and_drag()` are
  focusable (`Sense::CLICK` the const is not), and every custom widget here uses
  the constructors, so Tab reaches the custom buttons; the focus ring is the
  visible indicator. Motion does not steal focus or request it.
- **Input is never blocked** by an animation; the only `interactable(false)`
  Ui remains the existing toast area.

## Repaint & allocation budget

- **Idle**: motion adds zero repaint requests. Baseline `logic()` still asks for
  a frame every 250 ms (MPRIS/tray/instance polling) — 4 fps, not 0. That is
  pre-existing and out of scope for this pass; the motion layer must not add to
  it.
- **Explicit repaint calls** in the motion layer are only two: the route
  progress while `p < 1` (200 ms) and shimmer's
  `request_repaint_after(33 ms)` while a skeleton is visible. Everything else
  relies on egui's animation manager, which stops requesting as soon as each
  value reaches its target.
- **Transitions**: one 60 fps burst per interaction, longest 250 ms (dialog,
  player, mini player). State layers/rows end in 100 ms.
- **Loops**: shimmer 30 fps (`request_repaint_after(33 ms)`) only while a
  skeleton is on screen; vinyl 60 fps only while playing; playing bars 60 fps
  only while the queue dialog is open and playing. Fades in the toast switch to
  60 fps only during the first 150 ms / last 200 ms.
- **Memory**: egui's `AnimationManager` keeps one entry per animated id. Ids
  are per widget instance; rows use their response id, so the entries are
  bounded by the widgets rendered at least once (a few hundred), not by the
  catalogue. No `Vec`/`String`/`format!` is added to a per-frame path; colours
  use `Color32::lerp_to_gamma` and rect maths only.
- Existing per-frame galley layouts in `paint_icon`/`text` already allocate;
  motion does not add any. Memoising them is a separate perf item.

## Testing

### Pure tests (`motion.rs`, `#[cfg(test)]`)

- `bezier` endpoints: `f(0)=0`, `f(1)=1`, monotone on 33 samples, no overshoot
  for (0.2,0,0,1) and (0,0,0,1).
- `progress`: before start →0, at start+dur →1, mid → eased midpoint; `secs=0`
  →1 (reduced motion).
- `appear` with `secs=0` →1 on the first call.
- Tint: `0 → 0.00`, hover only → `0.08`, hover+press → `0.12` (±1e-6).
- `press_scale`: 1.0 at t=0, 0.98 at t=1.
- `nav_pill`: 32×28 at t=0, 56×32 at t=1, monotonically increasing.
- `chip_radius`: 8 at t=0, 16 at t=1.

### Widget-level hover/pressed states (`controls.rs` tests)

`ctx.run_ui` with `RawInput` pointer events and explicit `RawInput.time` ticks,
as the shell tests already do (`shell/mod.rs:410-505`). Run ~10 frames at 16 ms
per state and capture `FullOutput.shapes`; assert:

- idle icon button paints no state-layer circle;
- after 120 ms of `PointerMoved` onto it, a state-layer circle exists whose fill
  equals `state_layer_at(base, 1, 0)`;
- after 120 ms with `PointerButton{pressed:true}` held, the circle fill equals
  `state_layer_at(base, 1, 1)` and its radius is `press_scale_at(1)` smaller
  than the hover radius;
- after the pointer leaves and 3 frames pass, the circle is gone and
  `ctx.has_requested_repaint()` is false (no idle churn).

This replaces pointer PNG capture: eframe owns the live `RawInput`, so faking a
hover in the running screenshot loop would mean re-injecting events into
`InputState` — fragile, while shape assertions test the exact pixels that would
be captured.

### Performance checks

- `after_settle_no_repaint`: run the widget for 4 frames after the target is
  reached; `ctx.has_requested_repaint()` must be false.
- `reduced_motion_is_instant`: `set_reduced(true)`, one frame → target value,
  no repaint requested.
- Manual protocol: `metrolist --smoke 120` before/after must stay `SMOKE_OK`;
  idle CPU is measured with the app stopped on Home for 10 s (expect the
  pre-existing 4 wakeups/s, no animation frames); during a 405-wide hover sweep
  the frame time budget is 16.6 ms and must not be exceeded. No per-vsync
  allocation is claimed beyond the existing painters.
- All Rust builds/tests run under the repo's 2-core cap (`.cargo/config.toml`).

### Proofs

`crates/ui/src/app.rs` (with `services.rs`) and `crates/ui/src/pure.rs` are
frozen sources for `proofs/radio` and `proofs/ui` (see `docs/PROOFS.md`). Any
APPLY edit to them requires the normal reviewed re-freeze (`proofs/freeze.sh`,
fresh reviewer) — do not run it as part of the motion pass without that.
`motion.rs` is f32/bezier/egui state, i.e. OUT-OF-SUBSET like
`center_column`; it is covered by unit tests and gets one honest line in the
OUT-OF-SUBSET table, not a trust marker and not a fake proof.

## Implementation checklist (APPLY, in order)

*(Done, with the deviations listed in "Implementation record".)*

**P0 — shared core**

1. `crates/ui/src/motion.rs` (new): tokens, bezier, `t`, `state_layer`,
   `state_layer_at`, `press_scale`/`press_scale_at`, `nav_pill`, `chip_radius`,
   `focus_ring`, `appear`, `progress`, reduced flag, pure tests; add
   `pub mod motion;` to `lib.rs`.
2. `theme/mod.rs`: delete the boolean `state_layer`; set
   `style.animation_time = HOVER` and a reduced-motion-aware
   `style.scroll_animation`; re-export nothing (call sites move to `motion::`).
3. `widgets/controls.rs`: icon button, filled/outlined/play-all, chip, nav item,
   FAB, slider thumb; all use `motion::state_layer` + `press_scale` + ring.
4. `widgets/rows.rs`: song row, album card, playing bars.
5. `widgets/feedback.rs`: shimmer repaint + fade-in, `Toasts` fade/rise, `modal`
   enter.
6. `shell/mod.rs`: mini-player fade/rise; top-bar avatar hover ring.
7. `app.rs`: route-enter state + `set_route`/`switch_tab`/`go_back`/
   `start_search` direction; shell content offset; player-open fade/scale;
   `theme_key` gains `reduce_motion` (install calls `set_reduced`).
8. Mechanical `state_layer` replacement in `screens/{home,library,queue,stats}.rs`
   (6 sites).

**P1 — screens**

9. `screens/player.rs`: play/pause crossfade (reuse the helper), like/shuffle/
   repeat hover states, vinyl reduced-motion gate; **do not touch
   `progress_slider`/seek smoothing**.
10. `screens/search.rs`: suggestion popover fade/slide + row hover.
11. `screens/lyrics.rs`: active-line fill fade (colour/weight swap at midpoint)
    + auto-scroll on active change.
12. `screens/mod.rs`: skeleton fade-in (`motion::appear`).
13. `ml_core::Settings::reduce_motion` + Settings → Appearance switch +
    env override; add the one-line OUT-OF-SUBSET entry for `motion.rs` in
    `docs/PROOFS.md`.

**P2 — polish**

14. Animated equalizer in the queue dialog.
15. Focus-ring pass on rows/cards/nav (same helper, one call each).
16. Optional: pointer-state screenshot harness in the `--screenshot` CLI
    (`--pointer X,Y`, `--pressed`) and `docs/motion/*.png`; deliberately last
    because it touches the live input path.

Verification after the whole list: one `cargo test -p ml-core -p ml-ui` run
(single compile), `metrolist --smoke 120`, 405×900 and 1100×760 screenshots of
every demo screen compared against `docs/`, and the reduced-motion pass (env
var) on Home, Player, Search, Queue and Lyrics.

## Files & coordination

| File | Now | After ui-functional lands |
|---|---|---|
| `crates/ui/src/{app,state,pure}.rs`, `screens/mod.rs`, `screens/{library,detail,liked,local_playlist,player,catalog,history}.rs`, `widgets/{rows,controls,mod}.rs` | ui-functional (active; list taken from its live reply and `git status`) | motion re-reads the diff, then edits its hooks |
| `crates/ui/src/shell/mod.rs`, `theme/mod.rs`, `widgets/feedback.rs`, `screens/{home,search,lyrics,stats,queue}.rs`, `settings.rs` | free | motion owns |
| `crates/ui/src/motion.rs` | does not exist | motion owns |
| `crates/ui/src/widgets/controls.rs` `progress_slider` | ui-functional adds seek smoothing | motion adds only thumb hover/press visuals; never duplicates smoothing |
| Toast / Modal / mini player | confirmed untouched by ui-functional | motion owns |

Design decisions taken here and worth vetoing before APPLY if unwanted: (a)
selected chips become full pills (radius 8→16), (b) the player opens with a
0.985 zoom, (c) no exit animations anywhere.

## Implementation record (2026-11-03)

Files: `crates/ui/src/motion.rs` (new), `theme/mod.rs`, `widgets/{mod,controls,rows,feedback}.rs`,
`shell/mod.rs`, `app.rs`, `screens/{mod,home,library,search,player,lyrics,queue,stats,settings}.rs`,
`downloads.rs`, `crates/core/src/settings.rs`, `lib.rs` (`pub mod motion;`, batch-mode
instance bypass).

Deviations from the design text:

- **Mini player** enters with a fade plus a small inset scale, not a rise: it keeps
  the fixed 64 dp slot so the list below never reflows.
- **Demos never play random tracks:** Queue and Lyrics build a three-row local list
  of the approved `Empty Streets` fixture (muted) instead of the `next` radio
  queue; the Player demo already played one list track. `app.rs` has a test that
  every demo-queue id is `DEMO_SONG_ID` and the radio-start rule stays false.
- **Selected chips morph to full pills** (radius 8 → 16 dp), as decided in Phase 1;
  Home/Search screenshots were re-shot for the new shape.
- **Two one-off dialogs snap open** (`downloads.rs` and the library import dialog
  pass `progress = 1.0`); they are not part of a screen's main flow.
- **Batch CLI modes skip the single-instance guard** so `--smoke`/`--screenshot`
  can run next to an interactive window instead of being forwarded and exiting.
- **Screenshots wait for idle wall time, not frame count:** demo modes flip their
  final state (pause, popover) on the first idle frame, and an uncapped GPU can
  render six frames inside one crossfade. The capture waits 0.35 s from the first
  idle frame, so no capture freezes a half-finished animation.
- **Focus ring colour** comes from the active widget stroke (primary), so it stays
  a theme decision rather than a motion constant.

Verification: `cargo fmt -p ml-ui -- --check` clean; `cargo test -p ml-ui --lib`
109 passed; the full `cargo test -p ml-ui` is 122 = 109 lib + 13 integration,
6 live tests ignored (workspace: 744 passed / 0 failed / 31 ignored);
`metrolist --smoke 120` printed `SMOKE_OK frames=120 backend=Vulkan
home_sections=2`. The final capture set is
`docs/ui-motion-{home,search,player,library,queue,lyrics}-{405,1100}.png`,
`ui-motion-mini-405.png`, and reduced-motion
`ui-motion-reduced-{home,player}-405.png`; the player captures were retaken
after the audit fixes.
The Queue and Lyrics demos now queue three copies of the approved `Empty Streets`
fixture (list source, muted; no radio, no `next`); their 405/1100 captures were
re-shot with only that track. The final Player captures are active playback at
1:01 (405 and 1100), muted, with no error; the earlier 1100 error did not
reproduce in isolated runs.

Deliberate omissions: no toast screenshot capture (toasts need a live action in
the pointer harness) and the demo Library renders a fresh profile (0 songs).
The `proofs/radio` re-freeze is accepted: 12 pairs, 296 functions verified,
`REVIEW.md` ACCEPT, and `docs/PROOFS.md` is current.

## Library evaluation (2026-11-03)

Both candidates were downloaded and exercised in throwaway crates; neither is
adopted. `crates/ui/src/motion.rs` stays the system of record.

**`egui_animate` 0.6.0** (MIT OR Apache-2.0, 1,049 Rust LOC, one dependency:
egui). crates.io and git `master` are both `egui = "0.36"`; there is no 0.37
release. A 0.37 port works: bump the dependency to `egui = "0.37"` (the crate's
own code compiles unchanged) and pass `Id::unique`/`Id::new` at call sites,
because 0.37 dropped `From<&str> for Id`. A scratch sequence fade and layer
crossfade ran. Rejected for three reasons beyond maintenance: (a) `Sequence`
renders one page at a time and `Layer` renders both every frame, so a route
transition would double-render whole screens, and our `App` renders one route
against live state rather than a `Clone + PartialEq + Default` page value;
(b) an interrupted transition keeps its original start time, so the next change
resumes the stale timeline (`run_state` reports running while the value is back
at the start) instead of reversing; (c) its default normals are linear, so we
would still supply the M3 bezier and reduced-motion branching.

**`blinc_animation` 0.5.1** (Apache-2.0, 4,112 LOC; pulls `blinc_core`,
slotmap, smallvec, tracing, web-time; MSRV 1.75). The adapter worked: a `Spring`
in `ctx.data_mut`, stepped by `stable_dt` (clamped to 50 ms) with
`request_repaint()` only while `!is_settled()`, plus `AnimationPreset::pop_in`
advanced by `tick(ms)`. Measured at 60 fps: `stiff` settles in 383 ms with 2.8 %
overshoot, `snappy` 300 ms / 1.2 %, `gentle` 667 ms / 7.3 %; `pop_in(250 ms)`
peaks at scale 1.1. Our M3 tokens are 100–250 ms with no overshoot, and egui's
`animate_bool_with_time_and_easing` already stores the interrupted value and
requests repaints only mid-flight. Springs would add a second, non-M3 timing
language and a 4k-LOC dependency for no gain on state layers, nav, chips or the
player transition.

If page crossfades become a requirement later, the smallest path is a local
two-route render pass driven by one start time, not either crate.
