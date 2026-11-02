//! Live smoke test against the public Listen Together server.
//!
//! Disabled by default so `cargo test` stays offline:
//!
//! ```text
//! ML_TOGETHER_LIVE=1 cargo test -p ml-together --test live_server -- --nocapture
//! ```
//!
//! It creates exactly one room with a timestamp-derived username, asserts the
//! `room_created` reply, then leaves. A failed connection reports the exact
//! client error.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ml_together::{SessionEvent, TogetherClient, TogetherEvent};

#[test]
fn create_room_against_public_server() {
    if std::env::var_os("ML_TOGETHER_LIVE").is_none() {
        eprintln!("skipping live Listen Together test: set ML_TOGETHER_LIVE=1 to run");
        return;
    }

    let client = TogetherClient::new();
    let events = client.events();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before the epoch")
        .as_millis()
        % 1_000_000_000;
    let username = format!("mltest{stamp:09}");
    client.create_room(&username);

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut room_code = None;
    while Instant::now() < deadline {
        let event = match events.recv_timeout(Duration::from_millis(250)) {
            Ok(event) => event,
            Err(_) => continue,
        };
        match event {
            TogetherEvent::Session(event) => {
                if let SessionEvent::RoomCreated {
                    room_code: code, ..
                } = *event
                {
                    room_code = Some(code);
                    break;
                }
            }
            TogetherEvent::ConnectionError(error) => {
                panic!("Listen Together server unreachable: {error}");
            }
            TogetherEvent::Disconnected {
                will_reconnect: false,
            } => panic!("Listen Together server closed the connection before room_created"),
            _ => {}
        }
    }

    let room_code = room_code.expect("room_created within 30 s");
    assert!(
        !room_code.trim().is_empty(),
        "server returned an empty room code"
    );
    assert!(
        room_code
            .chars()
            .all(|character| character.is_ascii_alphanumeric()),
        "unexpected room code {room_code:?}"
    );
    eprintln!("created Listen Together room {room_code} as {username}");

    client.leave();
    std::thread::sleep(Duration::from_millis(500));
}
