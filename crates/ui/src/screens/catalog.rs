//! Catalog/browse screens: charts, moods, new releases and podcasts.

use eframe::egui::{self, Ui, Vec2};
use ml_core::{Artist, HomeFeed, QueueSource, Song, YtItem};
use ml_innertube::pages::browse::BrowsePage;
use ml_innertube::pages::moods::MoodsAndGenres;
use ml_innertube::pages::podcast::PodcastPage;

use crate::art::ArtCache;
use crate::routes::Route;
use crate::services::Services;
use crate::state::{item_route, item_song, item_subtitle, item_thumb, section_songs, Load};
use crate::theme::{
    bold_font, medium_font, regular_font, Theme, DP12, DP16, DP24, DP4, DP8, GRID_GAP, TITLE_LARGE,
    TITLE_MEDIUM,
};
use crate::widgets::{
    album_card, art_or_placeholder, chip, empty_state, filled_button, section_header, song_row,
    SongRowOptions,
};

use super::{error_block, load_more, section_block, skeleton_cards, skeleton_rows, Intent};

fn direct(intents: Vec<Intent>) -> Vec<Intent> {
    intents
        .into_iter()
        .map(|intent| match intent {
            Intent::Play { songs, index, .. } => Intent::PlayCatalog { songs, index },
            other => other,
        })
        .collect()
}

fn section_title(title: &str, count: usize) -> String {
    if title.is_empty() {
        format!("{count} items")
    } else {
        format!("{title} · {count}")
    }
}

fn browse_status(
    ui: &mut Ui,
    theme: &Theme,
    page: &Load<BrowsePage>,
    width: f32,
    intents: &mut Vec<Intent>,
) -> bool {
    if page.initial_loading() {
        skeleton_cards(ui, theme, width);
        ui.add_space(DP8);
        skeleton_rows(ui, theme, width);
        ui.add_space(DP24);
        return false;
    }
    if page.loaded().is_none() {
        if let Some(error) = &page.error {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
        } else {
            empty_state(
                ui,
                theme,
                "explore",
                "Nothing here yet",
                "YouTube Music returned no catalog page.",
            );
        }
        ui.add_space(DP24);
        return false;
    }
    true
}

fn browse_sections(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    page: &Load<BrowsePage>,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if !browse_status(ui, theme, page, width, &mut intents) {
        return intents;
    }
    let Some(browse) = page.loaded() else {
        return intents;
    };
    if browse
        .sections
        .iter()
        .all(|section| section.items.is_empty())
    {
        empty_state(
            ui,
            theme,
            "inbox",
            "No sections returned",
            "Try another category or refresh.",
        );
        ui.add_space(DP24);
        return intents;
    }
    for section in &browse.sections {
        if section.items.is_empty() {
            continue;
        }
        ui.add_space(DP4);
        let title = section_title(&section.title, section.items.len());
        let playable = section.items.iter().all(|item| item_song(item).is_some());
        if let Some(response) = section_header(ui, theme, &title, width, playable) {
            if response.clicked() {
                let songs = section_songs(section);
                if !songs.is_empty() {
                    intents.push(Intent::PlayCatalog { songs, index: 0 });
                }
            }
        }
        intents.extend(direct(section_block(
            ui,
            theme,
            services,
            art,
            section,
            current,
            width,
            QueueSource::List,
        )));
        ui.add_space(DP8);
    }
    if browse.continuation.is_some() && load_more(ui, theme, page.more_loading) {
        intents.push(Intent::LoadMore);
    }
    ui.add_space(DP24);
    intents
}

pub fn charts(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    page: &Load<BrowsePage>,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    browse_sections(ui, theme, services, art, page, current, width)
}

pub fn mood_results(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    page: &Load<BrowsePage>,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    browse_sections(ui, theme, services, art, page, current, width)
}

pub fn new_releases(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    page: &Load<BrowsePage>,
    _current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if !browse_status(ui, theme, page, width, &mut intents) {
        return intents;
    }
    let Some(browse) = page.loaded() else {
        return intents;
    };
    let sections: Vec<_> = browse
        .sections
        .iter()
        .filter(|section| {
            !section.items.is_empty()
                && !(section.title.is_empty() && section.kind == ml_core::SectionKind::Unknown)
        })
        .collect();
    if sections.is_empty() {
        empty_state(
            ui,
            theme,
            "new_releases",
            "No releases returned",
            "YouTube Music has no new releases for this region.",
        );
        ui.add_space(DP24);
        return intents;
    }
    for section in sections {
        ui.add_space(DP4);
        let title = section_title(&section.title, section.items.len());
        section_header(ui, theme, &title, width, false);
        let songs = section_songs(section);
        let columns = if width >= 840.0 {
            5
        } else if width >= 560.0 {
            4
        } else {
            3
        };
        let card_width = ((width - GRID_GAP * (columns - 1) as f32) / columns as f32).max(80.0);
        for chunk in section.items.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GRID_GAP;
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
                    let song = item_song(item);
                    if response.play.clicked() {
                        if let Some(song) = song {
                            let index = songs
                                .iter()
                                .position(|entry| entry.id == song.id)
                                .unwrap_or(0);
                            intents.push(Intent::PlayCatalog {
                                songs: songs.clone(),
                                index,
                            });
                        } else if let Some(route) = item_route(item) {
                            intents.push(Intent::OpenPlay(route));
                        }
                    } else if response.card.clicked() {
                        if let Some(route) = item_route(item) {
                            intents.push(Intent::Open(route));
                        } else if let Some(song) = song {
                            let index = songs
                                .iter()
                                .position(|entry| entry.id == song.id)
                                .unwrap_or(0);
                            intents.push(Intent::PlayCatalog {
                                songs: songs.clone(),
                                index,
                            });
                        }
                    }
                }
            });
            ui.add_space(GRID_GAP);
        }
    }
    if browse.continuation.is_some() && load_more(ui, theme, page.more_loading) {
        intents.push(Intent::LoadMore);
    }
    ui.add_space(DP24);
    intents
}

pub fn moods(ui: &mut Ui, theme: &Theme, page: &Load<MoodsAndGenres>, width: f32) -> Vec<Intent> {
    let mut intents = Vec::new();
    if page.initial_loading() {
        skeleton_cards(ui, theme, width);
        ui.add_space(DP24);
        return intents;
    }
    let Some(data) = page.loaded() else {
        if let Some(error) = &page.error {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
        } else {
            empty_state(
                ui,
                theme,
                "category",
                "No moods or genres",
                "YouTube Music returned no categories for this region.",
            );
        }
        ui.add_space(DP24);
        return intents;
    };
    if data.groups.iter().all(|group| group.items.is_empty()) {
        empty_state(
            ui,
            theme,
            "category",
            "No moods or genres",
            "YouTube Music returned no category buttons.",
        );
        ui.add_space(DP24);
        return intents;
    }
    for group in &data.groups {
        if group.items.is_empty() {
            continue;
        }
        ui.add_space(DP4);
        ui.label(
            egui::RichText::new(&group.title)
                .font(bold_font(TITLE_MEDIUM))
                .color(theme.scheme.on_surface),
        );
        ui.label(
            egui::RichText::new(format!("{} categories", group.items.len()))
                .font(regular_font(12.0))
                .color(theme.scheme.on_surface_variant),
        );
        ui.add_space(DP8);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(DP8, DP8);
            for item in &group.items {
                if chip(ui, theme, &item.title, false).clicked() {
                    intents.push(Intent::Open(Route::MoodResults {
                        params: item.params.clone(),
                        title: Some(item.title.clone()),
                    }));
                }
            }
        });
        ui.add_space(DP16);
    }
    ui.add_space(DP8);
    intents
}

fn episode_song(episode: &ml_core::Episode) -> Song {
    let mut song = episode.as_song();
    if song.artists.is_empty() {
        if let Some(date) = &episode.publish_date_text {
            song.artists = vec![Artist::new(date.clone(), None)];
        }
    }
    song
}

fn episode_rows(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    episodes: &[ml_core::Episode],
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let songs: Vec<Song> = episodes.iter().map(episode_song).collect();
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
                radio: false,
            },
        );
        if response.download.clicked() {
            intents.push(Intent::Download {
                song: song.clone(),
                action: response.download_action,
            });
        } else if response.row.clicked() {
            intents.push(Intent::PlayCatalog {
                songs: songs.clone(),
                index,
            });
        }
    }
    intents
}

pub fn podcasts(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    page: &Load<HomeFeed>,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if page.initial_loading() {
        skeleton_rows(ui, theme, width);
        ui.add_space(DP24);
        return intents;
    }
    let Some(feed) = page.loaded() else {
        if let Some(error) = &page.error {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
        } else {
            empty_state(
                ui,
                theme,
                "podcasts",
                "No podcast episodes",
                "YouTube Music returned no new episodes for this region.",
            );
        }
        ui.add_space(DP24);
        return intents;
    };
    if feed.sections.iter().all(|section| section.items.is_empty()) {
        empty_state(
            ui,
            theme,
            "podcasts",
            "No podcast episodes",
            "YouTube Music returned no new episodes for this region.",
        );
        ui.add_space(DP24);
        return intents;
    }
    ui.add_space(DP4);
    ui.label(
        egui::RichText::new("New episodes")
            .font(bold_font(TITLE_LARGE))
            .color(theme.scheme.on_surface),
    );
    ui.label(
        egui::RichText::new(
            "Subscribed shows need an account; anonymous browsing lists new episodes only.",
        )
        .font(regular_font(12.0))
        .color(theme.scheme.on_surface_variant),
    );
    for section in &feed.sections {
        let episodes: Vec<ml_core::Episode> = section
            .items
            .iter()
            .filter_map(|item| match item {
                YtItem::Episode(episode) => Some(episode.clone()),
                _ => None,
            })
            .collect();
        if episodes.is_empty() {
            continue;
        }
        ui.add_space(DP12);
        section_header(
            ui,
            theme,
            &section_title(&section.title, episodes.len()),
            width,
            false,
        );
        intents.extend(episode_rows(
            ui, theme, services, art, &episodes, current, width,
        ));
    }
    ui.add_space(DP24);
    intents
}

pub fn podcast(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    page: &Load<PodcastPage>,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if page.initial_loading() {
        skeleton_rows(ui, theme, width);
        ui.add_space(DP24);
        return intents;
    }
    let Some(data) = page.loaded() else {
        if let Some(error) = &page.error {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
        } else {
            empty_state(ui, theme, "podcasts", "Podcast unavailable", "");
        }
        ui.add_space(DP24);
        return intents;
    };
    let show = &data.show;
    let url = show
        .thumbnail_url
        .as_deref()
        .map(|url| ml_core::resize_thumbnail_url(url, 400))
        .unwrap_or_default();
    let texture = art.texture(ui.ctx(), services, &url);
    ui.horizontal_top(|ui| {
        let art_size = if width >= 560.0 { 160.0 } else { 120.0 };
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(art_size), egui::Sense::hover());
        art_or_placeholder(ui, theme, texture.as_ref(), rect, 8);
        ui.add_space(DP16);
        ui.vertical(|ui| {
            ui.set_width((width - art_size - DP16).max(120.0));
            ui.label(
                egui::RichText::new(&show.title)
                    .font(bold_font(TITLE_LARGE))
                    .color(theme.scheme.on_surface),
            );
            if let Some(author) = &show.author {
                ui.label(
                    egui::RichText::new(&author.name)
                        .font(medium_font(14.0))
                        .color(theme.scheme.on_surface_variant),
                );
            }
            if let Some(count) = &show.song_count_text {
                ui.label(
                    egui::RichText::new(count)
                        .font(regular_font(12.0))
                        .color(theme.scheme.on_surface_variant),
                );
            }
            ui.add_space(DP8);
            let songs: Vec<Song> = data.episodes.iter().map(episode_song).collect();
            if !songs.is_empty()
                && filled_button(ui, theme, Some("play_arrow"), "Play latest").clicked()
            {
                intents.push(Intent::PlayCatalog { songs, index: 0 });
            }
        });
    });
    if let Some(description) = &show.description {
        if !description.is_empty() {
            ui.add_space(DP12);
            ui.label(
                egui::RichText::new(description)
                    .font(regular_font(13.0))
                    .color(theme.scheme.on_surface_variant),
            );
        }
    }
    ui.add_space(DP12);
    ui.label(
        egui::RichText::new(format!("{} episodes", data.episodes.len()))
            .font(medium_font(13.0))
            .color(theme.scheme.on_surface),
    );
    intents.extend(episode_rows(
        ui,
        theme,
        services,
        art,
        &data.episodes,
        current,
        width,
    ));
    if data.continuation.is_some() && load_more(ui, theme, page.more_loading) {
        intents.push(Intent::LoadMore);
    }
    ui.add_space(DP24);
    intents
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_store::SettingsPaths;
    use crate::theme::Theme;
    use ml_core::{Album, ArtistItem, Episode, HomeSection, Playlist, SectionKind, Song, YtItem};
    use ml_innertube::pages::moods::{MoodGenre, MoodGroup};
    use std::path::PathBuf;

    fn song(id: &str, title: &str, video: bool) -> Song {
        Song {
            id: id.to_string(),
            title: title.to_string(),
            artists: vec![Artist::new("Artist", None)],
            duration: Some(180),
            thumbnail_url: Some("file:///nonexistent-art.png".to_string()),
            music_video_type: video.then(|| "MUSIC_VIDEO_TYPE_OMV".to_string()),
            ..Song::default()
        }
    }

    fn playlist(id: &str, title: &str) -> Playlist {
        Playlist {
            id: id.to_string(),
            title: title.to_string(),
            author: Some(Artist::new("Author", None)),
            song_count_text: Some("20 songs".to_string()),
            ..Playlist::default()
        }
    }

    fn album(id: &str, title: &str) -> Album {
        Album {
            id: id.to_string(),
            playlist_id: format!("OLAK{id}"),
            title: title.to_string(),
            artists: vec![Artist::new("Artist", None)],
            year: Some(2026),
            thumbnail_url: None,
            explicit: false,
            song_count: Some(10),
            duration: Some(1800),
        }
    }

    fn episode(id: &str, title: &str) -> Episode {
        Episode {
            id: id.to_string(),
            title: title.to_string(),
            author: Some(Artist::new("Show", None)),
            duration: Some(1800),
            publish_date_text: Some("Today".to_string()),
            thumbnail_url: Some("file:///nonexistent-art.png".to_string()),
            ..Episode::default()
        }
    }

    fn section(title: &str, kind: SectionKind, items: Vec<YtItem>) -> HomeSection {
        HomeSection {
            title: title.to_string(),
            items,
            continuation: None,
            kind,
        }
    }

    fn browse_fixture() -> Load<BrowsePage> {
        Load {
            data: Some(BrowsePage {
                title: Some("Charts".to_string()),
                sections: vec![
                    section(
                        "Top songs",
                        SectionKind::Songs,
                        vec![YtItem::Song(song("aaaaaaaaaaa", "Top song", false))],
                    ),
                    section(
                        "Top artists",
                        SectionKind::Artists,
                        vec![YtItem::Artist(ArtistItem {
                            id: "UCartist".to_string(),
                            name: "Top artist".to_string(),
                            thumbnail_url: None,
                            channel_id: Some("UCartist".to_string()),
                            subscribers: Some("1M subscribers".to_string()),
                        })],
                    ),
                    section(
                        "Trending",
                        SectionKind::Playlists,
                        vec![YtItem::Playlist(playlist("VLtrending", "Trending"))],
                    ),
                ],
                continuation: Some("more".to_string()),
            }),
            ..Load::default()
        }
    }

    fn releases_fixture() -> Load<BrowsePage> {
        Load {
            data: Some(BrowsePage {
                title: Some("New releases".to_string()),
                sections: vec![
                    section(
                        "",
                        SectionKind::Unknown,
                        vec![YtItem::Playlist(playlist("VLpromo", "Promo"))],
                    ),
                    section(
                        "Albums & singles",
                        SectionKind::Albums,
                        vec![YtItem::Album(album("MPREa", "Album"))],
                    ),
                    section(
                        "Music videos",
                        SectionKind::Videos,
                        vec![YtItem::Song(song("bbbbbbbbbbb", "Video", true))],
                    ),
                ],
                continuation: None,
            }),
            ..Load::default()
        }
    }

    fn mood_results_fixture() -> Load<BrowsePage> {
        Load {
            data: Some(BrowsePage {
                title: Some("Autumn".to_string()),
                sections: vec![section(
                    "Featured playlists",
                    SectionKind::Playlists,
                    vec![YtItem::Playlist(playlist("VLautumn", "Autumn mix"))],
                )],
                continuation: None,
            }),
            ..Load::default()
        }
    }

    fn moods_fixture() -> Load<MoodsAndGenres> {
        Load {
            data: Some(MoodsAndGenres {
                groups: vec![MoodGroup {
                    title: "Moods & moments".to_string(),
                    items: vec![
                        MoodGenre {
                            title: "Autumn".to_string(),
                            browse_id: "FEmusic_moods_and_genres_category".to_string(),
                            params: Some("ggMP".to_string()),
                            stripe_color: Some(4_288_988_671),
                        },
                        MoodGenre {
                            title: "Chill".to_string(),
                            browse_id: "FEmusic_moods_and_genres_category".to_string(),
                            params: Some("ggMC".to_string()),
                            stripe_color: None,
                        },
                    ],
                }],
            }),
            ..Load::default()
        }
    }

    fn podcasts_fixture() -> Load<HomeFeed> {
        Load {
            data: Some(HomeFeed {
                sections: vec![
                    section(
                        "Parenting",
                        SectionKind::Episodes,
                        vec![
                            YtItem::Episode(episode("ep1", "Episode one")),
                            YtItem::Episode(episode("ep2", "Episode two")),
                        ],
                    ),
                    section(
                        "History",
                        SectionKind::Episodes,
                        vec![YtItem::Episode(episode("ep3", "Episode three"))],
                    ),
                ],
                continuation: None,
            }),
            ..Load::default()
        }
    }

    fn podcast_fixture() -> Load<PodcastPage> {
        Load {
            data: Some(PodcastPage {
                show: Playlist {
                    id: "MPSPshow".to_string(),
                    title: "A podcast".to_string(),
                    author: Some(Artist::new("Show author", None)),
                    song_count_text: Some("20 episodes".to_string()),
                    thumbnail_url: None,
                    description: Some("A show about things.".to_string()),
                    is_podcast: true,
                    ..Playlist::default()
                },
                episodes: vec![episode("ep1", "Episode one"), episode("ep2", "Episode two")],
                continuation: Some("more".to_string()),
                channel_id: Some("UCshow".to_string()),
            }),
            ..Load::default()
        }
    }

    fn services() -> (Services, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "metrolist-catalog-render-{}-{}",
            std::process::id(),
            ml_core::now_millis()
        ));
        let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
        let ctx = egui::Context::default();
        let (services, _notes) = Services::new(&ctx, paths, &Default::default());
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
        assert!(actions.is_empty(), "unexpected catalog action at {width}");
    }

    #[test]
    fn catalog_screens_render_at_both_window_classes() {
        let (services, root) = services();
        let theme = Theme::from_settings(&Default::default());
        let charts_page = browse_fixture();
        let releases_page = releases_fixture();
        let mood_results_page = mood_results_fixture();
        let moods_page = moods_fixture();
        let podcasts_page = podcasts_fixture();
        let podcast_page = podcast_fixture();
        for (width, height) in [(405.0, 900.0), (1100.0, 760.0)] {
            render_at(width, height, |ui, art| {
                charts(ui, &theme, &services, art, &charts_page, None, width)
            });
            render_at(width, height, |ui, art| {
                new_releases(ui, &theme, &services, art, &releases_page, None, width)
            });
            render_at(width, height, |ui, art| {
                mood_results(ui, &theme, &services, art, &mood_results_page, None, width)
            });
            render_at(width, height, |ui, _| moods(ui, &theme, &moods_page, width));
            render_at(width, height, |ui, art| {
                podcasts(ui, &theme, &services, art, &podcasts_page, None, width)
            });
            render_at(width, height, |ui, art| {
                podcast(ui, &theme, &services, art, &podcast_page, None, width)
            });
        }
        drop(services);
        let _ = std::fs::remove_dir_all(root);
    }
}
