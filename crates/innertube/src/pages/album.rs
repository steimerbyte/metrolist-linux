//! Album (`MPREb_*`, uploaded release detail, `VL<playlistId>`) parsing.

use ml_core::{Album, AlbumPage, Song};
use serde_json::Value;

use crate::parse::renderers::album_song;
use crate::parse::{
    array_at, continuation, extract_artists, runs_from, shelf_continuation, text_of,
};

fn section_list_contents(value: &Value) -> &[Value] {
    value
        .pointer("/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn embedded_section_list_contents(value: &Value) -> &[Value] {
    value
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn responsive_header(value: &Value) -> Option<&Value> {
    section_list_contents(value)
        .iter()
        .find_map(|content| content.get("musicResponsiveHeaderRenderer"))
        .or_else(|| {
            embedded_section_list_contents(value)
                .iter()
                .find_map(|content| content.get("musicResponsiveHeaderRenderer"))
        })
}

fn uploaded_header(value: &Value) -> Option<&Value> {
    value.pointer("/header/musicDetailHeaderRenderer")
}

/// First `watchPlaylistEndpoint.playlistId` found by depth-first search inside
/// `scope`; used for header play buttons whose nesting varies.
fn find_playlist_id(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(endpoint) = map.get("watchPlaylistEndpoint") {
                if let Some(id) = endpoint.get("playlistId").and_then(Value::as_str) {
                    if !id.is_empty() {
                        return Some(id.to_string());
                    }
                }
            }
            map.values().find_map(find_playlist_id)
        }
        Value::Array(values) => values.iter().find_map(find_playlist_id),
        _ => None,
    }
}

fn thumbnail(renderer: &Value) -> Option<String> {
    crate::parse::thumbnail_url(Some(renderer))
}

fn description(value: &Value, header: Option<&Value>) -> Option<String> {
    let from_header = header
        .and_then(|header| header.pointer("/description/musicDescriptionShelfRenderer/description"))
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .filter(|text| !text.is_empty());
    from_header.or_else(|| {
        section_list_contents(value)
            .iter()
            .find_map(|content| content.get("musicDescriptionShelfRenderer"))
            .or_else(|| {
                embedded_section_list_contents(value)
                    .iter()
                    .find_map(|content| content.get("musicDescriptionShelfRenderer"))
            })
            .and_then(|shelf| shelf.get("description"))
            .map(runs_from)
            .map(|runs| text_of(&runs))
            .filter(|text| !text.is_empty())
    })
}

fn album_from_responsive_header(value: &Value, browse_id: &str, header: &Value) -> Album {
    let title = header
        .pointer("/title")
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .unwrap_or_default();
    let subtitle_runs = header
        .pointer("/subtitle")
        .map(runs_from)
        .unwrap_or_default();
    let artists = header
        .pointer("/straplineTextOne")
        .map(runs_from)
        .map(|runs| extract_artists(&runs))
        .filter(|artists| !artists.is_empty())
        .unwrap_or_default();
    let playlist_id = value
        .pointer("/microformat/microformatDataRenderer/urlCanonical")
        .and_then(Value::as_str)
        .and_then(|url| url.rsplit('=').next())
        .map(str::to_string)
        .filter(|id| !id.is_empty())
        .or_else(|| find_playlist_id(header))
        .or_else(|| find_playlist_id(value))
        .unwrap_or_default();
    Album {
        id: browse_id.to_string(),
        playlist_id,
        title,
        artists,
        year: subtitle_runs
            .last()
            .and_then(|run| run.text.trim().parse::<u16>().ok()),
        thumbnail_url: header.get("thumbnail").and_then(thumbnail),
        explicit: false,
        ..Album::default()
    }
}

fn album_from_uploaded_header(value: &Value, browse_id: &str, header: &Value) -> Album {
    let subtitle_runs = header
        .pointer("/subtitle")
        .map(runs_from)
        .unwrap_or_default();
    Album {
        id: browse_id.to_string(),
        playlist_id: find_playlist_id(header)
            .or_else(|| find_playlist_id(value))
            .unwrap_or_default(),
        title: header
            .pointer("/title")
            .map(runs_from)
            .map(|runs| text_of(&runs))
            .unwrap_or_default(),
        artists: extract_artists(&subtitle_runs),
        year: subtitle_runs
            .last()
            .and_then(|run| run.text.trim().parse::<u16>().ok()),
        thumbnail_url: header
            .pointer("/thumbnail")
            .and_then(|thumb| crate::parse::thumbnail_url(Some(thumb))),
        explicit: false,
        ..Album::default()
    }
}

fn parse_songs(shelf: &Value, album: Option<&Album>) -> Vec<Song> {
    let contents = array_at(shelf, "/contents");
    contents
        .iter()
        .filter_map(|content| content.get("musicResponsiveListItemRenderer"))
        .filter_map(|renderer| album_song(renderer, album))
        .collect()
}

fn songs_and_token(value: &Value, album: &Album) -> (Vec<Song>, Option<String>) {
    // Regular album: secondaryContents.sectionListRenderer.contents[0].
    let secondary = value.pointer(
        "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents",
    );
    if let Some(contents) = secondary.and_then(Value::as_array) {
        for content in contents {
            if let Some(shelf) = content
                .get("musicPlaylistShelfRenderer")
                .or_else(|| content.get("musicShelfRenderer"))
            {
                let songs = parse_songs(shelf, Some(album));
                let token = continuation(shelf.get("continuations"))
                    .or_else(|| shelf_continuation(shelf.get("contents")));
                return (songs, token);
            }
        }
    }
    // Uploaded release / embedded fallback: singleColumn section contents.
    for content in embedded_section_list_contents(value) {
        if let Some(shelf) = content
            .get("musicShelfRenderer")
            .or_else(|| content.get("musicPlaylistShelfRenderer"))
        {
            let songs = parse_songs(shelf, Some(album));
            let token = continuation(shelf.get("continuations"))
                .or_else(|| shelf_continuation(shelf.get("contents")));
            return (songs, token);
        }
    }
    (Vec::new(), None)
}

/// Parse an album browse response (`MPREb_*` or uploaded detail).
pub fn from_browse(value: &Value, browse_id: &str) -> AlbumPage {
    let (album, songs, continuation) = if let Some(header) = uploaded_header(value)
        .filter(|_| browse_id.contains("FEmusic_library_privately_owned_release_detail"))
    {
        let album = album_from_uploaded_header(value, browse_id, header);
        let (songs, continuation) = songs_and_token(value, &album);
        (album, songs, continuation)
    } else if let Some(header) = responsive_header(value) {
        let album = album_from_responsive_header(value, browse_id, header);
        let (songs, continuation) = songs_and_token(value, &album);
        (album, songs, continuation)
    } else {
        (
            Album {
                id: browse_id.to_string(),
                ..Album::default()
            },
            Vec::new(),
            None,
        )
    };
    AlbumPage {
        description: description(
            value,
            responsive_header(value).or_else(|| uploaded_header(value)),
        ),
        album,
        songs,
        continuation,
    }
}

/// Rows from either album shelf shape without an album context.
fn parse_songs_flat(value: &Value) -> Vec<Song> {
    if let Some(contents) = value
        .pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents")
        .and_then(Value::as_array)
    {
        for content in contents {
            if let Some(shelf) = content
                .get("musicPlaylistShelfRenderer")
                .or_else(|| content.get("musicShelfRenderer"))
            {
                return parse_songs(shelf, None);
            }
        }
    }
    for content in embedded_section_list_contents(value) {
        if let Some(shelf) = content
            .get("musicShelfRenderer")
            .or_else(|| content.get("musicPlaylistShelfRenderer"))
        {
            return parse_songs(shelf, None);
        }
    }
    Vec::new()
}

/// Parse `browse {browseId: "VL<playlistId>"}` tracks (album tracks endpoint).
pub fn tracks_from_browse(value: &Value, album: Option<&Album>) -> (Vec<Song>, Option<String>) {
    // `songs_and_token` needs a concrete album for the request; when the caller
    // has no header context, parse rows with linked metadata only.
    let Some(album) = album.filter(|album| !album.id.is_empty()) else {
        return (parse_songs_flat(value), None);
    };
    songs_and_token(value, album)
}

/// Parse continuation items for album/playlist shelves.
pub fn tracks_from_continuation(
    value: &Value,
    album: Option<&Album>,
) -> (Vec<Song>, Option<String>) {
    let album = album.filter(|album| !album.id.is_empty());
    let contents = value
        .pointer("/continuationContents/musicPlaylistShelfContinuation/contents")
        .or_else(|| value.pointer("/continuationContents/musicShelfContinuation/contents"))
        .or_else(|| {
            value.pointer(
                "/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems",
            )
        })
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let songs = contents
        .iter()
        .filter_map(|content| content.get("musicResponsiveListItemRenderer"))
        .filter_map(|renderer| album_song(renderer, album))
        .collect();
    let token = continuation(
        value.pointer("/continuationContents/musicPlaylistShelfContinuation/continuations"),
    )
    .or_else(|| {
        continuation(value.pointer("/continuationContents/musicShelfContinuation/continuations"))
    });
    (songs, token)
}
