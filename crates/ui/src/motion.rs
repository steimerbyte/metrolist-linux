//! Shared M3 motion tokens and helpers.
//!
//! Interaction state uses egui's rate-based `animate_bool_with_time_and_easing`:
//! a reversed animation continues from its current value, nothing queues, and a
//! repaint is requested only while a value is between 0 and 1. One-shot
//! entrances (routes, skeletons, dialogs) use [`appear`]/[`progress`] instead,
//! because the first `animate_bool` call for an id returns the target instantly.
//!
//! Reduced motion is a context flag written by `Theme::install`; every helper
//! reads it and collapses its duration to zero, which egui snaps.

use eframe::egui::{Color32, Context, Id, Rect, Response, Stroke, StrokeKind, Ui, Vec2};

/// State layer, press scale, thumb and focus-ring time (M3 short2).
pub const HOVER: f32 = 0.10;
/// Same duration as [`HOVER`], named for the press intent.
pub const PRESS: f32 = 0.10;
/// Chip/nav selection morph, skeleton fade, icon swap (M3 short3).
pub const SELECT: f32 = 0.15;
/// Route enter and lyric fill (M3 short4).
pub const ENTER: f32 = 0.20;
/// Toast fade-out; the only exit in the current design (M3 short4).
pub const EXIT: f32 = 0.20;
/// Dialog, player-open and mini-player entrance (M3 medium1).
pub const MODAL: f32 = 0.25;

/// White overlay alpha on hover.
pub const HOVER_TINT: f32 = 0.08;
/// White overlay alpha while pressed.
pub const PRESS_TINT: f32 = 0.12;
/// Button scale while pressed.
pub const PRESS_SCALE: f32 = 0.98;
/// Whole-window zoom while the player opens.
pub const PLAYER_SCALE: f32 = 0.985;
/// Route-enter offset in dp.
pub const ROUTE_SLIDE: f32 = 12.0;
/// Queue-dialog rise in dp.
pub const MODAL_RISE: f32 = 12.0;
/// Toast rise in dp.
pub const TOAST_RISE: f32 = 8.0;
/// Album-card play badge hover scale.
pub const BADGE_SCALE: f32 = 1.06;

/// Shimmer band loop, seconds.
pub const SHIMMER_SWEEP: f32 = 1.8;
/// Shimmer brightness loop, seconds.
pub const SHIMMER_PULSE: f32 = 3.93;
/// Queue playing-bars loop, seconds.
pub const BARS_PERIOD: f32 = 1.1;

fn reduced_id() -> Id {
    Id::unique("metrolist_motion_reduced")
}

/// Store the reduced-motion flag for this context.
pub fn set_reduced(ctx: &Context, reduced: bool) {
    ctx.data_mut(|data| data.insert_temp(reduced_id(), reduced));
}

/// Whether motion is reduced. Defaults to `false` when `Theme` never installed.
pub fn reduced(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(reduced_id()).unwrap_or(false))
}

/// Cubic-bezier easing. `(x1, y1, x2, y2)` with implicit ends `(0,0)`/`(1,1)`.
///
/// Solves `x(u) = t` with 12 bisection steps. Exact M3 curves cost ~40 flops
/// and are only evaluated while an animation is in flight.
pub fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let sample = |a: f32, b: f32, u: f32| {
        let v = 1.0 - u;
        3.0 * v * v * u * a + 3.0 * v * u * u * b + u * u * u
    };
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for _ in 0..12 {
        let mid = (lo + hi) * 0.5;
        if sample(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    sample(y1, y2, (lo + hi) * 0.5)
}

/// M3 Emphasized easing: `cubic-bezier(0.2, 0, 0, 1)`. Default for size and
/// position changes.
pub fn emphasized(t: f32) -> f32 {
    bezier(0.2, 0.0, 0.0, 1.0, t)
}

/// M3 Standard easing. The spec uses the same curve as [`emphasized`]; the name
/// records intent (colour/opacity changes rather than travel).
pub fn standard(t: f32) -> f32 {
    bezier(0.2, 0.0, 0.0, 1.0, t)
}

/// M3 standard decelerate: `cubic-bezier(0, 0, 0, 1)`.
pub fn decelerate(t: f32) -> f32 {
    bezier(0.0, 0.0, 0.0, 1.0, t)
}

/// M3 standard accelerate: `cubic-bezier(0.3, 0, 1, 1)`.
pub fn accelerate(t: f32) -> f32 {
    bezier(0.3, 0.0, 1.0, 1.0, t)
}

/// Linear interpolation; `t` is expected in `0..=1`.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Eased progress of a one-shot entrance.
///
/// Pure: the caller requests a repaint while the result is below 1. Durations
/// at or below zero (reduced motion) return 1 immediately.
pub fn progress(now: f64, start: f64, secs: f32) -> f32 {
    if secs <= 0.0 {
        return 1.0;
    }
    emphasized(((now - start) as f32 / secs).clamp(0.0, 1.0))
}

/// One-shot fade-in for a block: 0 on its first frame, eased to 1 over `secs`.
///
/// The start time is remembered per `id`, so this fades once per app run;
/// a reload that should replay the fade needs an id keyed by load generation.
/// Requests repaints while the fade is in flight.
pub fn appear(ctx: &Context, id: Id, secs: f32) -> f32 {
    if reduced(ctx) || secs <= 0.0 {
        return 1.0;
    }
    let now = ctx.input(|input| input.time);
    let start = ctx.data_mut(|data| *data.get_temp_mut_or(id.with("appear"), now));
    let value = progress(now, start, secs);
    if value < 1.0 {
        ctx.request_repaint();
    }
    value
}

/// Animated bool target with the shared emphasized curve.
///
/// The first call for an `id` returns the target, as egui does; use a bool
/// animation only for state that can change after that first frame.
pub fn t(ui: &Ui, id: Id, on: bool, secs: f32) -> f32 {
    let secs = if reduced(ui.ctx()) {
        0.0
    } else {
        secs.max(0.0)
    };
    ui.ctx()
        .animate_bool_with_time_and_easing(id, on, secs, emphasized)
}

/// State-layer tint for explicit hover/press amounts (pure).
pub fn state_layer_at(base: Color32, hover_t: f32, press_t: f32) -> Color32 {
    let hover_t = hover_t.clamp(0.0, 1.0);
    let press_t = press_t.clamp(0.0, 1.0);
    let tint = HOVER_TINT * hover_t + (PRESS_TINT - HOVER_TINT) * press_t;
    base.lerp_to_gamma(Color32::WHITE, tint.clamp(0.0, 1.0))
}

/// Animated hover/press amounts for a widget response.
pub fn interaction(ui: &Ui, response: &Response) -> (f32, f32) {
    let hovered = response.hovered() || response.is_pointer_button_down_on();
    let pressed = response.is_pointer_button_down_on();
    let hover_t = t(ui, response.id.with("hover"), hovered, HOVER);
    let press_t = t(ui, response.id.with("press"), pressed, PRESS);
    (hover_t, press_t)
}

/// M3 state layer for a widget response: 8 % hover, 12 % pressed.
pub fn state_layer(ui: &Ui, base: Color32, response: &Response) -> Color32 {
    let (hover_t, press_t) = interaction(ui, response);
    state_layer_at(base, hover_t, press_t)
}

/// Animated press scale; 1 at rest, [`PRESS_SCALE`] while held.
pub fn press_scale(ui: &Ui, response: &Response) -> f32 {
    press_scale_at(t(
        ui,
        response.id.with("press"),
        response.is_pointer_button_down_on(),
        PRESS,
    ))
}

/// Press scale for an explicit animation value (pure).
pub fn press_scale_at(t: f32) -> f32 {
    lerp(1.0, PRESS_SCALE, t.clamp(0.0, 1.0))
}

/// Nav-item indicator size: 32×28 dp unselected, 56×32 dp selected.
pub fn nav_pill(t: f32) -> Vec2 {
    let t = t.clamp(0.0, 1.0);
    Vec2::new(lerp(32.0, 56.0, t), lerp(28.0, 32.0, t))
}

/// Chip corner radius: 8 dp unselected, 16 dp (full pill) selected.
pub fn chip_radius(t: f32) -> f32 {
    lerp(8.0, 16.0, t.clamp(0.0, 1.0))
}

/// Keyboard focus ring: 2 dp inside the control, fading in over [`HOVER`].
///
/// The colour comes from `visuals.widgets.active.bg_stroke`, which `Theme`
/// maps to the primary role. Draws nothing when the widget has no focus.
pub fn focus_ring(ui: &Ui, rect: Rect, radius: u8, response: &Response) {
    let focus_t = t(ui, response.id.with("focus"), response.has_focus(), HOVER);
    if focus_t <= 0.0 {
        return;
    }
    let color = ui.visuals().widgets.active.bg_stroke.color;
    let color = Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (color.a() as f32 * focus_t).round() as u8,
    );
    ui.painter()
        .rect_stroke(rect, radius, Stroke::new(2.0, color), StrokeKind::Inside);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{CentralPanel, Pos2, RawInput};

    fn raw_input(time: f64) -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0))),
            time: Some(time),
            ..RawInput::default()
        }
    }

    #[test]
    fn bezier_hits_endpoints_is_monotone_and_stays_in_range() {
        for (x1, y1, x2, y2) in [
            (0.2, 0.0, 0.0, 1.0),
            (0.0, 0.0, 0.0, 1.0),
            (0.3, 0.0, 1.0, 1.0),
        ] {
            assert_eq!(bezier(x1, y1, x2, y2, 0.0), 0.0);
            assert_eq!(bezier(x1, y1, x2, y2, 1.0), 1.0);
            let mut previous = 0.0;
            for step in 0..=32 {
                let value = bezier(x1, y1, x2, y2, step as f32 / 32.0);
                assert!((0.0..=1.0).contains(&value), "{value} out of range");
                assert!(value >= previous - 1e-5, "not monotone at step {step}");
                previous = value;
            }
        }
    }

    #[test]
    fn progress_clamps_and_eases_towards_one() {
        assert_eq!(progress(9.9, 10.0, 0.2), 0.0);
        assert_eq!(progress(10.2, 10.0, 0.2), 1.0);
        assert_eq!(progress(11.0, 10.0, 0.2), 1.0);
        assert_eq!(progress(20.0, 10.0, 0.0), 1.0, "zero duration snaps");
        let mid = progress(10.1, 10.0, 0.2);
        assert!(mid > 0.5 && mid < 1.0, "expected eased midpoint, got {mid}");
    }

    #[test]
    fn tints_go_from_nothing_to_hover_to_press() {
        let base = Color32::from_rgb(100, 100, 100);
        let idle = state_layer_at(base, 0.0, 0.0);
        let hover = state_layer_at(base, 1.0, 0.0);
        let press = state_layer_at(base, 1.0, 1.0);
        assert_eq!(idle, base);
        assert_eq!(hover.a(), 255);
        assert_eq!(press.a(), 255);
        assert!((111..=113).contains(&hover.r()), "hover {:?}", hover);
        assert!((118..=120).contains(&press.r()), "press {:?}", press);
        assert!(hover.r() < press.r());
    }

    #[test]
    fn press_and_selection_geometry_hit_their_endpoints() {
        assert_eq!(press_scale_at(0.0), 1.0);
        assert!((press_scale_at(1.0) - PRESS_SCALE).abs() < 1e-6);
        assert_eq!(nav_pill(0.0), Vec2::new(32.0, 28.0));
        assert_eq!(nav_pill(1.0), Vec2::new(56.0, 32.0));
        let halfway = nav_pill(0.5);
        assert!(halfway.x > 32.0 && halfway.x < 56.0);
        assert_eq!(chip_radius(0.0), 8.0);
        assert_eq!(chip_radius(1.0), 16.0);
    }

    #[test]
    fn reduced_motion_snaps_bool_animations_and_latches_the_flag() {
        let ctx = Context::default();
        let mut value = -1.0;
        // The first passes repaint for egui's own setup (window theme command and
        // widget-rect comparison against an empty pass); a static UI is quiet by
        // the fourth frame. That is exactly the idle behaviour motion must keep.
        for frame in 0..4 {
            let mut output = ctx.run_ui(raw_input(frame as f64 / 60.0), |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    set_reduced(ui.ctx(), true);
                    value = t(ui, Id::unique("reduced_test"), true, HOVER);
                });
            });
            output.textures_delta.clear();
        }
        assert_eq!(value, 1.0, "reduced motion must jump to the target");
        assert!(reduced(&ctx));
        let causes = ctx.repaint_causes();
        assert!(!ctx.has_requested_repaint(), "causes: {causes:?}");
    }

    #[test]
    fn finished_bool_animation_stops_requesting_repaints() {
        let ctx = Context::default();
        let mut value = -1.0;
        // 200 ms steps finish a 150 ms HOVER animation in the first move; the
        // remaining passes let egui's setup repaints drain.
        for frame in 0..5 {
            let mut output = ctx.run_ui(raw_input(frame as f64 * 0.2), |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    value = t(ui, Id::unique("idle_test"), true, HOVER);
                });
            });
            output.textures_delta.clear();
        }
        assert_eq!(value, 1.0);
        assert!(
            !ctx.has_requested_repaint(),
            "settled animation must not keep repainting: {:?}",
            ctx.repaint_causes()
        );
    }

    #[test]
    fn appear_fades_once_and_snaps_with_zero_duration() {
        let ctx = Context::default();
        let mut first = -1.0;
        let mut later = -1.0;
        let mut instant = -1.0;
        let mut output = ctx.run_ui(raw_input(0.0), |ctx| {
            first = appear(ctx, Id::unique("fade"), ENTER);
        });
        output.textures_delta.clear();
        output = ctx.run_ui(raw_input(0.1), |ctx| {
            later = appear(ctx, Id::unique("fade"), ENTER);
            instant = appear(ctx, Id::unique("instant"), 0.0);
        });
        output.textures_delta.clear();
        assert_eq!(first, 0.0);
        assert!(later > 0.0 && later < 1.0, "expected mid-fade, got {later}");
        assert_eq!(instant, 1.0);
    }
}
