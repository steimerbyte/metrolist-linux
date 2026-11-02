//! Provider ladder tests over a fake transport, plus the env-gated live LRCLIB call.

use std::sync::Arc;

use ml_lyrics::{
    clean_artist, clean_title, fetch_kugou, fetch_lrclib, refusal, FakeTransport, HttpTransport,
    LyricsClient, LyricsProvider, LyricsQuery, ReqwestTransport,
};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// A transport that always answers the same body, whatever the URL.
fn always(body: &str) -> Arc<dyn HttpTransport> {
    FakeTransport::new([("https://".to_string(), body.to_string())])
}

fn query() -> LyricsQuery {
    LyricsQuery::new("Never Gonna Give You Up", "Rick Astley")
        .with_album("Whenever You Need Somebody")
        .with_duration(212)
}

#[tokio::test]
async fn lrclib_prefers_synced_and_matches_duration() {
    let http = always(&fixture("lrclib_search_synced.json"));
    let doc = fetch_lrclib(http.as_ref(), &query()).await.unwrap();
    assert_eq!(doc.provider, "lrclib");
    assert!(doc.is_synced());
    assert_eq!(doc.lines.len(), 2);
    assert_eq!(doc.lines[0].start_ms, 290);
    assert_eq!(doc.lines[0].text, "Never gonna give you up");
    assert_eq!(doc.lines[1].start_ms, 5_000);
}

#[tokio::test]
async fn lrclib_falls_back_to_plain_text() {
    let http = always(&fixture("lrclib_search_plain.json"));
    let q = LyricsQuery::new("Plain Only", "Someone").with_duration(180);
    let doc = fetch_lrclib(http.as_ref(), &q).await.unwrap();
    assert!(!doc.is_synced());
    assert_eq!(
        doc.plain.as_deref(),
        Some("First plain line\nSecond plain line")
    );
}

#[tokio::test]
async fn lrclib_reports_not_found_when_every_strategy_is_empty() {
    let http = always(&fixture("lrclib_search_empty.json"));
    let error = fetch_lrclib(http.as_ref(), &query()).await.unwrap_err();
    assert!(matches!(error, ml_core::MlError::NotFound(_)), "{error:?}");
}

/// Instrumentals and hits with no text never win.
#[tokio::test]
async fn lrclib_ignores_instrumental_hits() {
    let http = always(&fixture("lrclib_search_instrumental.json"));
    let error = fetch_lrclib(http.as_ref(), &query()).await.unwrap_err();
    assert!(matches!(error, ml_core::MlError::NotFound(_)), "{error:?}");
}

/// The ladder stops at the first strategy that returns lyric-bearing tracks, and the
/// first strategy carries title, artist and album.
#[tokio::test]
async fn lrclib_first_strategy_sends_title_artist_album() {
    let fake = FakeTransport::new([(
        "https://lrclib.net".to_string(),
        fixture("lrclib_search_synced.json"),
    )]);
    let http = fake.clone();
    fetch_lrclib(http.as_ref(), &query()).await.unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].contains("track_name=Never+Gonna+Give+You+Up")
            || calls[0].contains("track_name=Never%20Gonna"),
        "{calls:?}"
    );
    assert!(calls[0].contains("artist_name=Rick"), "{calls:?}");
    assert!(calls[0].contains("album_name=Whenever"), "{calls:?}");
}

#[tokio::test]
async fn lrclib_walks_the_ladder_until_something_matches() {
    // First call (title+artist+album) is empty, so the second (title only) is used.
    let http = Arc::new(FakeTransport {
        routes: vec![
            (
                "https://lrclib.net/api/search?track_name=Never+Gonna+Give+You+Up&artist_name=Rick+Astley&album_name=Whenever+You+Need+Somebody".to_string(),
                fixture("lrclib_search_empty.json"),
            ),
            (
                "https://lrclib.net/api/search?track_name=Never+Gonna+Give+You+Up".to_string(),
                fixture("lrclib_search_synced.json"),
            ),
        ],
        ..Default::default()
    }) as Arc<dyn HttpTransport>;
    let doc = fetch_lrclib(http.as_ref(), &query()).await.unwrap();
    assert!(doc.is_synced());
}

/// Titles and artists are cleaned with the same rules the app uses.
#[test]
fn title_and_artist_cleaning() {
    assert_eq!(
        clean_title("Never Gonna Give You Up (Official Video)"),
        "Never Gonna Give You Up"
    );
    assert_eq!(clean_title("Song [Official Lyric Video]"), "Song");
    assert_eq!(clean_title("Song (feat. Someone)"), "Song");
    assert_eq!(clean_title("Song | Album"), "Song");
    assert_eq!(clean_title("Song 【MV】"), "Song");
    assert_eq!(clean_title("Plain Title"), "Plain Title");
    assert_eq!(clean_artist("Rick Astley & Someone"), "Rick Astley");
    assert_eq!(clean_artist("A, B and C"), "A");
    assert_eq!(clean_artist("Solo"), "Solo");
}

#[tokio::test]
async fn kugou_downloads_and_decodes_base64_lrc() {
    let http = Arc::new(FakeTransport {
        routes: vec![
            (
                "https://lyrics.kugou.com/search".to_string(),
                fixture("kugou_search.json"),
            ),
            (
                "https://lyrics.kugou.com/download".to_string(),
                fixture("kugou_download.json"),
            ),
        ],
        ..Default::default()
    }) as Arc<dyn HttpTransport>;
    let doc = fetch_kugou(http.as_ref(), &query()).await.unwrap();
    assert_eq!(doc.provider, "kugou");
    assert_eq!(doc.lines[0].start_ms, 290);
    assert_eq!(doc.lines[0].text, "Never gonna give you up");
}

#[tokio::test]
async fn kugou_rejects_candidates_outside_its_tolerance() {
    let http = Arc::new(FakeTransport {
        routes: vec![(
            "https://lyrics.kugou.com/search".to_string(),
            r#"{"candidates":[{"id":"1","accesskey":"k","duration":90000}]}"#.to_string(),
        )],
        ..Default::default()
    }) as Arc<dyn HttpTransport>;
    let error = fetch_kugou(http.as_ref(), &query()).await.unwrap_err();
    assert!(matches!(error, ml_core::MlError::NotFound(_)), "{error:?}");
}

/// Malformed provider payloads are errors, never panics.
#[tokio::test]
async fn malformed_provider_json_is_an_error() {
    let http = always("<html>502 Bad Gateway</html>");
    let error = fetch_lrclib(http.as_ref(), &query()).await.unwrap_err();
    assert!(matches!(error, ml_core::MlError::Parse(_)), "{error:?}");
}

/// The client walks providers in order and returns the first synced document.
#[tokio::test]
async fn client_returns_the_first_synced_document() {
    let http = always(&fixture("lrclib_search_synced.json"));
    let client = LyricsClient::with_transport(vec![LyricsProvider::LrcLib], http);
    let doc = client.fetch(&query()).await.unwrap();
    assert!(doc.is_synced());
    assert_eq!(doc.provider, "lrclib");
}

/// A plain-only result is kept as a fallback when nothing synced exists.
#[tokio::test]
async fn client_keeps_plain_lyrics_as_a_fallback() {
    let http = always(&fixture("lrclib_search_plain.json"));
    let client = LyricsClient::with_transport(vec![LyricsProvider::LrcLib], http);
    let q = LyricsQuery::new("Plain Only", "Someone").with_duration(180);
    let doc = client.fetch(&q).await.unwrap();
    assert!(!doc.is_synced());
    assert!(doc.plain.is_some());
}

/// Disabled providers refuse without touching the network.
#[tokio::test]
async fn disabled_providers_refuse_typed_errors() {
    let http = always("{}");
    let client = LyricsClient::with_transport(
        vec![
            LyricsProvider::BetterLyrics,
            LyricsProvider::LyricsPlus,
            LyricsProvider::Paxsenix,
            LyricsProvider::Zemer,
        ],
        http,
    );
    let error = client.fetch(&query()).await.unwrap_err();
    assert!(
        matches!(
            error,
            ml_core::MlError::AuthRequired | ml_core::MlError::Unavailable(_)
        ),
        "{error:?}"
    );
    assert!(matches!(
        refusal(LyricsProvider::BetterLyrics.kind()),
        ml_core::MlError::AuthRequired
    ));
}

#[test]
fn only_lrclib_and_kugou_are_available() {
    for provider in [LyricsProvider::LrcLib, LyricsProvider::KuGou] {
        assert!(provider.is_available(), "{}", provider.name());
    }
    for provider in [
        LyricsProvider::BetterLyrics,
        LyricsProvider::LyricsPlus,
        LyricsProvider::Paxsenix,
        LyricsProvider::Zemer,
    ] {
        assert!(!provider.is_available(), "{}", provider.name());
    }
}

/// Live LRCLIB call. Run with `ML_LYRICS_LIVE=1 cargo test -p ml-lyrics -- --ignored`.
#[tokio::test]
#[ignore = "network: set ML_LYRICS_LIVE=1 to run"]
async fn live_lrclib_returns_synced_lines() {
    if std::env::var("ML_LYRICS_LIVE").as_deref() != Ok("1") {
        eprintln!("skipped: ML_LYRICS_LIVE != 1");
        return;
    }
    let client = LyricsClient::with_transport(
        vec![LyricsProvider::LrcLib],
        Arc::new(ReqwestTransport::with_defaults()),
    );
    let doc = client
        .fetch(&LyricsQuery::new("Never Gonna Give You Up", "Rick Astley").with_duration(212))
        .await
        .expect("LRCLIB must answer");
    eprintln!(
        "provider={} lines={} first={:?}",
        doc.provider,
        doc.lines.len(),
        doc.lines.first().map(|l| (&l.text, l.start_ms))
    );
    assert!(!doc.lines.is_empty(), "expected synced lines");
}

// -------------------------------------------------- order, availability, typed refusals

/// The default ladder is LRCLIB only; KuGou is opt-in (upstream ships it off too).
#[test]
fn default_ladder_is_lrclib_only() {
    assert_eq!(
        LyricsClient::default_providers().providers(),
        &[LyricsProvider::LrcLib]
    );
    let defaults: Vec<LyricsProvider> = LyricsProvider::ALL
        .into_iter()
        .filter(|provider| provider.enabled_by_default())
        .collect();
    assert_eq!(defaults, vec![LyricsProvider::LrcLib]);
}

/// A stored id list drives the order; unknown ids are ignored, duplicates collapse, and
/// an all-garbage list falls back to LRCLIB instead of an empty ladder.
#[test]
fn provider_order_comes_from_stored_names() {
    let client = LyricsClient::from_names(&["kugou", "lrclib", "kugou", "nope"]);
    assert_eq!(
        client.providers(),
        &[LyricsProvider::KuGou, LyricsProvider::LrcLib]
    );
    assert_eq!(
        LyricsClient::from_names(&["LRCLIB"]).providers(),
        &[LyricsProvider::LrcLib]
    );
    assert_eq!(
        LyricsClient::from_names(&[]).providers(),
        &[LyricsProvider::LrcLib]
    );
    assert_eq!(
        LyricsProvider::from_name("  KuGou "),
        Some(LyricsProvider::KuGou)
    );
    assert_eq!(LyricsProvider::from_name(""), None);
}

/// The settings UI can list every provider with whether it works and why not.
#[test]
fn availability_report_names_every_provider() {
    let report = LyricsProvider::availability();
    assert_eq!(report.len(), LyricsProvider::ALL.len());
    for (provider, usable, reason) in report {
        assert_eq!(usable, provider.is_available(), "{}", provider.name());
        assert_eq!(reason.is_some(), !usable, "{}", provider.name());
    }
    let reason = |provider: LyricsProvider| provider.disabled_reason();
    assert!(reason(LyricsProvider::BetterLyrics).is_some());
    assert!(reason(LyricsProvider::LrcLib).is_none());
    assert!(reason(LyricsProvider::KuGou).is_none());
}

/// Order is respected even when the first provider only has plain lyrics: a later
/// synced document still wins, and the plain one is kept only as a fallback.
#[tokio::test]
async fn order_is_respected_and_synced_beats_plain() {
    let http = Arc::new(FakeTransport {
        routes: vec![(
            "https://lrclib.net".to_string(),
            fixture("lrclib_search_plain.json"),
        )],
        ..Default::default()
    }) as Arc<dyn HttpTransport>;
    let plain_first = LyricsClient::with_transport(vec![LyricsProvider::LrcLib], http.clone());
    let plain_query = LyricsQuery::new("Plain Only", "Someone").with_duration(180);
    let doc = plain_first.fetch(&plain_query).await.unwrap();
    assert!(!doc.is_synced());
    assert_eq!(
        plain_first.providers(),
        &[LyricsProvider::LrcLib],
        "config is kept verbatim"
    );

    // KuGou first returns synced lyrics; LRCLIB would only have plain text.
    let mixed = Arc::new(FakeTransport {
        routes: vec![
            (
                "https://lyrics.kugou.com/search".to_string(),
                fixture("kugou_search.json"),
            ),
            (
                "https://lyrics.kugou.com/download".to_string(),
                fixture("kugou_download.json"),
            ),
            (
                "https://lrclib.net".to_string(),
                fixture("lrclib_search_plain.json"),
            ),
        ],
        ..Default::default()
    }) as Arc<dyn HttpTransport>;
    let client =
        LyricsClient::with_transport(vec![LyricsProvider::KuGou, LyricsProvider::LrcLib], mixed);
    let doc = client.fetch(&query()).await.unwrap();
    assert_eq!(doc.provider, "kugou");
    assert!(doc.is_synced());
}

/// A provider that refuses does not stop the ladder: error aggregation only reports
/// when *every* provider failed.
#[tokio::test]
async fn disabled_first_provider_falls_through_to_lrclib() {
    let http = always(&fixture("lrclib_search_synced.json"));
    let client = LyricsClient::with_transport(
        vec![LyricsProvider::BetterLyrics, LyricsProvider::LrcLib],
        http,
    );
    let doc = client.fetch(&query()).await.unwrap();
    assert_eq!(doc.provider, "lrclib");
}

/// LRCLIB and KuGou are matched from their fixtures, so a matching regression shows up
/// without the network.
#[tokio::test]
async fn both_wired_providers_match_the_same_track() {
    let http = Arc::new(FakeTransport {
        routes: vec![
            (
                "https://lrclib.net".to_string(),
                fixture("lrclib_search_synced.json"),
            ),
            (
                "https://lyrics.kugou.com/search".to_string(),
                fixture("kugou_search.json"),
            ),
            (
                "https://lyrics.kugou.com/download".to_string(),
                fixture("kugou_download.json"),
            ),
        ],
        ..Default::default()
    }) as Arc<dyn HttpTransport>;
    for provider in [LyricsProvider::LrcLib, LyricsProvider::KuGou] {
        let client = LyricsClient::with_transport(vec![provider], http.clone());
        let doc = client.fetch(&query()).await.unwrap();
        assert!(doc.is_synced(), "{} produced no timings", provider.name());
        assert_eq!(doc.lines[0].start_ms, 290, "{}", provider.name());
    }
}
