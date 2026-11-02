//! Desktop notifications via `notify-rust` (org.freedesktop.Notifications).
//!
//! Two surfaces: free-form [`notify`] for errors/updates, and
//! [`NowPlayingNotifier`] which posts one transient notification per track
//! change. `notify-rust` uses zbus 5 with `async-io`; its blocking `show()`
//! owns the runtime, so this module stays synchronous and thread-agnostic.
//!
//! ## Verus scope
//!
//! Nothing here is a candidate: every function performs D-Bus I/O. The only
//! pure decision ([`NowPlayingNotifier::should_notify`]) is a string compare and
//! is covered by a unit test.

use ml_core::{MlError, MlResult, PlayerSnapshot};
use notify_rust::{Hint, Notification, Timeout};

const APP_NAME: &str = "Metrolist Desktop";
const DESKTOP_ENTRY: &str = "Metrolist-Desktop";

/// Post a plain notification. `icon` may be an icon name or an absolute path.
pub fn notify_with(summary: &str, body: &str, icon: Option<&str>) -> MlResult<()> {
    let mut notification = Notification::new();
    notification
        .appname(APP_NAME)
        .summary(summary)
        .body(body)
        .hint(Hint::DesktopEntry(DESKTOP_ENTRY.to_owned()));
    if let Some(icon) = icon.filter(|icon| !icon.is_empty()) {
        notification.icon(icon);
    }
    notification
        .show()
        .map(|_| ())
        .map_err(|error| MlError::Io(format!("notification: {error}")))
}

/// Post a plain notification with the app's own icon.
pub fn notify(summary: &str, body: &str) -> MlResult<()> {
    notify_with(summary, body, Some("metrolist-desktop"))
}

/// Publishes at most one "now playing" notification per track.
#[derive(Debug, Default)]
pub struct NowPlayingNotifier {
    last_track: Option<String>,
    enabled: bool,
}

impl NowPlayingNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Turn now-playing notifications on/off (follows the UI setting).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.last_track = None;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The decision part of [`Self::on_snapshot`], free of I/O: notify when
    /// enabled, a track is playing, and it differs from the last notified one.
    pub fn should_notify(&self, snapshot: &PlayerSnapshot) -> bool {
        let Some(song) = snapshot.current.as_ref() else {
            return false;
        };
        if !self.enabled || !snapshot.is_playing() {
            return false;
        }
        self.last_track.as_deref()
            != Some(Self::key(song.id.as_str(), song.title.as_str()).as_str())
    }

    fn key(id: &str, title: &str) -> String {
        format!("{id}|{title}")
    }

    /// Show the now-playing notification when the track changed and playback
    /// started; seeks, repeats and volume changes never re-notify.
    pub fn on_snapshot(&mut self, snapshot: &PlayerSnapshot) -> MlResult<()> {
        if !self.should_notify(snapshot) {
            return Ok(());
        }
        let Some(song) = snapshot.current.as_ref() else {
            return Ok(());
        };
        let art = song.thumbnail_url.clone().unwrap_or_default();
        let mut notification = Notification::new();
        notification
            .appname(APP_NAME)
            .summary(&song.title)
            .body(&song.artist_names())
            .timeout(Timeout::Milliseconds(4000))
            // Transient: no notification history entry per track.
            .hint(Hint::Transient(true))
            .hint(Hint::DesktopEntry(DESKTOP_ENTRY.to_owned()));
        if !art.is_empty() {
            notification.hint(Hint::ImagePath(art));
        }
        notification
            .show()
            .map_err(|error| MlError::Io(format!("notification: {error}")))?;
        self.last_track = Some(Self::key(&song.id, &song.title));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ml_core::{PlaybackStatus, Song};

    fn snapshot(status: PlaybackStatus, id: &str) -> PlayerSnapshot {
        PlayerSnapshot {
            status,
            current: Some(Song {
                id: id.into(),
                title: format!("Track {id}"),
                ..Song::default()
            }),
            ..PlayerSnapshot::default()
        }
    }

    #[test]
    fn notifies_once_per_track_while_playing() {
        let mut notifier = NowPlayingNotifier::new();
        let playing = snapshot(PlaybackStatus::Playing, "a");
        // Disabled by default.
        assert!(!notifier.should_notify(&playing));
        notifier.set_enabled(true);
        assert!(notifier.should_notify(&playing));

        // Same track again: no second bubble, even after a seek.
        notifier.last_track = Some(NowPlayingNotifier::key("a", "Track a"));
        let mut moved = playing.clone();
        moved.position_ms = 42_000;
        assert!(!notifier.should_notify(&moved));

        // Paused/idle/empty never notify.
        assert!(!notifier.should_notify(&snapshot(PlaybackStatus::Paused, "b")));
        assert!(!notifier.should_notify(&snapshot(PlaybackStatus::Idle, "b")));
        assert!(!notifier.should_notify(&PlayerSnapshot::default()));

        // A different track does.
        assert!(notifier.should_notify(&snapshot(PlaybackStatus::Playing, "b")));

        // Disabling forgets the last track.
        notifier.set_enabled(false);
        notifier.set_enabled(true);
        assert!(notifier.should_notify(&playing));
    }
}
