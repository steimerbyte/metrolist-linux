//! `Envelope` framing with optional gzip payload compression.
//!
//! Wire format is the Android `MessageCodec`: a protobuf `Envelope` whose
//! `payload` is the serialized message, gzipped when compression is enabled,
//! the payload is larger than [`COMPRESSION_THRESHOLD`] bytes, and gzip made it
//! smaller. Decode never panics: a corrupt gzip stream falls back to the raw
//! bytes, and a corrupt envelope is an error.

use std::io::{Read, Write};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use prost::Message;

use crate::messages::{msg, Outbound, ServerMessage};
use crate::proto;

/// Kotlin `COMPRESSION_THRESHOLD`.
pub const COMPRESSION_THRESHOLD: usize = 100;

/// A decoded envelope; `payload` is already decompressed when `compressed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedEnvelope {
    pub kind: String,
    pub payload: Vec<u8>,
    pub compressed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),
}

/// Encodes and decodes Listen Together envelopes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessageCodec {
    compression_enabled: bool,
}

impl MessageCodec {
    pub fn new(compression_enabled: bool) -> Self {
        Self {
            compression_enabled,
        }
    }

    pub fn compression_enabled(&self) -> bool {
        self.compression_enabled
    }

    pub fn set_compression_enabled(&mut self, enabled: bool) {
        self.compression_enabled = enabled;
    }

    /// Encode a typed message into an envelope.
    pub fn encode<M: Message>(&self, kind: &str, message: &M) -> Vec<u8> {
        self.encode_bytes(kind, message.encode_to_vec())
    }

    /// Encode a type-only message (e.g. `leave_room`, `request_sync`) with an
    /// empty payload.
    pub fn encode_empty(&self, kind: &str) -> Vec<u8> {
        self.encode_bytes(kind, Vec::new())
    }

    pub fn encode_outbound(&self, outbound: &Outbound) -> Vec<u8> {
        match outbound {
            Outbound::CreateRoom { username } => self.encode(
                msg::CREATE_ROOM,
                &proto::CreateRoomPayload {
                    username: username.clone(),
                },
            ),
            Outbound::JoinRoom {
                room_code,
                username,
            } => self.encode(
                msg::JOIN_ROOM,
                &proto::JoinRoomPayload {
                    room_code: room_code.clone(),
                    username: username.clone(),
                },
            ),
            Outbound::LeaveRoom => self.encode_empty(msg::LEAVE_ROOM),
            Outbound::ApproveJoin { user_id } => self.encode(
                msg::APPROVE_JOIN,
                &proto::ApproveJoinPayload {
                    user_id: user_id.clone(),
                },
            ),
            Outbound::RejectJoin { user_id, reason } => self.encode(
                msg::REJECT_JOIN,
                &proto::RejectJoinPayload {
                    user_id: user_id.clone(),
                    reason: reason.clone().unwrap_or_default(),
                },
            ),
            Outbound::PlaybackAction(payload) => self.encode(msg::PLAYBACK_ACTION, payload),
            Outbound::BufferReady { track_id } => self.encode(
                msg::BUFFER_READY,
                &proto::BufferReadyPayload {
                    track_id: track_id.clone(),
                },
            ),
            Outbound::KickUser { user_id, reason } => self.encode(
                msg::KICK_USER,
                &proto::KickUserPayload {
                    user_id: user_id.clone(),
                    reason: reason.clone().unwrap_or_default(),
                },
            ),
            Outbound::TransferHost { new_host_id } => self.encode(
                msg::TRANSFER_HOST,
                &proto::TransferHostPayload {
                    new_host_id: new_host_id.clone(),
                },
            ),
            Outbound::Ping {
                client_time,
                sequence,
            } => self.encode(
                msg::PING,
                &proto::PingPayload {
                    client_time: *client_time,
                    sequence: *sequence,
                },
            ),
            Outbound::RequestSync => self.encode_empty(msg::REQUEST_SYNC),
            Outbound::Reconnect { session_token } => self.encode(
                msg::RECONNECT,
                &proto::ReconnectPayload {
                    session_token: session_token.clone(),
                },
            ),
            Outbound::SuggestTrack { track_info } => self.encode(
                msg::SUGGEST_TRACK,
                &proto::SuggestTrackPayload {
                    track_info: Some(track_info.clone()),
                },
            ),
            Outbound::ApproveSuggestion { suggestion_id } => self.encode(
                msg::APPROVE_SUGGESTION,
                &proto::ApproveSuggestionPayload {
                    suggestion_id: suggestion_id.clone(),
                },
            ),
            Outbound::RejectSuggestion {
                suggestion_id,
                reason,
            } => self.encode(
                msg::REJECT_SUGGESTION,
                &proto::RejectSuggestionPayload {
                    suggestion_id: suggestion_id.clone(),
                    reason: reason.clone().unwrap_or_default(),
                },
            ),
            Outbound::Pong => self.encode_empty(msg::PONG),
        }
    }

    fn encode_bytes(&self, kind: &str, mut payload: Vec<u8>) -> Vec<u8> {
        let mut compressed = false;
        if self.compression_enabled && payload.len() > COMPRESSION_THRESHOLD {
            if let Some(candidate) = gzip(&payload) {
                if should_compress(payload.len(), candidate.len()) {
                    payload = candidate;
                    compressed = true;
                }
            }
        }
        proto::Envelope {
            r#type: kind.to_string(),
            payload,
            compressed,
        }
        .encode_to_vec()
    }

    pub fn decode(&self, data: &[u8]) -> Result<DecodedEnvelope, CodecError> {
        let envelope = proto::Envelope::decode(data)?;
        let mut payload = envelope.payload;
        if envelope.compressed {
            if let Some(raw) = gunzip(&payload) {
                payload = raw;
            }
        }
        Ok(DecodedEnvelope {
            kind: envelope.r#type,
            payload,
            compressed: envelope.compressed,
        })
    }

    /// Decode an envelope and its typed payload in one step.
    ///
    /// Returns `None` for unknown message types, empty payloads, malformed
    /// protobuf payloads, and corrupt envelopes.
    pub fn decode_server_message(&self, data: &[u8]) -> Option<ServerMessage> {
        let envelope = self.decode(data).ok()?;
        ServerMessage::decode(&envelope.kind, &envelope.payload)
    }
}

/// Compression policy for a payload that the caller already decided is worth
/// trying: gzip only when strictly smaller. Pure and total.
pub fn should_compress(original_len: usize, compressed_len: usize) -> bool {
    compressed_len < original_len
}

fn gzip(data: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).ok()?;
    encoder.finish().ok()
}

fn gunzip(data: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    GzDecoder::new(data).read_to_end(&mut output).ok()?;
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codec() -> MessageCodec {
        MessageCodec::new(true)
    }

    #[test]
    fn ping_envelope_golden_bytes_match_the_wire_format() {
        let bytes = MessageCodec::new(false).encode(
            msg::PING,
            &proto::PingPayload {
                client_time: 1_000,
                sequence: 3,
            },
        );
        // field 1 string "ping", field 2 bytes PingPayload{client_time=1000,
        // sequence=3}. `compressed = false` is the proto3 default and omitted.
        assert_eq!(
            bytes,
            vec![0x0A, 0x04, b'p', b'i', b'n', b'g', 0x12, 0x05, 0x08, 0xE8, 0x07, 0x10, 0x03,]
        );

        let envelope = MessageCodec::new(false).decode(&bytes).unwrap();
        assert_eq!(envelope.kind, "ping");
        assert!(!envelope.compressed);
        assert_eq!(
            proto::PingPayload::decode(&envelope.payload[..]).unwrap(),
            proto::PingPayload {
                client_time: 1_000,
                sequence: 3,
            }
        );
    }

    #[test]
    fn playback_timing_fields_survive_a_round_trip() {
        let payload = proto::PlaybackActionPayload {
            action: crate::messages::action::PLAY.to_string(),
            track_id: "track".to_string(),
            position: 1_234,
            server_time: 9_000,
            revision: 12,
            captured_at_server_time: 8_950,
            ..Default::default()
        };

        let envelope = codec()
            .decode(&codec().encode(msg::PLAYBACK_ACTION, &payload))
            .unwrap();
        assert_eq!(envelope.kind, msg::PLAYBACK_ACTION);
        let decoded = ServerMessage::decode(msg::SYNC_PLAYBACK, &envelope.payload).unwrap();
        let ServerMessage::SyncPlayback(decoded) = decoded else {
            panic!("expected sync_playback");
        };
        assert_eq!(decoded, payload);
    }

    #[test]
    fn timestamped_ping_is_encoded_and_pong_is_decoded() {
        let ping = proto::PingPayload {
            client_time: 1_000,
            sequence: 3,
        };
        let (kind, payload) = {
            let envelope = MessageCodec::new(false)
                .decode(&MessageCodec::new(false).encode(msg::PING, &ping))
                .unwrap();
            (envelope.kind, envelope.payload)
        };
        assert_eq!(kind, msg::PING);
        assert_eq!(proto::PingPayload::decode(&payload[..]).unwrap(), ping);

        let pong = proto::PongPayload {
            client_time: 1_000,
            server_receive_time: 10_000,
            server_send_time: 10_001,
            sequence: 3,
        };
        let bytes = proto::Envelope {
            r#type: msg::PONG.to_string(),
            payload: pong.encode_to_vec(),
            compressed: false,
        }
        .encode_to_vec();
        let decoded = codec().decode_server_message(&bytes).unwrap();
        assert_eq!(decoded, ServerMessage::Pong(pong));
        assert!(pong.server_send_time >= pong.server_receive_time);
    }

    #[test]
    fn playback_action_golden_bytes_match_the_android_field_numbers() {
        let payload = proto::PlaybackActionPayload {
            action: crate::messages::action::PLAY.to_string(),
            position: 1_234,
            server_time: 9_000,
            revision: 12,
            captured_at_server_time: 8_950,
            ..Default::default()
        };
        let bytes = MessageCodec::new(false).encode(msg::PLAYBACK_ACTION, &payload);

        // Envelope: type + payload. Payload fields in tag order:
        //   1 action "play"           -> 0A 04 play
        //   3 position 1234           -> 18 D2 09
        //   9 server_time 9000        -> 48 A8 46
        //  10 revision 12             -> 50 0C
        //  11 captured_at 8950        -> 58 F6 45
        let expected = vec![
            0x0A, 0x0F, b'p', b'l', b'a', b'y', b'b', b'a', b'c', b'k', b'_', b'a', b'c', b't',
            b'i', b'o', b'n', 0x12, 0x11, 0x0A, 0x04, b'p', b'l', b'a', b'y', 0x18, 0xD2, 0x09,
            0x48, 0xA8, 0x46, 0x50, 0x0C, 0x58, 0xF6, 0x45,
        ];
        assert_eq!(bytes, expected);

        // And it round-trips through the receive path as sync_playback.
        let decoded = codec().decode_server_message(&codec().encode(msg::SYNC_PLAYBACK, &payload));
        assert_eq!(decoded, Some(ServerMessage::SyncPlayback(payload)));
    }

    #[test]
    fn compression_policy_requires_a_strictly_smaller_payload() {
        assert!(should_compress(101, 100));
        assert!(!should_compress(100, 100));
        assert!(!should_compress(100, 101));
        assert!(!should_compress(0, 0));
    }

    #[test]
    fn compressible_payloads_above_the_threshold_are_gzipped() {
        let track = proto::TrackInfo {
            id: "x".repeat(400),
            title: "x".repeat(400),
            ..Default::default()
        };
        let bytes = codec().encode(msg::PLAYBACK_ACTION, &track);
        let envelope = codec().decode(&bytes).unwrap();
        assert!(
            envelope.compressed,
            "large repetitive payload must be gzipped"
        );
        assert_eq!(
            proto::TrackInfo::decode(&envelope.payload[..]).unwrap(),
            track
        );
        // The raw envelope declares the compressed flag (field 3, varint 1).
        assert!(bytes.windows(2).any(|pair| pair == [0x18, 0x01]));
    }

    #[test]
    fn small_or_incompressible_payloads_stay_uncompressed() {
        let small = proto::PingPayload {
            client_time: 1,
            sequence: 1,
        };
        let envelope = codec().decode(&codec().encode(msg::PING, &small)).unwrap();
        assert!(!envelope.compressed);

        // 120 random-ish bytes do not get smaller under gzip.
        let noisy: Vec<u8> = (0u32..120).map(|i| (i * 37 % 251) as u8).collect();
        let envelope = codec()
            .decode(&codec().encode_bytes(msg::PING, noisy.clone()))
            .unwrap();
        assert!(!envelope.compressed);
        assert_eq!(envelope.payload, noisy);
    }

    #[test]
    fn corrupt_gzip_falls_back_to_the_raw_payload() {
        let raw = vec![0xAB; 200];
        let bytes = proto::Envelope {
            r#type: msg::PING.to_string(),
            payload: raw.clone(),
            compressed: true,
        }
        .encode_to_vec();
        let envelope = codec().decode(&bytes).unwrap();
        assert!(envelope.compressed);
        assert_eq!(envelope.payload, raw);
    }

    #[test]
    fn unknown_types_and_corrupt_bytes_do_not_panic() {
        let envelope = proto::Envelope {
            r#type: "not_a_real_type".to_string(),
            payload: vec![1, 2, 3],
            compressed: false,
        }
        .encode_to_vec();
        assert!(ServerMessage::decode("not_a_real_type", &[1, 2, 3]).is_none());
        assert!(codec().decode_server_message(&envelope).is_none());

        // Empty payload for a known type is null in Kotlin.
        assert!(ServerMessage::decode(msg::PING, &[]).is_none());

        // A known type with a malformed payload is tolerated.
        assert!(ServerMessage::decode(msg::PONG, &[0xFF, 0xFF, 0xFF, 0xFF]).is_none());

        // Truncated envelope.
        assert!(codec().decode(&[0x0A, 0x04, b'p']).is_err());
        assert!(codec().decode_server_message(&[]).is_none());

        // Random bytes never panic.
        for len in 0..64 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 97 + 13) as u8).collect();
            let _ = codec().decode(&bytes);
        }
    }

    #[test]
    fn outbound_messages_use_the_kotlin_type_strings() {
        let codec = MessageCodec::new(false);
        let cases = [
            (
                Outbound::CreateRoom {
                    username: "nyx".into(),
                },
                msg::CREATE_ROOM,
            ),
            (
                Outbound::JoinRoom {
                    room_code: "ABC123".into(),
                    username: "nyx".into(),
                },
                msg::JOIN_ROOM,
            ),
            (Outbound::LeaveRoom, msg::LEAVE_ROOM),
            (
                Outbound::ApproveJoin {
                    user_id: "u1".into(),
                },
                msg::APPROVE_JOIN,
            ),
            (
                Outbound::RejectJoin {
                    user_id: "u1".into(),
                    reason: None,
                },
                msg::REJECT_JOIN,
            ),
            (
                Outbound::PlaybackAction(proto::PlaybackActionPayload::default()),
                msg::PLAYBACK_ACTION,
            ),
            (
                Outbound::BufferReady {
                    track_id: "t".into(),
                },
                msg::BUFFER_READY,
            ),
            (
                Outbound::KickUser {
                    user_id: "u1".into(),
                    reason: Some("bye".into()),
                },
                msg::KICK_USER,
            ),
            (
                Outbound::TransferHost {
                    new_host_id: "u1".into(),
                },
                msg::TRANSFER_HOST,
            ),
            (
                Outbound::Ping {
                    client_time: 1,
                    sequence: 1,
                },
                msg::PING,
            ),
            (Outbound::RequestSync, msg::REQUEST_SYNC),
            (
                Outbound::Reconnect {
                    session_token: "t".into(),
                },
                msg::RECONNECT,
            ),
            (
                Outbound::SuggestTrack {
                    track_info: proto::TrackInfo::default(),
                },
                msg::SUGGEST_TRACK,
            ),
            (
                Outbound::ApproveSuggestion {
                    suggestion_id: "s".into(),
                },
                msg::APPROVE_SUGGESTION,
            ),
            (
                Outbound::RejectSuggestion {
                    suggestion_id: "s".into(),
                    reason: None,
                },
                msg::REJECT_SUGGESTION,
            ),
            (Outbound::Pong, msg::PONG),
        ];
        for (outbound, expected) in cases {
            let envelope = codec.decode(&codec.encode_outbound(&outbound)).unwrap();
            assert_eq!(envelope.kind, expected);
        }
    }
}
