//! Album, artist and playlist detail pages (header, play/shuffle, songs).

use eframe::egui::{self, Ui, Vec2};
use ml_core::{format_duration_opt, QueueSource};

use crate::art::ArtCache;
use crate::services::Services;
use crate::state::{Load, PageData};
use crate::theme::{
    bold_font, medium_font, regular_font, Theme, DP12, DP16, DP24, DP4, DP8, TITLE_LARGE, TOUCH,
};
use crate::widgets::{
    elided, filled_button, filled_button_width, icon_button, outlined_button,
    outlined_button_width, section_header,
};

use super::{error_block, load_more, section_block, skeleton_rows, song_list, Intent};

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    page: &Load<PageData>,
    current: Option<&ml_core::Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();

    if page.initial_loading() {
        crate::widgets::shimmer(ui, theme, Vec2::new(160.0, 160.0), 8);
        ui.add_space(DP12);
        skeleton_rows(ui, theme, width);
        ui.add_space(DP24);
        return intents;
    }
    let Some(data) = page.loaded() else {
        if let Some(error) = &page.error {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
        }
        ui.add_space(DP24);
        return intents;
    };

    match data {
        PageData::Album(album_page) => {
            let album = &album_page.album;
            let subtitle = album.artist_names();
            let mut meta = String::from("Album");
            if let Some(year) = album.year {
                meta.push_str(&format!(" · {year}"));
            }
            if let Some(count) = album.song_count {
                meta.push_str(&format!(" · {count} songs"));
            }
            if let Some(duration) = album.duration {
                meta.push_str(&format!(" · {}", format_duration_opt(Some(duration))));
            }
            let actions = header(
                ui,
                theme,
                art,
                services,
                &album.thumbnail_url.clone().unwrap_or_default(),
                &album.title,
                &subtitle,
                &meta,
                width,
            );
            if let Some(description) = &album_page.description {
                if !description.is_empty() {
                    ui.add_space(DP8);
                    description_block(ui, theme, width, "album", description);
                }
            }
            ui.add_space(DP8);
            intents.extend(actions_into(actions, &album_page.songs, QueueSource::Album));
            intents.extend(song_list(
                ui,
                theme,
                services,
                art,
                &album_page.songs,
                current,
                width,
                QueueSource::Album,
            ));
            if album_page.continuation.is_some() {
                if load_more(ui, theme, page.more_loading) {
                    intents.push(Intent::LoadMore);
                }
            }
        }
        PageData::Playlist(playlist_page) => {
            let playlist = &playlist_page.playlist;
            let subtitle = playlist
                .author
                .as_ref()
                .map(|author| author.name.clone())
                .unwrap_or_default();
            let meta = playlist
                .song_count_text
                .clone()
                .unwrap_or_else(|| format!("{} songs", playlist_page.songs.len()));
            let actions = header(
                ui,
                theme,
                art,
                services,
                &playlist.thumbnail_url.clone().unwrap_or_default(),
                &playlist.title,
                &subtitle,
                &meta,
                width,
            );
            if let Some(description) = &playlist.description {
                if !description.is_empty() {
                    ui.add_space(DP8);
                    description_block(ui, theme, width, "playlist", description);
                }
            }
            ui.add_space(DP8);
            intents.extend(actions_into(
                actions,
                &playlist_page.songs,
                QueueSource::Playlist,
            ));
            intents.extend(song_list(
                ui,
                theme,
                services,
                art,
                &playlist_page.songs,
                current,
                width,
                QueueSource::Playlist,
            ));
            if playlist_page.continuation.is_some() {
                if load_more(ui, theme, page.more_loading) {
                    intents.push(Intent::LoadMore);
                }
            }
        }
        PageData::Artist(artist_page) => {
            let artist = &artist_page.artist;
            let meta = artist.subscribers.clone().unwrap_or_default();
            let actions = header(
                ui,
                theme,
                art,
                services,
                &artist.thumbnail_url.clone().unwrap_or_default(),
                &artist.name,
                "Artist",
                &meta,
                width,
            );
            if let Some(description) = &artist_page.description {
                if !description.is_empty() {
                    ui.add_space(DP8);
                    description_block(ui, theme, width, "artist", description);
                }
            }
            ui.add_space(DP8);
            intents.extend(actions_into(
                actions,
                &artist_page.top_songs,
                QueueSource::Artist,
            ));
            if !artist_page.top_songs.is_empty() {
                section_header(ui, theme, "Top songs", width, false);
                intents.extend(song_list(
                    ui,
                    theme,
                    services,
                    art,
                    &artist_page.top_songs,
                    current,
                    width,
                    QueueSource::Artist,
                ));
            }
            for section in &artist_page.sections {
                if section.items.is_empty() {
                    continue;
                }
                section_header(ui, theme, &section.title, width, false);
                intents.extend(section_block(
                    ui,
                    theme,
                    services,
                    art,
                    section,
                    current,
                    width,
                    QueueSource::Artist,
                ));
            }
            if artist_page.continuation.is_some() && load_more(ui, theme, page.more_loading) {
                intents.push(Intent::LoadMore);
            }
        }
    }

    ui.add_space(DP24);
    intents
}

struct HeaderActions {
    play: Option<egui::Response>,
    shuffle: Option<egui::Response>,
    download: Option<egui::Response>,
}

#[allow(clippy::too_many_arguments)]
fn header(
    ui: &mut Ui,
    theme: &Theme,
    art: &mut ArtCache,
    services: &Services,
    thumbnail_url: &str,
    title: &str,
    subtitle: &str,
    meta: &str,
    width: f32,
) -> HeaderActions {
    let url = if thumbnail_url.is_empty() {
        String::new()
    } else {
        ml_core::resize_thumbnail_url(thumbnail_url, 400)
    };
    let texture = art.texture(ui.ctx(), services, &url);
    let art_size = if width >= 560.0 {
        176.0
    } else {
        width.min(200.0)
    };

    let mut play = None;
    let mut shuffle = None;
    let mut download = None;
    if width >= 560.0 {
        ui.horizontal_top(|ui| {
            let (art_rect, _) = ui.allocate_exact_size(Vec2::splat(art_size), egui::Sense::hover());
            crate::widgets::art_or_placeholder(ui, theme, texture.as_ref(), art_rect, 8);
            ui.add_space(DP16);
            ui.vertical(|ui| {
                ui.set_width((width - art_size - 24.0).max(120.0));
                ui.spacing_mut().item_spacing.y = DP4;
                let text_width = ui.available_width();
                let (title_rect, _) =
                    ui.allocate_exact_size(Vec2::new(text_width, 28.0), egui::Sense::hover());
                elided(
                    ui.painter(),
                    title_rect.left_top(),
                    title,
                    bold_font(TITLE_LARGE),
                    theme.scheme.on_surface,
                    text_width,
                );
                ui.label(
                    egui::RichText::new(subtitle)
                        .font(medium_font(14.0))
                        .color(theme.scheme.on_surface_variant),
                );
                ui.label(
                    egui::RichText::new(meta)
                        .font(regular_font(12.0))
                        .color(theme.scheme.on_surface_variant),
                );
                ui.add_space(DP8);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = DP8;
                    play = Some(filled_button(ui, theme, Some("play_arrow"), "Play"));
                    shuffle = Some(outlined_button(ui, theme, "Shuffle"));
                    download = Some(icon_button(ui, theme, "download", "Download all", 22.0));
                });
            });
        });
    } else {
        ui.vertical_centered(|ui| {
            let (art_rect, _) = ui.allocate_exact_size(Vec2::splat(art_size), egui::Sense::hover());
            crate::widgets::art_or_placeholder(ui, theme, texture.as_ref(), art_rect, 8);
            ui.add_space(DP12);
            ui.spacing_mut().item_spacing.y = DP4;
            ui.label(
                egui::RichText::new(title)
                    .font(bold_font(22.0))
                    .color(theme.scheme.on_surface),
            );
            ui.label(
                egui::RichText::new(subtitle)
                    .font(medium_font(14.0))
                    .color(theme.scheme.on_surface_variant),
            );
            ui.label(
                egui::RichText::new(meta)
                    .font(regular_font(12.0))
                    .color(theme.scheme.on_surface_variant),
            );
            ui.add_space(DP8);
            let painter = ui.painter();
            let row_width = filled_button_width(painter, "Play", true)
                + DP8
                + outlined_button_width(painter, "Shuffle")
                + DP8
                + TOUCH;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = DP8;
                ui.add_space(((width - row_width) / 2.0).max(0.0));
                play = Some(filled_button(ui, theme, Some("play_arrow"), "Play"));
                shuffle = Some(outlined_button(ui, theme, "Shuffle"));
                download = Some(icon_button(ui, theme, "download", "Download all", 22.0));
            });
        });
    }
    HeaderActions {
        play,
        shuffle,
        download,
    }
}

/// Bio/description block: three lines with a "Show more" pill.
///
/// Wiki text is unbounded upstream (artist pages push the tracks below the
/// fold); the expand flag lives in egui memory so the screen stays stateless.
fn description_block(ui: &mut Ui, theme: &Theme, width: f32, id: &str, text: &str) {
    const COLLAPSED_LINES: f32 = 3.0;
    let font = regular_font(13.0);
    let color = theme.scheme.on_surface_variant;
    let row_height = ui.ctx().fonts_mut(|fonts| fonts.row_height(&font));
    let galley = ui
        .painter()
        .layout(text.to_owned(), font.clone(), color, width);
    let collapsed = (row_height * COLLAPSED_LINES).min(galley.rect.height());
    if galley.rect.height() <= collapsed + 0.5 {
        ui.add(egui::Label::new(
            egui::RichText::new(text).font(font).color(color),
        ));
        return;
    }
    let state = ui.scope_id().with(("description", id));
    let mut expanded = ui.data(|data| data.get_temp::<bool>(state).unwrap_or(false));
    let height = if expanded {
        galley.rect.height()
    } else {
        collapsed
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    ui.painter()
        .with_clip_rect(rect)
        .galley(rect.min, galley, color);
    let label = if expanded { "Show less" } else { "Show more" };
    if outlined_button(ui, theme, label).clicked() {
        expanded = !expanded;
        ui.data_mut(|data| data.insert_temp(state, expanded));
    }
}

fn actions_into(
    actions: HeaderActions,
    songs: &[ml_core::Song],
    source: QueueSource,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let play = actions.play.is_some_and(|response| response.clicked());
    let shuffle = actions.shuffle.is_some_and(|response| response.clicked());
    let download = actions.download.is_some_and(|response| response.clicked());
    if download && !songs.is_empty() {
        intents.push(Intent::DownloadAll(songs.to_vec()));
    } else if play && !songs.is_empty() {
        intents.push(Intent::Play {
            songs: songs.to_vec(),
            index: 0,
            source,
        });
    } else if shuffle && !songs.is_empty() {
        intents.push(Intent::PlayShuffled {
            songs: songs.to_vec(),
            source,
        });
    }
    intents
}
