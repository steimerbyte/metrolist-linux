//! Account sign-in via browser cookies and library sync.

use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Ui, Vec2};
use ml_innertube::auth::AccountInfo;

use crate::art::ArtCache;
use crate::pure::expiry_hint;
use crate::services::Services;
use crate::state::{AccountState, SyncCounts};
use crate::theme::{
    bold_font, medium_font, regular_font, Theme, DP12, DP4, DP8, TITLE_LARGE, TITLE_MEDIUM,
};
use crate::widgets::{art_or_placeholder, filled_button, outlined_button};

use super::measured_column;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountAction {
    ImportFile,
    ImportBrowser,
    Refresh,
    Sync,
    SignOut,
}

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    state: &mut AccountState,
    width: f32,
) -> Vec<AccountAction> {
    let avatar = state
        .session
        .info
        .as_ref()
        .and_then(|info| info.thumbnail_url.as_deref())
        .and_then(|url| art.texture(ui.ctx(), services, url));
    render(ui, theme, state, avatar.as_ref(), width)
}

/// Rendering without services/network, used by the render smoke tests.
fn render(
    ui: &mut Ui,
    theme: &Theme,
    state: &mut AccountState,
    avatar: Option<&egui::TextureHandle>,
    width: f32,
) -> Vec<AccountAction> {
    let mut actions = Vec::new();
    let content_width = width.min(600.0);
    measured_column(ui, 600.0, |ui| {
        if state.session.signed_in {
            signed_in(ui, theme, state, avatar, &mut actions);
        } else {
            signed_out(ui, theme, state, content_width, &mut actions);
        }
    });
    actions
}

fn signed_out(
    ui: &mut Ui,
    theme: &Theme,
    state: &mut AccountState,
    width: f32,
    actions: &mut Vec<AccountAction>,
) {
    ui.add_space(DP4);
    ui.label(
        egui::RichText::new("YouTube Music account")
            .font(bold_font(TITLE_LARGE))
            .color(theme.scheme.on_surface),
    );
    ui.label(
        egui::RichText::new("Sign in by importing cookies exported from a browser that is signed in to music.youtube.com.")
            .font(regular_font(13.0))
            .color(theme.scheme.on_surface_variant),
    );
    ui.add_space(DP12);

    if let Some(cache) = state.cache.as_ref() {
        ui.label(
            egui::RichText::new(format!("Last cached account: {}", cache.info.name))
                .font(medium_font(13.0))
                .color(theme.scheme.on_surface),
        );
        if let Some(contact) = contact_line(&cache.info.masked()) {
            ui.label(
                egui::RichText::new(contact)
                    .font(regular_font(12.0))
                    .color(theme.scheme.on_surface_variant),
            );
        }
        if cache.synced_at.is_some() {
            ui.label(
                egui::RichText::new(sync_label(cache.counts))
                    .font(regular_font(12.0))
                    .color(theme.scheme.on_surface_variant),
            );
        }
        ui.add_space(DP8);
    }

    ui.label(
        egui::RichText::new("Cookies file path")
            .font(medium_font(13.0))
            .color(theme.scheme.on_surface),
    );
    ui.add(
        egui::TextEdit::singleline(&mut state.imported_path)
            .hint_text("/home/you/Documents/cookies.txt")
            .desired_width(width),
    );
    ui.add_space(DP8);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if filled_button(ui, theme, Some("upload_file"), "Import file").clicked() {
            actions.push(AccountAction::ImportFile);
        }
        let profile = state.browser_profile.as_deref();
        let response = ui
            .add_enabled_ui(profile.is_some(), |ui| {
                outlined_button(ui, theme, "Import from LibreWolf")
            })
            .inner;
        if response.clicked() && profile.is_some() {
            actions.push(AccountAction::ImportBrowser);
        }
        if outlined_button(ui, theme, "Refresh account").clicked() {
            actions.push(AccountAction::Refresh);
        }
    });
    match state.browser_profile.as_deref() {
        Some(profile) => ui.label(
            egui::RichText::new(format!("Newest LibreWolf/Firefox profile: {profile}"))
                .font(regular_font(12.0))
                .color(theme.scheme.on_surface_variant),
        ),
        None => ui.label(
            egui::RichText::new(
                "No LibreWolf/Firefox profile with a readable cookie database was detected.",
            )
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
        ),
    };
    ui.add_space(DP8);
    ui.label(
        egui::RichText::new("A stale export can be rejected. The saved session rotates only while the source browser remains signed in; the cookies file is stored privately (0600).")
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
    );

    if state.loading {
        ui.add_space(DP12);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(
                egui::RichText::new("Checking cookies with YouTube Music…")
                    .font(regular_font(13.0))
                    .color(theme.scheme.on_surface_variant),
            );
        });
    }
    if let Some(error) = state.error.as_deref() {
        error_text(ui, theme, error);
    }
}

fn signed_in(
    ui: &mut Ui,
    theme: &Theme,
    state: &mut AccountState,
    avatar: Option<&egui::TextureHandle>,
    actions: &mut Vec<AccountAction>,
) {
    let Some(info) = state.session.info.as_ref() else {
        ui.add_space(DP4);
        ui.label(
            egui::RichText::new("Signed in")
                .font(bold_font(TITLE_LARGE))
                .color(theme.scheme.on_surface),
        );
        ui.label(
            egui::RichText::new(
                "A cookie session is loaded, but account details have not been fetched yet.",
            )
            .font(regular_font(13.0))
            .color(theme.scheme.on_surface_variant),
        );
        ui.add_space(DP12);
        if filled_button(ui, theme, Some("refresh"), "Refresh account").clicked() {
            actions.push(AccountAction::Refresh);
        }
        if let Some(error) = state.error.as_deref() {
            error_text(ui, theme, error);
        }
        return;
    };

    ui.add_space(DP4);
    let avatar_size = 72.0;
    let (avatar_rect, _) = ui.allocate_exact_size(Vec2::splat(avatar_size), egui::Sense::hover());
    art_or_placeholder(ui, theme, avatar, avatar_rect, 36);
    ui.add_space(DP8);
    ui.label(
        egui::RichText::new(&info.name)
            .font(bold_font(TITLE_MEDIUM))
            .color(theme.scheme.on_surface),
    );
    if let Some(contact) = contact_line(&info.masked()) {
        ui.label(
            egui::RichText::new(contact)
                .font(regular_font(13.0))
                .color(theme.scheme.on_surface_variant),
        );
    }
    if let Some(hint) = expiry_hint(state.session.expires_at, unix_now()) {
        ui.label(
            egui::RichText::new(hint)
                .font(regular_font(12.0))
                .color(theme.scheme.on_surface_variant),
        );
    }
    ui.add_space(DP12);

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        let sync_label = if state.syncing {
            "Syncing…"
        } else {
            "Sync library"
        };
        if filled_button(ui, theme, Some("sync"), sync_label).clicked() && !state.syncing {
            actions.push(AccountAction::Sync);
        }
        if outlined_button(ui, theme, "Refresh account").clicked() {
            actions.push(AccountAction::Refresh);
        }
        if outlined_button(ui, theme, "Sign out").clicked() {
            actions.push(AccountAction::SignOut);
        }
    });

    ui.add_space(DP12);
    if state.session.synced_at.is_some() || state.session.counts.total() > 0 {
        ui.label(
            egui::RichText::new("Last library sync")
                .font(medium_font(13.0))
                .color(theme.scheme.on_surface),
        );
        ui.label(
            egui::RichText::new(sync_label(state.session.counts))
                .font(regular_font(13.0))
                .color(theme.scheme.on_surface_variant),
        );
    } else {
        ui.label(
            egui::RichText::new("No library sync yet. Syncing upserts songs, albums, artists and playlists into the local database.")
                .font(regular_font(12.0))
                .color(theme.scheme.on_surface_variant),
        );
    }
    ui.label(
        egui::RichText::new(format!(
            "Session cookies: {}",
            state.session.snapshot.cookie_count
        ))
        .font(regular_font(11.0))
        .color(theme.scheme.on_surface_variant),
    );
    ui.add_space(DP8);
    ui.label(
        egui::RichText::new("The session file is private (0600). Cookies rotate only while the source browser is signed in; if Google rejects them, import a fresh export.")
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
    );
    if state.loading || state.syncing {
        ui.add_space(DP8);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(
                egui::RichText::new(if state.syncing {
                    "Reading the signed-in library…"
                } else {
                    "Refreshing account…"
                })
                .font(regular_font(12.0))
                .color(theme.scheme.on_surface_variant),
            );
        });
    }
    if let Some(error) = state.error.as_deref() {
        error_text(ui, theme, error);
    }
}

fn contact_line(info: &AccountInfo) -> Option<String> {
    info.email
        .clone()
        .or_else(|| info.channel_handle.clone())
        .or_else(|| info.channel_id.clone())
}

fn sync_label(counts: SyncCounts) -> String {
    format!(
        "{} songs · {} albums · {} artists · {} playlists",
        counts.songs, counts.albums, counts.artists, counts.playlists
    )
}

fn error_text(ui: &mut Ui, theme: &Theme, error: &str) {
    ui.add_space(DP8);
    ui.label(
        egui::RichText::new(error)
            .font(regular_font(12.0))
            .color(theme.scheme.error),
    );
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AccountSession, SyncCounts};
    use ml_innertube::auth::{AccountInfo, AuthSnapshot};

    fn info() -> AccountInfo {
        AccountInfo {
            name: "Rahul".to_string(),
            email: Some("rahul@example.com".to_string()),
            channel_handle: Some("@rahul".to_string()),
            thumbnail_url: None,
            channel_id: Some("UCxyz".to_string()),
        }
    }

    fn signed_in() -> AccountState {
        let info = info();
        AccountState {
            session: AccountSession {
                signed_in: true,
                info: Some(info),
                identity: Default::default(),
                snapshot: AuthSnapshot {
                    authenticated: true,
                    cookie_count: 4,
                    has_sapisid: true,
                    has_data_sync_id: true,
                    auth_user: Some("0".to_string()),
                },
                synced_at: Some(1_700_000_000_000),
                counts: SyncCounts {
                    songs: 12,
                    albums: 2,
                    artists: 1,
                    playlists: 3,
                },
                expires_at: Some(1_800_000_000),
            },
            ..AccountState::default()
        }
    }

    fn render_at(width: f32, height: f32, mut state: AccountState) {
        let ctx = egui::Context::default();
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
                    actions = render(
                        ui,
                        &Theme::from_settings(&Default::default()),
                        &mut state,
                        None,
                        width,
                    );
                });
            });
        });
        output.textures_delta.clear();
        assert!(actions.is_empty(), "unexpected account action at {width}");
    }

    #[test]
    fn account_renders_signed_out_signed_in_and_error_at_both_sizes() {
        for (width, height) in [(405.0, 900.0), (1100.0, 760.0)] {
            let mut signed_out = AccountState {
                browser_profile: Some("default".to_string()),
                ..AccountState::default()
            };
            signed_out.imported_path = "/tmp/cookies.txt".to_string();
            render_at(width, height, signed_out);

            render_at(width, height, signed_in());

            let mut error = signed_in();
            error.error = Some("Google rejected this session. Import a fresh export.".to_string());
            error.loading = true;
            render_at(width, height, error);

            let mut failed_import = AccountState {
                browser_profile: None,
                error: Some("No LibreWolf profile found.".to_string()),
                ..AccountState::default()
            };
            failed_import.imported_path = String::new();
            render_at(width, height, failed_import);
        }
    }
}
