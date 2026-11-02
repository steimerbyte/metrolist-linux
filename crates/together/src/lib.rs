//! Listen Together: wire-compatible protobuf envelopes, the server clock,
//! host-authoritative playback/queue synchronisation, a room state machine and
//! a tokio-tungstenite websocket client.
//!
//! Wire compatibility is with the Android app in
//! `metrolist/app/src/main/kotlin/com/metrolist/music/listentogether/**`; the
//! generated types come from the upstream `listentogether.proto` (field numbers
//! and types unchanged).
//!
//! ```no_run
//! use ml_together::{TogetherClient, TogetherEvent};
//!
//! let client = TogetherClient::new();
//! client.create_room("nyx");
//! for event in client.events() {
//!     match event {
//!         TogetherEvent::Session(session) => println!("{session:?}"),
//!         other => println!("{other:?}"),
//!     }
//! }
//! ```

pub mod client;
pub mod clock;
pub mod codec;
pub mod messages;
pub mod model;
mod proto;
pub mod session;
pub mod sync;

pub use client::{TogetherClient, TogetherEvent, MAX_RECONNECT_ATTEMPTS};
pub use clock::ServerClock;
pub use codec::{
    should_compress, CodecError, DecodedEnvelope, MessageCodec, COMPRESSION_THRESHOLD,
};
pub use messages::{
    action, msg, normalize_server_url, Outbound, PlaybackAction, ServerInfo, ServerMessage,
    DEFAULT_SERVER_URL, SERVERS,
};
pub use model::{song_to_track_info, track_info_to_song};
pub use proto::*;
pub use session::{
    role_after_host_change, Role, RoomSession, SessionEffect, SessionEvent, SessionSnapshot,
};
pub use sync::{
    apply_host_change, apply_playback_action, apply_sync_state, backoff_delay_ms,
    canonical_playback_queue, drift_decision, position_at, queue_from_room, queue_to_room,
    should_reconnect, should_seek_during_active_playback, upcoming_queue_items, Buffering,
    DriftDecision, PendingSync, PlaybackSnapshot, RevisionGate, ACTIVE_PLAYBACK_SYNC_TOLERANCE_MS,
    DRIFT_CHECK_INTERVAL_MS, DRIFT_CORRECTION_SPEED, HARD_SYNC_THRESHOLD_MS,
    HOST_HEARTBEAT_INTERVAL_MS, SOFT_SYNC_THRESHOLD_MS,
};
