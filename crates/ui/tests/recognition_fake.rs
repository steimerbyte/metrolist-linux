//! Recognition wiring without a microphone: a synthetic PCM fixture produces a
//! real signature and a local fake Shazam endpoint returns one match.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use ml_core::{Artist, Song};
use ml_recognition::{signature_uri_from_pcm16le, ShazamClient};
use ml_ui::pure::{
    best_recognition_match, recognition_error_message, recognition_subtitle,
    recognition_timing_text,
};

fn canned_response() -> &'static str {
    r#"{"tagid":"tag-fake","track":{"key":"42","title":"Fake Song","subtitle":"Fake Artist","images":{"coverart":"https://img/cover","coverarthq":"https://img/cover-hq"},"url":"https://www.shazam.com/track/42","isrc":"USRC17607839","genres":{"primary":"Pop"},"sections":[{"type":"SONG","metadata":[{"title":"Album","text":"Fake Album"},{"title":"Released","text":"2026"}]}],"hub":{"options":[{"providername":"YouTube","type":"VIDEO","actions":[{"uri":"https://www.youtube.com/watch?v=dQw4w9WgXcQ"}]}]}}}"#
}

fn serve_once(response: &'static str, status: u16) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake Shazam");
    let address = listener.local_addr().expect("fake address");
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = read_request(&mut stream);
            let body = if status == 200 { response } else { "" };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.flush();
        }
    });
    format!("http://{address}")
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        let Some(header_end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let header_end = header_end + 4;
        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if buffer.len() >= header_end + length {
            return Ok(());
        }
    }
}

fn sine_fixture(rate: u32) -> Vec<u8> {
    let samples: Vec<i16> = (0..rate)
        .map(|index| {
            let phase = (index as f32 / rate as f32) * 440.0 * std::f32::consts::TAU;
            (phase.sin() * 12_000.0) as i16
        })
        .collect();
    ml_recognition::samples_to_pcm16le(&samples)
}

#[test]
fn fake_signature_and_transport_produce_a_result_and_playable_match() {
    let pcm = sine_fixture(16_000);
    let signature = signature_uri_from_pcm16le(&pcm).expect("fixture signature");
    assert!(signature.starts_with("data:audio/vnd.shazam.sig;base64,"));
    assert!(signature.len() > 100);

    let base_url = serve_once(canned_response(), 200);
    let client = ShazamClient::with_base_url(base_url);
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    let result = runtime
        .block_on(client.recognize(&signature, 1_000))
        .expect("fake match");
    assert_eq!(result.title, "Fake Song");
    assert_eq!(result.artist, "Fake Artist");
    assert_eq!(result.album.as_deref(), Some("Fake Album"));
    assert_eq!(result.youtube_video_id.as_deref(), Some("dQw4w9WgXcQ"));
    assert_eq!(recognition_subtitle(&result), "Fake Artist · Fake Album");
    assert_eq!(
        recognition_timing_text(12_000, 1_400),
        "12.0s sample · matched in 1.4s"
    );

    let songs = vec![Song {
        id: "dQw4w9WgXcQ".to_string(),
        title: "Fake Song".to_string(),
        artists: vec![Artist::new("Fake Artist", None)],
        ..Song::default()
    }];
    assert_eq!(best_recognition_match(&result, &songs), Some(0));
}

#[test]
fn fake_transport_404_is_a_no_match_message() {
    let base_url = serve_once("", 404);
    let client = ShazamClient::with_base_url(base_url);
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    let error = runtime
        .block_on(client.recognize("data:audio/vnd.shazam.sig;base64,AA==", 12_000))
        .expect_err("404 is no match");
    assert_eq!(error, ml_recognition::RecognitionError::NoMatch);
    assert!(recognition_error_message(&error).contains("No match"));
}
