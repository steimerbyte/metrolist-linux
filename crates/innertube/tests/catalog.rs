//! Catalog/browse parity: charts, moods & genres, new releases, podcasts and
//! radio queue growth. Fixtures are trimmed WEB_REMIX responses captured
//! 2026-11-02 (`gl=DE`, `hl=en`, unauthenticated).

use ml_core::{AlbumRef, Artist, QueuePage, SectionKind, Song, YtItem};
use ml_innertube::pages;
use ml_innertube::{ClientConfig, InnertubeClient};
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

fn song(id: &str) -> Song {
    Song {
        id: id.to_string(),
        title: id.to_string(),
        ..Song::default()
    }
}

#[test]
fn charts_keep_video_chart_playlists_and_top_artists() {
    let page = pages::browse::from_browse(&fixture("charts_de.json"));
    assert_eq!(page.title.as_deref(), Some("Charts"));
    // The country filter shelf has no rows and is skipped.
    assert_eq!(page.sections.len(), 2);

    let videos = &page.sections[0];
    assert_eq!(videos.title, "Video charts");
    assert_eq!(videos.kind, SectionKind::Videos);
    assert_eq!(videos.items.len(), 3);
    assert_eq!(videos.items[0].title(), "Trending 20 Germany");
    assert_eq!(
        videos.items[0].id(),
        "OLAK5uy_nPQBmQpOIYYxuvYW4gIHsjgDqX9kc--Dg"
    );
    assert!(matches!(videos.items[0], YtItem::Playlist(_)));

    let artists = &page.sections[1];
    assert_eq!(artists.title, "Top artists");
    assert_eq!(artists.kind, SectionKind::Artists);
    assert_eq!(artists.items.len(), 2);
    match &artists.items[0] {
        YtItem::Artist(artist) => {
            assert_eq!(artist.name, "LISA");
            assert_eq!(artist.id, "UCXtPv6CbKfuXPNj3MyKFOig");
        }
        other => panic!("expected an artist, got {other:?}"),
    }
    assert!(page.continuation.is_none());
}

#[test]
fn moods_and_genres_expose_category_params() {
    let page = pages::moods::from_browse(&fixture("moods_and_genres.json"));
    assert_eq!(page.groups.len(), 2);
    assert_eq!(page.groups[0].title, "Moods & moments");
    assert_eq!(page.groups[0].items.len(), 3);
    let autumn = &page.groups[0].items[0];
    assert_eq!(autumn.title, "Autumn");
    assert_eq!(autumn.browse_id, "FEmusic_moods_and_genres_category");
    assert_eq!(autumn.params.as_deref(), Some("ggMPOg1uX3JBUDJTM2ZUUVJM"));
    assert_eq!(autumn.stripe_color, Some(4_288_988_671));

    assert_eq!(page.groups[1].title, "Genres");
    assert_eq!(page.groups[1].items.len(), 3);
    assert!(page
        .groups
        .iter()
        .flat_map(|group| &group.items)
        .all(|item| item.params.is_some()));
}

#[test]
fn mood_genre_page_lists_playlists() {
    let page = pages::browse::from_browse(&fixture("moods_and_genres_category.json"));
    assert_eq!(page.title.as_deref(), Some("Autumn"));
    let titles: Vec<&str> = page
        .sections
        .iter()
        .map(|section| section.title.as_str())
        .collect();
    assert_eq!(
        titles,
        ["Spotlight", "Featured playlists", "Community playlists"]
    );
    assert_eq!(
        page.sections
            .iter()
            .map(|section| section.items.len())
            .collect::<Vec<_>>(),
        [2, 2, 2]
    );
    assert!(page
        .sections
        .iter()
        .flat_map(|section| &section.items)
        .all(|item| matches!(item, YtItem::Playlist(_))));
    assert!(page.continuation.is_none());
}

#[test]
fn new_releases_split_albums_and_videos() {
    let page = pages::browse::from_browse(&fixture("new_releases.json"));
    assert_eq!(page.title.as_deref(), Some("New releases"));
    assert_eq!(page.sections.len(), 2);

    let albums = &page.sections[0];
    assert_eq!(albums.title, "Albums & singles");
    assert_eq!(albums.kind, SectionKind::Albums);
    assert_eq!(albums.items.len(), 2);
    match &albums.items[0] {
        YtItem::Album(album) => {
            assert_eq!(album.id, "MPREb_3nFl0NSBd1n");
            assert_eq!(album.title, "The Hearts of the North (feat. Laga)");
            assert_eq!(
                album.artists,
                vec![Artist::new(
                    "Ragalor",
                    Some("UCaf7GwsoGVGGSo_l_cQZQeQ".into())
                )]
            );
            assert!(album.playlist_id.is_empty() || album.playlist_id.starts_with("OLAK5uy_"));
        }
        other => panic!("expected an album, got {other:?}"),
    }

    let videos = &page.sections[1];
    assert_eq!(videos.title, "Music videos");
    assert_eq!(videos.kind, SectionKind::Videos);
    assert_eq!(videos.items.len(), 2);
    match &videos.items[0] {
        YtItem::Song(video) => {
            assert_eq!(video.id, "5j8fIDh1V0A");
            assert_eq!(video.title, "Te Imagino");
            assert_eq!(
                video.music_video_type.as_deref(),
                Some("MUSIC_VIDEO_TYPE_OMV")
            );
        }
        other => panic!("expected a video song, got {other:?}"),
    }
}

#[test]
fn new_release_album_grid_keeps_browse_and_playlist_ids() {
    let albums = pages::browse::albums(pages::browse::from_browse(&fixture(
        "new_releases_albums.json",
    )));
    assert_eq!(albums.len(), 4);
    assert_eq!(albums[0].id, "MPREb_r7BwZyiJV39");
    assert_eq!(albums[0].title, "The Rhythm Takes Control");
    assert!(!albums[0].playlist_id.is_empty());
    assert_eq!(albums[1].id, "MPREb_WpZEP1BF6D6");
}

#[test]
fn podcast_landing_parses_episode_shelves() {
    let feed = pages::home::from_browse(&fixture("podcasts_home.json"));
    let titles: Vec<&str> = feed
        .sections
        .iter()
        .map(|section| section.title.as_str())
        .collect();
    assert_eq!(titles, ["Parenting", "Religion", "History"]);
    assert_eq!(
        feed.sections
            .iter()
            .map(|section| section.items.len())
            .collect::<Vec<_>>(),
        [2, 2, 2]
    );
    match &feed.sections[0].items[0] {
        YtItem::Episode(episode) => {
            assert_eq!(episode.id, "BaIJZkhOmYg");
            assert_eq!(
                episode.title,
                "Wat je lichaam nog weet van vóór je geboorte | Diep - Anke Hornbüssel"
            );
        }
        other => panic!("expected an episode, got {other:?}"),
    }
    assert!(feed.continuation.is_some());
}

#[test]
fn home_chips_find_the_podcast_shelf() {
    let home = fixture("home_chips.json");
    assert!(pages::home::chips(&home).len() >= 11);
    assert_eq!(
        pages::home::chip_params(&home, "podcast").as_deref(),
        Some("ggNCSgQIDBADSgQICRABSgQICBABSgQIBxABSgQIBBABSgQIAxABSgQIDhABSgQIBhABSgQIDRABSgQIChABSgQIBRAB")
    );
    assert_eq!(
        pages::home::chip_params(&home, "PODCASTS"),
        pages::home::chip_params(&home, "podcast")
    );
    assert!(pages::home::chip_params(&home, "no such shelf").is_none());
}

#[test]
fn podcast_show_header_and_episodes_carry_credits() {
    let show_id = "MPSPPLjCQ2hLBRw6cUJyz87NF-GcxX3QwtfWPx";
    let page = pages::podcast::from_show_browse(&fixture("podcast_show.json"), show_id);
    assert_eq!(page.show.id, show_id);
    assert_eq!(page.show.title, "CERAMAH KIYAI");
    assert!(page.show.is_podcast);
    assert!(page.show.description.is_some());
    assert_eq!(
        page.show.author,
        Some(Artist::new(
            "NGAOS SUNDA",
            Some("UCtKYEyKHnNLz7sPxKJozwdg".into())
        ))
    );
    assert_eq!(page.channel_id.as_deref(), Some("UCtKYEyKHnNLz7sPxKJozwdg"));
    assert_eq!(page.episodes.len(), 2);
    assert_eq!(page.episodes[0].id, "J5RY8AICg20");
    assert_eq!(page.episodes[0].title, "KH ASEP DIMYATI TERBARU 2026");
    assert_eq!(
        page.episodes[0].podcast,
        Some(AlbumRef {
            id: show_id.to_string(),
            title: "CERAMAH KIYAI".to_string()
        })
    );
    assert_eq!(
        page.episodes[0].author.as_ref().map(|a| a.name.as_str()),
        Some("NGAOS SUNDA")
    );
    assert!(page.continuation.is_some());
}

#[test]
fn podcast_show_continuation_keeps_episode_ids() {
    let (episodes, token) =
        pages::podcast::from_show_continuation(&fixture("podcast_show_continuation.json"));
    assert_eq!(episodes.len(), 2);
    assert_eq!(episodes[0].id, "FQWBxs2munU");
    assert!(episodes.iter().all(|episode| !episode.id.is_empty()));
    assert!(token.is_some());
}

#[test]
fn queue_append_keeps_song_order_and_current_index() {
    let mut page = pages::next::from_response(&fixture("next_radio_despacito.json"));
    let first_ids: Vec<String> = page.songs.iter().map(|song| song.id.clone()).collect();
    let current_index = page.current_index;
    let appended = pages::next::append(
        &mut page,
        pages::next::from_continuation(&fixture("next_radio_continuation.json")),
    );
    assert_eq!(appended, 3);
    assert_eq!(page.current_index, current_index);
    assert_eq!(
        page.songs[..first_ids.len()]
            .iter()
            .map(|song| song.id.clone())
            .collect::<Vec<_>>(),
        first_ids
    );
    assert_eq!(page.songs[first_ids.len()].id, "9sg-A-eS6Ig");
    assert!(page.continuation.is_some(), "radio keeps a next token");
}

#[test]
fn queue_append_dedups_by_video_id() {
    let mut page = QueuePage {
        songs: vec![song("a"), song("b")],
        current_index: Some(1),
        title: Some("Radio".to_string()),
        continuation: Some("token-1".to_string()),
    };
    let appended = pages::next::append(
        &mut page,
        QueuePage {
            songs: vec![song("b"), song("c"), song("a")],
            current_index: None,
            title: None,
            continuation: Some("token-2".to_string()),
        },
    );
    assert_eq!(appended, 1);
    assert_eq!(
        page.songs
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(page.current_index, Some(1));
    assert_eq!(page.title.as_deref(), Some("Radio"));
    assert_eq!(page.continuation.as_deref(), Some("token-2"));
}

#[test]
fn queue_append_with_an_empty_page_keeps_the_queue() {
    let mut page = QueuePage {
        songs: vec![song("a")],
        current_index: Some(0),
        continuation: Some("token-1".to_string()),
        ..QueuePage::default()
    };
    assert_eq!(pages::next::append(&mut page, QueuePage::default()), 0);
    assert_eq!(page.songs.len(), 1);
    assert_eq!(page.current_index, Some(0));
    assert!(page.continuation.is_none());
}

#[tokio::test]
async fn queue_append_is_a_noop_without_a_token() {
    let client = InnertubeClient::new(ClientConfig::default());
    let mut page = QueuePage {
        songs: vec![song("a")],
        current_index: Some(0),
        ..QueuePage::default()
    };
    assert_eq!(client.queue_append(&mut page).await.unwrap(), 0);
    assert_eq!(page.songs.len(), 1);
}

#[test]
fn malformed_and_empty_pages_degrade_to_empty() {
    let shapes = [
        json!({}),
        json!({ "contents": {} }),
        json!({ "contents": { "singleColumnBrowseResultsRenderer": {} } }),
        json!({ "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [] } } }),
        json!({ "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{}] } } }),
    ];
    for value in shapes {
        assert_eq!(
            pages::browse::from_browse(&value),
            pages::browse::BrowsePage::default()
        );
        assert_eq!(
            pages::browse::from_continuation(&value),
            pages::browse::BrowsePage::default()
        );
        assert_eq!(
            pages::moods::from_browse(&value),
            pages::moods::MoodsAndGenres::default()
        );
        assert!(pages::home::from_browse(&value).sections.is_empty());
        assert!(pages::home::chips(&value).is_empty());
        assert!(pages::browse::albums(pages::browse::from_browse(&value)).is_empty());
        let show = pages::podcast::from_show_browse(&value, "MPSPbroken");
        assert_eq!(show.show.id, "MPSPbroken");
        assert!(show.show.title.is_empty());
        assert!(show.episodes.is_empty());
        assert!(show.continuation.is_none());
        let (episodes, token) = pages::podcast::from_show_continuation(&value);
        assert!(episodes.is_empty());
        assert!(token.is_none());
    }
}

#[test]
fn podcast_show_without_a_header_keeps_the_browse_id() {
    let page = pages::podcast::from_show_browse(
        &json!({ "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{
            "tabRenderer": { "content": { "sectionListRenderer": { "contents": [] } } }
        }]}}}),
        "MPSPempty",
    );
    assert_eq!(page.show.id, "MPSPempty");
    assert!(page.show.is_podcast);
    assert!(page.episodes.is_empty());
    assert!(page.channel_id.is_none());
}
