//! Home: live InnerTube shelves with loading, error and empty states.

use eframe::egui::{self, Ui, Vec2};
use ml_core::{HomeFeed, QueueSource, Song};

use crate::art::ArtCache;
use crate::routes::Route;
use crate::services::Services;
use crate::state::Load;
use crate::theme::{
    bold_font, regular_font, state_layer, Theme, DP12, DP16, DP24, DP4, DP48, DP8, GRID_GAP,
};
use crate::widgets::{elided, empty_state, paint_icon, section_header};

use super::{error_block, load_more, section_block, skeleton_cards, skeleton_rows, Intent};

/// Upstream Home chips (the API also ships dynamic chips; these are the
/// curated fallback set). Duplicate "Party" in the brief is collapsed.
pub const CHIPS: [&str; 13] = [
    "Energize",
    "Workout",
    "Relax",
    "Feel good",
    "Commute",
    "Pop",
    "Hip-hop",
    "Chill",
    "Sleep",
    "Party",
    "Romance",
    "Sad",
    "Focus",
];

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    home: &Load<HomeFeed>,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();

    if home.initial_loading() {
        skeleton_cards(ui, theme, width);
        ui.add_space(DP8);
        skeleton_rows(ui, theme, width);
        ui.add_space(DP24);
        return intents;
    }

    let Some(feed) = home.loaded() else {
        if let Some(error) = &home.error {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
        } else {
            empty_state(
                ui,
                theme,
                "home",
                "Nothing here yet",
                "Pull fresh shelves from YouTube Music.",
            );
            ui.vertical_centered(|ui| {
                if crate::widgets::filled_button(ui, theme, Some("refresh"), "Reload home")
                    .clicked()
                {
                    intents.push(Intent::Refresh);
                }
            });
        }
        ui.add_space(DP24);
        return intents;
    };

    if feed.sections.is_empty() {
        empty_state(
            ui,
            theme,
            "inbox",
            "No shelves returned",
            "YouTube Music returned an empty home feed.",
        );
        ui.add_space(DP24);
        return intents;
    }

    intents.extend(discover(ui, theme, width));

    for section in &feed.sections {
        if section.items.is_empty() {
            continue;
        }
        ui.add_space(DP4);
        let playable = section.items.iter().all(|item| {
            matches!(
                item,
                ml_core::YtItem::Song(_) | ml_core::YtItem::Episode(_) | ml_core::YtItem::Video(_)
            )
        });
        let source = QueueSource::List;
        if let Some(response) = section_header(
            ui,
            theme,
            &section.title,
            width,
            playable && section.items.len() > 1,
        ) {
            if response.clicked() {
                let songs: Vec<Song> = section
                    .items
                    .iter()
                    .filter_map(crate::state::item_song)
                    .collect();
                if !songs.is_empty() {
                    intents.push(Intent::Play {
                        songs,
                        index: 0,
                        source,
                    });
                }
            }
        }
        intents.extend(section_block(
            ui, theme, services, art, section, current, width, source,
        ));
        ui.add_space(DP8);
    }

    if let Some(_token) = &home.more {
        if load_more(ui, theme, home.more_loading) {
            intents.push(Intent::LoadMore);
        }
    }
    ui.add_space(DP24);
    intents
}

/// Compact entry points to the catalog pages. Four-up when expanded, two-up
/// on the compact class; intentionally not search chips.
fn discover(ui: &mut Ui, theme: &Theme, width: f32) -> Vec<Intent> {
    const ENTRIES: [(&str, &str, &str, Route); 4] = [
        ("bar_chart", "Charts", "Top songs & artists", Route::Charts),
        ("category", "Moods & genres", "Browse by mood", Route::Moods),
        (
            "new_releases",
            "New releases",
            "Albums & videos",
            Route::NewReleases,
        ),
        ("podcasts", "Podcasts", "New episodes", Route::Podcasts),
    ];
    let mut intents = Vec::new();
    section_header(ui, theme, "Discover", width, false);
    let columns = if width >= 840.0 { 4 } else { 2 };
    let gap = GRID_GAP;
    let card_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(120.0);
    for chunk in ENTRIES.chunks(columns) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (icon, title, subtitle, route) in chunk {
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(card_width, 80.0), egui::Sense::click());
                let fill = state_layer(
                    theme.scheme.surface_container_high,
                    response.hovered(),
                    response.is_pointer_button_down_on(),
                );
                ui.painter().rect_filled(rect, 12, fill);
                let icon_center = rect.left_top() + Vec2::new(28.0, 28.0);
                ui.painter()
                    .circle_filled(icon_center, 18.0, theme.scheme.secondary_container);
                paint_icon(
                    ui.painter(),
                    icon_center,
                    icon,
                    20.0,
                    theme.scheme.on_secondary_container,
                );
                let text_x = rect.left() + DP48 + DP8;
                let text_width = (rect.right() - DP12 - text_x).max(24.0);
                elided(
                    ui.painter(),
                    egui::pos2(text_x, rect.top() + DP16),
                    title,
                    bold_font(15.0),
                    theme.scheme.on_surface,
                    text_width,
                );
                elided(
                    ui.painter(),
                    egui::pos2(text_x, rect.top() + DP16 + 21.0),
                    subtitle,
                    regular_font(11.0),
                    theme.scheme.on_surface_variant,
                    text_width,
                );
                if response.hovered() {
                    ui.painter().rect_stroke(
                        rect,
                        12,
                        egui::Stroke::new(1.0, theme.scheme.outline_variant),
                        egui::StrokeKind::Inside,
                    );
                }
                if response.clicked() {
                    intents.push(Intent::Open(route.clone()));
                }
            }
        });
        ui.add_space(DP8);
    }
    ui.add_space(DP4);
    intents
}
