//! Single instance + argv/deep-link forwarding.
//!
//! `single_instance::SingleInstance` takes an abstract `AF_UNIX` name on Linux,
//! which proves exclusivity but carries no payload. So the primary instance
//! also listens on a real socket in `$XDG_RUNTIME_DIR/<app_id>.sock`; a second
//! launch connects, writes one line per argv, and exits. Framing is one
//! length-prefixed argument per write so a value containing a newline still
//! round-trips.
//!
//! [`InstanceGuard::try_acquire`] never blocks the caller: the accept loop runs
//! on its own thread and hands arguments to a `crossbeam_channel` receiver.
//!
//! ## Verus scope
//!
//! The framing helpers ([`encode_argv`], [`decode_argv`]) are pure and total,
//! but they own `Vec`/`String` allocation and integer parsing; the proof lead
//! owns their spec. Sockets and threads are outside Verus.

use std::io::{BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use crossbeam_channel::{bounded, Receiver, Sender};
use ml_core::{MlError, MlResult};

/// Prefix of the socket file name inside `$XDG_RUNTIME_DIR`.
const SOCKET_SUFFIX: &str = ".sock";
/// Upper bound on one forwarded argument, so a hostile writer cannot balloon memory.
const MAX_ARG_BYTES: usize = 64 * 1024;
/// Upper bound on forwarded arguments per launch.
const MAX_ARGS: usize = 4096;

/// Encode argv as `len\nbytes` pairs.
///
/// Lengths are byte counts of the UTF-8 argument, so arguments may contain
/// newlines and any other byte; empty arguments encode as `0\n`.
pub fn encode_argv<I, S>(args: I) -> Vec<u8>
where
    I: IntoIterator<Item = S>,
    S: AsRef<[u8]>,
{
    let mut out = Vec::new();
    for arg in args {
        let arg = arg.as_ref();
        out.extend_from_slice(arg.len().to_string().as_bytes());
        out.push(b'\n');
        out.extend_from_slice(arg);
    }
    out
}

/// Decode [`encode_argv`] output. Stops at the first malformed or truncated
/// entry and returns what was parsed so far.
pub fn decode_argv(bytes: &[u8]) -> Vec<String> {
    let mut args = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() && args.len() < MAX_ARGS {
        let Some(newline) = rest.iter().position(|byte| *byte == b'\n') else {
            break;
        };
        let len = std::str::from_utf8(&rest[..newline])
            .ok()
            .and_then(|text| text.parse::<usize>().ok());
        let Some(len) = len.filter(|len| *len <= MAX_ARG_BYTES) else {
            break;
        };
        let body = &rest[newline + 1..];
        if body.len() < len {
            break;
        }
        let Some(arg) = std::str::from_utf8(&body[..len]).ok() else {
            break;
        };
        args.push(arg.to_owned());
        rest = &body[len..];
    }
    args
}

/// Socket path for an app id: `$XDG_RUNTIME_DIR/<app_id>.sock`.
///
/// Errors when `XDG_RUNTIME_DIR` is unset or not a directory: the socket must
/// live in a per-user, already-private directory, and `fallback` to `/tmp`
/// would hand argv forwarding to another user.
pub fn socket_path(app_id: &str) -> MlResult<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .ok_or_else(|| MlError::Io("XDG_RUNTIME_DIR is not set; cannot forward argv".into()))?;
    Ok(dir.join(format!("{app_id}{SOCKET_SUFFIX}")))
}

/// Outcome of taking the lock.
#[derive(Debug)]
pub enum Instance {
    /// We are the first process; `guard` holds the lock and `requests` yields
    /// forwarded argv from later launches.
    Primary(InstanceGuard),
    /// Another process owns the lock and received our argv.
    Forwarded,
}

/// Holds the single-instance lock and the forwarding socket for the process
/// lifetime. Dropping it removes the socket and stops the accept thread.
pub struct InstanceGuard {
    _lock: single_instance::SingleInstance,
    socket_path: PathBuf,
    requests: Receiver<Vec<String>>,
    shutdown: Option<Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// `true` when the lock was acquired but the socket could not be served;
    /// forwarding then has nowhere to go.
    degraded: Option<String>,
}

impl std::fmt::Debug for InstanceGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceGuard")
            .field("socket_path", &self.socket_path)
            .field("degraded", &self.degraded)
            .finish()
    }
}

impl InstanceGuard {
    /// Try to become the primary instance.
    ///
    /// - First process: `Ok(Instance::Primary(guard))`.
    /// - Another process holds the lock and accepted our argv: `Ok(Instance::Forwarded)`.
    /// - Another process holds the lock but no socket answers: the argv could
    ///   not be delivered, so this reports the socket error instead of silently
    ///   starting a second window.
    pub fn try_acquire(app_id: &str) -> MlResult<Instance> {
        let lock = single_instance::SingleInstance::new(app_id)
            .map_err(|error| MlError::Io(format!("single-instance lock: {error}")))?;

        let path = socket_path(app_id)?;
        if !lock.is_single() {
            forward(&path)?;
            return Ok(Instance::Forwarded);
        }

        // A stale socket from a crashed run would block bind(); the abstract
        // lock above already proves nobody live owns it.
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                let reason = format!("bind {}: {error}", path.display());
                return Ok(Instance::Primary(Self::degraded(lock, path, reason)));
            }
        };
        let _ = std::fs::set_permissions(
            &path,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        );
        let (requests_tx, requests) = bounded(64);
        let (shutdown_tx, shutdown_rx) = bounded(1);
        let thread = std::thread::Builder::new()
            .name("ml-instance".into())
            .spawn(move || serve(listener, requests_tx, shutdown_rx))
            .map_err(|error| MlError::Io(format!("spawn instance thread: {error}")))?;

        Ok(Instance::Primary(Self {
            _lock: lock,
            socket_path: path,
            requests,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
            degraded: None,
        }))
    }

    /// Convenience for callers that only care about being primary: `Ok(None)`
    /// means another instance was notified and this process should exit.
    pub fn acquire(app_id: &str) -> MlResult<Option<Self>> {
        match Self::try_acquire(app_id)? {
            Instance::Primary(guard) => Ok(Some(guard)),
            Instance::Forwarded => Ok(None),
        }
    }

    fn degraded(
        lock: single_instance::SingleInstance,
        socket_path: PathBuf,
        reason: String,
    ) -> Self {
        let (_tx, requests) = bounded(0);
        Self {
            _lock: lock,
            socket_path,
            requests,
            shutdown: None,
            thread: None,
            degraded: Some(reason),
        }
    }

    /// Arguments forwarded by later launches, minus argv[0].
    pub fn requests(&self) -> &Receiver<Vec<String>> {
        &self.requests
    }

    /// Blocking wait for the next forwarded argv (for a supervisor thread).
    pub fn recv(&self) -> Option<Vec<String>> {
        self.requests.recv().ok()
    }

    /// Set when the lock was taken but later launches cannot be received.
    pub fn degraded_reason(&self) -> Option<&str> {
        self.degraded.as_deref()
    }

    /// The forwarding socket path.
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.try_send(());
        }
        // Wake the accept loop if it is blocked in `accept()`.
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn serve(listener: UnixListener, requests: Sender<Vec<String>>, shutdown: Receiver<()>) {
    for stream in listener.incoming() {
        if shutdown.try_recv().is_ok() {
            return;
        }
        let Ok(mut stream) = stream else { continue };
        let mut args = Vec::new();
        // Args are framed `len\nbytes`; the writer shuts down its send half.
        let mut reader = BufReader::new(&stream);
        let mut payload = Vec::new();
        if reader.read_to_end(&mut payload).is_ok() {
            args.extend(decode_argv(&payload));
        }
        let _ = stream.flush();
        if !args.is_empty() {
            // A full channel means the UI is not draining; dropping the oldest
            // impossible case is better than blocking the accept loop.
            let _ = requests.try_send(args);
        }
        if shutdown.try_recv().is_ok() {
            return;
        }
    }
}

fn forward(path: &PathBuf) -> MlResult<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    forward_argv(path, &argv)
}

/// Send `argv` to the primary instance and close the send half.
fn forward_argv(path: &PathBuf, argv: &[String]) -> MlResult<()> {
    let mut stream = UnixStream::connect(path).map_err(|error| {
        MlError::Io(format!(
            "another instance runs but would not accept argv at {}: {error}",
            path.display()
        ))
    })?;
    stream
        .write_all(&encode_argv(argv))
        .and_then(|()| stream.shutdown(std::net::Shutdown::Write))
        .map_err(|error| MlError::Io(format!("forward argv: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_round_trips_including_hostile_payloads() {
        let cases: Vec<Vec<&str>> = vec![
            vec![],
            vec!["metrolist://play?v=abc"],
            vec!["", "a", ""],
            vec!["line\nbreak", "0\n", "3"],
            vec!["ünïcode ✓", "quote\"'", "\\backslash"],
            vec!["999999999999999999999999"],
            vec!["5"],
        ];
        for argv in cases {
            let encoded = encode_argv(&argv);
            let decoded = decode_argv(&encoded);
            assert_eq!(decoded, argv, "round-trip failed for {argv:?}");
        }
    }

    #[test]
    fn decode_stops_at_malformed_input_without_panicking() {
        // Truncated body.
        assert_eq!(decode_argv(b"10\nshort"), Vec::<String>::new());
        // Length is not a number.
        assert_eq!(decode_argv(b"abc\ndata"), Vec::<String>::new());
        // Declared length beyond the cap.
        assert_eq!(decode_argv(b"99999999\nx"), Vec::<String>::new());
        // No newline at all.
        assert_eq!(decode_argv(b"123"), Vec::<String>::new());
        // Valid prefix, then garbage: keep the prefix.
        let mut bytes = encode_argv(["kept"]);
        bytes.extend_from_slice(b"nope");
        assert_eq!(decode_argv(&bytes), vec!["kept".to_string()]);
        // Invalid UTF-8 in the body.
        assert_eq!(decode_argv(b"2\n\xff\xfe"), Vec::<String>::new());
        // Empty input.
        assert_eq!(decode_argv(b""), Vec::<String>::new());
    }

    #[test]
    fn socket_path_is_per_user_and_app_scoped() {
        let path = socket_path("org.metrolist.Metrolist").expect("test runs with XDG_RUNTIME_DIR");
        assert!(path.starts_with(std::env::var_os("XDG_RUNTIME_DIR").unwrap()));
        let name = path.file_name().unwrap().to_string_lossy();
        assert_eq!(name, "org.metrolist.Metrolist.sock");
        assert_ne!(
            socket_path("a").unwrap(),
            socket_path("b").unwrap(),
            "app ids must not share a socket"
        );
    }

    /// End-to-end on a real socket: primary receives the second process's argv.
    #[test]
    fn forwarded_argv_reaches_the_primary() {
        let app_id = format!("ml-integrations-test-{}", std::process::id());
        let guard = InstanceGuard::acquire(&app_id)
            .expect("acquire")
            .expect("first acquire must win the lock");
        assert!(
            guard.degraded_reason().is_none(),
            "{:?}",
            guard.degraded_reason()
        );

        let path = guard.socket_path().clone();
        let argv = vec![
            "metrolist://play?v=dQw4w9WgXcQ".to_string(),
            "second\narg".to_string(),
        ];
        forward_argv(&path, &argv).expect("forward");

        let received = guard
            .requests()
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("primary must receive forwarded argv");
        assert_eq!(received, argv);

        // A second guard on the same id must not become primary.
        assert!(InstanceGuard::acquire(&app_id)
            .expect("second acquire")
            .is_none());

        drop(guard);
        assert!(!path.exists(), "socket is removed on drop");
        // Lock is released: a fresh acquire wins again.
        assert!(InstanceGuard::acquire(&app_id)
            .expect("reacquire")
            .is_some());
    }
}
