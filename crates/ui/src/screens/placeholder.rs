//! Honest placeholders for destinations whose backend/screens land later.

use eframe::egui::{self, RichText, Stroke, Ui};

use crate::routes::Tab;
use crate::theme::{font_label, regular_font, Theme, DP24, DP8, SHAPE_LARGE};

pub fn coming_soon(ui: &mut Ui, app_theme: &Theme, tab: Tab) {
    let body = match tab {
        Tab::Together => "Listen together needs the sync server plus a shared queue.",
        Tab::Library => {
            "Signed-out library. Liked songs, playlists and history are stored locally as they land."
        }
        Tab::Search => "Search arrives with the InnerTube client.",
        Tab::Home => "Home is being prepared.",
    };
    stub(ui, app_theme, tab.label(), body, tab.icon());
    ui.add_space(DP24);
}

/// Generic centered "not yet" block used by route stubs.
pub fn stub(ui: &mut Ui, app_theme: &Theme, title: &str, body: &str, _icon: &str) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0, app_theme.scheme.outline_variant))
        .corner_radius(SHAPE_LARGE)
        .inner_margin(egui::Margin::same(DP24 as i8))
        .show(ui, |ui| {
            ui.label(
                RichText::new(title.to_uppercase())
                    .font(font_label(11.0))
                    .color(app_theme.scheme.on_surface_variant),
            );
            ui.add_space(DP8);
            ui.label(
                RichText::new(body)
                    .font(regular_font(13.0))
                    .color(app_theme.scheme.on_surface_variant),
            );
        });
}
