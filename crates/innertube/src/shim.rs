//! Local HTTP shim so mpv can play googlevideo URLs again.
//!
//! Measured 2026-11-27 (SABR rollout): a player URL answers 403 to
//! `Range: bytes=0-` and to plain GETs, and one URL serves only a few 512 KiB
//! ranges before it 403s too. mpv's ffmpeg always opens with `Range: bytes=0-`,
//! so it cannot load the URL at all, while [`crate::stream`] fetches 512 KiB
//! ranges and re-mints on 403/410. The shim accepts mpv's request on
//! 127.0.0.1, replays the requested span as one 206 and keeps mpv's cache and
//! seek behaviour (a seek is just another connection with a start offset).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ml_core::{AudioQuality, MlError, MlResult, StreamInfo};

use crate::stream::{self, RANGE_CHUNK};
use crate::InnertubeClient;

/// Header block cap: ffmpeg and libmpv send well under 4 KiB.
const MAX_HEAD: usize = 8 * 1024;
/// Re-mints allowed while one chunk keeps failing before the shim gives up.
const REMINT_ATTEMPTS: usize = 3;
/// Loopback port for a running shim, its token and the thread that serves it.
pub struct StreamShim {
    port: u16,
    token: String,
    _shared: Arc<Shared>,
}

struct Shared {
    client: InnertubeClient,
    /// Read per request: the UI owns the setting and its lock type.
    quality: Box<dyn Fn() -> AudioQuality + Send + Sync>,
    token: String,
    runtime: tokio::runtime::Runtime,
}

/// Byte span mpv asked for: `start` inclusive, `end` inclusive when present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u64,
    pub end: Option<u64>,
}

impl StreamShim {
    /// Bind loopback and start serving. The accept thread ends with the
    /// process; there is no shutdown handshake because `Services` outlives
    /// every player session.
    pub fn start(
        client: InnertubeClient,
        quality: impl Fn() -> AudioQuality + Send + Sync + 'static,
    ) -> MlResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| MlError::Network(format!("stream shim bind: {error}")))?;
        let port = listener
            .local_addr()
            .map_err(|error| MlError::Network(format!("stream shim address: {error}")))?
            .port();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|error| MlError::Network(format!("stream shim runtime: {error}")))?;
        let token = fresh_token();
        let shared = Arc::new(Shared {
            client,
            quality: Box::new(quality),
            token: token.clone(),
            runtime,
        });
        let accept = shared.clone();
        std::thread::Builder::new()
            .name("ml-stream-shim".to_string())
            .spawn(move || {
                for socket in listener.incoming() {
                    if let Ok(socket) = socket {
                        let shared = accept.clone();
                        std::thread::spawn(move || {
                            let _ = serve(socket, &shared);
                        });
                    }
                }
            })
            .map_err(|error| MlError::Network(format!("stream shim thread: {error}")))?;
        Ok(Self {
            port,
            token,
            _shared: shared,
        })
    }

    /// Loopback URL for `video_id`; safe because ids are `[A-Za-z0-9_-]`.
    pub fn url_for(&self, video_id: &str) -> String {
        format!("http://127.0.0.1:{}/{}/{}", self.port, self.token, video_id)
    }

    /// Test hook: the port the listener bound.
    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Parse `GET /<token>/<id> HTTP/1.1` and the optional `Range:` line.
/// `None` for another method, a foreign token, or an unparsable range.
pub fn parse_request(request: &str, token: &str) -> Option<(String, Span)> {
    let mut lines = request.lines();
    let mut first = lines.next()?.split_whitespace();
    if first.next()? != "GET" {
        return None;
    }
    let (_, path) = first.next()?.split_once('/')?;
    let (path_token, video_id) = path.split_once('/')?;
    if path_token != token || video_id.is_empty() {
        return None;
    }
    let mut span = Span {
        start: 0,
        end: None,
    };
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("range") {
            continue;
        }
        let value = value.trim();
        let (unit, range) = value.split_once('=')?;
        if !unit.eq_ignore_ascii_case("bytes") {
            return None;
        }
        let (start, end) = range.split_once('-')?;
        span.start = if start.trim().is_empty() {
            0
        } else {
            start.trim().parse().ok()?
        };
        span.end = if end.trim().is_empty() {
            None
        } else {
            Some(end.trim().parse().ok()?)
        };
    }
    Some((video_id.to_string(), span))
}

/// `HTTP/1.1 206` head answering one requested span of `total` bytes.
fn response_head(mime: &str, span: Span, total: u64) -> String {
    let end = span.end.map_or(total - 1, |end| end.min(total - 1));
    let length = end.saturating_sub(span.start) + 1;
    format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\n\
         Content-Range: bytes {}-{end}/{total}\r\nContent-Length: {length}\r\n\
         Connection: close\r\n\r\n",
        span.start
    )
}

fn serve(mut socket: TcpStream, shared: &Shared) -> MlResult<()> {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = socket.set_write_timeout(Some(Duration::from_secs(30)));
    let request = read_head(&mut socket)?;
    let Some((video_id, span)) = parse_request(&request, &shared.token) else {
        let _ = socket.write_all(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return Ok(());
    };
    let quality = (shared.quality)();
    let mut info = shared
        .runtime
        .block_on(shared.client.stream_info(&video_id, quality))?;
    // The first chunk also proves the total when the mint did not know it.
    let (first, total) = fetch_chunk(shared, &mut info, &video_id, quality, span.start)?;
    let total = info.content_length.or(total).unwrap_or(0);
    if total == 0 || span.start >= total {
        let _ = socket.write_all(
            b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return Ok(());
    }
    let last = span.end.map_or(total - 1, |end| end.min(total - 1));
    socket
        .write_all(response_head(&info.mime, span, total).as_bytes())
        .map_err(|error| MlError::Io(error.to_string()))?;
    socket
        .write_all(&first)
        .map_err(|error| MlError::Io(error.to_string()))?;
    let mut position = span.start + first.len() as u64;
    while position <= last {
        let (chunk, _) = fetch_chunk(shared, &mut info, &video_id, quality, position)?;
        if chunk.is_empty() {
            break;
        }
        socket
            .write_all(&chunk)
            .map_err(|error| MlError::Io(error.to_string()))?;
        position += chunk.len() as u64;
    }
    Ok(())
}

/// One ≤512 KiB ranged GET, re-minting the URL on 403/410. Returns the bytes
/// and the total announced by `Content-Range` when the server sent one.
fn fetch_chunk(
    shared: &Shared,
    info: &mut StreamInfo,
    video_id: &str,
    quality: AudioQuality,
    start: u64,
) -> MlResult<(Vec<u8>, Option<u64>)> {
    let mut last_status = 0;
    for _ in 0..REMINT_ATTEMPTS {
        let end = start.saturating_add(RANGE_CHUNK - 1);
        let response = shared.runtime.block_on(stream::send_range(
            &shared.client.http,
            &info.url,
            start,
            end,
        ))?;
        let status = response.status().as_u16();
        if stream::is_auth_failure(status) {
            shared.client.invalidate_stream(video_id, quality);
            *info = shared
                .runtime
                .block_on(shared.client.stream_info(video_id, quality))?;
            last_status = status;
            continue;
        }
        if status != 206 {
            return Err(MlError::Http {
                status,
                message: "stream shim: ranged GET".to_string(),
            });
        }
        let total = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(stream::parse_content_range)
            .and_then(|(_, _, total)| total);
        let body = shared
            .runtime
            .block_on(response.bytes())
            .map_err(|error| MlError::Network(error.to_string()))?;
        return Ok((body.to_vec(), total));
    }
    Err(MlError::Http {
        status: last_status,
        message: "stream shim: re-mint exhausted".to_string(),
    })
}

fn read_head(socket: &mut TcpStream) -> MlResult<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut probe = [0u8; 1024];
    while !buffer
        .windows(4)
        .any(|window| window == b"\r\n\r\n".as_slice())
    {
        let read = socket
            .read(&mut probe)
            .map_err(|error| MlError::Io(error.to_string()))?;
        if read == 0 || buffer.len() + read > MAX_HEAD {
            return Err(MlError::Io("stream shim: bad request head".to_string()));
        }
        buffer.extend_from_slice(&probe[..read]);
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

/// Loopback-only secret so other local processes cannot use the shim blindly.
fn fresh_token() -> String {
    use sha1::{Digest, Sha1};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha1::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(nanos.to_le_bytes());
    hasher
        .finalize()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mpv_open_ended_range() {
        let request = "GET /abc/5hj3dmm6Xw0 HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes=0-\r\n\r\n";
        let (id, span) = parse_request(request, "abc").expect("request");
        assert_eq!(id, "5hj3dmm6Xw0");
        assert_eq!(
            span,
            Span {
                start: 0,
                end: None
            }
        );
    }

    #[test]
    fn parses_seek_and_rejects_foreign_token() {
        let seek = "GET /tok/x HTTP/1.1\r\nrange: bytes=786432-1048575\r\n\r\n";
        let (_, span) = parse_request(seek, "tok").expect("seek");
        assert_eq!(span.start, 786_432);
        assert_eq!(span.end, Some(1_048_575));
        assert!(parse_request(seek, "other").is_none());
        assert!(parse_request("POST /tok/x HTTP/1.1\r\n\r\n", "tok").is_none());
    }

    #[test]
    fn head_answers_the_whole_span() {
        let head = response_head(
            "audio/webm",
            Span {
                start: 0,
                end: None,
            },
            2_353_698,
        );
        assert!(head.contains("Content-Range: bytes 0-2353697/2353698"));
        assert!(head.contains("Content-Length: 2353698"));
        let tail = response_head(
            "audio/webm",
            Span {
                start: 2_353_600,
                end: Some(9_999_999),
            },
            2_353_698,
        );
        assert!(tail.contains("Content-Range: bytes 2353600-2353697/2353698"));
        assert!(tail.contains("Content-Length: 98"));
    }
}
