//! Queue overlay, driven by the playback controller's current queue.

use eframe::egui::{self, Align, Layout, Pos2, Rect, Sense, Ui, Vec2};

use crate::pure::{download_click, download_hint, download_icon, DownloadBadge, DownloadClick};
use crate::services::Services;
use crate::theme::{bold_font, medium_font, regular_font, state_layer, Theme, DP12, DP4};
use crate::widgets::{icon_button_at, outlined_button, paint_icon, playing_bars, text, text_width};
use ml_core::{format_duration_opt, Song};

#[derive(Debug, Clone, PartialEq)]
pub enum QueueAction {
    None,
    Play(usize),
    Move { from: usize, to: usize },
    Remove(usize),
    LoadMore,
    Download { song: Song, action: DownloadClick },
    Close,
}

/// `songs` are in playback order; `position` indexes into it.
pub fn show_modal(
    ctx: &egui::Context,
    app_theme: &Theme,
    services: &Services,
    songs: &[Song],
    position: Option<usize>,
) -> QueueAction {
    let mut action = QueueAction::None;
    let response = crate::widgets::modal(ctx, app_theme, "queue_modal", |ui| {
        // Modal frame adds 20 dp margin + 1 px stroke per side. Size from the
        // window so a 405 px viewport keeps 16 px gutters instead of using the
        // narrower content rect egui reports inside the modal.
        let width = (ui.ctx().viewport_rect().width() - 74.0).clamp(240.0, 360.0);
        ui.set_width(width);
        let (header, _) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::hover());
        text(
            ui.painter(),
            header.left_top() + Vec2::new(2.0, 5.0),
            "Queue",
            bold_font(crate::theme::TITLE_LARGE),
            app_theme.scheme.on_surface,
        );
        let count = format!("{} tracks", songs.len());
        let count_width = text_width(ui.painter(), &count, regular_font(12.0));
        text(
            ui.painter(),
            Pos2::new(header.right() - 48.0 - count_width, header.center().y - 8.0),
            &count,
            regular_font(12.0),
            app_theme.scheme.on_surface_variant,
        );
        let close = icon_button_at(
            ui,
            app_theme,
            "queue_close",
            Rect::from_center_size(
                Pos2::new(header.right() - 20.0, header.center().y),
                Vec2::splat(40.0),
            ),
            "close",
            "Close queue",
            22.0,
        );
        if close.clicked() {
            action = QueueAction::Close;
        }
        ui.add_space(DP4);

        // Explicit height: an auto-sized modal area otherwise reports a tiny
        // available height to the scroll area and clips the list to one row.
        ui.allocate_ui_with_layout(
            Vec2::new(width, 380.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(width);
                egui::ScrollArea::vertical()
                    .id_salt("queue_scroll")
                    .max_height(380.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        queue_rows(ui, app_theme, services, songs, position, &mut action);
                    });
            },
        );

        if songs.is_empty() {
            ui.add_space(DP12);
            ui.label(
                egui::RichText::new("The queue is empty.")
                    .font(regular_font(14.0))
                    .color(app_theme.scheme.on_surface_variant),
            );
        } else if services.radio_can_extend()
            && outlined_button(ui, app_theme, "Load more").clicked()
        {
            action = QueueAction::LoadMore;
        }
        action
    });
    if response.backdrop_response.clicked() {
        return QueueAction::Close;
    }
    response.inner
}

fn queue_rows(
    ui: &mut Ui,
    app_theme: &Theme,
    services: &Services,
    songs: &[Song],
    position: Option<usize>,
    action: &mut QueueAction,
) {
    for (index, song) in songs.iter().enumerate() {
        let (rect, row) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::click());
        let active = position == Some(index);
        if active {
            ui.painter()
                .rect_filled(rect, 8, app_theme.scheme.surface_container_high);
        } else if row.hovered() {
            ui.painter().rect_filled(
                rect,
                8,
                state_layer(
                    app_theme.bg(),
                    row.hovered(),
                    row.is_pointer_button_down_on(),
                ),
            );
        }
        if active {
            playing_bars(
                ui,
                app_theme,
                Rect::from_center_size(
                    Pos2::new(rect.left() + 20.0, rect.center().y),
                    Vec2::splat(20.0),
                ),
            );
        } else {
            let number = format!("{}", index + 1);
            let number_width = text_width(ui.painter(), &number, regular_font(12.0));
            text(
                ui.painter(),
                Pos2::new(
                    rect.left() + 20.0 - number_width / 2.0,
                    rect.center().y - 8.0,
                ),
                &number,
                regular_font(12.0),
                app_theme.scheme.outline,
            );
        }

        let text_x = rect.left() + 42.0;
        let action_x = rect.right() - 102.0;
        let text_width = (action_x - 8.0 - text_x).max(80.0);
        crate::widgets::elided(
            ui.painter(),
            Pos2::new(text_x, rect.top() + 10.0),
            &song.title,
            medium_font(14.0),
            app_theme.scheme.on_surface,
            text_width,
        );
        let subtitle = match song.duration {
            Some(duration) => format!(
                "{} · {}",
                song.artist_names(),
                format_duration_opt(Some(duration))
            ),
            None => song.artist_names(),
        };
        crate::widgets::elided(
            ui.painter(),
            Pos2::new(text_x, rect.top() + 28.0),
            &subtitle,
            regular_font(12.0),
            app_theme.scheme.on_surface_variant,
            text_width,
        );

        let icon = |name: &str, key: &str, x: f32, label: &str, color| {
            let rect = Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::splat(32.0));
            let response = ui.interact(rect, row.id.with(key), Sense::click());
            paint_icon(ui.painter(), rect.center(), name, 17.0, color);
            let response = response.on_hover_text(label);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
            response
        };
        let muted = app_theme.scheme.on_surface_variant;
        let badge = services.download_badge(&song.id);
        let badge_color = match badge {
            DownloadBadge::Completed => app_theme.scheme.primary,
            DownloadBadge::Failed => app_theme.scheme.error,
            _ => muted,
        };
        let download = icon(
            download_icon(badge),
            "queue_download",
            rect.right() - 18.0,
            download_hint(badge),
            badge_color,
        );
        let remove = icon(
            "delete",
            "queue_remove",
            rect.right() - 46.0,
            "Remove from queue",
            muted,
        );
        let up = icon(
            "keyboard_arrow_up",
            "queue_up",
            rect.right() - 102.0,
            "Move up",
            if index > 0 {
                muted
            } else {
                app_theme.scheme.outline
            },
        );
        let down = icon(
            "keyboard_arrow_down",
            "queue_down",
            rect.right() - 74.0,
            "Move down",
            if index + 1 < songs.len() {
                muted
            } else {
                app_theme.scheme.outline
            },
        );
        row.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                true,
                format!("Play {} by {}", song.title, song.artist_names()),
            )
        });
        if download.clicked() {
            *action = QueueAction::Download {
                song: song.clone(),
                action: download_click(badge),
            };
        } else if up.clicked() && index > 0 {
            *action = QueueAction::Move {
                from: index,
                to: index - 1,
            };
        } else if down.clicked() && index + 1 < songs.len() {
            *action = QueueAction::Move {
                from: index,
                to: index + 1,
            };
        } else if remove.clicked() {
            *action = QueueAction::Remove(index);
        } else if row.clicked() {
            *action = QueueAction::Play(index);
        }
    }
}
