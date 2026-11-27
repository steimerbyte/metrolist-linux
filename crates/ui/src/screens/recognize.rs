//! Song recognition: explicit microphone capture, Shazam match and actions.

use std::time::Instant;

use eframe::egui::{self, Rect, RichText, Sense, StrokeKind, Ui, Vec2};
use ml_recognition::RecognitionResult;

use crate::art::ArtCache;
use crate::pure::{
    recognition_copy_text, recognition_meta, recognition_progress, recognition_search_query,
    recognition_subtitle, recognition_timer_text, recognition_timing_text,
    recognition_youtube_music_url,
};
use crate::services::Services;
use crate::state::{RecognitionMatch, RecognitionState, RECOGNITION_SECONDS};
use crate::theme::{
    bold_font, font_label, regular_font, Theme, DP12, DP16, DP4, DP8, SHAPE_LARGE, TITLE_LARGE,
};
use crate::widgets::{art_or_placeholder, filled_button, outlined_button};

use super::{measured_column, Intent};

fn card(theme: &Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.scheme.surface_container)
        .stroke(theme.hairline())
        .corner_radius(SHAPE_LARGE)
        .inner_margin(egui::Margin::same(DP16 as i8))
}

fn caps(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(font_label(11.0))
            .color(theme.scheme.on_surface_variant),
    );
}

fn heading(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .font(bold_font(TITLE_LARGE))
            .color(theme.scheme.on_surface),
    );
}

fn note(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
    );
}

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    state: &RecognitionState,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let width = width.min(640.0);
    measured_column(ui, width, |ui| {
        ui.add_space(DP4);
        match state {
            RecognitionState::Idle => idle(ui, theme, &mut intents),
            RecognitionState::Listening { started } => listening(ui, theme, *started, &mut intents),
            RecognitionState::Processing => processing(ui, theme, &mut intents),
            RecognitionState::Success { matched } => {
                success(ui, theme, services, art, matched, &mut intents)
            }
            RecognitionState::Error { message } => error(ui, theme, message, &mut intents),
        }
    });
    intents
}

fn idle(ui: &mut Ui, theme: &Theme, intents: &mut Vec<Intent>) {
    card(theme).show(ui, |ui| {
        caps(ui, theme, "Recognize");
        ui.add_space(DP8);
        heading(ui, theme, "Recognize what's playing");
        ui.add_space(DP4);
        note(
            ui,
            theme,
            "Record a 12-second sample from your default microphone.",
        );
        note(
            ui,
            theme,
            "Audio is fingerprinted and discarded; nothing is saved.",
        );
        ui.add_space(DP16);
        if filled_button(ui, theme, None, "Start listening").clicked() {
            intents.push(Intent::StartRecognition);
        }
    });
    ui.add_space(DP8);
    note(
        ui,
        theme,
        "The microphone only opens after you press Start listening.",
    );
}

fn listening(ui: &mut Ui, theme: &Theme, started: Instant, intents: &mut Vec<Intent>) {
    let sample_ms = (f64::from(RECOGNITION_SECONDS) * 1000.0) as u64;
    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let progress = recognition_progress(elapsed_ms, sample_ms);
    let timer = recognition_timer_text(elapsed_ms, sample_ms);
    card(theme).show(ui, |ui| {
        ui.horizontal(|ui| {
            caps(ui, theme, "Listening");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(timer.as_str())
                        .font(font_label(11.0))
                        .color(theme.scheme.on_surface),
                );
            });
        });
        ui.add_space(DP8);
        note(ui, theme, "Play the music near your microphone.");
        ui.add_space(DP12);
        let (bar, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 2.0), Sense::hover());
        ui.painter()
            .rect_filled(bar, 0, theme.scheme.outline_variant);
        let filled = Rect::from_min_size(bar.min, Vec2::new(bar.width() * progress, bar.height()));
        ui.painter().rect_filled(filled, 0, theme.scheme.on_surface);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Label, true, format!("Listening, {timer}"))
        });
        ui.add_space(DP16);
        if outlined_button(ui, theme, "Cancel").clicked() {
            intents.push(Intent::CancelRecognition);
        }
    });
    ui.add_space(DP8);
    note(
        ui,
        theme,
        "Cancel closes the microphone before the sample ends.",
    );
}

fn processing(ui: &mut Ui, theme: &Theme, intents: &mut Vec<Intent>) {
    card(theme).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::Spinner::new()
                    .size(14.0)
                    .color(theme.scheme.on_surface_variant),
            );
            caps(ui, theme, "Identifying");
        });
        ui.add_space(DP8);
        note(ui, theme, "Comparing the sample with Shazam.");
        ui.add_space(DP16);
        if outlined_button(ui, theme, "Cancel").clicked() {
            intents.push(Intent::CancelRecognition);
        }
    });
}

fn success(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    matched: &RecognitionMatch,
    intents: &mut Vec<Intent>,
) {
    let result = &matched.result;
    let artwork = result
        .artwork_hq_url
        .as_deref()
        .or(result.artwork_url.as_deref())
        .unwrap_or("");
    card(theme).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            let (art_rect, _) = ui.allocate_exact_size(Vec2::splat(124.0), Sense::hover());
            let texture = art.texture(ui.ctx(), services, artwork);
            art_or_placeholder(ui, theme, texture.as_ref(), art_rect, 0);
            ui.painter()
                .rect_stroke(art_rect, 0, theme.hairline(), StrokeKind::Inside);
            ui.add_space(DP16);
            ui.vertical(|ui| {
                caps(ui, theme, "Shazam match");
                ui.add_space(DP4);
                let title = if result.title.trim().is_empty() {
                    "Recognised track"
                } else {
                    result.title.trim()
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(title)
                            .font(bold_font(20.0))
                            .color(theme.scheme.on_surface),
                    )
                    .truncate(),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(recognition_subtitle(result))
                            .font(regular_font(14.0))
                            .color(theme.scheme.on_surface_variant),
                    )
                    .truncate(),
                );
                let meta = recognition_meta(result);
                if !meta.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(meta)
                                .font(regular_font(12.0))
                                .color(theme.scheme.on_surface_variant),
                        )
                        .truncate(),
                    );
                }
                ui.add_space(DP4);
                ui.label(
                    RichText::new(recognition_timing_text(
                        matched.sample_ms,
                        matched.elapsed_ms,
                    ))
                    .font(font_label(11.0))
                    .color(theme.scheme.on_surface_variant),
                );
            });
        });
    });
    ui.add_space(DP16);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        let query = recognition_search_query(result);
        if filled_button(ui, theme, None, "Play").clicked() {
            intents.push(Intent::RecognitionPlay {
                query: query.clone(),
            });
        }
        if outlined_button(ui, theme, "Open in YouTube Music").clicked() {
            ui.ctx()
                .open_url(egui::OpenUrl::new_tab(recognition_youtube_music_url(
                    &query,
                )));
        }
        if outlined_button(ui, theme, "Copy").clicked() {
            ui.ctx().copy_text(recognition_copy_text(result));
        }
    });
    ui.add_space(DP12);
    let detail = if matched.elapsed_ms > 0 {
        format!(
            "Matched in {:.1}s. Confidence is not included in the Shazam response.",
            matched.elapsed_ms as f64 / 1000.0
        )
    } else {
        "Confidence is not included in the Shazam response.".to_string()
    };
    note(ui, theme, &detail);
}

fn error(ui: &mut Ui, theme: &Theme, message: &str, intents: &mut Vec<Intent>) {
    egui::Frame::new()
        .stroke(theme.hairline())
        .corner_radius(SHAPE_LARGE)
        .inner_margin(egui::Margin::same(DP16 as i8))
        .show(ui, |ui| {
            ui.label(
                RichText::new("RECOGNITION FAILED")
                    .font(font_label(11.0))
                    .color(theme.scheme.error),
            );
            ui.add_space(DP8);
            ui.label(
                RichText::new(message)
                    .font(regular_font(13.0))
                    .color(theme.scheme.on_surface_variant),
            );
            ui.add_space(DP16);
            if filled_button(ui, theme, None, "Try again").clicked() {
                intents.push(Intent::StartRecognition);
            }
        });
}

/// Deterministic fixture used by `--demo recognize-success`.
pub fn demo_match() -> RecognitionMatch {
    RecognitionMatch {
        result: RecognitionResult {
            track_key: "demo-recognition".to_string(),
            title: "Demo track".to_string(),
            artist: "Demo artist".to_string(),
            album: Some("Demo album".to_string()),
            genre: Some("Pop".to_string()),
            release_date: Some("2026-11-02".to_string()),
            label: Some("Demo label".to_string()),
            shazam_url: Some("https://www.shazam.com/track/demo".to_string()),
            ..RecognitionResult::default()
        },
        sample_ms: 12_000,
        elapsed_ms: 1_400,
    }
}

pub fn demo_error() -> String {
    "No match found. Move closer to the music and try again.".to_string()
}

#[cfg(test)]
mod tests {
    use eframe::egui::{self, Ui};
    use ml_core::Settings;

    use super::*;
    use crate::settings_store::SettingsPaths;

    fn services() -> (Services, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "metrolist-recognize-render-{}-{}",
            std::process::id(),
            ml_core::now_millis()
        ));
        let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
        let ctx = egui::Context::default();
        let (services, _notes) = Services::new(&ctx, paths, &Settings::default());
        (services, root)
    }

    fn render_at(
        width: f32,
        height: f32,
        mut draw: impl FnMut(&mut Ui, &mut ArtCache) -> Vec<Intent>,
    ) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, height),
            )),
            ..Default::default()
        };
        let mut art = ArtCache::new();
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    actions = draw(ui, &mut art);
                });
            });
        });
        output.textures_delta.clear();
        assert!(actions.is_empty(), "unexpected recognition action");
    }

    #[test]
    fn recognition_states_render_at_both_sizes() {
        let (services, root) = services();
        let theme = Theme::from_settings(&Settings::default());
        let states = [
            RecognitionState::Idle,
            RecognitionState::Listening {
                started: Instant::now(),
            },
            RecognitionState::Processing,
            RecognitionState::Success {
                matched: Box::new(demo_match()),
            },
            RecognitionState::Error {
                message: demo_error(),
            },
        ];
        for (width, height) in [(405.0, 900.0), (1100.0, 760.0)] {
            for state in &states {
                render_at(width, height, |ui, art| {
                    show(ui, &theme, &services, art, state, width)
                });
            }
        }
        drop(services);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn demo_match_has_honest_fields() {
        let matched = demo_match();
        assert_eq!(matched.result.title, "Demo track");
        assert_eq!(matched.sample_ms, 12_000);
        assert_eq!(matched.elapsed_ms, 1_400);
        assert!(matched.result.artwork_url.is_none());
    }
}
