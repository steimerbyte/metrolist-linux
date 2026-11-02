//! `ml-audio` — mpv-backed playback engine.
//!
//! mpv runs as a child process driven over its JSON IPC socket
//! (`--input-ipc-server`). A dedicated thread owns the socket, coalesces
//! property updates into ~10 Hz [`AudioEvent`]s, and never blocks the UI
//! thread. [`PlaybackController`] sits on top and owns the [`ml_core::Queue`],
//! auto-advancing on [`AudioEvent::EndFile`] and asking the caller to resolve
//! stream URLs asynchronously.
//!
//! Why mpv and not a pure-Rust decoder: YouTube Music's primary stream is
//! Opus/WebM and symphonia has no Opus decoder (see `arch/audio-notes.md`).
//! mpv also provides HTTP range seeks, native PipeWire device selection,
//! gapless playlist playback, pitch-corrected speed and lavfi EQ — all
//! verified on the target host.

use std::path::PathBuf;

use ml_core::PlayerSnapshot;

mod controller;
mod devices;
mod engine;
mod eq;
mod ipc;
mod normalize;

pub use controller::{
    sleep_window_contains, ControllerEvent, PlaybackController, QueueView, SleepTimer,
    StreamResolver,
};
pub use devices::AudioDevice;
pub use engine::{list_devices, sweep_stale_sockets, sweep_stale_sockets_in, MpvEngine};
pub use eq::{build_af, EQ_BANDS_HZ};
pub use normalize::normalization_gain_db;

/// Commands accepted by the engine thread.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioCommand {
    /// Replace the playlist and start at `index` (the selected URL is loaded;
    /// queue order stays owned by the caller).
    Load {
        urls: Vec<String>,
        index: usize,
    },
    /// Load a single URL, replacing the current playlist. `start_ms` seeks
    /// once the file has loaded.
    LoadUrl {
        url: String,
        start_ms: Option<u64>,
    },
    /// Append `url` to the mpv playlist so it starts gaplessly after the
    /// current track (used for prefetching).
    EnqueueNext {
        url: String,
    },
    Play,
    Pause,
    TogglePause,
    Stop,
    SeekAbsolute(u64),
    SeekRelative(i64),
    /// Linear volume, `0.0..=1.0`.
    SetVolume(f32),
    SetMuted(bool),
    SetSpeed(f32),
    /// Apply [`AudioCommand::SetGainDb`] (fabricated loudness is never used;
    /// the caller supplies the gain computed from track metadata).
    SetNormalization(bool),
    /// ReplayGain-style correction in dB, applied through the `af` chain.
    SetGainDb(f32),
    /// Drop mid-stream silence longer than 0.5 s below -50 dBFS via the
    /// `silenceremove` lavfi filter (see [`build_af`]).
    SetSkipSilence(bool),
    /// 10-band graphic EQ gains in dB; bands are [`EQ_BANDS_HZ`].
    SetEqBands(Vec<f32>),
    /// mpv audio device id (e.g. `pipewire/<node>`), empty selects `auto`.
    SetDevice(String),
    /// Asynchronously enumerate output devices (`mpv --audio-device=help`).
    ListDevices,
    /// Play next in the mpv playlist (queue order is owned by `ml-core`).
    Next,
    Previous,
    Shutdown,
}

/// Decoded audio stream parameters reported by mpv.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioParams {
    pub format: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

/// Track title/artist embedded in or broadcast with the stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
}

/// Events published by the engine thread.
// Snapshot dominates the enum size; events are coalesced to ~10 Hz, so a box
// would cost an allocation per event and change the frozen `Snapshot` shape.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    /// State change (not every position tick).
    Snapshot(PlayerSnapshot),
    /// Coalesced position update (~10 Hz).
    Position {
        position_ms: u64,
        duration_ms: u64,
    },
    Buffering(bool),
    /// mpv reached the end of the current file; the queue decides what plays next.
    EndFile,
    Error(String),
    /// Output devices, answering [`AudioCommand::ListDevices`].
    Devices(Vec<AudioDevice>),
    /// Current mpv `path` property changed (empty paths are not reported).
    Path(String),
    /// Stream format changed.
    AudioParams(AudioParams),
    /// Embedded/broadcast metadata changed.
    Metadata(TrackMetadata),
    /// The selected output device changed. `fallback_from` is set when the
    /// requested device was not in `mpv --audio-device=help`, so playback fell
    /// back to the mpv default; `id` is then `"auto"`.
    DeviceChanged {
        id: String,
        fallback_from: Option<String>,
    },
}

/// Engine process configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioConfig {
    /// Path to the mpv binary; `None` resolves `$APPDIR/usr/bin/mpv`, then
    /// `mpv` on `PATH`.
    pub mpv_binary: Option<PathBuf>,
    /// Extra mpv CLI arguments (device, cache, audio-exclusive, …). An
    /// explicit `--ao=…` disables the built-in pipewire/pulse/alsa fallback.
    pub extra_args: Vec<String>,
    /// Initial mpv audio device id (e.g. `pipewire/<node>`); persisted by the
    /// caller and validated by [`MpvEngine::spawn`]. `None`/`"auto"` uses the
    /// server default. See [`MpvEngine::device`].
    pub device: Option<String>,
    /// Mute engine output. `None` (the app default) follows the
    /// `METROLIST_TEST_MUTE` environment variable (unset = unmuted);
    /// `Some(true)` forces silent test playback and `Some(false)` forces
    /// unmuted. Test mode also uses the legacy `Metrolist Desktop` client id
    /// (muted by the host's stream-restore entry) and always passes
    /// `--mute=yes`, so playback tests cannot be audible. Normal playback uses
    /// `Metrolist Desktop Playback` with an explicit `--mute=no`, which
    /// bypasses the stale per-app mute entry.
    pub muted: Option<bool>,
}
