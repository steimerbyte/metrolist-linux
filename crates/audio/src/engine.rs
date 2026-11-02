//! mpv subprocess + JSON IPC engine thread.
//!
//! One thread owns the write half of the Unix socket while a reader thread
//! parses newline-delimited JSON. Property observations are coalesced in
//! [`Ipc::tick`] so the UI sees at most ~10 position events per second.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender};
use ml_core::{MlError, MlResult, PlaybackStatus, PlayerSnapshot, RepeatMode};
use serde_json::{json, Value};

use crate::devices::{self, AudioDevice};
use crate::eq::{self, EQ_BANDS_HZ};
use crate::ipc::{self, MpvMessage};
use crate::{AudioCommand, AudioConfig, AudioEvent, AudioParams, TrackMetadata};

const OBSERVED_PROPERTIES: &[&str] = &[
    "time-pos",
    "duration",
    "pause",
    "volume",
    "mute",
    "idle-active",
    "eof-reached",
    "audio-params",
    "metadata",
    "playlist",
    "path",
    "paused-for-cache",
];

/// Position/state coalescing interval (~10 Hz).
const TICK: Duration = Duration::from_millis(100);
const SOCKET_WAIT: Duration = Duration::from_secs(5);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);
const DEMUXER_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Normal playback client id. Deliberately distinct from the legacy
/// `Metrolist Desktop` id, whose PipeWire stream-restore entry is muted on some
/// hosts; an explicit `--mute=no` plus a fresh id keeps normal playback
/// audible.
const AUDIO_CLIENT_NAME: &str = "Metrolist Desktop Playback";
/// Test/probe client id. The host's per-app mute keeps it silent, and
/// `--mute=yes` makes the silence independent of that stored state.
const TEST_AUDIO_CLIENT_NAME: &str = "Metrolist Desktop";

/// Resolve the mpv binary: explicit config, bundled AppImage copy, then PATH.
fn resolve_binary(config: &AudioConfig) -> MlResult<PathBuf> {
    if let Some(path) = config.mpv_binary.as_ref() {
        if path.is_file() {
            return Ok(path.clone());
        }
        return Err(MlError::Playback(format!(
            "mpv binary not found: {}",
            path.display()
        )));
    }
    if let Some(appdir) = std::env::var_os("APPDIR") {
        let bundled = PathBuf::from(appdir).join("usr").join("bin").join("mpv");
        if bundled.is_file() {
            return Ok(bundled);
        }
    }
    Ok(PathBuf::from("mpv"))
}

/// Output devices reported by `mpv --audio-device=help`.
pub fn list_devices(config: &AudioConfig) -> MlResult<Vec<AudioDevice>> {
    list_devices_with_binary(&resolve_binary(config)?)
}

fn list_devices_with_binary(binary: &Path) -> MlResult<Vec<AudioDevice>> {
    let output = Command::new(binary)
        .args(["--no-config", "--audio-device=help"])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| {
            MlError::Playback(format!("failed to run {}: {error}", binary.display()))
        })?;
    if !output.status.success() {
        return Err(MlError::Playback(format!(
            "{} --audio-device=help exited ({})",
            binary.display(),
            output.status
        )));
    }
    let devices = devices::parse_devices(&String::from_utf8_lossy(&output.stdout));
    if devices.is_empty() {
        return Err(MlError::Playback(format!(
            "mpv reported no audio devices: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(devices)
}

/// Validate a persisted device against the current device list. An unknown or
/// removed device is dropped (mpv's automatic device is used); when the list
/// cannot be read the caller's id is kept, because mpv itself falls back to the
/// server default for unknown ids.
fn initial_device(binary: &Path, requested: Option<&str>) -> Option<String> {
    let requested = requested
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != "auto")?;
    match list_devices_with_binary(binary) {
        Ok(devices) if devices.iter().any(|device| device.id == requested) => {
            Some(requested.to_string())
        }
        Ok(_) => None,
        Err(_) => Some(requested.to_string()),
    }
}

/// Silent test playback when the config does not say otherwise. `0`, empty
/// and unset all mean unmuted; this is only for probes/integration tests.
fn env_test_mute() -> bool {
    std::env::var_os("METROLIST_TEST_MUTE")
        .and_then(|value| value.to_str().map(str::to_string))
        .is_some_and(|value| !value.is_empty() && value != "0")
}

/// Resolve the effective mute flag: explicit config wins, then the test env.
pub(crate) fn resolve_muted(config: &AudioConfig) -> bool {
    config.muted.unwrap_or_else(env_test_mute)
}

fn base_args(socket_path: &Path, muted: bool) -> Vec<String> {
    let client_name = if muted {
        TEST_AUDIO_CLIENT_NAME
    } else {
        AUDIO_CLIENT_NAME
    };
    vec![
        "--no-config".to_string(),
        "--idle=yes".to_string(),
        "--no-video".to_string(),
        "--audio-display=no".to_string(),
        format!("--input-ipc-server={}", socket_path.display()),
        "--msg-level=all=warn".to_string(),
        "--gapless-audio=yes".to_string(),
        "--prefetch-playlist=yes".to_string(),
        "--cache=yes".to_string(),
        format!("--demuxer-max-bytes={DEMUXER_MAX_BYTES}"),
        format!("--audio-client-name={client_name}"),
        // Explicit in both modes: the stream-restore per-app mute must not
        // decide whether playback is audible.
        if muted {
            "--mute=yes".to_string()
        } else {
            "--mute=no".to_string()
        },
        "--keep-open=no".to_string(),
        // scaletempo2, so `SetSpeed` keeps pitch (verified on mpv 0.41:
        // `get_property audio-pitch-correction` is true).
        "--audio-pitch-correction=yes".to_string(),
    ]
}

/// Speed range accepted by [`AudioCommand::SetSpeed`]; non-finite input means 1x.
pub(crate) fn clamp_speed(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(0.25, 4.0)
    } else {
        1.0
    }
}

/// Audio outputs tried in order when the process fails to come up. An
/// explicit `--ao` in `extra_args` disables the fallback.
fn ao_candidates(config: &AudioConfig) -> Vec<Option<&'static str>> {
    let overridden = config
        .extra_args
        .iter()
        .any(|arg| arg == "--ao" || arg.starts_with("--ao="));
    if overridden {
        vec![None]
    } else {
        vec![Some("pipewire"), Some("pulse"), Some("alsa")]
    }
}

/// Engine IPC socket names: `metrolist-audio-<pid>-<nanos>.sock`.
const SOCKET_PREFIX: &str = "metrolist-audio-";
const SOCKET_SUFFIX: &str = ".sock";

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn unique_socket_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    runtime_dir().join(format!(
        "{SOCKET_PREFIX}{}-{nanos}{SOCKET_SUFFIX}",
        std::process::id()
    ))
}

/// Owning pid of an engine socket file name, or `None` for anything else.
///
/// The pattern is exact: `metrolist-audio-<digits>-<digits>.sock`. The pid is
/// the app process that created the engine, not the mpv child, so a dead pid
/// proves the socket is stale (the app unlinks it while still running).
fn socket_owner_pid(name: &str) -> Option<u32> {
    let rest = name
        .strip_prefix(SOCKET_PREFIX)?
        .strip_suffix(SOCKET_SUFFIX)?;
    let (pid, nanos) = rest.split_once('-')?;
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(pid) || !digits(nanos) {
        return None;
    }
    pid.parse().ok()
}

fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // `kill(pid, 0)` probes existence; EPERM means it exists but is not ours.
    unsafe {
        libc::kill(pid, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

/// Remove engine sockets left behind by a dead app process. Sockets whose
/// owner is alive (a second instance, or this one) are never touched.
pub fn sweep_stale_sockets_in(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let own_uid = unsafe { libc::geteuid() };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(pid) = socket_owner_pid(name) else {
            continue;
        };
        if pid_alive(pid) {
            continue;
        }
        let path = entry.path();
        // `symlink_metadata`: a symlink is not a socket, and following one
        // could delete a file outside the runtime dir.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_socket() || meta.uid() != own_uid {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Sweep the default runtime directory (`$XDG_RUNTIME_DIR`, usually
/// `/run/user/<uid>`).
pub fn sweep_stale_sockets() -> usize {
    sweep_stale_sockets_in(&runtime_dir())
}

/// Ask the kernel to kill the mpv child when this process dies, including on
/// SIGKILL. The parent re-check closes the window where the parent died
/// between `fork` and `prctl`.
#[cfg(target_os = "linux")]
fn die_with_parent(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    let parent = std::process::id() as libc::pid_t;
    // SAFETY: the closure only calls async-signal-safe libc functions.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::_exit(0);
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn die_with_parent(_command: &mut Command) {}

fn wait_for_socket(socket_path: &Path, child: &mut Child) -> MlResult<UnixStream> {
    let deadline = Instant::now() + SOCKET_WAIT;
    loop {
        if socket_path.exists() {
            match UnixStream::connect(socket_path) {
                Ok(stream) => return Ok(stream),
                Err(error) if Instant::now() >= deadline => {
                    return Err(MlError::Playback(format!("mpv IPC connect: {error}")));
                }
                Err(_) => {}
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(MlError::Playback(format!(
                "mpv exited before opening its IPC socket ({status})"
            )));
        }
        if Instant::now() >= deadline {
            return Err(MlError::Playback(format!(
                "timed out waiting for mpv IPC socket {}",
                socket_path.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// mpv child shared by the engine thread (which drives it) and
/// [`MpvEngine::shutdown`], so a wedged thread cannot keep the child alive.
type ChildSlot = Arc<Mutex<Option<Child>>>;

/// Handle to the mpv engine thread.
pub struct MpvEngine {
    command_tx: Sender<AudioCommand>,
    event_rx: Receiver<AudioEvent>,
    thread: Option<JoinHandle<()>>,
    binary: PathBuf,
    /// Last successfully applied device, shared with the IPC thread. `None`
    /// means mpv's automatic device.
    device: Arc<Mutex<Option<String>>>,
    child: ChildSlot,
    pid: u32,
    socket_path: PathBuf,
}

impl MpvEngine {
    /// Start mpv and the IPC reader.
    pub fn spawn(config: AudioConfig) -> MlResult<Self> {
        let binary = resolve_binary(&config)?;
        let socket_path = unique_socket_path();
        let _ = std::fs::remove_file(&socket_path);
        let device = initial_device(&binary, config.device.as_deref());
        let muted = resolve_muted(&config);

        let mut last_error = None;
        for ao in ao_candidates(&config) {
            let mut command = Command::new(&binary);
            command.args(base_args(&socket_path, muted));
            if let Some(ao) = ao {
                command.arg(format!("--ao={ao}"));
            }
            if let Some(device) = &device {
                command.arg(format!("--audio-device={device}"));
            }
            command.args(&config.extra_args);
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            die_with_parent(&mut command);

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    last_error = Some(MlError::Playback(format!(
                        "failed to start {}: {error}",
                        binary.display()
                    )));
                    continue;
                }
            };
            match wait_for_socket(&socket_path, &mut child) {
                Ok(stream) => {
                    return Self::start(stream, child, socket_path, binary, device.clone());
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(&socket_path);
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| MlError::Playback("no usable audio output".to_string())))
    }

    fn start(
        stream: UnixStream,
        child: Child,
        socket_path: PathBuf,
        binary: PathBuf,
        device: Option<String>,
    ) -> MlResult<Self> {
        let reader_stream = match stream.try_clone() {
            Ok(stream) => stream,
            Err(error) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&socket_path);
                return Err(MlError::Playback(format!(
                    "failed to clone mpv IPC socket: {error}"
                )));
            }
        };
        let pid = child.id();
        let child: ChildSlot = Arc::new(Mutex::new(Some(child)));
        let thread_child = child.clone();
        let thread_socket = socket_path.clone();
        let (command_tx, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let engine_binary = binary.clone();
        let shared_device = Arc::new(Mutex::new(device.clone()));
        let thread_device = shared_device.clone();
        let thread = std::thread::spawn(move || {
            let (reader_tx, reader_rx) = crossbeam_channel::unbounded();
            let reader = std::thread::Builder::new()
                .name("ml-audio-ipc".to_string())
                .spawn(move || read_loop(reader_stream, reader_tx));
            let mut ipc = Ipc::new(
                stream,
                thread_child,
                thread_socket,
                engine_binary,
                device,
                thread_device,
            );
            match reader {
                Ok(reader) => {
                    ipc.observe_all(&event_tx);
                    run_loop(&mut ipc, command_rx, reader_rx, &event_tx);
                    ipc.teardown();
                    let _ = reader.join();
                }
                Err(error) => {
                    let _ = event_tx.send(AudioEvent::Error(format!(
                        "failed to start mpv IPC reader: {error}"
                    )));
                    ipc.kill();
                }
            }
        });
        Ok(Self {
            command_tx,
            event_rx,
            thread: Some(thread),
            binary,
            device: shared_device,
            child,
            pid,
            socket_path,
        })
    }

    pub fn send(&self, command: AudioCommand) -> MlResult<()> {
        self.command_tx
            .send(command)
            .map_err(|_| MlError::Playback("audio engine stopped".to_string()))
    }

    /// Non-blocking event drain point for the UI thread.
    pub fn try_recv(&self) -> Option<AudioEvent> {
        self.event_rx.try_recv().ok()
    }

    pub fn event_receiver(&self) -> &Receiver<AudioEvent> {
        &self.event_rx
    }

    pub fn command_sender(&self) -> &Sender<AudioCommand> {
        &self.command_tx
    }

    /// Last applied output device; `None` is mpv's automatic device. Use this
    /// with [`AudioConfig::device`] to persist the selection.
    pub fn device(&self) -> Option<String> {
        self.device.lock().ok().and_then(|guard| guard.clone())
    }

    /// Switch the output device. An unknown id falls back to `auto` and is
    /// reported as [`AudioEvent::DeviceChanged`] with `fallback_from` set.
    /// Non-blocking: the switch happens on the engine thread.
    pub fn set_device(&self, device: Option<&str>) -> MlResult<()> {
        self.send(AudioCommand::SetDevice(
            device.unwrap_or_default().to_string(),
        ))
    }

    /// Enumerate output devices with the same mpv binary used for playback.
    /// This blocks for a few tens of milliseconds; prefer
    /// [`AudioCommand::ListDevices`] on a UI thread.
    pub fn list_devices(&self) -> MlResult<Vec<AudioDevice>> {
        list_devices_with_binary(&self.binary)
    }

    /// Process id of the mpv child this engine started.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// IPC socket path mpv opened for this engine.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Stop mpv and the IPC thread, then remove the IPC socket.
    ///
    /// Idempotent: [`Drop`] calls it again. The child is killed directly (not
    /// only through the engine thread) so a wedged thread cannot keep mpv
    /// alive during app shutdown.
    pub fn shutdown(&mut self) {
        let _ = self.command_tx.send(AudioCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            // The engine thread reaps mpv itself (up to `SHUTDOWN_WAIT`); wait
            // a little longer, then take over instead of hanging on join.
            let deadline = Instant::now() + SHUTDOWN_WAIT + Duration::from_millis(500);
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
        let child = self.child.lock().ok().and_then(|mut slot| slot.take());
        if let Some(mut child) = child {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for MpvEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

enum ReaderMessage {
    Line(String),
    Closed,
}

fn read_loop(stream: UnixStream, tx: Sender<ReaderMessage>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    continue;
                }
                if tx.send(ReaderMessage::Line(trimmed.to_string())).is_err() {
                    return;
                }
            }
            Err(_) => break,
        }
    }
    let _ = tx.send(ReaderMessage::Closed);
}

fn run_loop(
    ipc: &mut Ipc,
    command_rx: Receiver<AudioCommand>,
    reader_rx: Receiver<ReaderMessage>,
    event_tx: &Sender<AudioEvent>,
) {
    let tick = crossbeam_channel::tick(TICK);
    loop {
        crossbeam_channel::select! {
            recv(command_rx) -> command => match command {
                Ok(command) => {
                    if !ipc.handle_command(command, event_tx) {
                        break;
                    }
                }
                Err(_) => break,
            },
            recv(reader_rx) -> message => match message {
                Ok(ReaderMessage::Line(line)) => match ipc::parse_line(&line) {
                    Ok(message) => {
                        if !ipc.handle_message(message, event_tx) {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = event_tx.send(AudioEvent::Error(error.to_string()));
                    }
                },
                Ok(ReaderMessage::Closed) | Err(_) => break,
            },
            recv(tick) -> _ => {
                if !ipc.tick(event_tx) {
                    break;
                }
            },
        }
    }
}

#[derive(Debug)]
struct EngineState {
    idle: bool,
    paused: bool,
    buffering: bool,
    loading: bool,
    volume: f32,
    muted: bool,
    position_ms: u64,
    duration_ms: u64,
    eof: bool,
    path: Option<String>,
    eq_bands: Vec<f32>,
    gain_db: f32,
    normalization: bool,
    skip_silence: bool,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            idle: true,
            paused: false,
            buffering: false,
            loading: false,
            volume: 1.0,
            muted: false,
            position_ms: 0,
            duration_ms: 0,
            eof: false,
            path: None,
            eq_bands: vec![0.0; EQ_BANDS_HZ.len()],
            gain_db: 0.0,
            normalization: true,
            skip_silence: false,
        }
    }
}

impl EngineState {
    fn status(&self) -> PlaybackStatus {
        if self.idle && self.path.is_none() {
            PlaybackStatus::Idle
        } else if self.eof {
            PlaybackStatus::Ended
        } else if self.paused {
            PlaybackStatus::Paused
        } else if self.loading && !self.buffering {
            PlaybackStatus::Loading
        } else if self.buffering {
            PlaybackStatus::Buffering
        } else if self.idle {
            PlaybackStatus::Stopped
        } else {
            PlaybackStatus::Playing
        }
    }

    fn snapshot(&self) -> PlayerSnapshot {
        PlayerSnapshot {
            status: self.status(),
            position_ms: self.position_ms,
            duration_ms: self.duration_ms,
            volume: self.volume,
            muted: self.muted,
            shuffle: false,
            repeat: RepeatMode::Off,
            current: None,
            queue_len: 0,
            queue_index: None,
            buffering: self.buffering,
        }
    }
}

struct Ipc {
    stream: UnixStream,
    child: ChildSlot,
    socket_path: PathBuf,
    binary: PathBuf,
    next_id: u64,
    pending: HashMap<u64, String>,
    device: Option<String>,
    shared_device: Arc<Mutex<Option<String>>>,
    state: EngineState,
    applied_af: Option<String>,
    latest_pos: Option<u64>,
    last_emitted_pos: u64,
    end_emitted: bool,
    pending_start_ms: Option<u64>,
    last_metadata: TrackMetadata,
    last_audio_params: AudioParams,
    dirty: bool,
    broken: bool,
}

impl Ipc {
    fn new(
        stream: UnixStream,
        child: ChildSlot,
        socket_path: PathBuf,
        binary: PathBuf,
        device: Option<String>,
        shared_device: Arc<Mutex<Option<String>>>,
    ) -> Self {
        Self {
            stream,
            child,
            socket_path,
            binary,
            next_id: 0,
            pending: HashMap::new(),
            device,
            shared_device,
            state: EngineState::default(),
            applied_af: None,
            latest_pos: None,
            last_emitted_pos: u64::MAX,
            end_emitted: false,
            pending_start_ms: None,
            last_metadata: TrackMetadata::default(),
            last_audio_params: AudioParams::default(),
            dirty: true,
            broken: false,
        }
    }

    fn observe_all(&mut self, event_tx: &Sender<AudioEvent>) {
        for (index, property) in OBSERVED_PROPERTIES.iter().enumerate() {
            self.request(
                vec![
                    json!("observe_property"),
                    json!(index as u64 + 1),
                    json!(property),
                ],
                event_tx,
            );
        }
    }

    fn request(&mut self, command: Vec<Value>, event_tx: &Sender<AudioEvent>) -> Option<u64> {
        self.next_id = self.next_id.wrapping_add(1);
        let request_id = self.next_id;
        let description = command
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let line = ipc::request(request_id, command);
        if let Err(error) = self
            .stream
            .write_all(line.as_bytes())
            .and_then(|()| self.stream.flush())
        {
            self.broken = true;
            let _ = event_tx.send(AudioEvent::Error(format!("mpv IPC write failed: {error}")));
            return None;
        }
        self.pending.insert(request_id, description);
        Some(request_id)
    }

    fn start_file(&mut self, url: String, start_ms: Option<u64>, event_tx: &Sender<AudioEvent>) {
        self.request(vec![json!("playlist-clear")], event_tx);
        self.request(
            vec![json!("loadfile"), json!(url), json!("replace")],
            event_tx,
        );
        self.pending_start_ms = start_ms.filter(|ms| *ms > 0);
        self.end_emitted = false;
        self.state.eof = false;
        self.state.loading = true;
        self.state.position_ms = 0;
        self.state.duration_ms = 0;
        self.latest_pos = Some(0);
        self.dirty = true;
    }

    fn handle_command(&mut self, command: AudioCommand, event_tx: &Sender<AudioEvent>) -> bool {
        match command {
            AudioCommand::Load { urls, index } => match urls.get(index) {
                Some(url) => self.start_file(url.clone(), None, event_tx),
                None => {
                    let _ = event_tx.send(AudioEvent::Error(format!(
                        "queue index {index} out of range ({} urls)",
                        urls.len()
                    )));
                }
            },
            AudioCommand::LoadUrl { url, start_ms } => {
                self.start_file(url, start_ms, event_tx);
            }
            AudioCommand::EnqueueNext { url } => {
                self.request(
                    vec![json!("loadfile"), json!(url), json!("append")],
                    event_tx,
                );
            }
            AudioCommand::Play => {
                self.state.paused = false;
                self.request(
                    vec![json!("set_property"), json!("pause"), json!(false)],
                    event_tx,
                );
                self.dirty = true;
            }
            AudioCommand::Pause => {
                self.state.paused = true;
                self.request(
                    vec![json!("set_property"), json!("pause"), json!(true)],
                    event_tx,
                );
                self.dirty = true;
            }
            AudioCommand::TogglePause => {
                self.state.paused = !self.state.paused;
                self.request(vec![json!("cycle"), json!("pause")], event_tx);
                self.dirty = true;
            }
            AudioCommand::Stop => {
                self.request(vec![json!("stop")], event_tx);
                self.end_emitted = true;
                self.pending_start_ms = None;
                self.state.eof = false;
                self.state.loading = false;
                self.state.position_ms = 0;
                self.state.duration_ms = 0;
                self.dirty = true;
            }
            AudioCommand::SeekAbsolute(ms) => {
                self.state.position_ms = ms;
                self.latest_pos = Some(ms);
                self.request(
                    vec![
                        json!("seek"),
                        json!(ms as f64 / 1000.0),
                        json!("absolute+exact"),
                    ],
                    event_tx,
                );
                self.dirty = true;
            }
            AudioCommand::SeekRelative(delta_ms) => {
                let target = self.state.position_ms.saturating_add_signed(delta_ms);
                self.state.position_ms = target;
                self.latest_pos = Some(target);
                self.request(
                    vec![
                        json!("seek"),
                        json!(delta_ms as f64 / 1000.0),
                        json!("relative+exact"),
                    ],
                    event_tx,
                );
                self.dirty = true;
            }
            AudioCommand::SetVolume(volume) => {
                let volume = volume.clamp(0.0, 1.0);
                self.state.volume = volume;
                self.request(
                    vec![
                        json!("set_property"),
                        json!("volume"),
                        json!(volume * 100.0),
                    ],
                    event_tx,
                );
                self.dirty = true;
            }
            AudioCommand::SetMuted(muted) => {
                self.state.muted = muted;
                self.request(
                    vec![json!("set_property"), json!("mute"), json!(muted)],
                    event_tx,
                );
                self.dirty = true;
            }
            AudioCommand::SetSpeed(speed) => {
                let speed = clamp_speed(speed);
                self.request(
                    vec![json!("set_property"), json!("speed"), json!(speed)],
                    event_tx,
                );
            }
            AudioCommand::SetNormalization(enabled) => {
                self.state.normalization = enabled;
                self.apply_af(event_tx);
                self.dirty = true;
            }
            AudioCommand::SetGainDb(gain_db) => {
                if gain_db.is_finite() {
                    self.state.gain_db = gain_db;
                }
                self.apply_af(event_tx);
                self.dirty = true;
            }
            AudioCommand::SetSkipSilence(enabled) => {
                if self.state.skip_silence != enabled {
                    self.state.skip_silence = enabled;
                    self.apply_af(event_tx);
                }
            }
            AudioCommand::SetEqBands(bands) => {
                self.state.eq_bands = bands;
                self.apply_af(event_tx);
            }
            AudioCommand::SetDevice(device) => self.set_device(device, event_tx),
            AudioCommand::ListDevices => match list_devices_with_binary(&self.binary) {
                Ok(devices) => {
                    let _ = event_tx.send(AudioEvent::Devices(devices));
                }
                Err(error) => {
                    let _ = event_tx.send(AudioEvent::Error(error.to_string()));
                }
            },
            AudioCommand::Next => {
                self.request(vec![json!("playlist-next"), json!("weak")], event_tx);
            }
            AudioCommand::Previous => {
                self.request(vec![json!("playlist-prev"), json!("weak")], event_tx);
            }
            AudioCommand::Shutdown => return false,
        }
        true
    }

    /// Apply a device id, validating it against mpv's device list first because
    /// mpv accepts unknown ids silently and falls back to the server default.
    fn set_device(&mut self, requested: String, event_tx: &Sender<AudioEvent>) {
        let requested = requested.trim();
        let (target, fallback_from) = if requested.is_empty() || requested == "auto" {
            ("auto".to_string(), None)
        } else {
            match list_devices_with_binary(&self.binary) {
                Ok(devices) if devices.iter().any(|device| device.id == requested) => {
                    (requested.to_string(), None)
                }
                Ok(_) => ("auto".to_string(), Some(requested.to_string())),
                // Cannot enumerate (mpv failed): let mpv decide; it falls back
                // to the server default for ids that no longer exist.
                Err(_) => (requested.to_string(), None),
            }
        };
        self.request(
            vec![json!("set_property"), json!("audio-device"), json!(target)],
            event_tx,
        );
        self.device = if target == "auto" {
            None
        } else {
            Some(target.clone())
        };
        if let Ok(mut guard) = self.shared_device.lock() {
            *guard = self.device.clone();
        }
        let _ = event_tx.send(AudioEvent::DeviceChanged {
            id: target,
            fallback_from,
        });
    }

    fn apply_af(&mut self, event_tx: &Sender<AudioEvent>) {
        let gain_db = if self.state.normalization {
            self.state.gain_db
        } else {
            0.0
        };
        let desired = eq::build_af(&self.state.eq_bands, gain_db, self.state.skip_silence);
        if desired == self.applied_af {
            return;
        }
        // `af` accepts an empty string to clear the chain.
        let value = desired.clone().unwrap_or_default();
        self.request(
            vec![json!("set_property"), json!("af"), json!(value)],
            event_tx,
        );
        self.applied_af = desired;
    }

    fn handle_message(&mut self, message: MpvMessage, event_tx: &Sender<AudioEvent>) -> bool {
        match message {
            MpvMessage::Response {
                request_id,
                error,
                data: _,
            } => {
                let command = self
                    .pending
                    .remove(&request_id)
                    .unwrap_or_else(|| format!("#{request_id}"));
                if error != "success" && error != "property unavailable" {
                    let _ = event_tx.send(AudioEvent::Error(format!("mpv {command}: {error}")));
                }
                true
            }
            MpvMessage::Event {
                event,
                name,
                data,
                reason,
                ..
            } => match event.as_str() {
                "property-change" => {
                    if let Some(name) = name {
                        self.handle_property_change(&name, data.as_ref(), event_tx);
                    }
                    true
                }
                "file-loaded" => {
                    self.end_emitted = false;
                    self.state.eof = false;
                    self.state.loading = false;
                    self.dirty = true;
                    if let Some(start_ms) = self.pending_start_ms.take() {
                        self.request(
                            vec![
                                json!("seek"),
                                json!(start_ms as f64 / 1000.0),
                                json!("absolute+exact"),
                            ],
                            event_tx,
                        );
                    }
                    true
                }
                "start-file" => {
                    self.state.loading = true;
                    self.dirty = true;
                    true
                }
                "playback-restart" => {
                    self.dirty = true;
                    true
                }
                "end-file" => {
                    match reason.as_deref() {
                        Some("eof") => self.emit_end_once(event_tx),
                        Some("error") => {
                            self.state.loading = false;
                            self.dirty = true;
                            let _ = event_tx.send(AudioEvent::Error(
                                "mpv could not play the current file".to_string(),
                            ));
                        }
                        _ => {
                            self.state.loading = false;
                            self.dirty = true;
                        }
                    }
                    true
                }
                "idle" => {
                    self.state.idle = true;
                    self.state.loading = false;
                    self.dirty = true;
                    true
                }
                "audio-reconfig" => {
                    self.dirty = true;
                    true
                }
                "shutdown" => false,
                _ => true,
            },
        }
    }

    fn handle_property_change(
        &mut self,
        name: &str,
        data: Option<&Value>,
        event_tx: &Sender<AudioEvent>,
    ) {
        match name {
            "time-pos" => {
                if let Some(seconds) = data.and_then(Value::as_f64) {
                    let position_ms = (seconds.max(0.0) * 1000.0).round() as u64;
                    if position_ms != self.state.position_ms {
                        self.state.position_ms = position_ms;
                        self.latest_pos = Some(position_ms);
                    }
                }
            }
            "duration" => {
                if let Some(seconds) = data.and_then(Value::as_f64) {
                    self.state.duration_ms = (seconds.max(0.0) * 1000.0).round() as u64;
                    self.dirty = true;
                }
            }
            "pause" => {
                if let Some(paused) = data.and_then(Value::as_bool) {
                    self.state.paused = paused;
                    self.dirty = true;
                }
            }
            "volume" => {
                if let Some(volume) = data.and_then(Value::as_f64) {
                    self.state.volume = (volume / 100.0).clamp(0.0, 1.0) as f32;
                    self.dirty = true;
                }
            }
            "mute" => {
                if let Some(muted) = data.and_then(Value::as_bool) {
                    self.state.muted = muted;
                    self.dirty = true;
                }
            }
            "idle-active" => {
                if let Some(idle) = data.and_then(Value::as_bool) {
                    self.state.idle = idle;
                    self.dirty = true;
                    if idle && self.state.eof {
                        self.emit_end_once(event_tx);
                    }
                }
            }
            "eof-reached" => {
                if let Some(eof) = data.and_then(Value::as_bool) {
                    self.state.eof = eof;
                    self.dirty = true;
                    if eof {
                        self.emit_end_once(event_tx);
                    }
                }
            }
            "audio-params" => {
                let params = parse_audio_params(data);
                if params != self.last_audio_params {
                    let _ = event_tx.send(AudioEvent::AudioParams(params.clone()));
                    self.last_audio_params = params;
                }
            }
            "metadata" => {
                let metadata = parse_metadata(data);
                if metadata != self.last_metadata {
                    let _ = event_tx.send(AudioEvent::Metadata(metadata.clone()));
                    self.last_metadata = metadata;
                }
            }
            "path" => {
                match data.and_then(Value::as_str) {
                    Some(path) if !path.is_empty() => {
                        if self.state.path.as_deref() != Some(path) {
                            self.state.path = Some(path.to_string());
                            let _ = event_tx.send(AudioEvent::Path(path.to_string()));
                        }
                    }
                    _ => self.state.path = None,
                }
                self.dirty = true;
            }
            "paused-for-cache" => {
                if let Some(buffering) = data.and_then(Value::as_bool) {
                    if self.state.buffering != buffering {
                        self.state.buffering = buffering;
                        let _ = event_tx.send(AudioEvent::Buffering(buffering));
                        self.dirty = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn emit_end_once(&mut self, event_tx: &Sender<AudioEvent>) {
        if self.end_emitted {
            return;
        }
        self.end_emitted = true;
        self.state.eof = true;
        self.state.loading = false;
        self.dirty = true;
        let _ = event_tx.send(AudioEvent::EndFile);
    }

    /// Emit coalesced position + state events; `false` means the loop should stop.
    fn tick(&mut self, event_tx: &Sender<AudioEvent>) -> bool {
        if self.broken {
            return false;
        }
        if let Some(position_ms) = self.latest_pos.take() {
            if position_ms != self.last_emitted_pos {
                self.last_emitted_pos = position_ms;
                let _ = event_tx.send(AudioEvent::Position {
                    position_ms,
                    duration_ms: self.state.duration_ms,
                });
            }
        }
        if self.dirty {
            self.dirty = false;
            let _ = event_tx.send(AudioEvent::Snapshot(self.state.snapshot()));
        }
        // `None` means `MpvEngine::shutdown` already took the child; the loop
        // then ends when the socket closes.
        let waited = self
            .child
            .lock()
            .map_err(|_| "mpv child handle poisoned".to_string())
            .and_then(|mut slot| match slot.as_mut() {
                Some(child) => child
                    .try_wait()
                    .map_err(|error| format!("mpv wait failed: {error}")),
                None => Ok(None),
            });
        match waited {
            Ok(Some(status)) => {
                let _ = event_tx.send(AudioEvent::Error(format!("mpv exited ({status})")));
                false
            }
            Ok(None) => true,
            Err(error) => {
                let _ = event_tx.send(AudioEvent::Error(error));
                false
            }
        }
    }

    fn teardown(&mut self) {
        let _ = self
            .stream
            .write_all(ipc::request(0, vec![json!("quit")]).as_bytes());
        let _ = self.stream.flush();
        if let Some(mut child) = self.take_child() {
            let deadline = Instant::now() + SHUTDOWN_WAIT;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }

    fn take_child(&self) -> Option<Child> {
        self.child.lock().ok().and_then(|mut slot| slot.take())
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.take_child() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn parse_audio_params(data: Option<&Value>) -> AudioParams {
    AudioParams {
        format: data
            .and_then(|value| value.get("format"))
            .and_then(Value::as_str)
            .map(str::to_string),
        sample_rate: data
            .and_then(|value| value.get("samplerate"))
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        channels: data
            .and_then(|value| value.get("channel-count"))
            .and_then(Value::as_u64)
            .map(|value| value as u32),
    }
}

fn parse_metadata(data: Option<&Value>) -> TrackMetadata {
    let object = data.and_then(Value::as_object);
    let find = |keys: &[&str]| {
        keys.iter().find_map(|key| {
            object
                .and_then(|object| object.get(*key))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
    };
    TrackMetadata {
        title: find(&["Title", "title", "icy-title"]),
        artist: find(&["Artist", "artist"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_is_clamped_to_mpv_supported_range() {
        assert_eq!(clamp_speed(1.0), 1.0);
        assert_eq!(clamp_speed(0.25), 0.25);
        assert_eq!(clamp_speed(4.0), 4.0);
        assert_eq!(clamp_speed(0.0), 0.25);
        assert_eq!(clamp_speed(10.0), 4.0);
        assert_eq!(clamp_speed(f32::NAN), 1.0);
        assert_eq!(clamp_speed(f32::INFINITY), 1.0);
    }

    #[test]
    fn mute_resolution_prefers_config_over_environment() {
        assert!(!resolve_muted(&AudioConfig {
            muted: Some(false),
            ..AudioConfig::default()
        }));
        assert!(resolve_muted(&AudioConfig {
            muted: Some(true),
            ..AudioConfig::default()
        }));
    }

    #[test]
    fn base_args_carry_an_explicit_mute_and_client_name() {
        let args = base_args(Path::new("/tmp/ml.sock"), false);
        assert!(args.contains(&"--mute=no".to_string()));
        assert!(args.contains(&format!("--audio-client-name={AUDIO_CLIENT_NAME}")));
        assert!(!args.iter().any(|arg| arg == "--mute=yes"));

        let args = base_args(Path::new("/tmp/ml.sock"), true);
        assert!(args.contains(&"--mute=yes".to_string()));
        assert!(args.contains(&format!("--audio-client-name={TEST_AUDIO_CLIENT_NAME}")));
        assert!(!args.iter().any(|arg| arg == "--mute=no"));
    }

    #[test]
    fn engine_state_starts_with_flat_filters_and_automatic_device() {
        let state = EngineState::default();
        assert_eq!(state.gain_db, 0.0);
        assert!(state.normalization);
        assert!(!state.skip_silence);
        assert_eq!(state.eq_bands, vec![0.0; EQ_BANDS_HZ.len()]);
        assert_eq!(
            eq::build_af(&state.eq_bands, state.gain_db, state.skip_silence),
            None
        );
    }

    #[test]
    fn socket_owner_pid_accepts_only_engine_socket_names() {
        assert_eq!(
            socket_owner_pid("metrolist-audio-1234-5678.sock"),
            Some(1234)
        );
        assert_eq!(socket_owner_pid("metrolist-audio-1-0.sock"), Some(1));
        assert_eq!(socket_owner_pid("metrolist-audio-1234.sock"), None);
        assert_eq!(socket_owner_pid("metrolist-audio--5678.sock"), None);
        assert_eq!(socket_owner_pid("metrolist-audio-1234-.sock"), None);
        assert_eq!(socket_owner_pid("metrolist-audio-1234-5678.socket"), None);
        assert_eq!(socket_owner_pid("metrolist-audio-1234-5678.sock.bak"), None);
        assert_eq!(socket_owner_pid("metrolist-audio-12a4-5678.sock"), None);
        assert_eq!(socket_owner_pid("metrolist-audio-1234-56-78.sock"), None);
        assert_eq!(socket_owner_pid("other-1234-5678.sock"), None);
        assert_eq!(socket_owner_pid("metrolist-audio-4294967296-1.sock"), None);
    }

    #[test]
    fn sweep_removes_dead_engine_sockets_only() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join(format!("ml-audio-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp sweep dir");
        // A live owner (this process) keeps its socket.
        let live_socket = dir.join(format!("metrolist-audio-{}-1.sock", std::process::id()));
        let live_listener = UnixListener::bind(&live_socket).expect("bind live socket");
        // 999999999 is above `pid_max`, so no process can own it.
        let dead_socket = dir.join("metrolist-audio-999999999-1.sock");
        drop(UnixListener::bind(&dead_socket).expect("bind dead socket"));
        let dead_file = dir.join("metrolist-audio-999999999-2.sock");
        std::fs::write(&dead_file, b"not a socket").expect("write plain file");
        let foreign_socket = dir.join("other-app-999999999-1.sock");
        drop(UnixListener::bind(&foreign_socket).expect("bind foreign socket"));

        assert_eq!(sweep_stale_sockets_in(&dir), 1);
        assert!(live_socket.exists(), "a live owner's socket must survive");
        assert!(!dead_socket.exists(), "a dead owner's socket must go");
        assert!(dead_file.exists(), "only sockets are swept");
        assert!(foreign_socket.exists(), "only engine sockets are swept");

        drop(live_listener);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
