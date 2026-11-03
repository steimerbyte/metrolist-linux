//! List rows and album cards.

use eframe::egui::{
    self, Color32, CursorIcon, Pos2, Rect, Response, Sense, Shape, Stroke, TextureHandle, Ui, Vec2,
};
use egui::{StrokeKind, WidgetInfo};

use super::{art_or_placeholder, card_hover_stroke, elided, paint_icon, text_width};
use crate::motion;
use crate::pure::{download_click, download_hint, download_icon, DownloadBadge, DownloadClick};
use crate::theme::{medium_font, regular_font, Theme, DP12, DP4, DP8};
use ml_core::Song;

/// Result of [`song_row`]: the row click plus its download affordance.
#[derive(Debug)]
pub struct SongRowResponse {
    pub row: Response,
    /// Downloads icon interaction. Check this before `row.clicked()` so a
    /// download click never also starts playback.
    pub download: Response,
    pub download_action: DownloadClick,
    /// Radio icon interaction; `None` on narrow rows that hide it.
    pub radio: Option<Response>,
    /// Artist-name text interaction; `None` when no artist carries an id.
    /// Check this before `row` so the link never also starts playback.
    pub artist: Option<Response>,
}

/// Per-row affordances: download badge and whether the radio action is shown.
#[derive(Debug, Clone, Copy)]
pub struct SongRowOptions {
    pub download: DownloadBadge,
    pub radio: bool,
}

/// Song row: 56 dp art, title, artist · duration, download and radio actions.
pub fn song_row(
    ui: &mut Ui,
    app_theme: &Theme,
    song: &Song,
    art: Option<&TextureHandle>,
    width: f32,
    active: bool,
    options: SongRowOptions,
) -> SongRowResponse {
    let SongRowOptions {
        download,
        radio: show_radio,
    } = options;
    let width = width.max(120.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 64.0), Sense::click());
    let (hover_t, press_t) = motion::interaction(ui, &response);
    let active_t = motion::t(ui, response.id.with("active"), active, motion::HOVER);
    if active_t > 0.0 || hover_t > 0.0 || press_t > 0.0 {
        let base = app_theme
            .bg()
            .lerp_to_gamma(app_theme.scheme.surface_container_high, active_t);
        let fill = motion::state_layer_at(base, hover_t, press_t);
        ui.painter().rect_filled(rect, 8, fill);
    }
    let art_rect = Rect::from_min_size(rect.left_top() + Vec2::new(0.0, DP4), Vec2::splat(56.0));
    art_or_placeholder(ui, app_theme, art, art_rect, 4);

    let text_x = art_rect.right() + DP12;
    // 40 dp targets 40 dp apart: the radio and download rects touch but never
    // overlap, so a click in the gap cannot hit the neighbouring action.
    let download_x = rect.right() - 24.0;
    let radio_x = download_x - 40.0;
    let action_edge = (if show_radio { radio_x } else { download_x }) - 20.0;
    let max_text_width = (action_edge - DP12 - text_x).max(8.0);

    elided(
        ui.painter(),
        Pos2::new(text_x, art_rect.top() + DP8),
        &song.title,
        medium_font(15.0),
        app_theme.scheme.on_surface,
        max_text_width,
    );
    let artist = song.artist_names();
    let subtitle = if artist.is_empty() {
        song.album
            .as_ref()
            .map(|album| album.title.clone())
            .unwrap_or_default()
    } else if song.duration.is_some() {
        format!("{artist} · {}", song.duration_text())
    } else {
        artist
    };
    elided(
        ui.painter(),
        Pos2::new(text_x, art_rect.top() + 29.0),
        &subtitle,
        regular_font(13.0),
        app_theme.scheme.on_surface_variant,
        max_text_width,
    );
    let linked_artist = song
        .artists
        .iter()
        .find(|artist| artist.id.as_deref().is_some_and(|id| !id.is_empty()));
    let artist_response = linked_artist.map(|artist| {
        let names = song.artist_names();
        let width = text_width(ui.painter(), &names, regular_font(13.0)).min(max_text_width);
        let rect = Rect::from_min_size(
            Pos2::new(text_x, art_rect.top() + 27.0),
            Vec2::new(width.max(1.0), 18.0),
        );
        let response = ui.interact(rect, response.id.with("artist"), Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            ui.painter().line_segment(
                [
                    Pos2::new(rect.left(), rect.bottom()),
                    Pos2::new(rect.right(), rect.bottom()),
                ],
                Stroke::new(1.0, app_theme.scheme.primary),
            );
        }
        response.widget_info(|| {
            WidgetInfo::labeled(
                egui::Role::Button,
                true,
                format!("Open artist {}", artist.name),
            )
        });
        response
    });
    let download_id = response.id.with("download");
    let download_response = ui.interact(
        Rect::from_center_size(Pos2::new(download_x, rect.center().y), Vec2::splat(40.0)),
        download_id,
        Sense::click(),
    );
    let color = match download {
        DownloadBadge::Completed => app_theme.scheme.primary,
        DownloadBadge::Failed => app_theme.scheme.error,
        _ => app_theme.scheme.on_surface_variant,
    };
    paint_icon(
        ui.painter(),
        Pos2::new(download_x, rect.center().y),
        download_icon(download),
        20.0,
        color,
    );
    if let DownloadBadge::Downloading(percent) = download {
        let center = Pos2::new(download_x, rect.center().y);
        let radius = 12.0;
        let points = 16;
        let sweep = (percent.min(100) as f32 / 100.0) * std::f32::consts::TAU;
        let arc: Vec<Pos2> = (0..=points)
            .map(|step| {
                let angle = -std::f32::consts::FRAC_PI_2 + sweep * step as f32 / points as f32;
                center + Vec2::angled(angle) * radius
            })
            .collect();
        ui.painter().add(egui::Shape::line(
            arc,
            Stroke::new(2.0, app_theme.scheme.primary),
        ));
    }
    let download_action = download_click(download);
    let download_response = download_response.on_hover_text(download_hint(download));
    let radio_response = show_radio.then(|| {
        let response = ui.interact(
            Rect::from_center_size(Pos2::new(radio_x, rect.center().y), Vec2::splat(40.0)),
            response.id.with("radio"),
            Sense::click(),
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let response = response.on_hover_text("Start radio");
        response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, "Start radio"));
        paint_icon(
            ui.painter(),
            Pos2::new(radio_x, rect.center().y),
            "radio",
            20.0,
            app_theme.scheme.on_surface_variant,
        );
        response
    });
    download_response
        .widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, download_hint(download)));
    response.widget_info(|| {
        WidgetInfo::labeled(
            egui::Role::Button,
            true,
            format!("{} — {}", song.title, subtitle),
        )
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    SongRowResponse {
        row: response,
        download: download_response,
        download_action,
        radio: radio_response,
        artist: artist_response,
    }
}

/// Album card with play overlay.
#[derive(Debug, Clone)]
pub struct AlbumCardResponse {
    pub card: Response,
    pub play: Response,
}

pub fn album_card(
    ui: &mut Ui,
    app_theme: &Theme,
    title: &str,
    subtitle: &str,
    art: Option<&TextureHandle>,
    width: f32,
) -> AlbumCardResponse {
    let width = width.max(72.0);
    let (rect, card) = ui.allocate_exact_size(Vec2::new(width, width + 56.0), Sense::click());
    let art_rect = Rect::from_min_size(rect.left_top(), Vec2::splat(width));
    art_or_placeholder(ui, app_theme, art, art_rect, 8);
    let badge_center = art_rect.right_bottom() - Vec2::new(22.0, 22.0);
    let play_rect = Rect::from_center_size(badge_center, Vec2::splat(48.0));
    let play = ui.interact(
        play_rect,
        ui.scope_id().with(("album_play", title)),
        Sense::click(),
    );
    let hover_t = motion::t(
        ui,
        card.id.with("hover"),
        card.hovered() || play.hovered() || card.is_pointer_button_down_on(),
        motion::HOVER,
    );
    if hover_t > 0.0 {
        let stroke = card_hover_stroke(app_theme);
        let color = Color32::from_rgba_unmultiplied(
            stroke.color.r(),
            stroke.color.g(),
            stroke.color.b(),
            (stroke.color.a() as f32 * hover_t).round() as u8,
        );
        ui.painter().rect_stroke(
            art_rect,
            8,
            Stroke::new(stroke.width, color),
            StrokeKind::Inside,
        );
    }
    let badge_scale = motion::lerp(1.0, motion::BADGE_SCALE, hover_t);
    let badge_alpha = motion::lerp(160.0, 210.0, hover_t).round() as u8;
    ui.painter().circle_filled(
        badge_center,
        16.0 * badge_scale,
        Color32::from_black_alpha(badge_alpha),
    );
    ui.painter().add(Shape::convex_polygon(
        vec![
            badge_center + Vec2::new(-4.0, -6.0) * badge_scale,
            badge_center + Vec2::new(-4.0, 6.0) * badge_scale,
            badge_center + Vec2::new(6.0, 0.0) * badge_scale,
        ],
        Color32::WHITE,
        Stroke::NONE,
    ));
    play.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, format!("Play {title}")));

    elided(
        ui.painter(),
        rect.left_top() + Vec2::new(0.0, width + DP8),
        title,
        medium_font(15.0),
        app_theme.scheme.on_surface,
        width,
    );
    elided(
        ui.painter(),
        rect.left_top() + Vec2::new(0.0, width + 28.0), // art + 8 dp, 18 dp title line, 2 dp
        subtitle,
        regular_font(13.0),
        app_theme.scheme.on_surface_variant,
        width,
    );
    card.widget_info(|| {
        WidgetInfo::labeled(egui::Role::Button, true, format!("{title}, {subtitle}"))
    });
    if card.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    AlbumCardResponse { card, play }
}

/// Small "playing now" indicator used by the queue modal.
pub fn playing_bars(ui: &mut Ui, app_theme: &Theme, rect: Rect, playing: bool) {
    let center = rect.center();
    let time = ui.input(|input| input.time) as f32;
    let animate = playing && !motion::reduced(ui.ctx());
    if animate {
        ui.ctx().request_repaint();
    }
    for (i, height) in [8.0, 14.0, 10.0].into_iter().enumerate() {
        let height = if animate {
            height
                + 3.0
                    * (std::f32::consts::TAU * time / motion::BARS_PERIOD
                        + i as f32 * std::f32::consts::FRAC_PI_3)
                        .sin()
        } else {
            height
        };
        let x = center.x - 6.0 + i as f32 * 6.0;
        ui.painter().rect_filled(
            Rect::from_center_size(Pos2::new(x, center.y), Vec2::new(3.0, height)),
            1,
            app_theme.scheme.primary,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pure::DownloadBadge;
    use crate::theme::Theme;
    use ml_core::{Settings, Song};

    #[test]
    fn radio_affordance_is_optional_per_row() {
        let ctx = egui::Context::default();
        let theme = Theme::from_settings(&Settings::default());
        let song = Song::default();
        let mut visible = None;
        let mut hidden = None;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(360.0, 160.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let options = SongRowOptions {
                    download: DownloadBadge::None,
                    radio: true,
                };
                visible = Some(
                    song_row(ui, &theme, &song, None, 320.0, false, options)
                        .radio
                        .is_some(),
                );
                let options = SongRowOptions {
                    download: DownloadBadge::None,
                    radio: false,
                };
                hidden = Some(
                    song_row(ui, &theme, &song, None, 320.0, false, options)
                        .radio
                        .is_none(),
                );
            });
        });
        output.textures_delta.clear();
        assert_eq!(visible, Some(true));
        assert_eq!(hidden, Some(true));
    }

    #[test]
    fn trailing_row_actions_are_disjoint_and_inside_the_row() {
        let ctx = egui::Context::default();
        let theme = Theme::from_settings(&Settings::default());
        let song = Song::default();
        let mut measured = None;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(360.0, 160.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = song_row(
                    ui,
                    &theme,
                    &song,
                    None,
                    320.0,
                    false,
                    SongRowOptions {
                        download: DownloadBadge::None,
                        radio: true,
                    },
                );
                measured = Some((
                    response.row.rect,
                    response.download.rect,
                    response.radio.expect("radio requested").rect,
                ));
            });
        });
        output.textures_delta.clear();
        let (row, download, radio) = measured.expect("row rendered");
        assert!(
            radio.max.x <= download.min.x + 0.01,
            "radio {radio:?} overlaps download {download:?}"
        );
        for (name, rect) in [("download", download), ("radio", radio)] {
            assert!(
                row.contains(rect.min) && row.contains(rect.max),
                "{name} {rect:?} escapes row {row:?}"
            );
        }
    }
}
