//! `ml-integrations` — the desktop surfaces of Metrolist Linux.
//!
//! | Surface | Crate | Bus name |
//! |---|---|---|
//! | MPRIS2 (`org.mpris.MediaPlayer2.Player`) | `mpris-server` 0.11 (zbus 6) | `org.mpris.MediaPlayer2.metrolist` |
//! | StatusNotifierItem tray | `ksni` 0.3 (zbus 5, async-io) | `org.kde.StatusNotifierItem-<pid>-1` |
//! | Notifications | `notify-rust` 4 (zbus 5, async-io) | `org.freedesktop.Notifications` |
//! | Single instance | `single-instance` 0.3 + `AF_UNIX` socket | `$XDG_RUNTIME_DIR/<app_id>.sock` |
//! | Scrobbling (Last.fm) | `reqwest` + md5-signed form POSTs | `https://ws.audioscrobbler.com/2.0/` |
//! | Discord presence (feature `discord`) | hand-rolled IPC | `$XDG_RUNTIME_DIR/discord-ipc-0` |
//!
//! Two zbus generations coexist on purpose: `mpris-server` requires zbus 6 and
//! is driven by the tokio runtime this crate owns, while `ksni`/`notify-rust`
//! require zbus 5 with `async-io`, which cannot share a tokio runtime. Both are
//! their own threads; nothing is spawned onto a caller's executor.
//!
//! Nothing here depends on a UI toolkit: the app passes in a [`PlayerControl`]
//! and drains [`TrayAction`]s, [`InstanceGuard::requests`] and snapshots.
//!
//! ## Verification scope (Verus)
//!
//! [`mapping`] is the pure, total, panic-free core (status/loop/shuffle mapping,
//! ms↔µs conversion, `mpris:trackid` encoding, metadata projection, property
//! diffing), and [`lastfm`]'s signature/threshold/queue helpers join it
//! (`signature`, `scrobble_threshold_ms`, `scrobble_params`, `classify_error`,
//! `encode_queue`/`decode_queue`). Neither module does I/O, locks or floats in
//! its pure half, so the proof lead can state them in Verus directly.
//! [`mpris`], [`tray`], [`notify`], [`instance`] and the [`lastfm`] transport
//! drive D-Bus, threads, sockets and HTTP — outside Verus's subset — and are
//! covered by tests plus live evidence instead.

#![deny(missing_debug_implementations)]

pub mod instance;
pub mod lastfm;
pub mod mapping;
pub mod mpris;
pub mod notify;
pub mod tray;

#[cfg(feature = "discord")]
pub mod discord;

#[cfg(feature = "discord")]
pub use discord::{DiscordPresence, CLIENT_ID as DISCORD_CLIENT_ID};
pub use instance::{Instance, InstanceGuard};
pub use lastfm::{
    classify_error, signature, Failure, LastFmClient, LastFmConfig, LastFmError, LastFmTransport,
    Scrobble, ScrobblePolicy, ScrobbleQueue, Scrobbler, ScrobblerHooks, SessionInvalid, Transition,
};
pub use mapping::{Change, Published, TrackMetadata, TransportLoop, TransportStatus};
pub use mpris::{MprisService, PlayerCommand, PlayerControl, SnapshotControl, UpdateThrottle};
pub use notify::{notify, notify_with, NowPlayingNotifier};
pub use tray::{spawn_tray, TrayAction, TrayHandle};

/// Take the single-instance lock for `app_id`.
///
/// `Ok(Some(guard))` — this process is the primary instance; drain
/// [`InstanceGuard::requests`] for argv forwarded by later launches.
/// `Ok(None)` — another instance was running and has already received this
/// process's argv; exit 0 without starting a window.
pub fn acquire_single_instance(app_id: &str) -> ml_core::MlResult<Option<InstanceGuard>> {
    InstanceGuard::acquire(app_id)
}

/// Public identity used by the desktop entry and the single-instance lock.
pub const APP_ID: &str = "org.metrolist.Metrolist";
/// Human name shown in the tray tooltip and notifications.
pub const APP_NAME: &str = "Metrolist Desktop";

#[cfg(test)]
mod tests {
    use super::*;

    /// Every long-lived surface must release its thread on drop.
    #[test]
    fn dropping_services_leaves_no_threads_behind() {
        let before = std::fs::read_dir("/proc/self/task")
            .map(|entries| entries.count())
            .unwrap_or(0);
        for _ in 0..3 {
            let (control, _rx) = mpris::SnapshotControl::new();
            // No session bus in unit tests: either it starts or it reports why.
            let _ = mpris::MprisService::spawn(control);

            // The instance guard owns an accept thread + socket.
            let app_id = format!("ml-leak-test-{}-{}", std::process::id(), before);
            if let Ok(Some(guard)) = instance::InstanceGuard::acquire(&app_id) {
                let path = guard.socket_path().clone();
                drop(guard);
                assert!(!path.exists(), "socket must be removed on drop");
            }
        }
        let after = std::fs::read_dir("/proc/self/task")
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert!(
            after <= before + 1,
            "threads leaked: {before} before, {after} after"
        );
    }

    #[test]
    fn identity_matches_the_packaged_desktop_entry() {
        // packaging/Metrolist-Desktop.desktop: Icon=metrolist-desktop, Exec=metrolist
        assert_eq!(APP_ID, "org.metrolist.Metrolist");
        assert_eq!(mpris::MPRIS_BUS_NAME, "org.mpris.MediaPlayer2.metrolist");
        assert_eq!(mpris::DESKTOP_ENTRY, "Metrolist-Desktop");
    }
}
