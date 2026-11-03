//! Routed content screens and the intents they hand back to the app shell.
//!
//! Screens are stateless renderers over `Load`ed data plus the shared
//! [`ArtCache`]; any asynchronous work is expressed as an [`Intent`] and
//! executed by `App`.

pub mod account;
pub mod catalog;
pub mod detail;
pub mod downloads;
pub mod history;
pub mod home;
pub mod library;
pub mod liked;
pub mod local_playlist;
pub mod lyrics;
pub mod placeholder;
pub mod player;
pub mod queue;
pub mod recognize;
pub mod search;
pub mod settings;
pub mod stats;
pub mod together;

use eframe::egui::{self, Align, Align2, Layout, Pos2, Sense, Ui, Vec2};
use ml_core::{QueueSource, Song, YtItem};

use crate::art::ArtCache;
use crate::motion;
use crate::pure::DownloadClick;
use crate::routes::Route;
use crate::services::Services;
use crate::state::{
    item_route, item_song, item_subtitle, item_thumb, section_songs, song_artist_id,
};
use crate::theme::{
    medium_font, regular_font, Theme, DP16, DP32, DP4, DP8, GRID_GAP, LABEL_LARGE, SHAPE_SMALL,
    TOUCH,
};
use crate::widgets::{
    album_card, empty_state, filled_button, outlined_button, paint_icon, shimmer, shimmer_row,
    song_row, text_width, SongRowOptions,
};

/// A UI intent that needs the app shell (services, navigation, playback).
#[derive(Debug, Clone)]
pub enum Intent {
    Play {
        songs: Vec<Song>,
        index: usize,
        source: QueueSource,
    },
    /// An explicit "Play all" from a list header. Always starts the list at its
    /// first track; unlike [`Intent::Play`] it never turns a one-song list into
    /// a radio queue.
    PlayAll {
        songs: Vec<Song>,
        source: QueueSource,
    },
    /// Play a catalog selection without starting a radio queue.
    PlayCatalog {
        songs: Vec<Song>,
        index: usize,
    },
    PlayShuffled {
        songs: Vec<Song>,
        source: QueueSource,
    },
    Open(Route),
    /// Open the page and start playing it as soon as its songs are loaded.
    OpenPlay(Route),
    /// Run a search and switch to the Search tab.
    Search(String),
    /// Submit the current search box text with the selected filter.
    SubmitSearch,
    /// Re-run the current screen's request.
    Refresh,
    LoadMore,
    StartRadio(Song),
    StartLyrics(Song),
    CreatePlaylist(String),
    AddToPlaylist {
        playlist_id: String,
        song: Song,
    },
    PlayLocalPlaylist(String),
    AddCurrentToPlaylist {
        playlist_id: String,
    },
    RenameLocalPlaylist {
        id: String,
        name: String,
    },
    DeleteLocalPlaylist {
        id: String,
    },
    RemoveFromPlaylist {
        id: String,
        song_id: String,
    },
    MovePlaylistSong {
        id: String,
        from: usize,
        to: usize,
    },
    /// Row download icon. The app maps the action from the current badge.
    Download {
        song: Song,
        action: DownloadClick,
    },
    /// Bulk "download album/playlist" action.
    DownloadAll(Vec<Song>),
    CancelDownload(String),
    RetryDownload(String),
    RemoveDownload(String),
    ClearFailedDownloads,
    /// Switch the shared Stats/Wrapped window and reload it.
    StatsPeriod(ml_store::StatsPeriod),
    /// Explicitly start or stop microphone capture.
    StartRecognition,
    CancelRecognition,
    /// Search for and play the recognised track without a radio queue.
    RecognitionPlay {
        query: String,
    },
    ToggleLyricsRomanize,
    AdjustLyricsOffset(i64),
    ResetLyricsOffset,
    TranslateLyrics,
}

/// Open the first artist with an id from a song row, if there is one.
pub fn artist_intent(song: &Song) -> Option<Intent> {
    song_artist_id(song).map(|id| Intent::Open(Route::Artist { id }))
}

/// Compact display-sort menu: one chip with the current order, a popup of
/// options. Writes back a fallback when the choice is not offered on this tab.
pub fn sort_menu<T: Copy + PartialEq>(
    ui: &mut Ui,
    theme: &Theme,
    options: &[T],
    selected: &mut T,
    label: impl Fn(T) -> &'static str,
) {
    let Some(first) = options.first().copied() else {
        return;
    };
    if !options.contains(selected) {
        *selected = first;
    }
    let text = format!("Sort: {}", label(*selected));
    let font = medium_font(LABEL_LARGE);
    let text_width = text_width(ui.painter(), &text, font.clone());
    let width = (text_width + 16.0 + 32.0).min(ui.available_width());
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, DP32), Sense::click());
    let fill = motion::state_layer(ui, theme.scheme.surface_container_high, &response);
    ui.painter().rect_filled(rect, SHAPE_SMALL, fill);
    ui.painter().text(
        Pos2::new(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        &text,
        font,
        theme.scheme.on_surface,
    );
    paint_icon(
        ui.painter(),
        Pos2::new(rect.right() - 14.0, rect.center().y),
        "keyboard_arrow_down",
        18.0,
        theme.scheme.on_surface_variant,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::ComboBox,
            true,
            format!("Sort by {}", label(*selected)),
        )
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    egui::Popup::menu(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.set_min_width(width.max(160.0));
            for &option in options {
                let current = *selected == option;
                if ui.selectable_label(current, label(option)).clicked() && !current {
                    *selected = option;
                }
            }
        });
}

/// Center a text/form column inside the available width, up to `max_width`.
/// Compact layouts use the full width and keep the existing page insets.
pub fn measured_column<R>(ui: &mut Ui, max_width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let available = ui.available_width();
    let width = available.min(max_width);
    if available <= width + 0.5 {
        return add(ui);
    }
    let indent = (available - width) * 0.5;
    ui.horizontal_top(|ui| {
        ui.add_space(indent);
        ui.vertical(|ui| {
            ui.set_width(width);
            add(ui)
        })
        .inner
    })
    .inner
}

/// Loading skeleton for a shelf of cards, using the existing shimmer widgets.
pub fn skeleton_cards(ui: &mut Ui, theme: &Theme, width: f32) {
    let columns = if width >= 840.0 {
        5
    } else if width >= 560.0 {
        4
    } else {
        3
    };
    let card_width = ((width - (columns - 1) as f32 * GRID_GAP) / columns as f32).max(72.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = GRID_GAP;
        for _ in 0..columns {
            ui.allocate_ui_with_layout(
                Vec2::new(card_width, card_width + 56.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(card_width);
                    shimmer(ui, theme, Vec2::splat(card_width), 8);
                    ui.add_space(DP8);
                    shimmer(ui, theme, Vec2::new(card_width * 0.7, 12.0), 4);
                },
            );
        }
    });
}

/// Loading skeleton for song lists.
pub fn skeleton_rows(ui: &mut Ui, theme: &Theme, width: f32) {
    for _ in 0..4 {
        shimmer_row(ui, theme, width);
    }
}

/// Error block with a retry button. Returns true when retry was clicked.
pub fn error_block(ui: &mut Ui, theme: &Theme, message: &str) -> bool {
    empty_state(ui, theme, "cloud_off", "Could not load", message);
    let mut retry = false;
    ui.vertical_centered(|ui| {
        if filled_button(ui, theme, Some("refresh"), "Try again").clicked() {
            retry = true;
        }
    });
    retry
}

/// "Load more" row; returns true when clicked.
pub fn load_more(ui: &mut Ui, theme: &Theme, more_loading: bool) -> bool {
    ui.add_space(DP8);
    if more_loading {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, TOUCH), egui::Sense::hover());
        crate::widgets::text(
            ui.painter(),
            rect.left_top() + Vec2::new(DP8, 10.0),
            "Loading more…",
            regular_font(14.0),
            theme.scheme.on_surface_variant,
        );
        return false;
    }
    let clicked = outlined_button(ui, theme, "Load more").clicked();
    ui.add_space(DP4);
    clicked
}

/// Render one Home/Artist shelf. Returns intents for clicks inside it.
pub fn section_block(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    section: &ml_core::HomeSection,
    current: Option<&Song>,
    width: f32,
    source: QueueSource,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let songs = section_songs(section);
    let all_songs = !songs.is_empty() && songs.len() == section.items.len();

    if all_songs {
        let wide = width >= 700.0;
        let columns = if wide { 2 } else { 1 };
        let gap = DP16;
        let col_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(160.0);
        let rows = songs.len().div_ceil(columns);
        let mut draw = |ui: &mut Ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for column in 0..columns {
                    ui.allocate_ui_with_layout(
                        Vec2::new(col_width, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(col_width);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for row in 0..rows {
                                let index = column * rows + row;
                                let Some(song) = songs.get(index) else {
                                    continue;
                                };
                                let url = song.thumb(160);
                                let texture = art.texture(ui.ctx(), services, &url);
                                let active = current.is_some_and(|c| c.id == song.id);
                                let response = song_row(
                                    ui,
                                    theme,
                                    song,
                                    texture.as_ref(),
                                    col_width,
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
                                } else if response
                                    .radio
                                    .as_ref()
                                    .is_some_and(|radio| radio.clicked())
                                {
                                    intents.push(Intent::StartRadio(song.clone()));
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
                                        source,
                                    });
                                }
                            }
                        },
                    );
                }
            });
        };
        if wide {
            draw(ui);
        } else {
            egui::ScrollArea::horizontal()
                .id_salt(("section_rows", section.title.as_str()))
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, draw);
        }
        return intents;
    }

    let columns = if width >= 840.0 {
        5
    } else if width >= 560.0 {
        4
    } else {
        3
    };
    let gap = GRID_GAP;
    let card_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(80.0);
    for chunk in section.items.chunks(columns) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for item in chunk {
                let url = item_thumb(item, 200);
                let texture = art.texture(ui.ctx(), services, &url);
                let response = album_card(
                    ui,
                    theme,
                    item.title(),
                    &item_subtitle(item),
                    texture.as_ref(),
                    card_width,
                );
                if response.play.clicked() {
                    if let Some(route) = item_route(item) {
                        intents.push(Intent::OpenPlay(route));
                    } else if let Some(song) = item_song(item) {
                        intents.push(Intent::Play {
                            songs: vec![song],
                            index: 0,
                            source,
                        });
                    }
                } else if response.card.clicked() {
                    if let Some(route) = item_route(item) {
                        intents.push(Intent::Open(route));
                    } else if let Some(song) = item_song(item) {
                        if let Some(index) = songs.iter().position(|entry| entry.id == song.id) {
                            intents.push(Intent::Play {
                                songs: songs.clone(),
                                index,
                                source,
                            });
                        }
                    }
                }
            }
        });
        ui.add_space(GRID_GAP);
    }
    intents
}

/// Play a mixed item list (search results): rows for playable items, cards for
/// the rest. Returns intents.
pub fn mixed_items(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    items: &[YtItem],
    current: Option<&Song>,
    width: f32,
    source: QueueSource,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let songs: Vec<Song> = items.iter().filter_map(item_song).collect();
    for item in items {
        if let Some(song) = item_song(item) {
            let url = song.thumb(160);
            let texture = art.texture(ui.ctx(), services, &url);
            let active = current.is_some_and(|c| c.id == song.id);
            let response = song_row(
                ui,
                theme,
                &song,
                texture.as_ref(),
                width,
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
            } else if response
                .artist
                .as_ref()
                .is_some_and(|artist| artist.clicked())
            {
                intents.extend(artist_intent(&song));
            } else if response.row.clicked() {
                let index = songs
                    .iter()
                    .position(|entry| entry.id == song.id)
                    .unwrap_or(0);
                intents.push(Intent::Play {
                    songs: songs.clone(),
                    index,
                    source,
                });
            }
        }
    }
    let cards: Vec<&YtItem> = items
        .iter()
        .filter(|item| item_route(item).is_some())
        .collect();
    if !cards.is_empty() {
        let columns = if width >= 840.0 {
            5
        } else if width >= 560.0 {
            4
        } else {
            3
        };
        let gap = GRID_GAP;
        let card_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(80.0);
        for chunk in cards.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for item in chunk {
                    let url = item_thumb(item, 200);
                    let texture = art.texture(ui.ctx(), services, &url);
                    let response = album_card(
                        ui,
                        theme,
                        item.title(),
                        &item_subtitle(item),
                        texture.as_ref(),
                        card_width,
                    );
                    if response.play.clicked() {
                        if let Some(route) = item_route(item) {
                            intents.push(Intent::OpenPlay(route));
                        }
                    } else if response.card.clicked() {
                        if let Some(route) = item_route(item) {
                            intents.push(Intent::Open(route));
                        }
                    }
                }
            });
            ui.add_space(GRID_GAP);
        }
    }
    intents
}

/// Song list for album/playlist pages with click-to-play.
pub fn song_list(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    songs: &[Song],
    current: Option<&Song>,
    width: f32,
    source: QueueSource,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    for (index, song) in songs.iter().enumerate() {
        let url = song.thumb(160);
        let texture = art.texture(ui.ctx(), services, &url);
        let active = current.is_some_and(|c| c.id == song.id);
        let response = song_row(
            ui,
            theme,
            song,
            texture.as_ref(),
            width,
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
        } else if response
            .artist
            .as_ref()
            .is_some_and(|artist| artist.clicked())
        {
            intents.extend(artist_intent(song));
        } else if response.row.clicked() {
            intents.push(Intent::Play {
                songs: songs.to_vec(),
                index,
                source,
            });
        }
    }
    intents
}
