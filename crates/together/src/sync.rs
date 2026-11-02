//! Pure playback/queue synchronisation logic.
//!
//! Everything here is deterministic and total: it only depends on the
//! arguments, never on wall clocks or I/O. `RoomSession` uses these helpers to
//! fold server events into local state.

use std::cmp::max;

use ml_core::Queue;

use crate::clock::ServerClock;
use crate::messages::action;
use crate::proto;

/// Android `SOFT_SYNC_THRESHOLD_MS`: below this drift is ignored.
pub const SOFT_SYNC_THRESHOLD_MS: i64 = 50;
/// Android `HARD_SYNC_THRESHOLD_MS`: at or above this drift we seek.
pub const HARD_SYNC_THRESHOLD_MS: i64 = 750;
/// Android `DRIFT_CORRECTION_SPEED`.
pub const DRIFT_CORRECTION_SPEED: f32 = 0.02;
/// Android `DRIFT_CHECK_INTERVAL_MS`.
pub const DRIFT_CHECK_INTERVAL_MS: u64 = 250;
/// Android `ACTIVE_PLAYBACK_SYNC_TOLERANCE_MS`.
pub const ACTIVE_PLAYBACK_SYNC_TOLERANCE_MS: i64 = 2_000;
/// Host heartbeat interval (Android `startHeartbeat`).
pub const HOST_HEARTBEAT_INTERVAL_MS: u64 = 8_000;

pub const MAX_RECONNECT_ATTEMPTS: u32 = 15;
pub const INITIAL_RECONNECT_DELAY_MS: u64 = 1_000;
pub const MAX_RECONNECT_DELAY_MS: u64 = 120_000;

/// `[current] + upcoming`, current first, upcoming de-duplicated and without
/// the current track. Port of `canonicalPlaybackQueue`.
pub fn canonical_playback_queue(
    current: &proto::TrackInfo,
    upcoming: &[proto::TrackInfo],
) -> Vec<proto::TrackInfo> {
    let mut queue = Vec::with_capacity(upcoming.len() + 1);
    queue.push(current.clone());
    for track in upcoming {
        if track.id != current.id && !queue.iter().any(|queued| queued.id == track.id) {
            queue.push(track.clone());
        }
    }
    queue
}

/// Items after the current index. Port of `upcomingQueueItems`; `None`
/// corresponds to Kotlin's `-1` / out-of-range index.
pub fn upcoming_queue_items<T: Clone>(queue: &[T], current_index: Option<usize>) -> Vec<T> {
    match current_index {
        Some(index) if index < queue.len() => queue[index + 1..].to_vec(),
        _ => Vec::new(),
    }
}

/// Port of `shouldSeekDuringActivePlayback`: the 8-second host heartbeat must
/// not interrupt playback for ordinary drift, only a hard (> 2 s) mismatch.
pub fn should_seek_during_active_playback(
    position_difference_ms: i64,
    playback_ready: bool,
) -> bool {
    playback_ready && position_difference_ms > ACTIVE_PLAYBACK_SYNC_TOLERANCE_MS
}

/// What the drift corrector should do next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DriftDecision {
    /// Within tolerance or not playing: leave the player alone.
    Hold,
    /// Drift too large to hide: seek to this position.
    Seek(i64),
    /// Play at this speed multiplier until the next check.
    Speed(f32),
}

/// Port of the `startDriftCorrection` loop body for one iteration.
pub fn drift_decision(
    target_position_ms: i64,
    current_position_ms: i64,
    playback_ready: bool,
) -> DriftDecision {
    if !playback_ready {
        return DriftDecision::Hold;
    }
    let drift = target_position_ms - current_position_ms;
    let absolute = drift.unsigned_abs();
    if absolute <= SOFT_SYNC_THRESHOLD_MS as u64 {
        DriftDecision::Hold
    } else if absolute >= HARD_SYNC_THRESHOLD_MS as u64 {
        DriftDecision::Seek(target_position_ms)
    } else if drift > 0 {
        DriftDecision::Speed(1.0 + DRIFT_CORRECTION_SPEED)
    } else {
        DriftDecision::Speed(1.0 - DRIFT_CORRECTION_SPEED)
    }
}

/// Extrapolate a paused/playing position between two server timestamps.
pub fn position_at(
    position: i64,
    effective_at_server_time: Option<i64>,
    is_playing: bool,
    server_now: Option<i64>,
) -> i64 {
    let Some(effective_at) = effective_at_server_time.filter(|at| *at > 0) else {
        return position;
    };
    let Some(server_now) = server_now.filter(|_| is_playing) else {
        return position;
    };
    position.saturating_add(server_now.saturating_sub(effective_at).max(0))
}

/// Monotonic revision gate. Revision 0 means "unversioned" and is always
/// accepted; otherwise stale revisions are dropped, equal ones accepted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RevisionGate {
    last: u64,
}

impl RevisionGate {
    pub fn last(&self) -> u64 {
        self.last
    }

    pub fn reset(&mut self) {
        self.last = 0;
    }

    /// Force the gate to a known revision (join/reconnect snapshots).
    pub fn set(&mut self, revision: u64) {
        self.last = revision;
    }

    pub fn accept(&mut self, revision: u64) -> bool {
        if revision == 0 {
            return true;
        }
        if revision < self.last {
            return false;
        }
        self.last = max(self.last, revision);
        true
    }
}

/// Host-authoritative playback view derived from a room state.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackSnapshot {
    pub track: Option<proto::TrackInfo>,
    pub is_playing: bool,
    pub position: i64,
    pub effective_at_server_time: Option<i64>,
    pub revision: u64,
}

impl PlaybackSnapshot {
    pub fn from_room_state(state: &proto::RoomState) -> Self {
        Self {
            track: state.current_track.clone(),
            is_playing: state.is_playing,
            position: state.position.max(0),
            effective_at_server_time: (state.last_update > 0).then_some(state.last_update),
            revision: state.revision,
        }
    }

    pub fn position_at(&self, clock: &ServerClock, elapsed_realtime: i64) -> i64 {
        clock.position_at(
            self.position,
            self.effective_at_server_time,
            self.is_playing,
            elapsed_realtime,
        )
    }
}

/// Buffer-wait bookkeeping for the current track.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Buffering {
    waiting_for: Vec<String>,
}

impl Buffering {
    pub fn waiting_for(&self) -> &[String] {
        &self.waiting_for
    }

    pub fn is_waiting(&self) -> bool {
        !self.waiting_for.is_empty()
    }

    /// `buffer_wait`: replaces the waiting set (may be empty).
    pub fn on_wait(&mut self, waiting_for: Vec<String>) {
        self.waiting_for = waiting_for;
    }

    /// `buffer_complete`: clears the set; returns whether we were waiting.
    pub fn on_complete(&mut self) -> bool {
        let was_waiting = self.is_waiting();
        self.waiting_for.clear();
        was_waiting
    }

    pub fn clear(&mut self) {
        self.waiting_for.clear();
    }
}

/// A playback state held back until every guest signalled `buffer_complete`.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingSync {
    pub track_id: Option<String>,
    pub is_playing: bool,
    pub position: i64,
    pub effective_at_server_time: Option<i64>,
}

impl PendingSync {
    pub fn from_sync_state(payload: &proto::SyncStatePayload) -> Self {
        Self {
            track_id: payload.current_track.as_ref().map(|track| track.id.clone()),
            is_playing: payload.is_playing,
            position: payload.position,
            effective_at_server_time: (payload.last_update > 0).then_some(payload.last_update),
        }
    }

    pub fn from_playback(
        track_id: Option<&str>,
        is_playing: bool,
        position: i64,
        effective_at_server_time: Option<i64>,
    ) -> Self {
        Self {
            track_id: track_id.map(str::to_owned),
            is_playing,
            position,
            effective_at_server_time,
        }
    }

    /// Port of `applyPendingSyncIfReady`: the sync may only be applied once the
    /// buffer-complete message named this pending track.
    pub fn ready(
        &self,
        buffering_track_id: Option<&str>,
        completed_track_id: Option<&str>,
    ) -> bool {
        let track_id = match self.track_id.as_deref().or(buffering_track_id) {
            Some(track_id) => track_id,
            None => return false,
        };
        completed_track_id == Some(track_id)
    }
}

/// Fold one `sync_playback` payload into a room state, mirroring the Android
/// client's `handleMessage` branch.
pub fn apply_playback_action(
    state: &proto::RoomState,
    payload: &proto::PlaybackActionPayload,
) -> proto::RoomState {
    let mut next = state.clone();
    let revision = max(state.revision, payload.revision);

    match payload.action.as_str() {
        action::PLAY => {
            next.is_playing = true;
            next.position = payload.position_or_none().unwrap_or(state.position);
            next.last_update = payload.server_time_or_none().unwrap_or(state.last_update);
        }
        action::PAUSE => {
            next.is_playing = false;
            next.position = payload.position_or_none().unwrap_or(state.position);
            next.last_update = payload.server_time_or_none().unwrap_or(state.last_update);
        }
        action::SEEK => {
            next.position = payload.position_or_none().unwrap_or(state.position);
            next.last_update = payload.server_time_or_none().unwrap_or(state.last_update);
        }
        action::CHANGE_TRACK => {
            next.current_track = payload.track_info.clone();
            next.is_playing = false;
            next.position = 0;
            next.last_update = payload.server_time_or_none().unwrap_or(state.last_update);
            if payload.revision > 0 {
                next.queue = payload.queue.clone();
            }
        }
        action::QUEUE_ADD => {
            if let Some(track) = payload.track_info.clone() {
                next.queue = if payload.revision > 0 {
                    payload.queue.clone()
                } else if payload.insert_next_or_none().is_some() {
                    let mut queue = Vec::with_capacity(state.queue.len() + 1);
                    queue.push(track);
                    queue.extend(state.queue.iter().cloned());
                    queue
                } else {
                    let mut queue = state.queue.clone();
                    queue.push(track);
                    queue
                };
            }
        }
        action::QUEUE_REMOVE => {
            if let Some(track_id) = payload.track_id_or_none() {
                next.queue = if payload.revision > 0 {
                    payload.queue.clone()
                } else {
                    state
                        .queue
                        .iter()
                        .filter(|track| track.id != track_id)
                        .cloned()
                        .collect()
                };
            }
        }
        action::QUEUE_CLEAR => {
            next.queue = if payload.revision > 0 {
                payload.queue.clone()
            } else {
                Vec::new()
            };
        }
        action::SYNC_QUEUE => {
            next.queue = payload.queue.clone();
        }
        action::SET_VOLUME => {
            if let Some(volume) = payload.volume_or_none() {
                next.volume = volume.clamp(0.0, 1.0);
            }
        }
        _ => {}
    }

    next.revision = revision;
    next
}

/// Fold a `sync_state` payload into a room state.
pub fn apply_sync_state(
    state: &proto::RoomState,
    payload: &proto::SyncStatePayload,
) -> proto::RoomState {
    let mut next = state.clone();
    next.current_track = payload.current_track.clone();
    next.is_playing = payload.is_playing;
    next.position = payload.position;
    next.last_update = payload.last_update;
    next.volume = payload.volume;
    next.queue = payload.queue.clone();
    next.revision = max(state.revision, payload.revision);
    next
}

/// Move the host flag to `new_host_id` in a room state.
pub fn apply_host_change(state: &mut proto::RoomState, new_host_id: &str) {
    state.host_id = new_host_id.to_string();
    for user in &mut state.users {
        user.is_host = user.user_id == new_host_id;
    }
}

/// Rebuild a local playback queue from a room snapshot plus its upcoming
/// track list, preserving the current track at index 0.
pub fn queue_from_room(current: Option<&proto::TrackInfo>, upcoming: &[proto::TrackInfo]) -> Queue {
    let mut songs = Vec::new();
    if let Some(current) = current {
        for track in canonical_playback_queue(current, upcoming) {
            songs.push(crate::model::track_info_to_song(&track));
        }
    } else {
        for track in upcoming {
            songs.push(crate::model::track_info_to_song(track));
        }
    }
    Queue::new(songs, ml_core::QueueSource::List)
}

/// The local queue as the host advertises it: current track plus everything
/// queued after it, excluding playback history.
pub fn queue_to_room(queue: &Queue) -> (Option<proto::TrackInfo>, Vec<proto::TrackInfo>) {
    let current = queue.current().map(crate::model::song_to_track_info);
    let upcoming = queue
        .iter()
        .skip(queue.position() + 1)
        .map(crate::model::song_to_track_info)
        .collect();
    (current, upcoming)
}

/// Exponential reconnect delay with up to 20 % jitter.
///
/// `jitter_sample` must be in `[0, 1)`. The exponent is capped at four doublings,
/// exactly like `calculateBackoffDelay` (`2 shl min(attempt - 1, 4)`), so the
/// maximum is 32 s even though the nominal cap is 120 s.
pub fn backoff_delay_ms(attempt: u32, jitter_sample: f64) -> u64 {
    let shift = attempt.saturating_sub(1).min(4);
    let exponential = INITIAL_RECONNECT_DELAY_MS.saturating_mul(2u64 << shift);
    let capped = exponential.min(MAX_RECONNECT_DELAY_MS);
    let jitter = (capped as f64 * 0.2 * jitter_sample.clamp(0.0, 1.0)) as u64;
    capped + jitter
}

pub fn should_reconnect(attempts: u32) -> bool {
    attempts < MAX_RECONNECT_ATTEMPTS
}

/// Actions that carry a host-side `captured_at_server_time` for guests.
pub fn requires_server_capture(action: &str) -> bool {
    matches!(action, action::PLAY | action::PAUSE | action::SEEK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{msg, ServerMessage};

    fn track(id: &str) -> proto::TrackInfo {
        proto::TrackInfo {
            id: id.to_string(),
            title: id.to_string(),
            artist: "Artist".to_string(),
            duration: 1,
            ..Default::default()
        }
    }

    fn room_state(id: &str) -> proto::RoomState {
        proto::RoomState {
            room_code: "ROOM".to_string(),
            host_id: "host".to_string(),
            current_track: Some(track(id)),
            queue: vec![track("next")],
            revision: 1,
            ..Default::default()
        }
    }

    #[test]
    fn upcoming_queue_never_replaces_current_track() {
        let current = track("current");
        let suggested = track("suggested");
        let later = track("later");

        let queue = canonical_playback_queue(&current, &[suggested, later]);

        assert_eq!(
            queue
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            ["current", "suggested", "later"]
        );
    }

    #[test]
    fn canonical_queue_removes_current_and_duplicate_upcoming_entries() {
        let current = track("current");
        let next = track("next");

        let queue = canonical_playback_queue(&current, &[current.clone(), next.clone(), next]);

        assert_eq!(
            queue
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            ["current", "next"]
        );
    }

    #[test]
    fn outgoing_queue_excludes_playback_history_and_current_item() {
        let queue = ["played", "current", "suggested", "later"];
        assert_eq!(
            upcoming_queue_items(&queue, Some(1)),
            ["suggested", "later"]
        );
        assert!(upcoming_queue_items::<&str>(&queue, None).is_empty());
        assert!(upcoming_queue_items(&queue, Some(4)).is_empty());
        assert!(upcoming_queue_items(&queue, Some(usize::MAX)).is_empty());
    }

    #[test]
    fn heartbeat_drift_does_not_interrupt_active_playback() {
        for difference in [0, 50, 750, 1_999, 2_000] {
            assert!(!should_seek_during_active_playback(difference, true));
        }
        assert!(!should_seek_during_active_playback(10_000, false));
        assert!(should_seek_during_active_playback(2_001, true));
        // Negative differences cannot occur (abs is taken), but be total.
        assert!(!should_seek_during_active_playback(i64::MIN, true));
    }

    #[test]
    fn drift_decision_ports_soft_and_hard_thresholds() {
        assert_eq!(drift_decision(1_000, 960, true), DriftDecision::Hold);
        assert_eq!(
            drift_decision(1_000, 900, true),
            DriftDecision::Speed(1.0 + DRIFT_CORRECTION_SPEED)
        );
        assert_eq!(
            drift_decision(900, 1_000, true),
            DriftDecision::Speed(1.0 - DRIFT_CORRECTION_SPEED)
        );
        assert_eq!(
            drift_decision(1_750, 1_000, true),
            DriftDecision::Seek(1_750)
        );
        assert_eq!(drift_decision(1_000, 1_000, false), DriftDecision::Hold);
    }

    #[test]
    fn position_at_requires_playback_and_anchor() {
        assert_eq!(position_at(500, Some(10_000), true, Some(10_565)), 1_065);
        assert_eq!(position_at(500, Some(10_000), false, Some(10_565)), 500);
        assert_eq!(position_at(500, Some(0), true, Some(10_565)), 500);
        assert_eq!(position_at(500, None, true, Some(10_565)), 500);
        assert_eq!(position_at(500, Some(10_000), true, None), 500);
        assert_eq!(position_at(500, Some(10_000), true, Some(9_000)), 500);
        // Totality: extreme timestamps saturate instead of overflowing.
        assert_eq!(
            position_at(i64::MAX - 5, Some(1), true, Some(i64::MAX)),
            i64::MAX
        );
        assert_eq!(position_at(10, Some(i64::MAX), true, Some(1)), 10);
    }

    #[test]
    fn revision_gate_accepts_equal_and_zero_but_rejects_stale() {
        let mut gate = RevisionGate::default();
        assert!(gate.accept(0));
        assert!(gate.accept(5));
        assert_eq!(gate.last(), 5);
        assert!(gate.accept(5));
        assert!(!gate.accept(4));
        assert!(gate.accept(6));
        assert_eq!(gate.last(), 6);
        assert!(gate.accept(0));
        assert_eq!(gate.last(), 6);

        gate.set(9);
        assert!(!gate.accept(8));
        gate.reset();
        assert_eq!(gate.last(), 0);
    }

    #[test]
    fn playback_actions_update_room_state_like_the_android_client() {
        let state = room_state("current");

        let play = proto::PlaybackActionPayload {
            action: action::PLAY.to_string(),
            position: 1_234,
            server_time: 9_000,
            revision: 2,
            ..Default::default()
        };
        let next = apply_playback_action(&state, &play);
        assert!(next.is_playing);
        assert_eq!(next.position, 1_234);
        assert_eq!(next.last_update, 9_000);
        assert_eq!(next.revision, 2);

        let pause = proto::PlaybackActionPayload {
            action: action::PAUSE.to_string(),
            position: 0,
            revision: 2,
            ..Default::default()
        };
        let next = apply_playback_action(&next, &pause);
        assert!(!next.is_playing);
        assert_eq!(next.position, 0, "pause always carries a position");

        let seek_without_position = proto::PlaybackActionPayload {
            action: action::SKIP_NEXT.to_string(),
            revision: 3,
            ..Default::default()
        };
        let next = apply_playback_action(&next, &seek_without_position);
        assert_eq!(
            next.position, 0,
            "other actions only carry nonzero positions"
        );
        assert_eq!(next.revision, 3);
    }

    #[test]
    fn queue_actions_respect_revision_authority() {
        let state = room_state("current");

        // queue_add without revision and insert_next prepends.
        let add = proto::PlaybackActionPayload {
            action: action::QUEUE_ADD.to_string(),
            track_info: Some(track("suggested")),
            insert_next: true,
            ..Default::default()
        };
        let next = apply_playback_action(&state, &add);
        assert_eq!(
            next.queue
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            ["suggested", "next"]
        );

        // queue_add with revision replaces the authoritative queue.
        let add_versioned = proto::PlaybackActionPayload {
            action: action::QUEUE_ADD.to_string(),
            track_info: Some(track("ignored")),
            revision: 7,
            queue: vec![track("a"), track("b")],
            ..Default::default()
        };
        let next = apply_playback_action(&next, &add_versioned);
        assert_eq!(
            next.queue
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(next.revision, 7);

        // Unversioned remove filters by id; versioned remove replaces.
        let remove = proto::PlaybackActionPayload {
            action: action::QUEUE_REMOVE.to_string(),
            track_id: "a".to_string(),
            ..Default::default()
        };
        let next = apply_playback_action(&next, &remove);
        assert_eq!(
            next.queue
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            ["b"]
        );

        let clear = proto::PlaybackActionPayload {
            action: action::QUEUE_CLEAR.to_string(),
            revision: 8,
            queue: vec![track("c")],
            ..Default::default()
        };
        let next = apply_playback_action(&next, &clear);
        assert_eq!(next.queue.len(), 1);
        assert_eq!(next.queue[0].id, "c");

        let sync_queue = proto::PlaybackActionPayload {
            action: action::SYNC_QUEUE.to_string(),
            queue: vec![track("x"), track("y")],
            ..Default::default()
        };
        let next = apply_playback_action(&next, &sync_queue);
        assert_eq!(next.queue.len(), 2);
    }

    #[test]
    fn change_track_clears_playback_and_takes_the_queue_only_when_versioned() {
        let state = room_state("current");
        let change = proto::PlaybackActionPayload {
            action: action::CHANGE_TRACK.to_string(),
            track_info: Some(track("new")),
            revision: 4,
            queue: vec![track("q1")],
            server_time: 5_000,
            ..Default::default()
        };
        let next = apply_playback_action(&state, &change);
        assert_eq!(next.current_track.as_ref().unwrap().id, "new");
        assert!(!next.is_playing);
        assert_eq!(next.position, 0);
        assert_eq!(next.queue.len(), 1);
        assert_eq!(next.last_update, 5_000);

        // Unversioned change keeps the existing queue.
        let unversioned = proto::PlaybackActionPayload {
            action: action::CHANGE_TRACK.to_string(),
            track_info: None,
            ..Default::default()
        };
        let next = apply_playback_action(&state, &unversioned);
        assert!(
            next.current_track.is_none(),
            "absent track clears the current track"
        );
        assert_eq!(next.queue, state.queue);
    }

    #[test]
    fn volume_actions_set_and_clamp_volume() {
        let state = room_state("current");
        let over = proto::PlaybackActionPayload {
            action: action::SET_VOLUME.to_string(),
            volume: 4.5,
            ..Default::default()
        };
        assert_eq!(apply_playback_action(&state, &over).volume, 1.0);

        let under = proto::PlaybackActionPayload {
            action: action::SET_VOLUME.to_string(),
            volume: -1.0,
            ..Default::default()
        };
        assert_eq!(apply_playback_action(&state, &under).volume, 0.0);

        // A play action's volume field is ignored (volumeOrNull semantics).
        let ignored = proto::PlaybackActionPayload {
            action: action::PLAY.to_string(),
            volume: 0.25,
            ..Default::default()
        };
        assert_eq!(apply_playback_action(&state, &ignored).volume, state.volume);
    }

    #[test]
    fn sync_state_is_authoritative_and_versioned() {
        let state = room_state("current");
        let payload = proto::SyncStatePayload {
            current_track: Some(track("other")),
            is_playing: true,
            position: 10,
            last_update: 20,
            queue: Vec::new(),
            volume: 0.5,
            revision: 3,
        };
        let next = apply_sync_state(&state, &payload);
        assert_eq!(next.current_track.as_ref().unwrap().id, "other");
        assert!(next.queue.is_empty());
        assert_eq!(next.volume, 0.5);
        assert_eq!(next.revision, 3);
    }

    #[test]
    fn pending_sync_waits_for_the_matching_buffer_complete() {
        let pending = PendingSync::from_playback(Some("track"), true, 500, Some(1_000));
        assert!(!pending.ready(None, None));
        assert!(!pending.ready(Some("track"), Some("other")));
        assert!(pending.ready(Some("track"), Some("track")));
        assert!(pending.ready(None, Some("track")));

        let no_track = PendingSync::from_playback(None, false, 0, None);
        assert!(!no_track.ready(None, None));
    }

    #[test]
    fn buffering_tracks_the_waiting_set() {
        let mut buffering = Buffering::default();
        assert!(!buffering.is_waiting());
        buffering.on_wait(vec!["a".to_string(), "b".to_string()]);
        assert!(buffering.is_waiting());
        assert_eq!(buffering.waiting_for(), ["a", "b"]);
        assert!(buffering.on_complete());
        assert!(!buffering.is_waiting());
        assert!(!buffering.on_complete());
    }

    #[test]
    fn reconnect_backoff_matches_the_android_formula() {
        assert_eq!(backoff_delay_ms(1, 0.0), 2_000);
        assert_eq!(backoff_delay_ms(2, 0.0), 4_000);
        assert_eq!(backoff_delay_ms(3, 0.0), 8_000);
        assert_eq!(backoff_delay_ms(4, 0.0), 16_000);
        assert_eq!(backoff_delay_ms(5, 0.0), 32_000);
        assert_eq!(backoff_delay_ms(15, 0.0), 32_000);
        assert_eq!(backoff_delay_ms(1, 1.0), 2_400);
        assert_eq!(backoff_delay_ms(5, 0.5), 35_200);
        // Jitter input is clamped to [0, 1].
        assert_eq!(backoff_delay_ms(1, -1.0), 2_000);
        assert_eq!(backoff_delay_ms(1, 2.0), 2_400);
        assert_eq!(backoff_delay_ms(0, 0.0), 2_000);
        assert!(should_reconnect(0));
        assert!(should_reconnect(14));
        assert!(!should_reconnect(15));
        assert!(requires_server_capture(action::SEEK));
        assert!(!requires_server_capture(action::QUEUE_ADD));
    }

    #[test]
    fn queue_round_trip_preserves_track_metadata() {
        let current = proto::TrackInfo {
            id: "vid".to_string(),
            title: "Title".to_string(),
            artist: "A, B".to_string(),
            album: "Album".to_string(),
            duration: 215_000,
            thumbnail: "https://img".to_string(),
            suggested_by: "suggester".to_string(),
        };
        let queue = queue_from_room(Some(&current), &[track("next")]);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.current().unwrap().id, "vid");
        assert_eq!(queue.current().unwrap().title, "Title");
        assert_eq!(queue.current().unwrap().duration, Some(215));

        let (back_current, upcoming) = queue_to_room(&queue);
        assert_eq!(back_current.unwrap().id, "vid");
        assert_eq!(
            upcoming.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["next"]
        );
    }

    #[test]
    fn host_change_moves_the_host_flag() {
        let mut state = proto::RoomState {
            host_id: "a".to_string(),
            users: vec![
                proto::UserInfo {
                    user_id: "a".to_string(),
                    is_host: true,
                    ..Default::default()
                },
                proto::UserInfo {
                    user_id: "b".to_string(),
                    is_host: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        apply_host_change(&mut state, "b");
        assert_eq!(state.host_id, "b");
        assert!(!state.users[0].is_host);
        assert!(state.users[1].is_host);
    }

    #[test]
    fn playback_snapshot_extrapolates_from_the_server_clock() {
        let state = proto::RoomState {
            current_track: Some(track("current")),
            is_playing: true,
            position: 500,
            last_update: 10_000,
            revision: 4,
            ..Default::default()
        };
        let snapshot = PlaybackSnapshot::from_room_state(&state);
        let mut clock = ServerClock::new();
        clock.record_pong(1_000, 10_000, 10_000, 1_100);
        assert_eq!(snapshot.position_at(&clock, 1_220), 670);

        let paused = PlaybackSnapshot::from_room_state(&proto::RoomState {
            is_playing: false,
            ..state
        });
        assert_eq!(paused.position_at(&clock, 1_220), 500);
    }

    #[test]
    fn server_message_decode_is_exposed_for_the_session() {
        // Compile-time smoke test that the helper is wired.
        let bytes = proto::ErrorPayload {
            code: "x".to_string(),
            message: "y".to_string(),
        };
        let decoded = ServerMessage::decode(msg::ERROR, &{
            use prost::Message;
            bytes.encode_to_vec()
        });
        assert!(matches!(decoded, Some(ServerMessage::Error(_))));
    }
}
