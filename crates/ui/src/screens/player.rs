//! Full player route: backdrop, art, seek, transport, volume, like, lyrics.

use eframe::egui::{self, Align, Layout, Pos2, Rect, Sense, TextureHandle, Ui, Vec2, WidgetInfo};

use crate::motion;
use crate::theme::{
    font_label, medium_font, regular_font, Theme, DP12, DP16, DP24, DP4, DP8, HEADLINE_SMALL,
    LABEL_SMALL, SHAPE_LARGE, TOUCH,
};
use crate::widgets::{
    art_or_placeholder, icon_button, icon_button_at, outlined_button, paint_icon, paint_icon_swap,
    progress_slider, slider,
};
use ml_core::{format_position, PlaybackStatus, RepeatMode, Song};

/// Volume slider width; mute button (44) + 12 dp gap + 150 dp slider = 206 dp row.
const VOLUME_SLIDER: f32 = 150.0;

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerAction {
    None,
    TogglePlay,
    /// Reload the current track after a playback error.
    Retry,
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
    /// Reason behind [`PlaybackStatus::Error`], shown inline instead of a toast.
    pub error: Option<&'a str>,
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

/// Dither strength of the player stage behind the cover.
const STAGE_STRENGTH: f32 = 0.12;

/// Flat backdrop: the Nothing player never tints or blurs the cover into the
/// surface; the artwork stays a framed square on `bg()`.
pub fn backdrop(ui: &mut Ui, app_theme: &Theme, _texture: Option<&TextureHandle>) {
    ui.painter().rect_filled(ui.max_rect(), 0, app_theme.bg());
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
            Vec2::splat(TOUCH),
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
            Vec2::splat(TOUCH),
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
            Pos2::new(top_rect.right() - 72.0, top_rect.center().y),
            Vec2::splat(TOUCH),
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
            Pos2::new(top_rect.right() - 120.0, top_rect.center().y),
            Vec2::splat(TOUCH),
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
    // Eyebrow: the route context, uppercase mono 11.
    let eyebrow_x = top_rect.left() + 24.0 + TOUCH / 2.0 + 8.0;
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(eyebrow_x, top_rect.center().y - 7.0),
        "NOW PLAYING",
        font_label(LABEL_SMALL),
        app_theme.scheme.on_surface_variant,
        (top_rect.right() - 142.0 - 8.0 - eyebrow_x).max(0.0),
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
    // Full-width square hero; the height cap keeps the transport above the fold.
    let art_size = width
        .min(430.0)
        .min((available_height * 0.5).max(140.0))
        .max(120.0);
    ui.add_space(DP4);
    // The stage spans the window, not the content column: the dots stay
    // visible in the side insets on compact widths.
    let window = ui.input(|input| input.viewport_rect());
    let stage = (window, window.x_range());
    ui.vertical_centered(|ui| {
        let (art_rect, art_response) =
            ui.allocate_exact_size(Vec2::splat(art_size), Sense::hover());
        art_response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Album art"));
        art_response.on_hover_text("Album art");
        // The stage behind the cover: dots show in the margins, the artwork
        // itself stays a clean framed square.
        let band = Rect::from_x_y_ranges(stage.1, art_rect.y_range());
        let stage_painter = ui.painter().with_clip_rect(stage.0);
        crate::widgets::dither_rect(&stage_painter, band, STAGE_STRENGTH);
        art_or_placeholder(ui, app_theme, view.texture, art_rect, SHAPE_LARGE);
        ui.add_space(DP16);
        ui.add(
            egui::Label::new(
                egui::RichText::new(&song.title)
                    .font(medium_font(HEADLINE_SMALL))
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
                        .color(app_theme.scheme.on_surface_variant),
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
                        .font(font_label(LABEL_SMALL))
                        .color(app_theme.scheme.on_surface_variant),
                );
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    ui.label(
                        egui::RichText::new(format_position(view.duration_ms))
                            .font(font_label(LABEL_SMALL))
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

        match view.status {
            PlaybackStatus::Loading | PlaybackStatus::Buffering => {
                ui.add_space(DP8);
                let status = if view.status == PlaybackStatus::Loading {
                    "LOADING STREAM…"
                } else {
                    "BUFFERING…"
                };
                ui.label(
                    egui::RichText::new(status)
                        .font(font_label(LABEL_SMALL))
                        .color(app_theme.scheme.on_surface_variant),
                );
            }
            PlaybackStatus::Error => {
                // `error` is the documented red text role (`on_error_container`);
                // `scheme.error` is fill-only at 4.05:1 and fails AA as text.
                ui.add_space(DP8);
                ui.label(
                    egui::RichText::new(view.error.unwrap_or("PLAYBACK ERROR").to_uppercase())
                        .font(font_label(LABEL_SMALL))
                        .color(app_theme.scheme.on_error_container),
                );
                ui.add_space(DP8);
                if outlined_button(ui, app_theme, "Retry").clicked() {
                    action = PlayerAction::Retry;
                }
            }
            _ => {}
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
                transport_button(ui, app_theme, "shuffle", "Shuffle", 20.0)
                    .on_hover_text("Shuffle (S)")
            });
            if view.shuffle {
                active_dot(ui, app_theme, &shuffle);
            }
            if shuffle.clicked() {
                action = PlayerAction::ToggleShuffle;
            }
            let previous = transport_slot(ui, slot, TOUCH, |ui| {
                transport_button(ui, app_theme, "skip_previous", "Previous", 22.0)
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
                play_pill(ui, app_theme, play_mix).on_hover_text("Play/Pause (Space)")
            });
            if toggle.clicked() {
                action = PlayerAction::TogglePlay;
            }
            let next = transport_slot(ui, slot, TOUCH, |ui| {
                transport_button(ui, app_theme, "skip_next", "Next", 22.0).on_hover_text("Next")
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
                transport_button(ui, app_theme, repeat_icon, repeat_label, 20.0)
                    .on_hover_text("Repeat (R)")
            });
            if view.repeat != RepeatMode::Off {
                active_dot(ui, app_theme, &repeat);
            }
            if repeat.clicked() {
                action = PlayerAction::CycleRepeat;
            }
            let radio = transport_slot(ui, slot, TOUCH, |ui| {
                transport_button(ui, app_theme, "radio", "Start radio", 20.0)
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
/// slots plus the 64 dp play pill are centred, so play always lands on the
/// art's centre line without absolute button positions.
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

/// Like toggle: the heart inverts to `on_surface` when liked; no hover fill.
fn like_button(ui: &mut Ui, app_theme: &Theme, liked: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(TOUCH), Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_icon(
        ui.painter(),
        rect.center(),
        "favorite",
        22.0 * motion::press_scale(ui, &response),
        if liked {
            app_theme.scheme.on_surface
        } else {
            app_theme.scheme.on_surface_variant
        },
    );
    motion::focus_ring(ui, rect, 22, &response);
    response
        .widget_info(|| WidgetInfo::selected(egui::Role::Button, true, liked, "Like this song"));
    response
}

/// 44 dp secondary transport target: a 1 px frame instead of a fill.
fn transport_button(
    ui: &mut Ui,
    app_theme: &Theme,
    icon_name: &str,
    label: &str,
    icon_size: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(TOUCH), Sense::click());
    let pill = (rect.height() * 0.5).round() as u8;
    ui.painter().rect_stroke(
        rect,
        pill,
        egui::Stroke::new(1.0, app_theme.scheme.outline),
        egui::StrokeKind::Inside,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_icon(
        ui.painter(),
        rect.center(),
        icon_name,
        icon_size,
        app_theme.scheme.on_surface,
    );
    motion::focus_ring(ui, rect, pill, &response);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

/// Inverted play pill: `on_surface` fill, `surface` glyph.
fn play_pill(ui: &mut Ui, app_theme: &Theme, mix: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(64.0, TOUCH), Sense::click());
    let pill = (rect.height() * 0.5).round() as u8;
    ui.painter()
        .rect_filled(rect, pill, app_theme.scheme.on_surface);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_icon_swap(
        ui.painter(),
        rect.center(),
        "play_arrow",
        "pause",
        mix,
        24.0,
        app_theme.scheme.surface,
    );
    motion::focus_ring(ui, rect, pill, &response);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, "Play/Pause"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme as TestTheme;
    use eframe::egui::{CentralPanel, Context, Event, Modifiers, PointerButton, RawInput, Shape};
    use ml_core::Settings;

    fn test_view(song: &Song) -> PlayerView<'_> {
        PlayerView {
            song: Some(song),
            playing: true,
            status: PlaybackStatus::Playing,
            error: None,
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
        // Narrowest width where six touch-sized slots still fit beside play.
        let narrowest = TOUCH * 6.0 + 64.0;
        for width in [narrowest, 373.0, 405.0, 560.0, 1100.0] {
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
        assert!(slot >= TOUCH, "405-wide player keeps 44 dp icon slots");
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
    fn error_state_paints_the_reason_and_a_retry_button() {
        let theme = TestTheme::from_settings(&Settings::default());
        let song = test_song();
        let mut view = test_view(&song);
        view.playing = false;
        view.status = PlaybackStatus::Error;
        view.error = Some("mpv could not play the current file");
        let ctx = Context::default();
        let (_, output) = clickable_frame(&ctx, 405.0, 900.0, Vec::new(), |ui| {
            show(ui, &theme, &view, 357.0)
        });
        let texts: Vec<&str> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        assert!(
            texts.iter().any(|text| text.contains("MPV COULD NOT PLAY")),
            "error reason is inline: {texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.contains("RETRY")),
            "error state offers RETRY: {texts:?}"
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
