//! Ports of `AlbumPageTest`, `PlaylistPageTest` and `PodcastPageTest`.

use ml_core::{Album, Artist};
use ml_innertube::parse::renderers::{
    album_song, extract_podcast_artists, multi_row_episode, podcast_episode_row,
};
use ml_innertube::parse::{extract_podcast_byline, owner_from_detail_subtitle, runs_from, Run};
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

fn album(id: &str, playlist_id: &str) -> Album {
    Album {
        id: id.to_string(),
        playlist_id: playlist_id.to_string(),
        title: "Compilation".to_string(),
        artists: vec![Artist::new("Various Artists", None)],
        thumbnail_url: Some("cover".to_string()),
        ..Album::default()
    }
}

fn album_row(subtitle: Value) -> Value {
    let mut row = fixture("album_row.json");
    row["flexColumns"][1]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"] = subtitle;
    row
}

#[test]
fn uploaded_compilation_track_keeps_its_tagged_performer() {
    let album = album(
        "FEmusic_library_privately_owned_release_detail_upload",
        "uploaded-playlist",
    );
    let song = album_song(
        &album_row(json!([{"text": "Tagged Performer"}])),
        Some(&album),
    )
    .unwrap();
    assert_eq!(song.artists, vec![Artist::new("Tagged Performer", None)]);
    assert_eq!(
        song.album.as_ref().map(|a| a.id.as_str()),
        Some(album.id.as_str())
    );
    assert_eq!(song.thumbnail_url.as_deref(), Some("cover"));
}

#[test]
fn art_track_performer_takes_precedence_over_label_in_known_album_context() {
    let label = Artist::new("Distributor", Some("UClabel".into()));
    let olak = Album {
        artists: vec![label.clone()],
        playlist_id: "OLAK5uy_album".into(),
        ..album("MPREb_album", "OLAK5uy_album")
    };
    let performer = json!([{"text": "10 Years and Song"}]);
    assert_eq!(
        album_song(&album_row(performer.clone()), Some(&olak))
            .unwrap()
            .artists,
        vec![Artist::new("10 Years and Song", None)]
    );
    assert_eq!(
        album_song(&album_row(json!([])), Some(&olak))
            .unwrap()
            .artists,
        vec![label.clone()]
    );
    let regular = Album {
        playlist_id: "regular-playlist".into(),
        ..olak.clone()
    };
    assert_eq!(
        album_song(&album_row(performer), Some(&regular))
            .unwrap()
            .artists,
        vec![label.clone()]
    );
    let linked = json!([{
        "text": "Linked Performer",
        "navigationEndpoint": {"browseEndpoint": {"browseId": "UCperformer"}}
    }]);
    assert_eq!(
        album_song(&album_row(linked), Some(&olak)).unwrap().artists,
        vec![Artist::new("Linked Performer", Some("UCperformer".into()))]
    );
}

#[test]
fn legacy_detail_subtitle_has_an_owner_in_its_second_section() {
    let owned = runs_from(&json!({"runs": [
        {"text": "Playlist"}, {"text": " • "}, {"text": "Owner & Friends"},
        {"text": " • "}, {"text": "14 songs"}
    ]}));
    assert_eq!(
        owner_from_detail_subtitle(&owned),
        Some(Artist::new("Owner & Friends", None))
    );
    assert_eq!(owner_from_detail_subtitle(&[Run::new("canción")]), None);
    let whitespace = runs_from(&json!({"runs": [
        {"text": "Playlist"}, {"text": " • "}, {"text": " "}
    ]}));
    assert_eq!(owner_from_detail_subtitle(&whitespace), None);
}

fn podcast_run(name: &str) -> Run {
    Run {
        page_type: Some("MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE".into()),
        ..Run::linked(name, "MPSPshow")
    }
}

#[test]
fn episode_credits_require_a_trusted_artist_or_podcast_link() {
    let episode = fixture("multi_row_episode.json");
    assert!(extract_podcast_artists(&episode, &[]).is_empty());
    assert!(extract_podcast_artists(&episode, &[Run::new("New episodes")]).is_empty());
    assert_eq!(
        extract_podcast_artists(&episode, &[podcast_run("Known Show")]),
        vec![Artist::new("Known Show", None)]
    );
    let mut with_secondary = episode.clone();
    with_secondary["secondarySubtitle"] = json!({"runs": [{
        "text": "Known Show",
        "navigationEndpoint": {
            "browseEndpoint": {
                "browseId": "MPSPshow",
                "browseEndpointContextSupportedConfigs": {
                    "browseEndpointContextMusicConfig": {
                        "pageType": "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE"
                    }
                }
            }
        }
    }]});
    assert_eq!(
        extract_podcast_artists(&with_secondary, &[]),
        vec![Artist::new("Known Show", None)]
    );
    let mut linked = episode;
    linked["secondSubtitle"] = json!({"runs": [{
        "text": "Song & Dance",
        "navigationEndpoint": {"browseEndpoint": {"browseId": "UCartist"}}
    }]});
    assert_eq!(
        extract_podcast_artists(&linked, &[]),
        vec![Artist::new("Song & Dance", Some("UCartist".into()))]
    );
}

#[test]
fn saved_episodes_retain_the_channel_byline_without_podcast_or_album_ids() {
    let channel = Run::new("Known Show & Friends");
    let podcast = podcast_run("Known Show");
    let album = Run::linked("Album", "MPREb_album");
    let metadata = [Run::new(" • "), Run::new("3,3 M reproducciones")];
    let joined = |runs: Vec<Run>| {
        let mut all = runs;
        all.extend(metadata.iter().cloned());
        all
    };
    assert_eq!(
        extract_podcast_byline(&joined(vec![channel.clone()])),
        vec![Artist::new("Known Show & Friends", None)]
    );
    assert_eq!(
        extract_podcast_byline(&joined(vec![podcast.clone()])),
        vec![Artist::new("Known Show", None)]
    );
    assert_eq!(
        extract_podcast_byline(&[channel.clone(), Run::new(" • "), album.clone()]),
        vec![Artist::new("Known Show & Friends", None)]
    );
    assert!(extract_podcast_byline(&[album]).is_empty());
    let linked = Run::linked("21 Savage and Song", "UCartist");
    assert_eq!(
        extract_podcast_byline(&[channel, Run::new(" • "), linked]),
        vec![Artist::new("21 Savage and Song", Some("UCartist".into()))]
    );
    assert!(extract_podcast_byline(&[]).is_empty());
}

#[test]
fn menu_podcast_link_validates_a_plain_show_byline_without_using_menu_label() {
    let mut episode = fixture("multi_row_episode.json");
    episode["secondSubtitle"] = json!({"runs": [{"text": "Known Show"}]});
    episode["menu"] = json!({"menuRenderer": {"items": [{
        "menuNavigationItemRenderer": {
            "text": {"runs": [{"text": "Go to podcast"}]},
            "icon": {"iconType": "PODCAST"},
            "navigationEndpoint": {
                "browseEndpoint": {
                    "browseId": "MPSPshow",
                    "browseEndpointContextSupportedConfigs": {
                        "browseEndpointContextMusicConfig": {
                            "pageType": "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE"
                        }
                    }
                }
            }
        }
    }]}});
    assert_eq!(
        extract_podcast_artists(&episode, &[]),
        vec![Artist::new("Known Show", None)]
    );
    episode["menu"] = Value::Null;
    assert!(extract_podcast_artists(&episode, &[]).is_empty());
    episode["secondSubtitle"] =
        json!({"runs": [{"text": "canción"}, {"text": "14 m reproducciones"}]});
    assert!(extract_podcast_artists(&episode, &[]).is_empty());
}

#[test]
fn multi_row_episode_uses_its_on_tap_video_id() {
    let mut episode = fixture("multi_row_episode.json");
    episode["onTap"] = json!({"watchEndpoint": {"videoId": "episode"}});
    let parsed = multi_row_episode(&episode).unwrap();
    assert_eq!(parsed.id, "episode");
    assert_eq!(parsed.title, "Episode");
}

#[test]
fn podcast_episode_row_uses_podcast_context_for_author_and_id() {
    let mut row = fixture("episode_row.json");
    row["flexColumns"][0]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"] =
        json!([{"text": "Episode"}]);
    let podcast = ml_core::Playlist {
        id: "MPSPshow".into(),
        title: "Known Show".into(),
        author: Some(Artist::new("Known Show", None)),
        ..Default::default()
    };
    let parsed = podcast_episode_row(&row, Some(&podcast)).unwrap();
    assert_eq!(parsed.author, Some(Artist::new("Known Show", None)));
    assert_eq!(
        parsed.podcast.as_ref().map(|p| p.id.as_str()),
        Some("MPSPshow")
    );
}
