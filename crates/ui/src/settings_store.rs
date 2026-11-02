//! XDG paths and settings persistence.
//!
//! Linux paths (`directories` 6, application name `metrolist-desktop`):
//! - config: `$XDG_CONFIG_HOME/metrolist-desktop/settings.json`
//! - data:   `$XDG_DATA_HOME/metrolist-desktop/library.db`
//! - cache:  `$XDG_CACHE_HOME/metrolist-desktop/`
//!
//! Settings are written atomically (temp file + rename) so a crash cannot
//! truncate them.

use std::path::PathBuf;

use directories::ProjectDirs;
use ml_core::Settings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl SettingsPaths {
    pub fn from_env() -> Self {
        if let Some(dirs) = ProjectDirs::from("", "", "metrolist-desktop") {
            Self {
                config_dir: dirs.config_dir().to_path_buf(),
                data_dir: dirs.data_dir().to_path_buf(),
                cache_dir: dirs.cache_dir().to_path_buf(),
            }
        } else {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            Self {
                config_dir: home.join(".config/metrolist-desktop"),
                data_dir: home.join(".local/share/metrolist-desktop"),
                cache_dir: home.join(".cache/metrolist-desktop"),
            }
        }
    }

    pub fn new(
        config_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
            cache_dir: cache_dir.into(),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    pub fn database_file(&self) -> PathBuf {
        self.data_dir.join("library.db")
    }

    pub fn art_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("art")
    }

    /// Load settings, falling back to defaults on a missing/corrupt file.
    pub fn load(&self) -> Settings {
        let path = self.settings_file();
        match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_else(|error| {
                tracing::warn!("invalid settings at {}: {error}", path.display());
                Settings::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(error) => {
                tracing::warn!("cannot read {}: {error}", path.display());
                Settings::default()
            }
        }
    }

    /// Persist settings atomically.
    pub fn save(&self, settings: &Settings) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        let path = self.settings_file();
        let temporary = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(settings)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        std::fs::write(&temporary, json)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }

    /// Write defaults on first run; returns the path that was created.
    pub fn ensure_settings_file(&self) -> Option<PathBuf> {
        let path = self.settings_file();
        if path.exists() {
            return None;
        }
        match self.save(&Settings::default()) {
            Ok(()) => Some(path),
            Err(error) => {
                tracing::warn!("cannot write default settings: {error}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> SettingsPaths {
        let root = std::env::temp_dir().join(format!(
            "metrolist-settings-{tag}-{}-{}",
            std::process::id(),
            ml_core::now_millis()
        ));
        SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"))
    }

    #[test]
    fn missing_file_yields_defaults() {
        let paths = temp_paths("missing");
        assert_eq!(paths.load(), Settings::default());
    }

    #[test]
    fn save_then_load_round_trip() {
        let paths = temp_paths("roundtrip");
        let mut settings = Settings::default();
        settings.theme_mode = ml_core::ThemeMode::PureBlack;
        settings.seed_color = 0x123456;
        paths.save(&settings).unwrap();
        assert_eq!(paths.load(), settings);
        assert!(paths.settings_file().exists());
        std::fs::remove_dir_all(paths.config_dir.parent().unwrap()).ok();
    }

    #[test]
    fn first_run_writes_once() {
        let paths = temp_paths("first");
        assert!(paths.ensure_settings_file().is_some());
        assert!(paths.ensure_settings_file().is_none());
        std::fs::remove_dir_all(paths.config_dir.parent().unwrap()).ok();
    }
}
