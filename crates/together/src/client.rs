//! Tokio-tungstenite websocket client.
//!
//! The client owns a background current-thread runtime on its own OS thread.
//! The UI thread only pushes commands (non-blocking) and drains a crossbeam
//! event channel; it is never blocked on network I/O.
//!
//! Behaviour mirrors the Android `ListenTogetherClient`: heartbeat pings at
//! 25 s (four fast pings right after connect), pong-based clock estimation,
//! reconnect with the session token and exponential backoff, and role-guarded
//! commands. Only one connection attempt is made; a failed attempt backs off.

use std::future::pending;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{sleep_until, Instant as TokioInstant};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use crate::codec::MessageCodec;
use crate::messages::{
    msg, normalize_server_url, Outbound, PlaybackAction, ServerMessage, DEFAULT_SERVER_URL,
};
use crate::proto;
use crate::session::{RoomSession, SessionEffect, SessionEvent};
use crate::sync;

/// Android `PING_INTERVAL_MS`.
pub const PING_INTERVAL_MS: u64 = 25_000;
/// Android `INITIAL_PING_INTERVAL_MS` (the quick clock-sync burst).
pub const INITIAL_PING_INTERVAL_MS: u64 = 250;
/// Android `MAX_RECONNECT_ATTEMPTS`.
pub const MAX_RECONNECT_ATTEMPTS: u32 = sync::MAX_RECONNECT_ATTEMPTS;
/// Android `connectTimeout`.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// `session_not_found` rejoin delay.
pub const REJOIN_DELAY: Duration = Duration::from_millis(500);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Connection-level plus room-level events.
#[derive(Debug, Clone, PartialEq)]
pub enum TogetherEvent {
    Connected,
    Disconnected { will_reconnect: bool },
    Reconnecting { attempt: u32, max_attempts: u32 },
    ConnectionError(String),
    Session(Box<SessionEvent>),
}

#[derive(Debug)]
enum Command {
    Connect,
    CreateRoom { username: String },
    JoinRoom { room_code: String, username: String },
    Playback(PlaybackAction),
    Send(Outbound),
    SuggestTrack(proto::TrackInfo),
    BufferReady(String),
    Leave,
    Disconnect,
    ForceReconnect,
    Shutdown,
}

/// Handle for the Listen Together connection. Dropping it stops the client.
pub struct TogetherClient {
    commands: UnboundedSender<Command>,
    events: Receiver<TogetherEvent>,
}

impl Default for TogetherClient {
    fn default() -> Self {
        Self::new()
    }
}

impl TogetherClient {
    /// Connect to the default community server.
    pub fn new() -> Self {
        Self::with_server(DEFAULT_SERVER_URL)
    }

    /// Connect to `server_url` (blank or the legacy host normalises to the
    /// default server, matching `normalizeServerUrl`).
    pub fn with_server(server_url: impl Into<String>) -> Self {
        // Match the application's rustls provider (reqwest uses aws-lc-rs).
        // Installing explicitly is harmless when rustls already picked it and
        // avoids the auto-detection panic in builds with several TLS stacks.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let url = normalize_server_url(&server_url.into());
        let (command_tx, command_rx) = unbounded_channel();
        let (event_tx, event_rx) = unbounded();
        let actor_command_tx = command_tx.clone();

        // Detached runtime thread: process exit terminates it, and dropping the
        // handle only asks the actor to stop. No UI thread ever joins.
        let _ = std::thread::Builder::new()
            .name("ml-together".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build Listen Together runtime");
                runtime
                    .block_on(ClientActor::new(url, command_rx, event_tx, actor_command_tx).run());
            });

        Self {
            commands: command_tx,
            events: event_rx,
        }
    }

    // ---- Public API ----

    pub fn connect(&self) {
        self.send(Command::Connect);
    }

    /// Create a room; connects first when necessary.
    pub fn create_room(&self, username: &str) {
        self.send(Command::CreateRoom {
            username: username.to_string(),
        });
    }

    /// Join a room; connects first when necessary. The server receives the
    /// upper-cased code, like the Android client.
    pub fn join_room(&self, room_code: &str, username: &str) {
        self.send(Command::JoinRoom {
            room_code: room_code.to_ascii_uppercase(),
            username: username.to_string(),
        });
    }

    pub fn approve_join(&self, user_id: &str) {
        self.send(Command::Send(Outbound::ApproveJoin {
            user_id: user_id.to_string(),
        }));
    }

    pub fn reject_join(&self, user_id: &str, reason: Option<&str>) {
        self.send(Command::Send(Outbound::RejectJoin {
            user_id: user_id.to_string(),
            reason: reason.map(str::to_owned),
        }));
    }

    pub fn kick_user(&self, user_id: &str, reason: Option<&str>) {
        self.send(Command::Send(Outbound::KickUser {
            user_id: user_id.to_string(),
            reason: reason.map(str::to_owned),
        }));
    }

    pub fn transfer_host(&self, new_host_id: &str) {
        self.send(Command::Send(Outbound::TransferHost {
            new_host_id: new_host_id.to_string(),
        }));
    }

    /// Push a host-authoritative playback change.
    pub fn send_playback(&self, action: PlaybackAction) {
        self.send(Command::Playback(action));
    }

    pub fn buffer_ready(&self, track_id: &str) {
        self.send(Command::BufferReady(track_id.to_string()));
    }

    /// Suggest a track to the host (guests only).
    pub fn suggest_track(&self, track: proto::TrackInfo) {
        self.send(Command::SuggestTrack(track));
    }

    pub fn approve_suggestion(&self, suggestion_id: &str) {
        self.send(Command::Send(Outbound::ApproveSuggestion {
            suggestion_id: suggestion_id.to_string(),
        }));
    }

    pub fn reject_suggestion(&self, suggestion_id: &str, reason: Option<&str>) {
        self.send(Command::Send(Outbound::RejectSuggestion {
            suggestion_id: suggestion_id.to_string(),
            reason: reason.map(str::to_owned),
        }));
    }

    pub fn request_sync(&self) {
        self.send(Command::Send(Outbound::RequestSync));
    }

    /// Leave the room but keep the websocket open.
    pub fn leave(&self) {
        self.send(Command::Leave);
    }

    /// Close the socket and forget the session.
    pub fn disconnect(&self) {
        self.send(Command::Disconnect);
    }

    pub fn force_reconnect(&self) {
        self.send(Command::ForceReconnect);
    }

    /// Clone of the event channel (crossbeam receivers are multi-consumer).
    pub fn events(&self) -> Receiver<TogetherEvent> {
        self.events.clone()
    }

    pub fn try_recv_event(&self) -> Option<TogetherEvent> {
        self.events.try_recv().ok()
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

impl Drop for TogetherClient {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

struct ClientActor {
    url: String,
    socket: Option<Socket>,
    commands: UnboundedReceiver<Command>,
    command_tx: UnboundedSender<Command>,
    events: Sender<TogetherEvent>,
    codec: MessageCodec,
    session: RoomSession,
    pending_action: Option<Outbound>,
    reconnect_attempts: u32,
    reconnect_at: Option<TokioInstant>,
    ping_at: Option<TokioInstant>,
    ping_burst_left: u32,
    epoch: Instant,
    shutting_down: bool,
}

impl ClientActor {
    fn new(
        url: String,
        commands: UnboundedReceiver<Command>,
        events: Sender<TogetherEvent>,
        command_tx: UnboundedSender<Command>,
    ) -> Self {
        Self {
            url,
            socket: None,
            commands,
            command_tx,
            events,
            codec: MessageCodec::new(true),
            session: RoomSession::new(),
            pending_action: None,
            reconnect_attempts: 0,
            reconnect_at: None,
            ping_at: None,
            ping_burst_left: 0,
            epoch: Instant::now(),
            shutting_down: false,
        }
    }

    fn elapsed_ms(&self) -> i64 {
        self.epoch.elapsed().as_millis() as i64
    }

    fn emit(&self, event: TogetherEvent) {
        let _ = self.events.send(event);
    }

    fn emit_session(&self, event: SessionEvent) {
        self.emit(TogetherEvent::Session(Box::new(event)));
    }

    fn emit_snapshot(&self) {
        self.emit_session(SessionEvent::Snapshot(self.session.snapshot()));
    }

    async fn run(mut self) {
        self.connect().await;

        loop {
            if self.shutting_down {
                break;
            }
            let deadline = self.next_deadline();
            if let Some(mut socket) = self.socket.take() {
                tokio::select! {
                    biased;
                    command = self.commands.recv() => {
                        self.socket = Some(socket);
                        match command {
                            Some(command) => {
                                if !self.handle_command(command).await {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    frame = socket.next() => {
                        self.socket = Some(socket);
                        self.handle_frame(frame).await;
                    }
                    _ = wake_at(deadline) => {
                        self.socket = Some(socket);
                        self.on_timer().await;
                    }
                }
            } else {
                tokio::select! {
                    biased;
                    command = self.commands.recv() => {
                        match command {
                            Some(command) => {
                                if !self.handle_command(command).await {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    _ = wake_at(deadline) => self.on_timer().await,
                }
            }
        }

        if let Some(mut socket) = self.socket.take() {
            let _ = socket.close(None).await;
        }
    }

    fn next_deadline(&self) -> Option<TokioInstant> {
        if self.socket.is_some() {
            self.ping_at
        } else {
            self.reconnect_at
        }
    }

    async fn on_timer(&mut self) {
        if self.socket.is_some() {
            self.ping_at = None;
            let ping = {
                let elapsed = self.elapsed_ms();
                self.session.next_ping(elapsed)
            };
            self.send(ping).await;
            let delay = if self.ping_burst_left > 0 {
                self.ping_burst_left -= 1;
                INITIAL_PING_INTERVAL_MS
            } else {
                PING_INTERVAL_MS
            };
            if self.socket.is_some() {
                self.ping_at = Some(TokioInstant::now() + Duration::from_millis(delay));
            }
        } else if self.reconnect_at.take().is_some() {
            self.connect().await;
        }
    }

    async fn connect(&mut self) {
        self.reconnect_at = None;
        if self.socket.is_some() {
            return;
        }
        let result = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(self.url.as_str())).await;
        match result {
            Ok(Ok((socket, _response))) => {
                self.socket = Some(socket);
                self.session.on_connected();
                self.reconnect_attempts = 0;
                self.emit(TogetherEvent::Connected);
                self.ping_burst_left = 3;
                self.ping_at = Some(TokioInstant::now());
                if let (Some(token), Some(_room)) = (
                    self.session.session_token().map(str::to_owned),
                    self.session.room_code(),
                ) {
                    self.send(Outbound::Reconnect {
                        session_token: token,
                    })
                    .await;
                } else if let Some(action) = self.pending_action.take() {
                    self.send(action).await;
                }
            }
            Ok(Err(error)) => {
                self.emit(TogetherEvent::ConnectionError(error.to_string()));
                self.schedule_reconnect();
            }
            Err(_elapsed) => {
                self.emit(TogetherEvent::ConnectionError(format!(
                    "connection timed out after {}s",
                    CONNECT_TIMEOUT.as_secs()
                )));
                self.schedule_reconnect();
            }
        }
    }

    fn schedule_reconnect(&mut self) {
        self.socket = None;
        self.ping_at = None;
        self.session.on_disconnected();
        let has_session = self.session.session_token().is_some()
            || self.session.is_in_room()
            || self.pending_action.is_some();
        if has_session && sync::should_reconnect(self.reconnect_attempts) {
            self.reconnect_attempts += 1;
            let delay = sync::backoff_delay_ms(self.reconnect_attempts, rand::random::<f64>());
            self.reconnect_at = Some(TokioInstant::now() + Duration::from_millis(delay));
            self.emit(TogetherEvent::Reconnecting {
                attempt: self.reconnect_attempts,
                max_attempts: MAX_RECONNECT_ATTEMPTS,
            });
            self.emit(TogetherEvent::Disconnected {
                will_reconnect: true,
            });
        } else {
            self.reconnect_at = None;
            self.emit(TogetherEvent::Disconnected {
                will_reconnect: false,
            });
            if has_session {
                self.emit(TogetherEvent::ConnectionError(format!(
                    "connection failed after {MAX_RECONNECT_ATTEMPTS} attempts"
                )));
            }
        }
    }

    async fn send(&mut self, outbound: Outbound) -> bool {
        let bytes = self.codec.encode_outbound(&outbound);
        let result = match self.socket.as_mut() {
            Some(socket) => socket.send(Message::binary(bytes)).await,
            None => return false,
        };
        match result {
            Ok(()) => true,
            Err(error) => {
                self.emit(TogetherEvent::ConnectionError(error.to_string()));
                self.schedule_reconnect();
                false
            }
        }
    }

    async fn handle_frame(
        &mut self,
        frame: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) {
        match frame {
            Some(Ok(Message::Binary(bytes))) => self.handle_envelope(&bytes).await,
            Some(Ok(Message::Text(text))) => self.handle_envelope(text.as_bytes()).await,
            Some(Ok(Message::Ping(payload))) => {
                if let Some(socket) = self.socket.as_mut() {
                    let _ = socket.send(Message::Pong(payload)).await;
                }
            }
            Some(Ok(Message::Close(frame))) => {
                let reason = frame
                    .map(|frame| frame.reason.to_string())
                    .unwrap_or_default();
                tracing::info!(reason, "Listen Together server closed the connection");
                self.schedule_reconnect();
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                self.emit(TogetherEvent::ConnectionError(error.to_string()));
                self.schedule_reconnect();
            }
            None => {
                self.schedule_reconnect();
            }
        }
    }

    async fn handle_envelope(&mut self, bytes: &[u8]) {
        let envelope = match self.codec.decode(bytes) {
            Ok(envelope) => envelope,
            Err(error) => {
                tracing::warn!(%error, "Listen Together envelope failed to decode");
                return;
            }
        };
        let Some(message) = ServerMessage::decode(&envelope.kind, &envelope.payload) else {
            if envelope.kind == msg::PING {
                // Server keepalive: answer with an empty pong envelope.
                self.send(Outbound::Pong).await;
            } else {
                tracing::warn!(kind = %envelope.kind, "unknown Listen Together message type");
            }
            return;
        };

        let effects = self.session.apply(message, self.elapsed_ms());
        for effect in effects {
            match effect {
                SessionEffect::Send(outbound) => {
                    self.send(outbound).await;
                }
                SessionEffect::Event(event) => self.emit_session(event),
                SessionEffect::ClearSession => {
                    // Room state already cleared; the crate keeps no persistence.
                }
                SessionEffect::ScheduleRejoin {
                    room_code,
                    username,
                } => {
                    self.session.forget_server_session();
                    self.emit_snapshot();
                    let commands = self.command_tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(REJOIN_DELAY).await;
                        let _ = commands.send(Command::JoinRoom {
                            room_code,
                            username,
                        });
                    });
                }
            }
        }
    }

    async fn handle_command(&mut self, command: Command) -> bool {
        match command {
            Command::Connect => {
                if self.socket.is_none() {
                    self.connect().await;
                }
            }
            Command::CreateRoom { username } => {
                self.session.begin_create(&username);
                self.emit_snapshot();
                let outbound = Outbound::CreateRoom {
                    username: username.clone(),
                };
                if self.socket.is_some() {
                    self.send(outbound).await;
                } else {
                    self.pending_action = Some(outbound);
                    self.connect().await;
                }
            }
            Command::JoinRoom {
                room_code,
                username,
            } => {
                let room_code = room_code.to_ascii_uppercase();
                self.session.begin_join(&room_code, &username);
                self.emit_snapshot();
                let outbound = Outbound::JoinRoom {
                    room_code,
                    username,
                };
                if self.socket.is_some() {
                    self.send(outbound).await;
                } else {
                    self.pending_action = Some(outbound);
                    self.connect().await;
                }
            }
            Command::Playback(action) => {
                match self.session.host_playback(action, self.elapsed_ms()) {
                    Some(outbound) => {
                        self.send(outbound).await;
                    }
                    None => tracing::debug!("ignoring playback action: not the host"),
                }
            }
            Command::Send(outbound) => {
                if allowed(&self.session, &outbound) {
                    self.send(outbound).await;
                } else {
                    tracing::debug!("ignoring command not allowed for the current role");
                }
            }
            Command::SuggestTrack(track) => match self.session.suggest(track) {
                Some(outbound) => {
                    self.send(outbound).await;
                }
                None => tracing::debug!("ignoring suggestion: only guests may suggest"),
            },
            Command::BufferReady(track_id) => {
                if let Some(outbound) = self.session.buffer_ready(&track_id) {
                    self.send(outbound).await;
                }
            }
            Command::Leave => {
                if self.socket.is_some() {
                    self.send(Outbound::LeaveRoom).await;
                }
                self.session.clear_session();
                self.pending_action = None;
                self.emit_snapshot();
            }
            Command::Disconnect => {
                self.reconnect_at = None;
                self.pending_action = None;
                if let Some(mut socket) = self.socket.take() {
                    let _ = socket.close(None).await;
                }
                self.session.clear_session();
                self.emit_snapshot();
                self.emit(TogetherEvent::Disconnected {
                    will_reconnect: false,
                });
            }
            Command::ForceReconnect => {
                self.reconnect_attempts = 0;
                if let Some(mut socket) = self.socket.take() {
                    let _ = socket.close(None).await;
                }
                self.session.on_disconnected();
                self.connect().await;
            }
            Command::Shutdown => {
                self.shutting_down = true;
                return false;
            }
        }
        true
    }
}

/// Resolve to `()` at the deadline, or never when there is no deadline.
async fn wake_at(deadline: Option<TokioInstant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => pending().await,
    }
}

/// Role guards, mirroring the Kotlin client checks.
fn allowed(session: &RoomSession, outbound: &Outbound) -> bool {
    match outbound {
        Outbound::ApproveJoin { .. }
        | Outbound::RejectJoin { .. }
        | Outbound::KickUser { .. }
        | Outbound::TransferHost { .. }
        | Outbound::PlaybackAction(_)
        | Outbound::ApproveSuggestion { .. }
        | Outbound::RejectSuggestion { .. } => session.is_host(),
        Outbound::SuggestTrack { .. } => session.is_in_room() && !session.is_host(),
        Outbound::RequestSync => session.is_in_room(),
        Outbound::BufferReady { track_id } => !track_id.trim().is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::action;

    #[test]
    fn connection_timing_constants_match_android() {
        assert_eq!(PING_INTERVAL_MS, 25_000);
        assert_eq!(INITIAL_PING_INTERVAL_MS, 250);
        assert_eq!(MAX_RECONNECT_ATTEMPTS, 15);
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(30));
    }

    #[test]
    fn host_and_guest_commands_are_guarded() {
        let mut session = RoomSession::new();
        assert!(!allowed(
            &session,
            &Outbound::ApproveJoin {
                user_id: "u".into()
            }
        ));

        session.begin_create("host");
        session.apply(
            ServerMessage::RoomCreated(proto::RoomCreatedPayload {
                room_code: "R".into(),
                user_id: "u".into(),
                session_token: "t".into(),
            }),
            0,
        );
        assert!(allowed(
            &session,
            &Outbound::PlaybackAction(proto::PlaybackActionPayload {
                action: action::PLAY.into(),
                ..Default::default()
            })
        ));
        assert!(!allowed(
            &session,
            &Outbound::SuggestTrack {
                track_info: proto::TrackInfo::default()
            }
        ));
        assert!(allowed(&session, &Outbound::RequestSync));
        assert!(!allowed(
            &session,
            &Outbound::BufferReady {
                track_id: "   ".into()
            }
        ));
        assert!(allowed(
            &session,
            &Outbound::BufferReady {
                track_id: "t".into()
            }
        ));

        session.begin_join("R", "guest");
        session.apply(
            ServerMessage::JoinApproved(proto::JoinApprovedPayload {
                room_code: "R".into(),
                user_id: "g".into(),
                session_token: "t".into(),
                state: Some(proto::RoomState::default()),
            }),
            0,
        );
        assert!(!allowed(
            &session,
            &Outbound::TransferHost {
                new_host_id: "g".into()
            }
        ));
        assert!(allowed(
            &session,
            &Outbound::SuggestTrack {
                track_info: proto::TrackInfo::default()
            }
        ));
    }
}
