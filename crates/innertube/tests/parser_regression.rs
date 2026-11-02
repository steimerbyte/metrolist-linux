//! Port of upstream `ParserRegressionTest` (7 cases) using the Kotlin JSON literals.

use ml_core::{Artist, YtItem};
use ml_innertube::parse::renderers::{
    card_item, playlist_song, podcast_episode_row, responsive_item, two_row_item, RowOptions,
};
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

fn album(item: YtItem) -> ml_core::Album {
    match item {
        YtItem::Album(album) => album,
        other => panic!("expected album, got {other:?}"),
    }
}

fn playlist(item: YtItem) -> ml_core::Playlist {
    match item {
        YtItem::Playlist(playlist) => playlist,
        other => panic!("expected playlist, got {other:?}"),
    }
}

#[test]
fn related_album_retains_menu_only_artist_id_and_other_metadata() {
    let album = album(two_row_item(&fixture("two_row_album.json")).unwrap());
    assert_eq!(
        album.artists,
        vec![Artist::new("", Some("UCartist".into()))]
    );
    assert_eq!(album.year, Some(2024));
    assert_eq!(album.thumbnail_url.as_deref(), Some("cover"));
    assert_eq!(album.playlist_id, "playlist");
}

#[test]
fn related_playlist_retains_unlinked_owner_in_last_subtitle_section() {
    let playlist = playlist(two_row_item(&fixture("two_row_playlist.json")).unwrap());
    assert_eq!(playlist.author, Some(Artist::new("Owner & Friends", None)));
    assert_eq!(playlist.song_count_text.as_deref(), Some("12 songs"));
}

#[test]
fn summary_playlist_and_podcast_retain_unlinked_owner() {
    let from_card = playlist(card_item(&fixture("card_playlist.json")).unwrap());
    assert_eq!(from_card.author, Some(Artist::new("Owner & Friends", None)));
    let podcast = playlist(card_item(&fixture("card_podcast.json")).unwrap());
    assert!(podcast.is_podcast);
    assert_eq!(podcast.author, Some(Artist::new("Owner & Friends", None)));
}

#[test]
fn search_playlist_retains_unlinked_owner_with_or_without_type_label() {
    for prefix in [Some("Playlist"), None] {
        let mut renderer = fixture("search_playlist_row.json");
        let runs = if let Some(label) = prefix {
            json!([
                {"text": label}, {"text": " • "},
                {"text": "Owner & Friends"}, {"text": " • "}, {"text": "12 songs"}
            ])
        } else {
            json!([
                {"text": "Owner & Friends"}, {"text": " • "}, {"text": "12 songs"}
            ])
        };
        renderer["flexColumns"][1]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"] =
            runs;
        let playlist = playlist(responsive_item(&renderer, &RowOptions::default()).unwrap());
        assert_eq!(
            playlist.author,
            Some(Artist::new("Owner & Friends", None)),
            "{prefix:?}"
        );
    }
}

#[test]
fn podcast_episode_row_retains_unlinked_channel_byline() {
    let episode = podcast_episode_row(&fixture("episode_row.json"), None).unwrap();
    assert_eq!(episode.author, Some(Artist::new("Owner & Friends", None)));
    assert_eq!(episode.duration, Some(192));
    assert_eq!(episode.publish_date_text.as_deref(), Some("Yesterday"));
}

#[test]
fn home_and_artist_episode_tiles_retain_plain_and_podcast_linked_bylines() {
    for name in ["episode_tile_plain.json", "episode_tile_podcast_link.json"] {
        let episode = match two_row_item(&fixture(name)).unwrap() {
            YtItem::Episode(episode) => episode,
            other => panic!("expected episode for {name}, got {other:?}"),
        };
        assert_eq!(
            episode.author,
            Some(Artist::new("Owner & Friends", None)),
            "{name}"
        );
        assert_eq!(episode.title, "Title");
    }
}

#[test]
fn playlist_row_albums_do_not_require_a_set_video_id() {
    let mut renderer = fixture("episode_row.json");
    renderer["playlistItemData"] = json!({"videoId": "videoId"});
    renderer["flexColumns"][1]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"] =
        json!([{"text": "Artist"}, {"text": " • "}, {"text": "Album"}]);
    let song = playlist_song(&renderer).unwrap();
    assert_eq!(song.id, "videoId");
    assert_eq!(song.title, "Title");
    // Unlinked metadata is never promoted to an artist.
    assert!(song.artists.is_empty());
}
