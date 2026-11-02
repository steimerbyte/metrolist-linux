//! User settings (serde JSON). Path resolution lives in `ml-ui`, not here.

use serde::{Deserialize, Serialize};

/// Material 3 theme mode. `PureBlack` forces all surface tiers to `#000000`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
    PureBlack,
}

/// Audio format preference; mapped to InnerTube itags by `ml-innertube`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioQuality {
    #[default]
    Auto,
    Low,
    High,
}

/// Navigation shell preference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavStyle {
    #[default]
    Auto,
    Bottom,
    Rail,
}

/// Library list ordering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    #[default]
    RecentlyAdded,
    Alphabetical,
    MostPlayed,
    Artist,
}

/// Proxy settings. Passwords will move to the OS keyring in a later milestone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    pub enabled: bool,
    pub url: String,
    pub username: String,
    pub password: String,
}

/// Lyrics provider switches (upstream order: enabled ones are tried in turn).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricsSettings {
    pub lrclib: bool,
    pub lyrics_plus: bool,
    pub kugou: bool,
    pub better_lyrics: bool,
    pub paxsenix: bool,
    pub zemer: bool,
    pub romanize: bool,
}

impl Default for LyricsSettings {
    fn default() -> Self {
        Self {
            lrclib: true,
            lyrics_plus: true,
            kugou: false,
            better_lyrics: false,
            paxsenix: false,
            zemer: false,
            romanize: true,
        }
    }
}

/// Download directory and concurrency.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadSettings {
    /// Absolute path; `None` means `$XDG_MUSIC_DIR/Metrolist` (resolved by `ml-ui`).
    pub directory: Option<String>,
    pub concurrent_limit: u8,
    pub normalize: bool,
}

/// Persistent application settings. Missing JSON fields fall back to defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme_mode: ThemeMode,
    /// Default seed from `ui/theme/Theme.kt`: `#ED5564`.
    pub seed_color: u32,
    /// Read the desktop accent colour when available; falls back to `seed_color`.
    pub dynamic_accent: bool,
    pub audio_quality: AudioQuality,
    pub content_language: String,
    pub content_country: String,
    pub proxy: ProxyConfig,
    pub lyrics: LyricsSettings,
    pub downloads: DownloadSettings,
    pub audio_normalization: bool,
    pub crossfade_enabled: bool,
    pub crossfade_seconds: f32,
    pub sleep_timer_minutes: Option<u32>,
    pub sleep_timer_fade: bool,
    pub library_sort: LibrarySort,
    pub nav_style: NavStyle,
    pub hide_explicit: bool,
    pub hide_video_songs: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::System,
            seed_color: 0x00ED_5564,
            dynamic_accent: true,
            audio_quality: AudioQuality::Auto,
            content_language: "en".to_string(),
            content_country: "US".to_string(),
            proxy: ProxyConfig::default(),
            lyrics: LyricsSettings::default(),
            downloads: DownloadSettings {
                directory: None,
                concurrent_limit: 3,
                normalize: false,
            },
            audio_normalization: false,
            crossfade_enabled: false,
            crossfade_seconds: 5.0,
            sleep_timer_minutes: None,
            sleep_timer_fade: true,
            library_sort: LibrarySort::RecentlyAdded,
            nav_style: NavStyle::Auto,
            hide_explicit: false,
            hide_video_songs: false,
        }
    }
}

impl Settings {
    /// Seed as `(r, g, b)` for `material-colors`.
    pub fn seed_rgb(&self) -> (u8, u8, u8) {
        (
            ((self.seed_color >> 16) & 0xFF) as u8,
            ((self.seed_color >> 8) & 0xFF) as u8,
            (self.seed_color & 0xFF) as u8,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_upstream_seed() {
        let settings = Settings::default();
        assert_eq!(settings.seed_color, 0x00ED_5564);
        assert_eq!(settings.seed_rgb(), (0xED, 0x55, 0x64));
        assert_eq!(settings.theme_mode, ThemeMode::System);
        assert_eq!(settings.downloads.concurrent_limit, 3);
        assert!(settings.lyrics.lrclib);
    }

    #[test]
    fn missing_fields_fill_from_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"theme_mode":"pure_black"}"#).unwrap();
        assert_eq!(settings.theme_mode, ThemeMode::PureBlack);
        assert_eq!(settings.seed_color, 0x00ED_5564);
        assert_eq!(settings.audio_quality, AudioQuality::Auto);
    }

    #[test]
    fn round_trip() {
        let mut settings = Settings::default();
        settings.proxy.enabled = true;
        settings.proxy.url = "socks5://127.0.0.1:1080".into();
        settings.crossfade_seconds = 3.5;
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, settings);
    }
}
