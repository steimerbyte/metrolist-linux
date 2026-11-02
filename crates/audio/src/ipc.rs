//! mpv JSON IPC wire format: newline-delimited JSON requests and messages.

use ml_core::{MlError, MlResult};
use serde_json::{json, Value};

/// A message read from the mpv IPC socket.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MpvMessage {
    Response {
        request_id: u64,
        error: String,
        data: Option<Value>,
    },
    Event {
        event: String,
        name: Option<String>,
        id: Option<u64>,
        data: Option<Value>,
        reason: Option<String>,
    },
}

/// Serialize one request line (including the trailing newline).
pub(crate) fn request(request_id: u64, command: Vec<Value>) -> String {
    let mut line = json!({ "command": command, "request_id": request_id }).to_string();
    line.push('\n');
    line
}

/// Parse one line received from mpv.
pub(crate) fn parse_line(line: &str) -> MlResult<MpvMessage> {
    let value: Value = serde_json::from_str(line)
        .map_err(|error| MlError::Parse(format!("mpv IPC JSON: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| MlError::Parse("mpv IPC message is not an object".to_string()))?;

    if let Some(event) = object.get("event").and_then(Value::as_str) {
        return Ok(MpvMessage::Event {
            event: event.to_string(),
            name: object
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            id: object.get("id").and_then(Value::as_u64),
            data: object.get("data").cloned().filter(|v| !v.is_null()),
            reason: object
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }

    if let Some(request_id) = object.get("request_id").and_then(Value::as_u64) {
        return Ok(MpvMessage::Response {
            request_id,
            error: object
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string(),
            data: object.get("data").cloned().filter(|v| !v.is_null()),
        });
    }

    Err(MlError::Parse(format!(
        "unrecognized mpv IPC message: {line}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_command_and_id() {
        let line = request(
            7,
            vec![
                json!("loadfile"),
                json!("https://example/x"),
                json!("replace"),
            ],
        );
        assert!(line.ends_with('\n'));
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["request_id"], 7);
        assert_eq!(value["command"][0], "loadfile");
        assert_eq!(value["command"][2], "replace");
    }

    #[test]
    fn parses_success_response() {
        let message = parse_line(r#"{"request_id":3,"error":"success","data":null}"#).unwrap();
        assert_eq!(
            message,
            MpvMessage::Response {
                request_id: 3,
                error: "success".to_string(),
                data: None,
            }
        );
    }

    #[test]
    fn parses_error_response_with_data() {
        let message =
            parse_line(r#"{"request_id":4,"error":"property unavailable","data":true}"#).unwrap();
        match message {
            MpvMessage::Response {
                request_id,
                error,
                data,
            } => {
                assert_eq!(request_id, 4);
                assert_eq!(error, "property unavailable");
                assert_eq!(data, Some(Value::Bool(true)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_property_change_event() {
        let message =
            parse_line(r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#)
                .unwrap();
        assert_eq!(
            message,
            MpvMessage::Event {
                event: "property-change".to_string(),
                name: Some("time-pos".to_string()),
                id: Some(1),
                data: Some(json!(12.5)),
                reason: None,
            }
        );
    }

    #[test]
    fn parses_end_file_reason() {
        let message = parse_line(r#"{"event":"end-file","reason":"eof"}"#).unwrap();
        match message {
            MpvMessage::Event { event, reason, .. } => {
                assert_eq!(event, "end-file");
                assert_eq!(reason.as_deref(), Some("eof"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn missing_data_is_none() {
        let message =
            parse_line(r#"{"event":"property-change","id":7,"name":"eof-reached"}"#).unwrap();
        match message {
            MpvMessage::Event { data, .. } => assert_eq!(data, None),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(parse_line("not json"), Err(MlError::Parse(_))));
        assert!(matches!(parse_line("[1,2,3]"), Err(MlError::Parse(_))));
        assert!(matches!(parse_line(r#"{"foo":1}"#), Err(MlError::Parse(_))));
    }
}
