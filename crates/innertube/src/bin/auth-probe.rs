//! Live auth probe: `auth-probe --cookies <path> [--auth-user <n>] <account|library>`.
//!
//! Prints only masked identities and counts — never a cookie value, never a
//! full email. Failures print the HTTP status/error text the client produced.
//!
//! The cookie file is opened **read-only** and is never rewritten: unlike
//! `yt-dlp --cookies <path>` (which saves its jar back on exit), nothing here
//! calls [`CookieJar::save`].

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use ml_innertube::cookies::{permission_warning, CookieJar};
use ml_innertube::{ClientConfig, InnertubeClient};
use serde_json::{json, Value};

fn usage() -> ! {
    eprintln!(
        "usage: auth-probe (--cookies <path> | --session <path>) [--auth-user <index>] [--data-sync-id <id>] [--discover] <account|library>"
    );
    std::process::exit(2)
}

fn print(value: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).unwrap_or_default()
    );
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut cookies: Option<PathBuf> = None;
    let mut session: Option<PathBuf> = None;
    let mut auth_user: Option<String> = None;
    let mut data_sync_id: Option<String> = None;
    let mut discover = false;
    let mut command: Option<String> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--cookies" => {
                index += 1;
                cookies = Some(PathBuf::from(
                    args.get(index).ok_or("--cookies needs a path")?,
                ));
            }
            "--session" => {
                index += 1;
                session = Some(PathBuf::from(
                    args.get(index).ok_or("--session needs a path")?,
                ));
            }
            "--auth-user" => {
                index += 1;
                auth_user = Some(args.get(index).ok_or("--auth-user needs a value")?.clone());
            }
            "--data-sync-id" => {
                index += 1;
                data_sync_id = Some(
                    args.get(index)
                        .ok_or("--data-sync-id needs a value")?
                        .clone(),
                );
            }
            "--discover" => discover = true,
            other => command = Some(other.to_string()),
        }
        index += 1;
    }

    let command = command.unwrap_or_else(|| usage());
    // `--cookies` is a read-only seed; `--session` is the writable copy that
    // receives every `Set-Cookie` rotation (0600). It wins when it already has
    // cookies, so a re-run continues from the last rotation.
    let seed = match cookies.as_deref() {
        Some(path) => {
            if let Some(warning) = permission_warning(path) {
                eprintln!("warning: {warning}");
            }
            Some(CookieJar::load(path).map_err(|error| error.to_string())?)
        }
        None => None,
    };
    let mut jar = match session.as_deref() {
        Some(path) if path.is_file() => CookieJar::load(path).map_err(|error| error.to_string())?,
        _ => CookieJar::default(),
    };
    if jar.is_empty() {
        jar = seed.unwrap_or_default();
    }
    if jar.is_empty() {
        return Err("no cookies parsed from --cookies/--session".to_string());
    }

    let mut client = InnertubeClient::new(ClientConfig {
        country: "DE".to_string(),
        ..ClientConfig::default()
    });
    client.set_cookie_jar(&jar);
    if let Some(path) = session.clone() {
        client.set_session_path(Some(path));
    }
    client.set_login_identity(data_sync_id.clone(), auth_user.clone());
    if discover && data_sync_id.is_none() {
        // Opt-in: one cookie-bearing page GET. It answers with ~10 `Set-Cookie`
        // rotations that a static export cannot persist, so prefer passing the
        // delegated id from the login flow. Only presence is reported.
        match client.discover_session().await {
            Ok(identity) => eprintln!(
                "session: data_sync_id={} visitor={} auth_user={:?}",
                identity.data_sync_id.is_some(),
                identity.visitor_data.is_some(),
                identity.auth_user
            ),
            Err(error) => eprintln!("session discovery failed: {error}"),
        }
    }
    let snapshot = client.auth_snapshot();

    if !snapshot.authenticated {
        let summary = json!({
            "auth": {
                "authenticated": snapshot.authenticated,
                "cookie_count": snapshot.cookie_count,
                "has_sapisid": snapshot.has_sapisid,
                "auth_user": snapshot.auth_user,
                "has_data_sync_id": snapshot.has_data_sync_id,
            },
            "error": "no SAPISID in the parsed jar: not signed in",
        });
        print(summary);
        return Err("no SAPISID in the parsed jar".to_string());
    }

    match command.as_str() {
        "account" => {
            let account = client
                .account_info()
                .await
                .map_err(|error| format!("account_info failed: {error}"))?
                .masked();
            print(json!({
                "auth": {
                    "authenticated": snapshot.authenticated,
                    "cookie_count": snapshot.cookie_count,
                    "has_sapisid": snapshot.has_sapisid,
                    "auth_user": snapshot.auth_user,
                    "has_data_sync_id": snapshot.has_data_sync_id,
                },
                "account": {
                    "name": account.name,
                    "email": account.email,
                    "channel_handle": account.channel_handle,
                    "channel_id": account.channel_id,
                    "thumbnail": account.thumbnail_url.is_some(),
                }
            }));
        }
        "library" => {
            let playlists = client
                .library_playlists()
                .await
                .map_err(|error| format!("library playlists failed: {error}"))?;
            let albums = client
                .library_albums()
                .await
                .map_err(|error| format!("library albums failed: {error}"))?;
            let artists = client
                .library_artists()
                .await
                .map_err(|error| format!("library artists failed: {error}"))?;
            let songs = client
                .library_songs()
                .await
                .map_err(|error| format!("library songs failed: {error}"))?;
            let uploads = client
                .library_uploads()
                .await
                .map_err(|error| format!("library uploads failed: {error}"))?;
            print(json!({
                "auth": {
                    "authenticated": snapshot.authenticated,
                    "cookie_count": snapshot.cookie_count,
                    "auth_user": snapshot.auth_user,
                    "has_data_sync_id": snapshot.has_data_sync_id,
                },
                "library": {
                    "playlists": playlists.playlists.len(),
                    "albums": albums.albums.len(),
                    "artists": artists.artists.len(),
                    "liked_songs": songs.songs.len(),
                    "uploads": uploads.songs.len(),
                    "continuations": {
                        "playlists": playlists.continuation.is_some(),
                        "albums": albums.continuation.is_some(),
                        "artists": artists.continuation.is_some(),
                        "songs": songs.continuation.is_some(),
                        "uploads": uploads.continuation.is_some(),
                    },
                    "sample_song": songs.songs.first().map(|song| json!({
                        "id": song.id,
                        "title": song.title,
                        "artists": song.artists.iter().map(|artist| artist.name.clone()).collect::<Vec<_>>(),
                        "duration": song.duration,
                    })),
                }
            }));
        }
        other => {
            eprintln!("unknown command: {other}");
            usage();
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
