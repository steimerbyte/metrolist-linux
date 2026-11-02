//! Error taxonomy for InnerTube + googlevideo responses.

use ml_core::MlError;

/// Map a non-success HTTP response from `youtubei` to the shared error type.
pub(crate) fn api_error(status: u16, body: &str) -> MlError {
    let message = body.chars().take(300).collect::<String>();
    match status {
        // A stale clientVersion answers 404; the caller refreshes and retries once.
        404 => MlError::Http {
            status,
            message: "InnerTube endpoint returned 404 (stale client version?)".to_string(),
        },
        401 | 403 => MlError::AuthRequired,
        _ => MlError::Http { status, message },
    }
}

/// Map `playabilityStatus` to an error. `OK` is handled by the caller.
pub(crate) fn playability_error(status: &str, reason: Option<&str>) -> MlError {
    let reason = reason.unwrap_or(status);
    match status {
        "LOGIN_REQUIRED" => MlError::AuthRequired,
        _ => MlError::Unavailable(reason.to_string()),
    }
}

pub(crate) fn http_error(status: u16, message: impl Into<String>) -> MlError {
    MlError::Http {
        status,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_statuses_map_to_typed_errors() {
        assert!(matches!(
            api_error(404, ""),
            MlError::Http { status: 404, .. }
        ));
        assert_eq!(api_error(401, ""), MlError::AuthRequired);
        assert_eq!(api_error(403, ""), MlError::AuthRequired);
        assert!(matches!(
            api_error(500, "boom"),
            MlError::Http { status: 500, message } if message == "boom"
        ));
    }

    #[test]
    fn playability_login_is_auth_and_anything_else_is_unavailable() {
        assert_eq!(
            playability_error("LOGIN_REQUIRED", None),
            MlError::AuthRequired
        );
        assert_eq!(
            playability_error("AGE_CHECK_REQUIRED", Some("Sign in to confirm age")),
            MlError::Unavailable("Sign in to confirm age".to_string())
        );
        assert_eq!(
            playability_error("UNPLAYABLE", None),
            MlError::Unavailable("UNPLAYABLE".to_string())
        );
    }
}
