//! Live lyric-provider check. Network is opt-in:
//! `ML_LYRICS_LIVE=1 cargo test -p ml-ui --test lyrics_live -- --ignored`.

use ml_lyrics::{LyricsClient, LyricsProvider, LyricsQuery};

#[tokio::test]
#[ignore = "network: set ML_LYRICS_LIVE=1 to run"]
async fn live_lrclib_japanese_lyrics_romanize() {
    if std::env::var("ML_LYRICS_LIVE").as_deref() != Ok("1") {
        eprintln!("skipped: ML_LYRICS_LIVE != 1");
        return;
    }
    let client = LyricsClient::new(vec![LyricsProvider::LrcLib]);
    let query = LyricsQuery::new("夜に駆ける", "YOASOBI").with_duration(261);
    let document = client.fetch(&query).await.expect("live LRCLIB document");
    assert!(document.is_synced(), "LRCLIB returned no timings");
    let romanized = ml_lyrics::romanized(&document).expect("Japanese lyrics detected");
    assert_eq!(romanized.lines.len(), document.lines.len());
    assert_eq!(romanized.lines[0].start_ms, document.lines[0].start_ms);
    assert!(
        romanized
            .lines
            .iter()
            .any(|line| line.text.chars().any(|c| c.is_ascii_alphanumeric())),
        "romanized document has no Latin text"
    );
    println!(
        "live LRCLIB romanize: provider={} lines={} first_start_ms={}",
        document.provider,
        document.lines.len(),
        document.lines[0].start_ms
    );
}
