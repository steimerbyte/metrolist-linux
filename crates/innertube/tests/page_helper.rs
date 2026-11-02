//! Port of upstream `PageHelperTest` (9 cases): metadata runs never become artists.

use ml_core::Artist;
use ml_innertube::parse::{
    extract_artists, extract_duration, text_of, Run, PAGE_ALBUM, PAGE_ARTIST, PAGE_USER_CHANNEL,
};

fn linked(name: &str, id: &str, page_type: Option<&str>) -> Run {
    Run {
        page_type: page_type.map(str::to_string),
        ..Run::linked(name, id)
    }
}

#[test]
fn duration_is_not_parsed_as_an_artist() {
    assert!(extract_artists(&[Run::new("3:42")]).is_empty());
}

#[test]
fn search_metadata_is_not_parsed_as_an_artist() {
    let runs = vec![
        Run::new("Song"),
        Run::new(" • "),
        Run::new("3:04"),
        Run::new("6m\u{a0}plays"),
    ];
    assert!(extract_artists(&runs).is_empty());
    assert_eq!(extract_duration(&runs), Some(184));
}

#[test]
fn linked_artist_is_found_after_search_type_label() {
    let runs = vec![
        Run::new("Song"),
        Run::new(" • "),
        Run::linked("Lupus Nocte", "UC123"),
    ];
    assert_eq!(
        extract_artists(&runs),
        vec![Artist::new("Lupus Nocte", Some("UC123".into()))]
    );
}

#[test]
fn unlinked_metadata_is_never_promoted_to_artists() {
    let runs = vec![
        Run::new("Artist"),
        Run::new(" • "),
        Run::new("Album"),
        Run::new(" • "),
        Run::new("3:42"),
    ];
    assert!(extract_artists(&runs).is_empty());
}

#[test]
fn commas_between_linked_artists_are_not_parsed_as_artists() {
    let runs = vec![
        Run::linked("Primary Artist", "UC123"),
        Run::new(", "),
        Run::linked("Featured Artist", "UC456"),
    ];
    assert_eq!(
        extract_artists(&runs),
        vec![
            Artist::new("Primary Artist", Some("UC123".into())),
            Artist::new("Featured Artist", Some("UC456".into())),
        ]
    );
}

#[test]
fn linked_artist_does_not_authorize_unlinked_neighbors() {
    let runs = vec![
        Run::linked("Primary Artist", "UC123"),
        Run::new(" & "),
        Run::new("Featured Artist"),
    ];
    assert_eq!(
        extract_artists(&runs),
        vec![Artist::new("Primary Artist", Some("UC123".into()))]
    );
}

#[test]
fn localized_counts_and_labels_cannot_become_artists_but_linked_names_remain_whole() {
    let mut runs: Vec<Run> = [
        "canción",
        "14 m reproducciones",
        "3,3 M reproducciones",
        "歌曲",
        "1億 回再生",
    ]
    .iter()
    .map(|text| Run::new(*text))
    .collect();
    assert!(extract_artists(&runs).is_empty());

    runs.extend([
        Run::linked("21 Savage", "UC21"),
        Run::new(" & "),
        Run::linked("10 Years and Song & Dance", "UC10"),
        Run::linked(" ", "UCblank"),
        Run::linked("Album title", "MPREb_album"),
    ]);
    assert_eq!(
        extract_artists(&runs),
        vec![
            Artist::new("21 Savage", Some("UC21".into())),
            Artist::new("10 Years and Song & Dance", Some("UC10".into())),
        ]
    );
    assert_eq!(text_of(&runs[6..7]), " & ");
}

#[test]
fn user_channel_link_is_trusted_but_album_typed_link_is_not() {
    let runs = vec![
        linked("Song", "MPLA_channel", Some(PAGE_USER_CHANNEL)),
        linked("Album", "UCnotAnArtist", Some(PAGE_ALBUM)),
    ];
    assert_eq!(
        extract_artists(&runs),
        vec![Artist::new("Song", Some("MPLA_channel".into()))]
    );
}

#[test]
fn typed_artist_endpoint_does_not_require_a_channel_id() {
    let runs = vec![linked("Artist", "MPLA123", Some(PAGE_ARTIST))];
    assert_eq!(
        extract_artists(&runs),
        vec![Artist::new("Artist", Some("MPLA123".into()))]
    );
}
