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

/// Cell of the fine band grain: 2 pt, one 1 px dot per cell.
const BAND_TILE_PT: f32 = 2.0;

/// The 4x4 ordered Bayer matrix (0-15), thresholded at half: 8 of 16 cells
/// carry a 1 px dot. Deterministic, ordered, no noise.
fn bayer4(x: usize, y: usize) -> u8 {
    const M: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    M[y & 3][x & 3]
}

/// Fine ordered grain for the waveform band: a 2 pt Bayer checker at
/// `strength`, tiled behind the bars so no bar falls apart into dots.
pub fn bayer_band(painter: &Painter, rect: Rect, strength: f32) {
    if strength <= 0.0 || !rect.is_positive() {
        return;
    }
    let ctx = painter.ctx();
    let texels = (BAND_TILE_PT * ctx.pixels_per_point())
        .round()
        .clamp(2.0, 16.0) as usize;
    let dark = ctx.theme() == eframe::egui::Theme::Dark;
    let id = Id::unique(("nothing-bayer-band", dark, texels));
    let texture = match ctx.data(|data| data.get_temp::<TextureHandle>(id)) {
        Some(handle) => handle,
        None => {
            let mut pixels = vec![Color32::TRANSPARENT; texels * texels];
            for y in 0..texels {
                for x in 0..texels {
                    if bayer4(x, y) < 8 {
                        pixels[y * texels + x] = dot(dark, 255);
                    }
                }
            }
            let handle = ctx.load_texture(
                "nothing-bayer-band",
                ColorImage::new([texels, texels], pixels),
                TextureOptions {
                    wrap_mode: TextureWrapMode::Repeat,
                    ..TextureOptions::NEAREST
                },
            );
            ctx.data_mut(|data| data.insert_temp(id, handle.clone()));
            handle
        }
    };
    let alpha = (strength.clamp(0.0, MAX_STRENGTH) * 255.0).round() as u8;
    let uv = Rect::from_min_size(Pos2::ZERO, rect.size() / BAND_TILE_PT);
    let mut mesh = Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(rect, uv, dot(dark, alpha));
    painter.add(eframe::egui::Shape::mesh(mesh));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_band_grain_is_an_ordered_checker() {
        let mut seen = [false; 16];
        let mut lit = 0;
        for y in 0..4 {
            for x in 0..4 {
                let value = bayer4(x, y);
                assert!(!seen[value as usize], "threshold {value} twice");
                seen[value as usize] = true;
                if value < 8 {
                    lit += 1;
                }
            }
        }
        assert_eq!(lit, 8, "half the 2 pt cells carry a dot");
    }

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
