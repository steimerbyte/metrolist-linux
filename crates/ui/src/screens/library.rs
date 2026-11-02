//! Library destination: local songs/albums/artists/playlists/history.

use eframe::egui::{self, Ui, Vec2};
use ml_core::{QueueSource, Song};

use crate::art::ArtCache;
use crate::routes::Route;
use crate::services::Services;
use crate::state::{DownloadsState, LibTab, LibraryState};
use crate::theme::{
    bold_font, medium_font, regular_font, state_layer, Theme, DP12, DP16, DP24, DP4, DP48, DP8,
    GRID_GAP,
};
use crate::widgets::{
    album_card, chip, empty_state, filled_button, icon_button, paint_icon, section_header,
    song_row, SongRowOptions,
};

use super::{downloads, error_block, history, skeleton_cards, skeleton_rows, Intent};

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    state: &mut LibraryState,
    downloads_state: &mut DownloadsState,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    ui.add_space(DP4);

    // Liked shortcut with the live count.
    let liked_count = state.data.data.as_ref().map(|data| data.liked.len());
    if liked_tile(ui, theme, liked_count, width) {
        intents.push(Intent::Open(Route::Liked));
    }
    ui.add_space(DP12);
    if stats_entry(ui, theme, width) {
        intents.push(Intent::Open(Route::Stats));
    }
    ui.add_space(DP12);

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for tab in LibTab::ALL {
            let selected = state.tab == tab;
            let response = chip(ui, theme, tab.label(), selected);
            if response.clicked() && !selected {
                state.tab = tab;
            }
        }
    });
    ui.add_space(DP8);

    if state.tab == LibTab::Downloads {
        intents.extend(downloads::show(ui, theme, services, art, downloads_state));
        ui.add_space(DP24);
        return intents;
    }

    if state.data.initial_loading() {
        match state.tab {
            LibTab::Albums | LibTab::Artists | LibTab::Playlists => {
                skeleton_cards(ui, theme, width);
            }
            _ => skeleton_rows(ui, theme, width),
        }
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
    let Some(data) = state.data.data.as_ref() else {
        empty_state(
            ui,
            theme,
            "library_music",
            "Library unavailable",
            "The local library database could not be opened.",
        );
        ui.add_space(DP24);
        return intents;
    };

    match state.tab {
        LibTab::Songs => {
            if data.songs.is_empty() {
                empty_state(
                    ui,
                    theme,
                    "library_add",
                    "No songs in your library yet",
                    "Use the library button in the player to keep a track here.",
                );
            } else {
                section_header(
                    ui,
                    theme,
                    &format!(
                        "{} {}",
                        data.songs.len(),
                        if data.songs.len() == 1 {
                            "song"
                        } else {
                            "songs"
                        }
                    ),
                    width,
                    true,
                );
                let songs = data.songs.clone();
                intents.extend(library_song_rows(
                    ui,
                    theme,
                    services,
                    art,
                    &songs,
                    current,
                    width,
                    &mut state.add_song,
                ));
            }
        }
        LibTab::Albums => {
            if data.albums.is_empty() {
                empty_state(
                    ui,
                    theme,
                    "album",
                    "No albums yet",
                    "Albums appear here when one of their songs is in your library.",
                );
            } else {
                section_header(ui, theme, "Albums", width, false);
                intents.extend(album_cards(
                    ui,
                    theme,
                    services,
                    art,
                    data.albums.iter().map(|album| {
                        (
                            album.title.clone(),
                            album.artist_names(),
                            Route::Album {
                                id: album.id.clone(),
                            },
                            album
                                .thumbnail_url
                                .as_ref()
                                .map(|url| ml_core::resize_thumbnail_url(url, 200))
                                .unwrap_or_default(),
                        )
                    }),
                    width,
                ));
            }
        }
        LibTab::Artists => {
            if data.artists.is_empty() {
                empty_state(
                    ui,
                    theme,
                    "person",
                    "No artists yet",
                    "Artists appear here when one of their songs is in your library.",
                );
            } else {
                section_header(ui, theme, "Artists", width, false);
                intents.extend(album_cards(
                    ui,
                    theme,
                    services,
                    art,
                    data.artists.iter().map(|artist| {
                        (
                            artist.name.clone(),
                            artist.subscribers.clone().unwrap_or_default(),
                            Route::Artist {
                                id: artist
                                    .channel_id
                                    .clone()
                                    .unwrap_or_else(|| artist.id.clone()),
                            },
                            artist
                                .thumbnail_url
                                .as_ref()
                                .map(|url| ml_core::resize_thumbnail_url(url, 200))
                                .unwrap_or_default(),
                        )
                    }),
                    width,
                ));
            }
        }
        LibTab::Playlists => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let edit = egui::TextEdit::singleline(&mut state.new_playlist)
                    .hint_text("New playlist name")
                    .desired_width((width - 140.0).max(120.0))
                    .font(regular_font(16.0));
                ui.add(edit);
                if filled_button(ui, theme, Some("add"), "Create").clicked()
                    && !state.new_playlist.trim().is_empty()
                {
                    intents.push(Intent::CreatePlaylist(state.new_playlist.clone()));
                    state.new_playlist.clear();
                }
            });
            ui.add_space(GRID_GAP);
            if data.playlists.is_empty() {
                empty_state(
                    ui,
                    theme,
                    "playlist_add",
                    "No playlists yet",
                    "Create one above, then add songs from the Songs tab or the player.",
                );
            } else {
                let playlists = data.playlists.clone();
                let columns = card_columns(width);
                let gap = GRID_GAP;
                let card_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(80.0);
                for chunk in playlists.chunks(columns) {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for playlist in chunk {
                            let url = playlist
                                .thumbnail_url
                                .as_ref()
                                .map(|url| ml_core::resize_thumbnail_url(url, 200))
                                .unwrap_or_default();
                            let texture = art.texture(ui.ctx(), services, &url);
                            let subtitle = format!(
                                "{} {}",
                                playlist.song_count,
                                if playlist.song_count == 1 {
                                    "song"
                                } else {
                                    "songs"
                                }
                            );
                            let response = album_card(
                                ui,
                                theme,
                                &playlist.name,
                                &subtitle,
                                texture.as_ref(),
                                card_width,
                            );
                            let route = Route::LocalPlaylist {
                                id: playlist.id.clone(),
                            };
                            if response.play.clicked() {
                                intents.push(Intent::PlayLocalPlaylist(playlist.id.clone()));
                            } else if response.card.clicked() {
                                intents.push(Intent::Open(route));
                            }
                        }
                    });
                    ui.add_space(GRID_GAP);
                }
            }
        }
        LibTab::History => {
            intents.extend(history::entries_block(
                ui,
                theme,
                services,
                art,
                &data.history,
                current,
                width,
            ));
        }
        LibTab::Downloads => {}
    }

    // Add-to-playlist picker.
    if let Some(song) = state.add_song.clone() {
        let playlists = data.playlists.clone();
        let theme = theme.clone();
        let response = crate::widgets::modal(ui.ctx(), &theme, "add_to_playlist", |ui| {
            ui.set_width(320.0);
            ui.label(
                egui::RichText::new(format!("Add “{}” to", song.title))
                    .font(medium_font(16.0))
                    .color(theme.scheme.on_surface),
            );
            ui.add_space(DP8);
            if playlists.is_empty() {
                ui.label(
                    egui::RichText::new("Create a playlist in the Playlists tab first.")
                        .font(regular_font(13.0))
                        .color(theme.scheme.on_surface_variant),
                );
            }
            for playlist in &playlists {
                let response = ui.add_sized(
                    [320.0, 40.0],
                    egui::Button::new(
                        egui::RichText::new(&playlist.name)
                            .font(regular_font(15.0))
                            .color(theme.scheme.on_surface),
                    )
                    .frame(false),
                );
                if response.clicked() {
                    return Some(playlist.id.clone());
                }
            }
            None
        });
        if let Some(playlist_id) = response.inner {
            intents.push(Intent::AddToPlaylist {
                playlist_id,
                song: song.clone(),
            });
            state.add_song = None;
        } else if response.backdrop_response.clicked() {
            state.add_song = None;
        }
    }

    ui.add_space(DP24);
    intents
}

fn liked_tile(ui: &mut Ui, theme: &Theme, count: Option<usize>, width: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 64.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            10,
            crate::theme::state_layer(theme.bg(), true, response.is_pointer_button_down_on()),
        );
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::Pos2::new(rect.left() + 28.0, rect.center().y),
        Vec2::splat(40.0),
    );
    ui.painter()
        .circle_filled(icon_rect.center(), 20.0, theme.scheme.primary_container);
    paint_icon(
        ui.painter(),
        icon_rect.center(),
        "favorite",
        22.0,
        theme.scheme.on_primary_container,
    );
    crate::widgets::text(
        ui.painter(),
        egui::Pos2::new(rect.left() + 60.0, rect.top() + 12.0),
        "Liked songs",
        medium_font(16.0),
        theme.scheme.on_surface,
    );
    let subtitle = match count {
        Some(count) => format!("{} {}", count, if count == 1 { "song" } else { "songs" }),
        None => "Loading…".to_string(),
    };
    crate::widgets::text(
        ui.painter(),
        egui::Pos2::new(rect.left() + 60.0, rect.top() + 34.0),
        &subtitle,
        regular_font(12.0),
        theme.scheme.on_surface_variant,
    );
    response.clicked()
}

fn library_song_rows(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    songs: &[Song],
    current: Option<&Song>,
    width: f32,
    add_song: &mut Option<Song>,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    for (index, song) in songs.iter().enumerate() {
        let url = song.thumb(160);
        let texture = art.texture(ui.ctx(), services, &url);
        let active = current.is_some_and(|current| current.id == song.id);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let response = song_row(
                ui,
                theme,
                song,
                texture.as_ref(),
                width - 44.0,
                active,
                SongRowOptions {
                    download: services.download_badge(&song.id),
                    radio: true,
                },
            );
            if response.download.clicked() {
                intents.push(Intent::Download {
                    song: song.clone(),
                    action: response.download_action,
                });
            } else if response.radio.as_ref().is_some_and(|radio| radio.clicked()) {
                intents.push(Intent::StartRadio(song.clone()));
            } else if response.row.clicked() {
                intents.push(Intent::Play {
                    songs: songs.to_vec(),
                    index,
                    source: QueueSource::List,
                });
            }
            if icon_button(ui, theme, "playlist_add", "Add to playlist", 20.0).clicked() {
                *add_song = Some(song.clone());
            }
        });
    }
    intents
}

type CardRow = (String, String, Route, String);

fn album_cards(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    rows: impl Iterator<Item = CardRow>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let columns = card_columns(width);
    let gap = GRID_GAP;
    let card_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(80.0);
    let rows: Vec<CardRow> = rows.collect();
    for chunk in rows.chunks(columns) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (title, subtitle, route, url) in chunk {
                let texture = art.texture(ui.ctx(), services, url);
                let response = album_card(ui, theme, title, subtitle, texture.as_ref(), card_width);
                if response.play.clicked() {
                    intents.push(Intent::OpenPlay(route.clone()));
                } else if response.card.clicked() {
                    intents.push(Intent::Open(route.clone()));
                }
            }
        });
        ui.add_space(GRID_GAP);
    }
    intents
}

fn card_columns(width: f32) -> usize {
    if width >= 840.0 {
        5
    } else if width >= 560.0 {
        4
    } else {
        3
    }
}

/// Library header entry to the listening stats and Wrapped story.
fn stats_entry(ui: &mut Ui, theme: &Theme, width: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 68.0), egui::Sense::click());
    let fill = if response.hovered() {
        state_layer(
            theme.scheme.surface_container_high,
            true,
            response.is_pointer_button_down_on(),
        )
    } else {
        theme.scheme.surface_container
    };
    ui.painter().rect_filled(rect, 12, fill);
    let icon_center = rect.left_center() + Vec2::new(DP48 / 2.0, 0.0);
    ui.painter()
        .circle_filled(icon_center, 18.0, theme.scheme.secondary_container);
    paint_icon(
        ui.painter(),
        icon_center,
        "bar_chart",
        20.0,
        theme.scheme.on_secondary_container,
    );
    let text_x = rect.left() + DP48 + DP8;
    let text_width = (rect.right() - DP24 - DP8 - text_x).max(40.0);
    crate::widgets::elided(
        ui.painter(),
        egui::pos2(text_x, rect.top() + DP16),
        "Your stats",
        bold_font(15.0),
        theme.scheme.on_surface,
        text_width,
    );
    crate::widgets::elided(
        ui.painter(),
        egui::pos2(text_x, rect.top() + DP16 + 21.0),
        "Listening totals, top songs & Wrapped",
        regular_font(11.0),
        theme.scheme.on_surface_variant,
        text_width,
    );
    paint_icon(
        ui.painter(),
        egui::pos2(rect.right() - DP24, rect.center().y),
        "chevron_right",
        20.0,
        theme.scheme.on_surface_variant,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Your stats"));
    response.clicked()
}
