//! Library > Downloads: offline manifest, progress, storage budget and delete.

use eframe::egui::{self, Pos2, Rect, Sense, Ui, Vec2};
use ml_downloads::{Download, DownloadState};

use crate::art::ArtCache;
use crate::pure::{bytes_text, download_progress_text, storage_percent};
use crate::services::Services;
use crate::state::DownloadsState;
use crate::theme::{medium_font, regular_font, Theme, DP12, DP16, DP24, DP4, DP8};
use crate::widgets::{
    art_or_placeholder, elided, empty_state, filled_button, icon_button_at, outlined_button, text,
};

use super::Intent;

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    state: &mut DownloadsState,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let width = ui.available_width();
    ui.add_space(DP4);
    text(
        ui.painter(),
        ui.cursor().min,
        "Offline downloads",
        medium_font(18.0),
        theme.scheme.on_surface,
    );
    ui.add_space(DP24);

    if let Some(error) = &state.error {
        empty_state(ui, theme, "cloud_off", "Downloads unavailable", error);
        ui.add_space(DP24);
        return intents;
    }

    // Storage meter.
    let used = state.used;
    let limit = state.limit;
    let used_text = match limit {
        Some(limit) => format!("{} of {} used", bytes_text(used), bytes_text(limit)),
        None => format!("{} used · no limit", bytes_text(used)),
    };
    ui.label(
        egui::RichText::new(used_text)
            .font(regular_font(13.0))
            .color(theme.scheme.on_surface_variant),
    );
    if limit.is_some() {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, 3, theme.scheme.surface_container_high);
        if let Some(percent) = storage_percent(used, limit) {
            if percent > 0 {
                let filled = Rect::from_min_size(
                    rect.min,
                    Vec2::new(rect.width() * percent as f32 / 100.0, rect.height()),
                );
                ui.painter().rect_filled(filled, 3, theme.scheme.primary);
            }
        }
        ui.add_space(DP4);
    }
    ui.label(
        egui::RichText::new(format!("Folder: {}", services.download_dir().display()))
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
    );
    if let Some(quality) = services.download_quality() {
        ui.label(
            egui::RichText::new(format!(
                "New downloads use {quality:?} quality (launch setting)."
            ))
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
        );
    }
    let failed = state.failed_count();
    if failed > 0 {
        ui.add_space(DP4);
        if outlined_button(ui, theme, &format!("Clear failed ({failed})")).clicked() {
            intents.push(Intent::ClearFailedDownloads);
        }
    }
    ui.add_space(DP8);

    if state.items.is_empty() {
        empty_state(
            ui,
            theme,
            "download",
            "No downloads yet",
            "Use the download icon on any song, album or playlist to keep it offline.",
        );
        ui.add_space(DP24);
        return intents;
    }

    let items = state.items.clone();
    for entry in &items {
        intents.extend(download_row(ui, theme, services, art, entry, width, state));
        ui.add_space(DP4);
    }
    ui.add_space(DP12);

    if let Some(entry) = state.confirm_remove.clone() {
        let response = crate::widgets::modal(ui.ctx(), theme, "download_remove", |ui| {
            ui.set_width(320.0);
            ui.label(
                egui::RichText::new("Remove download?")
                    .font(medium_font(18.0))
                    .color(theme.scheme.on_surface),
            );
            ui.add_space(DP8);
            ui.label(
                egui::RichText::new(format!(
                    "“{}” will be deleted from disk. It can be downloaded again later.",
                    entry.song.title
                ))
                .font(regular_font(14.0))
                .color(theme.scheme.on_surface_variant),
            );
            ui.add_space(DP16);
            let mut choice = None;
            ui.horizontal(|ui| {
                if outlined_button(ui, theme, "Cancel").clicked() {
                    choice = Some(false);
                }
                ui.add_space(DP8);
                if filled_button(ui, theme, Some("delete"), "Delete").clicked() {
                    choice = Some(true);
                }
            });
            choice
        });
        match response.inner {
            Some(true) => {
                intents.push(Intent::RemoveDownload(entry.id.clone()));
                state.confirm_remove = None;
            }
            Some(false) => state.confirm_remove = None,
            None if response.backdrop_response.clicked() => state.confirm_remove = None,
            None => {}
        }
    }

    intents
}

fn download_row(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    entry: &Download,
    width: f32,
    state: &mut DownloadsState,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let actions: &[(&str, &str, u8)] = match entry.state {
        DownloadState::Queued | DownloadState::Downloading => &[("close", "Cancel download", 0)],
        DownloadState::Failed | DownloadState::Cancelled => &[
            ("refresh", "Retry download", 1),
            ("delete", "Delete download", 2),
        ],
        DownloadState::Completed => &[("delete", "Delete download", 2)],
    };
    let action_count = actions.len() as f32;
    let text_width = (width - 56.0 - 12.0 - action_count * 44.0).max(60.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 84.0), Sense::hover());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 8, theme.scheme.surface_container_high);
    }
    let art_rect = Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 8.0), Vec2::splat(64.0));
    let url = entry.song.thumb(160);
    let texture = art.texture(ui.ctx(), services, &url);
    art_or_placeholder(ui, theme, texture.as_ref(), art_rect, 6);

    let text_x = art_rect.right() + 12.0;
    let text_top = rect.top() + 14.0;
    elided(
        ui.painter(),
        Pos2::new(text_x, text_top),
        &entry.song.title,
        medium_font(15.0),
        theme.scheme.on_surface,
        text_width,
    );
    elided(
        ui.painter(),
        Pos2::new(text_x, text_top + 21.0),
        &entry.song.artist_names(),
        regular_font(12.0),
        theme.scheme.on_surface_variant,
        text_width,
    );
    let (status, color) = match entry.state {
        DownloadState::Queued => ("Queued · 0%".to_string(), theme.scheme.on_surface_variant),
        DownloadState::Downloading => (
            format!(
                "Downloading · {}",
                download_progress_text(
                    entry.bytes_downloaded,
                    entry.total_bytes.unwrap_or(entry.size_bytes)
                )
            ),
            theme.scheme.primary,
        ),
        DownloadState::Completed => (
            format!("Downloaded · 100% · {}", bytes_text(entry.size_bytes)),
            theme.scheme.primary,
        ),
        DownloadState::Failed => (
            format!(
                "Failed · {}",
                entry.error.as_deref().unwrap_or("unknown error")
            ),
            theme.scheme.error,
        ),
        DownloadState::Cancelled => (
            "Cancelled · 0%".to_string(),
            theme.scheme.on_surface_variant,
        ),
    };
    elided(
        ui.painter(),
        Pos2::new(text_x, text_top + 40.0),
        &status,
        regular_font(12.0),
        color,
        text_width,
    );
    if entry.state == DownloadState::Downloading {
        let bar = Rect::from_min_size(
            Pos2::new(text_x, text_top + 55.0),
            Vec2::new(text_width, 3.0),
        );
        ui.painter()
            .rect_filled(bar, 2, theme.scheme.surface_container_high);
        let percent = entry.percent().min(100) as f32;
        ui.painter().rect_filled(
            Rect::from_min_size(
                bar.min,
                Vec2::new(bar.width() * percent / 100.0, bar.height()),
            ),
            2,
            theme.scheme.primary,
        );
    }

    for (index, (icon, label, kind)) in actions.iter().enumerate() {
        let center = Pos2::new(rect.right() - 22.0 - index as f32 * 40.0, rect.center().y);
        let action = icon_button_at(
            ui,
            theme,
            ("download_row_action", entry.id.as_str(), *kind),
            Rect::from_center_size(center, Vec2::splat(40.0)),
            icon,
            label,
            20.0,
        );
        if action.clicked() {
            match kind {
                0 => intents.push(Intent::CancelDownload(entry.id.clone())),
                1 => intents.push(Intent::RetryDownload(entry.id.clone())),
                _ => state.confirm_remove = Some(entry.clone()),
            }
        }
    }
    intents
}
