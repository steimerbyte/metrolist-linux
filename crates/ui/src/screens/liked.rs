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
            intents.push(Intent::Play {
                songs: songs.clone(),
                index: 0,
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
