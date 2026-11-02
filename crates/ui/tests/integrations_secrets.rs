//! Last.fm credentials must never land in PlayerExtras/kv, and the file must
//! stay private.

use ml_ui::integrations::{
    delete_secrets, load_secrets, save_secrets, secrets_path, LastFmSecrets,
};
use ml_ui::state::PlayerExtras;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "metrolist-secrets-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn secrets_round_trip_with_private_permissions_and_no_password() {
    let dir = temp_dir("roundtrip");
    let path = secrets_path(&dir);
    let secrets = LastFmSecrets {
        api_key: "api-key-123".to_string(),
        secret: "shared-secret-456".to_string(),
        session: "session-key-789".to_string(),
        username: "listener".to_string(),
    };
    save_secrets(&path, &secrets).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secrets file must be owner-only");
        let parent = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(parent, 0o700, "secret parent must be owner-only");
    }
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(!body.contains("password"));
    assert_eq!(load_secrets(&path).unwrap(), secrets);

    let persisted = PlayerExtras {
        lastfm_enabled: true,
        discord_enabled: true,
        discord_app_id: "123456789012345678".to_string(),
        ..PlayerExtras::default()
    }
    .persisted();
    for (_, value) in persisted {
        for secret in [
            secrets.api_key.as_str(),
            secrets.secret.as_str(),
            secrets.session.as_str(),
        ] {
            assert!(
                !value.contains(secret),
                "secret leaked into persistent extras"
            );
        }
    }

    delete_secrets(&path).unwrap();
    assert!(!path.exists());
    assert!(load_secrets(&path).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn malformed_secrets_are_reported_not_silently_erased() {
    let dir = temp_dir("malformed");
    let path = secrets_path(&dir);
    std::fs::write(&path, "{not json").unwrap();
    let error = load_secrets(&path).unwrap_err();
    assert!(error.contains("invalid"), "{error}");
    assert!(path.exists(), "malformed file must not be deleted");
    let _ = std::fs::remove_dir_all(dir);
}
