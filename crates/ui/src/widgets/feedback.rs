//! Snackbar/toast, modal shell and loading placeholders.

use std::hash::Hash;

use eframe::egui::{self, Align2, Color32, Id, Order, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use super::paint_icon;
use crate::theme::{
    medium_font, Theme, BODY_MEDIUM, DP12, DP16, DP20, DP4, DP48, DP8, SHAPE_EXTRA_LARGE,
};

/// Bottom inset that clears the compact mini player (64 dp) plus bottom nav
/// (72 dp) and their 8 dp insets.
///
/// shortcut: fixed inset; pass the measured bottom-panel height once
/// `App` hands it to [`Toasts::show`].
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

    pub fn show(&mut self, ctx: &egui::Context, app_theme: &Theme) {
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
        egui::Area::new(Id::unique("metrolist_toasts"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -TOAST_INSET))
            .order(Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                for toast in &self.items {
                    egui::Frame::new()
                        .fill(app_theme.scheme.inverse_surface)
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(DP16 as i8, DP12 as i8))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&toast.text)
                                    .font(medium_font(BODY_MEDIUM))
                                    .color(app_theme.scheme.inverse_on_surface),
                            );
                        });
                    ui.add_space(DP4);
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Centered M3 modal shell with a dim backdrop.
pub fn modal<R>(
    ctx: &egui::Context,
    app_theme: &Theme,
    id_salt: impl Hash + std::fmt::Debug,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::ModalResponse<R> {
    let frame = egui::Frame::new()
        .fill(app_theme.scheme.surface_container_high)
        .corner_radius(SHAPE_EXTRA_LARGE)
        .inner_margin(egui::Margin::same(DP20 as i8))
        .stroke(Stroke::new(1.0, app_theme.scheme.outline_variant));
    egui::Modal::new(Id::unique(id_salt))
        .backdrop_color(Color32::from_black_alpha(180))
        .frame(frame)
        .show(ctx, add_contents)
}

/// Animated loading block.
pub fn shimmer(ui: &mut Ui, app_theme: &Theme, size: Vec2, radius: u8) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let time = ui.input(|i| i.time) as f32;
    let pulse = 0.5 + 0.5 * (time * 1.6).sin();
    let fill = app_theme
        .scheme
        .surface_container
        .lerp_to_gamma(app_theme.scheme.surface_container_high, pulse * 0.7);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    let band_x = rect.left() - 30.0 + (rect.width() + 60.0) * ((time * 0.55) % 1.0);
    let band = Rect::from_min_size(
        Pos2::new(band_x, rect.top()),
        Vec2::new(26.0, rect.height()),
    );
    painter
        .with_clip_rect(rect)
        .rect_filled(band, radius, Color32::from_white_alpha(12));
}

/// Placeholder row used while a shelf loads (art + two text bars).
pub fn shimmer_row(ui: &mut Ui, app_theme: &Theme, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 64.0), Sense::hover());
    let art = Rect::from_min_size(rect.left_top() + Vec2::new(0.0, DP4), Vec2::splat(56.0));
    let painter = ui.painter();
    painter.rect_filled(art, 4, app_theme.scheme.surface_container_high);
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(art.right() + DP12, art.top() + DP8),
            Vec2::new((width * 0.45).clamp(40.0, 220.0), 12.0),
        ),
        4,
        app_theme.scheme.surface_container_high,
    );
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(art.right() + DP12, art.top() + 28.0),
            Vec2::new((width * 0.3).clamp(30.0, 160.0), 10.0),
        ),
        4,
        app_theme.scheme.surface_container,
    );
}

/// Empty-state block for "coming soon" and error screens.
pub fn empty_state(ui: &mut Ui, app_theme: &Theme, icon_name: &str, title: &str, body: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(DP48);
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
        ui.painter().circle_filled(
            icon_rect.center(),
            36.0,
            app_theme.scheme.surface_container_high,
        );
        paint_icon(
            ui.painter(),
            icon_rect.center(),
            icon_name,
            32.0,
            app_theme.scheme.on_surface_variant,
        );
        ui.add_space(DP16);
        ui.label(egui::RichText::new(title).font(crate::theme::bold_font(22.0)));
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
