//! Full player route: backdrop, art, seek, transport, volume, like, lyrics.

use eframe::egui::{
    self, Align, Color32, Layout, Pos2, Rect, Sense, TextureHandle, Ui, Vec2, WidgetInfo,
};

use crate::theme::{bold_font, medium_font, regular_font, Theme, DP12, DP16, DP4, DP8, TOUCH};
use crate::widgets::{
    art_or_placeholder, filled_icon_button, icon_button, icon_button_at, paint_icon, slider,
    text_width,
};
use ml_core::{format_position, PlaybackStatus, RepeatMode, Song};

/// Volume slider width; mute button (40) + 12 dp gap + slider = 202 dp row.
const VOLUME_SLIDER: f32 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerAction {
    None,
    TogglePlay,
    Next,
    Previous,
    Seek(u64),
    ToggleShuffle,
    CycleRepeat,
    OpenQueue,
    OpenLyrics,
    ToggleLike,
    ToggleLibrary,
    ToggleMute,
    SetVolume(f32),
    Close,
}

pub struct PlayerView<'a> {
    pub song: Option<&'a Song>,
    pub playing: bool,
    pub status: PlaybackStatus,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub texture: Option<&'a TextureHandle>,
    pub volume: f32,
    pub liked: bool,
    pub in_library: bool,
    pub muted: bool,
    pub offline: bool,
}

/// Full-window backdrop derived from the cover (or a scheme gradient).
pub fn backdrop(ui: &mut Ui, app_theme: &Theme, texture: Option<&TextureHandle>) {
    let rect = ui.max_rect();
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0, app_theme.bg());
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        let surface = app_theme.scheme.surface;
        painter.rect_filled(
            rect,
            0,
            Color32::from_rgba_unmultiplied(surface.r(), surface.g(), surface.b(), 232),
        );
    } else {
        painter.circle_filled(
            rect.left_top() + Vec2::new(rect.width() * 0.15, rect.height() * 0.08),
            rect.width() * 0.85,
            Color32::from_rgba_unmultiplied(
                app_theme.scheme.primary_container.r(),
                app_theme.scheme.primary_container.g(),
                app_theme.scheme.primary_container.b(),
                60,
            ),
        );
        painter.circle_filled(
            rect.right_bottom() - Vec2::new(rect.width() * 0.1, rect.height() * 0.05),
            rect.width() * 0.7,
            Color32::from_rgba_unmultiplied(
                app_theme.scheme.tertiary_container.r(),
                app_theme.scheme.tertiary_container.g(),
                app_theme.scheme.tertiary_container.b(),
                48,
            ),
        );
    }
}

pub fn show(ui: &mut Ui, app_theme: &Theme, view: &PlayerView<'_>, width: f32) -> PlayerAction {
    let Some(song) = view.song else {
        crate::widgets::empty_state(
            ui,
            app_theme,
            "music_off",
            "Nothing playing",
            "Pick a song from Home or Search.",
        );
        return PlayerAction::None;
    };

    let mut action = PlayerAction::None;
    ui.add_space(DP8);
    let (top_rect, _) = ui.allocate_exact_size(Vec2::new(width, 56.0), Sense::hover());
    let back = icon_button_at(
        ui,
        app_theme,
        "player_back",
        Rect::from_center_size(
            Pos2::new(top_rect.left() + 24.0, top_rect.center().y),
            Vec2::splat(40.0),
        ),
        "arrow_back",
        "Back",
        24.0,
    );
    let queue = icon_button_at(
        ui,
        app_theme,
        "player_queue",
        Rect::from_center_size(
            Pos2::new(top_rect.right() - 24.0, top_rect.center().y),
            Vec2::splat(40.0),
        ),
        "queue_music",
        "Queue",
        24.0,
    );
    let lyrics = icon_button_at(
        ui,
        app_theme,
        "player_lyrics",
        Rect::from_center_size(
            Pos2::new(top_rect.right() - 68.0, top_rect.center().y),
            Vec2::splat(40.0),
        ),
        "lyrics",
        "Lyrics",
        22.0,
    );
    let library = icon_button_at(
        ui,
        app_theme,
        "player_library",
        Rect::from_center_size(
            Pos2::new(top_rect.right() - 112.0, top_rect.center().y),
            Vec2::splat(40.0),
        ),
        if view.in_library {
            "library_music"
        } else {
            "library_add"
        },
        if view.in_library {
            "Remove from library"
        } else {
            "Add to library"
        },
        22.0,
    );
    let title = "Now playing";
    let title_width = text_width(ui.painter(), title, medium_font(14.0));
    crate::widgets::text(
        ui.painter(),
        Pos2::new(
            top_rect.center().x - title_width / 2.0,
            top_rect.center().y - 8.0,
        ),
        title,
        medium_font(14.0),
        app_theme.scheme.on_surface_variant,
    );
    if back.clicked() {
        return PlayerAction::Close;
    }
    if queue.clicked() {
        return PlayerAction::OpenQueue;
    }
    if lyrics.clicked() {
        return PlayerAction::OpenLyrics;
    }
    if library.clicked() {
        return PlayerAction::ToggleLibrary;
    }

    let available_height = ui.available_height();
    let art_size = (width - 24.0)
        .min(380.0)
        .min((available_height * 0.48).max(140.0))
        .max(120.0);
    ui.add_space(DP4);
    ui.vertical_centered(|ui| {
        let (art_rect, _) = ui.allocate_exact_size(Vec2::splat(art_size), Sense::hover());
        art_or_placeholder(ui, app_theme, view.texture, art_rect, 12);
        ui.add_space(DP16);
        ui.add(
            egui::Label::new(
                egui::RichText::new(&song.title)
                    .font(bold_font(24.0))
                    .color(app_theme.scheme.on_surface),
            )
            .truncate(),
        );
        ui.add_space(DP4);
        let mut byline = song.artist_names();
        if view.offline {
            byline.push_str(" · Offline");
        }
        ui.add(
            egui::Label::new(
                egui::RichText::new(byline)
                    .font(regular_font(14.0))
                    .color(app_theme.scheme.on_surface_variant),
            )
            .truncate(),
        );
        if let Some(album) = &song.album {
            ui.add_space(DP4);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&album.title)
                        .font(regular_font(12.0))
                        .color(app_theme.scheme.outline),
                )
                .truncate(),
            );
        }
        ui.add_space(DP12);

        let slider_width = (width - 48.0).min(460.0);
        let mut progress = if view.duration_ms > 0 {
            (view.position_ms as f32 / view.duration_ms as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let seek = slider(
            ui,
            app_theme,
            &mut progress,
            0.0..=1.0,
            slider_width,
            "Seek",
        )
        .on_hover_text("Seek (←/→ 5 s)");
        if seek.changed() {
            action = PlayerAction::Seek(crate::pure::seek_target(progress, view.duration_ms));
        }
        ui.allocate_ui_with_layout(
            Vec2::new(slider_width, 18.0),
            Layout::left_to_right(Align::Min),
            |ui| {
                ui.label(
                    egui::RichText::new(format_position(view.position_ms))
                        .font(regular_font(11.0))
                        .color(app_theme.scheme.on_surface_variant),
                );
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    ui.label(
                        egui::RichText::new(format_position(view.duration_ms))
                            .font(regular_font(11.0))
                            .color(app_theme.scheme.on_surface_variant),
                    );
                });
            },
        );
        ui.add_space(DP8);

        // Three controls sit left of the play button and two right of it; the
        // trailing spacer keeps the button on the art's centre line.
        let total = TOUCH * 6.0 + 64.0 + DP8 * 6.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.add_space(((width - total) / 2.0).max(0.0));
            if like_button(ui, app_theme, view.liked).clicked() {
                action = PlayerAction::ToggleLike;
            }
            let shuffle =
                icon_button(ui, app_theme, "shuffle", "Shuffle", 22.0).on_hover_text("Shuffle (S)");
            if view.shuffle {
                ui.painter().circle_filled(
                    Pos2::new(shuffle.rect.center().x, shuffle.rect.bottom() - 2.0),
                    2.0,
                    app_theme.scheme.primary,
                );
            }
            if shuffle.clicked() {
                action = PlayerAction::ToggleShuffle;
            }
            let previous = icon_button(ui, app_theme, "skip_previous", "Previous", 26.0)
                .on_hover_text("Previous");
            if previous.clicked() {
                action = PlayerAction::Previous;
            }
            let toggle = filled_icon_button(
                ui,
                app_theme,
                if view.playing { "pause" } else { "play_arrow" },
                if view.playing { "Pause" } else { "Play" },
                64.0,
            )
            .on_hover_text("Play/Pause (Space)");
            if toggle.clicked() {
                action = PlayerAction::TogglePlay;
            }
            let next = icon_button(ui, app_theme, "skip_next", "Next", 26.0).on_hover_text("Next");
            if next.clicked() {
                action = PlayerAction::Next;
            }
            let repeat_icon = if view.repeat == RepeatMode::One {
                "repeat_one"
            } else {
                "repeat"
            };
            let repeat_label = match view.repeat {
                RepeatMode::Off => "Repeat off",
                RepeatMode::All => "Repeat all",
                RepeatMode::One => "Repeat one",
            };
            let repeat = icon_button(ui, app_theme, repeat_icon, repeat_label, 22.0)
                .on_hover_text("Repeat (R)");
            if view.repeat != RepeatMode::Off {
                ui.painter().circle_filled(
                    Pos2::new(repeat.rect.center().x, repeat.rect.bottom() - 2.0),
                    2.0,
                    app_theme.scheme.primary,
                );
            }
            if repeat.clicked() {
                action = PlayerAction::CycleRepeat;
            }
            ui.add_space(TOUCH);
        });

        let status = match view.status {
            PlaybackStatus::Loading => Some("Loading stream…"),
            PlaybackStatus::Buffering => Some("Buffering…"),
            PlaybackStatus::Error => Some("Playback error — see notifications"),
            _ => None,
        };
        if let Some(status) = status {
            ui.add_space(DP8);
            ui.label(
                egui::RichText::new(status)
                    .font(regular_font(12.0))
                    .color(app_theme.scheme.error),
            );
        }

        ui.add_space(DP12);
        let volume_total = TOUCH + DP12 + VOLUME_SLIDER;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = DP12;
            ui.add_space(((width - volume_total) / 2.0).max(0.0));
            let mute = icon_button(
                ui,
                app_theme,
                if view.muted || view.volume <= 0.001 {
                    "volume_off"
                } else {
                    "volume_up"
                },
                "Mute",
                18.0,
            )
            .on_hover_text("Mute (M)");
            if mute.clicked() {
                action = PlayerAction::ToggleMute;
            }
            let mut volume = view.volume.clamp(0.0, 1.0);
            let volume_response = slider(
                ui,
                app_theme,
                &mut volume,
                0.0..=1.0,
                VOLUME_SLIDER,
                "Volume",
            )
            .on_hover_text("Volume (↑/↓, M mute)");
            if volume_response.changed() {
                action = PlayerAction::SetVolume(volume);
            }
        });
    });

    action
}

fn like_button(ui: &mut Ui, app_theme: &Theme, liked: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(TOUCH), Sense::click());
    if response.hovered() {
        ui.painter()
            .circle_filled(rect.center(), 18.0, app_theme.scheme.surface_container_high);
    }
    paint_icon(
        ui.painter(),
        rect.center(),
        "favorite",
        22.0,
        if liked {
            app_theme.scheme.primary
        } else {
            app_theme.scheme.on_surface_variant
        },
    );
    response
        .widget_info(|| WidgetInfo::selected(egui::Role::Button, true, liked, "Like this song"));
    response
}
