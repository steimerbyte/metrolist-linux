//! `MusicResponsiveHeaderRendererTest`, `YouTubeUrlParser` and `PlaybackRegistrationTest`.

use ml_innertube::pages::album;
use ml_innertube::parse::url::{parse_url, ParsedUrl};
use ml_innertube::player::validate_tracking_url;
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

#[test]
fn album_header_buttons_may_be_omitted() {
    let page = album::from_browse(&fixture("album_header_no_buttons.json"), "MPREb_x");
    assert_eq!(page.album.id, "MPREb_x");
    assert!(page.album.title.is_empty());
    assert!(page.album.playlist_id.is_empty());
    assert!(page.songs.is_empty());
    assert!(page.continuation.is_none());
}

#[test]
fn uploaded_release_header_parses_artist_year_and_playlist_id() {
    let page = album::from_browse(
        &fixture("uploaded_album.json"),
        "FEmusic_library_privately_owned_release_detail_upload",
    );
    assert_eq!(page.album.title, "Uploaded Album");
    assert_eq!(page.album.playlist_id, "PLuploaded");
    assert_eq!(
        page.album.artists,
        vec![ml_core::Artist::new(
            "Uploaded Artist",
            Some("UCuploader".into())
        )]
    );
    assert_eq!(page.album.year, Some(2021));
    assert_eq!(page.album.thumbnail_url.as_deref(), Some("cover"));
    assert_eq!(page.songs.len(), 2);
    assert_eq!(
        page.songs[0].album.as_ref().map(|a| a.id.as_str()),
        Some("FEmusic_library_privately_owned_release_detail_upload")
    );
}

#[test]
fn youtube_url_parser_keeps_the_upstream_precedence() {
    for url in [
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RDAMVM",
        "https://youtu.be/dQw4w9WgXcQ?si=x",
        "https://www.youtube.com/shorts/dQw4w9WgXcQ",
        "  https://youtube.com/watch?v=dQw4w9WgXcQ  ",
    ] {
        assert_eq!(
            parse_url(url),
            Some(ParsedUrl::Video("dQw4w9WgXcQ".into())),
            "{url}"
        );
    }
    assert_eq!(
        parse_url("https://www.youtube.com/playlist?list=PL123"),
        Some(ParsedUrl::Playlist("PL123".into()))
    );
    assert_eq!(
        parse_url("https://music.youtube.com/playlist?list=OLAK5uy_album"),
        Some(ParsedUrl::Album("OLAK5uy_album".into()))
    );
    assert_eq!(
        parse_url("https://music.youtube.com/channel/UCabc"),
        Some(ParsedUrl::Artist("UCabc".into()))
    );
    assert_eq!(
        parse_url("https://music.youtube.com/browse/MPREb_album"),
        Some(ParsedUrl::Artist("MPREb_album".into()))
    );
    for url in [
        "https://example.com/watch?v=dQw4w9WgXcQ",
        "https://www.youtube.com/channel/UCabc",
        "not a url",
        "",
    ] {
        assert_eq!(parse_url(url), None, "{url}");
    }
}

#[test]
fn only_the_canonical_playback_tracking_url_is_trusted() {
    assert!(
        validate_tracking_url("https://s.youtube.com:443/api/stats/playback?docid=song").is_ok()
    );
    for url in [
        "https://example.com/api/stats/playback",
        "http://s.youtube.com/api/stats/playback",
        "https://s.youtube.com:444/api/stats/playback",
        "https://user:pass@s.youtube.com/api/stats/playback",
        "https://s.youtube.com/api/stats/watchtime",
    ] {
        assert!(validate_tracking_url(url).is_err(), "{url}");
    }
}

#[test]
fn player_response_maps_playability_status() {
    let ok = json!({
        "playabilityStatus": {"status": "OK"},
        "streamingData": {"expiresInSeconds": "21540", "adaptiveFormats": [{
            "itag": 251, "mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 160542,
            "contentLength": "3594577", "audioQuality": "AUDIO_QUALITY_MEDIUM",
            "url": "https://rr1.googlevideo.com/videoplayback?x=1"
        }]},
        "videoDetails": {"videoId": "v", "title": "T", "author": "A", "viewCount": "12"}
    });
    let parsed = ml_innertube::player::parse_player(&ok).unwrap();
    assert_eq!(parsed.formats[0].content_length, Some(3_594_577));
    assert_eq!(parsed.expires_in_seconds, 21540);
    assert_eq!(parsed.media.view_count, Some(12));

    let login = json!({"playabilityStatus": {"status": "LOGIN_REQUIRED", "reason": "Sign in"}});
    assert!(matches!(
        ml_innertube::player::parse_player(&login),
        Err(ml_core::MlError::AuthRequired)
    ));
    let unavailable =
        json!({"playabilityStatus": {"status": "ERROR", "reason": "This video is unavailable"}});
    assert!(matches!(
        ml_innertube::player::parse_player(&unavailable),
        Err(ml_core::MlError::Unavailable(reason)) if reason == "This video is unavailable"
    ));
}
