//! Full player route: backdrop, art, seek, transport, volume, like, lyrics.

use eframe::egui::{
    self, Align, Color32, Layout, Pos2, Rect, Sense, TextureHandle, Ui, Vec2, WidgetInfo,
};

use crate::motion;
use crate::pure::advance_vinyl_angle;
use crate::theme::{
    bold_font, medium_font, regular_font, Theme, DP12, DP16, DP24, DP4, DP8, TOUCH,
};
use crate::widgets::{
    filled_icon_button_swap, icon_button, icon_button_at, paint_icon, progress_slider, slider,
    text_width, vinyl_disc,
};
use ml_core::{format_position, PlaybackStatus, RepeatMode, Song};

/// Volume slider width; mute button (40) + 12 dp gap + slider = 202 dp row.
const VOLUME_SLIDER: f32 = 150.0;

#[derive(Debug, Clone, PartialEq)]
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
    StartRadio,
    OpenArtist(String),
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
    /// The installed queue came from a radio (`QueueSource::Radio`).
    pub radio_active: bool,
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
    let spinning =
        view.playing && view.status == PlaybackStatus::Playing && !motion::reduced(ui.ctx());
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
        let byline_action = artist_label(ui, app_theme, song, view.offline);
        if byline_action != PlayerAction::None {
            action = byline_action;
        }
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
        // Clearance for the hover time bubble above the timeline, which
        // otherwise overlaps the artist byline on the compact layout.
        ui.add_space(DP24);
        let slider_width = (width - 48.0).min(460.0);
        let buffering = matches!(
            view.status,
            PlaybackStatus::Loading | PlaybackStatus::Buffering
        );
        let seek = progress_slider(
            ui,
            app_theme,
            view.position_ms,
            view.duration_ms,
            buffering,
            slider_width,
        );
        seek.response.on_hover_text("Seek (←/→ 5 s)");
        if let Some(target) = seek.seek {
            action = PlayerAction::Seek(target);
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

        let next = transport(ui, app_theme, view, width);
        if next != PlayerAction::None {
            action = next;
        }

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

/// Artist byline under the title. Returns `OpenArtist` when the first artist
/// carries an id and its text is clicked; a missing id stays plain text.
fn artist_label(ui: &mut Ui, app_theme: &Theme, song: &Song, offline: bool) -> PlayerAction {
    let mut byline = song.artist_names();
    if offline {
        byline.push_str(" · Offline");
    }
    let linked_artist = song
        .artists
        .iter()
        .find(|artist| artist.id.as_deref().is_some_and(|id| !id.is_empty()));
    let Some(artist) = linked_artist else {
        ui.add(
            egui::Label::new(
                egui::RichText::new(byline)
                    .font(regular_font(14.0))
                    .color(app_theme.scheme.on_surface_variant),
            )
            .truncate(),
        );
        return PlayerAction::None;
    };
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(byline)
                .font(regular_font(14.0))
                .color(app_theme.scheme.on_surface_variant),
        )
        .truncate()
        .sense(Sense::click()),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| {
        WidgetInfo::labeled(
            egui::Role::Button,
            true,
            format!("Open artist {}", artist.name),
        )
    });
    if response.clicked() {
        artist
            .id
            .clone()
            .map(PlayerAction::OpenArtist)
            .unwrap_or(PlayerAction::None)
    } else {
        PlayerAction::None
    }
}

fn transport(ui: &mut Ui, app_theme: &Theme, view: &PlayerView<'_>, width: f32) -> PlayerAction {
    let mut action = PlayerAction::None;
    let (leading, slot) = transport_geometry(width);
    ui.allocate_ui_with_layout(
        Vec2::new(width, 64.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(leading);
            let like = transport_slot(ui, slot, TOUCH, |ui| like_button(ui, app_theme, view.liked));
            if like.clicked() {
                action = PlayerAction::ToggleLike;
            }
            let shuffle = transport_slot(ui, slot, TOUCH, |ui| {
                icon_button(ui, app_theme, "shuffle", "Shuffle", 22.0).on_hover_text("Shuffle (S)")
            });
            if view.shuffle {
                active_dot(ui, app_theme, &shuffle);
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
            let play_mix = motion::t(
                ui,
                egui::Id::unique("player_play_mix"),
                view.playing,
                motion::SELECT,
            );
            let toggle = transport_slot(ui, 64.0, 64.0, |ui| {
                filled_icon_button_swap(
                    ui,
                    app_theme,
                    "play_arrow",
                    "pause",
                    play_mix,
                    "Play/Pause",
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
                active_dot(ui, app_theme, &repeat);
            }
            if repeat.clicked() {
                action = PlayerAction::CycleRepeat;
            }
            let radio = transport_slot(ui, slot, TOUCH, |ui| {
                icon_button(ui, app_theme, "radio", "Start radio", 22.0)
                    .on_hover_text("Start radio")
            });
            if view.radio_active {
                active_dot(ui, app_theme, &radio);
            }
            if radio.clicked() {
                action = PlayerAction::StartRadio;
            }
        },
    );
    action
}

/// Small primary dot under an active transport toggle.
fn active_dot(ui: &Ui, app_theme: &Theme, response: &egui::Response) {
    ui.painter().circle_filled(
        Pos2::new(response.rect.center().x, response.rect.bottom() - 2.0),
        2.0,
        app_theme.scheme.primary,
    );
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
    // Allocate the slot on the row itself: `allocate_ui_with_layout` advances
    // by the child's min width, so six slightly narrower icon cells would
    // pull the 64 dp play cell left of the content centre.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(slot_width, 64.0), Sense::hover());
    let mut slot = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    slot.add_space(((slot_width - content_width) * 0.5).max(0.0));
    add(&mut slot)
}

fn like_button(ui: &mut Ui, app_theme: &Theme, liked: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(TOUCH), Sense::click());
    let (hover_t, press_t) = motion::interaction(ui, &response);
    if hover_t > 0.0 || press_t > 0.0 {
        let scale = motion::press_scale_at(press_t);
        ui.painter().circle_filled(
            rect.center(),
            18.0 * scale,
            motion::state_layer_at(app_theme.scheme.surface_container_high, hover_t, press_t),
        );
    }
    paint_icon(
        ui.painter(),
        rect.center(),
        "favorite",
        22.0 * motion::press_scale_at(press_t),
        if liked {
            app_theme.scheme.primary
        } else {
            app_theme.scheme.on_surface_variant
        },
    );
    motion::focus_ring(ui, rect, 20, &response);
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
    use eframe::egui::{CentralPanel, Context, Event, Modifiers, PointerButton, RawInput, Shape};
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

    fn test_view(song: &Song) -> PlayerView<'_> {
        PlayerView {
            song: Some(song),
            playing: true,
            status: PlaybackStatus::Playing,
            position_ms: 30_000,
            duration_ms: 120_000,
            shuffle: false,
            repeat: RepeatMode::Off,
            texture: None,
            volume: 0.7,
            liked: false,
            in_library: false,
            muted: false,
            offline: false,
            radio_active: false,
        }
    }

    fn test_song() -> Song {
        Song {
            id: "song".into(),
            title: "Song".into(),
            artists: vec![ml_core::Artist::new("LONOWN", Some("UCartist".into()))],
            duration: Some(120),
            ..Song::default()
        }
    }

    fn clickable_frame(
        ctx: &Context,
        width: f32,
        height: f32,
        events: Vec<Event>,
        mut draw: impl FnMut(&mut Ui) -> PlayerAction,
    ) -> (PlayerAction, eframe::egui::FullOutput) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))),
            events,
            ..RawInput::default()
        };
        let mut action = PlayerAction::None;
        let mut output = ctx.run_ui(input, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                action = draw(ui);
            });
        });
        output.textures_delta.clear();
        (action, output)
    }

    fn press_release(pos: Pos2) -> [Vec<Event>; 3] {
        [
            vec![Event::PointerMoved(pos)],
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::default(),
                },
            ],
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::default(),
                },
            ],
        ]
    }

    #[test]
    fn transport_slots_keep_play_centred_at_every_width() {
        for width in [320.0, 373.0, 405.0, 560.0, 1100.0] {
            let (leading, slot) = transport_geometry(width);
            assert!(leading >= 0.0);
            assert!(
                (slot * 6.0 + 64.0 - width.min(360.0)).abs() < 0.01,
                "seven transport cells fill the row at {width}"
            );
            assert!(slot >= TOUCH, "slot {slot} below touch at {width}");
            let play_center = leading + slot * 3.0 + 32.0;
            assert!(
                (play_center - width * 0.5).abs() < 0.01,
                "play centre {play_center} != {} at {width}",
                width * 0.5
            );
        }
    }

    #[test]
    fn rendered_play_button_stays_on_the_content_centre_line() {
        let theme = TestTheme::from_settings(&Settings::default());
        let song = test_song();
        let view = test_view(&song);
        for (screen_width, width) in [(405.0, 357.0), (600.0, 560.0), (440.0, 400.0)] {
            let ctx = Context::default();
            let mut play_hits = Vec::new();
            let mut row_center = 0.0;
            for x in (0..=(width as i32 + 32)).step_by(2) {
                let pos = Pos2::new(x as f32, 48.0);
                let mut last = PlayerAction::None;
                for events in press_release(pos) {
                    let (action, _) = clickable_frame(&ctx, screen_width, 200.0, events, |ui| {
                        ui.add_space(8.0);
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(width, 64.0), Sense::hover());
                        row_center = rect.center().x;
                        let mut row = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect)
                                .layout(Layout::left_to_right(Align::Center)),
                        );
                        transport(&mut row, &theme, &view, width)
                    });
                    last = action;
                }
                if last == PlayerAction::TogglePlay {
                    play_hits.push(x as f32);
                }
            }
            assert!(!play_hits.is_empty(), "play button not hit at {width}");
            let measured = (play_hits.first().unwrap() + play_hits.last().unwrap()) / 2.0;
            assert!(
                (measured - row_center).abs() <= 2.0,
                "rendered play centre {measured} != row centre {row_center} at width {width}"
            );
        }
    }

    #[test]
    fn radio_button_sits_right_of_repeat_and_reports_active_state() {
        let theme = TestTheme::from_settings(&Settings::default());
        let width = 357.0;
        let (_, slot) = transport_geometry(width);
        assert!(slot >= TOUCH, "405-wide player keeps 40 dp icon slots");
        let song = test_song();
        let mut view = test_view(&song);

        let mut repeat_hits = Vec::new();
        let mut radio_hits = Vec::new();
        for x in (8..400).step_by(4) {
            let ctx = Context::default();
            let pos = Pos2::new(x as f32, 40.0);
            let mut last = PlayerAction::None;
            for events in press_release(pos) {
                let (action, _) = clickable_frame(&ctx, 405.0, 200.0, events, |ui| {
                    transport(ui, &theme, &view, width)
                });
                last = action;
            }
            match last {
                PlayerAction::CycleRepeat => repeat_hits.push(x),
                PlayerAction::StartRadio => radio_hits.push(x),
                _ => {}
            }
        }
        assert!(!repeat_hits.is_empty(), "repeat must be clickable");
        assert!(!radio_hits.is_empty(), "radio must be clickable");
        assert!(
            radio_hits[0] > *repeat_hits.last().unwrap(),
            "radio sits immediately right of repeat: {repeat_hits:?} / {radio_hits:?}"
        );

        view.radio_active = true;
        let ctx = Context::default();
        let (_, output) = clickable_frame(&ctx, 405.0, 200.0, Vec::new(), |ui| {
            transport(ui, &theme, &view, width)
        });
        let dots: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Circle(circle) if (circle.radius - 2.0).abs() < 0.01 => {
                    Some(circle.center.x)
                }
                _ => None,
            })
            .collect();
        assert_eq!(dots.len(), 1, "only the radio active dot is drawn");
        assert!(
            dots[0] >= radio_hits[0] as f32 - 2.0
                && dots[0] <= *radio_hits.last().unwrap() as f32 + 2.0,
            "active dot {:?} under the radio hit area {radio_hits:?}",
            dots[0]
        );

        view.radio_active = false;
        let ctx = Context::default();
        let (_, output) = clickable_frame(&ctx, 405.0, 200.0, Vec::new(), |ui| {
            transport(ui, &theme, &view, width)
        });
        assert!(
            !output.shapes.iter().any(|clipped| matches!(
                &clipped.shape,
                Shape::Circle(circle) if (circle.radius - 2.0).abs() < 0.01
            )),
            "no active dot without a radio queue"
        );
    }

    #[test]
    fn artist_byline_opens_the_first_artist_with_an_id() {
        let theme = TestTheme::from_settings(&Settings::default());
        let song = test_song();
        let click = Pos2::new(24.0, 16.0);
        let ctx = Context::default();
        let mut action = PlayerAction::None;
        for events in press_release(click) {
            let (next, _) = clickable_frame(&ctx, 405.0, 120.0, events, |ui| {
                artist_label(ui, &theme, &song, false)
            });
            action = next;
        }
        assert_eq!(action, PlayerAction::OpenArtist("UCartist".into()));

        let mut plain = song.clone();
        plain.artists = vec![ml_core::Artist::new("LONOWN", None)];
        let ctx = Context::default();
        for events in press_release(click) {
            let (next, _) = clickable_frame(&ctx, 405.0, 120.0, events, |ui| {
                artist_label(ui, &theme, &plain, false)
            });
            assert_eq!(next, PlayerAction::None);
        }
    }
}
