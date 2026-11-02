//! Regression tests for literal search and radio using trimmed real WEB_REMIX
//! responses captured 2026-11-02 (query `Shape of You`, radio `kJQP7kiw5Fk`).

use ml_core::{Queue, QueuePage, QueueSource, YtItem};
use ml_innertube::pages;
use ml_innertube::parse::url::{parse_url, ParsedUrl};
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

fn search_all() -> ml_core::SearchPage {
    pages::search::from_response(&fixture("search_literal_all.json"), "Shape of You")
}

fn radio_first_page() -> QueuePage {
    pages::next::from_response(&fixture("next_radio_despacito.json"))
}

#[test]
fn literal_query_returns_playable_results() {
    let page = search_all();
    assert_eq!(page.query, "Shape of You");
    assert!(!page.items.is_empty(), "exact query returned nothing");
    assert!(!page.songs.is_empty());
    assert_eq!(page.songs[0].id, "JGwWNGJdvx8");
    assert!(page
        .items
        .iter()
        .any(|item| matches!(item, YtItem::Song(_))));
}

#[test]
fn filtered_search_parses_songs_and_a_continuation() {
    let page = pages::search::from_response(&fixture("search_filter_songs.json"), "Shape of You");
    assert_eq!(page.items.len(), page.songs.len());
    assert!(page
        .items
        .iter()
        .all(|item| matches!(item, YtItem::Song(_))));
    assert!(page.continuation.is_some());
}

#[test]
fn search_continuation_adds_rows_the_first_page_lacks() {
    let first = pages::search::from_response(&fixture("search_filter_songs.json"), "Shape of You");
    let more = pages::search::from_continuation(&fixture("search_filter_songs_continuation.json"));
    assert!(!more.items.is_empty());
    assert!(more.continuation.is_some());
    for item in &more.items {
        assert!(
            !first.items.iter().any(|known| known.id() == item.id()),
            "continuation repeated {}",
            item.id()
        );
    }
}

#[test]
fn radio_next_page_has_a_multi_track_queue_and_current_index() {
    let page = radio_first_page();
    assert_eq!(page.title.as_deref(), Some("Despacito Mix"));
    assert!(page.songs.len() > 1, "radio queue was not multi-track");
    assert_eq!(page.current_index, Some(0));
    assert_eq!(page.songs[0].id, "kJQP7kiw5Fk");
    assert_eq!(page.songs[0].title, "Despacito");
    assert_eq!(
        page.songs[0]
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        ["Luis Fonsi", "Daddy Yankee"]
    );
    assert_eq!(page.songs[0].duration, Some(282));
    assert!(page.continuation.is_some());
}

#[test]
fn radio_continuation_parses_without_a_selected_row() {
    let first = radio_first_page();
    let more = pages::next::from_continuation(&fixture("next_radio_continuation.json"));
    assert!(more.songs.len() > 1);
    assert_eq!(more.current_index, None);
    assert!(more.continuation.is_some());
    for song in &more.songs {
        assert!(!first.songs.iter().any(|known| known.id == song.id));
    }
}

#[test]
fn radio_queue_advances_to_the_next_radio_track() {
    let page = radio_first_page();
    let mut queue = Queue::new(page.songs.clone(), QueueSource::Radio);
    let start = page.current_index.unwrap_or(0);
    assert_eq!(
        queue.jump_to(start).map(|song| song.id.as_str()),
        Some("kJQP7kiw5Fk")
    );
    let next = queue.next().expect("queue did not advance");
    assert_eq!(next.id, "DiItGE3eAyQ");
    assert_eq!(queue.position(), start + 1);
}

#[test]
fn urls_route_to_video_playlist_album_and_artist() {
    assert_eq!(
        parse_url("https://music.youtube.com/watch?v=kJQP7kiw5Fk&list=RDAMVM"),
        Some(ParsedUrl::Video("kJQP7kiw5Fk".into()))
    );
    assert_eq!(
        parse_url("https://www.youtube.com/playlist?list=PL123"),
        Some(ParsedUrl::Playlist("PL123".into()))
    );
    assert_eq!(
        parse_url("https://music.youtube.com/playlist?list=OLAK5uy_album"),
        Some(ParsedUrl::Album("OLAK5uy_album".into()))
    );
    assert_eq!(
        parse_url("https://music.youtube.com/channel/UCartist"),
        Some(ParsedUrl::Artist("UCartist".into()))
    );
}
