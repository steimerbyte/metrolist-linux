//! Fake local Discord IPC socket: handshake, activity set, clear on pause.
//! No Discord client and no network are used.

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ml_core::{Artist, PlaybackStatus, PlayerSnapshot, Song};
use ml_integrations::DiscordPresence;

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;

fn read_frame(stream: &mut std::os::unix::net::UnixStream) -> Option<(u32, serde_json::Value)> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).ok()?;
    let op = u32::from_le_bytes(header[..4].try_into().ok()?);
    let len = u32::from_le_bytes(header[4..].try_into().ok()?) as usize;
    if len > 64 * 1024 {
        return None;
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).ok()?;
    Some((op, serde_json::from_slice(&body).ok()?))
}

fn frame(op: u32, value: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

struct FakeServer {
    dir: PathBuf,
    socket: PathBuf,
    frames: Arc<Mutex<Vec<(u32, serde_json::Value)>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FakeServer {
    fn start() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "metrolist-discord-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("discord-ipc-0");
        let listener = UnixListener::bind(&socket).unwrap();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let handle = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
            if let Some(frame) = read_frame(&mut stream) {
                sink.lock().unwrap().push(frame);
            }
            let ready = serde_json::json!({
                "cmd": "DISPATCH",
                "evt": "READY",
                "data": {"v": 1}
            });
            let _ = stream.write_all(&frame(OP_FRAME, &ready));
            while let Some(frame) = read_frame(&mut stream) {
                let close = frame.0 == 2;
                sink.lock().unwrap().push(frame);
                if close {
                    break;
                }
            }
        });
        Self {
            dir,
            socket,
            frames,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> Vec<(u32, serde_json::Value)> {
        let _ = std::os::unix::net::UnixStream::connect(&self.socket);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let frames = self.frames.lock().unwrap().clone();
        let _ = std::fs::remove_dir_all(&self.dir);
        frames
    }
}

#[test]
fn fake_discord_handshake_update_and_clear() {
    let server = FakeServer::start();
    let mut presence = DiscordPresence::with_app_id("999888777");
    let path = presence
        .connect_to(std::slice::from_ref(&server.socket))
        .expect("handshake");
    assert_eq!(path, server.socket);
    assert!(presence.is_connected());

    let song = Song {
        id: "dQw4w9WgXcQ".into(),
        title: "Never Gonna Give You Up".into(),
        artists: vec![Artist::new("Rick Astley", None)],
        duration: Some(213),
        ..Song::default()
    };
    let playing = PlayerSnapshot {
        status: PlaybackStatus::Playing,
        position_ms: 5_000,
        current: Some(song),
        ..PlayerSnapshot::default()
    };
    presence.on_snapshot(&playing).expect("set activity");
    let paused = PlayerSnapshot {
        status: PlaybackStatus::Paused,
        ..playing
    };
    presence.on_snapshot(&paused).expect("clear on pause");
    presence.disconnect();
    drop(presence);

    let frames = server.finish();
    assert_eq!(frames[0].0, OP_HANDSHAKE);
    assert_eq!(frames[0].1["client_id"], "999888777");
    let activities: Vec<&(u32, serde_json::Value)> = frames
        .iter()
        .filter(|(_, value)| value["cmd"] == "SET_ACTIVITY")
        .collect();
    assert_eq!(activities.len(), 2, "one set, one clear: {frames:?}");
    assert_eq!(
        activities[0].1["args"]["activity"]["details"],
        "Never Gonna Give You Up"
    );
    assert_eq!(activities[1].1["args"]["activity"], serde_json::Value::Null);
}
