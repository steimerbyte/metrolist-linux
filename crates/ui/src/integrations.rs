//! Last.fm credentials and the small UI state shared by the integration panel.
//!
//! Secrets live in `${data_dir}/secrets.json` (0600, parent 0700), never in the
//! settings/kv store. The password used for `auth.getMobileSession` is held only
//! for the request and is never serialized.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use ml_integrations::{LastFmConfig, DISCORD_CLIENT_ID};

/// File under the data directory holding Last.fm credentials.
pub const SECRETS_FILE: &str = "secrets.json";
/// Offline scrobble queue path.
pub const QUEUE_FILE: &str = "lastfm-queue.jsonl";

/// Persisted Last.fm credentials. No password field.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LastFmSecrets {
    pub api_key: String,
    pub secret: String,
    pub session: String,
    pub username: String,
}

impl std::fmt::Debug for LastFmSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LastFmSecrets")
            .field("api_key", &masked(&self.api_key))
            .field("secret", &masked(&self.secret))
            .field("session", &masked(&self.session))
            .field("username", &self.username)
            .finish()
    }
}

impl LastFmSecrets {
    pub fn is_empty(&self) -> bool {
        self.api_key.is_empty()
            && self.secret.is_empty()
            && self.session.is_empty()
            && self.username.is_empty()
    }

    pub fn config(&self) -> LastFmConfig {
        LastFmConfig::new(
            self.api_key.trim(),
            Some(self.secret.trim().to_string()).filter(|value| !value.is_empty()),
            Some(self.session.trim().to_string()).filter(|value| !value.is_empty()),
        )
    }
}

fn masked(value: &str) -> &'static str {
    if value.is_empty() {
        ""
    } else {
        "[redacted]"
    }
}

/// Values edited in Settings. `password` is session-only.
#[derive(Clone, Default)]
pub struct LastFmUiState {
    pub enabled: bool,
    pub api_key: String,
    pub secret: String,
    pub session: String,
    pub username: String,
    pub password: String,
    pub connecting: bool,
    pub error: Option<String>,
}

impl LastFmUiState {
    pub fn from_secrets(secrets: LastFmSecrets, enabled: bool) -> Self {
        Self {
            enabled,
            api_key: secrets.api_key,
            secret: secrets.secret,
            session: secrets.session,
            username: secrets.username,
            ..Self::default()
        }
    }

    pub fn secrets(&self) -> LastFmSecrets {
        LastFmSecrets {
            api_key: self.api_key.trim().to_string(),
            secret: self.secret.trim().to_string(),
            session: self.session.trim().to_string(),
            username: self.username.trim().to_string(),
        }
    }

    pub fn config(&self) -> LastFmConfig {
        self.secrets().config()
    }

    pub fn signed_in(&self) -> bool {
        !self.session.trim().is_empty()
    }

    pub fn clear_credentials(&mut self) {
        self.api_key.clear();
        self.secret.clear();
        self.session.clear();
        self.username.clear();
        self.password.clear();
        self.error = None;
        self.connecting = false;
        self.enabled = false;
    }
}

/// Snapshot of the worker's counters, rendered in Settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastFmStatus {
    pub signed_in: bool,
    pub user: Option<String>,
    pub scrobbles_sent: u64,
    pub queued: u64,
    pub session_invalid: bool,
    pub last_error: Option<String>,
    /// A successful request is in flight; used only to label the status.
    pub flushing: bool,
}

/// Discord connection state from the local IPC worker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscordStatus {
    pub enabled: bool,
    pub connected: bool,
    pub waiting: bool,
    pub last_error: Option<String>,
}

impl DiscordStatus {
    pub fn text(&self) -> &'static str {
        if !self.enabled {
            "Off"
        } else if self.connected {
            "Connected"
        } else if self.last_error.is_some() {
            "Discord not running — retrying"
        } else {
            "Waiting for Discord"
        }
    }
}

pub fn secrets_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SECRETS_FILE)
}

pub fn queue_path(data_dir: &Path) -> PathBuf {
    data_dir.join(QUEUE_FILE)
}

/// Effective app id: the saved override, else the packaged default.
pub fn discord_app_id(configured: &str) -> String {
    let configured = configured.trim();
    if configured.is_empty() {
        DISCORD_CLIENT_ID.to_string()
    } else {
        configured.to_string()
    }
}

fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Read the Last.fm credential file. Missing is `Ok(empty)`; unreadable or
/// malformed is an error so Settings can explain instead of silently forgetting.
pub fn load_secrets(path: &Path) -> Result<LastFmSecrets, String> {
    if !path.is_file() {
        return Ok(LastFmSecrets::default());
    }
    let body = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Ok(LastFmSecrets {
        api_key: field("api_key"),
        secret: field("secret"),
        session: field("session"),
        username: field("username"),
    })
}

/// Atomic, private write: 0600 file, 0700 parent.
pub fn save_secrets(path: &Path, secrets: &LastFmSecrets) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "secrets path has no parent".to_string())?;
    ensure_private_dir(parent).map_err(|error| format!("secrets directory: {error}"))?;
    let body = serde_json::json!({
        "api_key": secrets.api_key,
        "secret": secrets.secret,
        "session": secrets.session,
        "username": secrets.username,
    })
    .to_string();
    let temp = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|error| format!("cannot write {}: {error}", temp.display()))?;
    file.write_all(body.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", temp.display()))?;
    file.sync_all()
        .map_err(|error| format!("cannot sync {}: {error}", temp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("secrets permissions: {error}"))?;
    }
    fs::rename(&temp, path).map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

pub fn delete_secrets(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot remove {}: {error}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_app_id_falls_back_to_the_packaged_default() {
        assert_eq!(discord_app_id(""), DISCORD_CLIENT_ID);
        assert_eq!(discord_app_id("   "), DISCORD_CLIENT_ID);
        assert_eq!(discord_app_id(" 123456789012345678 "), "123456789012345678");
    }

    #[test]
    fn discord_status_text_covers_off_waiting_and_error() {
        assert_eq!(DiscordStatus::default().text(), "Off");
        assert_eq!(
            DiscordStatus {
                enabled: true,
                waiting: true,
                ..DiscordStatus::default()
            }
            .text(),
            "Waiting for Discord"
        );
        assert_eq!(
            DiscordStatus {
                enabled: true,
                waiting: true,
                last_error: Some("discord ipc: no socket found".to_string()),
                ..DiscordStatus::default()
            }
            .text(),
            "Discord not running — retrying"
        );
        assert_eq!(
            DiscordStatus {
                enabled: true,
                connected: true,
                ..DiscordStatus::default()
            }
            .text(),
            "Connected"
        );
    }
}
