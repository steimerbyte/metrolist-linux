//! Live HTTP probe: proves the real transport + error parsing work against
//! `ws.audioscrobbler.com` without needing credentials.
//!
//! The API answers an unsigned/unknown app with a JSON error object; seeing that
//! object decoded into `MlError` exercises DNS, TLS, the form POST, and
//! [`ml_integrations::lastfm::classify_body`] in one shot.
//!
//! Ignored by default; run with `ML_INTEGRATION_LIVE=1`.

use ml_integrations::lastfm::{HttpTransport, LastFmClient, LastFmConfig, LastFmTransport};
use std::collections::BTreeMap;

#[test]
#[ignore = "network; run with ML_INTEGRATION_LIVE=1"]
fn real_api_returns_a_classified_error_for_a_bogus_key() {
    if std::env::var_os("ML_INTEGRATION_LIVE").is_none() {
        eprintln!("skipping: ML_INTEGRATION_LIVE is not set");
        return;
    }
    let transport = HttpTransport::with_defaults();
    let mut params = BTreeMap::new();
    params.insert("method".to_string(), "auth.getToken".to_string());
    params.insert("api_key".to_string(), "definitely-not-a-key".to_string());
    params.insert("format".to_string(), "json".to_string());

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let body = runtime
        .block_on(LastFmTransport::post(&transport, &params))
        .expect("the API answers with HTTP 200 and a JSON error body");
    println!("body: {body}");
    let failure =
        ml_integrations::lastfm::classify_body(&body).expect_err("a bogus key must be rejected");
    println!("classified as {failure:?}");
    assert!(matches!(
        failure,
        ml_integrations::lastfm::Failure::Drop
            | ml_integrations::lastfm::Failure::Retry
            | ml_integrations::lastfm::Failure::InvalidSession
    ));

    // And the client surfaces it as an MlError, not Ok.
    let client = LastFmClient::new(
        LastFmConfig::new("definitely-not-a-key", None, Some("session".into())),
        std::sync::Arc::new(transport),
    );
    let error = runtime
        .block_on(client.update_now_playing(&ml_core::Song {
            title: "t".into(),
            artists: vec![ml_core::Artist::new("a", None)],
            ..ml_core::Song::default()
        }))
        .expect_err("bogus credentials cannot succeed");
    println!("client error: {error}");
}
