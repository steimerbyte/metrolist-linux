//! Lyrics view: offset/romanised display, active line and optional translation.

use eframe::egui::{self, Ui};
use ml_core::{LyricsDocument, LyricsLine, Song};

use crate::pure::lyrics_offset_label;
use crate::state::Load;
use crate::theme::{medium_font, regular_font, Theme, DP12, DP4, DP8};
use crate::widgets::{chip, empty_state, outlined_button};

use super::{error_block, measured_column, Intent};

/// Everything the lyrics view needs beyond the loaded document.
pub struct LyricsOptions<'a> {
    /// Document after romanisation and per-song offset are applied.
    pub display: Option<&'a LyricsDocument>,
    pub translation: &'a Load<()>,
    pub active_ms: u64,
    pub offset_ms: i64,
    pub romanize: bool,
    pub translation_available: bool,
    pub show_translation: bool,
}

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    song: Option<&Song>,
    lyrics: &Load<LyricsDocument>,
    options: LyricsOptions<'_>,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let Some(song) = song else {
        empty_state(
            ui,
            theme,
            "lyrics",
            "Nothing playing",
            "Lyrics follow the current track.",
        );
        return intents;
    };

    ui.add_space(DP8);
    ui.label(
        egui::RichText::new(&song.title)
            .font(medium_font(18.0))
            .color(theme.scheme.on_surface),
    );
    ui.label(
        egui::RichText::new(song.artist_names())
            .font(regular_font(13.0))
            .color(theme.scheme.on_surface_variant),
    );
    ui.add_space(DP12);

    if lyrics.loading && lyrics.data.is_none() {
        ui.label(
            egui::RichText::new("Looking for lyrics…")
                .font(regular_font(14.0))
                .color(theme.scheme.on_surface_variant),
        );
        return intents;
    }

    if let Some(error) = &lyrics.error {
        if lyrics.data.is_none() {
            if error_block(ui, theme, error) {
                intents.push(Intent::StartLyrics(song.clone()));
            }
            return intents;
        }
    }

    let Some(base) = lyrics.data.as_ref() else {
        empty_state(
            ui,
            theme,
            "lyrics",
            "No lyrics",
            "No provider returned lyrics for this track.",
        );
        return intents;
    };

    let document = options.display.unwrap_or(base);
    let has_lines = !document.lines.is_empty();
    toolbar(ui, theme, &options, has_lines, &mut intents);
    if let Some(error) = &options.translation.error {
        ui.label(
            egui::RichText::new(error)
                .font(regular_font(12.0))
                .color(theme.scheme.error),
        );
    }
    ui.add_space(DP8);

    if !has_lines {
        if let Some(plain) = &document.plain {
            ui.label(egui::RichText::new(plain).font(regular_font(15.0)));
            return intents;
        }
        empty_state(
            ui,
            theme,
            "lyrics",
            "No lyrics",
            &format!("{} returned an empty document.", document.provider),
        );
        return intents;
    }

    let active = ml_lyrics::active_line_at(document, options.active_ms);
    measured_column(ui, 640.0, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("lyrics_scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (index, line) in document.lines.iter().enumerate() {
                    let is_active = active == Some(index);
                    let color = if line.background {
                        if is_active {
                            theme.scheme.secondary
                        } else {
                            theme.scheme.outline
                        }
                    } else if is_active {
                        theme.scheme.primary
                    } else {
                        theme.scheme.on_surface
                    };
                    let fill = if is_active {
                        theme.scheme.surface_container_high
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    egui::Frame::new()
                        .fill(fill)
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(DP8 as i8, DP4 as i8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let mut text = egui::RichText::new(&line.text)
                                .font(regular_font(15.0))
                                .color(color);
                            if is_active {
                                text = text.strong();
                            }
                            ui.add(egui::Label::new(text).wrap());
                            if options.show_translation {
                                if let Some(translated) = document
                                    .translated
                                    .as_ref()
                                    .and_then(|lines| lines.get(index))
                                    .filter(|line| !line.text.trim().is_empty())
                                {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&translated.text)
                                                .font(regular_font(13.0))
                                                .color(theme.scheme.on_surface_variant),
                                        )
                                        .wrap(),
                                    );
                                }
                            }
                        });
                    ui.add_space(DP8);
                }
            });
    });
    intents
}

fn toolbar(
    ui: &mut Ui,
    theme: &Theme,
    options: &LyricsOptions<'_>,
    has_lines: bool,
    intents: &mut Vec<Intent>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if chip(ui, theme, "Romanize", options.romanize).clicked() {
            intents.push(Intent::ToggleLyricsRomanize);
        }
        if has_lines && options.translation_available {
            if options.translation.loading {
                ui.label(
                    egui::RichText::new("Translating…")
                        .font(regular_font(12.0))
                        .color(theme.scheme.on_surface_variant),
                );
            } else if outlined_button(ui, theme, "Translate").clicked() {
                intents.push(Intent::TranslateLyrics);
            }
        }
    });
    if has_lines {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.label(
                egui::RichText::new(format!("Offset {}", lyrics_offset_label(options.offset_ms)))
                    .font(regular_font(12.0))
                    .color(theme.scheme.on_surface_variant),
            );
            if outlined_button(ui, theme, "-0.5 s").clicked() {
                intents.push(Intent::AdjustLyricsOffset(-500));
            }
            if outlined_button(ui, theme, "+0.5 s").clicked() {
                intents.push(Intent::AdjustLyricsOffset(500));
            }
            if outlined_button(ui, theme, "Reset").clicked() {
                intents.push(Intent::ResetLyricsOffset);
            }
        });
    }
}

/// Deterministic fixture for `--demo lyrics` and render tests. Japanese text is
/// romanised by the real backend; the translation is pre-filled and padded.
pub fn demo_document() -> LyricsDocument {
    let lines = vec![
        line(0, 4_000, "こんにちは世界"),
        line(4_000, 8_000, "夜空の歌"),
        line(8_000, 12_000, "君と歩く"),
        line(12_000, 16_000, "また明日"),
    ];
    let translated = lines
        .iter()
        .zip([
            "Hello, world",
            "Song of the night sky",
            "Walking with you",
            "See you tomorrow",
        ])
        .map(|(line, text)| LyricsLine {
            text: text.to_string(),
            ..line.clone()
        })
        .collect();
    LyricsDocument {
        provider: "demo".to_string(),
        lines,
        translated: Some(translated),
        ..LyricsDocument::default()
    }
}

fn line(start_ms: u64, end_ms: u64, text: &str) -> LyricsLine {
    LyricsLine {
        start_ms,
        end_ms: Some(end_ms),
        text: text.to_string(),
        ..LyricsLine::default()
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{self, Ui};

    use super::*;
    use crate::state::Load;

    fn fixture_load() -> Load<LyricsDocument> {
        Load {
            data: Some(demo_document()),
            ..Load::default()
        }
    }

    fn fixture_display() -> LyricsDocument {
        let mut display = ml_lyrics::romanized(&demo_document()).expect("Japanese fixture");
        ml_lyrics::apply_offset(&mut display, 500);
        display
    }

    fn render_at(width: f32, height: f32, mut draw: impl FnMut(&mut Ui) -> Vec<Intent>) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(width, height),
            )),
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    actions = draw(ui);
                });
            });
        });
        output.textures_delta.clear();
        assert!(actions.is_empty(), "unexpected lyrics action");
    }

    #[test]
    fn romanized_display_keeps_word_timing_and_pads_translation() {
        let mut doc = demo_document();
        doc.lines[0].words.push(ml_core::LyricsWord {
            start_ms: 500,
            end_ms: 1_000,
            text: "こんにちは".to_string(),
        });
        let romanized = ml_lyrics::romanized(&doc).expect("Japanese romanization");
        assert_eq!(romanized.lines[0].text, "konnichiha sekai");
        assert_eq!(romanized.lines[0].words[0].start_ms, 500);
        assert_eq!(romanized.lines[0].words[0].text, "こんにちは");
        ml_lyrics::translated(&mut doc, vec!["only one".to_string()]);
        let translated = doc.translated.expect("translated lines");
        assert_eq!(translated.len(), doc.lines.len());
        assert_eq!(translated[0].text, "only one");
        assert_eq!(translated[1].text, "");
    }

    #[test]
    fn cjk_korean_and_indic_romanization_fixtures() {
        for (text, expected) in [
            ("你好", "ni hao"),
            ("한국어", "hangugeo"),
            ("नमस्ते", "nmste"),
        ] {
            let doc = LyricsDocument {
                lines: vec![LyricsLine {
                    start_ms: 0,
                    text: text.to_string(),
                    ..LyricsLine::default()
                }],
                ..LyricsDocument::default()
            };
            let romanized = ml_lyrics::romanized(&doc).expect("script recognised");
            assert_eq!(romanized.lines[0].text, expected, "fixture {text}");
        }
    }

    #[test]
    fn offset_changes_display_timing_without_reordering() {
        let display = fixture_display();
        assert_eq!(display.lines[0].start_ms, 500);
        assert_eq!(display.lines[1].start_ms, 4_500);
        assert!(display
            .lines
            .windows(2)
            .all(|pair| pair[0].start_ms <= pair[1].start_ms));
        let mut shifted = display.clone();
        ml_lyrics::apply_offset(&mut shifted, -1_000);
        assert_eq!(shifted.lines[0].start_ms, 0);
        assert!(shifted
            .lines
            .windows(2)
            .all(|pair| pair[0].start_ms <= pair[1].start_ms));
    }

    #[test]
    fn lyrics_fixture_renders_at_both_sizes() {
        let theme = Theme::from_settings(&ml_core::Settings::default());
        let song = Song {
            title: "Demo lyrics".to_string(),
            artists: vec![ml_core::Artist::new("Demo artist", None)],
            ..Song::default()
        };
        let lyrics = fixture_load();
        let display = fixture_display();
        let translation = Load::<()>::default();
        for (width, height) in [(405.0, 900.0), (1100.0, 760.0)] {
            render_at(width, height, |ui| {
                show(
                    ui,
                    &theme,
                    Some(&song),
                    &lyrics,
                    LyricsOptions {
                        display: Some(&display),
                        translation: &translation,
                        active_ms: 5_000,
                        offset_ms: 500,
                        romanize: true,
                        translation_available: true,
                        show_translation: true,
                    },
                )
            });
        }
    }

    #[test]
    fn translation_error_renders_with_document() {
        let theme = Theme::from_settings(&ml_core::Settings::default());
        let song = Song {
            title: "Demo lyrics".to_string(),
            ..Song::default()
        };
        let lyrics = fixture_load();
        let display = fixture_display();
        let translation = Load::<()> {
            error: Some(crate::pure::TRANSLATION_MISSING_KEY_MESSAGE.to_string()),
            ..Load::default()
        };
        render_at(405.0, 900.0, |ui| {
            show(
                ui,
                &theme,
                Some(&song),
                &lyrics,
                LyricsOptions {
                    display: Some(&display),
                    translation: &translation,
                    active_ms: 5_000,
                    offset_ms: 500,
                    romanize: true,
                    translation_available: true,
                    show_translation: false,
                },
            )
        });
    }
}
