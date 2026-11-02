//! Live cookie-session tests. They are skipped unless `ML_LIVE_COOKIES` names a
//! cookies file, so `cargo test` stays hermetic:
//!
//! ```text
//! ML_LIVE_COOKIES=~/.local/share/ml/cookies.txt cargo test -p ml-innertube --test auth_live -- --nocapture
//! ```
//!
//! Output is masked. A file whose session Google has already rotated is a
//! skip, not a failure: the test asserts what the *server* says, so a stale
//! export cannot masquerade as a code regression.

use std::path::Path;

use ml_core::MlError;
use ml_innertube::cookies::CookieJar;
use ml_innertube::{ClientConfig, InnertubeClient};

fn live_client() -> Option<InnertubeClient> {
    let path = std::env::var_os("ML_LIVE_COOKIES")?;
    let jar = CookieJar::load(Path::new(&path)).expect("cookies file parses");
    assert!(!jar.is_empty(), "no cookies parsed");
    println!(
        "cookies: {} parsed, sapisid={}, authenticated={}",
        jar.len(),
        jar.sapisid().is_some(),
        jar.is_authenticated()
    );
    let mut client = InnertubeClient::new(ClientConfig {
        country: "DE".to_string(),
        ..ClientConfig::default()
    });
    client.set_cookie_jar(&jar);
    assert!(client.auth_snapshot().authenticated);
    Some(client)
}

#[tokio::test]
async fn cookie_session_reaches_the_account_menu() {
    let Some(client) = live_client() else {
        println!("skipped: ML_LIVE_COOKIES is not set");
        return;
    };
    match client.account_info().await {
        Ok(account) => {
            let masked = account.masked();
            println!(
                "account: name={} email={:?} handle={:?} channel={:?} avatar={}",
                masked.name,
                masked.email,
                masked.channel_handle,
                masked.channel_id,
                masked.thumbnail_url.is_some()
            );
            assert!(!account.name.is_empty());
            assert!(account
                .email
                .as_deref()
                .is_none_or(|email| email.contains('@')));
        }
        Err(MlError::AuthRequired) => {
            println!("skipped: the API rejected the session (401/403)");
        }
        Err(MlError::Unavailable(message)) if message.contains("signed-out") => {
            println!("skipped: the API answered the signed-out account menu");
        }
        Err(error) => panic!("account_info failed: {error}"),
    }
}

#[tokio::test]
async fn library_browsers_answer_signed_in() {
    let Some(client) = live_client() else {
        println!("skipped: ML_LIVE_COOKIES is not set");
        return;
    };
    let playlists = client.library_playlists().await;
    let albums = client.library_albums().await;
    let artists = client.library_artists().await;
    let songs = client.library_songs().await;
    if matches!(playlists, Err(MlError::AuthRequired)) {
        println!("skipped: the API rejected the session (401/403)");
        return;
    }
    for (name, page) in [
        ("playlists", playlists),
        ("albums", albums),
        ("artists", artists),
        ("songs", songs),
    ] {
        let page = page.unwrap_or_else(|error| panic!("library {name} failed: {error}"));
        println!(
            "library {name}: {} items, continuation={}",
            page.playlists.len() + page.albums.len() + page.artists.len() + page.songs.len(),
            page.continuation.is_some()
        );
    }
}
