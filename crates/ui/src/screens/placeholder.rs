//! Honest placeholders for destinations whose backend/screens land later.

use eframe::egui::Ui;

use crate::routes::Tab;
use crate::theme::{Theme, DP24};
use crate::widgets::empty_state;

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
pub fn stub(ui: &mut Ui, app_theme: &Theme, title: &str, body: &str, icon: &str) {
    empty_state(ui, app_theme, icon, title, body);
}
