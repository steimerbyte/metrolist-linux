//! Custom-painted Material 3 widgets.
//!
//! Every interactive widget sets an AccessKit label where the visible label is
//! not already a stock egui widget. Painted text helpers mirror the UI probe.

mod controls;
mod feedback;
mod rows;

pub use controls::*;
pub use feedback::*;
pub use rows::*;

use eframe::egui::{self, Color32, FontId, Pos2, Stroke, TextureHandle, Ui, Vec2};
use egui::{text::LayoutJob, text::TextFormat, text::TextWrapping};

use crate::theme::{self, Theme};

/// Draw one line of text with its top-left at `at`.
pub fn text(painter: &egui::Painter, at: Pos2, value: &str, font: FontId, color: Color32) {
    let galley = painter.layout_no_wrap(value.to_owned(), font, color);
    painter.galley(at, galley, color);
}

/// Draw text truncated at `max_width` (top-left at `at`).
pub fn elided(
    painter: &egui::Painter,
    at: Pos2,
    value: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) {
    let mut job = LayoutJob::single_section(
        value.to_owned(),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width(max_width.max(4.0));
    painter.galley(at, painter.layout_job(job), color);
}

pub fn text_width(painter: &egui::Painter, value: &str, font: FontId) -> f32 {
    painter
        .layout_no_wrap(value.to_owned(), font, Color32::WHITE)
        .size()
        .x
}

/// Draw a Material Symbols ligature glyph centered at `center`.
pub fn paint_icon(painter: &egui::Painter, center: Pos2, name: &str, size: f32, color: Color32) {
    let galley = painter.layout_no_wrap(name.to_owned(), theme::icon_font(size), color);
    painter.galley(center - galley.size() / 2.0, galley, color);
}

/// UV rect that fills `target` from a `source` texture with a centre crop.
///
/// ytimg 4:3 thumbnails (`hqdefault`, `default`) carry 12.5 % black bars above
/// and below a 16:9 frame, so those bands are trimmed before the crop; a plain
/// aspect crop would keep them as letterbox bars inside the square frame.
pub fn cover_uv(source: egui::Vec2, target: egui::Vec2) -> egui::Rect {
    let mut min = egui::pos2(0.0, 0.0);
    let mut max = egui::pos2(1.0, 1.0);
    let source_aspect = source.x / source.y.max(1.0);
    if (source_aspect - 4.0 / 3.0).abs() < 0.02 {
        min.y = 0.125;
        max.y = 0.875;
    }
    let content = egui::Vec2::new(
        source.x.max(1.0) * (max.x - min.x),
        source.y.max(1.0) * (max.y - min.y),
    );
    let target_aspect = target.x / target.y.max(1.0);
    let content_aspect = content.x / content.y.max(1.0);
    if content_aspect > target_aspect {
        let visible = (target_aspect / content_aspect).clamp(0.0, 1.0);
        let inset = (1.0 - visible) * 0.5 * (max.x - min.x);
        min.x += inset;
        max.x -= inset;
    } else {
        let visible = (content_aspect / target_aspect).clamp(0.0, 1.0);
        let inset = (1.0 - visible) * 0.5 * (max.y - min.y);
        min.y += inset;
        max.y -= inset;
    }
    egui::Rect::from_min_max(min, max)
}

/// Draw a vinyl-style record: dark disc, rim, grooves, circular album crop
/// and centre spindle. The cover is a triangle fan sampling only the UV circle
/// inside the cover crop, so no square texture corners can appear.
pub fn vinyl_disc(
    ui: &Ui,
    app_theme: &Theme,
    texture: Option<&TextureHandle>,
    rect: egui::Rect,
    angle: f32,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let radius = rect.width().min(rect.height()) * 0.5;
    if radius < 2.0 {
        return;
    }
    let center = rect.center();
    let painter = ui.painter().with_clip_rect(rect);
    let white = Color32::WHITE;
    painter.circle_filled(center, radius, Color32::from_rgb(17, 16, 20));
    painter.circle_stroke(
        center,
        (radius - 0.5).max(0.0),
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 32)),
    );
    for factor in [0.96, 0.90, 0.84] {
        painter.circle_stroke(
            center,
            radius * factor,
            Stroke::new(0.8, Color32::from_rgba_unmultiplied(255, 255, 255, 20)),
        );
    }

    let cover_radius = radius * 0.78;
    if let Some(texture) = texture {
        let uv = cover_uv(texture.size_vec2(), rect.size());
        let uv_center = uv.center();
        let uv_radius = uv.width().min(uv.height()) * 0.5;
        let mut mesh = egui::Mesh::with_texture(texture.id());
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center,
            uv: uv_center,
            color: white,
        });
        const SEGMENTS: u32 = 96;
        const TAU: f32 = std::f32::consts::TAU;
        for index in 0..=SEGMENTS {
            let a = TAU * index as f32 / SEGMENTS as f32;
            let position = center + Vec2::new(a.cos(), a.sin()) * cover_radius;
            let rotated = a + angle;
            let uv = uv_center + Vec2::new(rotated.cos(), rotated.sin()) * uv_radius;
            mesh.vertices.push(egui::epaint::Vertex {
                pos: position,
                uv,
                color: white,
            });
            if index > 0 {
                mesh.add_triangle(0, index, index + 1);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
        painter.circle_stroke(
            center,
            cover_radius,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 48)),
        );
    } else {
        painter.circle_filled(
            center,
            cover_radius,
            app_theme.scheme.surface_container_high,
        );
        let primary = app_theme.scheme.primary_container;
        painter.circle_filled(
            center,
            cover_radius * 0.72,
            Color32::from_rgba_unmultiplied(primary.r(), primary.g(), primary.b(), 190),
        );
        paint_icon(
            &painter,
            center,
            "music_note",
            cover_radius * 0.55,
            app_theme.scheme.on_surface_variant,
        );
        painter.circle_stroke(
            center,
            cover_radius,
            Stroke::new(1.0, app_theme.scheme.outline_variant),
        );
    }

    let hole = (radius * 0.035).clamp(2.5, 6.0);
    painter.circle_filled(center, hole, Color32::from_rgb(9, 8, 11));
    painter.circle_stroke(
        center,
        hole,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 64)),
    );
}

/// Rounded image, or a themed placeholder when no texture is available.
pub fn art_or_placeholder(
    ui: &mut Ui,
    app_theme: &Theme,
    texture: Option<&TextureHandle>,
    rect: egui::Rect,
    radius: u8,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    if let Some(texture) = texture {
        // Cover, not letterbox: the M3 layouts are square, the thumbnails are
        // video frames with baked bars.
        let uv = cover_uv(texture.size_vec2(), rect.size());
        egui::Image::new((texture.id(), rect.size()))
            .uv(uv)
            .corner_radius(radius)
            .paint_at(ui, rect);
    } else {
        // No decoded cover: a scheme-tinted gradient keeps cards looking
        // intentional while the thumbnail download is still pending.
        let p = ui.painter();
        let clip = p.with_clip_rect(rect);
        clip.rect_filled(rect, radius, app_theme.scheme.surface_container_high);
        let radius_px = rect.width().min(rect.height()) * 0.45;
        clip.circle_filled(
            rect.center() + egui::Vec2::new(-rect.width() * 0.22, -rect.height() * 0.28),
            radius_px * 1.15,
            Color32::from_rgba_unmultiplied(
                app_theme.scheme.primary.r(),
                app_theme.scheme.primary.g(),
                app_theme.scheme.primary.b(),
                54,
            ),
        );
        clip.circle_filled(
            rect.center() + egui::Vec2::new(rect.width() * 0.26, rect.height() * 0.3),
            radius_px * 0.9,
            Color32::from_rgba_unmultiplied(
                app_theme.scheme.tertiary.r(),
                app_theme.scheme.tertiary.g(),
                app_theme.scheme.tertiary.b(),
                40,
            ),
        );
        paint_icon(
            p,
            rect.center(),
            "music_note",
            24.0,
            app_theme.scheme.on_surface_variant,
        );
    }
}

/// Composable row separator.
pub fn divider(ui: &mut Ui, app_theme: &Theme) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 0, app_theme.scheme.outline_variant);
}

/// Stroke used for hovered cards.
pub fn card_hover_stroke(app_theme: &Theme) -> Stroke {
    Stroke::new(1.0, app_theme.scheme.outline_variant)
}

#[cfg(test)]
mod tests {
    use super::cover_uv;
    use eframe::egui::Vec2;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn square_source_is_uncropped() {
        let uv = cover_uv(Vec2::splat(256.0), Vec2::splat(200.0));
        assert!(approx(uv.min.x, 0.0) && approx(uv.max.x, 1.0));
        assert!(approx(uv.min.y, 0.0) && approx(uv.max.y, 1.0));
    }

    #[test]
    fn ytimg_4x3_trims_bars_then_crops_sides() {
        // 480x360 hqdefault: 270 px of picture inside 45 px bars, square crop
        // keeps a 270x270 centre window.
        let uv = cover_uv(Vec2::new(480.0, 360.0), Vec2::splat(333.0));
        assert!(approx(uv.min.y, 0.125) && approx(uv.max.y, 0.875));
        assert!(approx(uv.min.x, 0.21875) && approx(uv.max.x, 0.78125));
    }

    #[test]
    fn no_negative_or_inverted_window() {
        for source in [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 1000.0),
            Vec2::new(1000.0, 1.0),
            Vec2::new(16.0, 9.0),
            Vec2::new(3.0, 4.0),
        ] {
            let uv = cover_uv(source, Vec2::new(10.0, 10.0));
            assert!(uv.min.x >= 0.0 && uv.max.x <= 1.0 && uv.min.x <= uv.max.x);
            assert!(uv.min.y >= 0.0 && uv.max.y <= 1.0 && uv.min.y <= uv.max.y);
        }
    }
}
