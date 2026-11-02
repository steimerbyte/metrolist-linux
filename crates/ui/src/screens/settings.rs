//! Settings: appearance, playback, content, network, lyrics, storage, about.
//! All edits mutate `Settings`; the app persists and applies them live.

use std::path::Path;
use std::time::Duration;

use eframe::egui::{self, Align, Color32, Layout, Sense, Ui, Vec2};
use ml_audio::SleepTimer;
use ml_core::{AudioQuality, LyricsSettings, Settings, ThemeMode};

use crate::desktop::DesktopStatus;
use crate::integrations::{DiscordStatus, LastFmStatus, LastFmUiState};
use crate::pure::{
    bytes_text, crossfade_label, crossfade_seconds, gain_label, normalization_gain, playback_speed,
    sanitize_proxy_url, seed_color_value, speed_percent,
};
use crate::state::{EqPreset, PlayerExtras};
use crate::theme::{
    bold_font, medium_font, regular_font, Theme, BODY_SMALL, DP16, DP24, DP4, DP8, TITLE_MEDIUM,
};
use crate::widgets::{chip, outlined_button, slider};

use super::measured_column;

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsAction {
    Changed,
    ClearImageCache,
    SleepTimer(SleepTimer),
    EqChanged,
    CrossfadeChanged,
    SkipSilenceChanged,
    NormalizationChanged,
    SpeedChanged(f32),
    DeviceChanged(Option<String>),
    NotificationsChanged(bool),
    ListDevices,
    DownloadLimitChanged(Option<u64>),
    ClearFailedDownloads,
    OpenDownloads,
    OpenAccount,
    /// Translation prefs changed; drop cached translations and persist.
    LyricsTranslationChanged,
    /// Last.fm fields/toggle changed; save credentials and refresh the worker.
    LastFmChanged,
    LastFmConnect,
    LastFmSignOut,
    LastFmForget,
    DiscordChanged,
}

/// UI-only state that lives outside `ml_core::Settings`.
pub struct SettingsStatus<'a> {
    pub extras: &'a mut PlayerExtras,
    pub crossfade_available: bool,
    pub focus_playback: bool,
    pub focus_sleep: bool,
    /// Demo/screenshot helper: bring the output-device list into view.
    pub focus_audio: bool,
    pub image_cache: usize,
    pub image_cache_bytes: usize,
    pub db_path: &'a Path,
    pub desktop: DesktopStatus,
    pub download_dir: &'a Path,
    pub download_used: u64,
    /// Next-launch limit; the active manager may still use a different one.
    pub download_limit: Option<u64>,
    pub download_active_limit: Option<u64>,
    pub download_failed: usize,
    pub download_quality: Option<AudioQuality>,
    pub downloads_available: bool,
    pub download_error: Option<&'a str>,
    pub account_signed_in: bool,
    pub account_name: Option<&'a str>,
    /// Session-only translation API key. Never persisted.
    pub translation_key: &'a mut String,
    pub lastfm: &'a mut LastFmUiState,
    pub lastfm_status: LastFmStatus,
    pub discord_status: DiscordStatus,
    /// Demo/screenshot helper: bring the integration panel into view.
    pub focus_integrations: bool,
}

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    settings: &mut Settings,
    status: SettingsStatus<'_>,
    width: f32,
) -> Vec<SettingsAction> {
    let SettingsStatus {
        extras,
        crossfade_available,
        focus_playback,
        focus_sleep,
        focus_audio,
        image_cache,
        image_cache_bytes,
        db_path,
        desktop,
        download_dir,
        download_used,
        download_limit,
        download_active_limit,
        download_failed,
        download_quality,
        downloads_available,
        download_error,
        account_signed_in,
        account_name,
        translation_key,
        lastfm,
        lastfm_status,
        discord_status,
        focus_integrations,
    } = status;
    let mut actions = Vec::new();
    let mut changed = false;

    section(ui, theme, "Account");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(
            egui::RichText::new(if account_signed_in {
                account_name.unwrap_or("Signed in")
            } else {
                "Signed out"
            })
            .font(medium_font(TITLE_MEDIUM))
            .color(theme.scheme.on_surface),
        );
        if outlined_button(ui, theme, "Manage account").clicked() {
            actions.push(SettingsAction::OpenAccount);
        }
    });
    ui.label(caption(
        theme,
        if account_signed_in {
            "Cookies are loaded. Sync the online library or sign out."
        } else {
            "Import YouTube Music cookies to sync your library."
        },
    ));

    section(ui, theme, "Appearance");
    ui.label(caption(theme, "Theme"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        for (mode, label) in [
            (ThemeMode::System, "System"),
            (ThemeMode::Light, "Light"),
            (ThemeMode::Dark, "Dark"),
            (ThemeMode::PureBlack, "Pure black"),
        ] {
            if chip(ui, theme, label, settings.theme_mode == mode).clicked() {
                settings.theme_mode = mode;
                changed = true;
            }
        }
    });
    ui.add_space(DP8);
    ui.label(caption(theme, "Seed colour"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        for (seed, name) in [
            (0x00ED_5564u32, "Coral"),
            (0x0042_85F4, "Blue"),
            (0x0034_A853, "Green"),
            (0x00F9_AB00, "Amber"),
            (0x009C_27B0, "Purple"),
            (0x0000_BCD4, "Cyan"),
        ] {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
            let color = seed_color(seed);
            ui.painter().circle_filled(rect.center(), 15.0, color);
            if settings.seed_color == seed {
                ui.painter().circle_stroke(
                    rect.center(),
                    17.0,
                    egui::Stroke::new(2.0, theme.scheme.primary),
                );
            }
            if response.clicked() && settings.seed_color != seed {
                settings.seed_color = seed;
                changed = true;
            }
            response.on_hover_text(name);
        }
        let (r, g, b) = settings.seed_rgb();
        let mut rgb = [r, g, b];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            let seed = ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | rgb[2] as u32;
            let seed = seed_color_value(seed);
            if seed != settings.seed_color {
                settings.seed_color = seed;
                changed = true;
            }
        }
    });
    ui.add_space(DP4);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if chip(ui, theme, "Dynamic colour", settings.dynamic_accent).clicked() {
            settings.dynamic_accent = !settings.dynamic_accent;
            changed = true;
        }
        ui.label(caption(
            theme,
            if settings.dynamic_accent {
                "Follows the desktop accent when available."
            } else {
                "Uses the seed colour."
            },
        ));
    });
    ui.label(caption(
        theme,
        "Dynamic desktop accent detection is not available yet; seed colour is used.",
    ));

    let playback_anchor = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(1.0, 1.0));
    section(ui, theme, "Playback");
    ui.label(caption(theme, "Stream quality"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        for (quality, label) in [
            (AudioQuality::Auto, "Auto"),
            (AudioQuality::Low, "Low"),
            (AudioQuality::High, "High"),
        ] {
            if chip(ui, theme, label, settings.audio_quality == quality).clicked() {
                settings.audio_quality = quality;
                changed = true;
            }
        }
    });
    ui.label(caption(
        theme,
        "Auto adapts per track; Low saves data; High prefers the best audio.",
    ));

    ui.add_space(DP8);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Crossfade"));
        ui.label(
            egui::RichText::new(crossfade_label(extras.crossfade_seconds))
                .font(medium_font(13.0))
                .color(theme.scheme.on_surface),
        );
    });
    if crossfade_available {
        let mut seconds = extras.crossfade_seconds as f32;
        let response = slider(
            ui,
            theme,
            &mut seconds,
            0.0..=30.0,
            (width - 16.0).max(120.0),
            "Crossfade seconds",
        );
        if response.changed() {
            extras.crossfade_seconds = crossfade_seconds(seconds);
            actions.push(SettingsAction::CrossfadeChanged);
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.label(caption(theme, "Keep gapless within an album"));
            if chip(ui, theme, "On", extras.crossfade_gapless).clicked() {
                extras.crossfade_gapless = true;
                actions.push(SettingsAction::CrossfadeChanged);
            }
            if chip(ui, theme, "Off", !extras.crossfade_gapless).clicked() {
                extras.crossfade_gapless = false;
                actions.push(SettingsAction::CrossfadeChanged);
            }
        });
        ui.label(caption(
            theme,
            "0 s turns crossfade off; same-album tracks stay gapless when enabled.",
        ));
    } else {
        ui.label(
            egui::RichText::new("Second audio engine unavailable; crossfade stays off.")
                .font(regular_font(12.0))
                .color(theme.scheme.error),
        );
    }

    ui.add_space(DP8);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Skip silence"));
        if chip(ui, theme, "On", extras.skip_silence).clicked() {
            extras.skip_silence = true;
            actions.push(SettingsAction::SkipSilenceChanged);
        }
        if chip(ui, theme, "Off", !extras.skip_silence).clicked() {
            extras.skip_silence = false;
            actions.push(SettingsAction::SkipSilenceChanged);
        }
    });
    ui.label(caption(
        theme,
        "Drops gaps longer than 0.5 s below -50 dBFS using mpv silenceremove.",
    ));

    ui.add_space(DP8);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Normalize volume"));
        if chip(ui, theme, "On", extras.normalization).clicked() {
            extras.normalization = true;
            actions.push(SettingsAction::NormalizationChanged);
        }
        if chip(ui, theme, "Off", !extras.normalization).clicked() {
            extras.normalization = false;
            actions.push(SettingsAction::NormalizationChanged);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Manual gain"));
        ui.label(
            egui::RichText::new(gain_label(extras.normalization_gain_db))
                .font(medium_font(13.0))
                .color(theme.scheme.on_surface),
        );
    });
    let mut gain = extras.normalization_gain_db;
    let response = slider(
        ui,
        theme,
        &mut gain,
        -15.0..=3.0,
        (width - 16.0).max(120.0),
        "Normalization gain dB",
    );
    if response.changed() {
        extras.normalization_gain_db = normalization_gain(gain);
        actions.push(SettingsAction::NormalizationChanged);
    }
    ui.label(caption(
        theme,
        "No per-track loudness metadata yet; applies the manual gain when normalization is on.",
    ));

    ui.add_space(DP8);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Playback speed"));
        ui.label(
            egui::RichText::new(format!("{}%", speed_percent(extras.speed)))
                .font(medium_font(13.0))
                .color(theme.scheme.on_surface),
        );
    });
    let mut speed = extras.speed;
    let response = slider(
        ui,
        theme,
        &mut speed,
        0.5..=2.0,
        (width - 16.0).max(120.0),
        "Playback speed",
    );
    if response.changed() {
        extras.speed = playback_speed(speed);
        actions.push(SettingsAction::SpeedChanged(extras.speed));
    }
    ui.label(caption(
        theme,
        "Pitch is preserved by mpv; 100% is normal speed.",
    ));

    ui.add_space(DP8);
    let sleep_anchor = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(1.0, 1.0));
    ui.label(caption(theme, "Sleep timer"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        for (timer, label) in [
            (SleepTimer::Off, "Off"),
            (SleepTimer::Minutes(15), "15m"),
            (SleepTimer::Minutes(30), "30m"),
            (SleepTimer::Minutes(60), "60m"),
            (SleepTimer::EndOfTrack, "End of track"),
            (
                SleepTimer::FadeOut(Duration::from_secs(30)),
                "Fade out 30 s",
            ),
        ] {
            let selected = sleep_selected(extras.sleep, timer);
            if chip(ui, theme, label, selected).clicked() {
                extras.sleep = timer;
                actions.push(SettingsAction::SleepTimer(timer));
            }
        }
    });
    ui.label(caption(
        theme,
        "Off/minutes/end of track persist; fade-out starts now and clears on restart.",
    ));

    ui.add_space(DP8);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "10-band equaliser"));
        if chip(ui, theme, "On", extras.eq_enabled).clicked() {
            extras.eq_enabled = true;
            actions.push(SettingsAction::EqChanged);
        }
        if chip(ui, theme, "Off", !extras.eq_enabled).clicked() {
            extras.eq_enabled = false;
            actions.push(SettingsAction::EqChanged);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        for preset in EqPreset::ALL {
            if chip(ui, theme, preset.label(), extras.eq_preset == preset).clicked() {
                extras.eq_preset = preset;
                actions.push(SettingsAction::EqChanged);
            }
        }
    });

    ui.add_space(DP8);
    let audio_anchor = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(1.0, 1.0));
    ui.label(caption(theme, "Output device"));
    if extras.devices.initial_loading() {
        ui.label(caption(theme, "Enumerating mpv output devices…"));
    } else if let Some(error) = extras.devices.error.as_ref() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.label(
                egui::RichText::new(format!("Device list unavailable: {error}"))
                    .font(regular_font(12.0))
                    .color(theme.scheme.error),
            );
            if outlined_button(ui, theme, "Retry").clicked() {
                actions.push(SettingsAction::ListDevices);
            }
        });
    } else if let Some(devices) = extras.devices.data.as_ref() {
        let list_width = ui.available_width();
        let list_height = (((devices.len() + 1) as f32) * 42.0 + DP8).clamp(80.0, 280.0);
        // The outer settings page is itself a scroll area, so a bare nested
        // ScrollArea gets clipped to the remaining page height. Allocate the
        // list explicitly and keep its own viewport fixed.
        ui.allocate_ui_with_layout(
            Vec2::new(list_width, list_height),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(list_width);
                egui::ScrollArea::vertical()
                    .id_salt("audio_devices")
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = DP4;
                        let automatic = extras
                            .device
                            .as_deref()
                            .is_none_or(|id| id.eq_ignore_ascii_case("auto"));
                        let auto = ui.add(
                            egui::Button::new(
                                egui::RichText::new("Automatic (system default)")
                                    .font(regular_font(13.0)),
                            )
                            .selected(automatic)
                            .wrap()
                            .min_size(Vec2::new(ui.available_width(), 0.0)),
                        );
                        if auto.clicked() {
                            actions.push(SettingsAction::DeviceChanged(None));
                        }
                        for device in devices {
                            let selected = extras.device.as_deref() == Some(device.id.as_str());
                            let label = if device.is_default {
                                format!("{} (default)", device.description)
                            } else {
                                device.description.clone()
                            };
                            let row = ui.add(
                                egui::Button::new(
                                    egui::RichText::new(label).font(regular_font(13.0)),
                                )
                                .selected(selected)
                                .wrap()
                                .min_size(Vec2::new(ui.available_width(), 0.0)),
                            );
                            if row.clicked() {
                                actions
                                    .push(SettingsAction::DeviceChanged(Some(device.id.clone())));
                            }
                        }
                    });
            },
        );
        if outlined_button(ui, theme, "Refresh").clicked() {
            actions.push(SettingsAction::ListDevices);
        }
    }

    section(ui, theme, "Content");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Language"));
        let language = ui.add(
            egui::TextEdit::singleline(&mut settings.content_language)
                .desired_width(70.0)
                .char_limit(8)
                .font(regular_font(15.0)),
        );
        ui.label(caption(theme, "Country"));
        let country = ui.add(
            egui::TextEdit::singleline(&mut settings.content_country)
                .desired_width(70.0)
                .char_limit(8)
                .font(regular_font(15.0)),
        );
        if language.changed() || country.changed() {
            changed = true;
        }
    });
    ui.label(caption(
        theme,
        "Used for InnerTube metadata and search; applies on the next launch.",
    ));

    section(ui, theme, "Network");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Proxy"));
        if chip(ui, theme, "Off", !settings.proxy.enabled).clicked() {
            settings.proxy.enabled = false;
            changed = true;
        }
        if chip(ui, theme, "On", settings.proxy.enabled).clicked() {
            settings.proxy.enabled = true;
            changed = true;
        }
    });
    if settings.proxy.enabled {
        ui.horizontal_wrapped(|ui| {
            ui.label(caption(theme, "URL"));
            let url = ui.add(
                egui::TextEdit::singleline(&mut settings.proxy.url)
                    .hint_text("http://127.0.0.1:8080")
                    .desired_width((width - 70.0).max(120.0))
                    .font(regular_font(15.0)),
            );
            if url.changed() {
                changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(caption(theme, "User"));
            let user = ui.add(
                egui::TextEdit::singleline(&mut settings.proxy.username)
                    .desired_width(130.0)
                    .font(regular_font(15.0)),
            );
            ui.label(caption(theme, "Password"));
            let password = ui.add(
                egui::TextEdit::singleline(&mut settings.proxy.password)
                    .password(true)
                    .desired_width(130.0)
                    .font(regular_font(15.0)),
            );
            if user.changed() || password.changed() {
                changed = true;
            }
        });
        if sanitize_proxy_url(&settings.proxy.url).is_none() {
            ui.label(
                egui::RichText::new("Enter an http:// or https:// proxy URL.")
                    .font(regular_font(12.0))
                    .color(theme.scheme.error),
            );
        }
    }
    ui.label(caption(
        theme,
        "Proxy changes apply when the app starts. Passwords are stored in plain JSON for now.",
    ));

    section(ui, theme, "Lyrics");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        let providers = [
            ("LRCLIB", &mut settings.lyrics.lrclib),
            ("LyricsPlus", &mut settings.lyrics.lyrics_plus),
            ("Kugou", &mut settings.lyrics.kugou),
            ("BetterLyrics", &mut settings.lyrics.better_lyrics),
            ("Paxsenix", &mut settings.lyrics.paxsenix),
            ("Zemer", &mut settings.lyrics.zemer),
        ];
        for (label, enabled) in providers {
            if chip(ui, theme, label, *enabled).clicked() {
                *enabled = !*enabled;
                changed = true;
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Romanize lyrics"));
        let value = settings.lyrics.romanize;
        if chip(ui, theme, if value { "On" } else { "Off" }, value).clicked() {
            settings.lyrics = LyricsSettings {
                romanize: !value,
                ..settings.lyrics.clone()
            };
            changed = true;
        }
    });
    ui.add_space(DP8);
    ui.label(caption(theme, "Translation"));
    let mut translation_changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        for provider in [
            crate::state::TranslationProvider::Off,
            crate::state::TranslationProvider::DeepL,
            crate::state::TranslationProvider::OpenAiCompatible,
        ] {
            if chip(
                ui,
                theme,
                provider.label(),
                extras.translation_provider == provider,
            )
            .clicked()
            {
                extras.translation_provider = provider;
                translation_changed = true;
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Show translation"));
        if chip(ui, theme, "On", extras.translation_enabled).clicked() {
            extras.translation_enabled = true;
            translation_changed = true;
        }
        if chip(ui, theme, "Off", !extras.translation_enabled).clicked() {
            extras.translation_enabled = false;
            translation_changed = true;
        }
    });
    if extras.translation_provider.enabled() {
        if extras.translation_provider == crate::state::TranslationProvider::OpenAiCompatible {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = DP8;
                ui.label(caption(theme, "Endpoint"));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut extras.translation_endpoint)
                        .hint_text("https://api.example.com/v1/chat/completions")
                        .desired_width((width - 90.0).max(160.0)),
                );
                if response.lost_focus() {
                    translation_changed = true;
                }
            });
            if !extras.translation_endpoint.is_empty()
                && !crate::pure::translation_endpoint_valid(&extras.translation_endpoint)
            {
                ui.label(
                    egui::RichText::new(crate::pure::TRANSLATION_ENDPOINT_MESSAGE)
                        .font(regular_font(12.0))
                        .color(theme.scheme.error),
                );
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.label(caption(theme, "Target language"));
            let response = ui.add(
                egui::TextEdit::singleline(&mut extras.translation_target)
                    .hint_text("en")
                    .desired_width(90.0),
            );
            if response.lost_focus() {
                if extras.translation_target.trim().is_empty() {
                    extras.translation_target = "en".to_string();
                }
                translation_changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.label(caption(theme, "API key"));
            let response = ui.add(
                egui::TextEdit::singleline(translation_key)
                    .password(true)
                    .hint_text("Session only")
                    .desired_width((width - 180.0).max(140.0)),
            );
            let _ = response;
            if !translation_key.is_empty() && outlined_button(ui, theme, "Clear key").clicked() {
                translation_key.clear();
            }
        });
        ui.label(caption(
            theme,
            "The API key stays in memory for this session and is never saved.",
        ));
        if translation_key.is_empty() {
            ui.label(
                egui::RichText::new(crate::pure::TRANSLATION_MISSING_KEY_MESSAGE)
                    .font(regular_font(12.0))
                    .color(theme.scheme.on_surface_variant),
            );
        }
    } else {
        ui.label(caption(
            theme,
            "Off — translated lines are hidden until a provider is chosen.",
        ));
    }
    if translation_changed {
        actions.push(SettingsAction::LyricsTranslationChanged);
    }
    if !any_provider(&settings.lyrics) {
        ui.label(
            egui::RichText::new("No provider enabled — lyrics will stay empty.")
                .font(regular_font(12.0))
                .color(theme.scheme.error),
        );
    }

    if focus_integrations {
        ui.scroll_to_cursor(Some(egui::Align::Min));
    }
    measured_column(ui, 640.0, |ui| {
        section(ui, theme, "Integrations");
        let mut lastfm_changed = false;
        let mut discord_changed = false;
        ui.label(
            egui::RichText::new("Last.fm")
                .font(medium_font(TITLE_MEDIUM))
                .color(theme.scheme.on_surface),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.label(caption(theme, "Scrobble plays"));
            if chip(ui, theme, "On", lastfm.enabled).clicked() {
                lastfm.enabled = true;
                lastfm_changed = true;
            }
            if chip(ui, theme, "Off", !lastfm.enabled).clicked() {
                lastfm.enabled = false;
                lastfm_changed = true;
            }
        });
        if lastfm.enabled {
            for (label, value, secret) in [
                ("API key", &mut lastfm.api_key, true),
                ("Shared secret", &mut lastfm.secret, true),
                ("Session key", &mut lastfm.session, true),
                ("Username", &mut lastfm.username, false),
            ] {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = DP8;
                    ui.label(caption(theme, label));
                    let response = ui.add(
                        egui::TextEdit::singleline(value)
                            .password(secret)
                            .hint_text(if secret {
                                "stored locally"
                            } else {
                                "Last.fm user"
                            })
                            .desired_width((ui.available_width() - 80.0).clamp(150.0, 360.0)),
                    );
                    if response.lost_focus() {
                        lastfm_changed = true;
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = DP8;
                ui.label(caption(theme, "Password"));
                ui.add(
                    egui::TextEdit::singleline(&mut lastfm.password)
                        .password(true)
                        .hint_text("session only")
                        .desired_width((ui.available_width() - 80.0).clamp(150.0, 360.0)),
                );
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = DP8;
                let connect_label = if lastfm_status.signed_in {
                    "Test"
                } else {
                    "Connect"
                };
                if outlined_button(ui, theme, connect_label).clicked() {
                    actions.push(SettingsAction::LastFmConnect);
                }
                if lastfm.signed_in() && outlined_button(ui, theme, "Sign out").clicked() {
                    actions.push(SettingsAction::LastFmSignOut);
                }
                if outlined_button(ui, theme, "Forget credentials").clicked() {
                    actions.push(SettingsAction::LastFmForget);
                }
            });
            if lastfm.connecting {
                ui.label(caption(theme, "Connecting to Last.fm…"));
            }
            let state_text = if lastfm_status.session_invalid {
                "Session rejected — reconnect or sign out.".to_string()
            } else if lastfm_status.signed_in {
                match lastfm_status
                    .user
                    .as_deref()
                    .filter(|name| !name.is_empty())
                {
                    Some(name) => format!("Connected as {name}"),
                    None => "Connected with a supplied session key".to_string(),
                }
            } else if lastfm.enabled {
                "Not connected".to_string()
            } else {
                "Scrobbling is off".to_string()
            };
            ui.label(caption(
                theme,
                &format!(
                    "{state_text} · {} scrobbles sent · {} queued{}",
                    lastfm_status.scrobbles_sent,
                    lastfm_status.queued,
                    if lastfm_status.flushing {
                        " · syncing"
                    } else {
                        ""
                    }
                ),
            ));
            if let Some(error) = &lastfm.error {
                ui.label(
                    egui::RichText::new(error)
                        .font(regular_font(12.0))
                        .color(theme.scheme.error),
                );
            }
            if let Some(error) = &lastfm_status.last_error {
                ui.label(
                    egui::RichText::new(error)
                        .font(regular_font(12.0))
                        .color(theme.scheme.error),
                );
            }
            ui.label(caption(
            theme,
            "Credentials are stored locally in secrets.json (0600). The password is never saved.",
        ));
        } else {
            ui.label(caption(
                theme,
                "Off — enable to connect and scrobble; credentials stay local.",
            ));
            if let Some(error) = &lastfm.error {
                ui.label(
                    egui::RichText::new(error)
                        .font(regular_font(12.0))
                        .color(theme.scheme.error),
                );
            }
        }

        ui.add_space(DP8);
        ui.label(
            egui::RichText::new("Discord")
                .font(medium_font(TITLE_MEDIUM))
                .color(theme.scheme.on_surface),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.label(caption(theme, "Rich presence"));
            if chip(ui, theme, "On", extras.discord_enabled).clicked() {
                extras.discord_enabled = true;
                discord_changed = true;
            }
            if chip(ui, theme, "Off", !extras.discord_enabled).clicked() {
                extras.discord_enabled = false;
                discord_changed = true;
            }
        });
        if extras.discord_enabled {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = DP8;
                ui.label(caption(theme, "App ID"));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut extras.discord_app_id)
                        .hint_text(ml_integrations::DISCORD_CLIENT_ID)
                        .desired_width((ui.available_width() - 80.0).clamp(150.0, 360.0)),
                );
                if response.lost_focus() {
                    discord_changed = true;
                }
            });
            ui.label(caption(theme, discord_status.text()));
            if let Some(error) = &discord_status.last_error {
                ui.label(
                    egui::RichText::new(error)
                        .font(regular_font(12.0))
                        .color(theme.scheme.error),
                );
            }
            ui.label(caption(
            theme,
            "Uses the local Discord IPC socket. No token or login; blank uses the bundled app ID.",
        ));
        } else {
            ui.label(caption(theme, "Off — no Discord IPC connection is made."));
        }
        if lastfm_changed {
            actions.push(SettingsAction::LastFmChanged);
        }
        if discord_changed {
            actions.push(SettingsAction::DiscordChanged);
        }
    });

    section(ui, theme, "Desktop");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(theme, "Now-playing notifications"));
        if chip(ui, theme, "On", extras.notifications).clicked() {
            extras.notifications = true;
            actions.push(SettingsAction::NotificationsChanged(true));
        }
        if chip(ui, theme, "Off", !extras.notifications).clicked() {
            extras.notifications = false;
            actions.push(SettingsAction::NotificationsChanged(false));
        }
    });
    ui.label(caption(
        theme,
        &format!(
            "MPRIS: {} · Tray: {}",
            if desktop.mpris {
                "active"
            } else {
                "unavailable"
            },
            if desktop.tray {
                "active"
            } else {
                "unavailable"
            }
        ),
    ));
    ui.label(caption(
        theme,
        "Closing or quitting stops playback. Hiding the window from the tray keeps audio playing.",
    ));

    section(ui, theme, "Storage");
    ui.label(caption(theme, "Downloads"));
    if downloads_available {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            if outlined_button(ui, theme, "Manage downloads").clicked() {
                actions.push(SettingsAction::OpenDownloads);
            }
            if download_failed > 0
                && outlined_button(ui, theme, &format!("Clear failed ({download_failed})"))
                    .clicked()
            {
                actions.push(SettingsAction::ClearFailedDownloads);
            }
        });
        ui.label(
            egui::RichText::new(format!("Folder: {}", download_dir.display()))
                .font(regular_font(12.0))
                .color(theme.scheme.on_surface_variant),
        );
        ui.label(caption(
            theme,
            &format!(
                "Used: {}{}",
                bytes_text(download_used),
                download_active_limit
                    .map(|limit| format!(" of {}", bytes_text(limit)))
                    .unwrap_or_else(|| " · no limit".to_string())
            ),
        ));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            ui.label(caption(theme, "Limit"));
            for (limit, label) in [
                (None, "No limit"),
                (Some(1 << 30), "1 GiB"),
                (Some(5 << 30), "5 GiB"),
                (Some(20 << 30), "20 GiB"),
            ] {
                if chip(ui, theme, label, download_limit == limit).clicked() {
                    actions.push(SettingsAction::DownloadLimitChanged(limit));
                }
            }
        });
        if download_limit != download_active_limit {
            ui.label(caption(
                theme,
                &format!(
                    "Next launch limit: {} (active: {}).",
                    download_limit
                        .map(bytes_text)
                        .unwrap_or_else(|| "no limit".to_string()),
                    download_active_limit
                        .map(bytes_text)
                        .unwrap_or_else(|| "no limit".to_string())
                ),
            ));
        }
        ui.label(caption(
            theme,
            &format!(
                "New downloads use {} quality from launch. Folder (settings.json) and limit changes apply after restart.",
                download_quality
                    .map(|quality| format!("{quality:?}"))
                    .unwrap_or_else(|| "unknown".to_string())
            ),
        ));
    } else {
        ui.label(
            egui::RichText::new(download_error.unwrap_or("Download manager did not start."))
                .font(regular_font(12.0))
                .color(theme.scheme.error),
        );
    }
    ui.add_space(DP4);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.label(caption(
            theme,
            &format!(
                "Image cache: {image_cache} thumbnails, ~{} in GPU memory",
                memory_text(image_cache_bytes)
            ),
        ));
        if outlined_button(ui, theme, "Clear images").clicked() {
            actions.push(SettingsAction::ClearImageCache);
        }
    });
    ui.label(
        egui::RichText::new(format!("Library database: {}", db_path.display()))
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
    );
    ui.label(caption(
        theme,
        "Image cache is in-memory only; downloads are deleted from Library > Downloads.",
    ));

    section(ui, theme, "About");
    ui.label(
        egui::RichText::new(format!("Metrolist Desktop {}", env!("CARGO_PKG_VERSION")))
            .font(medium_font(TITLE_MEDIUM))
            .color(theme.scheme.on_surface),
    );
    ui.label(caption(theme, "License: GPL-3.0-only"));
    ui.label(caption(
        theme,
        "A native Linux port of the Metrolist Android client. UI and behaviour follow upstream.",
    ));
    ui.label(caption(
        theme,
        "Built with egui, material-colors, mpv, Symphonia, SQLite and the InnerTube web client.",
    ));
    ui.add_space(DP4);
    ui.label(caption(
        theme,
        "Source and license texts are available in the repository.",
    ));

    if focus_playback {
        ui.scroll_to_rect(playback_anchor, Some(egui::Align::TOP));
    } else if focus_sleep {
        ui.scroll_to_rect(sleep_anchor, Some(egui::Align::TOP));
    } else if focus_audio {
        ui.scroll_to_rect(audio_anchor, Some(egui::Align::TOP));
    }
    if changed {
        actions.push(SettingsAction::Changed);
    }
    ui.add_space(DP24);
    actions
}

fn section(ui: &mut Ui, theme: &Theme, title: &str) {
    ui.add_space(DP16);
    ui.label(
        egui::RichText::new(title)
            .font(bold_font(TITLE_MEDIUM))
            .color(theme.scheme.primary),
    );
}

fn caption<'a>(theme: &Theme, text: &'a str) -> egui::RichText {
    egui::RichText::new(text)
        .font(regular_font(BODY_SMALL))
        .color(theme.scheme.on_surface_variant)
}

fn seed_color(seed: u32) -> Color32 {
    Color32::from_rgb(
        ((seed >> 16) & 0xFF) as u8,
        ((seed >> 8) & 0xFF) as u8,
        (seed & 0xFF) as u8,
    )
}

fn memory_text(bytes: usize) -> String {
    const MIB: usize = 1024 * 1024;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else {
        format!("{} KiB", bytes / 1024)
    }
}

fn sleep_selected(active: SleepTimer, option: SleepTimer) -> bool {
    match (active, option) {
        (SleepTimer::Off, SleepTimer::Off) | (SleepTimer::EndOfTrack, SleepTimer::EndOfTrack) => {
            true
        }
        (SleepTimer::Minutes(a), SleepTimer::Minutes(b)) => a == b,
        (SleepTimer::FadeOut(_), SleepTimer::FadeOut(_)) => true,
        _ => false,
    }
}

fn any_provider(settings: &LyricsSettings) -> bool {
    settings.lrclib
        || settings.lyrics_plus
        || settings.kugou
        || settings.better_lyrics
        || settings.paxsenix
        || settings.zemer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PlayerExtras;
    use std::path::Path;

    #[test]
    fn playback_settings_render_at_both_window_classes() {
        let mut settings = Settings::default();
        for (width, height) in [(405.0, 900.0), (1100.0, 760.0)] {
            let ctx = egui::Context::default();
            let mut extras = PlayerExtras {
                translation_enabled: true,
                translation_provider: crate::state::TranslationProvider::OpenAiCompatible,
                translation_endpoint: "https://llm.example/v1/chat/completions".to_string(),
                translation_target: "en".to_string(),
                ..PlayerExtras::default()
            };
            let mut translation_key = "session-key".to_string();
            let mut lastfm = LastFmUiState::default();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, height),
                )),
                ..Default::default()
            };
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        actions = show(
                            ui,
                            &Theme::from_settings(&settings),
                            &mut settings,
                            SettingsStatus {
                                extras: &mut extras,
                                crossfade_available: true,
                                focus_playback: false,
                                focus_sleep: false,
                                focus_audio: false,
                                image_cache: 0,
                                image_cache_bytes: 0,
                                db_path: Path::new("/tmp/metrolist.db"),
                                desktop: DesktopStatus {
                                    mpris: true,
                                    tray: true,
                                },
                                download_dir: Path::new("/tmp/downloads"),
                                download_used: 0,
                                download_limit: None,
                                download_active_limit: None,
                                download_failed: 0,
                                download_quality: Some(AudioQuality::Auto),
                                downloads_available: false,
                                download_error: None,
                                account_signed_in: false,
                                account_name: None,
                                translation_key: &mut translation_key,
                                lastfm: &mut lastfm,
                                lastfm_status: LastFmStatus::default(),
                                discord_status: DiscordStatus::default(),
                                focus_integrations: false,
                            },
                            width,
                        );
                    });
                });
            });
            output.textures_delta.clear();
            assert!(
                actions.is_empty(),
                "unexpected settings action at {width}: {actions:?}"
            );
        }
    }
    fn render_integration_settings(
        width: f32,
        height: f32,
        lastfm: &mut LastFmUiState,
        lastfm_status: &LastFmStatus,
        discord_status: &DiscordStatus,
    ) -> Vec<SettingsAction> {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let mut extras = PlayerExtras {
            lastfm_enabled: true,
            discord_enabled: true,
            ..PlayerExtras::default()
        };
        let mut translation_key = String::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, height),
            )),
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    actions = show(
                        ui,
                        &Theme::from_settings(&settings),
                        &mut settings,
                        SettingsStatus {
                            extras: &mut extras,
                            crossfade_available: true,
                            focus_playback: false,
                            focus_sleep: false,
                            focus_audio: false,
                            image_cache: 0,
                            image_cache_bytes: 0,
                            db_path: Path::new("/tmp/metrolist.db"),
                            desktop: DesktopStatus {
                                mpris: true,
                                tray: true,
                            },
                            download_dir: Path::new("/tmp/downloads"),
                            download_used: 0,
                            download_limit: None,
                            download_active_limit: None,
                            download_failed: 0,
                            download_quality: Some(AudioQuality::Auto),
                            downloads_available: false,
                            download_error: None,
                            account_signed_in: false,
                            account_name: None,
                            translation_key: &mut translation_key,
                            lastfm,
                            lastfm_status: lastfm_status.clone(),
                            discord_status: discord_status.clone(),
                            focus_integrations: true,
                        },
                        width,
                    );
                });
            });
        });
        output.textures_delta.clear();
        actions
    }

    #[test]
    fn integration_states_render_at_both_sizes() {
        for (width, height) in [(405.0, 900.0), (1100.0, 760.0)] {
            let mut disconnected = LastFmUiState {
                enabled: true,
                api_key: "api-key".to_string(),
                ..LastFmUiState::default()
            };
            let actions = render_integration_settings(
                width,
                height,
                &mut disconnected,
                &LastFmStatus::default(),
                &DiscordStatus {
                    enabled: true,
                    waiting: true,
                    ..DiscordStatus::default()
                },
            );
            assert!(actions.is_empty(), "disconnected at {width}: {actions:?}");

            let mut connected = LastFmUiState {
                enabled: true,
                api_key: "api-key".to_string(),
                secret: "secret".to_string(),
                session: "session".to_string(),
                username: "listener".to_string(),
                ..LastFmUiState::default()
            };
            let actions = render_integration_settings(
                width,
                height,
                &mut connected,
                &LastFmStatus {
                    signed_in: true,
                    user: Some("listener".to_string()),
                    scrobbles_sent: 128,
                    queued: 3,
                    ..LastFmStatus::default()
                },
                &DiscordStatus {
                    enabled: true,
                    connected: true,
                    ..DiscordStatus::default()
                },
            );
            assert!(actions.is_empty(), "connected at {width}: {actions:?}");

            let mut error = LastFmUiState {
                enabled: true,
                error: Some("Last.fm session rejected. Reconnect.".to_string()),
                ..LastFmUiState::default()
            };
            let actions = render_integration_settings(
                width,
                height,
                &mut error,
                &LastFmStatus {
                    session_invalid: true,
                    last_error: Some("Last.fm session rejected (9)".to_string()),
                    queued: 7,
                    ..LastFmStatus::default()
                },
                &DiscordStatus {
                    enabled: true,
                    waiting: true,
                    last_error: Some("Discord not running — retrying".to_string()),
                    ..DiscordStatus::default()
                },
            );
            assert!(actions.is_empty(), "error at {width}: {actions:?}");
        }
    }
}
