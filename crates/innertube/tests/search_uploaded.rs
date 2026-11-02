//! Ports of `SearchPageTest`: fallback artists, localized metadata and uploads.

use ml_core::{Artist, YtItem};
use ml_innertube::parse::renderers::{
    card_item, carousel_items, responsive_item, shelf_items, two_row_item, RowOptions,
};
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

fn song(item: YtItem) -> ml_core::Song {
    match item {
        YtItem::Song(song) => song,
        other => panic!("expected song, got {other:?}"),
    }
}

fn metadata(runs: Value) -> Value {
    let mut row = fixture("song_row.json");
    row["flexColumns"][1]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"] = runs;
    row
}

#[test]
fn card_song_inherits_artist_when_response_only_contains_type_and_duration() {
    let row = metadata(json!([
        {"text": "Song"}, {"text": " • "}, {"text": "3:04"}, {"text": "6m\u{a0}plays"}
    ]));
    let fallback = Artist::new("Lupus Nocte", Some("UC123".into()));
    let parsed = song(
        responsive_item(
            &row,
            &RowOptions {
                fallback_artists: std::slice::from_ref(&fallback),
                is_uploaded: false,
            },
        )
        .unwrap(),
    );
    assert_eq!(parsed.artists, vec![fallback]);
    assert_eq!(parsed.duration, Some(184));
}

#[test]
fn localized_metadata_only_songs_stay_playable_across_surfaces() {
    let row = fixture("song_row.json");
    let parsed = song(responsive_item(&row, &RowOptions::default()).unwrap());
    assert!(parsed.artists.is_empty());
    assert_eq!(parsed.id, "videoId");

    let mut title_only = row.clone();
    title_only["flexColumns"] = json!([row["flexColumns"][0].clone()]);
    let parsed = song(responsive_item(&title_only, &RowOptions::default()).unwrap());
    assert!(parsed.artists.is_empty());

    let shelf = json!({"contents": [{"musicResponsiveListItemRenderer": row}]});
    let items = shelf_items(&shelf);
    assert_eq!(items.len(), 1);
    assert!(song(items.into_iter().next().unwrap()).artists.is_empty());

    let tile = json!({
        "title": {"runs": [{"text": "Song"}]},
        "subtitle": {"runs": [
            {"text": "canción"}, {"text": " • "}, {"text": "14 m reproducciones"},
            {"text": " • "}, {"text": "3,3 M reproducciones"}, {"text": " • "},
            {"text": "歌曲"}, {"text": " • "}, {"text": "1億 回再生"}
        ]},
        "navigationEndpoint": {"watchEndpoint": {"videoId": "videoId"}},
        "thumbnailRenderer": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
            {"url": "cover", "width": 100, "height": 100}
        ]}}}
    });
    let parsed = song(two_row_item(&tile).unwrap());
    assert!(parsed.artists.is_empty());
    assert_eq!(parsed.id, "videoId");

    let carousel = json!({
        "header": {"musicCarouselShelfBasicHeaderRenderer": {"title": {"runs": [{"text": "Songs"}]}}},
        "contents": [{"musicTwoRowItemRenderer": tile}]
    });
    let items = carousel_items(&carousel);
    assert!(song(items.into_iter().next().unwrap()).artists.is_empty());
    let section = ml_innertube::pages::artist::section_from_content(
        &json!({"musicCarouselShelfRenderer": carousel}),
    )
    .unwrap();
    assert!(song(section.1.into_iter().next().unwrap())
        .artists
        .is_empty());
}

#[test]
fn linked_artists_and_card_context_survive_localized_metadata() {
    let linked = json!({
        "text": "Song & Dance",
        "navigationEndpoint": {"browseEndpoint": {"browseId": "UCsong"}}
    });
    let runs = json!([
        {"text": "canción"}, {"text": " • "}, linked, {"text": " • "},
        {"text": "3,3 M reproducciones"}
    ]);
    let expected = vec![Artist::new("Song & Dance", Some("UCsong".into()))];
    let row = metadata(runs.clone());
    assert_eq!(
        song(responsive_item(&row, &RowOptions::default()).unwrap()).artists,
        expected
    );

    let card = json!({
        "title": {"runs": [{"text": "Song"}]},
        "subtitle": {"runs": runs},
        "thumbnail": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
            {"url": "cover", "width": 100, "height": 100}
        ]}}},
        "onTap": {"watchEndpoint": {"videoId": "videoId"}},
        "buttons": []
    });
    assert_eq!(song(card_item(&card).unwrap()).artists, expected);

    let fallback = Artist::new("21 Savage", Some("UC21".into()));
    let unlinked = metadata(json!([
        {"text": "canción"}, {"text": " • "}, {"text": "14 m reproducciones"}
    ]));
    assert_eq!(
        song(
            responsive_item(
                &unlinked,
                &RowOptions {
                    fallback_artists: std::slice::from_ref(&fallback),
                    is_uploaded: false,
                },
            )
            .unwrap()
        )
        .artists,
        vec![fallback]
    );
    let mut card_without_artist = card;
    card_without_artist["subtitle"] = json!({"runs": [
        {"text": "canción"}, {"text": " • "}, {"text": "14 m reproducciones"}
    ]});
    assert!(song(card_item(&card_without_artist).unwrap())
        .artists
        .is_empty());
}

#[test]
fn uploaded_library_artist_columns_retain_tagged_names_without_relaxing_metadata() {
    let row = metadata(json!([
        {
            "text": "21 Savage",
            "navigationEndpoint": {"browseEndpoint": {"browseId": "UC21"}}
        },
        {"text": ", "},
        {"text": "Song & Dance"}
    ]));
    assert_eq!(
        song(
            responsive_item(
                &row,
                &RowOptions {
                    fallback_artists: &[],
                    is_uploaded: true,
                },
            )
            .unwrap()
        )
        .artists,
        vec![
            Artist::new("21 Savage", Some("UC21".into())),
            Artist::new("Song & Dance", None),
        ]
    );
    assert_eq!(
        song(responsive_item(&row, &RowOptions::default()).unwrap()).artists,
        vec![Artist::new("21 Savage", Some("UC21".into()))]
    );
    let plain = metadata(json!([{"text": "Song & Dance"}]));
    assert_eq!(
        song(
            responsive_item(
                &plain,
                &RowOptions {
                    fallback_artists: &[],
                    is_uploaded: true,
                },
            )
            .unwrap()
        )
        .artists,
        vec![Artist::new("Song & Dance", None)]
    );
    let plain_metadata = metadata(json!([{"text": "14 m reproducciones"}]));
    assert!(
        song(responsive_item(&plain_metadata, &RowOptions::default()).unwrap())
            .artists
            .is_empty()
    );
}
