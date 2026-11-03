//! Full player route: backdrop, art, seek, transport, volume, like, lyrics.

use eframe::egui::{
    self, Align, Color32, Layout, Pos2, Rect, Sense, TextureHandle, Ui, Vec2, WidgetInfo,
};

use crate::pure::advance_vinyl_angle;
use crate::theme::{bold_font, medium_font, regular_font, Theme, DP12, DP16, DP4, DP8, TOUCH};
use crate::widgets::{
    filled_icon_button, icon_button, icon_button_at, paint_icon, slider, text_width, vinyl_disc,
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
    let spinning = view.playing && view.status == PlaybackStatus::Playing;
    let dt = ui.input(|input| input.stable_dt);
    let vinyl_angle = ui.data_mut(|data| {
        let state = data.get_temp_mut_or_insert_with::<VinylState>(
            egui::Id::unique("player_vinyl_state"),
            || VinylState::new(&song.id),
        );
        state.step(&song.id, spinning, dt);
        state.angle
    });
    if spinning {
        ui.ctx().request_repaint();
    }
    ui.add_space(DP4);
    ui.vertical_centered(|ui| {
        let (art_rect, art_response) =
            ui.allocate_exact_size(Vec2::splat(art_size), Sense::hover());
        art_response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Album art"));
        art_response.on_hover_text("Album art");
        vinyl_disc(ui, app_theme, view.texture, art_rect, vinyl_angle);
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

        // Six equal icon slots surround the wider play button. The slot width
        // is derived from the available width, capped to keep the cluster tight
        // on wide windows. The fixed 64 dp row height is what keeps the 40 dp
        // icons vertically centred once the play button expands the row.
        let (leading, slot) = transport_geometry(width);
        ui.allocate_ui_with_layout(
            Vec2::new(width, 64.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(leading);
                let like =
                    transport_slot(ui, slot, TOUCH, |ui| like_button(ui, app_theme, view.liked));
                if like.clicked() {
                    action = PlayerAction::ToggleLike;
                }
                let shuffle = transport_slot(ui, slot, TOUCH, |ui| {
                    icon_button(ui, app_theme, "shuffle", "Shuffle", 22.0)
                        .on_hover_text("Shuffle (S)")
                });
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
                let previous = transport_slot(ui, slot, TOUCH, |ui| {
                    icon_button(ui, app_theme, "skip_previous", "Previous", 26.0)
                        .on_hover_text("Previous")
                });
                if previous.clicked() {
                    action = PlayerAction::Previous;
                }
                let toggle = transport_slot(ui, 64.0, 64.0, |ui| {
                    filled_icon_button(
                        ui,
                        app_theme,
                        if view.playing { "pause" } else { "play_arrow" },
                        if view.playing { "Pause" } else { "Play" },
                        64.0,
                    )
                    .on_hover_text("Play/Pause (Space)")
                });
                if toggle.clicked() {
                    action = PlayerAction::TogglePlay;
                }
                let next = transport_slot(ui, slot, TOUCH, |ui| {
                    icon_button(ui, app_theme, "skip_next", "Next", 26.0).on_hover_text("Next")
                });
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
                let repeat = transport_slot(ui, slot, TOUCH, |ui| {
                    icon_button(ui, app_theme, repeat_icon, repeat_label, 22.0)
                        .on_hover_text("Repeat (R)")
                });
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
            },
        );

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

/// Leading margin and icon-slot width for the transport row. The six icon
/// slots plus the 64 dp play cell are centred, so the play button always lands
/// on the art's centre line without absolute button positions.
fn transport_geometry(width: f32) -> (f32, f32) {
    let transport_width = width.min(360.0);
    let slot = ((transport_width - 64.0) / 6.0).max(0.0);
    (((width - transport_width) * 0.5).max(0.0), slot)
}

/// One transport cell. The cell height is fixed so every control shares a
/// centre line; content is centred inside its slot without absolute x offsets.
fn transport_slot<R>(
    ui: &mut Ui,
    slot_width: f32,
    content_width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(
        Vec2::new(slot_width, 64.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(((slot_width - content_width) * 0.5).max(0.0));
            add(ui)
        },
    )
    .inner
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

#[derive(Clone, Debug)]
struct VinylState {
    song_id: String,
    angle: f32,
}

impl VinylState {
    fn new(song_id: &str) -> Self {
        Self {
            song_id: song_id.to_owned(),
            angle: 0.0,
        }
    }

    fn step(&mut self, song_id: &str, playing: bool, dt: f32) {
        if self.song_id != song_id {
            self.song_id.clear();
            self.song_id.push_str(song_id);
            self.angle = 0.0;
            return;
        }
        self.angle = advance_vinyl_angle(self.angle, dt, playing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pure::{wrap_vinyl_angle, VINYL_RADIANS_PER_SEC};
    use crate::theme::Theme as TestTheme;
    use eframe::egui::{CentralPanel, Context, RawInput, Shape};
    use ml_core::Settings;

    #[test]
    fn vinyl_angle_wraps_clamps_freezes_and_resets_on_song_change() {
        let tau = std::f32::consts::TAU;
        let wrapped = wrap_vinyl_angle(tau + 0.5);
        assert!((0.0..tau).contains(&wrapped));
        assert!((wrapped - 0.5).abs() < 1e-5);
        let negative = wrap_vinyl_angle(-0.5);
        assert!((0.0..tau).contains(&negative));
        assert!((negative - (tau - 0.5)).abs() < 1e-5);

        let one_step = advance_vinyl_angle(0.0, 0.05, true);
        assert!((one_step - 0.05 * VINYL_RADIANS_PER_SEC).abs() < 1e-5);
        let clamped = advance_vinyl_angle(0.0, 10.0, true);
        assert!((clamped - one_step).abs() < 1e-5);
        let frozen = advance_vinyl_angle(1.2, 1.0, false);
        assert!((frozen - 1.2).abs() < 1e-6);

        let mut state = VinylState::new("song-a");
        state.step("song-a", true, 0.05);
        let before = state.angle;
        assert!(before > 0.0);
        state.step("song-a", false, 1.0);
        assert_eq!(state.angle, before);
        state.step("song-b", true, 0.05);
        assert_eq!(state.angle, 0.0);
        assert_eq!(state.song_id, "song-b");
    }

    #[test]
    fn vinyl_disc_renders_with_and_without_texture() {
        let ctx = Context::default();
        let theme = TestTheme::from_settings(&Settings::default());
        let texture = ctx.load_texture(
            "vinyl-test-cover",
            egui::ColorImage::filled([8, 8], Color32::from_rgb(90, 40, 120)),
            egui::TextureOptions::NEAREST,
        );
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(405.0, 900.0))),
            ..RawInput::default()
        };
        let mut painted = 0;
        let mut output = ctx.run_ui(input, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                vinyl_disc(
                    ui,
                    &theme,
                    None,
                    Rect::from_min_size(Pos2::new(16.0, 16.0), Vec2::splat(160.0)),
                    0.0,
                );
                vinyl_disc(
                    ui,
                    &theme,
                    Some(&texture),
                    Rect::from_min_size(Pos2::new(16.0, 200.0), Vec2::splat(160.0)),
                    0.37,
                );
                painted += 2;
            });
        });
        output.textures_delta.clear();
        assert_eq!(painted, 2);
        let mesh = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                Shape::Mesh(mesh) => Some(mesh),
                _ => None,
            })
            .expect("textured disc should paint a mesh");
        assert_eq!(
            mesh.vertices.len(),
            98,
            "centre vertex plus a 96-segment ring"
        );
        assert_eq!(mesh.indices.len(), 96 * 3, "one triangle per ring segment");
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| (0.0..=1.0).contains(&vertex.uv.x)
                    && (0.0..=1.0).contains(&vertex.uv.y)),
            "the circular fan must stay inside the texture"
        );
    }

    #[test]
    fn transport_slots_keep_play_centred_at_every_width() {
        for width in [320.0, 373.0, 405.0, 560.0, 1100.0] {
            let (leading, slot) = transport_geometry(width);
            assert!(leading >= 0.0);
            assert!(slot >= TOUCH, "slot {slot} too narrow at {width}");
            let play_center = leading + slot * 3.0 + 32.0;
            assert!(
                (play_center - width * 0.5).abs() < 0.01,
                "play centre {play_center} != {} at {width}",
                width * 0.5
            );
        }
    }
}
