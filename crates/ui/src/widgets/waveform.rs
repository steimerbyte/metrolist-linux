//! Player waveform: hollow bars, 1 px outline, ordered Bayer dots inside.
//!
//! shortcut: no real samples — mpv exposes no cheap PCM window over its IPC
//! socket, so heights come from a stable hash of the song id and are cached
//! once per context and track. Add a sample source if the engine ever offers
//! one at frame rate.

use std::sync::Arc;

use eframe::egui::{self, Color32, CursorIcon, Id, Mesh, Pos2, Rect, Sense, Ui, Vec2};
use egui::WidgetInfo;
use ml_core::format_position;

use super::{seek_bubble, seek_gesture, ProgressSliderResponse};
use crate::theme::Theme;

/// Bar pitch: a 4 dp bar (outline, hollow inside) with a 1 dp gap.
pub const PITCH: f32 = 5.0;
/// Width of one bar: two 1 px outlines around two interior columns.
const BAR_WIDTH: f32 = 4.0;
/// Interior columns: the ordered matrix needs more than one column to read.
const INTERIOR: usize = 2;
/// Waveform band height in dp.
pub const WAVE_HEIGHT: f32 = 80.0;
/// Dot density at the top and the bottom of every bar.
const DENSITY_TOP: f32 = 0.5;
const DENSITY_BOTTOM: f32 = 0.1;
/// Shortest bar so quiet passages still show a bar.
const MIN_BAR: f32 = 6.0;
/// Unplayed outlines recede but stay readable on black.
const UNPLAYED_OUTLINE: Color32 = Color32::from_gray(95);

/// Seek control drawn as a dithered bar waveform. Returns the same response
/// shape as the line slider it replaces.
pub fn waveform_slider(
    ui: &mut Ui,
    app_theme: &Theme,
    song_id: &str,
    position_ms: u64,
    duration_ms: u64,
    width: f32,
) -> ProgressSliderResponse {
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(width, WAVE_HEIGHT), Sense::click_and_drag());
    let gesture = seek_gesture(ui, &mut response, position_ms, duration_ms, rect);
    let bins = bins_for_width(rect.width());
    let bars = wave_bars(ui.ctx(), song_id, bins);
    let painter = ui.painter();
    // Integer edges: a half-pixel bar grew to three feathered columns.
    let left = rect.left().round();
    let center = rect.center().y.round();
    let band_top = (center - (rect.height() / 2.0).round()) as usize;
    let band_rows = rect.height().max(2.0) as usize;
    let bayer = bayer16();
    let mut played_edges = Mesh::default();
    let mut unplayed_edges = Mesh::default();
    let mut dots = Mesh::default();
    for (index, amplitude) in bars.iter().enumerate() {
        let x = left + index as f32 * PITCH;
        let height = (MIN_BAR + amplitude * (rect.height() - MIN_BAR)).round().max(3.0) as usize;
        let top = (center - (height as f32 / 2.0).round()) as usize;
        let played = bin_is_played(index, bins, gesture.shown);
        let outline = if played {
            &mut played_edges
        } else {
            &mut unplayed_edges
        };
        let edge = if played {
            app_theme.scheme.on_surface
        } else {
            UNPLAYED_OUTLINE
        };
        // Hollow: four 1 px edges, black inside, dots only between them.
        let pixel = |x: f32, y: usize| {
            Rect::from_min_size(Pos2::new(x, y as f32), Vec2::splat(1.0))
        };
        for row in 0..height {
            outline.add_colored_rect(pixel(x, top + row), edge);
            outline.add_colored_rect(pixel(x + BAR_WIDTH - 1.0, top + row), edge);
        }
        for column in 1..BAR_WIDTH as usize - 1 {
            outline.add_colored_rect(pixel(x + column as f32, top), edge);
            outline.add_colored_rect(pixel(x + column as f32, top + height - 1), edge);
        }
        // Two-level dots, ordered matrix, half at the top and a tenth at the bottom.
        for column in 1..=INTERIOR {
            for row in 1..height.saturating_sub(1) {
                let value = bayer[(top + row) & 15][(x as usize + column) & 15];
                let row_in_band = top + row - band_top;
                if dot_lit(row_in_band, band_rows, value) {
                    dots.add_colored_rect(pixel(x + column as f32, top + row), Color32::WHITE);
                }
            }
        }
    }
    painter.add(egui::Shape::mesh(played_edges));
    painter.add(egui::Shape::mesh(unplayed_edges));
    painter.add(egui::Shape::mesh(dots));
    if let Some((pointer, preview_ms)) = gesture.hover {
        seek_bubble(painter, app_theme, rect, pointer, preview_ms);
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
    ProgressSliderResponse {
        response,
        seek: gesture.seek,
    }
}

/// Bars that fit the band at the 5 dp pitch (4 dp bar + 1 dp gap).
pub fn bins_for_width(width: f32) -> usize {
    ((width / PITCH).floor() as usize).clamp(1, 160)
}

/// The ordered 16x16 Bayer matrix: every threshold 0-255 exactly once.
fn bayer16() -> [[u8; 16]; 16] {
    let mut matrix = [[0u8; 16]; 16];
    for (y, row) in matrix.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            let mut value = 0u32;
            for bit in (0..4).rev() {
                let xb = (x >> bit) & 1;
                let yb = (y >> bit) & 1;
                value = (value << 2) | (((xb ^ yb) as u32) << 1) | yb as u32;
            }
            *cell = value as u8;
        }
    }
    matrix
}

/// Dot on/off for one interior pixel: 50 % at the top of the band, 10 % at the
/// bottom, always decided by the ordered matrix so there is no noise.
fn dot_lit(row: usize, rows: usize, bayer: u8) -> bool {
    let span = rows.saturating_sub(1).max(1) as f32;
    let t = row as f32 / span;
    let density = DENSITY_TOP + (DENSITY_BOTTOM - DENSITY_TOP) * t;
    (bayer as f32) < density * 256.0
}

/// Bars left of the played fraction.
pub fn bin_is_played(index: usize, bins: usize, shown: f32) -> bool {
    bins > 0 && (index as f32) < shown * bins as f32
}

/// Bar heights for `song_id`, cached per context: one build per track/width.
fn wave_bars(ctx: &egui::Context, song_id: &str, bins: usize) -> Arc<Vec<f32>> {
    let id = Id::unique(("waveform-bars", song_id, bins));
    if let Some(bars) = ctx.data(|data| data.get_temp::<Arc<Vec<f32>>>(id)) {
        return bars;
    }
    let bars = Arc::new(generate_bars(song_id, bins));
    ctx.data_mut(|data| data.insert_temp(id, bars.clone()));
    bars
}

/// Deterministic pseudo-waveform in 0.18..=1.0: FNV-1a seed, LCG noise, 3-tap
/// smoothing and a peak normalisation so the tallest bar touches the top.
pub fn generate_bars(seed: &str, bins: usize) -> Vec<f32> {
    let mut state = 0xcbf2_9ce4_8422_2325u64;
    for byte in seed.as_bytes() {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x100_0000_01b3);
    }
    let mut raw = Vec::with_capacity(bins);
    for _ in 0..bins {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        raw.push(((state >> 40) as f32) / ((1u64 << 24) - 1) as f32);
    }
    let smoothed: Vec<f32> = (0..bins)
        .map(|index| {
            let before = raw[index.saturating_sub(1)];
            let after = raw[(index + 1).min(bins.saturating_sub(1))];
            (before + 2.0 * raw[index] + after) / 4.0
        })
        .collect();
    let peak = smoothed.iter().copied().fold(0.0f32, f32::max).max(0.001);
    smoothed
        .into_iter()
        .map(|value| (0.18 + 0.82 * value / peak).min(1.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{
        CentralPanel, Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2,
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
    fn bins_follow_the_five_dp_pitch() {
        // 330 dp band (405 px window): 66 bars of 4 dp + 1 dp gap.
        assert_eq!(bins_for_width(330.0), 66);
        assert_eq!(bins_for_width(331.5), 66);
        assert_eq!(bins_for_width(460.0), 92);
        assert_eq!(bins_for_width(1.0), 1);
        assert_eq!(bins_for_width(10_000.0), 160);
    }

    #[test]
    fn the_bayer_matrix_is_ordered_and_complete() {
        let matrix = bayer16();
        let mut seen = [false; 256];
        for row in matrix {
            for value in row {
                assert!(!seen[value as usize], "threshold {value} twice");
                seen[value as usize] = true;
            }
        }
        assert!(seen.iter().all(|hit| *hit), "every threshold appears");
        // The 2x2 sub-cell is the classic [[0, 2], [3, 1]].
        assert_eq!([matrix[0][0], matrix[0][8], matrix[8][0], matrix[8][8]], [0, 128, 192, 64]);
    }

    #[test]
    fn dots_thin_out_from_half_to_a_tenth() {
        let top = (0..256).filter(|b| dot_lit(0, 80, *b as u8)).count();
        let bottom = (0..256).filter(|b| dot_lit(79, 80, *b as u8)).count();
        assert_eq!(top, 128, "the top row is half lit");
        assert_eq!(bottom, 26, "the bottom row is a tenth lit");
        let mean = (0..80)
            .map(|row| (0..256).filter(|b| dot_lit(row, 80, *b as u8)).count())
            .sum::<usize>() as f32
            / (80.0 * 256.0);
        assert!((mean - 0.3).abs() < 0.01, "interior mean {mean}");
    }

    #[test]
    fn bars_are_deterministic_and_seeded() {
        let bins = bins_for_width(330.0);
        let a = generate_bars("oWqnJdGy_84", bins);
        let b = generate_bars("oWqnJdGy_84", bins);
        assert_eq!(a, b, "same song, same bars");
        assert_eq!(a.len(), bins);
        assert!(a.iter().all(|value| *value > 0.0 && *value <= 1.0));
        let peak = a.iter().copied().fold(0.0f32, f32::max);
        assert!((peak - 1.0).abs() < 1e-4, "peak normalised: {peak}");
        assert_ne!(a, generate_bars("another-song", bins));
    }

    #[test]
    fn played_bins_follow_the_shown_fraction() {
        assert!(!bin_is_played(0, 128, 0.0));
        assert!(bin_is_played(0, 128, 0.5));
        assert!(bin_is_played(63, 128, 0.5));
        assert!(!bin_is_played(64, 128, 0.5));
        assert!(bin_is_played(127, 128, 1.0));
        assert!(!bin_is_played(0, 0, 0.5));
    }

    #[test]
    fn a_drag_seeks_once_when_released() {
        let ctx = Context::default();
        let theme = Theme::from_settings(&Settings::default());
        let mut seeks = Vec::new();
        let press = Pos2::new(80.0, 40.0);
        let end = Pos2::new(308.0, 40.0);
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
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(400.0, 200.0),
                )),
                events,
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    let response =
                        waveform_slider(ui, &theme, "oWqnJdGy_84", 0, 120_000, 300.0);
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
    fn a_click_without_duration_never_seeks() {
        let ctx = Context::default();
        let theme = Theme::from_settings(&Settings::default());
        let pos = Pos2::new(200.0, 40.0);
        for events in press_release(pos) {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(400.0, 200.0),
                )),
                events,
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    let response = waveform_slider(ui, &theme, "oWqnJdGy_84", 0, 0, 300.0);
                    assert!(response.seek.is_none());
                });
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn a_mid_waveform_click_seeks_to_half() {
        let ctx = Context::default();
        let theme = Theme::from_settings(&Settings::default());
        let mut seeks = Vec::new();
        // 300 dp waveform inside the 400 dp panel: x 8..=308.
        let pos = Pos2::new(158.0, 40.0);
        for events in press_release(pos) {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(400.0, 200.0),
                )),
                events,
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    let response =
                        waveform_slider(ui, &theme, "oWqnJdGy_84", 0, 120_000, 300.0);
                    if let Some(seek) = response.seek {
                        seeks.push(seek);
                    }
                });
            });
            output.textures_delta.clear();
        }
        assert!(!seeks.is_empty(), "pressing the waveform seeks");
        assert!(
            seeks.iter().all(|seek| *seek == 60_000),
            "50 % of a 2:00 track is 1:00: {seeks:?}"
        );
    }
}
