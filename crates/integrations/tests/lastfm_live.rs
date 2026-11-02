//! Live Last.fm test: only runs when real credentials are in the environment.
//!
//! ```text
//! LASTFM_API_KEY=... LASTFM_SECRET=... LASTFM_SESSION=... \
//!   cargo test -p ml-integrations --test lastfm_live -- --ignored --nocapture
//! ```
//!
//! `LASTFM_LIVE=1` is required as an explicit opt-in so a developer machine with
//! keys exported cannot scrobble by accident. Submissions use a timestamp in the
//! past and a single, clearly labelled test track.

use ml_core::{Artist, Song};
use ml_integrations::lastfm::{
    HttpTransport, LastFmClient, LastFmConfig, Scrobble, ScrobblePolicy, ScrobbleQueue, Scrobbler,
    ScrobblerHooks,
};

fn credentials() -> Option<LastFmConfig> {
    if std::env::var_os("LASTFM_LIVE").is_none() {
        eprintln!("skipping: LASTFM_LIVE is not set");
        return None;
    }
    let api_key = std::env::var("LASTFM_API_KEY")
        .ok()
        .filter(|key| !key.is_empty());
    let session = std::env::var("LASTFM_SESSION")
        .ok()
        .filter(|key| !key.is_empty());
    match (api_key, session) {
        (Some(api_key), Some(session)) => Some(LastFmConfig::new(
            api_key,
            std::env::var("LASTFM_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            Some(session),
        )),
        _ => {
            eprintln!("skipping: LASTFM_API_KEY / LASTFM_SESSION are not set");
            None
        }
    }
}

#[test]
#[ignore = "needs real Last.fm credentials; run with LASTFM_LIVE=1"]
fn lastfm_accepts_now_playing_and_a_scrobble() {
    let Some(config) = credentials() else {
        return;
    };
    assert!(config.can_scrobble(), "config is incomplete");
    let client = LastFmClient::new(config, std::sync::Arc::new(HttpTransport::with_defaults()));

    // A deliberately identifiable test track, played "just now".
    let song = Song {
        id: "ml-integration-test".into(),
        title: "Metrolist Desktop Integration Test".into(),
        artists: vec![Artist::new("Metrolist Test Artist", None)],
        duration: Some(31),
        ..Song::default()
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime
        .block_on(client.update_now_playing(&song))
        .expect("now playing must be accepted");
    println!("now-playing accepted");

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let scrobble = Scrobble::from_song(&song, timestamp).expect("scrobble");
    runtime
        .block_on(client.scrobble(&scrobble))
        .expect("scrobble must be accepted");
    println!("scrobble accepted at {timestamp}");
}

#[test]
#[ignore = "needs real Last.fm credentials; run with LASTFM_LIVE=1"]
fn offline_queue_flushes_through_the_real_api() {
    let Some(config) = credentials() else {
        return;
    };
    let mut scrobbler = Scrobbler::new(
        ScrobblerHooks {
            enabled: true,
            now_playing_enabled: false,
            policy: ScrobblePolicy::default(),
        },
        ScrobbleQueue::in_memory(),
    );
    let song = Song {
        id: "ml-integration-test".into(),
        title: "Metrolist Desktop Integration Test".into(),
        artists: vec![Artist::new("Metrolist Test Artist", None)],
        duration: Some(31),
        ..Song::default()
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_sub(120);
    assert!(scrobbler.enqueue(Scrobble::from_song(&song, timestamp).unwrap()));

    let client = LastFmClient::new(config, std::sync::Arc::new(HttpTransport::with_defaults()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let sent = runtime.block_on(scrobbler.flush(&client)).expect("flush");
    assert_eq!(sent, 1);
    assert!(scrobbler.queue().is_empty());
    assert!(scrobbler.take_invalid_session().is_none());
    println!("flushed {sent} scrobble(s)");
}
