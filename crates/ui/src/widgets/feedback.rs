//! Snackbar/toast, modal shell and loading placeholders.

use std::hash::Hash;

use eframe::egui::{self, Align2, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::paint_icon;
use crate::theme::{font_label, Theme, DP12, DP16, DP20, DP24, DP4, DP48, DP8, SHAPE_LARGE};

/// Dot coverage of a loading placeholder; dense enough to read as a block.
const SKELETON_STRENGTH: f32 = 0.28;

/// Bottom inset that clears the compact mini player (64 dp) plus bottom nav
/// (72 dp) and their 8 dp insets. The full player has neither, so its toasts
/// sit just above the bottom edge instead.
const TOAST_INSET: f32 = 152.0;

/// One transient snackbar message.
#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub remaining: f32,
}

/// Toast queue drawn as a foreground overlay above the nav bar.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, text: impl Into<String>) {
        let text = text.into();
        // Repeated messages (one per failed provider, say) refresh the visible
        // toast instead of stacking copies over the mini player.
        if let Some(existing) = self.items.iter_mut().find(|toast| toast.text == text) {
            existing.remaining = 2.8;
            return;
        }
        self.items.push(Toast {
            text,
            remaining: 2.8,
        });
        if self.items.len() > 3 {
            self.items.remove(0);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn show(&mut self, ctx: &egui::Context, app_theme: &Theme, player_open: bool) {
        if self.items.is_empty() {
            return;
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        for toast in &mut self.items {
            toast.remaining -= dt;
        }
        self.items.retain(|toast| toast.remaining > 0.0);
        if self.items.is_empty() {
            return;
        }
        // Derived from the existing timer, so entering/exiting needs no state:
        // fade in over the first `SELECT`, out over the last `EXIT` seconds.
        let fade_in =
            |toast: &Toast| ((2.8 - toast.remaining) / crate::motion::SELECT).clamp(0.0, 1.0);
        let fade_out = |toast: &Toast| (toast.remaining / crate::motion::EXIT).clamp(0.0, 1.0);
        let entering = self
            .items
            .iter()
            .map(|toast| 1.0 - fade_in(toast))
            .fold(0.0_f32, f32::max);
        let animating = self
            .items
            .iter()
            .any(|toast| fade_in(toast) < 1.0 || fade_out(toast) < 1.0);
        let inset = if player_open { DP24 } else { TOAST_INSET };
        egui::Area::new(Id::unique("metrolist_toasts"))
            .anchor(
                Align2::CENTER_BOTTOM,
                Vec2::new(0.0, -inset + crate::motion::TOAST_RISE * entering),
            )
            .order(Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                for toast in &self.items {
                    ui.scope(|ui| {
                        ui.set_opacity(fade_in(toast).min(fade_out(toast)));
                        egui::Frame::new()
                            .fill(app_theme.scheme.surface_container)
                            .stroke(app_theme.hairline())
                            .inner_margin(egui::Margin::symmetric(DP16 as i8, DP12 as i8))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(toast.text.to_uppercase())
                                        .font(font_label(11.0))
                                        .color(app_theme.scheme.on_surface),
                                );
                            });
                    });
                    ui.add_space(DP4);
                }
            });
        if animating {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

/// Centered modal shell with a dim backdrop. `progress` is 0 on the opening
/// frame and 1 once settled; closing is instant (the caller stops drawing).
pub fn modal<R>(
    ctx: &egui::Context,
    app_theme: &Theme,
    id_salt: impl Hash + std::fmt::Debug,
    progress: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::ModalResponse<R> {
    let frame = egui::Frame::new()
        .fill(app_theme.card())
        .corner_radius(SHAPE_LARGE)
        .inner_margin(egui::Margin::same(DP20 as i8))
        .stroke(app_theme.hairline());
    let id = Id::unique(id_salt);
    let rise = crate::motion::MODAL_RISE * (1.0 - progress);
    let scrim = app_theme
        .scheme
        .scrim
        .gamma_multiply(progress.clamp(0.0, 1.0) * 150.0 / 255.0);
    egui::Modal::new(id)
        .backdrop_color(scrim)
        .frame(frame)
        .area(egui::Modal::default_area(id).anchor(Align2::CENTER_CENTER, Vec2::new(0.0, rise)))
        .show(ctx, |ui| {
            ui.set_opacity(progress);
            add_contents(ui)
        })
}

/// Animated loading block: a flat pulse between two surface tiers.
pub fn shimmer(ui: &mut Ui, app_theme: &Theme, size: Vec2, radius: u8) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let fade = crate::motion::appear(ui.ctx(), Id::unique("shimmer_fade"), crate::motion::SELECT);
    let mut painter = ui.painter().clone();
    painter.set_opacity(fade);
    crate::widgets::dither_rect(&painter, rect, SKELETON_STRENGTH);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, app_theme.scheme.outline_variant),
        StrokeKind::Inside,
    );
}

/// Placeholder row used while a shelf loads (art + two text bars).
pub fn shimmer_row(ui: &mut Ui, app_theme: &Theme, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 64.0), Sense::hover());
    let fade = crate::motion::appear(
        ui.ctx(),
        Id::unique("shimmer_row_fade"),
        crate::motion::SELECT,
    );
    let mut painter = ui.painter().clone();
    painter.set_opacity(fade);
    let time = ui.input(|i| i.time) as f32;
    let quiet = crate::motion::reduced(ui.ctx());
    let pulse = if quiet {
        0.5
    } else {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
        0.5 + 0.5 * (time * 1.6).sin()
    };
    let high = app_theme
        .scheme
        .surface_container_high
        .lerp_to_gamma(app_theme.scheme.surface_container_highest, pulse);
    let low = app_theme
        .scheme
        .surface_container_high
        .lerp_to_gamma(app_theme.scheme.surface_container_highest, pulse * 0.6);
    let art = Rect::from_min_size(rect.left_top() + Vec2::new(0.0, DP4), Vec2::splat(56.0));
    painter.rect_filled(art, 0, high);
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(art.right() + DP12, art.top() + DP8),
            Vec2::new((width * 0.45).clamp(40.0, 220.0), 12.0),
        ),
        0,
        high,
    );
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(art.right() + DP12, art.top() + 28.0),
            Vec2::new((width * 0.3).clamp(30.0, 160.0), 10.0),
        ),
        0,
        low,
    );
}

/// Empty-state block for "coming soon" and error screens.
pub fn empty_state(ui: &mut Ui, app_theme: &Theme, icon_name: &str, title: &str, body: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(DP48);
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
        paint_icon(
            ui.painter(),
            icon_rect.center(),
            icon_name,
            28.0,
            app_theme.scheme.on_surface_variant,
        );
        ui.add_space(DP16);
        ui.label(
            egui::RichText::new(title.to_uppercase())
                .font(font_label(12.0))
                .color(app_theme.scheme.on_surface),
        );
        ui.add_space(DP4);
        ui.label(
            egui::RichText::new(body)
                .font(crate::theme::regular_font(14.0))
                .color(app_theme.scheme.on_surface_variant),
        );
        ui.add_space(DP8);
        ui.allocate_space(Vec2::ZERO);
    });
}
