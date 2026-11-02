//! StatusNotifierItem tray via `ksni`.
//!
//! The menu publishes Show/Hide, Play/Pause, Next, Previous, Quit; every click
//! becomes a [`TrayAction`] on the caller's channel, so the tray thread never
//! touches player state. `ksni` runs on its own thread with an `async-io`
//! executor (the `blocking` API), never sharing a runtime with MPRIS.

use std::path::Path;

use crossbeam_channel::Sender;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem};
use ksni::{Icon, ToolTip};
use ml_core::{MlError, MlResult};

use crate::APP_ID;

/// User intent from the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// Toggle window visibility.
    ShowHide,
    /// Show (raise) the window.
    Show,
    /// Hide the window.
    Hide,
    PlayPause,
    Next,
    Previous,
    Quit,
}

/// Tray menu model. `playing` only decides the Play/Pause label.
struct MlTray {
    app_name: String,
    playing: bool,
    icon: Vec<Icon>,
    icon_name: String,
    tx: Sender<TrayAction>,
}

impl MlTray {
    fn item(&self, label: &str, icon: &str, action: TrayAction) -> MenuItem<Self> {
        let tx = self.tx.clone();
        MenuItem::Standard(StandardItem {
            label: label.into(),
            icon_name: icon.into(),
            activate: Box::new(move |_| {
                let _ = tx.try_send(action);
            }),
            ..Default::default()
        })
    }
}

impl ksni::Tray for MlTray {
    fn id(&self) -> String {
        APP_ID.into()
    }

    fn title(&self) -> String {
        self.app_name.clone()
    }

    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.app_name.clone(),
            description: if self.playing {
                "Playing".into()
            } else {
                "Paused".into()
            },
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.try_send(TrayAction::ShowHide); // dropped receiver = shutting down
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let play_label = if self.playing { "Pause" } else { "Play" };
        let play_icon = if self.playing {
            "media-playback-pause"
        } else {
            "media-playback-start"
        };
        vec![
            self.item("Show", "window-new", TrayAction::Show),
            self.item("Hide", "window-close", TrayAction::Hide),
            MenuItem::Separator,
            self.item(play_label, play_icon, TrayAction::PlayPause),
            self.item("Next", "media-skip-forward", TrayAction::Next),
            self.item("Previous", "media-skip-backward", TrayAction::Previous),
            MenuItem::Separator,
            self.item("Quit", "application-exit", TrayAction::Quit),
        ]
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        // No StatusNotifierWatcher/Host: stop instead of spinning forever.
        false
    }
}

/// Embedded 32x32 placeholder shown when the configured icon is unusable.
fn placeholder_icon() -> Vec<Icon> {
    const PNG: &[u8] = include_bytes!("tray_placeholder.png");
    decode_icon(PNG).into_iter().collect()
}

fn decode_icon(bytes: &[u8]) -> Option<Icon> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut data = rgba.into_vec();
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.rotate_right(1); // RGBA -> ARGB (network byte order)
    }
    Some(Icon {
        width: width as i32,
        height: height as i32,
        data,
    })
}

/// Decode the configured icon, or fall back to the embedded placeholder.
///
/// A missing or undecodable file is not an error — the tray must still appear —
/// so the second value always states which icon is in use and why.
fn load_icon(path: Option<&Path>) -> (Vec<Icon>, String) {
    let Some(path) = path else {
        return (placeholder_icon(), "embedded placeholder".into());
    };
    match std::fs::read(path) {
        Ok(bytes) => match decode_icon(&bytes) {
            Some(icon) => (vec![icon], format!("icon: {}", path.display())),
            None => (
                placeholder_icon(),
                format!("icon undecodable, using placeholder: {}", path.display()),
            ),
        },
        Err(error) => (
            placeholder_icon(),
            format!(
                "icon unreadable ({error}), using placeholder: {}",
                path.display()
            ),
        ),
    }
}

/// A running tray icon.
pub struct TrayHandle {
    handle: Handle<MlTray>,
    icon_source: String,
}

impl TrayHandle {
    /// Update the Play/Pause state shown in the menu and tooltip.
    pub fn set_playing(&self, playing: bool) {
        let _ = self
            .handle
            .update(|tray: &mut MlTray| tray.playing = playing);
    }

    /// `false` once the tray service stopped (no watcher, or `shutdown`).
    pub fn is_running(&self) -> bool {
        !self.handle.is_closed()
    }

    /// Stop the tray service and wait for the D-Bus thread to finish.
    pub fn shutdown(&self) {
        self.handle.shutdown().wait();
    }

    /// The icon actually in use: `icon: <path>`, or why the placeholder was used.
    pub fn icon_source(&self) -> &str {
        &self.icon_source
    }
}

impl std::fmt::Debug for TrayHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayHandle")
            .field("running", &self.is_running())
            .finish()
    }
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start the tray. Fails when there is no `org.kde.StatusNotifierWatcher` on
/// the session bus (headless, or no AppIndicator extension).
pub fn spawn_tray(
    app_name: &str,
    icon_path: Option<&Path>,
    tx: Sender<TrayAction>,
) -> MlResult<TrayHandle> {
    let (icon, icon_source) = load_icon(icon_path);
    let tray = MlTray {
        app_name: app_name.to_owned(),
        playing: false,
        icon,
        icon_name: "metrolist-desktop".into(),
        tx,
    };
    let handle = tray.spawn().map_err(|error| match error {
        ksni::Error::Watcher(error) => {
            MlError::Unavailable(format!("no StatusNotifierWatcher: {error}"))
        }
        ksni::Error::WontShow => MlError::Unavailable("no StatusNotifierHost".into()),
        ksni::Error::Dbus(error) => MlError::Io(format!("tray D-Bus: {error}")),
        other => MlError::Io(format!("tray: {other}")),
    })?;
    Ok(TrayHandle {
        handle,
        icon_source,
    })
}
