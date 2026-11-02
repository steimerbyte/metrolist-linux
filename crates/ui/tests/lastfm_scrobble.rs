//! Last.fm scrobbling through the real Scrobbler with an in-memory transport.
//! No network and no credentials are used.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ml_core::{Artist, MlError, MlResult, Song};
use ml_integrations::lastfm::{
    LastFmClient, LastFmConfig, LastFmTransport, ScrobbleQueue, Scrobbler, ScrobblerHooks,
};

#[derive(Default)]
struct FakeTransport {
    requests: Mutex<Vec<BTreeMap<String, String>>>,
    invalid_session: bool,
}

impl LastFmTransport for FakeTransport {
    fn post<'a>(
        &'a self,
        params: &'a BTreeMap<String, String>,
    ) -> Pin<Box<dyn Future<Output = MlResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(params.clone());
            let method = params.get("method").map(String::as_str).unwrap_or("");
            if self.invalid_session {
                return Ok(r#"{"error":9,"message":"Invalid session key"}"#.to_string());
            }
            if method == "track.updateNowPlaying" {
                Ok(r#"{"nowplaying":{}}"#.to_string())
            } else {
                Ok(r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#.to_string())
            }
        })
    }
}

fn song(id: &str, duration: u32) -> Song {
    Song {
        id: id.to_string(),
        title: format!("Track {id}"),
        artists: vec![Artist::new("Artist", None)],
        duration: Some(duration),
        ..Song::default()
    }
}

fn temp_queue(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "metrolist-scrobble-{tag}-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), dir.join("lastfm-queue.jsonl"))
}

fn config() -> LastFmConfig {
    LastFmConfig::new(
        "api-key",
        Some("shared-secret".to_string()),
        Some("session-key".to_string()),
    )
}

#[tokio::test]
async fn start_threshold_end_dedup_and_flush() {
    let (dir, path) = temp_queue("flow");
    let mut scrobbler = Scrobbler::new(
        ScrobblerHooks {
            enabled: true,
            now_playing_enabled: true,
            ..ScrobblerHooks::default()
        },
        ScrobbleQueue::load(&path),
    );
    let start = Instant::now();
    let now_playing = scrobbler
        .on_song_started(&song("a", 200), 1_700_000_000, start)
        .expect("now-playing song");
    assert_eq!(now_playing.id, "a");

    assert!(scrobbler.poll(start + Duration::from_secs(49)).is_none());
    let due = scrobbler
        .poll(start + Duration::from_secs(50))
        .expect("threshold scrobble");
    assert!(scrobbler.enqueue(due.clone()));
    assert!(!scrobbler.enqueue(due.clone()), "same play must dedupe");
    assert_eq!(scrobbler.queue().len(), 1);

    let transport = Arc::new(FakeTransport::default());
    let client = LastFmClient::new(config(), transport.clone());
    let sent = scrobbler.flush(&client).await.expect("flush");
    assert_eq!(sent, 1);
    assert!(scrobbler.queue().is_empty());
    assert_eq!(scrobbler.stats().scrobbles_sent, 1);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert_eq!(
        transport.requests.lock().unwrap()[0]
            .get("method")
            .map(String::as_str),
        Some("track.scrobble")
    );

    // End-of-track path: a different start time is a new scrobble, and the
    // ended track reports through on_stopped.
    let second = Instant::now();
    scrobbler.on_song_started(&song("b", 200), 1_700_000_100, second);
    let ended = scrobbler
        .on_stopped(second + Duration::from_secs(101))
        .expect("end scrobble");
    assert!(scrobbler.enqueue(ended));
    assert_eq!(scrobbler.queue().len(), 1);
    drop(scrobbler);

    // The queue survives a restart and flushes after reload.
    let mut reloaded = Scrobbler::new(
        ScrobblerHooks {
            enabled: true,
            ..ScrobblerHooks::default()
        },
        ScrobbleQueue::load(&path),
    );
    assert_eq!(reloaded.queue().len(), 1);
    let restarted_transport = Arc::new(FakeTransport::default());
    let restarted_client = LastFmClient::new(config(), restarted_transport.clone());
    assert_eq!(reloaded.flush(&restarted_client).await.unwrap(), 1);
    assert!(reloaded.queue().is_empty());
    assert_eq!(restarted_transport.requests.lock().unwrap().len(), 1);

    // A short track never scrobbles even at its end.
    let short = Instant::now();
    reloaded.on_song_started(&song("short", 20), 1_700_000_200, short);
    assert!(reloaded.poll(short + Duration::from_secs(19)).is_none());
    assert!(reloaded
        .on_stopped(short + Duration::from_secs(19))
        .is_none());

    drop(reloaded);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn invalid_session_keeps_queue_and_reports_once() {
    let (dir, path) = temp_queue("invalid");
    let mut scrobbler = Scrobbler::new(
        ScrobblerHooks {
            enabled: true,
            ..ScrobblerHooks::default()
        },
        ScrobbleQueue::load(&path),
    );
    let start = Instant::now();
    scrobbler.on_song_started(&song("c", 200), 1_700_000_300, start);
    let due = scrobbler
        .poll(start + Duration::from_secs(50))
        .expect("threshold");
    assert!(scrobbler.enqueue(due));

    let transport = Arc::new(FakeTransport {
        invalid_session: true,
        ..FakeTransport::default()
    });
    let client = LastFmClient::new(config(), transport);
    let error = scrobbler.flush(&client).await.expect_err("invalid session");
    assert!(
        matches!(
            error,
            MlError::AuthRequired | MlError::Http { .. } | MlError::Unavailable(_)
        ),
        "{error:?}"
    );
    assert_eq!(scrobbler.queue().len(), 1, "failed scrobbles stay queued");
    let invalid = scrobbler.take_invalid_session().expect("session invalid");
    assert_eq!(invalid.code, 9);
    assert!(scrobbler.take_invalid_session().is_none(), "reported once");

    drop(scrobbler);
    let _ = std::fs::remove_dir_all(dir);
}
