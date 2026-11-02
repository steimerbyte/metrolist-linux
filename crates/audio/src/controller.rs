//! Queue-aware playback controller.
//!
//! The controller owns the [`Queue`] and translates it into engine commands.
//! It never blocks: the UI calls [`PlaybackController::poll`] to drain engine
//! events and receives [`ControllerEvent`]s. Stream URLs come from a
//! [`StreamResolver`] (cache hits) or from the caller asynchronously via
//! [`PlaybackController::submit_resolved`]; when a URL is missing the
//! controller emits [`ControllerEvent::NeedUrl`] and resumes once it arrives.
//!
//! Gapless playback: after a track starts, the next resolved URL is appended
//! to the mpv playlist ([`AudioCommand::EnqueueNext`]). mpv then advances by
//! itself on EOF; the controller notices on [`AudioEvent::EndFile`] and moves
//! the queue pointer without reloading, so there is no gap.
//!
//! Crossfade: mpv cannot overlap two files in one process, so the controller
//! drives a second [`AudioCommand`]/[`AudioEvent`] pair (see
//! [`PlaybackController::attach_crossfade_engine`]). The outgoing track keeps
//! playing on the old engine while the incoming track starts on the new one;
//! both volumes ramp with Android's square-law curve and the controller then
//! swaps which channel pair is active. Without a second engine, or for tracks
//! in the same album when [`PlaybackController::set_crossfade_gapless`] is on,
//! the controller keeps the gapless playlist path.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use ml_core::{PlaybackStatus, PlayerSnapshot, Queue, QueueSource, RepeatMode, Song};

use crate::{AudioCommand, AudioEvent};

/// Synchronous cache lookup for stream URLs.
///
/// Resolving over the network is asynchronous, so this trait is only expected
/// to return URLs that are already known. Return `None` and the controller
/// emits [`ControllerEvent::NeedUrl`]; the caller then resolves and calls
/// [`PlaybackController::submit_resolved`].
pub trait StreamResolver: Send + Sync {
    fn resolve(&self, video_id: &str) -> Option<String>;
}

/// Sleep timer modes mirrored from the Android client.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SleepTimer {
    #[default]
    Off,
    /// Pause after this many minutes (zero pauses on the next tick). Clamped
    /// to 24 h internally.
    Minutes(u32),
    /// Pause when the current track finishes instead of advancing.
    EndOfTrack,
    /// Ramp the volume to zero over this duration, then pause. Applied
    /// immediately; the queue pointer does not move.
    FadeOut(Duration),
}

/// `true` when `now` is inside the daily window `start..end`, in minutes since
/// midnight. Windows wrap overnight (`22:00..06:00`); `start == end` means the
/// whole day. Mirrors the Android automatic sleep-timer window check.
pub fn sleep_window_contains(now_minutes: u32, start_minutes: u32, end_minutes: u32) -> bool {
    const DAY: u32 = 24 * 60;
    let now = now_minutes % DAY;
    let start = start_minutes % DAY;
    let end = end_minutes % DAY;
    if start == end {
        true
    } else if start < end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

/// Command/event channels of one mpv engine.
struct EngineSlot {
    commands: Sender<AudioCommand>,
    events: Receiver<AudioEvent>,
}

/// How the next track was prepared on the inactive engine.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Preload {
    /// Appended to the active mpv playlist; mpv advances by itself on EOF.
    Appended(String),
    /// Loaded paused at volume 0 on the inactive engine, ready for a crossfade.
    Prepared(String),
}

struct CrossfadeFade {
    started: Instant,
    duration: Duration,
}

struct SleepFade {
    started: Instant,
    duration: Duration,
}

/// Longest accepted crossfade; longer values would swallow whole tracks.
const MAX_CROSSFADE: Duration = Duration::from_secs(30);

/// Album identity used by the `crossfadeGapless` option: album id when present,
/// otherwise the album title. Missing/empty albums never match.
fn same_album(left: &Song, right: &Song) -> bool {
    let key = |song: &Song| {
        song.album
            .as_ref()
            .map(|album| {
                if album.id.is_empty() {
                    album.title.clone()
                } else {
                    album.id.clone()
                }
            })
            .filter(|value| !value.is_empty())
    };
    match (key(left), key(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// UI-facing queue state (songs in insertion order, plus playback order data).
#[derive(Debug, Clone, PartialEq)]
pub struct QueueView {
    pub songs: Vec<Song>,
    pub position: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub source: QueueSource,
}

impl QueueView {
    fn from_queue(queue: &Queue) -> Self {
        Self {
            songs: queue.songs().to_vec(),
            position: if queue.is_empty() {
                None
            } else {
                Some(queue.position())
            },
            shuffle: queue.is_shuffled(),
            repeat: queue.repeat_mode(),
            source: queue.source(),
        }
    }
}

/// Unified events emitted to the UI.
// Snapshot is the hot payload and events are low-rate; boxing would only add
// an allocation per event without changing the copy cost of `Snapshot`.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum ControllerEvent {
    Snapshot(PlayerSnapshot),
    QueueChanged(QueueView),
    Error(String),
    EndOfQueue,
    /// The caller must resolve this video id and call `submit_resolved`.
    /// `prefetch` marks requests for the *next* track (gapless preloading),
    /// not the track that should start now.
    NeedUrl {
        video_id: String,
        prefetch: bool,
    },
    /// Output-device fallback reported by an engine. `fallback_from` is set
    /// when the requested id was unavailable and playback moved to `auto`.
    DeviceChanged {
        id: String,
        fallback_from: Option<String>,
    },
}

/// Owns the queue and drives the engine.
pub struct PlaybackController {
    queue: Queue,
    active: EngineSlot,
    inactive: Option<EngineSlot>,
    resolver: Arc<dyn StreamResolver>,
    snapshot: PlayerSnapshot,
    resolved: HashMap<String, String>,
    current_url: Option<String>,
    awaiting: Option<String>,
    prefetch_pending: Option<String>,
    preloaded: Option<Preload>,
    sleep: SleepTimer,
    sleep_deadline: Option<Instant>,
    sleep_fade: Option<SleepFade>,
    crossfade: Duration,
    crossfade_gapless: bool,
    fade: Option<CrossfadeFade>,
    outbox: Vec<ControllerEvent>,
}

impl PlaybackController {
    pub fn new(
        commands: Sender<AudioCommand>,
        events: Receiver<AudioEvent>,
        resolver: Arc<dyn StreamResolver>,
    ) -> Self {
        Self {
            queue: Queue::default(),
            active: EngineSlot { commands, events },
            inactive: None,
            resolver,
            snapshot: PlayerSnapshot::default(),
            resolved: HashMap::new(),
            current_url: None,
            awaiting: None,
            prefetch_pending: None,
            preloaded: None,
            sleep: SleepTimer::Off,
            sleep_deadline: None,
            sleep_fade: None,
            crossfade: Duration::ZERO,
            crossfade_gapless: true,
            fade: None,
            outbox: Vec::new(),
        }
    }

    /// Replace the queue and start at `start_index` (clamped to range).
    pub fn play_queue(&mut self, mut queue: Queue, start_index: usize) {
        if !queue.is_empty() {
            let index = start_index.min(queue.len() - 1);
            let _ = queue.jump_to(index);
        }
        self.queue = queue;
        self.abort_crossfade();
        // A prepared next track is the only reason the inactive engine holds a
        // loaded file; drop it, otherwise it would keep a stream open.
        if matches!(self.preloaded, Some(Preload::Prepared(_))) {
            if let Some(slot) = &self.inactive {
                let _ = slot.commands.send(AudioCommand::Stop);
                let _ = slot.commands.send(AudioCommand::SetVolume(0.0));
            }
        }
        self.awaiting = None;
        self.prefetch_pending = None;
        self.preloaded = None;
        self.current_url = None;
        self.snapshot.status = PlaybackStatus::Loading;
        self.snapshot.position_ms = 0;
        self.snapshot.duration_ms = 0;
        self.start_current();
    }

    /// Install a persisted queue without autoplay or engine commands.
    ///
    /// The queue keeps its current position, source, shuffle and repeat state.
    /// The first `play_pause` resolves and starts the restored current track.
    pub fn restore_queue(&mut self, queue: Queue) {
        self.queue = queue;
        self.abort_crossfade();
        self.awaiting = None;
        self.prefetch_pending = None;
        self.preloaded = None;
        self.current_url = None;
        self.snapshot.status = PlaybackStatus::Stopped;
        self.snapshot.position_ms = 0;
        self.snapshot.duration_ms = 0;
        self.emit_snapshot();
        self.emit_queue();
    }

    /// Append radio continuation tracks without replacing the queue or moving
    /// the current track. Duplicate video ids are ignored.
    pub fn append_queue(&mut self, songs: Vec<Song>) -> usize {
        let mut known: HashSet<String> = self
            .queue
            .songs()
            .iter()
            .map(|song| song.id.clone())
            .collect();
        let mut added = 0;
        for song in songs {
            if known.insert(song.id.clone()) {
                self.queue.append(song);
                added += 1;
            }
        }
        if added == 0 {
            return 0;
        }
        if matches!(
            self.snapshot.status,
            PlaybackStatus::Playing
                | PlaybackStatus::Buffering
                | PlaybackStatus::Loading
                | PlaybackStatus::Paused
        ) {
            self.prefetch_next();
        }
        self.emit_queue();
        self.emit_snapshot();
        added
    }

    /// Move a playback-order item; the current track and its index follow the
    /// same move and the prepared successor is rebuilt when needed.
    pub fn move_queue_item(&mut self, from: usize, to: usize) -> bool {
        if !self.queue.move_item(from, to) {
            return false;
        }
        self.after_queue_reorder();
        self.emit_queue();
        self.emit_snapshot();
        true
    }

    /// Remove a playback-order item. Removing the current track starts its
    /// successor (or stops cleanly when the queue becomes empty).
    pub fn remove_queue_item(&mut self, position: usize) -> Option<Song> {
        let was_current = self.queue.position() == position;
        let removed = self.queue.remove(position)?;
        if self.queue.is_empty() {
            self.abort_crossfade();
            self.awaiting = None;
            self.prefetch_pending = None;
            self.preloaded = None;
            self.current_url = None;
            self.send(AudioCommand::Stop);
            self.snapshot.status = PlaybackStatus::Stopped;
            self.snapshot.position_ms = 0;
            self.snapshot.duration_ms = 0;
            self.emit_snapshot();
            self.emit_queue();
        } else if was_current {
            self.current_url = None;
            self.snapshot.position_ms = 0;
            self.start_current();
        } else {
            self.after_queue_reorder();
            self.emit_queue();
            self.emit_snapshot();
        }
        Some(removed)
    }

    /// Toggle play/pause; restarts the current track from Idle/Stopped/Ended.
    pub fn play_pause(&mut self) {
        match self.snapshot.status {
            PlaybackStatus::Playing | PlaybackStatus::Buffering | PlaybackStatus::Loading => {
                if let Some(slot) = &self.inactive {
                    if self.fade.is_some() {
                        let _ = slot.commands.send(AudioCommand::Pause);
                    }
                }
                self.send(AudioCommand::TogglePause);
            }
            PlaybackStatus::Paused => {
                if let Some(slot) = &self.inactive {
                    if self.fade.is_some() {
                        let _ = slot.commands.send(AudioCommand::Play);
                    }
                }
                self.send(AudioCommand::Play);
            }
            _ => {
                if self.queue.current().is_some() {
                    self.start_current();
                }
            }
        }
    }

    /// Advance to the next queued track (honours repeat/shuffle via the queue).
    pub fn next(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        match self.queue.next() {
            Some(_) => self.start_current(),
            None => self.outbox.push(ControllerEvent::EndOfQueue),
        }
    }

    /// Go back one track; at the start of the queue this restarts the track.
    pub fn previous(&mut self) {
        if self.queue.previous().is_some() {
            self.start_current();
        } else {
            self.send(AudioCommand::SeekAbsolute(0));
            self.snapshot.position_ms = 0;
            self.emit_snapshot();
        }
    }

    pub fn seek(&mut self, position_ms: u64) {
        self.send(AudioCommand::SeekAbsolute(position_ms));
        self.snapshot.position_ms = position_ms;
        self.emit_snapshot();
    }

    pub fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.send(AudioCommand::SetVolume(volume));
        self.snapshot.volume = volume;
        self.emit_snapshot();
    }

    pub fn toggle_shuffle(&mut self) {
        let shuffle = !self.queue.is_shuffled();
        self.queue.set_shuffle(shuffle);
        self.after_queue_reorder();
        self.emit_queue();
        self.emit_snapshot();
    }

    pub fn cycle_repeat(&mut self) {
        self.queue.cycle_repeat();
        self.after_queue_reorder();
        self.emit_queue();
        self.emit_snapshot();
    }

    /// Apply 10-band graphic-EQ gains (dB, [`crate::EQ_BANDS_HZ`]) to both
    /// engines, so a crossfade never switches to an unequalized track.
    pub fn set_eq(&mut self, bands: &[f32]) {
        self.send_both(AudioCommand::SetEqBands(bands.to_vec()));
    }

    /// Enable loudness normalization and set the per-track correction in dB
    /// (see [`crate::normalization_gain_db`]). `None` or a non-finite gain
    /// disables the gain stage; the caller keeps normalization off when the
    /// stream carries no loudness metadata.
    pub fn set_normalization(&mut self, enabled: bool, gain_db: Option<f32>) {
        let gain_db = gain_db.filter(|gain| gain.is_finite()).unwrap_or(0.0);
        self.send_both(AudioCommand::SetGainDb(gain_db));
        self.send_both(AudioCommand::SetNormalization(enabled));
    }

    /// Playback speed (`0.25..=4.0`, pitch preserved by mpv's scaletempo2).
    pub fn set_speed(&mut self, speed: f32) {
        self.send_both(AudioCommand::SetSpeed(speed));
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.send_both(AudioCommand::SetMuted(muted));
    }

    /// Drop silence gaps longer than 0.5 s below -50 dBFS on both engines.
    pub fn set_skip_silence(&mut self, enabled: bool) {
        self.send_both(AudioCommand::SetSkipSilence(enabled));
    }

    /// Switch both engines to `device` (empty/`None` = mpv automatic).
    pub fn set_device(&mut self, device: Option<&str>) {
        self.send_both(AudioCommand::SetDevice(
            device.unwrap_or_default().to_string(),
        ));
    }

    /// Attach the second mpv engine used for audible crossfade. The caller
    /// keeps the [`crate::MpvEngine`] alive and hands over its channels; the
    /// controller consumes them from then on. Returns `false` when a secondary
    /// engine is already attached.
    #[must_use]
    pub fn attach_crossfade_engine(
        &mut self,
        commands: Sender<AudioCommand>,
        events: Receiver<AudioEvent>,
    ) -> bool {
        if self.inactive.is_some() {
            return false;
        }
        self.inactive = Some(EngineSlot { commands, events });
        let appended = matches!(self.preloaded.take(), Some(Preload::Appended(_)));
        if appended {
            if let Some(url) = self.current_url.clone() {
                self.send(AudioCommand::LoadUrl {
                    url,
                    start_ms: Some(self.snapshot.position_ms),
                });
                self.snapshot.status = PlaybackStatus::Loading;
            }
        }
        self.prefetch_next();
        true
    }

    /// Crossfade duration; zero disables it. Clamped to 30 s. Requires
    /// [`Self::attach_crossfade_engine`]; without a second engine playback
    /// stays gapless. Tracks shorter than the crossfade are not faded.
    pub fn set_crossfade(&mut self, duration: Duration) {
        let duration = duration.min(MAX_CROSSFADE);
        if self.crossfade == duration {
            return;
        }
        self.crossfade = duration;
        self.abort_crossfade();
        self.after_queue_reorder();
    }

    /// Keep consecutive tracks from the same album gapless instead of fading
    /// them (Android `crossfadeGapless`).
    pub fn set_crossfade_gapless(&mut self, enabled: bool) {
        if self.crossfade_gapless == enabled {
            return;
        }
        self.crossfade_gapless = enabled;
        self.abort_crossfade();
        self.after_queue_reorder();
    }

    /// `(duration, gapless)` as configured.
    pub fn crossfade(&self) -> (Duration, bool) {
        (self.crossfade, self.crossfade_gapless)
    }

    /// `true` while two tracks overlap.
    pub fn is_crossfading(&self) -> bool {
        self.fade.is_some()
    }

    /// Arm, replace or clear the sleep timer. `FadeOut` starts ramping
    /// immediately; `Minutes` pauses at the deadline; `EndOfTrack` pauses
    /// instead of advancing when the current track ends.
    pub fn set_sleep_timer(&mut self, timer: SleepTimer) {
        self.clear_sleep_fade();
        self.sleep = timer;
        self.sleep_deadline = match timer {
            // Clamp to 24 h so the deadline always fits in `Instant`; Android's
            // picker never exceeds 60 min, but the public API takes any u32.
            SleepTimer::Minutes(minutes) => {
                let seconds = (minutes as u64 * 60).min(24 * 60 * 60);
                Some(Instant::now() + Duration::from_secs(seconds))
            }
            _ => None,
        };
        if let SleepTimer::FadeOut(duration) = timer {
            // Two ramps fighting over `volume` would leave the output audible.
            self.abort_crossfade();
            if duration.is_zero() {
                self.end_sleep();
            } else {
                self.sleep_fade = Some(SleepFade {
                    started: Instant::now(),
                    duration,
                });
            }
        }
    }

    pub fn sleep_timer(&self) -> SleepTimer {
        self.sleep
    }

    /// Android's automatic sleep timer: when `now` is inside the configured
    /// daily window, arm a `Minutes(minutes)` timer and return `true`.
    /// `now`/`start`/`end` are minutes since local midnight.
    pub fn arm_scheduled_sleep_timer(
        &mut self,
        now_minutes: u32,
        start_minutes: u32,
        end_minutes: u32,
        minutes: u32,
    ) -> bool {
        if !sleep_window_contains(now_minutes, start_minutes, end_minutes) {
            return false;
        }
        self.set_sleep_timer(SleepTimer::Minutes(minutes));
        true
    }

    pub fn stop(&mut self) {
        self.abort_crossfade();
        self.clear_sleep_fade();
        self.send(AudioCommand::Stop);
        self.awaiting = None;
        self.prefetch_pending = None;
        self.preloaded = None;
        self.current_url = None;
        self.snapshot.status = PlaybackStatus::Stopped;
        self.snapshot.position_ms = 0;
        self.emit_snapshot();
    }

    /// Ask both engines to stop and exit their threads. The owning
    /// [`crate::MpvEngine`]s reap the mpv children and remove their sockets;
    /// call [`crate::MpvEngine::shutdown`] for that. Idempotent.
    pub fn shutdown(&mut self) {
        let _ = self.active.commands.send(AudioCommand::Shutdown);
        if let Some(slot) = &self.inactive {
            let _ = slot.commands.send(AudioCommand::Shutdown);
        }
    }

    /// Caller-supplied result for a [`ControllerEvent::NeedUrl`] request.
    pub fn submit_resolved(&mut self, video_id: String, url: String) {
        self.resolved.insert(video_id.clone(), url);
        if self.awaiting.as_deref() == Some(video_id.as_str()) {
            self.awaiting = None;
            self.start_current();
            return;
        }
        if self.prefetch_pending.as_deref() == Some(video_id.as_str()) {
            self.prefetch_pending = None;
        }
        self.maybe_preload();
    }

    /// Drain engine events and emit UI events. Call from the UI loop; the
    /// fade ramps step once per call (the UI runs this every frame).
    pub fn poll(&mut self) -> Vec<ControllerEvent> {
        self.tick_sleep();
        self.tick_crossfade();
        // The inactive engine's events are not authoritative (it may be the
        // outgoing half of a fade or a paused preload); draining keeps the
        // channel bounded while the user listens to the active track.
        if let Some(slot) = &self.inactive {
            while slot.events.try_recv().is_ok() {}
        }
        while let Ok(event) = self.active.events.try_recv() {
            self.handle_engine_event(event);
        }
        std::mem::take(&mut self.outbox)
    }

    pub fn snapshot(&self) -> PlayerSnapshot {
        self.filled()
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn queue_view(&self) -> QueueView {
        QueueView::from_queue(&self.queue)
    }

    fn start_current(&mut self) {
        self.abort_crossfade();
        let Some((song_id, duration_ms)) = self.queue.current().map(|song| {
            (
                song.id.clone(),
                song.duration
                    .map(|seconds| seconds as u64 * 1000)
                    .unwrap_or(0),
            )
        }) else {
            self.emit_queue();
            self.outbox.push(ControllerEvent::EndOfQueue);
            return;
        };
        let Some(url) = self.url_for(&song_id) else {
            self.awaiting = Some(song_id.clone());
            self.outbox.push(ControllerEvent::NeedUrl {
                video_id: song_id,
                prefetch: false,
            });
            self.emit_queue();
            return;
        };
        self.awaiting = None;
        self.prefetch_pending = None;
        self.preloaded = None;
        self.current_url = Some(url.clone());
        self.snapshot.status = PlaybackStatus::Loading;
        self.snapshot.position_ms = 0;
        self.snapshot.duration_ms = duration_ms;
        self.send(AudioCommand::LoadUrl {
            url,
            start_ms: None,
        });
        self.emit_snapshot();
        self.emit_queue();
        self.prefetch_next();
    }

    fn url_for(&mut self, video_id: &str) -> Option<String> {
        if let Some(url) = self.resolved.get(video_id) {
            return Some(url.clone());
        }
        let url = self.resolver.resolve(video_id)?;
        self.resolved.insert(video_id.to_string(), url.clone());
        Some(url)
    }

    fn peek_next_song(&self) -> Option<Song> {
        let song = match self.queue.repeat_mode() {
            // Repeat-one is a hard reload; crossfading a track into itself is
            // not prepared (Android supports it, this port does not).
            RepeatMode::One => return None,
            RepeatMode::All => {
                let len = self.queue.len();
                if len == 0 {
                    None
                } else {
                    self.queue.song_at((self.queue.position() + 1) % len)
                }
            }
            RepeatMode::Off => self.queue.song_at(self.queue.position() + 1),
        };
        song.cloned()
    }

    /// `true` when the transition into `next` should be crossfaded: a second
    /// engine is attached, the duration is usable and the gapless option does
    /// not apply (same album), and a sleep timer is not about to stop playback.
    fn crossfade_candidate(&self, next: &Song) -> bool {
        self.crossfade > Duration::ZERO
            && self.inactive.is_some()
            && !matches!(self.sleep, SleepTimer::EndOfTrack | SleepTimer::FadeOut(_))
            && !(self.crossfade_gapless
                && self
                    .queue
                    .current()
                    .is_some_and(|current| same_album(current, next)))
    }

    fn prefetch_next(&mut self) {
        if self.current_url.is_none() || self.preloaded.is_some() || self.prefetch_pending.is_some()
        {
            return;
        }
        let Some(next) = self.peek_next_song() else {
            return;
        };
        if self.queue.current().is_some_and(|song| song.id == next.id) {
            return;
        }
        let crossfade = self.crossfade_candidate(&next);
        match self.url_for(&next.id) {
            Some(url) => {
                if crossfade {
                    self.prepare_crossfade(&url);
                    self.preloaded = Some(Preload::Prepared(next.id));
                } else {
                    self.send(AudioCommand::EnqueueNext { url });
                    self.preloaded = Some(Preload::Appended(next.id));
                }
            }
            None => {
                self.prefetch_pending = Some(next.id.clone());
                self.outbox.push(ControllerEvent::NeedUrl {
                    video_id: next.id,
                    prefetch: true,
                });
            }
        }
    }

    fn maybe_preload(&mut self) {
        self.prefetch_pending = None;
        self.prefetch_next();
    }

    /// Load `url` paused at volume 0 on the inactive engine; the crossfade
    /// starts it with [`AudioCommand::Play`] and ramps the volume.
    fn prepare_crossfade(&mut self, url: &str) {
        let Some(slot) = &self.inactive else {
            return;
        };
        let _ = slot.commands.send(AudioCommand::Pause);
        let _ = slot.commands.send(AudioCommand::SetVolume(0.0));
        let _ = slot.commands.send(AudioCommand::LoadUrl {
            url: url.to_string(),
            start_ms: None,
        });
        // Drop the backlog: those events belong to the inactive engine and
        // would be replayed as active state after the swap.
        while slot.events.try_recv().is_ok() {}
    }

    /// After shuffle/repeat changes a preloaded playlist entry may no longer be
    /// the right successor: rebuild the mpv playlist at the current position.
    /// A crossfade preparation on the inactive engine just needs discarding.
    fn after_queue_reorder(&mut self) {
        self.prefetch_pending = None;
        match self.preloaded.take() {
            Some(Preload::Appended(_)) => {
                if let Some(url) = self.current_url.clone() {
                    self.send(AudioCommand::LoadUrl {
                        url,
                        start_ms: Some(self.snapshot.position_ms),
                    });
                    self.snapshot.status = PlaybackStatus::Loading;
                }
            }
            Some(Preload::Prepared(_)) => {
                if let Some(slot) = &self.inactive {
                    let _ = slot.commands.send(AudioCommand::Stop);
                }
            }
            None => {}
        }
        self.prefetch_next();
    }

    fn handle_end_file(&mut self) {
        if self.fade.is_some() {
            // The incoming track ended before the ramp finished; stop the old
            // engine but do not prefetch until the queue pointer has moved.
            self.finish_crossfade(false);
        }
        let sleeping = matches!(self.sleep, SleepTimer::EndOfTrack);
        match self.preloaded.take() {
            Some(Preload::Appended(_)) => {
                // mpv already advanced to the appended track; move the queue
                // pointer in lockstep without issuing a load (that would stop
                // playback).
                let _ = self.queue.next();
                self.current_url = self
                    .queue
                    .current()
                    .and_then(|song| self.resolved.get(&song.id).cloned());
                self.snapshot.position_ms = 0;
                self.snapshot.duration_ms = self
                    .queue
                    .current()
                    .and_then(|song| song.duration)
                    .map(|seconds| seconds as u64 * 1000)
                    .unwrap_or(0);
                if sleeping {
                    // Pause the track mpv already started, not the finished one.
                    self.sleep = SleepTimer::Off;
                    self.send(AudioCommand::Pause);
                    self.snapshot.status = PlaybackStatus::Paused;
                    self.emit_snapshot();
                    self.emit_queue();
                    return;
                }
                self.snapshot.status = PlaybackStatus::Playing;
                self.emit_snapshot();
                self.emit_queue();
                self.prefetch_next();
            }
            Some(Preload::Prepared(_)) => {
                if sleeping {
                    // Match the gapless path: point the queue at the next
                    // track, but leave its promoted engine paused at 0:00 so
                    // pressing play resumes normally.
                    self.promote_prepared_paused();
                    return;
                }
                // No position tick landed inside the crossfade window (short
                // track or stalled clock): fall back to a hard cut.
                match self.queue.next() {
                    Some(_) => self.start_current(),
                    None => self.end_of_queue(),
                }
            }
            None => {
                if sleeping {
                    self.sleep = SleepTimer::Off;
                    self.send(AudioCommand::Pause);
                    self.snapshot.status = PlaybackStatus::Paused;
                    self.emit_snapshot();
                    return;
                }
                match self.queue.next() {
                    Some(_) => self.start_current(),
                    None => self.end_of_queue(),
                }
            }
        }
    }

    fn end_of_queue(&mut self) {
        self.current_url = None;
        self.send(AudioCommand::Stop);
        self.snapshot.status = PlaybackStatus::Ended;
        self.snapshot.position_ms = 0;
        self.emit_snapshot();
        self.outbox.push(ControllerEvent::EndOfQueue);
    }

    /// Start the audible overlap: promote the prepared inactive engine, move
    /// the queue pointer and begin the volume ramp. The outgoing engine keeps
    /// playing its tail on the (now inactive) channel pair.
    fn start_crossfade(&mut self) {
        let Some(slot) = self.inactive.take() else {
            return;
        };
        while slot.events.try_recv().is_ok() {}
        let _ = self.queue.next();
        let current = self.queue.current().cloned();
        let old = std::mem::replace(&mut self.active, slot);
        self.inactive = Some(old);
        self.preloaded = None;
        self.prefetch_pending = None;
        self.fade = Some(CrossfadeFade {
            started: Instant::now(),
            duration: self.crossfade,
        });
        let _ = self.active.commands.send(AudioCommand::SetVolume(0.0));
        let _ = self.active.commands.send(AudioCommand::Play);
        if let Some(song) = current {
            self.current_url = self.resolved.get(&song.id).cloned();
            self.snapshot.position_ms = 0;
            self.snapshot.duration_ms = song
                .duration
                .map(|seconds| seconds as u64 * 1000)
                .unwrap_or(0);
        }
        self.snapshot.status = PlaybackStatus::Playing;
        self.emit_snapshot();
        self.emit_queue();
    }

    /// Promote a prepared (paused, silent) engine without starting playback.
    /// Used by end-of-track sleep so the queue points at the next track while
    /// the UI shows Paused and resumes cleanly.
    fn promote_prepared_paused(&mut self) {
        let Some(slot) = self.inactive.take() else {
            return;
        };
        while slot.events.try_recv().is_ok() {}
        let _ = self.queue.next();
        let current = self.queue.current().cloned();
        let old = std::mem::replace(&mut self.active, slot);
        self.inactive = Some(old);
        self.preloaded = None;
        self.prefetch_pending = None;
        if let Some(slot) = &self.inactive {
            let _ = slot.commands.send(AudioCommand::Stop);
            let _ = slot.commands.send(AudioCommand::SetVolume(0.0));
        }
        let _ = self
            .active
            .commands
            .send(AudioCommand::SetVolume(self.snapshot.volume));
        if let Some(song) = current {
            self.current_url = self.resolved.get(&song.id).cloned();
            self.snapshot.position_ms = 0;
            self.snapshot.duration_ms = song
                .duration
                .map(|seconds| seconds as u64 * 1000)
                .unwrap_or(0);
        }
        self.sleep = SleepTimer::Off;
        self.snapshot.status = PlaybackStatus::Paused;
        self.emit_snapshot();
        self.emit_queue();
    }

    /// Start the crossfade when the outgoing track enters the fade window and
    /// the next track was prepared on the inactive engine.
    fn maybe_start_crossfade(&mut self) {
        if self.fade.is_some()
            || self.crossfade.is_zero()
            || self.inactive.is_none()
            || matches!(
                self.snapshot.status,
                PlaybackStatus::Paused
                    | PlaybackStatus::Stopped
                    | PlaybackStatus::Ended
                    | PlaybackStatus::Error
            )
        {
            return;
        }
        if matches!(self.sleep, SleepTimer::EndOfTrack | SleepTimer::FadeOut(_))
            || self.sleep_fade.is_some()
        {
            return;
        }
        let duration_ms = self.snapshot.duration_ms;
        let crossfade_ms = self.crossfade.as_millis().min(u64::MAX as u128) as u64;
        if duration_ms == 0
            || duration_ms <= crossfade_ms
            || self.snapshot.position_ms + crossfade_ms < duration_ms
        {
            return;
        }
        let Some(Preload::Prepared(prepared)) = self.preloaded.clone() else {
            return;
        };
        if self.peek_next_song().is_none_or(|next| next.id != prepared) {
            return;
        }
        self.start_crossfade();
    }

    /// Step the crossfade volume ramp. Android's curve: `fade_in = 1-(1-p)^2`,
    /// `fade_out = (1-p)^2`, scaled by the user's volume.
    fn tick_crossfade(&mut self) {
        let Some(fade) = self.fade.as_ref() else {
            return;
        };
        let progress = if fade.duration.is_zero() {
            1.0
        } else {
            (fade.started.elapsed().as_secs_f32() / fade.duration.as_secs_f32()).clamp(0.0, 1.0)
        };
        let user = self.snapshot.volume;
        let fade_in = 1.0 - (1.0 - progress) * (1.0 - progress);
        let fade_out = (1.0 - progress) * (1.0 - progress);
        let _ = self
            .active
            .commands
            .send(AudioCommand::SetVolume(user * fade_in));
        if let Some(slot) = &self.inactive {
            let _ = slot.commands.send(AudioCommand::SetVolume(user * fade_out));
        }
        if progress >= 1.0 {
            self.finish_crossfade(true);
        }
    }

    fn finish_crossfade(&mut self, prefetch: bool) {
        if self.fade.take().is_none() {
            return;
        }
        if let Some(slot) = &self.inactive {
            let _ = slot.commands.send(AudioCommand::Stop);
            let _ = slot.commands.send(AudioCommand::SetVolume(0.0));
        }
        let _ = self
            .active
            .commands
            .send(AudioCommand::SetVolume(self.snapshot.volume));
        if prefetch {
            self.prefetch_next();
        }
    }

    /// Hard-abort an in-progress crossfade (next/previous/stop/shuffle).
    fn abort_crossfade(&mut self) {
        if self.fade.take().is_none() {
            return;
        }
        if let Some(slot) = &self.inactive {
            let _ = slot.commands.send(AudioCommand::Stop);
            let _ = slot.commands.send(AudioCommand::SetVolume(0.0));
        }
        self.send(AudioCommand::SetVolume(self.snapshot.volume));
    }

    /// Step the sleep timer: fade ramp or deadline. Pauses the active engine
    /// and clears the timer when it fires.
    fn tick_sleep(&mut self) {
        if let Some(fade) = self.sleep_fade.as_ref() {
            let progress = if fade.duration.is_zero() {
                1.0
            } else {
                (fade.started.elapsed().as_secs_f32() / fade.duration.as_secs_f32()).clamp(0.0, 1.0)
            };
            self.send(AudioCommand::SetVolume(
                self.snapshot.volume * (1.0 - progress),
            ));
            if progress >= 1.0 {
                self.end_sleep();
                return;
            }
        }
        if let Some(deadline) = self.sleep_deadline {
            if Instant::now() >= deadline {
                self.end_sleep();
            }
        }
    }

    /// Pause and clear the timer. Restores the user volume if a fade lowered it.
    fn end_sleep(&mut self) {
        self.sleep_deadline = None;
        self.sleep = SleepTimer::Off;
        if self.sleep_fade.take().is_some() {
            self.send(AudioCommand::SetVolume(self.snapshot.volume));
        }
        self.send(AudioCommand::Pause);
        self.snapshot.status = PlaybackStatus::Paused;
        self.emit_snapshot();
    }

    /// Cancel a running `FadeOut` ramp (replacing/clearing the timer, stop).
    fn clear_sleep_fade(&mut self) {
        if self.sleep_fade.take().is_some() {
            self.send(AudioCommand::SetVolume(self.snapshot.volume));
            if matches!(self.sleep, SleepTimer::FadeOut(_)) {
                self.sleep = SleepTimer::Off;
            }
        }
    }

    fn handle_engine_event(&mut self, event: AudioEvent) {
        match event {
            AudioEvent::Snapshot(mut snapshot) => {
                // Fades drive the engine volume directly; the controller is the
                // only writer of `volume`, so keep the user's slider value
                // authoritative instead of echoing a ramp step back.
                snapshot.volume = self.snapshot.volume;
                snapshot.shuffle = self.queue.is_shuffled();
                snapshot.repeat = self.queue.repeat_mode();
                snapshot.queue_len = self.queue.len();
                snapshot.queue_index = if self.queue.is_empty() {
                    None
                } else {
                    Some(self.queue.position())
                };
                snapshot.current = self.queue.current().cloned();
                self.snapshot = snapshot;
                self.emit_snapshot();
            }
            AudioEvent::Position {
                position_ms,
                duration_ms,
            } => {
                self.snapshot.position_ms = position_ms;
                if duration_ms > 0 {
                    self.snapshot.duration_ms = duration_ms;
                }
                self.maybe_start_crossfade();
                self.emit_snapshot();
            }
            AudioEvent::Buffering(buffering) => {
                self.snapshot.buffering = buffering;
                if buffering {
                    self.snapshot.status = PlaybackStatus::Buffering;
                } else if self.snapshot.status == PlaybackStatus::Buffering {
                    self.snapshot.status = PlaybackStatus::Playing;
                }
                self.emit_snapshot();
            }
            AudioEvent::EndFile => self.handle_end_file(),
            AudioEvent::Error(message) => {
                self.snapshot.status = PlaybackStatus::Error;
                self.emit_snapshot();
                self.outbox.push(ControllerEvent::Error(message));
            }
            AudioEvent::DeviceChanged { id, fallback_from } => {
                self.outbox
                    .push(ControllerEvent::DeviceChanged { id, fallback_from });
            }
            AudioEvent::Path(_)
            | AudioEvent::AudioParams(_)
            | AudioEvent::Metadata(_)
            | AudioEvent::Devices(_) => {}
        }
    }

    fn send(&mut self, command: AudioCommand) {
        if self.active.commands.send(command).is_err() {
            self.outbox
                .push(ControllerEvent::Error("audio engine stopped".to_string()));
        }
    }

    /// Send the same command to the active and (when attached) inactive
    /// engine. Engine settings must not differ across a crossfade swap.
    fn send_both(&mut self, command: AudioCommand) {
        self.send(command.clone());
        if let Some(slot) = &self.inactive {
            let _ = slot.commands.send(command);
        }
    }

    fn filled(&self) -> PlayerSnapshot {
        let mut snapshot = self.snapshot.clone();
        snapshot.shuffle = self.queue.is_shuffled();
        snapshot.repeat = self.queue.repeat_mode();
        snapshot.queue_len = self.queue.len();
        snapshot.queue_index = if self.queue.is_empty() {
            None
        } else {
            Some(self.queue.position())
        };
        snapshot.current = self.queue.current().cloned();
        if snapshot.duration_ms == 0 {
            if let Some(duration) = snapshot.current.as_ref().and_then(|song| song.duration) {
                snapshot.duration_ms = duration as u64 * 1000;
            }
        }
        snapshot
    }

    fn emit_snapshot(&mut self) {
        self.outbox.push(ControllerEvent::Snapshot(self.filled()));
    }

    fn emit_queue(&mut self) {
        self.outbox
            .push(ControllerEvent::QueueChanged(QueueView::from_queue(
                &self.queue,
            )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Direct;

    impl StreamResolver for Direct {
        fn resolve(&self, video_id: &str) -> Option<String> {
            Some(format!("url:{video_id}"))
        }
    }

    struct Pending;

    impl StreamResolver for Pending {
        fn resolve(&self, _video_id: &str) -> Option<String> {
            None
        }
    }

    struct Harness {
        controller: PlaybackController,
        commands: Receiver<AudioCommand>,
        events: Sender<AudioEvent>,
        secondary_commands: Option<Receiver<AudioCommand>>,
        secondary_events: Option<Sender<AudioEvent>>,
    }

    impl Harness {
        fn new(resolver: Arc<dyn StreamResolver>) -> Self {
            let (command_tx, commands) = crossbeam_channel::unbounded();
            let (events, event_rx) = crossbeam_channel::unbounded();
            Self {
                controller: PlaybackController::new(command_tx, event_rx, resolver),
                commands,
                events,
                secondary_commands: None,
                secondary_events: None,
            }
        }

        /// Attach a fake secondary engine and return its channels.
        fn attach(&mut self) {
            let (command_tx, commands) = crossbeam_channel::unbounded();
            let (events, event_rx) = crossbeam_channel::unbounded();
            assert!(self
                .controller
                .attach_crossfade_engine(command_tx, event_rx));
            self.secondary_commands = Some(commands);
            self.secondary_events = Some(events);
        }

        fn commands(&self) -> Vec<AudioCommand> {
            drain(&self.commands)
        }

        fn secondary(&self) -> Vec<AudioCommand> {
            drain(
                self.secondary_commands
                    .as_ref()
                    .expect("secondary attached"),
            )
        }

        fn engine(&self, event: AudioEvent) {
            self.events.send(event).unwrap();
        }

        fn secondary_engine(&self, event: AudioEvent) {
            self.secondary_events
                .as_ref()
                .expect("secondary attached")
                .send(event)
                .unwrap();
        }
    }

    fn drain(receiver: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
        let mut commands = Vec::new();
        while let Ok(command) = receiver.try_recv() {
            commands.push(command);
        }
        commands
    }

    fn song(id: &str) -> Song {
        Song {
            id: id.to_string(),
            title: id.to_string(),
            duration: Some(120),
            ..Song::default()
        }
    }

    fn queue_of(ids: &[&str]) -> Queue {
        Queue::new(ids.iter().map(|id| song(id)).collect(), QueueSource::List)
    }

    fn album_song(id: &str, album: Option<&str>) -> Song {
        Song {
            id: id.to_string(),
            title: id.to_string(),
            duration: Some(120),
            album: album.map(|album| ml_core::AlbumRef {
                id: album.to_string(),
                title: album.to_string(),
            }),
            ..Song::default()
        }
    }

    fn prep(url: &str) -> Vec<AudioCommand> {
        vec![AudioCommand::Pause, AudioCommand::SetVolume(0.0), load(url)]
    }

    fn load(url: &str) -> AudioCommand {
        AudioCommand::LoadUrl {
            url: url.to_string(),
            start_ms: None,
        }
    }

    fn enqueue(url: &str) -> AudioCommand {
        AudioCommand::EnqueueNext {
            url: url.to_string(),
        }
    }

    #[test]
    fn play_queue_loads_current_and_preloads_next() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a", "b", "c"]), 0);
        assert_eq!(harness.commands(), vec![load("url:a"), enqueue("url:b")]);
        let events = harness.controller.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::QueueChanged(view)
                if view.position == Some(0) && view.songs.len() == 3
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, ControllerEvent::Snapshot(_))));
        assert_eq!(harness.controller.snapshot().current.unwrap().id, "a");
    }

    #[test]
    fn restore_queue_keeps_index_source_and_does_not_autoplay() {
        let mut queue = queue_of(&["a", "b", "c"]);
        queue.set_source(QueueSource::Radio);
        queue.set_repeat(RepeatMode::All);
        assert_eq!(queue.jump_to(2).map(|song| song.id.as_str()), Some("c"));
        let mut harness = Harness::new(Arc::new(Pending));
        harness.controller.restore_queue(queue);

        assert!(harness.commands().is_empty(), "restore must not load audio");
        let events = harness.controller.poll();
        assert!(!events
            .iter()
            .any(|event| matches!(event, ControllerEvent::NeedUrl { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::QueueChanged(view)
                if view.position == Some(2) && view.source == QueueSource::Radio
        )));
        let snapshot = harness.controller.snapshot();
        assert_eq!(snapshot.status, PlaybackStatus::Stopped);
        assert_eq!(snapshot.current.unwrap().id, "c");
        assert_eq!(snapshot.queue_index, Some(2));
        assert_eq!(snapshot.queue_len, 3);

        // The first transport action resolves the restored current track.
        harness.controller.play_pause();
        let events = harness.controller.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::NeedUrl { video_id, prefetch: false } if video_id == "c"
        )));
        assert!(harness.commands().is_empty());
    }

    #[test]
    fn append_queue_deduplicates_and_preserves_current_index() {
        let mut queue = queue_of(&["a", "b"]);
        assert!(queue.jump_to(1).is_some());
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue, 1);
        harness.commands();
        harness.controller.poll();

        let added =
            harness
                .controller
                .append_queue(vec![song("b"), song("c"), song("c"), song("d")]);
        assert_eq!(added, 2);
        assert_eq!(harness.controller.queue().position(), 1);
        assert_eq!(
            harness
                .controller
                .queue()
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c", "d"]
        );
        let snapshot = harness.controller.snapshot();
        assert_eq!(snapshot.current.unwrap().id, "b");
        assert_eq!(snapshot.queue_index, Some(1));
        assert_eq!(snapshot.queue_len, 4);
        let events = harness.controller.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::QueueChanged(view)
                if view.songs.len() == 4 && view.position == Some(1)
        )));
    }

    #[test]
    fn moving_the_current_item_updates_the_queue_pointer_and_order() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a", "b", "c"]), 1);
        harness.commands();
        harness.controller.poll();

        assert!(harness.controller.move_queue_item(1, 2));
        assert_eq!(harness.controller.queue().position(), 2);
        assert_eq!(
            harness
                .controller
                .queue()
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "c", "b"]
        );
        let snapshot = harness.controller.snapshot();
        assert_eq!(snapshot.current.unwrap().id, "b");
        assert_eq!(snapshot.queue_index, Some(2));
        assert!(!harness.controller.move_queue_item(0, 99));
        let events = harness.controller.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::QueueChanged(view) if view.position == Some(2)
        )));
    }

    #[test]
    fn removing_the_current_item_starts_the_successor_and_empty_stops() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a", "b", "c"]), 0);
        harness.commands();
        harness.controller.poll();

        let removed = harness.controller.remove_queue_item(0).unwrap();
        assert_eq!(removed.id, "a");
        assert_eq!(harness.controller.queue().position(), 0);
        assert_eq!(
            harness
                .controller
                .queue()
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert_eq!(harness.controller.snapshot().current.unwrap().id, "b");
        assert!(harness
            .commands()
            .iter()
            .any(|command| matches!(command, AudioCommand::LoadUrl { url, .. } if url == "url:b")));

        harness.controller.remove_queue_item(1).unwrap();
        harness.controller.remove_queue_item(0).unwrap();
        assert!(harness.controller.queue().is_empty());
        assert_eq!(
            harness.controller.snapshot().status,
            PlaybackStatus::Stopped
        );
        assert!(harness.controller.snapshot().current.is_none());
        assert!(harness
            .commands()
            .iter()
            .any(|command| matches!(command, AudioCommand::Stop)));
    }

    #[test]
    fn end_file_advances_gaplessly_then_ends_queue() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a", "b", "c"]), 0);
        harness.commands();

        harness.engine(AudioEvent::EndFile);
        let events = harness.controller.poll();
        assert_eq!(harness.commands(), vec![enqueue("url:c")]);
        assert_eq!(harness.controller.queue().position(), 1);
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::QueueChanged(view) if view.position == Some(1)
        )));
        assert_eq!(harness.controller.snapshot().current.unwrap().id, "b");

        harness.engine(AudioEvent::EndFile);
        harness.controller.poll();
        assert!(harness.commands().is_empty());
        assert_eq!(harness.controller.queue().position(), 2);

        harness.engine(AudioEvent::EndFile);
        let events = harness.controller.poll();
        assert!(events
            .iter()
            .any(|event| matches!(event, ControllerEvent::EndOfQueue)));
        assert!(harness.commands().contains(&AudioCommand::Stop));
        assert_eq!(harness.controller.snapshot().status, PlaybackStatus::Ended);
    }

    #[test]
    fn repeat_one_reloads_instead_of_preloading() {
        let mut queue = queue_of(&["a", "b"]);
        queue.set_repeat(RepeatMode::One);
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue, 0);
        assert_eq!(harness.commands(), vec![load("url:a")]);

        harness.engine(AudioEvent::EndFile);
        harness.controller.poll();
        assert_eq!(harness.commands(), vec![load("url:a")]);
        assert_eq!(harness.controller.queue().position(), 0);
    }

    #[test]
    fn end_of_queue_stops_engine() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a"]), 0);
        harness.commands();

        harness.engine(AudioEvent::EndFile);
        let events = harness.controller.poll();
        assert!(events
            .iter()
            .any(|event| matches!(event, ControllerEvent::EndOfQueue)));
        assert_eq!(harness.commands(), vec![AudioCommand::Stop]);
    }

    #[test]
    fn unresolved_urls_round_trip_through_need_url() {
        let mut harness = Harness::new(Arc::new(Pending));
        harness.controller.play_queue(queue_of(&["a", "b"]), 0);
        let events = harness.controller.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::NeedUrl { video_id, prefetch: false } if video_id == "a"
        )));
        assert!(harness.commands().is_empty());

        harness
            .controller
            .submit_resolved("a".to_string(), "url:a".to_string());
        assert_eq!(harness.commands(), vec![load("url:a")]);

        let events = harness.controller.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::NeedUrl { video_id, prefetch: true } if video_id == "b"
        )));
        harness
            .controller
            .submit_resolved("b".to_string(), "url:b".to_string());
        assert_eq!(harness.commands(), vec![enqueue("url:b")]);
    }

    #[test]
    fn sleep_timer_pauses_after_deadline() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a", "b"]), 0);
        harness.commands();
        harness.controller.set_sleep_timer(SleepTimer::Minutes(0));
        let events = harness.controller.poll();
        assert_eq!(harness.commands(), vec![AudioCommand::Pause]);
        assert_eq!(harness.controller.sleep_timer(), SleepTimer::Off);
        assert_eq!(harness.controller.snapshot().status, PlaybackStatus::Paused);
        assert!(!events.is_empty());
    }

    #[test]
    fn huge_sleep_timer_is_clamped_without_overflow() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a"]), 0);
        harness.commands();
        harness.controller.poll();
        harness
            .controller
            .set_sleep_timer(SleepTimer::Minutes(u32::MAX));
        assert_eq!(
            harness.controller.sleep_timer(),
            SleepTimer::Minutes(u32::MAX)
        );
        assert!(harness.controller.poll().is_empty());
        assert!(harness.commands().is_empty());
    }

    #[test]
    fn end_of_track_sleep_timer_stops_advancing() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a"]), 0);
        harness.commands();
        harness.controller.set_sleep_timer(SleepTimer::EndOfTrack);

        harness.engine(AudioEvent::EndFile);
        harness.controller.poll();
        assert_eq!(harness.commands(), vec![AudioCommand::Pause]);
        assert_eq!(harness.controller.queue().position(), 0);
        assert_eq!(harness.controller.sleep_timer(), SleepTimer::Off);
    }

    #[test]
    fn end_of_track_sleep_keeps_queue_in_sync_with_gapless_preload() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a", "b"]), 0);
        harness.commands();
        harness.controller.set_sleep_timer(SleepTimer::EndOfTrack);

        // mpv auto-advanced to the preloaded track before the pause lands.
        harness.engine(AudioEvent::EndFile);
        harness.controller.poll();
        assert_eq!(harness.commands(), vec![AudioCommand::Pause]);
        assert_eq!(harness.controller.queue().position(), 1);
        assert_eq!(harness.controller.snapshot().current.unwrap().id, "b");

        harness.controller.play_pause();
        assert_eq!(harness.commands(), vec![AudioCommand::Play]);
    }

    #[test]
    fn shuffle_and_repeat_rebuild_the_preloaded_playlist() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a", "b", "c"]), 0);
        harness.commands();
        harness.controller.poll();

        harness.controller.toggle_shuffle();
        let commands = harness.commands();
        assert!(matches!(
            commands.first(),
            Some(AudioCommand::LoadUrl {
                start_ms: Some(0),
                ..
            })
        ));
        assert!(harness.controller.queue().is_shuffled());
        assert_eq!(harness.controller.queue().position(), 0);
        assert!(harness.controller.poll().iter().any(|event| matches!(
            event,
            ControllerEvent::QueueChanged(view) if view.shuffle
        )));

        harness.controller.cycle_repeat();
        assert_eq!(harness.controller.queue().repeat_mode(), RepeatMode::All);
        assert!(matches!(
            harness.commands().first(),
            Some(AudioCommand::LoadUrl {
                start_ms: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn next_previous_and_seek_drive_the_engine() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a", "b"]), 0);
        harness.commands();

        harness.controller.next();
        assert_eq!(harness.commands(), vec![load("url:b")]);
        assert_eq!(harness.controller.queue().position(), 1);

        harness.controller.seek(30_000);
        assert_eq!(harness.commands(), vec![AudioCommand::SeekAbsolute(30_000)]);
        assert_eq!(harness.controller.snapshot().position_ms, 30_000);

        harness.controller.previous();
        assert_eq!(harness.commands().first(), Some(&load("url:a")));
        assert_eq!(harness.controller.queue().position(), 0);
    }

    #[test]
    fn crossfade_prepares_next_on_secondary_and_swaps() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.set_crossfade(Duration::from_millis(50));
        harness.attach();
        harness.controller.play_queue(queue_of(&["a", "b", "c"]), 0);
        assert_eq!(harness.commands(), vec![load("url:a")]);
        assert_eq!(harness.secondary(), prep("url:b"));
        harness.controller.poll();
        assert!(!harness.controller.is_crossfading());

        harness.engine(AudioEvent::Position {
            position_ms: 119_960,
            duration_ms: 120_000,
        });
        harness.controller.poll();
        assert!(harness.controller.is_crossfading());
        assert_eq!(harness.controller.queue().position(), 1);
        assert_eq!(harness.controller.snapshot().current.unwrap().id, "b");
        // The promoted engine starts silent and is ramped up by the fade.
        assert_eq!(
            harness.secondary(),
            vec![AudioCommand::SetVolume(0.0), AudioCommand::Play]
        );
        // The outgoing engine is ramped down, not stopped yet.
        assert!(!harness.commands().contains(&AudioCommand::Stop));

        std::thread::sleep(Duration::from_millis(70));
        harness.controller.poll();
        assert!(!harness.controller.is_crossfading());
        let old = harness.commands();
        assert!(old.contains(&AudioCommand::Stop));
        assert!(old.contains(&AudioCommand::SetVolume(0.0)));
        let now_active = harness.secondary();
        assert!(now_active.contains(&AudioCommand::SetVolume(1.0)));
    }

    #[test]
    fn crossfade_duration_is_clamped_and_zero_disables() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.attach();
        harness.controller.set_crossfade(Duration::from_secs(90));
        assert_eq!(harness.controller.crossfade(), (MAX_CROSSFADE, true));
        harness.controller.set_crossfade(Duration::ZERO);
        assert_eq!(harness.controller.crossfade(), (Duration::ZERO, true));
    }

    #[test]
    fn crossfade_gapless_keeps_same_album_transitions_appended() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.set_crossfade(Duration::from_millis(500));
        harness.attach();
        let queue = Queue::new(
            vec![
                album_song("a", Some("album")),
                album_song("b", Some("album")),
            ],
            QueueSource::List,
        );
        harness.controller.play_queue(queue, 0);
        assert_eq!(harness.commands(), vec![load("url:a"), enqueue("url:b")]);
        assert!(harness.secondary().is_empty());

        // Disabling the gapless option crossfades the same-album transition.
        harness.controller.set_crossfade_gapless(false);
        assert_eq!(harness.secondary(), prep("url:b"));
    }

    #[test]
    fn crossfade_is_not_prepared_for_end_of_track_sleep() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.set_crossfade(Duration::from_millis(500));
        harness.attach();
        harness.controller.set_sleep_timer(SleepTimer::EndOfTrack);
        harness.controller.play_queue(queue_of(&["a", "b"]), 0);
        assert_eq!(harness.commands(), vec![load("url:a"), enqueue("url:b")]);
        assert!(harness.secondary().is_empty());
    }

    #[test]
    fn prepared_crossfade_falls_back_to_hard_cut_at_end_file() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.set_crossfade(Duration::from_secs(5));
        harness.attach();
        harness.controller.play_queue(queue_of(&["a", "b"]), 0);
        harness.commands();
        harness.secondary();

        // No position tick landed in the window.
        harness.engine(AudioEvent::EndFile);
        let events = harness.controller.poll();
        assert_eq!(harness.commands(), vec![load("url:b")]);
        assert_eq!(harness.controller.queue().position(), 1);
        assert!(events
            .iter()
            .any(|event| matches!(event, ControllerEvent::QueueChanged(_))));
    }

    #[test]
    fn end_of_track_sleep_promotes_a_prepared_crossfade_paused() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.set_crossfade(Duration::from_secs(5));
        harness.attach();
        harness.controller.play_queue(queue_of(&["a", "b"]), 0);
        harness.commands();
        assert_eq!(harness.secondary(), prep("url:b"));
        // Arm the timer after the next track was prepared on the inactive engine.
        harness.controller.set_sleep_timer(SleepTimer::EndOfTrack);

        harness.engine(AudioEvent::EndFile);
        let events = harness.controller.poll();
        assert_eq!(harness.controller.queue().position(), 1);
        assert_eq!(harness.controller.snapshot().current.unwrap().id, "b");
        assert_eq!(harness.controller.snapshot().status, PlaybackStatus::Paused);
        assert_eq!(harness.controller.sleep_timer(), SleepTimer::Off);
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::QueueChanged(view) if view.position == Some(1)
        )));
        // Promoted engine is paused at 0 with the user volume; the old one stopped.
        assert_eq!(harness.secondary(), vec![AudioCommand::SetVolume(1.0)]);
        assert!(harness.commands().contains(&AudioCommand::Stop));

        harness.controller.play_pause();
        assert!(harness.secondary().contains(&AudioCommand::Play));
    }

    #[test]
    fn next_during_crossfade_stops_the_outgoing_engine() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.set_crossfade(Duration::from_secs(2));
        harness.attach();
        harness.controller.play_queue(queue_of(&["a", "b", "c"]), 0);
        harness.commands();
        harness.secondary();
        harness.controller.poll();
        harness.engine(AudioEvent::Position {
            position_ms: 118_500,
            duration_ms: 120_000,
        });
        harness.controller.poll();
        assert!(harness.controller.is_crossfading());
        harness.commands();
        harness.secondary();

        harness.controller.next();
        let old_engine = harness.commands();
        assert!(old_engine.contains(&AudioCommand::Stop));
        assert_eq!(
            harness.secondary(),
            vec![AudioCommand::SetVolume(1.0), load("url:c")]
        );
        assert!(!harness.controller.is_crossfading());
    }

    #[test]
    fn snapshot_volume_stays_at_the_user_value_during_crossfade() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.set_crossfade(Duration::from_secs(2));
        harness.attach();
        harness.controller.play_queue(queue_of(&["a", "b"]), 0);
        harness.commands();
        harness.secondary();
        harness.controller.set_volume(0.8);
        harness.commands();
        harness.engine(AudioEvent::Position {
            position_ms: 118_500,
            duration_ms: 120_000,
        });
        harness.controller.poll();
        harness.secondary_engine(AudioEvent::Snapshot(ml_core::PlayerSnapshot {
            status: PlaybackStatus::Playing,
            volume: 0.0,
            ..ml_core::PlayerSnapshot::default()
        }));
        harness.controller.poll();
        assert_eq!(harness.controller.snapshot().volume, 0.8);
    }

    #[test]
    fn engine_volume_does_not_clobber_the_user_value() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a"]), 0);
        harness.commands();
        harness.controller.set_volume(0.7);
        harness.engine(AudioEvent::Snapshot(ml_core::PlayerSnapshot {
            status: PlaybackStatus::Playing,
            volume: 0.2,
            ..ml_core::PlayerSnapshot::default()
        }));
        harness.controller.poll();
        assert_eq!(harness.controller.snapshot().volume, 0.7);
    }

    #[test]
    fn attaching_a_second_crossfade_engine_is_rejected() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.attach();
        let (command_tx, _commands) = crossbeam_channel::unbounded();
        let (_events, event_rx) = crossbeam_channel::unbounded();
        assert!(!harness
            .controller
            .attach_crossfade_engine(command_tx, event_rx));
    }

    #[test]
    fn sleep_fade_out_ramps_and_pauses() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a"]), 0);
        harness.commands();
        harness
            .controller
            .set_sleep_timer(SleepTimer::FadeOut(Duration::from_millis(60)));
        harness.controller.poll();

        std::thread::sleep(Duration::from_millis(80));
        let events = harness.controller.poll();
        let commands = harness.commands();
        assert!(commands
            .iter()
            .any(|command| matches!(command, AudioCommand::SetVolume(volume) if *volume < 0.5)));
        assert!(commands.contains(&AudioCommand::Pause));
        assert_eq!(harness.controller.sleep_timer(), SleepTimer::Off);
        assert_eq!(harness.controller.snapshot().status, PlaybackStatus::Paused);
        assert_eq!(harness.controller.snapshot().volume, 1.0);
        assert!(events
            .iter()
            .any(|event| matches!(event, ControllerEvent::Snapshot(_))));
    }

    #[test]
    fn replacing_a_fade_out_timer_restores_volume() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.play_queue(queue_of(&["a"]), 0);
        harness.commands();
        harness
            .controller
            .set_sleep_timer(SleepTimer::FadeOut(Duration::from_secs(10)));
        harness.controller.poll();
        harness.commands();

        harness.controller.set_sleep_timer(SleepTimer::Minutes(0));
        harness.controller.poll();
        let commands = harness.commands();
        assert!(commands.contains(&AudioCommand::SetVolume(1.0)));
        assert!(commands.contains(&AudioCommand::Pause));
        assert_eq!(harness.controller.sleep_timer(), SleepTimer::Off);
    }

    #[test]
    fn scheduled_sleep_window_wraps_overnight_and_arms_minutes() {
        assert!(sleep_window_contains(22 * 60 + 30, 22 * 60, 6 * 60));
        assert!(sleep_window_contains(5 * 60, 22 * 60, 6 * 60));
        assert!(!sleep_window_contains(12 * 60, 22 * 60, 6 * 60));
        assert!(sleep_window_contains(9 * 60, 9 * 60, 17 * 60));
        assert!(!sleep_window_contains(17 * 60, 9 * 60, 17 * 60));
        assert!(sleep_window_contains(0, 23 * 60, 23 * 60));

        let mut harness = Harness::new(Arc::new(Direct));
        assert!(harness
            .controller
            .arm_scheduled_sleep_timer(9 * 60 + 15, 9 * 60, 23 * 60, 30));
        assert_eq!(harness.controller.sleep_timer(), SleepTimer::Minutes(30));
        assert!(!harness
            .controller
            .arm_scheduled_sleep_timer(8 * 60, 9 * 60, 23 * 60, 30));
    }

    #[test]
    fn engine_settings_fan_out_to_both_engines() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.attach();
        harness.controller.set_normalization(true, Some(-6.5));
        harness.controller.set_eq(&[1.0; 10]);
        harness.controller.set_speed(1.5);
        harness.controller.set_skip_silence(true);
        harness.controller.set_muted(true);

        let active = harness.commands();
        let inactive = harness.secondary();
        assert_eq!(active, inactive);
        assert!(active.contains(&AudioCommand::SetGainDb(-6.5)));
        assert!(active.contains(&AudioCommand::SetNormalization(true)));
        assert!(active.contains(&AudioCommand::SetSpeed(1.5)));
        assert!(active.contains(&AudioCommand::SetSkipSilence(true)));
        assert!(active.contains(&AudioCommand::SetMuted(true)));
    }

    #[test]
    fn device_fallback_is_forwarded_to_the_ui() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.engine(AudioEvent::DeviceChanged {
            id: "auto".to_string(),
            fallback_from: Some("pipewire/gone".to_string()),
        });
        let events = harness.controller.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            ControllerEvent::DeviceChanged { id, fallback_from: Some(from) }
                if id == "auto" && from == "pipewire/gone"
        )));
    }

    #[test]
    fn normalization_without_gain_disables_the_stage() {
        let mut harness = Harness::new(Arc::new(Direct));
        harness.controller.set_normalization(true, None);
        assert_eq!(
            harness.commands(),
            vec![
                AudioCommand::SetGainDb(0.0),
                AudioCommand::SetNormalization(true)
            ]
        );
        harness.controller.set_normalization(false, Some(f32::NAN));
        assert_eq!(
            harness.commands(),
            vec![
                AudioCommand::SetGainDb(0.0),
                AudioCommand::SetNormalization(false)
            ]
        );
    }
}
