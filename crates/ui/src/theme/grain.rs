//! Nothing grain: one 128×128 dithered texture, repeated, never rebuilt.

use eframe::egui::{
    self, epaint::Vertex, Color32, ColorImage, Id, Mesh, Pos2, Shape, TextureHandle, TextureOptions,
};

const TILE: usize = 128;
const DENSITY: f32 = 0.04;
const ALPHA: u8 = 10;

/// Deterministic white specks: 4 % coverage from a fixed xorshift32 seed.
fn specks() -> Vec<Color32> {
    let mut pixels = vec![Color32::TRANSPARENT; TILE * TILE];
    let mut state = 0x9E37_79B9u32;
    let threshold = (DENSITY * (1u32 << 24) as f32) as u32;
    for pixel in &mut pixels {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        if state >> 8 < threshold {
            *pixel = Color32::from_rgba_unmultiplied(255, 255, 255, ALPHA);
        }
    }
    pixels
}

/// The grain texture, built once per context and kept in `ctx.data`.
fn texture(ctx: &egui::Context) -> TextureHandle {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_insert_with(Id::unique("nothing-grain"), || {
            ctx.load_texture(
                "nothing-grain",
                ColorImage::new([TILE, TILE], specks()),
                TextureOptions::NEAREST_REPEAT,
            )
        })
        .clone()
    })
}

/// Paint the repeating grain over `rect`; UVs exceed 1 so the texture tiles.
pub fn paint_grain(painter: &egui::Painter, rect: egui::Rect) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let texture = texture(painter.ctx());
    let uv = Pos2::new(rect.width() / TILE as f32, rect.height() / TILE as f32);
    let mut mesh = Mesh::with_texture(texture.id());
    mesh.vertices = vec![
        Vertex {
            pos: rect.left_top(),
            uv: Pos2::ZERO,
            color: Color32::WHITE,
        },
        Vertex {
            pos: rect.right_top(),
            uv: Pos2::new(uv.x, 0.0),
            color: Color32::WHITE,
        },
        Vertex {
            pos: rect.right_bottom(),
            uv,
            color: Color32::WHITE,
        },
        Vertex {
            pos: rect.left_bottom(),
            uv: Pos2::new(0.0, uv.y),
            color: Color32::WHITE,
        },
    ];
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specks_are_deterministic_and_sparse() {
        let a = specks();
        assert_eq!(a, specks());
        let lit = a.iter().filter(|pixel| pixel.a() > 0).count();
        let coverage = lit as f32 / a.len() as f32;
        assert!((0.02..0.07).contains(&coverage), "coverage {coverage}");
    }
}
