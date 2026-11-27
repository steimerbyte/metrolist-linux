//! Queue overlay, driven by the playback controller's current queue.

use eframe::egui::{self, Align, Layout, Pos2, Rect, Sense, Ui, Vec2};

use crate::art::ArtCache;
use crate::pure::{download_click, download_hint, download_icon, DownloadBadge, DownloadClick};
use crate::services::Services;
use crate::theme::{
    font_label, medium_font, regular_font, Theme, DP12, DP4, HEADLINE_SMALL, LABEL_SMALL, TOUCH,
};
use crate::widgets::{art_or_placeholder, icon_button_at, outlined_button, paint_icon, text, text_width};
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
    art: &mut ArtCache,
    songs: &[Song],
    position: Option<usize>,
    progress: f32,
) -> QueueAction {
    let mut action = QueueAction::None;
    let response = crate::widgets::modal(ctx, app_theme, "queue_modal", progress, |ui| {
        // Modal frame adds 20 dp margin + 1 px stroke per side. Size from the
        // window so a 405 px viewport keeps 16 px gutters instead of using the
        // narrower content rect egui reports inside the modal.
        let width = (ui.ctx().viewport_rect().width() - 74.0).clamp(240.0, 360.0);
        ui.set_width(width);
        let (header, _) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::hover());
        text(
            ui.painter(),
            Pos2::new(header.left() + 2.0, header.center().y - 14.0),
            "Queue",
            medium_font(HEADLINE_SMALL),
            app_theme.scheme.on_surface,
        );
        let count = format!("{} TRACKS", songs.len());
        let count_width = text_width(ui.painter(), &count, font_label(LABEL_SMALL));
        text(
            ui.painter(),
            Pos2::new(header.right() - 56.0 - count_width, header.center().y - 6.0),
            &count,
            font_label(LABEL_SMALL),
            app_theme.scheme.on_surface_variant,
        );
        let close = icon_button_at(
            ui,
            app_theme,
            "queue_close",
            Rect::from_center_size(
                Pos2::new(header.right() - 24.0, header.center().y),
                Vec2::splat(TOUCH),
            ),
            "close",
            "Close queue",
            22.0,
        );
        if close.clicked() {
            action = QueueAction::Close;
        }
        ui.add_space(DP4);

        // Size to the rows instead of a fixed 380 dp box: a three-track queue
        // otherwise leaves most of the modal as empty surface. Long queues cap
        // at 380 dp and keep scrolling.
        let body_height = queue_body_height(songs.len());
        ui.allocate_ui_with_layout(
            Vec2::new(width, body_height),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(width);
                egui::ScrollArea::vertical()
                    .id_salt("queue_scroll")
                    .max_height(body_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        queue_rows(
                            ui,
                            app_theme,
                            services,
                            art,
                            songs,
                            position,
                            &mut action,
                        );
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

/// Modal body height: one 56 dp row per track, at least one row, capped at
/// 380 dp so long queues scroll instead of growing the modal.
fn queue_body_height(rows: usize) -> f32 {
    (rows.max(1) as f32 * ROW_HEIGHT).min(MAX_BODY_HEIGHT)
}

/// 32 dp hit targets on a 36 dp pitch: 4 dp between neighbouring actions.
const ACTION_SIZE: f32 = 32.0;
const ACTION_PITCH: f32 = 36.0;
const ROW_HEIGHT: f32 = 56.0;
const MAX_BODY_HEIGHT: f32 = 380.0;

/// Action centres for `[move up, move down, remove, download]`.
fn queue_action_centers(right: f32) -> [f32; 4] {
    [
        right - ACTION_PITCH * 3.0 - 20.0,
        right - ACTION_PITCH * 2.0 - 20.0,
        right - ACTION_PITCH - 20.0,
        right - 20.0,
    ]
}

fn queue_rows(
    ui: &mut Ui,
    app_theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    songs: &[Song],
    position: Option<usize>,
    action: &mut QueueAction,
) {
    for (index, song) in songs.iter().enumerate() {
        let (rect, row) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
        let active = position == Some(index);
        if row.hovered() {
            ui.painter()
                .rect_filled(rect, 0, app_theme.scheme.surface_container_high);
        }
        if active {
            ui.painter().rect_filled(
                Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())),
                0,
                app_theme.scheme.on_surface,
            );
        }
        // 1 px row rule plus a hard 2 px selection line: no pills, no radii.
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            app_theme.hairline(),
        );
        // The cover replaces the old number column: one 40 dp slot keeps the
        // row at 56 dp and leaves the title room. The 2 px selection line stays.
        let cover = Rect::from_min_size(
            Pos2::new(rect.left() + 4.0, rect.center().y - 20.0),
            Vec2::splat(40.0),
        );
        let texture = art.texture(
            ui.ctx(),
            services,
            song.thumbnail_url.as_deref().unwrap_or(""),
        );
        art_or_placeholder(ui, app_theme, texture.as_ref(), cover, 3);

        let text_x = rect.left() + 52.0;
        let [up_x, down_x, remove_x, download_x] = queue_action_centers(rect.right());
        let action_x = up_x;
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
            Pos2::new(text_x, rect.top() + ROW_HEIGHT / 2.0),
            &subtitle,
            regular_font(12.0),
            app_theme.scheme.on_surface_variant,
            text_width,
        );

        let icon = |name: &str, key: &str, x: f32, label: &str, color| {
            let rect =
                Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::splat(ACTION_SIZE));
            let response = ui.interact(rect, row.id.with(key), Sense::click());
            paint_icon(ui.painter(), rect.center(), name, 17.0, color);
            let response = response.on_hover_text(label);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
            response
        };
        let muted = app_theme.scheme.on_surface_variant;
        let disabled = app_theme.scheme.on_surface.gamma_multiply(0.38);
        let badge = services.download_badge(&song.id);
        let badge_color = match badge {
            DownloadBadge::Completed => app_theme.scheme.success,
            // Several failed rows can coexist, so this stays a warning tone:
            // red is reserved for the single interrupt per screen.
            DownloadBadge::Failed => app_theme.scheme.warning,
            _ => muted,
        };
        let download = icon(
            download_icon(badge),
            "queue_download",
            download_x,
            download_hint(badge),
            badge_color,
        );
        let remove = icon(
            "delete",
            "queue_remove",
            remove_x,
            "Remove from queue",
            muted,
        );
        let up = icon(
            "keyboard_arrow_up",
            "queue_up",
            up_x,
            "Move up",
            if index > 0 { muted } else { disabled },
        );
        let down = icon(
            "keyboard_arrow_down",
            "queue_down",
            down_x,
            "Move down",
            if index + 1 < songs.len() {
                muted
            } else {
                disabled
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
        } else if up.clicked() {
            // Consume the click even at the first row so the row underneath
            // does not start playback.
            if index > 0 {
                *action = QueueAction::Move {
                    from: index,
                    to: index - 1,
                };
            }
        } else if down.clicked() {
            if index + 1 < songs.len() {
                *action = QueueAction::Move {
                    from: index,
                    to: index + 1,
                };
            }
        } else if remove.clicked() {
            *action = QueueAction::Remove(index);
        } else if row.clicked() {
            *action = QueueAction::Play(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_body_height_follows_row_count_and_caps() {
        assert_eq!(queue_body_height(0), 56.0);
        assert_eq!(queue_body_height(1), 56.0);
        assert_eq!(queue_body_height(3), 168.0);
        assert_eq!(queue_body_height(6), 336.0);
        assert_eq!(queue_body_height(7), 380.0);
        assert_eq!(queue_body_height(50), 380.0);
    }

    #[test]
    fn queue_action_hit_rects_are_disjoint_and_inside_the_row() {
        for width in [240.0, 331.0, 360.0] {
            let row = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, ROW_HEIGHT));
            let rects = queue_action_centers(row.right()).map(|x| {
                Rect::from_center_size(Pos2::new(x, row.center().y), Vec2::splat(ACTION_SIZE))
            });
            for (index, a) in rects.iter().enumerate() {
                assert!(
                    row.contains(a.min) && row.contains(a.max),
                    "action {index} {a:?} escapes row {row:?}"
                );
                for b in rects.iter().skip(index + 1) {
                    assert!(
                        a.max.x + 0.01 < b.min.x || b.max.x + 0.01 < a.min.x,
                        "action {a:?} overlaps {b:?}"
                    );
                }
            }
        }
    }
}
