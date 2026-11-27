//! Player waveform: 128 seeded bars with the Nothing dot dither.
//!
//! shortcut: no real samples — mpv exposes no cheap PCM window over its IPC
//! socket, so heights come from a stable hash of the song id and are cached
//! once per context and track. Add a sample source if the engine ever offers
//! one at frame rate.

use std::sync::Arc;

use eframe::egui::{self, CursorIcon, Id, Pos2, Rect, Sense, Ui, Vec2};
use egui::WidgetInfo;
use ml_core::format_position;

use super::{dither_bar, seek_bubble, seek_gesture, ProgressSliderResponse};
use crate::theme::Theme;

/// Bar pitch: a 2 dp bar with a 1 dp gap. The bar count follows the width;
/// the reference's 120–160 bins only fit a 360–480 dp band at this pitch.
pub const PITCH: f32 = 3.0;
/// Width of one bar.
const BAR_WIDTH: f32 = 2.0;
/// Waveform band height in dp.
pub const WAVE_HEIGHT: f32 = 80.0;
/// Played bars: the design rule's upper dither bound.
const PLAYED_STRENGTH: f32 = 0.3;
/// Unplayed bars stay visible but recede.
const UNPLAYED_STRENGTH: f32 = 0.2;
/// Shortest bar so quiet passages still show a line.
const MIN_BAR: f32 = 6.0;
/// Base fill of a bar: the column must read end to end, the dither is texture
/// inside it, not its replacement.
const PLAYED_BASE: f32 = 0.55;
const UNPLAYED_BASE: f32 = 0.4;

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
    // Integer edges: a half-pixel bar grew to three feathered columns and the
    // 1 dp gaps lit up at 15 % grey.
    let left = rect.left().round();
    let center = rect.center().y.round();
    for (index, amplitude) in bars.iter().enumerate() {
        let x = left + index as f32 * PITCH;
        let height = (MIN_BAR + amplitude * (rect.height() - MIN_BAR)).round();
        let top = center - (height / 2.0).round();
        let bar = Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + BAR_WIDTH, top + height));
        let played = bin_is_played(index, bins, gesture.shown);
        // A continuous column carries the amplitude; the dot texture rides on
        // top of it. A dither-only 2 dp bar fell apart into single dots.
        let base = if played {
            app_theme.scheme.on_surface.gamma_multiply(PLAYED_BASE)
        } else {
            app_theme.scheme.on_surface_variant.gamma_multiply(UNPLAYED_BASE)
        };
        painter.rect_filled(bar, 0, base);
        dither_bar(
            painter,
            bar,
            if played {
                PLAYED_STRENGTH
            } else {
                UNPLAYED_STRENGTH
            },
        );
        // 1-px caps top and bottom: crisp ends instead of a frayed texture edge.
        let cap = if played {
            app_theme.scheme.on_surface
        } else {
            app_theme.scheme.on_surface_variant
        };
        painter.rect_filled(
            Rect::from_min_max(bar.left_top(), Pos2::new(bar.right(), bar.top() + 1.0)),
            0,
            cap,
        );
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(bar.left(), bar.bottom() - 1.0), bar.right_bottom()),
            0,
            cap,
        );
    }
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

/// Bars that fit the band at the 3 dp pitch (2 dp bar + 1 dp gap).
pub fn bins_for_width(width: f32) -> usize {
    ((width / PITCH).floor() as usize).clamp(1, 160)
}

/// Bars left of the played fraction.
pub fn bin_is_played(index: usize, bins: usize, shown: f32) -> bool {
    bins > 0 && (index as f32) < shown * bins as f32
}

/// Bar heights for `song_id`, cached per context: one build per track/width.
fn wave_bars(ctx: &egui::Context, song_id: &str, bins: usize) -> Arc<Vec<f32>> {
    let id = Id::new(("waveform-bars", song_id, bins));
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
    fn bins_follow_the_three_dp_pitch() {
        // 330 dp band (405 px window): 110 bars of 2 dp + 1 dp gap.
        assert_eq!(bins_for_width(330.0), 110);
        assert_eq!(bins_for_width(331.5), 110);
        assert_eq!(bins_for_width(460.0), 153);
        assert_eq!(bins_for_width(1.0), 1);
        assert_eq!(bins_for_width(10_000.0), 160);
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
