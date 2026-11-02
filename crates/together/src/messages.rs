//! Protocol constants, decoded server messages, and outbound client messages.
//!
//! The message-type strings are the exact UTF-8 values carried in
//! `Envelope.type` by the Android app (`MessageTypes`/`PlaybackActions`).

use prost::Message;

use crate::proto;

pub mod msg {
    pub const CREATE_ROOM: &str = "create_room";
    pub const JOIN_ROOM: &str = "join_room";
    pub const LEAVE_ROOM: &str = "leave_room";
    pub const APPROVE_JOIN: &str = "approve_join";
    pub const REJECT_JOIN: &str = "reject_join";
    pub const PLAYBACK_ACTION: &str = "playback_action";
    pub const BUFFER_READY: &str = "buffer_ready";
    pub const KICK_USER: &str = "kick_user";
    pub const TRANSFER_HOST: &str = "transfer_host";
    pub const PING: &str = "ping";
    pub const CHAT: &str = "chat";
    pub const REQUEST_SYNC: &str = "request_sync";
    pub const RECONNECT: &str = "reconnect";
    pub const SUGGEST_TRACK: &str = "suggest_track";
    pub const APPROVE_SUGGESTION: &str = "approve_suggestion";
    pub const REJECT_SUGGESTION: &str = "reject_suggestion";
    pub const ROOM_CREATED: &str = "room_created";
    pub const JOIN_REQUEST: &str = "join_request";
    pub const JOIN_APPROVED: &str = "join_approved";
    pub const JOIN_REJECTED: &str = "join_rejected";
    pub const USER_JOINED: &str = "user_joined";
    pub const USER_LEFT: &str = "user_left";
    pub const SYNC_PLAYBACK: &str = "sync_playback";
    pub const BUFFER_WAIT: &str = "buffer_wait";
    pub const BUFFER_COMPLETE: &str = "buffer_complete";
    pub const ERROR: &str = "error";
    pub const PONG: &str = "pong";
    pub const HOST_CHANGED: &str = "host_changed";
    pub const KICKED: &str = "kicked";
    pub const SYNC_STATE: &str = "sync_state";
    pub const RECONNECTED: &str = "reconnected";
    pub const USER_RECONNECTED: &str = "user_reconnected";
    pub const USER_DISCONNECTED: &str = "user_disconnected";
    pub const SUGGESTION_RECEIVED: &str = "suggestion_received";
    pub const SUGGESTION_APPROVED: &str = "suggestion_approved";
    pub const SUGGESTION_REJECTED: &str = "suggestion_rejected";
}

pub mod action {
    pub const PLAY: &str = "play";
    pub const PAUSE: &str = "pause";
    pub const SEEK: &str = "seek";
    pub const SKIP_NEXT: &str = "skip_next";
    pub const SKIP_PREV: &str = "skip_prev";
    pub const CHANGE_TRACK: &str = "change_track";
    pub const QUEUE_ADD: &str = "queue_add";
    pub const QUEUE_REMOVE: &str = "queue_remove";
    pub const QUEUE_CLEAR: &str = "queue_clear";
    pub const SYNC_QUEUE: &str = "sync_queue";
    pub const SET_VOLUME: &str = "set_volume";
}

/// A community Listen Together server (mirrors `ListenTogetherServers.kt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerInfo {
    pub name: &'static str,
    pub url: &'static str,
    pub location: &'static str,
    pub operator: &'static str,
}

pub const SERVERS: [ServerInfo; 1] = [ServerInfo {
    name: "The Meowery",
    url: "wss://metroserverx.meowery.eu/ws",
    location: "Poland",
    operator: "Nyx",
}];

pub const DEFAULT_SERVER_URL: &str = SERVERS[0].url;

/// Mirrors `normalizeServerUrl`: blank or the legacy host maps to the default.
pub fn normalize_server_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return DEFAULT_SERVER_URL.to_string();
    }
    if trimmed
        .to_ascii_lowercase()
        .contains("metroserver.meowery.eu")
    {
        DEFAULT_SERVER_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

/// A decoded server -> client message.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    RoomCreated(proto::RoomCreatedPayload),
    JoinRequest(proto::JoinRequestPayload),
    JoinApproved(proto::JoinApprovedPayload),
    JoinRejected(proto::JoinRejectedPayload),
    UserJoined(proto::UserJoinedPayload),
    UserLeft(proto::UserLeftPayload),
    SyncPlayback(proto::PlaybackActionPayload),
    BufferWait(proto::BufferWaitPayload),
    BufferComplete(proto::BufferCompletePayload),
    Error(proto::ErrorPayload),
    HostChanged(proto::HostChangedPayload),
    Kicked(proto::KickedPayload),
    SyncState(proto::SyncStatePayload),
    Pong(proto::PongPayload),
    Reconnected(proto::ReconnectedPayload),
    UserReconnected(proto::UserReconnectedPayload),
    UserDisconnected(proto::UserDisconnectedPayload),
    SuggestionReceived(proto::SuggestionReceivedPayload),
    SuggestionApproved(proto::SuggestionApprovedPayload),
    SuggestionRejected(proto::SuggestionRejectedPayload),
}

impl ServerMessage {
    /// Mirrors `MessageCodec.decodePayload`: empty payloads and message types
    /// that the Android app never decodes yield `None`.
    pub fn decode(kind: &str, payload: &[u8]) -> Option<Self> {
        if payload.is_empty() {
            return None;
        }
        Some(match kind {
            msg::ROOM_CREATED => Self::RoomCreated(decode_message(payload)?),
            msg::JOIN_REQUEST => Self::JoinRequest(decode_message(payload)?),
            msg::JOIN_APPROVED => Self::JoinApproved(decode_message(payload)?),
            msg::JOIN_REJECTED => Self::JoinRejected(decode_message(payload)?),
            msg::USER_JOINED => Self::UserJoined(decode_message(payload)?),
            msg::USER_LEFT => Self::UserLeft(decode_message(payload)?),
            msg::SYNC_PLAYBACK => Self::SyncPlayback(decode_message(payload)?),
            msg::BUFFER_WAIT => Self::BufferWait(decode_message(payload)?),
            msg::BUFFER_COMPLETE => Self::BufferComplete(decode_message(payload)?),
            msg::ERROR => Self::Error(decode_message(payload)?),
            msg::HOST_CHANGED => Self::HostChanged(decode_message(payload)?),
            msg::KICKED => Self::Kicked(decode_message(payload)?),
            msg::SYNC_STATE => Self::SyncState(decode_message(payload)?),
            msg::PONG => Self::Pong(decode_message(payload)?),
            msg::RECONNECTED => Self::Reconnected(decode_message(payload)?),
            msg::USER_RECONNECTED => Self::UserReconnected(decode_message(payload)?),
            msg::USER_DISCONNECTED => Self::UserDisconnected(decode_message(payload)?),
            msg::SUGGESTION_RECEIVED => Self::SuggestionReceived(decode_message(payload)?),
            msg::SUGGESTION_APPROVED => Self::SuggestionApproved(decode_message(payload)?),
            msg::SUGGESTION_REJECTED => Self::SuggestionRejected(decode_message(payload)?),
            _ => return None,
        })
    }
}

fn decode_message<M: Message + Default>(bytes: &[u8]) -> Option<M> {
    M::decode(bytes).ok()
}

/// A message the client sends to the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Outbound {
    CreateRoom {
        username: String,
    },
    JoinRoom {
        room_code: String,
        username: String,
    },
    LeaveRoom,
    ApproveJoin {
        user_id: String,
    },
    RejectJoin {
        user_id: String,
        reason: Option<String>,
    },
    PlaybackAction(proto::PlaybackActionPayload),
    BufferReady {
        track_id: String,
    },
    KickUser {
        user_id: String,
        reason: Option<String>,
    },
    TransferHost {
        new_host_id: String,
    },
    Ping {
        client_time: i64,
        sequence: u64,
    },
    RequestSync,
    Reconnect {
        session_token: String,
    },
    SuggestTrack {
        track_info: proto::TrackInfo,
    },
    ApproveSuggestion {
        suggestion_id: String,
    },
    RejectSuggestion {
        suggestion_id: String,
        reason: Option<String>,
    },
    /// Empty `pong` envelope answering a server `ping` keepalive.
    Pong,
}

/// Host-side playback change before it becomes a wire payload. Mirrors the
/// nullable arguments of the Kotlin `PlaybackActionPayload(...)` builder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackAction {
    pub action: String,
    pub track_id: Option<String>,
    pub position: Option<i64>,
    pub track_info: Option<proto::TrackInfo>,
    pub insert_next: bool,
    pub queue: Option<Vec<proto::TrackInfo>>,
    pub queue_title: Option<String>,
    pub volume: Option<f32>,
}

impl PlaybackAction {
    pub fn new(action: &str) -> Self {
        Self {
            action: action.to_string(),
            ..Default::default()
        }
    }

    /// Build the protobuf payload. `server_time` stays 0: the server stamps it
    /// when relaying, exactly like the Android client.
    pub fn to_payload(&self, captured_at_server_time: Option<i64>) -> proto::PlaybackActionPayload {
        proto::PlaybackActionPayload {
            action: self.action.clone(),
            track_id: self.track_id.clone().unwrap_or_default(),
            position: self.position.unwrap_or(0),
            track_info: self.track_info.clone(),
            insert_next: self.insert_next,
            queue: self.queue.clone().unwrap_or_default(),
            queue_title: self.queue_title.clone().unwrap_or_default(),
            volume: self.volume.unwrap_or(1.0),
            server_time: 0,
            revision: 0,
            captured_at_server_time: captured_at_server_time.unwrap_or(0),
        }
    }
}

// ---- Presence helpers, exactly like the Kotlin extension properties ----

impl proto::PlaybackActionPayload {
    pub fn track_id_or_none(&self) -> Option<&str> {
        (!self.track_id.is_empty()).then_some(self.track_id.as_str())
    }

    pub fn position_or_none(&self) -> Option<i64> {
        let always_present = self.action == action::PLAY
            || self.action == action::PAUSE
            || self.action == action::SEEK;
        (self.position != 0 || always_present).then_some(self.position)
    }

    pub fn track_info_or_none(&self) -> Option<&proto::TrackInfo> {
        self.track_info.as_ref()
    }

    pub fn queue_title_or_none(&self) -> Option<&str> {
        (!self.queue_title.is_empty()).then_some(self.queue_title.as_str())
    }

    pub fn volume_or_none(&self) -> Option<f32> {
        (self.action == action::SET_VOLUME).then_some(self.volume)
    }

    pub fn server_time_or_none(&self) -> Option<i64> {
        (self.server_time > 0).then_some(self.server_time)
    }

    pub fn captured_at_server_time_or_none(&self) -> Option<i64> {
        (self.captured_at_server_time > 0).then_some(self.captured_at_server_time)
    }

    pub fn insert_next_or_none(&self) -> Option<bool> {
        self.insert_next.then_some(true)
    }
}

impl proto::RoomState {
    pub fn current_track_or_none(&self) -> Option<&proto::TrackInfo> {
        self.current_track.as_ref()
    }

    pub fn host(&self) -> Option<&proto::UserInfo> {
        self.users.iter().find(|user| user.user_id == self.host_id)
    }
}

impl proto::SyncStatePayload {
    pub fn current_track_or_none(&self) -> Option<&proto::TrackInfo> {
        self.current_track.as_ref()
    }
}

impl proto::SuggestionRejectedPayload {
    pub fn reason_or_none(&self) -> Option<&str> {
        (!self.reason.is_empty()).then_some(self.reason.as_str())
    }
}

impl proto::BufferWaitPayload {
    pub fn waiting_for(&self) -> &[String] {
        &self.waiting_for
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_url_normalisation_matches_android() {
        assert_eq!(normalize_server_url(""), DEFAULT_SERVER_URL);
        assert_eq!(normalize_server_url("   "), DEFAULT_SERVER_URL);
        assert_eq!(
            normalize_server_url("wss://metroserver.meowery.eu/ws"),
            DEFAULT_SERVER_URL
        );
        assert_eq!(
            normalize_server_url(" wss://metroserverx.meowery.eu/ws "),
            DEFAULT_SERVER_URL
        );
        assert_eq!(
            normalize_server_url("ws://127.0.0.1:8080/ws"),
            "ws://127.0.0.1:8080/ws"
        );
    }

    #[test]
    fn presence_helpers_mirror_the_kotlin_extensions() {
        let playing = proto::PlaybackActionPayload {
            action: action::PLAY.to_string(),
            position: 0,
            ..Default::default()
        };
        assert_eq!(
            playing.position_or_none(),
            Some(0),
            "play always carries a position"
        );

        let queue_add = proto::PlaybackActionPayload {
            action: action::QUEUE_ADD.to_string(),
            position: 0,
            ..Default::default()
        };
        assert_eq!(queue_add.position_or_none(), None);
        let queue_add_at = proto::PlaybackActionPayload {
            position: 12,
            ..queue_add
        };
        assert_eq!(queue_add_at.position_or_none(), Some(12));

        let full = proto::PlaybackActionPayload {
            action: action::SET_VOLUME.to_string(),
            track_id: "track".to_string(),
            track_info: Some(proto::TrackInfo::default()),
            queue_title: "Queue".to_string(),
            volume: 0.5,
            server_time: 9_000,
            captured_at_server_time: 8_950,
            insert_next: true,
            ..Default::default()
        };
        assert_eq!(full.track_id_or_none(), Some("track"));
        assert!(full.track_info_or_none().is_some());
        assert_eq!(full.queue_title_or_none(), Some("Queue"));
        assert_eq!(full.volume_or_none(), Some(0.5));
        assert_eq!(full.server_time_or_none(), Some(9_000));
        assert_eq!(full.captured_at_server_time_or_none(), Some(8_950));
        assert_eq!(full.insert_next_or_none(), Some(true));

        let empty = proto::PlaybackActionPayload::default();
        assert_eq!(empty.track_id_or_none(), None);
        assert!(empty.track_info_or_none().is_none());
        assert_eq!(empty.queue_title_or_none(), None);
        assert_eq!(
            empty.volume_or_none(),
            None,
            "volume only exists for set_volume"
        );
        assert_eq!(empty.server_time_or_none(), None);
        assert_eq!(empty.captured_at_server_time_or_none(), None);
        assert_eq!(empty.insert_next_or_none(), None, "only true is present");

        let room = proto::RoomState {
            host_id: "u".to_string(),
            users: vec![proto::UserInfo {
                user_id: "u".to_string(),
                username: "host".to_string(),
                is_host: true,
                is_connected: true,
            }],
            current_track: Some(proto::TrackInfo::default()),
            ..Default::default()
        };
        assert!(room.current_track_or_none().is_some());
        assert_eq!(room.host().map(|user| user.username.as_str()), Some("host"));

        assert_eq!(
            proto::SuggestionRejectedPayload::default().reason_or_none(),
            None
        );
        assert_eq!(
            proto::SuggestionRejectedPayload {
                reason: "no".to_string(),
                ..Default::default()
            }
            .reason_or_none(),
            Some("no")
        );
    }

    #[test]
    fn playback_action_payload_uses_proto3_defaults() {
        let minimum = PlaybackAction {
            action: action::PLAY.to_string(),
            ..Default::default()
        }
        .to_payload(None);
        assert_eq!(minimum.action, "play");
        assert!(minimum.track_id.is_empty());
        assert_eq!(minimum.position, 0);
        assert!(minimum.track_info.is_none());
        assert!(!minimum.insert_next);
        assert!(minimum.queue.is_empty());
        assert!(minimum.queue_title.is_empty());
        assert_eq!(minimum.volume, 1.0);
        assert_eq!(minimum.server_time, 0);
        assert_eq!(minimum.revision, 0);
        assert_eq!(minimum.captured_at_server_time, 0);

        let full = PlaybackAction {
            action: action::SEEK.to_string(),
            track_id: Some("t".to_string()),
            position: Some(50),
            insert_next: true,
            queue: Some(vec![proto::TrackInfo::default()]),
            queue_title: Some("q".to_string()),
            volume: Some(0.25),
            ..Default::default()
        }
        .to_payload(Some(8_950));
        assert_eq!(full.track_id, "t");
        assert_eq!(full.position, 50);
        assert!(full.insert_next);
        assert_eq!(full.queue.len(), 1);
        assert_eq!(full.queue_title, "q");
        assert_eq!(full.volume, 0.25);
        assert_eq!(full.captured_at_server_time, 8_950);
    }
}
