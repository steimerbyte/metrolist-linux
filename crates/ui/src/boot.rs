//! Nothing-OS style boot sequence painted over the app's first frames.
//!
//! `BootApp` wraps the real app: the app draws first, then a full-screen boot
//! layer covers it until every module reported ready and the fade-out ended.
//! A key, click or touch during the sequence skips straight to the app.

use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, Context, Event, Id, Key, LayerId, Order, Pos2, Stroke, TouchPhase, Ui,
};

use crate::app::App;
use crate::theme;

/// Boot overlay palette, RGB only; alpha comes from the fade.
const BG: (u8, u8, u8) = (0x00, 0x00, 0x00);
const BORDER: (u8, u8, u8) = (0x22, 0x22, 0x22);
const BORDER_VISIBLE: (u8, u8, u8) = (0x33, 0x33, 0x33);
const TEXT: (u8, u8, u8) = (0xE8, 0xE8, 0xE8);
const TEXT_SECONDARY: (u8, u8, u8) = (0x99, 0x99, 0x99);
const SUCCESS: (u8, u8, u8) = (0x4A, 0x9E, 0x5C);
const ERROR: (u8, u8, u8) = (0xD7, 0x19, 0x21);

const MARGIN: f32 = 24.0;
const ROW_H: f32 = 26.0;

fn tint(rgb: (u8, u8, u8), alpha: f32) -> Color32 {
    let (r, g, b) = rgb;
    Color32::from_rgba_unmultiplied(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The real app plus the boot overlay that runs before it is shown.
pub struct BootApp {
    inner: App,
    boot: Boot,
}

impl BootApp {
    pub fn new(inner: App, enabled: bool) -> Self {
        Self {
            inner,
            boot: Boot::new(enabled),
        }
    }
}

impl eframe::App for BootApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.inner, ui, frame);
        self.boot.show(ui.ctx());
    }

    fn logic(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        eframe::App::logic(&mut self.inner, ctx, frame);
    }

    /// Swallow every event while the overlay is visible; one skip event
    /// accelerates the sequence instead of reaching the app underneath.
    fn raw_input_hook(&mut self, ctx: &Context, raw_input: &mut egui::RawInput) {
        if !self.boot.consumes_input() {
            eframe::App::raw_input_hook(&mut self.inner, ctx, raw_input);
            return;
        }
        let skip = raw_input.events.iter().any(|event| match event {
            Event::Key {
                key: Key::Enter,
                pressed: true,
                ..
            } => true,
            Event::PointerButton { pressed: true, .. } => true,
            Event::Touch {
                phase: TouchPhase::Start,
                ..
            } => true,
            _ => false,
        });
        if skip {
            self.boot.skip = true;
        }
        raw_input.events.clear();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::App::save(&mut self.inner, storage);
    }

    fn on_exit(&mut self) {
        eframe::App::on_exit(&mut self.inner);
    }

    fn auto_save_interval(&self) -> Duration {
        eframe::App::auto_save_interval(&self.inner)
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        eframe::App::clear_color(&self.inner, visuals)
    }

    fn persist_egui_memory(&self) -> bool {
        eframe::App::persist_egui_memory(&self.inner)
    }
}

struct Boot {
    enabled: bool,
    start: Option<f64>,
    logged: usize,
    /// Result of the most recently logged module; only the active row shows it.
    failed: bool,
    done: bool,
    skip: bool,
    checked: bool,
}

impl Boot {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            start: None,
            logged: 0,
            failed: false,
            done: false,
            skip: false,
            checked: false,
        }
    }

    fn consumes_input(&self) -> bool {
        self.enabled && !self.done
    }

    fn show(&mut self, ctx: &Context) {
        if self.done {
            return;
        }
        let now = ctx.input(|i| i.time);
        if !self.checked {
            self.checked = true;
            if crate::motion::reduced(ctx) {
                self.skip = true;
            }
        }
        let len = ml_boot::MODULES.len();
        if len == 0 {
            self.done = true;
            return;
        }
        // Batch modes skip the sequence but still report every module.
        if !self.enabled {
            self.log_ready(len);
            self.done = true;
            return;
        }
        if self.skip {
            self.start = Some(now - ml_boot::total(len) as f64);
        }
        let start = *self.start.get_or_insert(now);
        let elapsed = (now - start).max(0.0) as f32;
        let state = ml_boot::state_at(elapsed, len);
        let ready = if state.finished {
            len
        } else {
            match state.phase {
                ml_boot::Phase::Ok => state.index + 1,
                ml_boot::Phase::Loading => state.index,
            }
            .min(len)
        };
        self.log_ready(ready);
        if state.finished {
            self.done = true;
            return;
        }
        paint(ctx, &state, elapsed, self.failed);
        ctx.request_repaint();
    }

    /// Log every module up to `upto` exactly once; skipped frames jump ahead.
    fn log_ready(&mut self, upto: usize) {
        let upto = upto.min(ml_boot::MODULES.len());
        while self.logged < upto {
            let module = &ml_boot::MODULES[self.logged];
            let failed = ml_boot::run_init(module) == Some(false);
            println!(
                "BOOT module={} status={}",
                module.name,
                if failed { "fail" } else { "ok" }
            );
            self.failed = failed;
            self.logged += 1;
        }
    }
}

/// Pure painter pass: no widgets, no layout, nothing but the boot screen.
fn paint(ctx: &Context, state: &ml_boot::State, elapsed: f32, failed: bool) {
    let screen = ctx.content_rect();
    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("metrolist-boot")));
    let fade = (1.0 - state.exit_t).clamp(0.0, 1.0);
    p.rect_filled(screen, 0.0, tint(BG, fade));

    let left = screen.left() + MARGIN;
    let right = screen.right() - MARGIN;

    let mark = p.text(
        Pos2::new(left, screen.top() + MARGIN),
        Align2::LEFT_TOP,
        "METROLIST",
        theme::font_display(48.0),
        tint(TEXT, fade),
    );
    let label_font = theme::font_label(12.0);
    let sub = p.text(
        Pos2::new(left, mark.bottom() + 8.0),
        Align2::LEFT_TOP,
        "SYSTEM BOOT",
        label_font.clone(),
        tint(TEXT_SECONDARY, fade),
    );
    let len = ml_boot::MODULES.len();
    let shown = (state.index + 1).min(len);
    p.text(
        Pos2::new(right, sub.bottom()),
        Align2::RIGHT_BOTTOM,
        format!("{shown:02}/{len:02}"),
        label_font.clone(),
        tint(TEXT_SECONDARY, fade),
    );
    let head_bottom = sub.bottom() + 16.0;
    p.line_segment(
        [Pos2::new(left, head_bottom), Pos2::new(right, head_bottom)],
        Stroke::new(1.0, tint(BORDER_VISIBLE, fade)),
    );

    let y0 = head_bottom + 16.0;
    let last = state.index.min(len.saturating_sub(1));
    for i in 0..=last {
        let active = i == state.index;
        let t = if active {
            crate::motion::decelerate(state.row_t).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let row_top = y0 + i as f32 * ROW_H;
        let y = row_top + ROW_H / 2.0 + if active { 4.0 * (1.0 - t) } else { 0.0 };
        let alpha = fade * t;
        let done = i < state.index || (active && state.phase == ml_boot::Phase::Ok);
        let (status, status_color, label_color) = if active && failed && done {
            ("FAIL", tint(ERROR, alpha), tint(TEXT, alpha))
        } else if done {
            ("OK", tint(SUCCESS, alpha), tint(TEXT_SECONDARY, alpha))
        } else {
            ("LOADING", tint(TEXT_SECONDARY, alpha), tint(TEXT, alpha))
        };
        p.text(
            Pos2::new(left, y),
            Align2::LEFT_CENTER,
            ml_boot::MODULES[i].label,
            label_font.clone(),
            label_color,
        );
        p.text(
            Pos2::new(right, y),
            Align2::RIGHT_CENTER,
            status,
            label_font.clone(),
            status_color,
        );
        if i < last {
            let divider = row_top + ROW_H;
            p.line_segment(
                [Pos2::new(left, divider), Pos2::new(right, divider)],
                Stroke::new(1.0, tint(BORDER, fade)),
            );
        }
    }

    let track_y = screen.bottom() - MARGIN;
    p.line_segment(
        [Pos2::new(left, track_y), Pos2::new(right, track_y)],
        Stroke::new(1.0, tint(BORDER, fade)),
    );
    let done = (elapsed / (len as f32 * ml_boot::SLOT)).clamp(0.0, 1.0);
    if done > 0.0 {
        p.line_segment(
            [
                Pos2::new(left, track_y),
                Pos2::new(left + (right - left) * done, track_y),
            ],
            Stroke::new(1.0, tint(TEXT, fade)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{epaint::Shape, RawInput, Rect, Vec2};

    fn frame(ctx: &Context, boot: &mut Boot, t: f64) -> eframe::egui::FullOutput {
        let input = RawInput {
            time: Some(t),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(405.0, 900.0))),
            ..RawInput::default()
        };
        let mut output = ctx.run_pass(input, |ctx| boot.show(ctx));
        output.textures_delta.clear();
        output
    }

    #[test]
    fn boot_overlay_paints_and_skips() {
        let ctx = Context::default();
        crate::theme::install_fonts(&ctx);
        let mut boot = Boot::new(true);

        let first = frame(&ctx, &mut boot, 0.0);
        assert!(
            first.shapes.iter().any(|clipped| match &clipped.shape {
                Shape::Rect(rect) => {
                    rect.rect.area() >= 0.9 * 405.0 * 900.0 && rect.fill == Color32::BLACK
                }
                _ => false,
            }),
            "first frame paints an opaque black full-screen background"
        );

        for t in [0.10, 0.20] {
            let output = frame(&ctx, &mut boot, t);
            assert!(!boot.done, "sequence still runs at {t}");
            assert!(!output.shapes.is_empty(), "rows are painted at {t}");
        }

        let end = ml_boot::total(ml_boot::MODULES.len()) as f64 + 0.05;
        let output = frame(&ctx, &mut boot, end);
        assert!(boot.done, "sequence finishes after its total duration");
        assert_eq!(boot.logged, ml_boot::MODULES.len());
        assert!(output.shapes.is_empty(), "a finished overlay draws nothing");

        let mut disabled = Boot::new(false);
        let idle_ctx = Context::default();
        let output = frame(&idle_ctx, &mut disabled, 0.0);
        assert_eq!(disabled.logged, ml_boot::MODULES.len());
        assert!(output.shapes.is_empty(), "batch modes draw no overlay");
    }
}
