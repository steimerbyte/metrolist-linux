//! Song recognition: explicit microphone capture, Shazam match and actions.

use std::time::Instant;

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Ui, Vec2};
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
    bold_font, medium_font, regular_font, Theme, DP12, DP16, DP4, DP8, TITLE_LARGE, TITLE_MEDIUM,
};
use crate::widgets::{art_or_placeholder, filled_button, outlined_button, paint_icon};

use super::{measured_column, Intent};

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
    measured_column(ui, 640.0, |ui| {
        ui.add_space(DP4);
        match state {
            RecognitionState::Idle => idle(ui, theme, width, &mut intents),
            RecognitionState::Listening { started } => {
                listening(ui, theme, *started, width, &mut intents)
            }
            RecognitionState::Processing => processing(ui, theme, &mut intents),
            RecognitionState::Success { matched } => {
                success(ui, theme, services, art, matched, width, &mut intents)
            }
            RecognitionState::Error { message } => error(ui, theme, message, &mut intents),
        }
    });
    intents
}

fn idle(ui: &mut Ui, theme: &Theme, width: f32, intents: &mut Vec<Intent>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 204.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 20, theme.scheme.surface_container);
    ui.painter().circle_filled(
        Pos2::new(rect.center().x, rect.top() + 58.0),
        38.0,
        theme.scheme.secondary_container,
    );
    paint_icon(
        ui.painter(),
        Pos2::new(rect.center().x, rect.top() + 58.0),
        "mic",
        32.0,
        theme.scheme.on_secondary_container,
    );
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 108.0),
        Align2::CENTER_TOP,
        "Recognize what's playing",
        bold_font(TITLE_LARGE),
        theme.scheme.on_surface,
    );
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 138.0),
        Align2::CENTER_TOP,
        "Record a 12-second sample from your default microphone.",
        regular_font(13.0),
        theme.scheme.on_surface_variant,
    );
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 158.0),
        Align2::CENTER_TOP,
        "Audio is fingerprinted and discarded; nothing is saved.",
        regular_font(12.0),
        theme.scheme.on_surface_variant,
    );
    ui.add_space(DP16);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if filled_button(ui, theme, Some("mic"), "Start listening").clicked() {
            intents.push(Intent::StartRecognition);
        }
    });
    ui.add_space(DP8);
    ui.label(
        egui::RichText::new("The microphone only opens after you press Start listening.")
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
    );
}

fn listening(ui: &mut Ui, theme: &Theme, started: Instant, width: f32, intents: &mut Vec<Intent>) {
    let sample_ms = (f64::from(RECOGNITION_SECONDS) * 1000.0) as u64;
    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let progress = recognition_progress(elapsed_ms, sample_ms);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 214.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 20, theme.scheme.surface_container);
    paint_icon(
        ui.painter(),
        Pos2::new(rect.left() + 36.0, rect.top() + 42.0),
        "mic",
        32.0,
        theme.scheme.primary,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 64.0, rect.top() + 24.0),
        Align2::LEFT_TOP,
        "Listening…",
        bold_font(TITLE_LARGE),
        theme.scheme.on_surface,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 64.0, rect.top() + 55.0),
        Align2::LEFT_TOP,
        "Play the music near your microphone.",
        regular_font(13.0),
        theme.scheme.on_surface_variant,
    );
    let timer = recognition_timer_text(elapsed_ms, sample_ms);
    ui.painter().text(
        Pos2::new(rect.right() - DP16, rect.top() + 28.0),
        Align2::RIGHT_TOP,
        &timer,
        medium_font(13.0),
        theme.scheme.on_surface,
    );
    let bar = Rect::from_min_size(
        Pos2::new(rect.left() + DP16, rect.top() + 96.0),
        Vec2::new((rect.width() - DP16 * 2.0).max(10.0), 8.0),
    );
    ui.painter()
        .rect_filled(bar, 4, theme.scheme.surface_container_highest);
    let filled = Rect::from_min_size(bar.min, Vec2::new(bar.width() * progress, bar.height()));
    ui.painter().rect_filled(filled, 4, theme.scheme.primary);
    let (progress_rect, response) = ui.allocate_exact_size(Vec2::new(1.0, 1.0), Sense::hover());
    let _ = progress_rect;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Label, true, format!("Listening, {timer}"))
    });
    ui.add_space(DP16);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if outlined_button(ui, theme, "Cancel").clicked() {
            intents.push(Intent::CancelRecognition);
        }
    });
    ui.add_space(DP8);
    ui.label(
        egui::RichText::new("Cancel closes the microphone before the sample ends.")
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
    );
}

fn processing(ui: &mut Ui, theme: &Theme, intents: &mut Vec<Intent>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 214.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 20, theme.scheme.surface_container);
    let spinner = egui::Spinner::new().size(38.0).color(theme.scheme.primary);
    ui.put(
        Rect::from_center_size(
            Pos2::new(rect.center().x, rect.top() + 58.0),
            Vec2::splat(42.0),
        ),
        spinner,
    );
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 92.0),
        Align2::CENTER_TOP,
        "Identifying…",
        bold_font(TITLE_LARGE),
        theme.scheme.on_surface,
    );
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 122.0),
        Align2::CENTER_TOP,
        "Comparing the sample with Shazam.",
        regular_font(13.0),
        theme.scheme.on_surface_variant,
    );
    ui.add_space(DP16);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
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
    width: f32,
    intents: &mut Vec<Intent>,
) {
    let result = &matched.result;
    let artwork = result
        .artwork_hq_url
        .as_deref()
        .or(result.artwork_url.as_deref())
        .unwrap_or("");
    let (card, _) = ui.allocate_exact_size(Vec2::new(width, 156.0), Sense::hover());
    ui.painter()
        .rect_filled(card, 20, theme.scheme.surface_container);
    let art_rect = Rect::from_min_size(card.left_top() + Vec2::new(DP16, DP16), Vec2::splat(124.0));
    let texture = art.texture(ui.ctx(), services, artwork);
    art_or_placeholder(ui, theme, texture.as_ref(), art_rect, 12);
    let text_x = art_rect.right() + DP16;
    let text_width = (card.right() - DP16 - text_x).max(40.0);
    let title = if result.title.trim().is_empty() {
        "Recognised track"
    } else {
        result.title.trim()
    };
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, card.top() + DP16),
        title,
        bold_font(TITLE_LARGE),
        theme.scheme.on_surface,
        text_width,
    );
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, card.top() + 46.0),
        &recognition_subtitle(result),
        regular_font(14.0),
        theme.scheme.on_surface_variant,
        text_width,
    );
    let meta = recognition_meta(result);
    if !meta.is_empty() {
        crate::widgets::elided(
            ui.painter(),
            Pos2::new(text_x, card.top() + 68.0),
            &meta,
            regular_font(12.0),
            theme.scheme.on_surface_variant,
            text_width,
        );
    }
    let badge = Rect::from_min_size(Pos2::new(text_x, card.top() + 96.0), Vec2::new(102.0, 24.0));
    ui.painter()
        .rect_filled(badge, 12, theme.scheme.secondary_container);
    ui.painter().text(
        badge.center(),
        Align2::CENTER_CENTER,
        "Shazam match",
        medium_font(11.0),
        theme.scheme.on_secondary_container,
    );
    ui.painter().text(
        Pos2::new(text_x, card.top() + 126.0),
        Align2::LEFT_TOP,
        recognition_timing_text(matched.sample_ms, matched.elapsed_ms),
        regular_font(11.0),
        theme.scheme.on_surface_variant,
    );

    ui.add_space(DP16);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        let query = recognition_search_query(result);
        if filled_button(ui, theme, Some("play_arrow"), "Play").clicked() {
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
    ui.label(
        egui::RichText::new(detail)
            .font(regular_font(11.0))
            .color(theme.scheme.on_surface_variant),
    );
}

fn error(ui: &mut Ui, theme: &Theme, message: &str, intents: &mut Vec<Intent>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 180.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 20, theme.scheme.error_container);
    paint_icon(
        ui.painter(),
        Pos2::new(rect.center().x, rect.top() + 40.0),
        "error",
        32.0,
        theme.scheme.on_error_container,
    );
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 76.0),
        Align2::CENTER_TOP,
        "Recognition failed",
        bold_font(TITLE_MEDIUM),
        theme.scheme.on_error_container,
    );
    let message_rect = Rect::from_min_max(
        Pos2::new(rect.left() + DP16, rect.top() + 96.0),
        Pos2::new(rect.right() - DP16, rect.bottom() - DP8),
    );
    ui.put(
        message_rect,
        egui::Label::new(
            egui::RichText::new(message)
                .font(regular_font(13.0))
                .color(theme.scheme.on_error_container),
        )
        .wrap(),
    );
    ui.add_space(DP16);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if filled_button(ui, theme, Some("refresh"), "Try again").clicked() {
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
