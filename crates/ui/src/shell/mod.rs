//! Responsive shell: top bar, filter chips, mini player, bottom nav and rail.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, TextureHandle, Ui, Vec2};

use crate::routes::Tab;
use crate::theme::{medium_font, regular_font, Theme, BODY_SMALL, DP20, DP4, DP8, TITLE_LARGE};
use crate::widgets::{chip, icon_button_at, nav_item, paint_icon};
use ml_core::Song;

/// Right-hand action cluster of the top app bar: 40 dp history + sync + avatar
/// hit areas plus the 8 dp gap before the title.
const TITLE_ACTIONS: f32 = 138.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopAction {
    Back,
    History,
    Sync,
    Avatar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiniAction {
    TogglePlay,
    Next,
    Previous,
    OpenPlayer,
}

/// 56 dp top app bar with history/sync/avatar affordances.
pub fn top_bar(
    ui: &mut Ui,
    app_theme: &Theme,
    title: &str,
    width: f32,
    back: bool,
) -> Option<TopAction> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 56.0), Sense::hover());
    let back_response = if back {
        Some(icon_button_at(
            ui,
            app_theme,
            "top_back",
            Rect::from_center_size(
                Pos2::new(rect.left() + 20.0, rect.center().y),
                Vec2::splat(40.0),
            ),
            "arrow_back",
            "Back",
            24.0,
        ))
    } else {
        None
    };
    let title_x = if back { 48.0 } else { 0.0 };
    // The history/sync/avatar hit areas start 130 dp before the right edge.
    let title_max = (rect.width() - title_x - TITLE_ACTIONS).max(0.0);
    crate::widgets::elided(
        ui.painter(),
        rect.left_top() + Vec2::new(title_x, 13.0),
        title,
        medium_font(TITLE_LARGE),
        app_theme.scheme.on_surface,
        title_max,
    );

    let avatar_center = Pos2::new(rect.right() - 22.0, rect.center().y);
    let avatar_rect = Rect::from_center_size(avatar_center, Vec2::splat(40.0));
    let avatar = ui.interact(
        avatar_rect,
        ui.scope_id().with("top_avatar"),
        Sense::click(),
    );
    ui.painter()
        .circle_filled(avatar_center, 14.0, Color32::from_rgb(0x3A, 0x4A, 0x5A));
    ui.painter().circle_filled(
        avatar_center + Vec2::new(0.0, 3.0),
        5.0,
        Color32::from_rgb(0x7A, 0x8A, 0x9A),
    );
    if avatar.hovered() {
        ui.painter().circle_stroke(
            avatar_center,
            16.0,
            egui::Stroke::new(1.0, app_theme.scheme.outline),
        );
    }
    avatar.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Account"));

    let sync = icon_button_at(
        ui,
        app_theme,
        "top_sync",
        Rect::from_center_size(avatar_center - Vec2::new(48.0, 0.0), Vec2::splat(40.0)),
        "sync",
        "Updates",
        24.0,
    );
    let history = icon_button_at(
        ui,
        app_theme,
        "top_history",
        Rect::from_center_size(avatar_center - Vec2::new(88.0, 0.0), Vec2::splat(40.0)),
        "history",
        "History",
        24.0,
    );

    if back_response.is_some_and(|response| response.clicked()) {
        Some(TopAction::Back)
    } else if history.clicked() {
        Some(TopAction::History)
    } else if sync.clicked() {
        Some(TopAction::Sync)
    } else if avatar.clicked() {
        Some(TopAction::Avatar)
    } else {
        None
    }
}

/// Horizontally scrolling home filters. Returns true when the selection changed.
pub fn filter_chips(
    ui: &mut Ui,
    app_theme: &Theme,
    labels: &[&str],
    selected: &mut Option<usize>,
) -> bool {
    let mut changed = false;
    egui::ScrollArea::horizontal()
        .id_salt("home_filter_chips")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = DP8;
                for (index, label) in labels.iter().enumerate() {
                    let response = chip(ui, app_theme, label, *selected == Some(index));
                    if response.clicked() && *selected != Some(index) {
                        *selected = Some(index);
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// Persistent 64 dp mini player. Returns an action when a control was clicked.
pub fn mini_player(
    ui: &mut Ui,
    app_theme: &Theme,
    song: &Song,
    playing: bool,
    texture: Option<&TextureHandle>,
    width: f32,
) -> Option<MiniAction> {
    if width < 220.0 {
        return None;
    }
    let (outer, _) = ui.allocate_exact_size(Vec2::new(width, 64.0), Sense::hover());
    let rect = outer.shrink2(Vec2::new(0.0, 2.0));
    ui.painter()
        .rect_filled(rect, 12, app_theme.scheme.surface_container);

    let art_rect = Rect::from_min_size(
        rect.left_center() + Vec2::new(8.0, -22.0),
        Vec2::splat(44.0),
    );
    crate::widgets::art_or_placeholder(ui, app_theme, texture, art_rect, 6);

    let text_x = art_rect.right() + 12.0;
    let controls_left = rect.right() - 132.0;
    let max_text = (controls_left - 12.0 - text_x).max(20.0);
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, art_rect.top() + 5.0),
        &song.title,
        medium_font(14.0),
        app_theme.scheme.on_surface,
        max_text,
    );
    let artist = song.artist_names();
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, art_rect.top() + 23.0),
        &artist,
        regular_font(12.0),
        app_theme.scheme.on_surface_variant,
        max_text,
    );

    let center_y = rect.center().y;
    let body_rect = Rect::from_min_max(rect.min, Pos2::new(controls_left, rect.bottom()));
    let body = ui.interact(
        body_rect,
        ui.scope_id().with(("mini_body", song.id.as_str())),
        Sense::click(),
    );
    body.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            format!("Open player: {} by {}", song.title, artist),
        )
    });

    let previous = icon_button_at(
        ui,
        app_theme,
        "mini_previous",
        Rect::from_center_size(Pos2::new(rect.right() - 112.0, center_y), Vec2::splat(40.0)),
        "skip_previous",
        "Previous",
        22.0,
    );
    let toggle = icon_button_at(
        ui,
        app_theme,
        "mini_toggle",
        Rect::from_center_size(Pos2::new(rect.right() - 70.0, center_y), Vec2::splat(44.0)),
        if playing { "pause" } else { "play_arrow" },
        if playing { "Pause" } else { "Play" },
        30.0,
    );
    let next = icon_button_at(
        ui,
        app_theme,
        "mini_next",
        Rect::from_center_size(Pos2::new(rect.right() - 28.0, center_y), Vec2::splat(40.0)),
        "skip_next",
        "Next",
        22.0,
    );

    if toggle.clicked() {
        Some(MiniAction::TogglePlay)
    } else if previous.clicked() {
        Some(MiniAction::Previous)
    } else if next.clicked() {
        Some(MiniAction::Next)
    } else if body.clicked() {
        Some(MiniAction::OpenPlayer)
    } else {
        None
    }
}

/// 80 dp bottom navigation bar (compact window class).
pub fn bottom_nav(ui: &mut Ui, app_theme: &Theme, active: Tab, width: f32) -> Option<Tab> {
    let mut chosen = None;
    ui.add_space(DP4);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let item_width = width / Tab::ALL.len() as f32;
        for tab in Tab::ALL {
            let response = nav_item(
                ui,
                app_theme,
                tab.icon(),
                tab.label(),
                tab == active,
                Vec2::new(item_width, 72.0),
            );
            if response.clicked() {
                chosen = Some(tab);
            }
        }
    });
    chosen
}

/// 88 dp navigation rail (expanded window class).
pub fn nav_rail(ui: &mut Ui, app_theme: &Theme, active: Tab) -> Option<Tab> {
    let mut chosen = None;
    ui.add_space(DP20);
    let (logo_rect, _) = ui.allocate_exact_size(Vec2::new(88.0, 48.0), Sense::hover());
    ui.painter()
        .circle_filled(logo_rect.center(), 18.0, app_theme.scheme.primary_container);
    paint_icon(
        ui.painter(),
        logo_rect.center(),
        "graphic_eq",
        22.0,
        app_theme.scheme.on_primary_container,
    );
    ui.add_space(DP20);
    for tab in Tab::ALL {
        let response = nav_item(
            ui,
            app_theme,
            tab.icon(),
            tab.label(),
            tab == active,
            Vec2::new(88.0, 72.0),
        );
        if response.clicked() {
            chosen = Some(tab);
        }
    }
    chosen
}

/// Window-class breakpoint matching Material 3 (`>= 840 dp` is expanded).
pub fn is_expanded(width: f32) -> bool {
    width >= 840.0
}

/// Small label helper for painted bars.
pub fn small_label(ui: &Ui, at: Pos2, value: &str, color: Color32) {
    crate::widgets::text(ui.painter(), at, value, regular_font(BODY_SMALL), color);
}
