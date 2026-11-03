//! `ml-ui` — eframe/egui shell for Metrolist Linux.
//!
//! The skeleton ships the Material 3 theme, custom widgets, responsive shell and
//! a mock Home/player. Backend crates are wired by later agents; see
//! `docs/ARCHITECTURE.md` for the frozen interfaces.

pub mod app;
pub mod art;
pub mod desktop;
pub mod integrations;
pub mod integrations_worker;
pub mod mock;
pub mod motion;
pub mod pure;
pub mod routes;
pub mod screens;
pub mod services;
pub mod settings_store;
pub mod shell;
pub mod signals;
pub mod state;
pub mod theme;
pub mod widgets;

use std::path::PathBuf;

pub use app::App;

/// How the process should terminate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    /// Normal interactive window.
    Normal,
    /// Open the window, render `frames` and exit 0 with `SMOKE_OK …`.
    Smoke { frames: u32 },
    /// Render, save one framebuffer PNG, then exit 0.
    Screenshot { path: PathBuf },
}

/// Deterministic startup state for player/queue/lyrics screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoMode {
    Player,
    PlayerPaused,
    Queue,
    Lyrics,
    Library,
    Settings,
    SettingsAudio,
    History,
    Liked,
    Downloads,
    Account,
    Charts,
    Moods,
    NewReleases,
    Podcasts,
    Stats,
    Wrapped,
    Recognize,
    RecognizeListening,
    RecognizeSuccess,
    RecognizeError,
    Integrations,
    IntegrationsConnected,
    IntegrationsError,
    Together,
    TogetherConnected,
    TogetherGuest,
}

impl DemoMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "player" => Some(Self::Player),
            "player-paused" => Some(Self::PlayerPaused),
            "queue" => Some(Self::Queue),
            "lyrics" => Some(Self::Lyrics),
            "library" => Some(Self::Library),
            "settings" => Some(Self::Settings),
            "settings-audio" => Some(Self::SettingsAudio),
            "history" => Some(Self::History),
            "liked" => Some(Self::Liked),
            "downloads" => Some(Self::Downloads),
            "account" => Some(Self::Account),
            "charts" => Some(Self::Charts),
            "moods" => Some(Self::Moods),
            "new-releases" => Some(Self::NewReleases),
            "podcasts" => Some(Self::Podcasts),
            "stats" => Some(Self::Stats),
            "wrapped" => Some(Self::Wrapped),
            "recognize" => Some(Self::Recognize),
            "recognize-listening" => Some(Self::RecognizeListening),
            "recognize-success" => Some(Self::RecognizeSuccess),
            "recognize-error" => Some(Self::RecognizeError),
            "integrations" => Some(Self::Integrations),
            "integrations-connected" => Some(Self::IntegrationsConnected),
            "integrations-error" => Some(Self::IntegrationsError),
            "together" => Some(Self::Together),
            "together-connected" => Some(Self::TogetherConnected),
            "together-guest" => Some(Self::TogetherGuest),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub mode: RunMode,
    pub width: u32,
    pub height: u32,
    /// Play a sample track and open a screen before capturing.
    pub demo: Option<DemoMode>,
    /// Screenshot-only synthetic pointer position (egui points). Used by the
    /// docs tooling; Wayland denies real hover control to external tools.
    pub pointer: Option<(f32, f32)>,
    /// Screenshot-only synthetic click at `pointer`, injected around frame 30.
    pub click: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            mode: RunMode::Normal,
            width: app::DEFAULT_WIDTH,
            height: app::DEFAULT_HEIGHT,
            demo: None,
            pointer: None,
            click: false,
        }
    }
}

/// Start eframe. Returns when the viewport closes.
pub fn run(options: RunOptions) -> eframe::Result {
    // Stale engine sockets from a killed instance are files, not leaks; sweep
    // them before a new engine picks a name.
    let swept = ml_audio::sweep_stale_sockets();
    if swept > 0 {
        eprintln!("audio: removed {swept} stale IPC socket(s)");
    }
    let width = options.width.max(320) as f32;
    let height = options.height.max(480) as f32;
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([width, height])
        .with_title("Metrolist");
    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    // Batch modes are tools: they may run next to an interactive instance, and
    // the single-instance guard would otherwise forward their argv and exit.
    let instance = if matches!(options.mode, RunMode::Normal) {
        match desktop::take_instance() {
            Ok(Some(guard)) => Some(guard),
            Ok(None) => {
                println!("forwarded to the running Metrolist instance");
                return Ok(());
            }
            Err(error) => {
                eprintln!("single instance unavailable: {error}");
                None
            }
        }
    } else {
        None
    };
    eframe::run_native(
        "metrolist",
        native_options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, options, instance)))),
    )
}
