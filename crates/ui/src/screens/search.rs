//! Search: suggestions, history, filters and mixed InnerTube results.

use std::time::Instant;

use eframe::egui::{self, Id, Key, Ui, Vec2};
use ml_core::{QueueSource, Song, YtItem};

use crate::art::ArtCache;
use crate::motion;
use crate::routes::Route;
use crate::services::Services;
use crate::state::{FilterChoice, SearchState};
use crate::theme::{regular_font, Theme, DP16, TOUCH};
use crate::widgets::{chip, elided, empty_state, icon_button, paint_icon, section_header};

use super::{error_block, load_more, mixed_items, skeleton_rows, Intent};

/// Widest the search pill gets on a wide window; beyond this the field is a
/// mostly-empty bar with the mic stranded at the far edge.
const PILL_MAX_WIDTH: f32 = 720.0;

/// Pill width and its left indent inside the content column.
///
/// The pill keeps the 16 dp page gutters on a compact window and stops growing
/// at [`PILL_MAX_WIDTH`], staying centred in the column at every width.
fn pill_geometry(content_width: f32) -> (f32, f32) {
    let width = (content_width - 2.0 * DP16).clamp(0.0, PILL_MAX_WIDTH);
    (width, ((content_width - width) / 2.0).max(0.0))
}

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    state: &mut SearchState,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let field_id = Id::unique("metrolist_search_field");
    if state.focus {
        ui.memory_mut(|memory| memory.request_focus(field_id));
        state.focus = false;
    }

    let (pill_width, pill_indent) = pill_geometry(width);
    pill_frame(ui, theme, pill_width, pill_indent, |ui| {
        pill_contents(ui, theme, state, field_id, pill_width, &mut intents);
    });
    ui.add_space(8.0);

    // Suggestion popover, above the filter row.
    if state.suggestions_open && !state.query.trim().is_empty() && !state.suggestions.is_empty() {
        let suggestions = state.suggestions.clone();
        let previous_opacity = ui.opacity();
        let fade = motion::appear(
            ui.ctx(),
            Id::unique("search_suggestions_fade"),
            motion::SELECT,
        );
        ui.set_opacity(previous_opacity * fade);
        for suggestion in suggestions.iter().take(6) {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(width, 36.0), egui::Sense::click());
            let (hover_t, press_t) = motion::interaction(ui, &response);
            if hover_t > 0.0 || press_t > 0.0 {
                ui.painter().rect_filled(
                    rect,
                    8,
                    motion::state_layer_at(theme.bg(), hover_t, press_t),
                );
            }
            elided(
                ui.painter(),
                egui::Pos2::new(rect.left() + 12.0, rect.center().y - 9.0),
                suggestion,
                regular_font(15.0),
                theme.scheme.on_surface,
                rect.width() - 24.0,
            );
            motion::focus_ring(ui, rect, 8, &response);
            if response.clicked() {
                state.query = suggestion.clone();
                state.suggestions_open = false;
                intents.push(Intent::SubmitSearch);
            }
        }
        ui.set_opacity(previous_opacity);
        ui.add_space(8.0);
    }

    // Filter row.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for choice in FilterChoice::ALL {
            let selected = state.filter == choice;
            let response = chip(ui, theme, choice.label(), selected);
            if response.clicked() && !selected {
                state.filter = choice;
                if !state.submitted.is_empty() {
                    intents.push(Intent::SubmitSearch);
                }
            }
        }
    });
    ui.add_space(8.0);

    let needle = state.query.trim();
    // Typing has not run a query yet; only a submission may show "No results".
    if state.submitted.is_empty() {
        if needle.is_empty() && !state.history.is_empty() {
            section_header(ui, theme, "Recent searches", width, false);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for query in state.history.iter().take(10) {
                    let response = chip(ui, theme, query, false);
                    if response.clicked() {
                        state.query = query.clone();
                        intents.push(Intent::SubmitSearch);
                    }
                }
            });
            ui.add_space(16.0);
        }
        empty_state(
            ui,
            theme,
            "search",
            "Search YouTube Music",
            "Find songs, albums, artists and playlists.",
        );
        return intents;
    }

    if state.results.initial_loading() {
        skeleton_rows(ui, theme, width);
        ui.add_space(24.0);
        return intents;
    }

    if let Some(error) = &state.results.error {
        if state.results.data.is_none() {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
            ui.add_space(24.0);
            return intents;
        }
    }

    let Some(page) = state.results.data.as_ref() else {
        empty_state(
            ui,
            theme,
            "search_off",
            "No results",
            "Try a different title or artist.",
        );
        ui.add_space(24.0);
        return intents;
    };

    let songs_only: Vec<YtItem> = page.songs.iter().cloned().map(YtItem::Song).collect();
    let items: &[YtItem] = if page.items.is_empty() {
        &songs_only
    } else {
        &page.items
    };

    if items.is_empty() {
        empty_state(
            ui,
            theme,
            "search_off",
            "No results",
            "Try a different title or artist.",
        );
        ui.add_space(24.0);
        return intents;
    }

    let heading = if page.query.is_empty() {
        "Results".to_string()
    } else {
        format!("Results for “{}”", page.query)
    };
    section_header(ui, theme, &heading, width, false);
    let row_intents = mixed_items(
        ui,
        theme,
        services,
        art,
        items,
        current,
        width,
        QueueSource::List,
    );
    for intent in row_intents {
        // Upstream's search results play the tapped track's endless radio, and
        // the app starts that radio for a single-song List queue.
        if let Intent::Play { songs, index, .. } = intent {
            if let Some(song) = songs.get(index).cloned() {
                intents.push(Intent::Play {
                    songs: vec![song],
                    index: 0,
                    source: QueueSource::List,
                });
            }
        } else {
            intents.push(intent);
        }
    }

    if page.continuation.is_some() && load_more(ui, theme, state.results.more_loading) {
        intents.push(Intent::LoadMore);
    }
    ui.add_space(24.0);
    intents
}

/// Draw the pill at `indent` and `pill_width` inside the content column and
/// return its painted rect. The frame is pinned to the pill width so it cannot
/// stretch back to the column, which is what put the mic at the far edge.
fn pill_frame(
    ui: &mut Ui,
    theme: &Theme,
    pill_width: f32,
    indent: f32,
    add_contents: impl FnOnce(&mut Ui),
) -> egui::Rect {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(indent);
        let inner_width = (pill_width - 2.0 * DP16).max(0.0);
        egui::Frame::new()
            .fill(theme.scheme.surface_container_high)
            .corner_radius(24)
            .inner_margin(egui::Margin::symmetric(16, 10))
            .show(ui, |ui| {
                let start = ui.cursor().min;
                add_contents(ui);
                // Frames shrink to their content; extend the content rect to
                // the pinned width so the background spans the whole pill.
                ui.expand_to_include_rect(egui::Rect::from_min_size(
                    start,
                    Vec2::new(inner_width, ui.min_rect().height()),
                ));
            })
            .response
            .rect
    })
    .inner
}

/// Icon, field and mic inside the pill. Both icons sit in 40 dp slots and the
/// field takes what the slots, gaps and 16 dp insets leave, so the text column
/// is centred between equal side margins instead of running out to the edges.
fn pill_contents(
    ui: &mut Ui,
    theme: &Theme,
    state: &mut SearchState,
    field_id: Id,
    pill_width: f32,
    intents: &mut Vec<Intent>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(TOUCH), egui::Sense::hover());
        paint_icon(
            ui.painter(),
            icon_rect.center(),
            "search",
            22.0,
            theme.scheme.on_surface_variant,
        );
        let field_width = (pill_width - 2.0 * DP16 - 2.0 * 10.0 - 2.0 * TOUCH).max(40.0);
        let edit = egui::TextEdit::singleline(&mut state.query)
            .id(field_id)
            .hint_text("Songs, albums, artists")
            .frame(egui::Frame::NONE)
            .font(regular_font(16.0))
            .desired_width(field_width);
        let response = ui.add(edit);
        if response.changed() {
            state.last_edit = Some(Instant::now());
            state.suggestions_open = true;
        }
        // A singleline TextEdit surrenders focus while handling Enter,
        // so `has_focus` is already false here; `lost_focus` is not.
        if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            state.suggestions_open = false;
            intents.push(Intent::SubmitSearch);
        }
        if ui.input(|input| input.key_pressed(Key::Escape)) && response.has_focus() {
            state.suggestions_open = false;
            response.surrender_focus();
        }
        if icon_button(ui, theme, "mic", "Recognize song", 22.0).clicked() {
            intents.push(Intent::Open(Route::Recognize));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pill_is_gutter_wide_on_compact_and_capped_wide() {
        // Compact column (405 window - 32 gutters): full column minus the 16 dp
        // page gutters, still centred.
        assert_eq!(pill_geometry(373.0), (341.0, 16.0));
        // Wide column (960): capped, with equal margins around it.
        assert_eq!(pill_geometry(960.0), (PILL_MAX_WIDTH, 120.0));
        // Exactly at the cap there is no extra margin yet.
        assert_eq!(pill_geometry(752.0), (720.0, 16.0));
    }

    #[test]
    fn pill_geometry_never_goes_negative() {
        // A column narrower than the gutters collapses the pill, not the math.
        assert_eq!(pill_geometry(0.0), (0.0, 0.0));
        assert_eq!(pill_geometry(-40.0), (0.0, 0.0));
        assert_eq!(pill_geometry(20.0), (0.0, 10.0));
    }

    #[test]
    fn pill_is_centred_in_the_content_column() {
        for width in [320.0, 405.0, 560.0, 752.0, 960.0, 1100.0, 2400.0] {
            let (pill, indent) = pill_geometry(width);
            let left = indent;
            let right = width - indent - pill;
            assert!(
                (left - right).abs() <= 1.0,
                "width {width}: pill {pill} off-centre by {}",
                left - right
            );
        }
    }

    /// Renders the pill inside the shell's real column for both window classes
    /// (405x900 compact -> 373 dp column; 1100x760 wide -> 960 dp column) and
    /// measures the rect it paints.
    #[test]
    fn painted_pill_is_centred_at_both_widths() {
        for (window, column, height, expected_indent) in [
            (405.0f32, 373.0f32, 900.0f32, 16.0f32),
            (1100.0, 960.0, 760.0, 120.0),
        ] {
            let ctx = egui::Context::default();
            let theme = Theme::from_settings(&Default::default());
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(window, height),
                )),
                ..Default::default()
            };
            let mut rect = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(input, |ui| {
                // The column is centred in the window by the shell; the pill is
                // centred in the column, so measure against the column origin.
                let (pill_width, indent) = pill_geometry(column);
                rect = pill_frame(ui, &theme, pill_width, indent, |ui| {
                    ui.add_sized([200.0, 20.0], egui::Label::new("field"));
                });
            });
            output.textures_delta.clear();
            let (pill_width, _) = pill_geometry(column);
            assert!(
                (rect.width() - pill_width).abs() <= 1.0,
                "window {window}: painted pill {} != {pill_width}",
                rect.width()
            );
            let left = rect.left();
            let right = column - rect.right();
            assert!(
                (left - right).abs() <= 1.0,
                "window {window}: pill margins {left} / {right} are uneven"
            );
            assert!(
                (left - expected_indent).abs() <= 1.0,
                "window {window}: left margin {left} != {expected_indent}"
            );
        }
    }
}
