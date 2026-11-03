//! Locally created/editable playlist detail.

use eframe::egui::{self, Ui};
use ml_core::{QueueSource, Song};

use crate::art::ArtCache;
use crate::pure::{playlist_move_target, playlist_songs, PlaylistOrder};
use crate::services::Services;
use crate::state::LocalPlaylistState;
use crate::theme::{bold_font, regular_font, Theme, DP12, DP24, DP4, DP8, TITLE_LARGE};
use crate::widgets::{
    empty_state, filled_button, icon_button, outlined_button, section_header, song_row,
    SongRowOptions,
};

use super::{artist_intent, error_block, skeleton_rows, sort_menu, Intent};

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    state: &mut LocalPlaylistState,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();

    if state.data.initial_loading() {
        skeleton_rows(ui, theme, width);
        ui.add_space(DP24);
        return intents;
    }
    if let Some(error) = &state.data.error {
        if state.data.data.is_none() {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
            ui.add_space(DP24);
            return intents;
        }
    }
    let Some(view) = state.data.data.clone() else {
        empty_state(
            ui,
            theme,
            "playlist_play",
            "Playlist unavailable",
            "This playlist may have been deleted.",
        );
        ui.add_space(DP24);
        return intents;
    };

    ui.add_space(DP4);
    ui.label(
        egui::RichText::new(&view.name)
            .font(bold_font(TITLE_LARGE))
            .color(theme.scheme.on_surface),
    );
    ui.label(
        egui::RichText::new(format!(
            "{} {}",
            view.songs.len(),
            if view.songs.len() == 1 {
                "song"
            } else {
                "songs"
            }
        ))
        .font(regular_font(13.0))
        .color(theme.scheme.on_surface_variant),
    );
    ui.add_space(DP8);

    // Display-only order: Play, Shuffle and row indices all use this copy.
    let songs = playlist_songs(&view.songs, state.sort);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if filled_button(ui, theme, Some("play_arrow"), "Play").clicked() && !songs.is_empty() {
            intents.push(Intent::PlayAll {
                songs: songs.clone(),
                source: QueueSource::List,
            });
        }
        if outlined_button(ui, theme, "Shuffle").clicked() && !songs.is_empty() {
            intents.push(Intent::PlayShuffled {
                songs: songs.clone(),
                source: QueueSource::List,
            });
        }
        if icon_button(ui, theme, "download", "Download all", 22.0).clicked() && !songs.is_empty() {
            intents.push(Intent::DownloadAll(songs.clone()));
        }
        if outlined_button(ui, theme, "Add current").clicked() {
            intents.push(Intent::AddCurrentToPlaylist {
                playlist_id: view.id.clone(),
            });
        }
    });
    ui.add_space(DP8);
    ui.horizontal(|ui| {
        if icon_button(ui, theme, "edit", "Rename playlist", 22.0).clicked() {
            state.rename = Some(view.name.clone());
            state.confirm_delete = false;
        }
        if icon_button(ui, theme, "delete", "Delete playlist", 22.0).clicked() {
            state.confirm_delete = true;
            state.rename = None;
        }
    });

    if let Some(mut name) = state.rename.clone() {
        ui.add_space(DP4);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            let edit = egui::TextEdit::singleline(&mut name)
                .hint_text("Playlist name")
                .desired_width((width - 150.0).max(100.0))
                .font(regular_font(16.0));
            ui.add(edit);
            if filled_button(ui, theme, None, "Save").clicked() && !name.trim().is_empty() {
                intents.push(Intent::RenameLocalPlaylist {
                    id: view.id.clone(),
                    name: name.trim().to_string(),
                });
                state.rename = None;
            } else if ui.button("Cancel").clicked() {
                state.rename = None;
            }
            if state.rename.is_some() {
                state.rename = Some(name.clone());
            }
        });
    }

    if state.confirm_delete {
        ui.add_space(DP4);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.label(
                egui::RichText::new(format!("Delete “{}”?", view.name))
                    .font(regular_font(14.0))
                    .color(theme.scheme.error),
            );
            if filled_button(ui, theme, Some("delete"), "Delete").clicked() {
                intents.push(Intent::DeleteLocalPlaylist {
                    id: view.id.clone(),
                });
                state.confirm_delete = false;
            }
            if outlined_button(ui, theme, "Cancel").clicked() {
                state.confirm_delete = false;
            }
        });
    }

    ui.add_space(DP12);
    if view.songs.is_empty() {
        empty_state(
            ui,
            theme,
            "queue_music",
            "This playlist is empty",
            "Play a track and use “Add current”, or add rows from your library.",
        );
        ui.add_space(DP24);
        return intents;
    }

    sort_menu(
        ui,
        theme,
        &PlaylistOrder::ALL,
        &mut state.sort,
        PlaylistOrder::label,
    );
    ui.add_space(DP4);
    section_header(ui, theme, "Tracks", width, false);
    for (index, song) in songs.iter().enumerate() {
        let url = song.thumb(160);
        let texture = art.texture(ui.ctx(), services, &url);
        let active = current.is_some_and(|current| current.id == song.id);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let row_width = (width - 132.0).max(100.0);
            let response = song_row(
                ui,
                theme,
                song,
                texture.as_ref(),
                row_width,
                active,
                SongRowOptions {
                    download: services.download_badge(&song.id),
                    radio: false,
                },
            );
            if response.download.clicked() {
                intents.push(Intent::Download {
                    song: song.clone(),
                    action: response.download_action,
                });
            } else if response
                .artist
                .as_ref()
                .is_some_and(|artist| artist.clicked())
            {
                intents.extend(artist_intent(song));
            } else if response.row.clicked() {
                intents.push(Intent::Play {
                    songs: songs.clone(),
                    index,
                    source: QueueSource::List,
                });
            }
            // Row positions only match stored indices in Default order; a sorted
            // view hides the reorder controls instead of moving the wrong song.
            if state.sort == PlaylistOrder::Default {
                if icon_button(ui, theme, "keyboard_arrow_up", "Move up", 18.0).clicked() {
                    if let Some(to) = playlist_move_target(index, songs.len(), -1) {
                        intents.push(Intent::MovePlaylistSong {
                            id: view.id.clone(),
                            from: index,
                            to,
                        });
                    }
                }
                if icon_button(ui, theme, "keyboard_arrow_down", "Move down", 18.0).clicked() {
                    if let Some(to) = playlist_move_target(index, songs.len(), 1) {
                        intents.push(Intent::MovePlaylistSong {
                            id: view.id.clone(),
                            from: index,
                            to,
                        });
                    }
                }
            }
            if icon_button(ui, theme, "remove_circle", "Remove from playlist", 18.0).clicked() {
                intents.push(Intent::RemoveFromPlaylist {
                    id: view.id.clone(),
                    song_id: song.id.clone(),
                });
            }
        });
    }

    ui.add_space(DP24);
    intents
}
