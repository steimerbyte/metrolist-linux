//! Buttons, chips, navigation items, section headers and the seek slider.

use std::hash::Hash;
use std::ops::RangeInclusive;

use eframe::egui::{
    self, Color32, CursorIcon, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use egui::WidgetInfo;
use ml_core::format_position;

use super::{paint_icon, paint_icon_swap, text, text_width};
use crate::motion;
use crate::pure::{fraction_at, progress_fraction, progress_pulse, seek_target, smooth_progress};
use crate::theme::{font_label, Theme, DP12, DP16, DP20, DP32, DP48, DP8, TOUCH};

fn paint_centered_line(
    painter: &egui::Painter,
    center: Pos2,
    value: &str,
    font: egui::FontId,
    color: Color32,
) {
    let galley = painter.layout_no_wrap(value.to_owned(), font, color);
    painter.galley(center - galley.size() / 2.0, galley, color);
}

/// Standard 40 dp icon button in the current layout.
pub fn icon_button(
    ui: &mut Ui,
    app_theme: &Theme,
    icon_name: &str,
    label: &str,
    icon_size: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(TOUCH), Sense::click());
    paint_icon_button(ui, app_theme, rect, &response, icon_name, icon_size);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

/// Icon button at an absolute rect (used by top bar, mini player, cards).
pub fn icon_button_at(
    ui: &mut Ui,
    app_theme: &Theme,
    id_salt: impl Hash + std::fmt::Debug,
    rect: Rect,
    icon_name: &str,
    label: &str,
    icon_size: f32,
) -> Response {
    let response = ui.interact(rect, ui.scope_id().with(id_salt), Sense::click());
    paint_icon_button(ui, app_theme, rect, &response, icon_name, icon_size);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

fn paint_icon_button(
    ui: &Ui,
    app_theme: &Theme,
    rect: Rect,
    response: &Response,
    icon_name: &str,
    icon_size: f32,
) {
    let scale = paint_icon_button_state(ui, app_theme, rect, response);
    paint_icon(
        ui.painter(),
        rect.center(),
        icon_name,
        icon_size * scale,
        app_theme.scheme.on_surface,
    );
}

/// Icon-coded button at an absolute rect whose glyph crossfades between two
/// names (mini-player play/pause).
pub fn icon_button_at_swap(
    ui: &mut Ui,
    app_theme: &Theme,
    id_salt: impl Hash + std::fmt::Debug,
    rect: Rect,
    from: &str,
    to: &str,
    mix: f32,
    label: &str,
    icon_size: f32,
) -> Response {
    let response = ui.interact(rect, ui.scope_id().with(id_salt), Sense::click());
    let scale = paint_icon_button_state(ui, app_theme, rect, &response);
    paint_icon_swap(
        ui.painter(),
        rect.center(),
        from,
        to,
        mix,
        icon_size * scale,
        app_theme.scheme.on_surface,
    );
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

/// Hover layer, press scale and focus ring of an icon button; returns the scale.
fn paint_icon_button_state(ui: &Ui, app_theme: &Theme, rect: Rect, response: &Response) -> f32 {
    let (hover_t, press_t) = motion::interaction(ui, response);
    let scale = motion::press_scale_at(press_t);
    if hover_t > 0.0 || press_t > 0.0 {
        let radius = ((rect.width().min(rect.height()) * 0.5 - 2.0).max(4.0)) * scale;
        let fill =
            motion::state_layer_at(app_theme.scheme.surface_container_high, hover_t, press_t);
        ui.painter().circle_filled(rect.center(), radius, fill);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    motion::focus_ring(
        ui,
        rect,
        (rect.width().min(rect.height()) * 0.5) as u8,
        response,
    );
    scale
}

/// Filter chip: 32 dp pill, hairline frame, filled when selected.
pub fn chip(ui: &mut Ui, app_theme: &Theme, label: &str, selected: bool) -> Response {
    let label = label.to_uppercase();
    let font = font_label(12.0);
    let width = text_width(ui.painter(), &label, font.clone()) + DP12 * 2.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    let selected_t = motion::t(ui, response.id.with("selected"), selected, motion::SELECT);
    let (hover_t, press_t) = motion::interaction(ui, &response);
    let scale = motion::press_scale_at(press_t);
    let draw = Rect::from_center_size(rect.center(), rect.size() * scale);
    let radius = u8::MAX;
    if selected_t > 0.0 || hover_t > 0.0 || press_t > 0.0 {
        let base = app_theme
            .scheme
            .surface
            .lerp_to_gamma(app_theme.scheme.on_surface, selected_t);
        ui.painter().rect_filled(
            draw,
            radius,
            base.lerp_to_gamma(
                app_theme.scheme.surface_container_highest,
                hover_t.max(press_t) * 0.25,
            ),
        );
    }
    let stroke = if selected_t > 0.5 {
        Stroke::new(1.0, app_theme.scheme.on_surface)
    } else {
        app_theme.hairline()
    };
    ui.painter()
        .rect_stroke(draw, radius, stroke, StrokeKind::Inside);
    let color = app_theme
        .scheme
        .on_surface_variant
        .lerp_to_gamma(app_theme.scheme.surface, selected_t);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    ui.painter()
        .galley(draw.center() - galley.size() / 2.0, galley, color);
    motion::focus_ring(ui, draw, radius, &response);
    response
        .widget_info(|| WidgetInfo::selected(egui::Role::CheckBox, true, selected, label.as_str()));
    response
}

/// Filled primary button: a pill, on_surface fill, surface text.
pub fn filled_button(
    ui: &mut Ui,
    app_theme: &Theme,
    icon_name: Option<&str>,
    label: &str,
) -> Response {
    let label = label.to_uppercase();
    let font = font_label(12.0);
    let tw = text_width(ui.painter(), &label, font.clone());
    let extra = if icon_name.is_some() { 26.0 } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(tw + extra + DP16 * 2.0, BUTTON_HEIGHT),
        Sense::click(),
    );
    let (hover_t, press_t) = motion::interaction(ui, &response);
    let scale = motion::press_scale_at(press_t);
    let draw = Rect::from_center_size(rect.center(), rect.size() * scale);
    // egui corner radii are u8, so the full pill is 255 rather than 999.
    let radius = u8::MAX;
    let fill = app_theme.scheme.on_surface.lerp_to_gamma(
        app_theme.scheme.surface_container_highest,
        hover_t * 0.12 + press_t * 0.2,
    );
    ui.painter().rect_filled(draw, radius, fill);
    let fg = app_theme.scheme.surface;
    let content_w = tw + extra;
    let start = draw.center().x - content_w / 2.0;
    if let Some(icon_name) = icon_name {
        paint_icon(
            ui.painter(),
            Pos2::new(start + 9.0, draw.center().y),
            icon_name,
            18.0 * scale,
            fg,
        );
    }
    let text_center = Pos2::new(start + extra + tw / 2.0, draw.center().y);
    paint_centered_line(ui.painter(), text_center, &label, font, fg);
    motion::focus_ring(ui, draw, radius, &response);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
    response
}

/// Body height of a filled or outlined button (matches `filled_button`).
pub const BUTTON_HEIGHT: f32 = TOUCH;

/// Width of a [`filled_button`] with `label`, for centring action rows.
pub fn filled_button_width(painter: &egui::Painter, label: &str, icon: bool) -> f32 {
    let extra = if icon { 26.0 } else { 0.0 };
    text_width(painter, &label.to_uppercase(), font_label(12.0)) + extra + DP16 * 2.0
}

/// Width of an [`outlined_button`] with `label`, for centring action rows.
pub fn outlined_button_width(painter: &egui::Painter, label: &str) -> f32 {
    text_width(painter, &label.to_uppercase(), font_label(12.0)) + DP16 * 2.0
}

/// Outlined secondary button: a pill with a hairline frame and no fill.
pub fn outlined_button(ui: &mut Ui, app_theme: &Theme, label: &str) -> Response {
    let label = label.to_uppercase();
    let font = font_label(12.0);
    let tw = text_width(ui.painter(), &label, font.clone());
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(tw + DP16 * 2.0, BUTTON_HEIGHT), Sense::click());
    let (hover_t, press_t) = motion::interaction(ui, &response);
    let scale = motion::press_scale_at(press_t);
    let draw = Rect::from_center_size(rect.center(), rect.size() * scale);
    let radius = u8::MAX;
    let t = hover_t.max(press_t);
    if t > 0.0 {
        ui.painter().rect_filled(
            draw,
            radius,
            app_theme
                .scheme
                .surface_container_high
                .gamma_multiply(t * 0.85),
        );
    }
    ui.painter()
        .rect_stroke(draw, radius, app_theme.hairline(), StrokeKind::Inside);
    paint_centered_line(
        ui.painter(),
        draw.center(),
        &label,
        font,
        app_theme.scheme.on_surface,
    );
    motion::focus_ring(ui, draw, radius, &response);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
    response
}

/// Section label with optional "Play all" pill.
pub fn section_header(
    ui: &mut Ui,
    app_theme: &Theme,
    title: &str,
    width: f32,
    play_all: bool,
) -> Option<Response> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, DP48), Sense::hover());
    text(
        ui.painter(),
        rect.left_top() + Vec2::new(0.0, DP8),
        &title.to_uppercase(),
        font_label(12.0),
        app_theme.scheme.on_surface_variant,
    );
    if !play_all {
        return None;
    }
    let label = "PLAY ALL";
    let font = font_label(11.0);
    let tw = text_width(ui.painter(), label, font.clone());
    let button_rect = Rect::from_min_size(
        Pos2::new(rect.right() - (tw + DP20 * 2.0), rect.top() + DP8),
        Vec2::new(tw + DP20 * 2.0, DP32),
    );
    let response = ui.interact(
        button_rect,
        ui.scope_id().with(("section_play_all", title)),
        Sense::click(),
    );
    let (hover_t, press_t) = motion::interaction(ui, &response);
    let scale = motion::press_scale_at(press_t);
    let draw = Rect::from_center_size(button_rect.center(), button_rect.size() * scale);
    let radius = u8::MAX;
    let t = hover_t.max(press_t);
    if t > 0.0 {
        ui.painter().rect_filled(
            draw,
            radius,
            app_theme
                .scheme
                .surface_container_high
                .gamma_multiply(t * 0.85),
        );
    }
    ui.painter()
        .rect_stroke(draw, radius, app_theme.hairline(), StrokeKind::Inside);
    paint_centered_line(
        ui.painter(),
        draw.center(),
        label,
        font,
        app_theme.scheme.on_surface,
    );
    motion::focus_ring(ui, draw, radius, &response);
    response
        .widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, format!("Play all {title}")));
    Some(response)
}

/// Seek slider: 2 dp line with a 12×12 thumb.
pub fn slider(
    ui: &mut Ui,
    app_theme: &Theme,
    value: &mut f32,
    range: RangeInclusive<f32>,
    width: f32,
    label: &str,
) -> Response {
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(width, TOUCH), Sense::click_and_drag());
    let min = *range.start();
    let max = *range.end();
    if max > min && (response.is_pointer_button_down_on() || response.clicked()) {
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            let track_left = rect.left() + 8.0;
            let track_width = (rect.width() - 16.0).max(1.0);
            let t = ((pointer.x - track_left) / track_width).clamp(0.0, 1.0);
            let new_value = min + t * (max - min);
            if (*value - new_value).abs() > f32::EPSILON {
                *value = new_value;
                response.mark_changed();
            }
        }
    }
    let value_norm = if max > min {
        ((*value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - 16.0, 2.0));
    let painter = ui.painter();
    painter.rect_filled(track, 0, app_theme.scheme.surface_container_highest);
    let active = Rect::from_min_size(
        track.left_top(),
        Vec2::new(track.width() * value_norm, track.height()),
    );
    painter.rect_filled(active, 0, app_theme.scheme.on_surface);
    let thumb_center = Pos2::new(track.left() + track.width() * value_norm, rect.center().y);
    painter.circle_filled(thumb_center, 6.0, app_theme.scheme.on_surface);
    response.widget_info(|| {
        WidgetInfo::labeled(egui::Role::Slider, true, format!("{label} {value_norm:.2}"))
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    response
}

/// Animated player timeline: smooth fill/thumb, hover time bubble and a
/// buffering pulse. The drawn value only ever approaches the real position;
/// pressing or dragging snaps it to the pointer so seeking stays pixel-exact.
#[derive(Clone, Copy)]
struct ProgressAnim {
    shown: f32,
}

pub struct ProgressSliderResponse {
    pub response: Response,
    /// Seek target in milliseconds when the user released a click or drag.
    pub seek: Option<u64>,
}

pub fn progress_slider(
    ui: &mut Ui,
    app_theme: &Theme,
    position_ms: u64,
    duration_ms: u64,
    buffering: bool,
    width: f32,
) -> ProgressSliderResponse {
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(width, TOUCH), Sense::click_and_drag());
    let track = Rect::from_center_size(
        rect.center(),
        Vec2::new((rect.width() - 16.0).max(1.0), 2.0),
    );
    let pointer = ui.input(|input| input.pointer.interact_pos());
    let dragging = response.is_pointer_button_down_on();
    let mut target = progress_fraction(position_ms, duration_ms);
    let mut seek = None;
    // Follow the pointer while pressed, but seek once on release: an mpv seek
    // per drag frame floods the engine. A plain click seeks on release too.
    let seek_now = response.clicked() || response.drag_stopped();
    if duration_ms > 0 && (dragging || seek_now) {
        if let Some(pointer) = pointer {
            let fraction = fraction_at(pointer.x, track.left(), track.width());
            target = fraction;
            if seek_now {
                seek = Some(seek_target(fraction, duration_ms));
                response.mark_changed();
            }
        }
    }

    let dt = ui.input(|input| input.stable_dt);
    let shown = ui.data_mut(|data| {
        let anim = data.get_temp_mut_or_insert_with(response.id, || ProgressAnim { shown: target });
        if dragging || seek.is_some() {
            anim.shown = target;
        } else if target < anim.shown - 0.01 {
            // A rewind, repeat or previous-track jump snaps; only forward
            // position updates are smoothed.
            anim.shown = target;
        } else {
            anim.shown = smooth_progress(anim.shown, target, dt);
        }
        anim.shown
    });
    if seek.is_none() && (shown - target).abs() > 0.0005 {
        ui.ctx().request_repaint();
    }
    if buffering {
        ui.ctx().request_repaint();
    }

    let painter = ui.painter();
    painter.rect_filled(track, 0, app_theme.scheme.surface_container_high);
    if buffering {
        let pulse = progress_pulse(ui.input(|input| input.time) as f32);
        painter.rect_filled(
            track,
            0,
            app_theme
                .scheme
                .surface_container_highest
                .lerp_to_gamma(app_theme.scheme.on_surface_variant, 0.25 + 0.35 * pulse),
        );
    }
    let active = Rect::from_min_size(
        track.left_top(),
        Vec2::new(track.width() * shown, track.height()),
    );
    painter.rect_filled(active, 0, app_theme.scheme.on_surface);
    let thumb_center = Pos2::new(track.left() + track.width() * shown, rect.center().y);
    painter.circle_filled(thumb_center, 6.0, app_theme.scheme.on_surface);

    let hover_preview = if (response.hovered() || dragging) && duration_ms > 0 {
        pointer.map(|pointer| {
            let fraction = fraction_at(pointer.x, track.left(), track.width());
            (pointer, seek_target(fraction, duration_ms))
        })
    } else {
        None
    };
    if let Some((pointer, preview_ms)) = hover_preview {
        let label = format_position(preview_ms);
        let font = font_label(11.0);
        let bubble = Vec2::new(text_width(painter, &label, font.clone()) + 16.0, 22.0);
        let left = (pointer.x - bubble.x / 2.0)
            .clamp(rect.left(), (rect.right() - bubble.x).max(rect.left()));
        let bubble_rect = Rect::from_min_size(
            Pos2::new(left, (rect.top() - bubble.y - 6.0).max(0.0)),
            bubble,
        );
        painter.rect_filled(bubble_rect, 0, app_theme.scheme.surface_container_high);
        painter.rect_stroke(bubble_rect, 0, app_theme.hairline(), StrokeKind::Inside);
        let galley = painter.layout_no_wrap(label, font, app_theme.scheme.on_surface);
        painter.galley(
            bubble_rect.center() - galley.size() / 2.0,
            galley,
            app_theme.scheme.on_surface,
        );
    }

    response.widget_info(|| {
        WidgetInfo::labeled(
            egui::Role::Slider,
            true,
            format!(
                "Seek {} / {}",
                format_position(position_ms),
                format_position(duration_ms)
            ),
        )
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    ProgressSliderResponse { response, seek }
}

/// Circular filled icon button (play/pause FAB, 56 dp default).
pub fn filled_icon_button(
    ui: &mut Ui,
    app_theme: &Theme,
    icon_name: &str,
    label: &str,
    size: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let scale = paint_filled_face(ui, app_theme, &response, rect, size);
    paint_icon(
        ui.painter(),
        rect.center(),
        icon_name,
        size * 0.5 * scale,
        app_theme.scheme.surface,
    );
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

/// Draw the animated circle of a filled icon button; returns the press scale.
fn paint_filled_face(
    ui: &Ui,
    app_theme: &Theme,
    response: &Response,
    rect: Rect,
    size: f32,
) -> f32 {
    let (hover_t, press_t) = motion::interaction(ui, response);
    let scale = motion::press_scale_at(press_t);
    let fill = app_theme.scheme.on_surface.lerp_to_gamma(
        app_theme.scheme.surface_container_highest,
        hover_t * 0.12 + press_t * 0.2,
    );
    ui.painter()
        .circle_filled(rect.center(), size * 0.5 * scale, fill);
    motion::focus_ring(ui, rect, (size * 0.5) as u8, response);
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    scale
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{
        CentralPanel, Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect,
    };
    use ml_core::Settings;

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
    fn progress_slider_seek_matches_the_pointer_pixel() {
        let ctx = Context::default();
        let theme = Theme::from_settings(&Settings::default());
        let mut seeks = Vec::new();
        // 300 dp slider inside the 400 dp panel: track spans x 16..=300.
        let pos = Pos2::new(158.0, 24.0);
        for events in press_release(pos) {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 100.0))),
                events,
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    let response = progress_slider(ui, &theme, 0, 120_000, false, 300.0);
                    if let Some(seek) = response.seek {
                        seeks.push(seek);
                    }
                });
            });
            output.textures_delta.clear();
        }
        assert!(!seeks.is_empty(), "pressing the track seeks");
        assert!(
            seeks.iter().all(|seek| *seek == 60_000),
            "mid-track click seeks to half: {seeks:?}"
        );
    }

    #[test]
    fn progress_slider_seeks_once_when_a_drag_is_released() {
        let ctx = Context::default();
        let theme = Theme::from_settings(&Settings::default());
        let mut seeks = Vec::new();
        let press = Pos2::new(80.0, 24.0);
        let end = Pos2::new(300.0, 24.0);
        let frames = [
            vec![Event::PointerMoved(press)],
            vec![
                Event::PointerMoved(press),
                Event::PointerButton {
                    pos: press,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::default(),
                },
            ],
            vec![Event::PointerMoved(end)],
            vec![
                Event::PointerMoved(end),
                Event::PointerButton {
                    pos: end,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::default(),
                },
            ],
        ];
        for (index, events) in frames.into_iter().enumerate() {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 100.0))),
                events,
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    let response = progress_slider(ui, &theme, 0, 120_000, false, 300.0);
                    if let Some(seek) = response.seek {
                        seeks.push(seek);
                    }
                });
            });
            output.textures_delta.clear();
            if index < 3 {
                assert!(seeks.is_empty(), "no seek before release: {seeks:?}");
            }
        }
        assert_eq!(seeks, vec![120_000], "one seek to the drag end on release");
    }

    #[test]
    fn progress_slider_handles_missing_duration_without_fake_position() {
        let ctx = Context::default();
        let theme = Theme::from_settings(&Settings::default());
        let pos = Pos2::new(200.0, 24.0);
        for events in press_release(pos) {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 100.0))),
                events,
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    let response = progress_slider(ui, &theme, 0, 0, true, 300.0);
                    assert!(response.seek.is_none());
                });
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn icon_button_state_layer_animates_hover_and_press() {
        let ctx = Context::default();
        let theme = Theme::from_settings(&Settings::default());
        let base = theme.scheme.surface_container_high;
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 120.0));
        let run = |time: f64, events: Vec<Event>| {
            let mut rect = Rect::NOTHING;
            let input = RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                events,
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    rect = icon_button(ui, &theme, "settings", "Settings", 22.0).rect;
                });
            });
            output.textures_delta.clear();
            let circle = output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Circle(circle) if circle.radius > 10.0 => {
                        Some((circle.fill, circle.radius))
                    }
                    _ => None,
                });
            (rect, circle)
        };

        // Warm-up frames: no pointer, no state layer.
        let mut button = Rect::NOTHING;
        for frame in 0..3 {
            let (rect, circle) = run(frame as f64 * 0.05, Vec::new());
            button = rect;
            assert!(circle.is_none());
        }
        let hover_pos = button.center();
        // Hover settles in HOVER (0.10 s).
        for frame in 0..3 {
            run(
                0.2 + frame as f64 * 0.05,
                if frame == 0 {
                    vec![Event::PointerMoved(hover_pos)]
                } else {
                    Vec::new()
                },
            );
        }
        let (hover_fill, hover_radius) = run(0.4, Vec::new())
            .1
            .expect("hover paints the state layer");
        assert_eq!(
            hover_fill,
            motion::state_layer_at(base, 1.0, 0.0),
            "hover reaches the 8 % tint"
        );

        // Press in and hold until the press animation settles.
        run(
            0.45,
            vec![Event::PointerButton {
                pos: hover_pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::default(),
            }],
        );
        for frame in 0..3 {
            run(0.5 + frame as f64 * 0.05, Vec::new());
        }
        let (press_fill, press_radius) = run(0.65, Vec::new())
            .1
            .expect("pressed paints the state layer");
        assert_eq!(
            press_fill,
            motion::state_layer_at(base, 1.0, 1.0),
            "press reaches the 12 % tint"
        );
        assert!(
            (press_radius - hover_radius * motion::PRESS_SCALE).abs() < 0.05,
            "press scales the circle: {press_radius} vs {hover_radius}"
        );
    }
}
