//! Playlist (`VLPL*`) page parsing.

use ml_core::{Artist, Playlist, PlaylistPage, Song};
use serde_json::Value;

use crate::parse::renderers::playlist_song;
use crate::parse::{
    array_at, continuation, extract_artists, owner_from_detail_subtitle, runs_from,
    shelf_continuation, text_of,
};

fn base_header(value: &Value) -> Option<&Value> {
    value.pointer(
        "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0",
    )
}

fn responsive_header(value: &Value) -> Option<&Value> {
    let base = base_header(value)?;
    base.get("musicResponsiveHeaderRenderer").or_else(|| {
        base.pointer(
            "/musicEditablePlaylistDetailHeaderRenderer/header/musicResponsiveHeaderRenderer",
        )
    })
}

fn detail_header(value: &Value) -> Option<&Value> {
    base_header(value)
        .and_then(|base| {
            base.pointer(
                "/musicEditablePlaylistDetailHeaderRenderer/header/musicDetailHeaderRenderer",
            )
        })
        .or_else(|| value.pointer("/header/musicDetailHeaderRenderer"))
}

fn music_header(value: &Value) -> Option<&Value> {
    value.pointer("/header/musicHeaderRenderer")
}

fn title(value: &Value, responsive: Option<&Value>, detail: Option<&Value>) -> String {
    responsive
        .and_then(|header| header.get("title"))
        .or_else(|| music_header(value).and_then(|header| header.get("title")))
        .or_else(|| detail.and_then(|header| header.get("title")))
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .unwrap_or_default()
}

fn runs(value: Option<&Value>) -> Vec<crate::parse::Run> {
    value.map(runs_from).unwrap_or_default()
}

fn author(value: &Value, responsive: Option<&Value>, detail: Option<&Value>) -> Option<Artist> {
    if let Some(run) = responsive.and_then(|header| header.pointer("/straplineTextOne/runs/0")) {
        let run = crate::parse::Run::from_value(run);
        if !run.text.trim().is_empty() {
            return Some(Artist::new(run.text.trim(), run.browse_id));
        }
    }
    let detail_subtitle = runs(detail.and_then(|header| header.get("subtitle")));
    if let Some(artist) = extract_artists(&detail_subtitle).into_iter().next() {
        return Some(artist);
    }
    let header_subtitle = runs(responsive.and_then(|header| header.get("subtitle")));
    if let Some(artist) = extract_artists(&header_subtitle).into_iter().next() {
        return Some(artist);
    }
    if let Some(facepile) = responsive.and_then(|header| header.get("facepile")) {
        let name = facepile
            .pointer("/avatarStackViewModel/text/content")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty());
        if let Some(name) = name {
            let id = facepile
                .pointer(
                    "/avatarStackViewModel/rendererContext/commandContext/onTap/innertubeCommand/browseEndpoint/browseId",
                )
                .and_then(Value::as_str)
                .map(str::to_string);
            return Some(Artist::new(name, id));
        }
    }
    if let Some(run) =
        music_header(value).and_then(|header| header.pointer("/straplineTextOne/runs/0"))
    {
        let run = crate::parse::Run::from_value(run);
        if !run.text.trim().is_empty() {
            return Some(Artist::new(run.text.trim(), run.browse_id));
        }
    }
    owner_from_detail_subtitle(&detail_subtitle)
}

fn song_count(value: &Value, responsive: Option<&Value>) -> Option<String> {
    let runs = runs(
        responsive
            .and_then(|header| header.get("secondSubtitle"))
            .or_else(|| music_header(value).and_then(|header| header.get("secondSubtitle"))),
    );
    runs.iter()
        .rev()
        .find(|run| {
            let text = run.text.to_lowercase();
            run.text.chars().any(|c| c.is_ascii_digit())
                && !text.contains("view")
                && !text.contains("hour")
                && !text.contains("minute")
        })
        .map(|run| run.text.clone())
}

fn description(
    value: &Value,
    responsive: Option<&Value>,
    detail: Option<&Value>,
) -> Option<String> {
    responsive
        .and_then(|header| header.pointer("/description/musicDescriptionShelfRenderer/description"))
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .filter(|text| !text.is_empty())
        .or_else(|| {
            detail
                .and_then(|header| header.get("description"))
                .map(runs_from)
                .map(|runs| text_of(&runs))
                .filter(|text| !text.is_empty())
        })
        .or_else(|| {
            value
                .pointer("/header/musicDetailHeaderRenderer/description")
                .map(runs_from)
                .map(|runs| text_of(&runs))
                .filter(|text| !text.is_empty())
        })
}

fn thumbnail(value: &Value, responsive: Option<&Value>, detail: Option<&Value>) -> Option<String> {
    crate::parse::thumbnail_url(responsive.and_then(|header| header.get("thumbnail")))
        .or_else(|| {
            music_header(value)
                .and_then(|header| header.get("thumbnail"))
                .and_then(|thumb| crate::parse::thumbnail_url(Some(thumb)))
        })
        .or_else(|| detail.and_then(|header| crate::parse::thumbnail_url(Some(header))))
}

fn is_editable(value: &Value) -> bool {
    let header_editable = base_header(value).is_some_and(|base| {
        base.get("musicEditablePlaylistDetailHeaderRenderer")
            .is_some()
    });
    header_editable
        || base_header(value)
            .and_then(|base| base.pointer("/musicResponsiveHeaderRenderer/menu/menuRenderer/items"))
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.pointer("/menuNavigationItemRenderer/icon/iconType")
                        .and_then(Value::as_str)
                        == Some("EDIT")
                })
            })
}

/// Parse `browse {browseId: "VL<playlistId>"}`.
pub fn from_browse(value: &Value, playlist_id: &str) -> PlaylistPage {
    let responsive = responsive_header(value);
    let detail = detail_header(value);
    let playlist = Playlist {
        id: playlist_id.to_string(),
        title: title(value, responsive, detail),
        author: author(value, responsive, detail),
        song_count_text: song_count(value, responsive),
        thumbnail_url: thumbnail(value, responsive, detail),
        description: description(value, responsive, detail),
        is_editable: is_editable(value),
        is_podcast: false,
        explicit: false,
    };
    let (songs, continuation) = songs_and_token(value);
    PlaylistPage {
        playlist,
        songs,
        continuation,
    }
}

fn songs_and_token(value: &Value) -> (Vec<Song>, Option<String>) {
    let Some(contents) = value
        .pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents")
        .and_then(Value::as_array)
    else {
        return (Vec::new(), None);
    };
    for content in contents {
        if let Some(shelf) = content
            .get("musicPlaylistShelfRenderer")
            .or_else(|| content.get("musicShelfRenderer"))
        {
            let songs = array_at(shelf, "/contents")
                .iter()
                .filter_map(|content| content.get("musicResponsiveListItemRenderer"))
                .filter_map(playlist_song)
                .collect();
            let token = continuation(shelf.get("continuations"))
                .or_else(|| shelf_continuation(shelf.get("contents")));
            return (songs, token);
        }
    }
    (Vec::new(), None)
}

/// Parse `browse {continuation}` for more playlist rows.
pub fn from_continuation(value: &Value) -> PlaylistPage {
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
        .filter_map(playlist_song)
        .collect();
    let token = continuation(
        value.pointer("/continuationContents/musicPlaylistShelfContinuation/continuations"),
    )
    .or_else(|| {
        continuation(value.pointer("/continuationContents/musicShelfContinuation/continuations"))
    });
    PlaylistPage {
        playlist: Playlist::default(),
        songs,
        continuation: token,
    }
}
