//! Listen Together lobby, room, users, queue and host controls.

use eframe::egui::{self, Align, Layout, RichText, Ui, Vec2};
use ml_core::{format_position, PlayerSnapshot};
use ml_together::{
    Role, RoomState, ServerInfo, SessionSnapshot, TrackInfo, UserInfo, DEFAULT_SERVER_URL, SERVERS,
};

use crate::state::{TogetherPhase, TogetherState};
use crate::theme::{medium_font, regular_font, Theme, DP12, DP16, DP4, DP8, TITLE_LARGE};
use crate::widgets::{chip, filled_button, outlined_button, paint_icon, section_header, slider};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TogetherAction {
    Create,
    Join,
    Retry,
    Leave,
    Copied,
    ApproveJoin(String),
    RejectJoin(String),
    HostPlayPause,
    HostNext,
    HostPrevious,
    HostSeek(u64),
    HostSync,
    PlayRoomTrack(String),
}

fn server_info(url: &str) -> ServerInfo {
    SERVERS
        .iter()
        .find(|server| server.url == url)
        .copied()
        .unwrap_or(SERVERS[0])
}

fn card(ui: &mut Ui, theme: &Theme, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme.scheme.surface_container)
        .corner_radius(16)
        .inner_margin(egui::Margin::same(DP16 as i8))
        .show(ui, add);
}

fn label(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
    );
}

fn lobby(ui: &mut Ui, theme: &Theme, state: &mut TogetherState, actions: &mut Vec<TogetherAction>) {
    ui.label(
        RichText::new("Listen together")
            .font(medium_font(TITLE_LARGE))
            .color(theme.scheme.on_surface),
    );
    label(
        ui,
        theme,
        "Host a room from this device or join one with a room code.",
    );
    ui.add_space(DP12);

    card(ui, theme, |ui| {
        ui.label(
            RichText::new("Room server")
                .font(medium_font(14.0))
                .color(theme.scheme.on_surface),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = DP8;
            for server in SERVERS {
                let selected = state.server == server.url;
                if chip(ui, theme, server.name, selected).clicked() && !selected {
                    state.server = server.url.to_string();
                }
            }
        });
        let server = server_info(&state.server);
        label(
            ui,
            theme,
            &format!(
                "{} · {} · operated by {}",
                server.name, server.location, server.operator
            ),
        );
        ui.add_space(DP12);
        ui.label(
            RichText::new("Display name")
                .font(medium_font(13.0))
                .color(theme.scheme.on_surface),
        );
        ui.add(
            egui::TextEdit::singleline(&mut state.display_name)
                .hint_text("Your name in the room")
                .desired_width(ui.available_width()),
        );
        ui.add_space(DP12);
        if filled_button(ui, theme, Some("add"), "Create room").clicked()
            && !state.display_name.trim().is_empty()
        {
            actions.push(TogetherAction::Create);
        }
        ui.add_space(DP12);
        label(ui, theme, "or join with a room code");
        ui.horizontal(|ui| {
            let code = egui::TextEdit::singleline(&mut state.join_code)
                .hint_text("Room code")
                .char_limit(16)
                .desired_width((ui.available_width() - 108.0).max(80.0));
            ui.add(code);
            let can_join =
                !state.display_name.trim().is_empty() && !state.join_code.trim().is_empty();
            if outlined_button(ui, theme, "Join").clicked() && can_join {
                actions.push(TogetherAction::Join);
            }
        });
        ui.add_space(DP12);
        label(
            ui,
            theme,
            "The room code is the only invite. Anyone with it can ask to join.",
        );
    });

    if let Some(error) = state.error.as_deref() {
        ui.add_space(DP12);
        egui::Frame::new()
            .fill(theme.scheme.error_container)
            .corner_radius(12)
            .inner_margin(egui::Margin::same(DP12 as i8))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(error)
                        .font(regular_font(12.0))
                        .color(theme.scheme.on_error_container),
                );
                ui.add_space(DP8);
                if outlined_button(ui, theme, "Try again").clicked() {
                    actions.push(TogetherAction::Retry);
                }
            });
    }
}

fn waiting(ui: &mut Ui, theme: &Theme, state: &TogetherState, actions: &mut Vec<TogetherAction>) {
    ui.label(
        RichText::new("Listen together")
            .font(medium_font(TITLE_LARGE))
            .color(theme.scheme.on_surface),
    );
    ui.add_space(DP12);
    card(ui, theme, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(18.0));
            ui.label(
                RichText::new(state.phase.label())
                    .font(medium_font(16.0))
                    .color(theme.scheme.on_surface),
            );
        });
        ui.add_space(DP4);
        label(
            ui,
            theme,
            if state.room_code().is_some() {
                "Waiting for the host to approve your request."
            } else {
                "Contacting the room server. This can take a few seconds."
            },
        );
        if let Some(error) = state.error.as_deref() {
            ui.add_space(DP8);
            ui.label(
                RichText::new(error)
                    .font(regular_font(12.0))
                    .color(theme.scheme.error),
            );
        }
        ui.add_space(DP12);
        if outlined_button(ui, theme, "Leave").clicked() {
            actions.push(TogetherAction::Leave);
        }
    });
}

fn role_note(ui: &mut Ui, theme: &Theme, host: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        let (label, fill, text) = if host {
            (
                "Host",
                theme.scheme.primary_container,
                theme.scheme.on_primary_container,
            )
        } else {
            (
                "Guest",
                theme.scheme.secondary_container,
                theme.scheme.on_secondary_container,
            )
        };
        egui::Frame::new()
            .fill(fill)
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(DP8 as i8, 3))
            .show(ui, |ui| {
                ui.label(RichText::new(label).font(medium_font(11.0)).color(text));
            });
        ui.label(
            RichText::new(if host {
                "You control playback"
            } else {
                "Host controls playback"
            })
            .font(regular_font(12.0))
            .color(theme.scheme.on_surface_variant),
        );
    });
}

fn users(ui: &mut Ui, theme: &Theme, state: &TogetherState, room: &RoomState) {
    section_header(ui, theme, "In the room", ui.available_width(), false);
    let me = state.user_id();
    for user in &room.users {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::hover());
        let initial = user
            .username
            .chars()
            .next()
            .unwrap_or('?')
            .to_ascii_uppercase();
        let row_host = user.is_host || user.user_id == room.host_id;
        ui.painter().circle_filled(
            rect.left_center() + Vec2::new(18.0, 0.0),
            16.0,
            theme.scheme.surface_container_highest,
        );
        ui.painter().text(
            rect.left_center() + Vec2::new(18.0, 0.0),
            egui::Align2::CENTER_CENTER,
            initial.to_string(),
            medium_font(13.0),
            theme.scheme.on_surface,
        );
        let name_x = if row_host {
            paint_icon(
                ui.painter(),
                rect.left_center() + Vec2::new(44.0, 0.0),
                "crown",
                16.0,
                theme.scheme.primary,
            );
            64.0
        } else {
            44.0
        };
        ui.painter().text(
            rect.left_top() + Vec2::new(name_x, 11.0),
            egui::Align2::LEFT_TOP,
            &user.username,
            medium_font(14.0),
            theme.scheme.on_surface,
        );
        let mut badges = Vec::new();
        if me == Some(user.user_id.as_str()) {
            badges.push("You");
        }
        if row_host {
            badges.push("Host");
        }
        if !user.is_connected {
            badges.push("Reconnecting");
        }
        if !badges.is_empty() {
            let text = badges.join(" · ");
            let name_width =
                crate::widgets::text_width(ui.painter(), &user.username, medium_font(14.0));
            ui.painter().text(
                rect.left_top() + Vec2::new(name_x + name_width + DP8, 13.0),
                egui::Align2::LEFT_TOP,
                text,
                regular_font(11.0),
                theme.scheme.primary,
            );
        }
    }
}

fn pending_requests(
    ui: &mut Ui,
    theme: &Theme,
    session: &SessionSnapshot,
    actions: &mut Vec<TogetherAction>,
) {
    if session.pending_join_requests.is_empty() {
        return;
    }
    section_header(ui, theme, "Join requests", ui.available_width(), false);
    for request in &session.pending_join_requests {
        card(ui, theme, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(&request.username)
                        .font(medium_font(14.0))
                        .color(theme.scheme.on_surface),
                );
                if outlined_button(ui, theme, "Approve").clicked() {
                    actions.push(TogetherAction::ApproveJoin(request.user_id.clone()));
                }
                if outlined_button(ui, theme, "Reject").clicked() {
                    actions.push(TogetherAction::RejectJoin(request.user_id.clone()));
                }
            });
        });
        ui.add_space(DP8);
    }
}

fn notices(ui: &mut Ui, theme: &Theme, state: &TogetherState) {
    if state.notices.is_empty() {
        return;
    }
    ui.add_space(DP8);
    egui::Frame::new()
        .fill(theme.scheme.surface_container_low)
        .corner_radius(12)
        .inner_margin(egui::Margin::same(DP12 as i8))
        .show(ui, |ui| {
            for notice in &state.notices {
                ui.label(
                    RichText::new(notice)
                        .font(regular_font(12.0))
                        .color(theme.scheme.on_surface_variant),
                );
            }
        });
}

fn current_track(ui: &mut Ui, theme: &Theme, room: &RoomState) {
    let Some(track) = room.current_track.as_ref() else {
        return;
    };
    section_header(ui, theme, "Now playing", ui.available_width(), false);
    card(ui, theme, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 8, theme.scheme.surface_container_highest);
            paint_icon(
                ui.painter(),
                rect.center(),
                if room.is_playing {
                    "graphic_eq"
                } else {
                    "pause"
                },
                24.0,
                theme.scheme.primary,
            );
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(&track.title)
                        .font(medium_font(15.0))
                        .color(theme.scheme.on_surface),
                );
                label(ui, theme, &track.artist);
            });
        });
    });
}

fn queue(
    ui: &mut Ui,
    theme: &Theme,
    room: &RoomState,
    host: bool,
    actions: &mut Vec<TogetherAction>,
) {
    section_header(ui, theme, "Room queue", ui.available_width(), false);
    if room.queue.is_empty() {
        label(
            ui,
            theme,
            if host {
                "Nothing queued. Start playback to publish your queue."
            } else {
                "The host has not queued anything yet."
            },
        );
        return;
    }
    for (index, track) in room.queue.iter().enumerate() {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), egui::Sense::click());
        if response.hovered() && host {
            ui.painter()
                .rect_filled(rect, 8, theme.scheme.surface_container_high);
        }
        ui.painter().text(
            rect.left_top() + Vec2::new(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            format!("{}", index + 1),
            regular_font(11.0),
            theme.scheme.on_surface_variant,
        );
        ui.painter().text(
            rect.left_top() + Vec2::new(28.0, 6.0),
            egui::Align2::LEFT_TOP,
            &track.title,
            medium_font(14.0),
            theme.scheme.on_surface,
        );
        let added = if track.suggested_by.is_empty() {
            "Host queue".to_string()
        } else {
            format!("Added by {}", track.suggested_by)
        };
        ui.painter().text(
            rect.left_top() + Vec2::new(28.0, 26.0),
            egui::Align2::LEFT_TOP,
            format!("{} · {}", track.artist, added),
            regular_font(11.0),
            theme.scheme.on_surface_variant,
        );
        if host && response.clicked() {
            actions.push(TogetherAction::PlayRoomTrack(track.id.clone()));
        }
    }
}

fn controls(
    ui: &mut Ui,
    theme: &Theme,
    state: &TogetherState,
    room: &RoomState,
    snapshot: &PlayerSnapshot,
    actions: &mut Vec<TogetherAction>,
) {
    let host = state.is_host();
    ui.add_space(DP8);
    ui.label(
        RichText::new("Playback")
            .font(medium_font(14.0))
            .color(theme.scheme.on_surface),
    );
    ui.add_space(DP4);
    if !host {
        label(
            ui,
            theme,
            "The host controls playback. Your local transport is locked while you are in the room.",
        );
    }
    let current_duration = room
        .current_track
        .as_ref()
        .map(|track| track.duration.max(0) as u64)
        .unwrap_or(0);
    let position = if host && !state.demo && snapshot.current.is_some() {
        snapshot.position_ms
    } else {
        room.position.max(0) as u64
    };
    let duration = if host && !state.demo && snapshot.duration_ms > 0 {
        snapshot.duration_ms
    } else {
        current_duration
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        ui.scope(|ui| {
            ui.set_opacity(if host { 1.0 } else { 0.45 });
            let play = filled_button(
                ui,
                theme,
                Some(if room.is_playing {
                    "pause"
                } else {
                    "play_arrow"
                }),
                if room.is_playing { "Pause" } else { "Play" },
            );
            if host && play.clicked() {
                actions.push(TogetherAction::HostPlayPause);
            }
            if host && outlined_button(ui, theme, "Previous").clicked() {
                actions.push(TogetherAction::HostPrevious);
            }
            if host && outlined_button(ui, theme, "Next").clicked() {
                actions.push(TogetherAction::HostNext);
            }
            if host && outlined_button(ui, theme, "Sync now").clicked() {
                actions.push(TogetherAction::HostSync);
            }
        });
    });
    ui.add_space(DP8);
    let slider_width = ui.available_width();
    let mut progress = if duration > 0 {
        (position as f32 / duration as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let seek = ui
        .scope(|ui| {
            ui.set_opacity(if host { 1.0 } else { 0.45 });
            slider(
                ui,
                theme,
                &mut progress,
                0.0..=1.0,
                slider_width,
                "Seek room playback",
            )
        })
        .inner
        .on_hover_text("Seek room playback");
    if host && seek.changed() && duration > 0 {
        actions.push(TogetherAction::HostSeek(
            (progress * duration as f32).round() as u64,
        ));
    }
    ui.scope(|ui| {
        ui.set_opacity(if host { 1.0 } else { 0.45 });
        ui.allocate_ui_with_layout(
            Vec2::new(slider_width, 18.0),
            Layout::left_to_right(Align::Min),
            |ui| {
                label(ui, theme, &format_position(position));
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    label(ui, theme, &format_position(duration));
                });
            },
        );
    });
}

fn connected(
    ui: &mut Ui,
    theme: &Theme,
    state: &mut TogetherState,
    snapshot: &PlayerSnapshot,
    actions: &mut Vec<TogetherAction>,
    width: f32,
) {
    let Some(room) = state.room().cloned() else {
        return;
    };
    let host = state.is_host();
    let code = state.room_code().unwrap_or("—").to_string();
    let server = server_info(&state.server);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Room {code}"))
                .font(medium_font(TITLE_LARGE))
                .color(theme.scheme.on_surface),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if outlined_button(ui, theme, "Leave").clicked() {
                actions.push(TogetherAction::Leave);
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if outlined_button(ui, theme, "Copy code").clicked() {
            ui.ctx().copy_text(code.clone());
            actions.push(TogetherAction::Copied);
        }
        label(ui, theme, &format!("{} · {}", server.name, server.location));
    });
    role_note(ui, theme, host);
    if matches!(state.phase, TogetherPhase::Reconnecting { .. }) {
        ui.add_space(DP8);
        ui.label(
            RichText::new(state.phase.label())
                .font(regular_font(12.0))
                .color(theme.scheme.error),
        );
    } else if let Some(error) = state.error.as_deref() {
        ui.add_space(DP8);
        ui.label(
            RichText::new(error)
                .font(regular_font(12.0))
                .color(theme.scheme.error),
        );
    }
    let session = state.session.clone();
    if width >= 760.0 {
        ui.add_space(DP8);
        ui.columns(2, |columns| {
            if let Some(session) = session.as_ref() {
                pending_requests(&mut columns[0], theme, session, actions);
            }
            users(&mut columns[0], theme, state, &room);
            notices(&mut columns[0], theme, state);
            columns[0].add_space(DP8);
            queue(&mut columns[0], theme, &room, host, actions);
            if !host {
                columns[0].add_space(DP8);
                label(
                    &mut columns[0],
                    theme,
                    "Suggestions and host migration are not available in this build.",
                );
            }
            current_track(&mut columns[1], theme, &room);
            if host {
                controls(&mut columns[1], theme, state, &room, snapshot, actions);
            } else {
                columns[1].add_space(DP12);
                label(
                    &mut columns[1],
                    theme,
                    "Playback is controlled by the host. Local transport is locked while you are in the room.",
                );
            }
        });
    } else {
        if let Some(session) = session.as_ref() {
            pending_requests(ui, theme, session, actions);
        }
        users(ui, theme, state, &room);
        notices(ui, theme, state);
        ui.add_space(DP8);
        current_track(ui, theme, &room);
        if host {
            controls(ui, theme, state, &room, snapshot, actions);
        }
        ui.add_space(DP8);
        queue(ui, theme, &room, host, actions);
        if !host {
            ui.add_space(DP12);
            label(
                ui,
                theme,
                "Playback is controlled by the host. Local transport is locked while you are in the room.",
            );
        }
    }
}

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    state: &mut TogetherState,
    snapshot: &PlayerSnapshot,
    width: f32,
) -> Vec<TogetherAction> {
    let mut actions = Vec::new();
    if state.room().is_some() {
        connected(ui, theme, state, snapshot, &mut actions, width);
    } else if matches!(
        state.phase,
        TogetherPhase::Connecting | TogetherPhase::Connected | TogetherPhase::Reconnecting { .. }
    ) {
        waiting(ui, theme, state, &mut actions);
    } else {
        lobby(ui, theme, state, &mut actions);
    }
    actions
}

fn track(id: &str, title: &str, artist: &str, added_by: &str) -> TrackInfo {
    TrackInfo {
        id: id.to_string(),
        title: title.to_string(),
        artist: artist.to_string(),
        duration: 214_000,
        suggested_by: added_by.to_string(),
        ..TrackInfo::default()
    }
}

/// Deterministic room fixture for screenshots and render tests. No network.
pub fn demo_session(host: bool) -> SessionSnapshot {
    let host_user = UserInfo {
        user_id: "u-nyx".to_string(),
        username: "Nyx".to_string(),
        is_host: true,
        is_connected: true,
    };
    let guest_user = UserInfo {
        user_id: "u-ava".to_string(),
        username: "Ava".to_string(),
        is_host: false,
        is_connected: true,
    };
    let current = track("JGwWNGJdvx8", "Shape of You", "Ed Sheeran", "Nyx");
    let queue = vec![
        track("kJQP7kiw5Fk", "Despacito", "Luis Fonsi", "Ava"),
        track("fJ9rUzIMcZQ", "Bohemian Rhapsody", "Queen", "Nyx"),
        track(
            "dQw4w9WgXcQ",
            "Never Gonna Give You Up",
            "Rick Astley",
            "Ava",
        ),
    ];
    let users = if host {
        vec![host_user]
    } else {
        vec![host_user, guest_user]
    };
    let room = RoomState {
        room_code: "R7RGSQUU".to_string(),
        host_id: "u-nyx".to_string(),
        users,
        current_track: Some(current),
        is_playing: true,
        position: 42_000,
        last_update: 1_000,
        volume: 0.8,
        queue,
        revision: 7,
    };
    SessionSnapshot {
        room: Some(room),
        role: if host { Role::Host } else { Role::Guest },
        user_id: Some(if host { "u-nyx" } else { "u-ava" }.to_string()),
        room_code: Some("R7RGSQUU".to_string()),
        username: Some(if host { "Nyx" } else { "Ava" }.to_string()),
        pending_join_requests: if host {
            vec![ml_together::JoinRequestPayload {
                user_id: "u-sam".to_string(),
                username: "Sam".to_string(),
            }]
        } else {
            Vec::new()
        },
        pending_suggestions: Vec::new(),
        buffering_users: Vec::new(),
    }
}

pub fn demo_state(host: bool) -> TogetherState {
    let mut state = TogetherState::new(if host { "Nyx" } else { "Ava" });
    state.server = DEFAULT_SERVER_URL.to_string();
    state.phase = TogetherPhase::Connected;
    state.session = Some(demo_session(host));
    state.demo = true;
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    fn render(state: &mut TogetherState, width: f32, height: f32) -> Vec<TogetherAction> {
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, height),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    actions = show(
                        ui,
                        &Theme::from_settings(&ml_core::Settings::default()),
                        state,
                        &PlayerSnapshot::default(),
                        width,
                    );
                });
            });
        });
        output.textures_delta.clear();
        actions
    }

    #[test]
    fn renders_lobby_waiting_host_guest_and_error_at_both_sizes() {
        for (width, height) in [(405.0, 900.0), (1100.0, 760.0)] {
            let mut lobby = TogetherState::new("Nyx");
            lobby.join_code = "R7RG".to_string();
            assert!(render(&mut lobby, width, height).is_empty());

            let mut waiting = TogetherState::new("Ava");
            waiting.phase = TogetherPhase::Connecting;
            assert!(render(&mut waiting, width, height).is_empty());

            let mut host = demo_state(true);
            assert!(render(&mut host, width, height).is_empty());

            let mut guest = demo_state(false);
            assert!(render(&mut guest, width, height).is_empty());

            let mut error = TogetherState::new("Ava");
            error.phase = TogetherPhase::Error;
            error.error = Some("Could not reach the room server".to_string());
            assert!(render(&mut error, width, height).is_empty());
        }
    }

    #[test]
    fn demo_roles_and_queue_are_deterministic() {
        let host = demo_session(true);
        assert_eq!(host.role, Role::Host);
        assert_eq!(host.pending_join_requests.len(), 1);
        assert_eq!(host.room.unwrap().queue.len(), 3);

        let guest = demo_session(false);
        assert_eq!(guest.role, Role::Guest);
        assert_eq!(guest.user_id.as_deref(), Some("u-ava"));
        assert!(guest.pending_join_requests.is_empty());
    }
}
