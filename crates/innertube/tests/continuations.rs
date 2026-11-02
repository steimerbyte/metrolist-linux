//! Continuation and "see all" shapes: the same renderers arrive under different
//! envelopes than the first page.

use ml_innertube::pages::{album, artist, next, playlist, search};
use serde_json::{json, Value};

fn song_row(title: &str, video_id: &str, artist: &str, artist_id: &str) -> Value {
    json!({
        "flexColumns": [
            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": title}]}}},
            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{
                "text": artist,
                "navigationEndpoint": {"browseEndpoint": {"browseId": artist_id}}
            }]}}}
        ],
        "fixedColumns": [
            {"musicResponsiveListItemFixedColumnRenderer": {"text": {"runs": [{"text": "3:04"}]}}}
        ],
        "playlistItemData": {"videoId": video_id, "playlistSetVideoId": "set"},
        "thumbnail": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
            {"url": "cover", "width": 100, "height": 100}
        ]}}}
    })
}

#[test]
fn artist_see_all_flattens_a_playlist_shelf() {
    let response = json!({
        "header": {"musicHeaderRenderer": {"title": {"runs": [{"text": "Top songs"}]}}},
        "contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {
            "content": {"sectionListRenderer": {"contents": [
                {"musicPlaylistShelfRenderer": {
                    "contents": [
                        {"musicResponsiveListItemRenderer": song_row("One", "v1", "A", "UCa")},
                        {"musicResponsiveListItemRenderer": song_row("Two", "v2", "B", "UCb")}
                    ],
                    "continuations": [{"nextContinuationData": {"continuation": "more"}}]
                }}
            ]}}
        }}]}}
    });
    let feed = artist::items_from_browse(&response);
    assert_eq!(feed.sections.len(), 1);
    assert_eq!(feed.sections[0].title, "Top songs");
    assert_eq!(feed.sections[0].items.len(), 2);
    assert_eq!(feed.sections[0].items[0].title(), "One");
    assert_eq!(feed.continuation.as_deref(), Some("more"));
}

#[test]
fn search_and_playlist_shelf_continuations_keep_songs() {
    let response = json!({
        "continuationContents": {"musicShelfContinuation": {
            "contents": [
                {"musicResponsiveListItemRenderer": song_row("Next", "v1", "A", "UCa")}
            ],
            "continuations": [{"nextContinuationData": {"continuation": "again"}}]
        }}
    });
    let page = search::from_continuation(&response);
    assert_eq!(page.songs.len(), 1);
    assert_eq!(page.songs[0].id, "v1");
    assert_eq!(page.continuation.as_deref(), Some("again"));

    let response = json!({
        "continuationContents": {"musicPlaylistShelfContinuation": {
            "contents": [
                {"musicResponsiveListItemRenderer": song_row("Next", "v2", "B", "UCb")}
            ]
        }}
    });
    let page = playlist::from_continuation(&response);
    assert_eq!(page.songs.len(), 1);
    assert_eq!(page.songs[0].id, "v2");
    assert_eq!(page.songs[0].set_video_id.as_deref(), Some("set"));

    let (songs, token) = album::tracks_from_continuation(&response, None);
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].id, "v2");
    assert!(token.is_none());

    let feed = artist::from_continuation(&response);
    assert_eq!(feed.sections.len(), 1);
    assert_eq!(feed.sections[0].items.len(), 1);
}

#[test]
fn queue_panel_continuation_keeps_title_artists_and_selection() {
    let response = json!({
        "continuationContents": {"playlistPanelContinuation": {
            "contents": [{"playlistPanelVideoRenderer": {
                "videoId": "v3",
                "title": {"runs": [{"text": "Queued"}]},
                "longBylineText": {"runs": [{
                    "text": "Artist",
                    "navigationEndpoint": {"browseEndpoint": {"browseId": "UCa"}}
                }]},
                "lengthText": {"runs": [{"text": "4:05"}]},
                "playlistSetVideoId": "set",
                "selected": true
            }}],
            "continuations": [{"nextRadioContinuationData": {"continuation": "radio-more"}}]
        }}
    });
    let page = next::from_continuation(&response);
    assert_eq!(page.songs.len(), 1);
    assert_eq!(page.songs[0].title, "Queued");
    assert_eq!(page.songs[0].artists[0].name, "Artist");
    assert_eq!(page.songs[0].duration, Some(245));
    assert_eq!(page.current_index, Some(0));
    assert_eq!(page.continuation.as_deref(), Some("radio-more"));
}

#[test]
fn home_continuation_parses_section_list_continuation() {
    let response = json!({
        "continuationContents": {"sectionListContinuation": {
            "contents": [{"musicCarouselShelfRenderer": {
                "header": {"musicCarouselShelfBasicHeaderRenderer": {
                    "title": {"runs": [{"text": "More"}]}
                }},
                "contents": [{"musicTwoRowItemRenderer": {
                    "title": {"runs": [{"text": "Card"}]},
                    "subtitle": {"runs": [{"text": "Artist"}]},
                    "navigationEndpoint": {"browseEndpoint": {
                        "browseId": "VLitem",
                        "browseEndpointContextSupportedConfigs": {
                            "browseEndpointContextMusicConfig": {
                                "pageType": "MUSIC_PAGE_TYPE_PLAYLIST"
                            }
                        }
                    }},
                    "thumbnailRenderer": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
                        {"url": "cover", "width": 100, "height": 100}
                    ]}}}
                }}]
            }}],
            "continuations": [{"nextContinuationData": {"continuation": "home-more"}}]
        }}
    });
    let feed = ml_innertube::pages::home::from_continuation(&response);
    assert_eq!(feed.sections.len(), 1);
    assert_eq!(feed.sections[0].title, "More");
    assert_eq!(feed.sections[0].items.len(), 1);
    assert_eq!(feed.continuation.as_deref(), Some("home-more"));
}
