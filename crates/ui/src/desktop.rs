//! Desktop integration: MPRIS, tray, notifications, single instance.
//!
//! All D-Bus work lives in `ml-integrations`; this module adapts the
//! `PlaybackController` to `PlayerControl` and turns integration callbacks into
//! UI events the app applies on the egui thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};
use ml_audio::PlaybackController;
use ml_core::{MlError, MlResult, PlaybackStatus, PlayerSnapshot, RepeatMode, Song};
use ml_integrations::{
    acquire_single_instance, spawn_tray, InstanceGuard, MprisService, NowPlayingNotifier,
    PlayerControl, TrayAction, TrayHandle, APP_ID, APP_NAME,
};
use parking_lot::Mutex;

use crate::pure::{parse_deep_link, tray_command, TrayCommand};

/// Controls raised by MPRIS or the tray that must run on the UI thread.
#[derive(Debug)]
enum UiSignal {
    Raise,
    Quit,
    OpenUri(String),
}

/// UI-thread commands produced by integration callbacks.
#[derive(Debug)]
pub enum DesktopEvent {
    ToggleWindow,
    Show,
    Hide,
    PlayPause,
    Next,
    Previous,
    Quit,
    OpenUri(String),
    /// argv forwarded by a second process.
    Forwarded(Vec<String>),
    Error(String),
}

/// Snapshot of which integrations are active, shown in Settings/About.
#[derive(Debug, Clone, Copy, Default)]
pub struct DesktopStatus {
    pub mpris: bool,
    pub tray: bool,
}

pub struct Desktop {
    mpris: Option<MprisService>,
    tray: Option<TrayHandle>,
    instance: Option<InstanceGuard>,
    signals: Receiver<UiSignal>,
    tray_actions: Receiver<TrayAction>,
    notifier: NowPlayingNotifier,
    last_snapshot: Option<PlayerSnapshot>,
    last_playing: Option<bool>,
    last_mpris_error: Option<String>,
    notify_failed_track: Option<String>,
}

impl Desktop {
    /// Start MPRIS/tray and take ownership of the already-acquired instance
    /// guard. Integration failures become startup notes, not fatal errors.
    pub fn start(
        controller: Arc<Mutex<PlaybackController>>,
        instance: Option<InstanceGuard>,
        notifications: bool,
        ctx: eframe::egui::Context,
    ) -> (Self, Vec<String>) {
        let (signal_tx, signals) = unbounded();
        let (tray_tx, tray_actions) = unbounded();
        let mut notes = Vec::new();

        let control: Arc<dyn PlayerControl> = Arc::new(UiController {
            controller,
            signals: signal_tx,
            ctx,
        });
        eprintln!("integrations: registering MPRIS");
        let mpris = match MprisService::spawn(control) {
            Ok(service) => Some(service),
            Err(error) => {
                notes.push(format!("MPRIS unavailable: {error}"));
                None
            }
        };
        eprintln!("integrations: registering tray");
        let tray = match spawn_tray(APP_NAME, tray_icon().as_deref(), tray_tx) {
            Ok(handle) => Some(handle),
            Err(error) => {
                notes.push(format!("Tray unavailable: {error}"));
                None
            }
        };

        eprintln!(
            "integrations: MPRIS={} tray={}",
            mpris.is_some(),
            tray.is_some()
        );
        let mut notifier = NowPlayingNotifier::new();
        notifier.set_enabled(notifications);
        (
            Self {
                mpris,
                tray,
                instance,
                signals,
                tray_actions,
                notifier,
                last_snapshot: None,
                last_playing: None,
                last_mpris_error: None,
                notify_failed_track: None,
            },
            notes,
        )
    }

    pub fn status(&self) -> DesktopStatus {
        DesktopStatus {
            mpris: self.mpris.as_ref().is_some_and(MprisService::is_running),
            tray: self.tray.as_ref().is_some_and(TrayHandle::is_running),
        }
    }

    pub fn set_notifications(&mut self, enabled: bool) {
        self.notifier.set_enabled(enabled);
    }

    /// Publish playback state and collect integration/forwarded events.
    pub fn poll(&mut self, snapshot: &PlayerSnapshot) -> Vec<DesktopEvent> {
        let mut events = Vec::new();

        if self.last_snapshot.as_ref() != Some(snapshot) {
            if let Some(mpris) = self.mpris.as_ref() {
                mpris.update(snapshot);
                let error = mpris.last_error();
                if error.is_some() && error != self.last_mpris_error {
                    events.push(DesktopEvent::Error(format!(
                        "MPRIS: {}",
                        error.as_deref().unwrap_or_default()
                    )));
                }
                self.last_mpris_error = error;
            }
            self.last_snapshot = Some(snapshot.clone());
        }
        let playing = snapshot.is_playing();
        if self.last_playing != Some(playing) {
            if let Some(tray) = self.tray.as_ref() {
                tray.set_playing(playing);
            }
            self.last_playing = Some(playing);
        }

        let track = snapshot
            .current
            .as_ref()
            .map(|song| format!("{}|{}", song.id, song.title));
        if track != self.notify_failed_track && self.notifier.should_notify(snapshot) {
            if let Err(error) = self.notifier.on_snapshot(snapshot) {
                self.notify_failed_track = track;
                events.push(DesktopEvent::Error(error.to_string()));
            }
        }

        while let Ok(action) = self.tray_actions.try_recv() {
            match tray_command(action) {
                TrayCommand::ToggleWindow => events.push(DesktopEvent::ToggleWindow),
                TrayCommand::Show => events.push(DesktopEvent::Show),
                TrayCommand::Hide => events.push(DesktopEvent::Hide),
                TrayCommand::PlayPause => events.push(DesktopEvent::PlayPause),
                TrayCommand::Next => events.push(DesktopEvent::Next),
                TrayCommand::Previous => events.push(DesktopEvent::Previous),
                TrayCommand::Quit => events.push(DesktopEvent::Quit),
            }
        }

        while let Ok(signal) = self.signals.try_recv() {
            match signal {
                UiSignal::Raise => events.push(DesktopEvent::Show),
                UiSignal::Quit => events.push(DesktopEvent::Quit),
                UiSignal::OpenUri(uri) => events.push(DesktopEvent::OpenUri(uri)),
            }
        }

        if let Some(instance) = self.instance.as_ref() {
            while let Ok(args) = instance.requests().try_recv() {
                events.push(DesktopEvent::Forwarded(args));
            }
        }

        events
    }

    /// Stop MPRIS/tray and release the single-instance lock and its socket.
    /// Idempotent; eframe may close the window without dropping `App`.
    pub fn shutdown(&mut self) {
        self.instance.take();
        self.mpris.take();
        self.tray.take();
    }
}

/// Acquire the single-instance lock before a window is created.
pub fn take_instance() -> MlResult<Option<InstanceGuard>> {
    acquire_single_instance(APP_ID)
}

/// `None` when forwarded without a playable deep link.
pub fn deep_link_video(args: &[String]) -> Option<String> {
    crate::pure::first_deep_link(args)
}

fn tray_icon() -> Option<PathBuf> {
    // Packaged AppImages export METROLIST_ASSETS; the next two cover a dev
    // build and a system install.
    let packaged = std::env::var_os("METROLIST_ASSETS")
        .map(|assets| Path::new(&assets).join("icons/metrolist-desktop-128.png"));
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/metrolist-desktop-128.png");
    let installed = Path::new("/usr/share/icons/hicolor/128x128/apps/metrolist-desktop.png");
    packaged
        .into_iter()
        .chain([source, installed.to_path_buf()])
        .find(|path| path.is_file())
}

struct UiController {
    controller: Arc<Mutex<PlaybackController>>,
    signals: Sender<UiSignal>,
    ctx: eframe::egui::Context,
}

impl UiController {
    /// MPRIS calls run on the D-Bus thread; the UI must poll the controller
    /// for the resulting events (including `NeedUrl`), so poke egui.
    fn wake(&self) {
        self.ctx.request_repaint();
    }

    fn send(&self, signal: UiSignal) -> MlResult<()> {
        self.signals
            .send(signal)
            .map_err(|_| MlError::Io("desktop event channel closed".into()))
    }
}

impl PlayerControl for UiController {
    fn play(&self) -> MlResult<()> {
        let mut controller = self.controller.lock();
        if !matches!(
            controller.snapshot().status,
            PlaybackStatus::Playing | PlaybackStatus::Buffering | PlaybackStatus::Loading
        ) {
            controller.play_pause();
        }
        self.wake();
        Ok(())
    }

    fn pause(&self) -> MlResult<()> {
        let mut controller = self.controller.lock();
        if matches!(
            controller.snapshot().status,
            PlaybackStatus::Playing | PlaybackStatus::Buffering | PlaybackStatus::Loading
        ) {
            controller.play_pause();
        }
        self.wake();
        Ok(())
    }

    fn play_pause(&self) -> MlResult<()> {
        self.controller.lock().play_pause();
        self.wake();
        Ok(())
    }

    fn stop(&self) -> MlResult<()> {
        self.controller.lock().stop();
        self.wake();
        Ok(())
    }

    fn next(&self) -> MlResult<()> {
        self.controller.lock().next();
        self.wake();
        Ok(())
    }

    fn previous(&self) -> MlResult<()> {
        self.controller.lock().previous();
        self.wake();
        Ok(())
    }

    fn seek_by_ms(&self, offset_ms: i64) -> MlResult<()> {
        let mut controller = self.controller.lock();
        let snapshot = controller.snapshot();
        let target =
            crate::pure::seek_relative(snapshot.position_ms, snapshot.duration_ms, offset_ms);
        controller.seek(target);
        drop(controller);
        self.wake();
        Ok(())
    }

    fn set_position_ms(&self, position_ms: u64) -> MlResult<()> {
        self.controller.lock().seek(position_ms);
        self.wake();
        Ok(())
    }

    fn set_volume(&self, volume: f64) -> MlResult<()> {
        self.controller
            .lock()
            .set_volume(volume.clamp(0.0, 1.0) as f32);
        self.wake();
        Ok(())
    }

    fn set_shuffle(&self, shuffle: bool) -> MlResult<()> {
        let mut controller = self.controller.lock();
        if controller.queue_view().shuffle != shuffle {
            controller.toggle_shuffle();
        }
        drop(controller);
        self.wake();
        Ok(())
    }

    fn set_repeat(&self, repeat: RepeatMode) -> MlResult<()> {
        let mut controller = self.controller.lock();
        for _ in 0..3 {
            if controller.queue_view().repeat == repeat {
                break;
            }
            controller.cycle_repeat();
        }
        drop(controller);
        self.wake();
        Ok(())
    }

    fn state(&self) -> PlayerSnapshot {
        self.controller.lock().snapshot()
    }

    fn current_song(&self) -> Option<Song> {
        self.controller.lock().snapshot().current
    }

    fn current_art_url(&self) -> Option<String> {
        self.controller
            .lock()
            .snapshot()
            .current
            .map(|song| song.thumb(512))
    }

    fn raise(&self) -> MlResult<()> {
        self.send(UiSignal::Raise)
    }

    fn can_raise(&self) -> bool {
        true
    }

    fn open_uri(&self, uri: &str) -> MlResult<()> {
        if parse_deep_link(uri).is_none() {
            return Err(MlError::Unavailable(format!("unsupported URI: {uri}")));
        }
        self.send(UiSignal::OpenUri(uri.to_string()))
    }

    fn quit(&self) -> MlResult<()> {
        self.send(UiSignal::Quit)
    }
}
