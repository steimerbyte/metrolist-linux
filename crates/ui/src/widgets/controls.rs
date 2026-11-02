//! Buttons, chips, navigation items, section headers and the seek slider.

use std::hash::Hash;
use std::ops::RangeInclusive;

use eframe::egui::{
    self, Color32, CursorIcon, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use egui::WidgetInfo;

use super::{paint_icon, text, text_width};
use crate::theme::{
    bold_font, medium_font, state_layer, Theme, BODY_SMALL, DP12, DP16, DP20, DP32, DP48, DP8,
    LABEL_LARGE, SHAPE_SMALL, TITLE_LARGE, TOUCH,
};

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
    let painter = ui.painter();
    if response.hovered() || response.is_pointer_button_down_on() {
        let radius = (rect.width().min(rect.height()) * 0.5 - 2.0).max(4.0);
        let fill = state_layer(
            app_theme.scheme.surface_container_high,
            response.hovered(),
            response.is_pointer_button_down_on(),
        );
        painter.circle_filled(rect.center(), radius, fill);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    paint_icon(
        painter,
        rect.center(),
        icon_name,
        icon_size,
        app_theme.scheme.on_surface,
    );
}

/// M3 filter chip (32 dp, radius 8).
pub fn chip(ui: &mut Ui, app_theme: &Theme, label: &str, selected: bool) -> Response {
    let font = medium_font(LABEL_LARGE);
    let width = text_width(ui.painter(), label, font.clone()) + DP12 * 2.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    let base = if selected {
        app_theme.scheme.secondary_container
    } else {
        app_theme.scheme.surface_container_high
    };
    let fill = state_layer(
        base,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );
    ui.painter().rect_filled(rect, SHAPE_SMALL, fill);
    let color = if selected {
        app_theme.scheme.on_secondary_container
    } else {
        app_theme.scheme.on_surface_variant
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    response.widget_info(|| WidgetInfo::selected(egui::Role::CheckBox, true, selected, label));
    response
}

/// Filled primary button with optional leading icon.
pub fn filled_button(
    ui: &mut Ui,
    app_theme: &Theme,
    icon_name: Option<&str>,
    label: &str,
) -> Response {
    let font = medium_font(LABEL_LARGE);
    let tw = text_width(ui.painter(), label, font.clone());
    let extra = if icon_name.is_some() { 26.0 } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(tw + extra + DP16 * 2.0, BUTTON_HEIGHT),
        Sense::click(),
    );
    let fill = state_layer(
        app_theme.scheme.primary,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );
    ui.painter().rect_filled(rect, 20, fill);
    let fg = app_theme.scheme.on_primary;
    let content_w = tw + extra;
    let start = rect.center().x - content_w / 2.0;
    if let Some(icon_name) = icon_name {
        paint_icon(
            ui.painter(),
            Pos2::new(start + 9.0, rect.center().y),
            icon_name,
            18.0,
            fg,
        );
    }
    let text_center = Pos2::new(start + extra + tw / 2.0, rect.center().y);
    paint_centered_line(ui.painter(), text_center, label, font, fg);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

/// Body height of a filled or outlined button (matches `filled_button`).
pub const BUTTON_HEIGHT: f32 = TOUCH;

/// Width of a [`filled_button`] with `label`, for centring action rows.
pub fn filled_button_width(painter: &egui::Painter, label: &str, icon: bool) -> f32 {
    let extra = if icon { 26.0 } else { 0.0 };
    text_width(painter, label, medium_font(LABEL_LARGE)) + extra + DP16 * 2.0
}

/// Width of an [`outlined_button`] with `label`, for centring action rows.
pub fn outlined_button_width(painter: &egui::Painter, label: &str) -> f32 {
    text_width(painter, label, medium_font(LABEL_LARGE)) + DP16 * 2.0
}

/// Outlined pill button ("Play all"), same height as [`filled_button`].
pub fn outlined_button(ui: &mut Ui, app_theme: &Theme, label: &str) -> Response {
    let font = medium_font(LABEL_LARGE);
    let tw = text_width(ui.painter(), label, font.clone());
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(tw + DP16 * 2.0, BUTTON_HEIGHT), Sense::click());
    let fill = if response.hovered() {
        app_theme.scheme.surface_container_high
    } else {
        app_theme.bg()
    };
    let rounding = (BUTTON_HEIGHT * 0.5) as u8;
    ui.painter().rect_filled(rect, rounding, fill);
    ui.painter().rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0, app_theme.scheme.outline),
        StrokeKind::Inside,
    );
    paint_centered_line(
        ui.painter(),
        rect.center(),
        label,
        font,
        app_theme.scheme.on_surface,
    );
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

/// Section title with optional "Play all" pill.
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
        title,
        bold_font(TITLE_LARGE),
        app_theme.scheme.on_surface,
    );
    if !play_all {
        return None;
    }
    let label = "Play all";
    let font = medium_font(LABEL_LARGE);
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
    let fill = if response.hovered() {
        app_theme.scheme.surface_container_high
    } else {
        app_theme.bg()
    };
    ui.painter().rect_filled(button_rect, 16, fill);
    ui.painter().rect_stroke(
        button_rect,
        16,
        Stroke::new(1.0, app_theme.scheme.outline),
        StrokeKind::Inside,
    );
    paint_centered_line(
        ui.painter(),
        button_rect.center(),
        label,
        font,
        app_theme.scheme.on_surface,
    );
    response
        .widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, format!("Play all {title}")));
    Some(response)
}

/// Bottom-nav / rail item: selected pill + 24 dp icon + label.
pub fn nav_item(
    ui: &mut Ui,
    app_theme: &Theme,
    icon_name: &str,
    label: &str,
    selected: bool,
    size: Vec2,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let center_x = rect.center().x;
    let pill_center = Pos2::new(center_x, rect.top() + 24.0);
    let pill = Rect::from_center_size(pill_center, Vec2::new(56.0, 32.0));
    if selected {
        ui.painter()
            .rect_filled(pill, 16, app_theme.scheme.secondary_container);
    } else if response.hovered() {
        ui.painter().rect_filled(
            pill,
            16,
            state_layer(app_theme.bg(), true, response.is_pointer_button_down_on()),
        );
    }
    let icon_color = if selected {
        app_theme.scheme.on_secondary_container
    } else {
        app_theme.scheme.on_surface_variant
    };
    paint_icon(ui.painter(), pill_center, icon_name, 24.0, icon_color);
    let label_color = if selected {
        app_theme.scheme.on_surface
    } else {
        app_theme.scheme.on_surface_variant
    };
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), medium_font(BODY_SMALL), label_color);
    ui.painter().galley(
        Pos2::new(center_x - galley.size().x / 2.0, rect.top() + 46.0),
        galley,
        label_color,
    );
    response.widget_info(|| WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    response
}

/// M3 seek slider (4 dp track, 12 dp thumb).
pub fn slider(
    ui: &mut Ui,
    app_theme: &Theme,
    value: &mut f32,
    range: RangeInclusive<f32>,
    width: f32,
    label: &str,
) -> Response {
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(width, DP32), Sense::click_and_drag());
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
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - 16.0, 4.0));
    let painter = ui.painter();
    painter.rect_filled(track, 2, app_theme.scheme.surface_container_highest);
    let active = Rect::from_min_size(
        track.left_top(),
        Vec2::new(track.width() * value_norm, track.height()),
    );
    painter.rect_filled(active, 2, app_theme.scheme.primary);
    let thumb_center = Pos2::new(track.left() + track.width() * value_norm, rect.center().y);
    let thumb_radius = if response.hovered() || response.dragged() {
        7.0
    } else {
        6.0
    };
    painter.circle_filled(thumb_center, thumb_radius, app_theme.scheme.primary);
    response.widget_info(|| {
        WidgetInfo::labeled(egui::Role::Slider, true, format!("{label} {value_norm:.2}"))
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    response
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
    let fill = state_layer(
        app_theme.scheme.primary,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );
    ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    paint_icon(
        ui.painter(),
        rect.center(),
        icon_name,
        size * 0.5,
        app_theme.scheme.on_primary,
    );
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}
