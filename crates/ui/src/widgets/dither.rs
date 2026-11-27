//! Nothing dot dither: one 4 pt cell raster, tiled, built once per context.
//!
//! Ported from the ophanim reference (4×4 dot cell, strength 0–0.3). The
//! texture is cached in `ctx.data` per (dark, texels) key, so a frame only
//! adds one textured quad per painted rect.

use eframe::egui::{
    Color32, ColorImage, Context, Id, Mesh, Painter, Pos2, Rect, TextureHandle, TextureOptions,
    TextureWrapMode,
};

/// One dither cell in points (the Nothing spec's 4 px pattern).
const TILE_PT: f32 = 4.0;
/// Upper bound from the design rule; anything denser reads as a fill.
const MAX_STRENGTH: f32 = 0.3;

/// The dot raster: a soft dot centred in an `texels`² cell.
fn dither_image(texels: usize, dark: bool) -> ColorImage {
    let n = texels;
    let mut pixels = vec![Color32::TRANSPARENT; n * n];
    let center = n as f32 / 2.0;
    let radius = n as f32 * 0.175;
    for y in 0..n {
        for x in 0..n {
            let distance = ((x as f32 + 0.5 - center).powi(2) + (y as f32 + 0.5 - center).powi(2)).sqrt();
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            pixels[y * n + x] = dot(dark, (coverage * 255.0).round() as u8);
        }
    }
    ColorImage::new([n, n], pixels)
}

/// White dots on dark surfaces, black dots on light ones.
fn dot(dark: bool, alpha: u8) -> Color32 {
    if dark {
        Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha)
    } else {
        Color32::from_black_alpha(alpha)
    }
}

/// The cached tile; repeated calls within a context never rebuild it.
fn texture(ctx: &Context, texels: usize, dark: bool) -> TextureHandle {
    let id = Id::unique(("nothing-dither", dark, texels));
    if let Some(handle) = ctx.data(|data| data.get_temp::<TextureHandle>(id)) {
        return handle;
    }
    let handle = ctx.load_texture(
        "nothing-dither",
        dither_image(texels, dark),
        TextureOptions {
            wrap_mode: TextureWrapMode::Repeat,
            ..TextureOptions::NEAREST
        },
    );
    ctx.data_mut(|data| data.insert_temp(id, handle.clone()));
    handle
}

/// Paint the dot raster over `rect`; `strength` is clamped to 0–0.3.
pub fn dither_rect(painter: &Painter, rect: Rect, strength: f32) {
    if strength <= 0.0 || !rect.is_positive() {
        return;
    }
    let texels = (TILE_PT * painter.ctx().pixels_per_point())
        .round()
        .clamp(4.0, 64.0) as usize;
    let dark = painter.ctx().theme() == eframe::egui::Theme::Dark;
    let texture = texture(painter.ctx(), texels, dark);
    let alpha = (strength.clamp(0.0, MAX_STRENGTH) * 255.0).round() as u8;
    let uv = Rect::from_min_size(Pos2::ZERO, rect.size() / TILE_PT);
    let mut mesh = Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(rect, uv, dot(dark, alpha));
    painter.add(eframe::egui::Shape::mesh(mesh));
}

/// Horizontal span of the dot core inside one cell. A 1.5 dp waveform bar
/// cannot carry a whole 4 pt cell, so it samples this column and repeats only
/// along y: every bar shows the same dot rhythm instead of a random slice.
const DOT_U0: f32 = 0.375;
const DOT_U1: f32 = 0.625;

/// One dot-dithered vertical bar: the dither texture pinned to the dot column.
pub fn dither_bar(painter: &Painter, rect: Rect, strength: f32) {
    if strength <= 0.0 || !rect.is_positive() {
        return;
    }
    let texels = (TILE_PT * painter.ctx().pixels_per_point())
        .round()
        .clamp(4.0, 64.0) as usize;
    let dark = painter.ctx().theme() == eframe::egui::Theme::Dark;
    let texture = texture(painter.ctx(), texels, dark);
    let alpha = (strength.clamp(0.0, MAX_STRENGTH) * 255.0).round() as u8;
    let uv = Rect::from_min_max(
        Pos2::new(DOT_U0, 0.0),
        Pos2::new(DOT_U1, rect.height() / TILE_PT),
    );
    let mut mesh = Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(rect, uv, dot(dark, alpha));
    painter.add(eframe::egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tile_is_a_centred_dot() {
        let image = dither_image(4, true);
        assert_eq!(image.size, [4, 4]);
        assert!(image.pixels[5].a() > 0, "centre texel carries the dot");
        assert_eq!(image.pixels[0].a(), 0, "corner stays clear");
    }

    #[test]
    fn the_texture_is_built_once_per_context() {
        let ctx = Context::default();
        let before = ctx.tex_manager().read().num_allocated();
        let first = texture(&ctx, 4, true);
        let second = texture(&ctx, 4, true);
        assert_eq!(first.id(), second.id(), "second call reuses the handle");
        assert_eq!(ctx.tex_manager().read().num_allocated(), before + 1);
    }
}
