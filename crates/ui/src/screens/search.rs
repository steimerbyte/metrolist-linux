//! Search: suggestions, history, filters and mixed InnerTube results.

use std::time::Instant;

use eframe::egui::{self, Id, Key, Ui, Vec2};
use ml_core::{QueueSource, Song, YtItem};

use crate::art::ArtCache;
use crate::routes::Route;
use crate::services::Services;
use crate::state::{FilterChoice, SearchState};
use crate::theme::{regular_font, Theme};
use crate::widgets::{chip, empty_state, icon_button, paint_icon, section_header};

use super::{error_block, load_more, mixed_items, skeleton_rows, Intent};

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    state: &mut SearchState,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let field_id = Id::unique("metrolist_search_field");
    if state.focus {
        ui.memory_mut(|memory| memory.request_focus(field_id));
        state.focus = false;
    }

    egui::Frame::new()
        .fill(theme.scheme.surface_container_high)
        .corner_radius(24)
        .inner_margin(egui::Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                paint_icon(
                    ui.painter(),
                    ui.cursor().min + Vec2::new(11.0, 12.0),
                    "search",
                    22.0,
                    theme.scheme.on_surface_variant,
                );
                ui.add_space(24.0);
                let edit = egui::TextEdit::singleline(&mut state.query)
                    .id(field_id)
                    .hint_text("Songs, albums, artists")
                    .frame(egui::Frame::NONE)
                    .font(regular_font(16.0))
                    .desired_width((width - 132.0).max(80.0));
                let response = ui.add(edit);
                if response.changed() {
                    state.last_edit = Some(Instant::now());
                    state.suggestions_open = true;
                }
                // A singleline TextEdit surrenders focus while handling Enter,
                // so `has_focus` is already false here; `lost_focus` is not.
                if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                    state.suggestions_open = false;
                    intents.push(Intent::SubmitSearch);
                }
                if ui.input(|input| input.key_pressed(Key::Escape)) && response.has_focus() {
                    state.suggestions_open = false;
                    response.surrender_focus();
                }
                if icon_button(ui, theme, "mic", "Recognize song", 22.0).clicked() {
                    intents.push(Intent::Open(Route::Recognize));
                }
            });
        });

    ui.add_space(8.0);

    // Suggestion popover, above the filter row.
    if state.suggestions_open && !state.query.trim().is_empty() && !state.suggestions.is_empty() {
        let suggestions = state.suggestions.clone();
        for suggestion in suggestions.iter().take(6) {
            let response = ui.add_sized(
                [width, 36.0],
                egui::Button::new(
                    egui::RichText::new(suggestion)
                        .font(regular_font(15.0))
                        .color(theme.scheme.on_surface),
                )
                .frame(false)
                .fill(egui::Color32::TRANSPARENT),
            );
            if response.clicked() {
                state.query = suggestion.clone();
                state.suggestions_open = false;
                intents.push(Intent::SubmitSearch);
            }
        }
        ui.add_space(8.0);
    }

    // Filter row.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for choice in FilterChoice::ALL {
            let selected = state.filter == choice;
            let response = chip(ui, theme, choice.label(), selected);
            if response.clicked() && !selected {
                state.filter = choice;
                if !state.submitted.is_empty() {
                    intents.push(Intent::SubmitSearch);
                }
            }
        }
    });
    ui.add_space(8.0);

    let needle = state.query.trim();
    // Typing has not run a query yet; only a submission may show "No results".
    if state.submitted.is_empty() {
        if needle.is_empty() && !state.history.is_empty() {
            section_header(ui, theme, "Recent searches", width, false);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for query in state.history.iter().take(10) {
                    let response = chip(ui, theme, query, false);
                    if response.clicked() {
                        state.query = query.clone();
                        intents.push(Intent::SubmitSearch);
                    }
                }
            });
            ui.add_space(16.0);
        }
        empty_state(
            ui,
            theme,
            "search",
            "Search YouTube Music",
            "Find songs, albums, artists and playlists.",
        );
        return intents;
    }

    if state.results.initial_loading() {
        skeleton_rows(ui, theme, width);
        ui.add_space(24.0);
        return intents;
    }

    if let Some(error) = &state.results.error {
        if state.results.data.is_none() {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
            ui.add_space(24.0);
            return intents;
        }
    }

    let Some(page) = state.results.data.as_ref() else {
        empty_state(
            ui,
            theme,
            "search_off",
            "No results",
            "Try a different title or artist.",
        );
        ui.add_space(24.0);
        return intents;
    };

    let songs_only: Vec<YtItem> = page.songs.iter().cloned().map(YtItem::Song).collect();
    let items: &[YtItem] = if page.items.is_empty() {
        &songs_only
    } else {
        &page.items
    };

    if items.is_empty() {
        empty_state(
            ui,
            theme,
            "search_off",
            "No results",
            "Try a different title or artist.",
        );
        ui.add_space(24.0);
        return intents;
    }

    let heading = if page.query.is_empty() {
        "Results".to_string()
    } else {
        format!("Results for “{}”", page.query)
    };
    section_header(ui, theme, &heading, width, false);
    let row_intents = mixed_items(
        ui,
        theme,
        services,
        art,
        items,
        current,
        width,
        QueueSource::List,
    );
    for intent in row_intents {
        // Upstream's search results play the tapped track's endless radio, and
        // the app starts that radio for a single-song List queue.
        if let Intent::Play { songs, index, .. } = intent {
            if let Some(song) = songs.get(index).cloned() {
                intents.push(Intent::Play {
                    songs: vec![song],
                    index: 0,
                    source: QueueSource::List,
                });
            }
        } else {
            intents.push(intent);
        }
    }

    if page.continuation.is_some() && load_more(ui, theme, state.results.more_loading) {
        intents.push(Intent::LoadMore);
    }
    ui.add_space(24.0);
    intents
}
