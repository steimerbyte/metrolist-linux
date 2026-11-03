//! Liked songs route.

use eframe::egui::Ui;
use ml_core::{QueueSource, Song};

use crate::art::ArtCache;
use crate::services::Services;
use crate::state::Load;
use crate::theme::{bold_font, regular_font, Theme, DP24, DP4, DP8, TITLE_LARGE};
use crate::widgets::{empty_state, filled_button, outlined_button};

use super::{error_block, skeleton_rows, song_list, Intent};

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    liked: &Load<Vec<Song>>,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();

    if liked.initial_loading() {
        skeleton_rows(ui, theme, width);
        ui.add_space(DP24);
        return intents;
    }
    if let Some(error) = &liked.error {
        if liked.data.is_none() {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
            ui.add_space(DP24);
            return intents;
        }
    }
    let Some(songs) = liked.data.as_ref() else {
        empty_state(
            ui,
            theme,
            "favorite",
            "Liked songs",
            "Songs you like appear here.",
        );
        ui.add_space(DP24);
        return intents;
    };

    ui.add_space(DP4);
    ui.label(
        egui::RichText::new("Liked songs")
            .font(bold_font(TITLE_LARGE))
            .color(theme.scheme.on_surface),
    );
    ui.label(
        egui::RichText::new(format!(
            "{} {}",
            songs.len(),
            if songs.len() == 1 { "song" } else { "songs" }
        ))
        .font(regular_font(13.0))
        .color(theme.scheme.on_surface_variant),
    );
    ui.add_space(DP8);

    if songs.is_empty() {
        empty_state(
            ui,
            theme,
            "favorite",
            "No liked songs yet",
            "Tap the heart in the player to save a track here.",
        );
        ui.add_space(DP24);
        return intents;
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if filled_button(ui, theme, Some("play_arrow"), "Play").clicked() {
            intents.push(Intent::PlayAll {
                songs: songs.clone(),
                source: QueueSource::List,
            });
        }
        if outlined_button(ui, theme, "Shuffle").clicked() {
            intents.push(Intent::PlayShuffled {
                songs: songs.clone(),
                source: QueueSource::List,
            });
        }
    });
    ui.add_space(DP8);

    intents.extend(song_list(
        ui,
        theme,
        services,
        art,
        songs,
        current,
        width,
        QueueSource::List,
    ));
    ui.add_space(DP24);
    intents
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_store::SettingsPaths;
    use crate::theme::Theme;
    use eframe::egui::{
        CentralPanel, Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2,
    };
    use ml_core::Settings;

    fn song(id: &str) -> Song {
        Song {
            id: id.into(),
            title: format!("Song {id}"),
            ..Song::default()
        }
    }

    fn services() -> (Services, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "ml-liked-render-{}-{}",
            std::process::id(),
            ml_core::now_millis()
        ));
        let paths = SettingsPaths::new(root.join("c"), root.join("d"), root.join("ca"));
        let (services, _notes) = Services::new(&Context::default(), paths, &Settings::default());
        (services, root)
    }

    fn render_play_click(
        services: &Services,
        theme: &Theme,
        art: &mut ArtCache,
        liked: &Load<Vec<Song>>,
        screen_width: f32,
        content_width: f32,
    ) -> Vec<Intent> {
        let ctx = Context::default();
        let click = Pos2::new(8.0 + 49.0, 64.0);
        let mut intents = Vec::new();
        let mut render = |events: Vec<Event>, intents: &mut Vec<Intent>| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(screen_width, 900.0),
                )),
                events,
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        *intents = show(ui, theme, services, art, liked, None, content_width);
                    });
                });
            });
            output.textures_delta.clear();
        };
        render(vec![Event::PointerMoved(click)], &mut intents);
        render(
            vec![
                Event::PointerMoved(click),
                Event::PointerButton {
                    pos: click,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::default(),
                },
            ],
            &mut intents,
        );
        assert!(intents.is_empty(), "press alone must not play");
        render(
            vec![
                Event::PointerMoved(click),
                Event::PointerButton {
                    pos: click,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::default(),
                },
            ],
            &mut intents,
        );
        intents
    }

    #[test]
    fn play_button_is_a_play_all_for_one_and_two_song_lists() {
        let (services, _root) = services();
        let theme = Theme::from_settings(&Settings::default());
        let mut art = ArtCache::new();
        for expected in [1, 2] {
            let liked = Load {
                data: Some((0..expected).map(|i| song(&format!("s{i}"))).collect()),
                ..Load::default()
            };
            for (screen_width, content_width) in [(405.0, 373.0), (1100.0, 560.0)] {
                let intents = render_play_click(
                    &services,
                    &theme,
                    &mut art,
                    &liked,
                    screen_width,
                    content_width,
                );
                let [Intent::PlayAll { songs, source }] = intents.as_slice() else {
                    panic!("Play yielded {intents:?} at {screen_width}/{expected}");
                };
                assert_eq!(songs.len(), expected, "the full liked list");
                assert_eq!(*source, QueueSource::List);
                assert!(
                    !crate::pure::play_starts_radio(songs.len(), *source, true),
                    "Play all must never route to radio"
                );
            }
        }
        assert!(crate::pure::play_starts_radio(1, QueueSource::List, false));
    }

    #[test]
    fn empty_liked_list_has_no_play_all() {
        let (services, _root) = services();
        let theme = Theme::from_settings(&Settings::default());
        let mut art = ArtCache::new();
        let liked = Load {
            data: Some(Vec::new()),
            ..Load::default()
        };
        let intents = render_play_click(&services, &theme, &mut art, &liked, 405.0, 373.0);
        assert!(
            !intents
                .iter()
                .any(|intent| matches!(intent, Intent::PlayAll { .. })),
            "an empty list has no Play all: {intents:?}"
        );
    }
}
