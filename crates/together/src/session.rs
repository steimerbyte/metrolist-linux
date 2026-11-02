//! Room session state machine.
//!
//! `RoomSession` folds decoded server messages into local room state and
//! returns the effects the network layer must execute (send a message, drop
//! the persisted session, schedule a rejoin). It performs no I/O, so the whole
//! protocol behaviour is unit-testable.

use std::collections::HashSet;

use crate::clock::ServerClock;
use crate::messages::{Outbound, PlaybackAction, ServerMessage};
use crate::proto;
use crate::sync::{self, Buffering, RevisionGate};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Role {
    #[default]
    None,
    Host,
    Guest,
}

/// A UI-facing snapshot of everything a room screen needs.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    pub room: Option<proto::RoomState>,
    pub role: Role,
    pub user_id: Option<String>,
    pub room_code: Option<String>,
    pub username: Option<String>,
    pub pending_join_requests: Vec<proto::JoinRequestPayload>,
    pub pending_suggestions: Vec<proto::SuggestionReceivedPayload>,
    pub buffering_users: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    Snapshot(SessionSnapshot),
    RoomCreated {
        room_code: String,
        user_id: String,
    },
    JoinRequest {
        user_id: String,
        username: String,
    },
    JoinApproved {
        room_code: String,
        user_id: String,
    },
    JoinRejected {
        reason: String,
    },
    UserJoined {
        user_id: String,
        username: String,
    },
    UserLeft {
        user_id: String,
        username: String,
    },
    HostChanged {
        new_host_id: String,
        new_host_name: String,
    },
    Kicked {
        reason: String,
    },
    PlaybackSync(proto::PlaybackActionPayload),
    BufferWait {
        track_id: String,
        waiting_for: Vec<String>,
    },
    BufferComplete {
        track_id: String,
    },
    SyncStateReceived(proto::SyncStatePayload),
    Reconnected {
        room_code: String,
        user_id: String,
        is_host: bool,
    },
    UserReconnected {
        user_id: String,
        username: String,
    },
    UserDisconnected {
        user_id: String,
        username: String,
    },
    SuggestionReceived(proto::SuggestionReceivedPayload),
    SuggestionApproved(proto::SuggestionApprovedPayload),
    SuggestionRejected(proto::SuggestionRejectedPayload),
    ServerError {
        code: String,
        message: String,
    },
}

/// Work the network layer must perform after a state transition.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEffect {
    Send(Outbound),
    Event(SessionEvent),
    /// Forget the persisted session token/room (leave, kick, expired host).
    ClearSession,
    /// The server forgot a guest session: rejoin after a short delay.
    ScheduleRejoin {
        room_code: String,
        username: String,
    },
}

impl SessionEffect {
    fn event(event: SessionEvent) -> Self {
        Self::Event(event)
    }

    fn send(outbound: Outbound) -> Self {
        Self::Send(outbound)
    }
}

/// Listens for a room, applying server events to local state.
#[derive(Debug, Clone)]
pub struct RoomSession {
    clock: ServerClock,
    room: Option<proto::RoomState>,
    role: Role,
    room_code: Option<String>,
    username: Option<String>,
    user_id: Option<String>,
    session_token: Option<String>,
    pending_join_requests: Vec<proto::JoinRequestPayload>,
    pending_suggestions: Vec<proto::SuggestionReceivedPayload>,
    buffering: Buffering,
    blocked_usernames: HashSet<String>,
    gate: RevisionGate,
    was_host: bool,
    ping_sequence: u64,
    /// Preferences that the Android client reads from DataStore.
    pub auto_approve_join: bool,
    pub auto_approve_suggestions: bool,
}

impl Default for RoomSession {
    fn default() -> Self {
        Self::new()
    }
}

impl RoomSession {
    pub fn new() -> Self {
        Self {
            clock: ServerClock::new(),
            room: None,
            role: Role::None,
            room_code: None,
            username: None,
            user_id: None,
            session_token: None,
            pending_join_requests: Vec::new(),
            pending_suggestions: Vec::new(),
            buffering: Buffering::default(),
            blocked_usernames: HashSet::new(),
            gate: RevisionGate::default(),
            was_host: false,
            ping_sequence: 0,
            auto_approve_join: false,
            auto_approve_suggestions: false,
        }
    }

    // ---- Accessors ----

    pub fn clock(&self) -> &ServerClock {
        &self.clock
    }

    pub fn set_clock(&mut self, clock: ServerClock) {
        self.clock = clock;
    }

    pub fn room(&self) -> Option<&proto::RoomState> {
        self.room.as_ref()
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn is_host(&self) -> bool {
        self.role == Role::Host
    }

    pub fn is_guest(&self) -> bool {
        self.role == Role::Guest
    }

    pub fn is_in_room(&self) -> bool {
        self.room.is_some()
    }

    pub fn room_code(&self) -> Option<&str> {
        self.room_code.as_deref()
    }

    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }

    pub fn session_token(&self) -> Option<&str> {
        self.session_token.as_deref()
    }

    pub fn was_host(&self) -> bool {
        self.was_host
    }

    pub fn pending_join_requests(&self) -> &[proto::JoinRequestPayload] {
        &self.pending_join_requests
    }

    pub fn pending_suggestions(&self) -> &[proto::SuggestionReceivedPayload] {
        &self.pending_suggestions
    }

    pub fn buffering_users(&self) -> &[String] {
        self.buffering.waiting_for()
    }

    pub fn revision(&self) -> u64 {
        self.gate.last()
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            room: self.room.clone(),
            role: self.role,
            user_id: self.user_id.clone(),
            room_code: self.room_code.clone(),
            username: self.username.clone(),
            pending_join_requests: self.pending_join_requests.clone(),
            pending_suggestions: self.pending_suggestions.clone(),
            buffering_users: self.buffering.waiting_for().to_vec(),
        }
    }

    // ---- Session lifecycle ----

    /// Reset per-session protocol state (keep preferences and blocks).
    pub fn clear_session(&mut self) {
        self.clock.reset();
        self.room = None;
        self.role = Role::None;
        self.room_code = None;
        self.username = None;
        self.user_id = None;
        self.session_token = None;
        self.pending_join_requests.clear();
        self.pending_suggestions.clear();
        self.buffering.clear();
        self.gate.reset();
        self.was_host = false;
    }

    /// Drop the server-side session but keep the username for an automatic
    /// rejoin (`session_not_found`).
    pub fn forget_server_session(&mut self) {
        self.room = None;
        self.role = Role::None;
        self.room_code = None;
        self.user_id = None;
        self.session_token = None;
        self.pending_join_requests.clear();
        self.pending_suggestions.clear();
        self.buffering.clear();
        self.gate.reset();
    }

    /// Prepare a new room creation: drop any previous session.
    pub fn begin_create(&mut self, username: &str) {
        self.clear_session();
        self.username = Some(username.to_string());
    }

    /// Prepare joining a room: drop any previous session.
    pub fn begin_join(&mut self, room_code: &str, username: &str) {
        self.clear_session();
        self.username = Some(username.to_string());
        self.room_code = Some(room_code.to_ascii_uppercase());
    }

    /// The connection opened: the Android client resets its clock here.
    pub fn on_connected(&mut self) {
        self.clock.reset();
    }

    /// The connection is gone; room state is kept for a reconnect.
    pub fn on_disconnected(&mut self) {
        self.clock.reset();
        self.pending_join_requests.clear();
        self.buffering.clear();
    }

    pub fn next_ping(&mut self, client_time: i64) -> Outbound {
        self.ping_sequence += 1;
        Outbound::Ping {
            client_time,
            sequence: self.ping_sequence,
        }
    }

    pub fn server_time_now(&self, elapsed_realtime: i64) -> Option<i64> {
        self.clock.now_at(elapsed_realtime)
    }

    pub fn position_at(
        &self,
        position: i64,
        effective_at_server_time: Option<i64>,
        is_playing: bool,
        elapsed_realtime: i64,
    ) -> i64 {
        self.clock.position_at(
            position,
            effective_at_server_time,
            is_playing,
            elapsed_realtime,
        )
    }

    /// Build the wire payload for a host playback action, stamping
    /// `captured_at_server_time` for play/pause/seek when the clock is ready.
    pub fn playback_outbound(&self, action: PlaybackAction, elapsed_realtime: i64) -> Outbound {
        let captured = (sync::requires_server_capture(&action.action) && action.position.is_some())
            .then(|| self.server_time_now(elapsed_realtime))
            .flatten();
        Outbound::PlaybackAction(action.to_payload(captured))
    }

    pub fn block_user(&mut self, username: &str) {
        self.blocked_usernames.insert(username.to_string());
        self.pending_join_requests
            .retain(|request| request.username != username);
        self.pending_suggestions
            .retain(|suggestion| suggestion.from_username != username);
    }

    pub fn unblock_user(&mut self, username: &str) {
        self.blocked_usernames.remove(username);
    }

    pub fn is_user_blocked(&self, username: &str) -> bool {
        self.blocked_usernames.contains(username)
    }

    // ---- Event handling ----

    pub fn apply(&mut self, message: ServerMessage, elapsed_realtime: i64) -> Vec<SessionEffect> {
        let before = self.snapshot();
        let mut effects = Vec::new();

        match message {
            ServerMessage::RoomCreated(payload) => {
                self.user_id = Some(payload.user_id.clone());
                self.role = Role::Host;
                self.was_host = true;
                self.gate.reset();
                self.session_token = Some(payload.session_token.clone());
                self.room_code = Some(payload.room_code.clone());
                let last_update = self.clock.now_at(elapsed_realtime).unwrap_or(0);
                self.room = Some(proto::RoomState {
                    room_code: payload.room_code.clone(),
                    host_id: payload.user_id.clone(),
                    users: vec![proto::UserInfo {
                        user_id: payload.user_id.clone(),
                        username: self.username.clone().unwrap_or_default(),
                        is_host: true,
                        is_connected: true,
                    }],
                    current_track: None,
                    is_playing: false,
                    position: 0,
                    last_update,
                    volume: 1.0,
                    queue: Vec::new(),
                    revision: 0,
                });
                effects.push(SessionEffect::event(SessionEvent::RoomCreated {
                    room_code: payload.room_code,
                    user_id: payload.user_id,
                }));
            }

            ServerMessage::JoinRequest(payload) => {
                if self.is_user_blocked(&payload.username) {
                    effects.push(SessionEffect::send(Outbound::RejectJoin {
                        user_id: payload.user_id,
                        reason: Some("You are blocked".to_string()),
                    }));
                } else {
                    self.pending_join_requests.push(payload.clone());
                    if self.auto_approve_join && self.is_host() {
                        effects.push(SessionEffect::send(Outbound::ApproveJoin {
                            user_id: payload.user_id.clone(),
                        }));
                    }
                    effects.push(SessionEffect::event(SessionEvent::JoinRequest {
                        user_id: payload.user_id,
                        username: payload.username,
                    }));
                }
            }

            ServerMessage::JoinApproved(payload) => {
                self.user_id = Some(payload.user_id.clone());
                self.role = Role::Guest;
                self.was_host = false;
                self.session_token = Some(payload.session_token.clone());
                self.room_code = Some(payload.room_code.clone());
                let state = payload.state.unwrap_or_default();
                self.gate.set(state.revision);
                self.room = Some(state);
                effects.push(SessionEffect::event(SessionEvent::JoinApproved {
                    room_code: payload.room_code,
                    user_id: payload.user_id,
                }));
            }

            ServerMessage::JoinRejected(payload) => {
                effects.push(SessionEffect::event(SessionEvent::JoinRejected {
                    reason: payload.reason,
                }));
            }

            ServerMessage::UserJoined(payload) => {
                if let Some(room) = self.room.as_mut() {
                    room.users.push(proto::UserInfo {
                        user_id: payload.user_id.clone(),
                        username: payload.username.clone(),
                        is_host: false,
                        is_connected: true,
                    });
                }
                self.pending_join_requests
                    .retain(|request| request.user_id != payload.user_id);
                effects.push(SessionEffect::event(SessionEvent::UserJoined {
                    user_id: payload.user_id,
                    username: payload.username,
                }));
            }

            ServerMessage::UserLeft(payload) => {
                if let Some(room) = self.room.as_mut() {
                    room.users.retain(|user| user.user_id != payload.user_id);
                }
                effects.push(SessionEffect::event(SessionEvent::UserLeft {
                    user_id: payload.user_id,
                    username: payload.username,
                }));
            }

            ServerMessage::HostChanged(payload) => {
                if let Some(room) = self.room.as_mut() {
                    sync::apply_host_change(room, &payload.new_host_id);
                }
                self.role = role_after_host_change(
                    self.role,
                    self.user_id.as_deref(),
                    &payload.new_host_id,
                );
                effects.push(SessionEffect::event(SessionEvent::HostChanged {
                    new_host_id: payload.new_host_id,
                    new_host_name: payload.new_host_name,
                }));
            }

            ServerMessage::Kicked(payload) => {
                self.clear_session();
                effects.push(SessionEffect::ClearSession);
                effects.push(SessionEffect::event(SessionEvent::Kicked {
                    reason: payload.reason,
                }));
            }

            ServerMessage::SyncPlayback(payload) => {
                if !self.gate.accept(payload.revision) {
                    return effects;
                }
                if let Some(room) = self.room.take() {
                    self.room = Some(sync::apply_playback_action(&room, &payload));
                }
                effects.push(SessionEffect::event(SessionEvent::PlaybackSync(payload)));
            }

            ServerMessage::BufferWait(payload) => {
                self.buffering.on_wait(payload.waiting_for.clone());
                effects.push(SessionEffect::event(SessionEvent::BufferWait {
                    track_id: payload.track_id,
                    waiting_for: payload.waiting_for,
                }));
            }

            ServerMessage::BufferComplete(payload) => {
                self.buffering.on_complete();
                effects.push(SessionEffect::event(SessionEvent::BufferComplete {
                    track_id: payload.track_id,
                }));
            }

            ServerMessage::SyncState(payload) => {
                if !self.gate.accept(payload.revision) {
                    return effects;
                }
                if let Some(room) = self.room.take() {
                    self.room = Some(sync::apply_sync_state(&room, &payload));
                }
                effects.push(SessionEffect::event(SessionEvent::SyncStateReceived(
                    payload,
                )));
            }

            ServerMessage::Pong(payload) => {
                let first_sample = self.clock.record_pong(
                    payload.client_time,
                    payload.server_receive_time,
                    payload.server_send_time,
                    elapsed_realtime,
                );
                if first_sample && self.is_guest() && self.room.is_some() {
                    effects.push(SessionEffect::send(Outbound::RequestSync));
                }
            }

            ServerMessage::Reconnected(payload) => {
                self.user_id = Some(payload.user_id.clone());
                self.role = if payload.is_host {
                    Role::Host
                } else {
                    Role::Guest
                };
                self.was_host = payload.is_host;
                let state = payload.state.unwrap_or_default();
                self.gate.set(state.revision);
                self.room = Some(state);
                effects.push(SessionEffect::event(SessionEvent::Reconnected {
                    room_code: payload.room_code,
                    user_id: payload.user_id,
                    is_host: payload.is_host,
                }));
            }

            ServerMessage::UserReconnected(payload) => {
                if let Some(room) = self.room.as_mut() {
                    for user in &mut room.users {
                        if user.user_id == payload.user_id {
                            user.is_connected = true;
                        }
                    }
                }
                effects.push(SessionEffect::event(SessionEvent::UserReconnected {
                    user_id: payload.user_id,
                    username: payload.username,
                }));
            }

            ServerMessage::UserDisconnected(payload) => {
                if let Some(room) = self.room.as_mut() {
                    for user in &mut room.users {
                        if user.user_id == payload.user_id {
                            user.is_connected = false;
                        }
                    }
                }
                effects.push(SessionEffect::event(SessionEvent::UserDisconnected {
                    user_id: payload.user_id,
                    username: payload.username,
                }));
            }

            ServerMessage::SuggestionReceived(payload) => {
                if self.is_host() && !self.is_user_blocked(&payload.from_username) {
                    if self.auto_approve_suggestions {
                        effects.push(SessionEffect::send(Outbound::ApproveSuggestion {
                            suggestion_id: payload.suggestion_id.clone(),
                        }));
                    } else {
                        self.pending_suggestions.push(payload.clone());
                    }
                    effects.push(SessionEffect::event(SessionEvent::SuggestionReceived(
                        payload,
                    )));
                }
            }

            ServerMessage::SuggestionApproved(payload) => {
                self.pending_suggestions
                    .retain(|suggestion| suggestion.suggestion_id != payload.suggestion_id);
                effects.push(SessionEffect::event(SessionEvent::SuggestionApproved(
                    payload,
                )));
            }

            ServerMessage::SuggestionRejected(payload) => {
                self.pending_suggestions
                    .retain(|suggestion| suggestion.suggestion_id != payload.suggestion_id);
                effects.push(SessionEffect::event(SessionEvent::SuggestionRejected(
                    payload,
                )));
            }

            ServerMessage::Error(payload) => {
                if payload.code == "session_not_found" {
                    self.room = None;
                    self.role = Role::None;
                    self.pending_join_requests.clear();
                    self.buffering.clear();
                    match (self.room_code.clone(), self.username.clone()) {
                        (Some(room_code), Some(username)) if !self.was_host => {
                            effects.push(SessionEffect::ScheduleRejoin {
                                room_code,
                                username,
                            });
                        }
                        _ => effects.push(SessionEffect::ClearSession),
                    }
                }
                effects.push(SessionEffect::event(SessionEvent::ServerError {
                    code: payload.code,
                    message: payload.message,
                }));
            }
        }

        let after = self.snapshot();
        if after != before {
            effects.push(SessionEffect::event(SessionEvent::Snapshot(after)));
        }
        effects
    }

    /// Convenience used by the client: only hosts may push playback changes.
    pub fn host_playback(&self, action: PlaybackAction, elapsed_realtime: i64) -> Option<Outbound> {
        self.is_host()
            .then(|| self.playback_outbound(action, elapsed_realtime))
    }

    /// `track_id.trim()` non-empty, as `sendBufferReady` requires.
    pub fn buffer_ready(&self, track_id: &str) -> Option<Outbound> {
        let track_id = track_id.trim();
        (!track_id.is_empty()).then(|| Outbound::BufferReady {
            track_id: track_id.to_string(),
        })
    }

    /// Guest-only suggestion precondition.
    pub fn suggest(&self, track: proto::TrackInfo) -> Option<Outbound> {
        (self.is_in_room() && !self.is_host())
            .then_some(Outbound::SuggestTrack { track_info: track })
    }
}

impl PartialEq for RoomSession {
    fn eq(&self, other: &Self) -> bool {
        self.snapshot() == other.snapshot()
            && self.clock == other.clock
            && self.session_token == other.session_token
            && self.was_host == other.was_host
            && self.ping_sequence == other.ping_sequence
    }
}

/// Role after a `host_changed`: the new host becomes host, a previous host
/// without the role becomes a guest, everyone else keeps their role.
pub fn role_after_host_change(current: Role, user_id: Option<&str>, new_host_id: &str) -> Role {
    if user_id == Some(new_host_id) {
        Role::Host
    } else if current == Role::Host {
        Role::Guest
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{action, msg};
    use prost::Message;

    fn track(id: &str) -> proto::TrackInfo {
        proto::TrackInfo {
            id: id.to_string(),
            title: id.to_string(),
            artist: "Artist".to_string(),
            duration: 1_000,
            ..Default::default()
        }
    }

    fn room_created(session: &mut RoomSession) -> Vec<SessionEffect> {
        session.begin_create("host");
        session.apply(
            ServerMessage::RoomCreated(proto::RoomCreatedPayload {
                room_code: "ABC123".to_string(),
                user_id: "u-host".to_string(),
                session_token: "token".to_string(),
            }),
            1_000,
        )
    }

    #[test]
    fn room_created_initialises_host_state() {
        let mut session = RoomSession::new();
        let effects = room_created(&mut session);

        assert_eq!(session.role(), Role::Host);
        assert!(session.was_host());
        assert_eq!(session.room_code(), Some("ABC123"));
        assert_eq!(session.user_id(), Some("u-host"));
        assert_eq!(session.session_token(), Some("token"));
        let room = session.room().unwrap();
        assert_eq!(room.host_id, "u-host");
        assert_eq!(room.users.len(), 1);
        assert!(room.users[0].is_host);
        assert!(!room.is_playing);

        assert!(
            effects.contains(&SessionEffect::Event(SessionEvent::RoomCreated {
                room_code: "ABC123".to_string(),
                user_id: "u-host".to_string(),
            }))
        );
        assert!(effects.iter().any(|effect| matches!(
            effect,
            SessionEffect::Event(SessionEvent::Snapshot(snapshot)) if snapshot.room.is_some()
        )));
    }

    #[test]
    fn join_request_is_pending_and_blocked_users_are_rejected_silently() {
        let mut session = RoomSession::new();
        room_created(&mut session);

        let effects = session.apply(
            ServerMessage::JoinRequest(proto::JoinRequestPayload {
                user_id: "guest-1".to_string(),
                username: "guest".to_string(),
            }),
            1_000,
        );
        assert_eq!(session.pending_join_requests().len(), 1);
        assert!(
            effects.contains(&SessionEffect::Event(SessionEvent::JoinRequest {
                user_id: "guest-1".to_string(),
                username: "guest".to_string(),
            }))
        );

        session.block_user("evil");
        let effects = session.apply(
            ServerMessage::JoinRequest(proto::JoinRequestPayload {
                user_id: "evil-1".to_string(),
                username: "evil".to_string(),
            }),
            1_000,
        );
        assert_eq!(session.pending_join_requests().len(), 1);
        assert!(effects.contains(&SessionEffect::Send(Outbound::RejectJoin {
            user_id: "evil-1".to_string(),
            reason: Some("You are blocked".to_string()),
        })));
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            SessionEffect::Event(SessionEvent::JoinRequest { username, .. }) if username == "evil"
        )));

        session.auto_approve_join = true;
        let effects = session.apply(
            ServerMessage::JoinRequest(proto::JoinRequestPayload {
                user_id: "guest-2".to_string(),
                username: "guest2".to_string(),
            }),
            1_000,
        );
        assert!(
            effects.contains(&SessionEffect::Send(Outbound::ApproveJoin {
                user_id: "guest-2".to_string(),
            }))
        );
    }

    #[test]
    fn join_approved_sets_guest_state_and_revision_baseline() {
        let mut session = RoomSession::new();
        session.begin_join("abc123", "guest");
        let state = proto::RoomState {
            room_code: "ABC123".to_string(),
            host_id: "u-host".to_string(),
            current_track: Some(track("current")),
            revision: 7,
            ..Default::default()
        };
        session.apply(
            ServerMessage::JoinApproved(proto::JoinApprovedPayload {
                room_code: "ABC123".to_string(),
                user_id: "u-guest".to_string(),
                session_token: "tok".to_string(),
                state: Some(state.clone()),
            }),
            1_000,
        );

        assert_eq!(session.role(), Role::Guest);
        assert!(!session.was_host());
        assert_eq!(session.room(), Some(&state));
        assert_eq!(session.revision(), 7);

        // Stale playback is discarded.
        let stale = proto::PlaybackActionPayload {
            action: action::PLAY.to_string(),
            revision: 6,
            position: 1,
            ..Default::default()
        };
        let effects = session.apply(ServerMessage::SyncPlayback(stale), 1_000);
        assert!(effects.is_empty());
        assert!(!session.room().unwrap().is_playing);

        // Equal revision is accepted (host heartbeats carry the same revision).
        let heartbeat = proto::PlaybackActionPayload {
            action: action::PLAY.to_string(),
            revision: 7,
            position: 500,
            server_time: 10_000,
            ..Default::default()
        };
        session.apply(ServerMessage::SyncPlayback(heartbeat), 1_000);
        assert!(session.room().unwrap().is_playing);
        assert_eq!(session.room().unwrap().position, 500);

        // Unversioned playback is always accepted.
        let unversioned = proto::PlaybackActionPayload {
            action: action::SET_VOLUME.to_string(),
            volume: 0.25,
            ..Default::default()
        };
        session.apply(ServerMessage::SyncPlayback(unversioned), 1_000);
        assert_eq!(session.room().unwrap().volume, 0.25);
        assert_eq!(session.revision(), 7);
    }

    #[test]
    fn sync_state_replaces_room_state_but_rejects_stale_revisions() {
        let mut session = RoomSession::new();
        room_created(&mut session);
        let state = proto::SyncStatePayload {
            current_track: Some(track("new")),
            is_playing: true,
            position: 42,
            last_update: 99,
            queue: vec![track("a")],
            volume: 0.5,
            revision: 3,
        };
        session.apply(ServerMessage::SyncState(state.clone()), 1_000);
        let room = session.room().unwrap();
        assert_eq!(room.current_track.as_ref().unwrap().id, "new");
        assert_eq!(room.queue.len(), 1);
        assert_eq!(room.revision, 3);

        let stale = proto::SyncStatePayload {
            revision: 2,
            ..state
        };
        let before = session.room().cloned();
        let effects = session.apply(ServerMessage::SyncState(stale), 1_000);
        assert!(effects.is_empty());
        assert_eq!(session.room(), before.as_ref());
    }

    #[test]
    fn guest_role_can_be_transferred_and_lost() {
        let mut session = RoomSession::new();
        session.begin_join("ROOM", "guest");
        session.apply(
            ServerMessage::JoinApproved(proto::JoinApprovedPayload {
                room_code: "ROOM".to_string(),
                user_id: "u-guest".to_string(),
                session_token: "tok".to_string(),
                state: Some(proto::RoomState {
                    room_code: "ROOM".to_string(),
                    host_id: "u-host".to_string(),
                    users: vec![
                        proto::UserInfo {
                            user_id: "u-host".to_string(),
                            username: "host".to_string(),
                            is_host: true,
                            is_connected: true,
                        },
                        proto::UserInfo {
                            user_id: "u-guest".to_string(),
                            username: "guest".to_string(),
                            is_host: false,
                            is_connected: true,
                        },
                    ],
                    ..Default::default()
                }),
            }),
            1_000,
        );

        session.apply(
            ServerMessage::HostChanged(proto::HostChangedPayload {
                new_host_id: "u-guest".to_string(),
                new_host_name: "guest".to_string(),
            }),
            1_000,
        );
        assert_eq!(session.role(), Role::Host);
        assert!(session.room().unwrap().users[1].is_host);

        session.apply(
            ServerMessage::HostChanged(proto::HostChangedPayload {
                new_host_id: "u-host".to_string(),
                new_host_name: "host".to_string(),
            }),
            1_000,
        );
        assert_eq!(session.role(), Role::Guest);
    }

    #[test]
    fn role_after_host_change_keeps_bystanders_unchanged() {
        assert_eq!(
            role_after_host_change(Role::Guest, Some("me"), "me"),
            Role::Host
        );
        assert_eq!(
            role_after_host_change(Role::Host, Some("me"), "other"),
            Role::Guest
        );
        assert_eq!(
            role_after_host_change(Role::Guest, Some("me"), "other"),
            Role::Guest
        );
        assert_eq!(
            role_after_host_change(Role::None, Some("me"), "other"),
            Role::None
        );
        assert_eq!(
            role_after_host_change(Role::Host, None, "other"),
            Role::Guest
        );
    }

    #[test]
    fn kicked_clears_the_session() {
        let mut session = RoomSession::new();
        room_created(&mut session);
        let effects = session.apply(
            ServerMessage::Kicked(proto::KickedPayload {
                reason: "bye".to_string(),
            }),
            1_000,
        );
        assert_eq!(session.role(), Role::None);
        assert!(session.room().is_none());
        assert!(session.session_token().is_none());
        assert!(effects.contains(&SessionEffect::ClearSession));
        assert!(
            effects.contains(&SessionEffect::Event(SessionEvent::Kicked {
                reason: "bye".to_string(),
            }))
        );
    }

    #[test]
    fn buffer_messages_track_waiting_users() {
        let mut session = RoomSession::new();
        room_created(&mut session);
        session.apply(
            ServerMessage::BufferWait(proto::BufferWaitPayload {
                track_id: "t".to_string(),
                waiting_for: vec!["a".to_string(), "b".to_string()],
            }),
            1_000,
        );
        assert_eq!(session.buffering_users(), ["a", "b"]);
        assert!(session
            .snapshot()
            .buffering_users
            .contains(&"b".to_string()));

        session.apply(
            ServerMessage::BufferComplete(proto::BufferCompletePayload {
                track_id: "t".to_string(),
            }),
            1_000,
        );
        assert!(session.buffering_users().is_empty());
    }

    #[test]
    fn suggestions_are_host_only_and_removed_on_decision() {
        let mut session = RoomSession::new();
        room_created(&mut session);
        session.auto_approve_suggestions = false;
        let suggestion = proto::SuggestionReceivedPayload {
            suggestion_id: "s1".to_string(),
            from_user_id: "u-guest".to_string(),
            from_username: "guest".to_string(),
            track_info: Some(track("suggested")),
        };
        session.apply(ServerMessage::SuggestionReceived(suggestion.clone()), 1_000);
        assert_eq!(session.pending_suggestions().len(), 1);

        session.apply(
            ServerMessage::SuggestionApproved(proto::SuggestionApprovedPayload {
                suggestion_id: "s1".to_string(),
                track_info: Some(track("suggested")),
            }),
            1_000,
        );
        assert!(session.pending_suggestions().is_empty());

        // Blocked senders are ignored.
        session.block_user("guest");
        session.apply(ServerMessage::SuggestionReceived(suggestion), 1_000);
        assert!(session.pending_suggestions().is_empty());

        // Auto-approval sends the approval instead of queueing it.
        session.unblock_user("guest");
        session.auto_approve_suggestions = true;
        let effects = session.apply(
            ServerMessage::SuggestionReceived(proto::SuggestionReceivedPayload {
                suggestion_id: "s2".to_string(),
                from_user_id: "u-guest".to_string(),
                from_username: "guest".to_string(),
                track_info: Some(track("suggested")),
            }),
            1_000,
        );
        assert!(
            effects.contains(&SessionEffect::Send(Outbound::ApproveSuggestion {
                suggestion_id: "s2".to_string(),
            }))
        );
        assert!(session.pending_suggestions().is_empty());
    }

    #[test]
    fn session_not_found_rejoins_guests_and_clears_hosts() {
        let mut guest = RoomSession::new();
        guest.begin_join("ROOM", "guest");
        guest.apply(
            ServerMessage::JoinApproved(proto::JoinApprovedPayload {
                room_code: "ROOM".to_string(),
                user_id: "u".to_string(),
                session_token: "tok".to_string(),
                state: Some(proto::RoomState::default()),
            }),
            1_000,
        );
        let effects = guest.apply(
            ServerMessage::Error(proto::ErrorPayload {
                code: "session_not_found".to_string(),
                message: "gone".to_string(),
            }),
            1_000,
        );
        assert!(effects.contains(&SessionEffect::ScheduleRejoin {
            room_code: "ROOM".to_string(),
            username: "guest".to_string(),
        }));
        assert!(guest.room().is_none());

        let mut host = RoomSession::new();
        room_created(&mut host);
        let effects = host.apply(
            ServerMessage::Error(proto::ErrorPayload {
                code: "session_not_found".to_string(),
                message: "gone".to_string(),
            }),
            1_000,
        );
        assert!(effects.contains(&SessionEffect::ClearSession));
    }

    #[test]
    fn first_pong_sets_the_clock_and_asks_guests_for_a_sync() {
        let mut session = RoomSession::new();
        session.begin_join("ROOM", "guest");
        session.apply(
            ServerMessage::JoinApproved(proto::JoinApprovedPayload {
                room_code: "ROOM".to_string(),
                user_id: "u".to_string(),
                session_token: "tok".to_string(),
                state: Some(proto::RoomState::default()),
            }),
            1_000,
        );
        let pong = proto::PongPayload {
            client_time: 1_000,
            server_receive_time: 10_000,
            server_send_time: 10_010,
            sequence: 1,
        };
        let effects = session.apply(ServerMessage::Pong(pong), 1_120);
        assert_eq!(session.clock().now_at(1_120), Some(10_065));
        assert!(effects.contains(&SessionEffect::Send(Outbound::RequestSync)));

        // A second pong does not request another sync.
        let effects = session.apply(ServerMessage::Pong(pong), 1_400);
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, SessionEffect::Send(Outbound::RequestSync))));
    }

    #[test]
    fn reconnect_restores_state_and_revision() {
        let mut session = RoomSession::new();
        session.begin_join("ROOM", "guest");
        let state = proto::RoomState {
            is_playing: true,
            position: 5,
            revision: 11,
            ..Default::default()
        };
        session.apply(
            ServerMessage::Reconnected(proto::ReconnectedPayload {
                room_code: "ROOM".to_string(),
                user_id: "u".to_string(),
                state: Some(state.clone()),
                is_host: false,
            }),
            1_000,
        );
        assert_eq!(session.role(), Role::Guest);
        assert_eq!(session.room(), Some(&state));
        assert_eq!(session.revision(), 11);
    }

    #[test]
    fn outbound_helpers_stamp_server_time_and_enforce_roles() {
        let mut session = RoomSession::new();
        room_created(&mut session);
        session.clock = {
            let mut clock = ServerClock::new();
            clock.record_pong(1_000, 10_000, 10_010, 1_120);
            clock
        };

        let outbound = session.playback_outbound(
            PlaybackAction {
                action: action::PLAY.to_string(),
                position: Some(1_000),
                ..Default::default()
            },
            1_120,
        );
        let Outbound::PlaybackAction(payload) = outbound else {
            panic!("expected playback action");
        };
        assert_eq!(payload.action, action::PLAY);
        assert_eq!(payload.captured_at_server_time, 10_065);
        assert_eq!(payload.position, 1_000);

        // Queue mutations are not stamped.
        let outbound = session.playback_outbound(
            PlaybackAction {
                action: action::SYNC_QUEUE.to_string(),
                queue: Some(vec![track("a")]),
                ..Default::default()
            },
            1_120,
        );
        let Outbound::PlaybackAction(payload) = outbound else {
            panic!("expected playback action");
        };
        assert_eq!(payload.captured_at_server_time, 0);
        assert_eq!(payload.queue.len(), 1);

        assert_eq!(session.buffer_ready("  "), None);
        assert_eq!(
            session.buffer_ready(" t "),
            Some(Outbound::BufferReady {
                track_id: "t".to_string()
            })
        );
        assert!(
            session.suggest(track("x")).is_none(),
            "hosts cannot suggest"
        );

        let ping = session.next_ping(5_000);
        assert_eq!(
            ping,
            Outbound::Ping {
                client_time: 5_000,
                sequence: 1
            }
        );
        assert_eq!(
            session.next_ping(5_250),
            Outbound::Ping {
                client_time: 5_250,
                sequence: 2
            }
        );
    }

    #[test]
    fn server_messages_decode_into_the_session_contract() {
        let payload = proto::RoomCreatedPayload {
            room_code: "ABC".to_string(),
            user_id: "u".to_string(),
            session_token: "t".to_string(),
        };
        let message = ServerMessage::decode(msg::ROOM_CREATED, &payload.encode_to_vec()).unwrap();
        let mut session = RoomSession::new();
        session.begin_create("host");
        session.apply(message, 1_000);
        assert_eq!(session.room_code(), Some("ABC"));
    }

    #[test]
    fn user_presence_updates_and_leaves() {
        let mut session = RoomSession::new();
        room_created(&mut session);
        session.apply(
            ServerMessage::UserJoined(proto::UserJoinedPayload {
                user_id: "u2".to_string(),
                username: "two".to_string(),
            }),
            1_000,
        );
        assert_eq!(session.room().unwrap().users.len(), 2);
        session.apply(
            ServerMessage::UserDisconnected(proto::UserDisconnectedPayload {
                user_id: "u2".to_string(),
                username: "two".to_string(),
            }),
            1_000,
        );
        assert!(!session.room().unwrap().users[1].is_connected);
        session.apply(
            ServerMessage::UserReconnected(proto::UserReconnectedPayload {
                user_id: "u2".to_string(),
                username: "two".to_string(),
            }),
            1_000,
        );
        assert!(session.room().unwrap().users[1].is_connected);
        session.apply(
            ServerMessage::UserLeft(proto::UserLeftPayload {
                user_id: "u2".to_string(),
                username: "two".to_string(),
            }),
            1_000,
        );
        assert_eq!(session.room().unwrap().users.len(), 1);
    }
}
