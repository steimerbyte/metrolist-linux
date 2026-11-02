//! Renderer → `ml_core` item normalisation.
//!
//! Card/shelf/renderer shapes drift constantly, so these functions read JSON
//! pointers and return `None` for anything unplayable. Metadata rules come from
//! the upstream parity tests: artists only from linked runs, durations only from
//! duration-shaped runs, owners from trusted byline sections.

use ml_core::{Album, AlbumRef, Artist, ArtistItem, Episode, Playlist, Song, YtItem};
use serde_json::Value;

use super::{
    array_at, extract_artists, extract_duration, has_explicit, is_podcast_run, library_tokens,
    odd_elements, owner_from_byline, parse_time, runs_from, split_by_separator, text_of,
    thumbnail_url, Run, PAGE_ALBUM, PAGE_ARTIST, PAGE_AUDIOBOOK, PAGE_AUDIO_TRACK,
    PAGE_LIBRARY_ARTIST, PAGE_PLAYLIST, PAGE_PODCAST, PAGE_USER_CHANNEL,
};

/// Caller context for a list row.
#[derive(Debug, Clone, Default)]
pub struct RowOptions<'a> {
    /// `SearchPage.toYTItem` fallback artists (card context), not used elsewhere.
    pub fallback_artists: &'a [Artist],
    /// Library upload rows keep tag-derived performers after a `", "` split.
    pub is_uploaded: bool,
}

fn pointer<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    value.pointer(path)
}

pub fn page_type(renderer: &Value) -> Option<&str> {
    pointer(
        renderer,
        "/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
    )
    .and_then(Value::as_str)
}

fn browse_id(renderer: &Value) -> Option<&str> {
    pointer(renderer, "/navigationEndpoint/browseEndpoint/browseId").and_then(Value::as_str)
}

fn watch(renderer: &Value) -> Option<&Value> {
    pointer(renderer, "/navigationEndpoint/watchEndpoint")
}

fn overlay_watch<'a>(renderer: &'a Value, key: &str) -> Option<&'a Value> {
    pointer(
        renderer,
        &format!(
            "/{key}/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint"
        ),
    )
    .or_else(|| {
        pointer(
            renderer,
            &format!(
                "/{key}/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchPlaylistEndpoint"
            ),
        )
    })
}

fn overlay_playlist_id<'a>(renderer: &'a Value, key: &str) -> Option<&'a str> {
    pointer(
        renderer,
        &format!(
            "/{key}/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchPlaylistEndpoint/playlistId"
        ),
    )
    .or_else(|| {
        pointer(
            renderer,
            &format!(
                "/{key}/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/playlistId"
            ),
        )
    })
    .and_then(Value::as_str)
}

/// `watchEndpointMusicSupportedConfigs.watchEndpointMusicConfig.musicVideoType`
fn music_video_type(renderer: &Value) -> Option<String> {
    for key in ["overlay", "thumbnailOverlay"] {
        if let Some(value) = pointer(
            renderer,
            &format!(
                "/{key}/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType"
            ),
        ) {
            if let Some(text) = value.as_str() {
                return Some(text.to_string());
            }
        }
    }
    pointer(
        renderer,
        "/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
    )
    .and_then(Value::as_str)
    .map(str::to_string)
}

fn column_runs(renderer: &Value, group: &str, index: usize) -> Vec<Run> {
    for key in [
        "musicResponsiveListItemFlexColumnRenderer",
        "musicResponsiveListItemFixedColumnRenderer",
    ] {
        if let Some(text) = pointer(renderer, &format!("/{group}/{index}/{key}/text")) {
            let runs = runs_from(text);
            if !runs.is_empty() {
                return runs;
            }
        }
    }
    Vec::new()
}

pub fn fixed_duration(renderer: &Value) -> Option<u32> {
    column_runs(renderer, "fixedColumns", 0)
        .first()
        .and_then(|run| parse_time(&run.text))
}

fn badge_explicit(renderer: &Value) -> bool {
    has_explicit(pointer(renderer, "/badges")) || has_explicit(pointer(renderer, "/subtitleBadges"))
}

/// `MusicResponsiveListItemRenderer.videoId`
pub fn video_id(renderer: &Value) -> Option<String> {
    if let Some(id) = pointer(renderer, "/playlistItemData/videoId").and_then(Value::as_str) {
        return Some(id.to_string());
    }
    if let Some(id) = watch(renderer)
        .and_then(|w| w.get("videoId"))
        .and_then(Value::as_str)
    {
        return Some(id.to_string());
    }
    if let Some(id) = overlay_watch(renderer, "overlay")
        .and_then(|w| w.get("videoId"))
        .and_then(Value::as_str)
    {
        return Some(id.to_string());
    }
    column_runs(renderer, "flexColumns", 0)
        .first()
        .and_then(|run| run.watch_video_id.clone())
}

fn set_video_id(renderer: &Value) -> Option<String> {
    pointer(renderer, "/playlistItemData/playlistSetVideoId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            overlay_watch(renderer, "overlay")
                .and_then(|w| w.get("playlistSetVideoId"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn row_title(renderer: &Value) -> Option<String> {
    column_runs(renderer, "flexColumns", 0)
        .first()
        .map(|run| run.text.clone())
}

fn row_is_song(renderer: &Value) -> bool {
    let has_nav = renderer
        .get("navigationEndpoint")
        .is_some_and(|v| !v.is_null());
    if !has_nav {
        return true;
    }
    watch(renderer).is_some()
        || pointer(renderer, "/navigationEndpoint/watchPlaylistEndpoint").is_some()
        || overlay_watch(renderer, "overlay")
            .is_some_and(|w| w.get("videoId").and_then(Value::as_str).is_some())
}

fn second_line(renderer: &Value) -> Vec<Run> {
    column_runs(renderer, "flexColumns", 1)
}

fn all_metadata_runs(renderer: &Value) -> Vec<Run> {
    let columns = renderer
        .get("flexColumns")
        .and_then(Value::as_array)
        .map(|columns| columns.len())
        .unwrap_or(0);
    (1..columns)
        .flat_map(|index| column_runs(renderer, "flexColumns", index))
        .collect()
}

fn album_run(renderer: &Value) -> Option<Run> {
    let columns = renderer
        .get("flexColumns")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    columns
        .iter()
        .enumerate()
        .flat_map(|(index, _)| column_runs(renderer, "flexColumns", index))
        .find(|run| {
            matches!(run.page_type.as_deref(), Some(PAGE_ALBUM | PAGE_AUDIOBOOK))
                && run.browse_id.is_some()
        })
}

fn album_ref(run: &Run) -> Option<AlbumRef> {
    Some(AlbumRef {
        id: run.browse_id.clone()?,
        title: run.text.clone(),
    })
}

fn row_is_episode(renderer: &Value) -> bool {
    if page_type(renderer) == Some(PAGE_AUDIO_TRACK) {
        return true;
    }
    if second_line(renderer)
        .first()
        .is_some_and(|run| run.text == "Episode")
    {
        return true;
    }
    let has_podcast_link = second_line(renderer).iter().any(is_podcast_run);
    has_podcast_link && video_id(renderer).is_some()
}

fn row_is_playlist(renderer: &Value) -> bool {
    page_type(renderer) == Some(PAGE_PLAYLIST)
}

fn row_is_album(renderer: &Value) -> bool {
    matches!(page_type(renderer), Some(PAGE_ALBUM | PAGE_AUDIOBOOK))
}

fn row_is_artist(renderer: &Value) -> bool {
    matches!(page_type(renderer), Some(PAGE_ARTIST | PAGE_LIBRARY_ARTIST))
}

fn row_is_user_channel(renderer: &Value) -> bool {
    page_type(renderer) == Some(PAGE_USER_CHANNEL)
}

fn row_is_podcast(renderer: &Value) -> bool {
    page_type(renderer) == Some(PAGE_PODCAST)
}

/// Upload rows keep the tagged performer column; linked runs keep their ids.
fn uploaded_artists(runs: &[Run]) -> Vec<Artist> {
    let Some(first_group) = split_by_separator(runs).first().cloned() else {
        return Vec::new();
    };
    odd_elements(&first_group)
        .into_iter()
        .flat_map(|run| {
            if run.browse_id.is_some() {
                extract_artists(&[run])
            } else {
                let name = run.text.trim();
                (!name.is_empty())
                    .then(|| Artist::new(name, None))
                    .into_iter()
                    .collect()
            }
        })
        .collect()
}

fn episode_from_rows(renderer: &Value) -> Option<Episode> {
    let groups = split_by_separator(&second_line(renderer));
    let podcast_index = groups
        .iter()
        .position(|group| group.iter().any(is_podcast_run));
    let podcast = podcast_index
        .and_then(|index| groups[index].first())
        .and_then(|run| {
            Some(AlbumRef {
                id: run.browse_id.clone()?,
                title: run.text.clone(),
            })
        });
    let publish_date_text = podcast_index
        .filter(|index| *index > 0)
        .and_then(|index| groups[index - 1].first())
        .map(|run| run.text.clone());
    Some(Episode {
        id: video_id(renderer)?,
        title: row_title(renderer)?,
        author: None,
        podcast,
        duration: groups
            .last()
            .and_then(|group| group.first())
            .and_then(|run| parse_time(&run.text)),
        publish_date_text,
        thumbnail_url: thumbnail_url(pointer(renderer, "/thumbnail")),
        explicit: badge_explicit(renderer),
    })
}

/// `SearchPage.toYTItem` / `ArtistItemsPage` / `LibraryPage` row normalisation.
pub fn responsive_item(renderer: &Value, options: &RowOptions) -> Option<YtItem> {
    if row_is_episode(renderer) {
        return episode_from_rows(renderer).map(YtItem::Episode);
    }
    if row_is_song(renderer) {
        let metadata = all_metadata_runs(renderer);
        let mut artists = if options.is_uploaded {
            uploaded_artists(&second_line(renderer))
        } else {
            extract_artists(&metadata)
        };
        if artists.is_empty() {
            artists = options.fallback_artists.to_vec();
        }
        return Some(YtItem::Song(Song {
            id: video_id(renderer)?,
            title: row_title(renderer)?,
            artists,
            duration: extract_duration(&metadata).or_else(|| fixed_duration(renderer)),
            thumbnail_url: thumbnail_url(pointer(renderer, "/thumbnail")),
            album: album_run(renderer).as_ref().and_then(album_ref),
            explicit: badge_explicit(renderer),
            set_video_id: set_video_id(renderer),
            music_video_type: music_video_type(renderer),
            is_episode: row_is_episode(renderer),
            ..Song::default()
        }));
    }
    let id = browse_id(renderer)?.to_string();
    let title = row_title(renderer)?;
    let thumb = thumbnail_url(pointer(renderer, "/thumbnail"));
    if row_is_artist(renderer) || row_is_user_channel(renderer) {
        return Some(YtItem::Artist(ArtistItem {
            id,
            name: title,
            thumbnail_url: thumb,
            ..ArtistItem::default()
        }));
    }
    if row_is_album(renderer) {
        return Some(YtItem::Album(Album {
            id: id.clone(),
            playlist_id: overlay_playlist_id(renderer, "overlay")
                .unwrap_or_default()
                .to_string(),
            title,
            artists: extract_artists(&second_line(renderer)),
            year: split_by_separator(&second_line(renderer))
                .get(2)
                .and_then(|group| group.first())
                .and_then(|run| run.text.trim().parse().ok()),
            thumbnail_url: thumb,
            explicit: badge_explicit(renderer),
            ..Album::default()
        }));
    }
    let tokens = library_tokens(renderer.get("menu"));
    if row_is_playlist(renderer) {
        let groups = split_by_separator(&second_line(renderer));
        let author = extract_artists(&all_metadata_runs(renderer))
            .into_iter()
            .next()
            .or_else(|| {
                let trimmed = &groups[..groups.len().saturating_sub(1)];
                trimmed.last().and_then(|group| owner_from_byline(group))
            });
        return Some(YtItem::Playlist(Playlist {
            id: id.trim_start_matches("VL").to_string(),
            title,
            author,
            song_count_text: second_line(renderer).last().map(|run| run.text.clone()),
            thumbnail_url: thumb,
            explicit: badge_explicit(renderer),
            ..Playlist::default()
        }));
    }
    if row_is_podcast(renderer) {
        return Some(YtItem::Playlist(Playlist {
            id,
            title,
            author: super::extract_podcast_byline(&all_metadata_runs(renderer))
                .into_iter()
                .next(),
            song_count_text: second_line(renderer).last().map(|run| run.text.clone()),
            thumbnail_url: thumb,
            is_podcast: true,
            ..Playlist::default()
        }));
    }
    let _ = tokens;
    None
}

fn group_text(run: &Run) -> String {
    run.text.trim().to_string()
}

fn is_count_like(text: &str) -> bool {
    let lower = text.to_lowercase();
    text.chars().any(|c| c.is_ascii_digit())
        && ["song", "episode", "view", "play", "minute", "hour", "year"]
            .iter()
            .any(|word| lower.contains(word))
}

/// Owner from a two-row/card subtitle: linked artist first, then the first
/// non-count byline section (locale-independent enough for the parity cases).
fn subtitle_owner(runs: &[Run]) -> Option<Artist> {
    if let Some(artist) = extract_artists(runs).into_iter().next() {
        return Some(artist);
    }
    let groups = split_by_separator(runs);
    groups
        .iter()
        .filter_map(|group| group.first())
        .find(|run| !is_count_like(&group_text(run)))
        .filter(|run| !run.text.trim().is_empty())
        .map(|run| Artist::new(run.text.trim(), None))
        .or_else(|| owner_from_byline(runs))
}

fn subtitles(renderer: &Value) -> Vec<Run> {
    runs_at(renderer, "/subtitle")
}

fn runs_at(renderer: &Value, path: &str) -> Vec<Run> {
    renderer.pointer(path).map(runs_from).unwrap_or_default()
}

fn title_runs(renderer: &Value) -> Vec<Run> {
    runs_at(renderer, "/title")
}

fn menu_icon_endpoint<'a>(renderer: &'a Value, icon: &str, key: &str) -> Option<&'a Value> {
    renderer
        .pointer("/menu/menuRenderer/items")?
        .as_array()?
        .iter()
        .find(|item| {
            item.pointer("/menuNavigationItemRenderer/icon/iconType")
                .and_then(Value::as_str)
                == Some(icon)
        })?
        .pointer(&format!(
            "/menuNavigationItemRenderer/navigationEndpoint/{key}"
        ))
}

fn two_row_is_song(renderer: &Value) -> bool {
    watch(renderer).is_some()
        || pointer(renderer, "/navigationEndpoint/watchPlaylistEndpoint").is_some()
}

fn two_row_is_album(renderer: &Value) -> bool {
    matches!(page_type(renderer), Some(PAGE_ALBUM | PAGE_AUDIOBOOK))
}

fn two_row_is_playlist(renderer: &Value) -> bool {
    page_type(renderer) == Some(PAGE_PLAYLIST)
}

fn two_row_is_artist(renderer: &Value) -> bool {
    matches!(
        page_type(renderer),
        Some(PAGE_ARTIST | PAGE_USER_CHANNEL | PAGE_LIBRARY_ARTIST)
    )
}

fn two_row_is_podcast(renderer: &Value) -> bool {
    page_type(renderer) == Some(PAGE_PODCAST)
}

fn two_row_is_episode(renderer: &Value) -> bool {
    page_type(renderer) == Some(PAGE_AUDIO_TRACK)
}

fn two_row_episode(renderer: &Value) -> Option<Episode> {
    let groups = split_by_separator(&subtitles(renderer));
    let podcast = subtitles(renderer)
        .iter()
        .find(|run| is_podcast_run(run))
        .and_then(|run| {
            Some(AlbumRef {
                id: run.browse_id.clone()?,
                title: run.text.clone(),
            })
        });
    Some(Episode {
        id: overlay_watch(renderer, "thumbnailOverlay")
            .and_then(|w| w.get("videoId"))
            .and_then(Value::as_str)?
            .to_string(),
        title: title_runs(renderer).first()?.text.clone(),
        author: super::extract_podcast_byline(&subtitles(renderer))
            .into_iter()
            .next(),
        podcast,
        duration: groups
            .last()
            .and_then(|g| g.first())
            .and_then(|r| parse_time(&r.text)),
        publish_date_text: groups
            .get(1)
            .and_then(|g| g.first())
            .map(|r| r.text.clone()),
        thumbnail_url: thumbnail_url(renderer.get("thumbnailRenderer")),
        explicit: badge_explicit(renderer),
    })
}

/// `musicTwoRowItemRenderer` (carousel card).
pub fn two_row_item(renderer: &Value) -> Option<YtItem> {
    let title = title_runs(renderer).first()?.text.clone();
    let thumb = thumbnail_url(renderer.get("thumbnailRenderer"));
    if two_row_is_song(renderer) {
        let watch = watch(renderer);
        let subtitle = subtitles(renderer);
        return Some(YtItem::Song(Song {
            id: watch?.get("videoId").and_then(Value::as_str)?.to_string(),
            title,
            artists: extract_artists(&subtitle),
            duration: split_by_separator(&subtitle)
                .last()
                .and_then(|group| group.first())
                .filter(|run| run.browse_id.is_none())
                .and_then(|run| parse_time(&run.text)),
            thumbnail_url: thumb,
            album: subtitle
                .iter()
                .find(|run| {
                    run.browse_id
                        .as_deref()
                        .is_some_and(|id| id.starts_with("MPREb_"))
                })
                .and_then(album_ref),
            explicit: badge_explicit(renderer),
            set_video_id: watch
                .and_then(|w| w.get("playlistSetVideoId"))
                .and_then(Value::as_str)
                .map(str::to_string),
            music_video_type: music_video_type(renderer),
            ..Song::default()
        }));
    }
    if two_row_is_album(renderer) {
        let subtitle = subtitles(renderer);
        let artists = {
            let linked = extract_artists(&subtitle);
            if linked.is_empty() {
                menu_icon_endpoint(renderer, "ARTIST", "browseEndpoint")
                    .filter(|browse| {
                        browse
                            .pointer("/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
                            .and_then(Value::as_str)
                            == Some(PAGE_ARTIST)
                            || browse.get("browseId").and_then(Value::as_str).is_some_and(|id| id.starts_with("UC"))
                    })
                    .and_then(|browse| browse.get("browseId").and_then(Value::as_str))
                    .map(|id| vec![Artist::new("", Some(id.to_string()))])
                    .unwrap_or_default()
            } else {
                linked
            }
        };
        return Some(YtItem::Album(Album {
            id: browse_id(renderer)?.to_string(),
            playlist_id: overlay_playlist_id(renderer, "thumbnailOverlay")
                .unwrap_or_default()
                .to_string(),
            title,
            artists,
            year: subtitle.last().and_then(|run| run.text.trim().parse().ok()),
            thumbnail_url: thumb,
            explicit: badge_explicit(renderer),
            ..Album::default()
        }));
    }
    if two_row_is_playlist(renderer) {
        let subtitle = subtitles(renderer);
        return Some(YtItem::Playlist(Playlist {
            id: browse_id(renderer)?.trim_start_matches("VL").to_string(),
            title,
            author: subtitle_owner(&subtitle),
            song_count_text: subtitle
                .iter()
                .rev()
                .find(|run| {
                    run.text.chars().any(|c| c.is_ascii_digit())
                        && !run.text.to_lowercase().contains("view")
                })
                .map(|run| run.text.clone()),
            thumbnail_url: thumb,
            explicit: badge_explicit(renderer),
            ..Playlist::default()
        }));
    }
    if two_row_is_podcast(renderer) {
        let subtitle = subtitles(renderer);
        return Some(YtItem::Playlist(Playlist {
            id: browse_id(renderer)?.to_string(),
            title,
            author: super::extract_podcast_byline(&subtitle).into_iter().next(),
            song_count_text: subtitle.last().map(|run| run.text.clone()),
            thumbnail_url: thumb,
            is_podcast: true,
            ..Playlist::default()
        }));
    }
    if two_row_is_artist(renderer) {
        return Some(YtItem::Artist(ArtistItem {
            id: browse_id(renderer)?.to_string(),
            name: title,
            thumbnail_url: thumb,
            ..ArtistItem::default()
        }));
    }
    if two_row_is_episode(renderer) {
        return two_row_episode(renderer).map(YtItem::Episode);
    }
    None
}

fn card_watch(renderer: &Value) -> Option<&Value> {
    pointer(renderer, "/onTap/watchEndpoint")
}

fn card_browse(renderer: &Value) -> Option<&Value> {
    pointer(renderer, "/onTap/browseEndpoint")
}

fn card_browse_page_type(renderer: &Value) -> Option<&str> {
    pointer(
        renderer,
        "/onTap/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
    )
    .and_then(Value::as_str)
}

fn button_playlist_id<'a>(renderer: &'a Value, icon: &str) -> Option<&'a str> {
    renderer
        .get("buttons")
        .and_then(Value::as_array)?
        .iter()
        .find(|button| {
            button
                .pointer("/buttonRenderer/icon/iconType")
                .and_then(Value::as_str)
                .is_some_and(|value| icon.is_empty() || value == icon)
        })
        .and_then(|button| {
            pointer(
                button,
                "/buttonRenderer/command/watchPlaylistEndpoint/playlistId",
            )
            .or_else(|| {
                pointer(
                    button,
                    "/buttonRenderer/navigationEndpoint/watchPlaylistEndpoint/playlistId",
                )
            })
        })
        .and_then(Value::as_str)
}

fn card_title(renderer: &Value) -> Option<String> {
    let header = runs_at(renderer, "/header/musicCardShelfHeaderBasicRenderer/title");
    let header = text_of(&header);
    (!header.is_empty())
        .then_some(header)
        .or_else(|| title_runs(renderer).first().map(|run| run.text.clone()))
}

/// `musicCardShelfRenderer` (search summary card / related header card).
pub fn card_item(renderer: &Value) -> Option<YtItem> {
    let title = card_title(renderer)?;
    let thumb = thumbnail_url(renderer.get("thumbnail"));
    let subtitle = runs_at(renderer, "/subtitle");
    if let Some(watch) = card_watch(renderer) {
        let groups = split_by_separator(&subtitle);
        return Some(YtItem::Song(Song {
            id: watch.get("videoId").and_then(Value::as_str)?.to_string(),
            title,
            artists: extract_artists(&subtitle),
            album: groups
                .get(2)
                .and_then(|group| group.first())
                .filter(|run| run.browse_id.is_some())
                .and_then(album_ref),
            duration: groups
                .last()
                .and_then(|group| group.first())
                .and_then(|run| parse_time(&run.text)),
            thumbnail_url: thumb,
            explicit: badge_explicit(renderer),
            music_video_type: watch
                .pointer(
                    "/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
                )
                .and_then(Value::as_str)
                .map(str::to_string),
            ..Song::default()
        }));
    }
    let browse = card_browse(renderer)?;
    let id = browse.get("browseId").and_then(Value::as_str)?.to_string();
    match card_browse_page_type(renderer) {
        Some(PAGE_ARTIST) => Some(YtItem::Artist(ArtistItem {
            id,
            name: title,
            thumbnail_url: thumb,
            ..ArtistItem::default()
        })),
        Some(PAGE_ALBUM | PAGE_AUDIOBOOK) => Some(YtItem::Album(Album {
            id: id.clone(),
            playlist_id: button_playlist_id(renderer, "")
                .unwrap_or_default()
                .to_string(),
            title,
            artists: extract_artists(&subtitle),
            thumbnail_url: thumb,
            explicit: badge_explicit(renderer),
            ..Album::default()
        })),
        Some(PAGE_PLAYLIST) => Some(YtItem::Playlist(Playlist {
            id: id.trim_start_matches("VL").to_string(),
            title,
            author: subtitle_owner(&subtitle),
            thumbnail_url: thumb,
            ..Playlist::default()
        })),
        Some(PAGE_PODCAST) => Some(YtItem::Playlist(Playlist {
            id,
            title,
            author: super::extract_podcast_byline(&subtitle).into_iter().next(),
            thumbnail_url: thumb,
            is_podcast: true,
            ..Playlist::default()
        })),
        _ => None,
    }
}

/// `PodcastPage.extractPodcastArtists`: trusted links, then the menu's podcast
/// link validating a single unlinked byline. Menu labels are never names.
/// `section_title` is the shelf title runs, which can carry the show link.
pub fn extract_podcast_artists(renderer: &Value, section_title: &[Run]) -> Vec<Artist> {
    let runs: Vec<Run> = runs_at(renderer, "/secondSubtitle")
        .into_iter()
        .chain(runs_at(renderer, "/secondarySubtitle"))
        .chain(section_title.iter().cloned())
        .collect();
    let linked = extract_artists(&runs);
    if !linked.is_empty() {
        return linked;
    }
    let podcasts: Vec<Artist> = runs
        .iter()
        .filter(|run| !run.text.trim().is_empty() && is_podcast_run(run))
        .map(|run| Artist::new(run.text.trim(), None))
        .collect();
    if !podcasts.is_empty() {
        return podcasts;
    }
    let menu_has_podcast = renderer
        .pointer("/menu/menuRenderer/items")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.pointer(
                    "/menuNavigationItemRenderer/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
                )
                .and_then(Value::as_str)
                    == Some(PAGE_PODCAST)
            })
        });
    if !menu_has_podcast {
        return Vec::new();
    }
    for path in ["/secondSubtitle", "/secondarySubtitle"] {
        let single = runs_at(renderer, path);
        if single.len() == 1 {
            let run = &single[0];
            if run.browse_id.is_none() && !run.text.trim().is_empty() {
                return vec![Artist::new(run.text.trim(), None)];
            }
        }
    }
    Vec::new()
}

/// `musicMultiRowListItemRenderer` (podcast episode tile/row).
pub fn multi_row_episode(renderer: &Value) -> Option<Episode> {
    let subtitle = renderer
        .pointer("/subtitle")
        .map(runs_from)
        .unwrap_or_default();
    let groups = split_by_separator(&subtitle);
    let author = extract_podcast_artists(renderer, &[]).into_iter().next();
    Some(Episode {
        id: renderer
            .pointer("/onTap/watchEndpoint/videoId")
            .and_then(Value::as_str)?
            .to_string(),
        title: runs_at(renderer, "/title").first()?.text.clone(),
        author,
        podcast: None,
        duration: groups
            .last()
            .and_then(|g| g.first())
            .and_then(|r| parse_time(&r.text)),
        publish_date_text: groups
            .first()
            .and_then(|g| g.first())
            .map(|r| r.text.clone()),
        thumbnail_url: thumbnail_url(renderer.get("thumbnail")),
        explicit: false,
    })
}

/// All items of a `musicShelfRenderer`.
pub fn shelf_items(shelf: &Value) -> Vec<YtItem> {
    let mut items = Vec::new();
    for content in array_at(shelf, "/contents") {
        if let Some(renderer) = content.get("musicResponsiveListItemRenderer") {
            if let Some(item) = responsive_item(renderer, &RowOptions::default()) {
                items.push(item);
            }
        } else if let Some(renderer) = content.get("musicMultiRowListItemRenderer") {
            if let Some(episode) = multi_row_episode(renderer) {
                items.push(YtItem::Episode(episode));
            }
        }
    }
    items
}

/// All items of a `musicCarouselShelfRenderer`.
pub fn carousel_items(carousel: &Value) -> Vec<YtItem> {
    let mut items = Vec::new();
    for content in array_at(carousel, "/contents") {
        if let Some(renderer) = content.get("musicTwoRowItemRenderer") {
            if let Some(item) = two_row_item(renderer) {
                items.push(item);
            }
        } else if let Some(renderer) = content.get("musicResponsiveListItemRenderer") {
            if let Some(item) = responsive_item(renderer, &RowOptions::default()) {
                items.push(item);
            }
        } else if let Some(renderer) = content.get("musicMultiRowListItemRenderer") {
            if let Some(episode) = multi_row_episode(renderer) {
                items.push(YtItem::Episode(episode));
            }
        }
    }
    items
}

/// All items of a `gridRenderer`.
pub fn grid_items(grid: &Value) -> Vec<YtItem> {
    array_at(grid, "/items")
        .iter()
        .filter_map(|item| item.get("musicTwoRowItemRenderer"))
        .filter_map(two_row_item)
        .collect()
}

/// Album shelf row, including the uploaded/ATV performer rules (`AlbumPageTest`).
pub fn album_song(renderer: &Value, album: Option<&Album>) -> Option<Song> {
    let title = row_title(renderer)?;
    let subtitle = second_line(renderer);
    let mut artists = extract_artists(&subtitle);
    if artists.is_empty() {
        let upload_context = album.is_some_and(|album| {
            album.playlist_id.starts_with("OLAK")
                || album
                    .id
                    .starts_with("FEmusic_library_privately_owned_release_detail")
        });
        if upload_context {
            artists = split_by_separator(&subtitle)
                .first()
                .map(|group| {
                    odd_elements(group)
                        .into_iter()
                        .filter(|run| run.browse_id.is_none())
                        .filter_map(|run| {
                            let name = run.text.trim();
                            (!name.is_empty()).then(|| Artist::new(name, None))
                        })
                        .collect()
                })
                .unwrap_or_default();
        }
    }
    if artists.is_empty() {
        artists = album.map(|album| album.artists.clone()).unwrap_or_default();
    }
    let tokens = library_tokens(renderer.get("menu"));
    Some(Song {
        id: video_id(renderer)?,
        title,
        artists,
        album: album
            .map(Album::as_ref)
            .or_else(|| album_run(renderer).as_ref().and_then(album_ref)),
        duration: fixed_duration(renderer),
        thumbnail_url: thumbnail_url(renderer.get("thumbnail"))
            .or_else(|| album.and_then(|album| album.thumbnail_url.clone())),
        explicit: badge_explicit(renderer),
        set_video_id: set_video_id(renderer),
        music_video_type: music_video_type(renderer),
        library_add_token: tokens.add,
        library_remove_token: tokens.remove,
        ..Song::default()
    })
}

/// Playlist shelf row (`musicPlaylistShelfRenderer` / `musicShelfRenderer`).
pub fn playlist_song(renderer: &Value) -> Option<Song> {
    let tokens = library_tokens(renderer.get("menu"));
    let album = column_runs(renderer, "flexColumns", 2)
        .first()
        .filter(|run| !run.text.trim().is_empty())
        .map(|run| AlbumRef {
            id: run.browse_id.clone().unwrap_or_default(),
            title: run.text.clone(),
        });
    Some(Song {
        id: video_id(renderer)?,
        title: row_title(renderer)?,
        artists: extract_artists(&second_line(renderer)),
        album,
        duration: fixed_duration(renderer),
        thumbnail_url: thumbnail_url(renderer.get("thumbnail")),
        explicit: badge_explicit(renderer),
        set_video_id: set_video_id(renderer),
        music_video_type: music_video_type(renderer),
        is_episode: row_is_episode(renderer),
        library_add_token: tokens.add,
        library_remove_token: tokens.remove,
        ..Song::default()
    })
}

/// `next` queue row (`playlistPanelVideoRenderer`).
pub fn panel_song(renderer: &Value) -> Option<(Song, bool)> {
    let byline = runs_at(renderer, "/longBylineText");
    let groups = split_by_separator(&byline);
    let tokens = library_tokens(renderer.get("menu"));
    let song = Song {
        id: pointer(renderer, "/videoId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                pointer(
                    renderer,
                    "/navigationEndpoint/watchEndpoint/videoId",
                )
                .and_then(Value::as_str)
                .map(str::to_string)
            })?,
        title: runs_at(renderer, "/title").first()?.text.clone(),
        artists: extract_artists(&byline),
        album: groups
            .get(1)
            .and_then(|group| group.first())
            .filter(|run| run.browse_id.is_some())
            .and_then(album_ref),
        duration: runs_at(renderer, "/lengthText")
            .first()
            .and_then(|run| parse_time(&run.text)),
        thumbnail_url: thumbnail_url(renderer.get("thumbnail")),
        explicit: has_explicit(pointer(renderer, "/badges")),
        set_video_id: renderer
            .get("playlistSetVideoId")
            .and_then(Value::as_str)
            .map(str::to_string),
        music_video_type: pointer(
            renderer,
            "/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
        )
        .and_then(Value::as_str)
        .map(str::to_string),
        library_add_token: tokens.add,
        library_remove_token: tokens.remove,
        ..Song::default()
    };
    let selected = renderer
        .get("selected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some((song, selected))
}

/// Podcast page row: byline from the first subtitle section, date second.
pub fn podcast_episode_row(renderer: &Value, podcast: Option<&Playlist>) -> Option<Episode> {
    let groups = split_by_separator(&second_line(renderer));
    Some(Episode {
        id: video_id(renderer)?,
        title: row_title(renderer)?,
        author: podcast
            .and_then(|podcast| podcast.author.clone())
            .or_else(|| {
                super::extract_podcast_byline(groups.first().map(Vec::as_slice).unwrap_or(&[]))
                    .into_iter()
                    .next()
            }),
        podcast: podcast.map(|podcast| AlbumRef {
            id: podcast.id.clone(),
            title: podcast.title.clone(),
        }),
        duration: groups
            .last()
            .and_then(|g| g.first())
            .and_then(|r| parse_time(&r.text)),
        publish_date_text: groups
            .get(1)
            .and_then(|g| g.first())
            .map(|r| r.text.clone()),
        thumbnail_url: thumbnail_url(renderer.get("thumbnail")),
        explicit: badge_explicit(renderer),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploaded_artists_keep_tagged_names() {
        let runs = vec![
            Run::linked("21 Savage", "UC21"),
            Run::new(", "),
            Run::new("Song & Dance"),
        ];
        assert_eq!(
            uploaded_artists(&runs),
            vec![
                Artist::new("21 Savage", Some("UC21".into())),
                Artist::new("Song & Dance", None)
            ]
        );
    }
}
