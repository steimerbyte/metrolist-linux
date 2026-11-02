//! Play-history timeline grouped by the store's period buckets.

use eframe::egui::Ui;
use ml_core::now_millis;
use ml_store::{HistoryEntry, PeriodBounds};

use crate::art::ArtCache;
use crate::pure::group_starts;
use crate::services::Services;
use crate::state::{LibraryData, Load};
use crate::theme::{Theme, DP16, DP24, DP8};
use crate::widgets::{section_header, song_row, SongRowOptions};

use super::{error_block, skeleton_rows, Intent};

/// History tab / History route: grouped rows, newest group first.
pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    library: &Load<LibraryData>,
    current: Option<&ml_core::Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if library.initial_loading() {
        skeleton_rows(ui, theme, width);
        ui.add_space(DP24);
        return intents;
    }
    let Some(data) = library.data.as_ref() else {
        if let Some(error) = &library.error {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
        }
        ui.add_space(DP24);
        return intents;
    };
    intents.extend(entries_block(
        ui,
        theme,
        services,
        art,
        &data.history,
        current,
        width,
    ));
    intents
}

/// Rows only; reusable inside the Library tab.
pub fn entries_block(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    entries: &[HistoryEntry],
    current: Option<&ml_core::Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if entries.is_empty() {
        crate::widgets::empty_state(
            ui,
            theme,
            "history",
            "No play history yet",
            "Tracks are recorded here as they start playing.",
        );
        return intents;
    }

    // The store returns rows by insertion id; ordering by timestamp keeps the
    // timeline correct for imported/backfilled plays too.
    let mut entries: Vec<&HistoryEntry> = entries.iter().collect();
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    let bounds = PeriodBounds::from_millis(now_millis() as i64);
    let labels: Vec<String> = entries
        .iter()
        .map(|entry| bounds.bucket_of(entry.timestamp).label())
        .collect();
    let starts = group_starts(&labels);
    let songs: Vec<ml_core::Song> = entries.iter().map(|entry| entry.song.clone()).collect();

    for (group, start) in starts.iter().copied().enumerate() {
        let end = starts.get(group + 1).copied().unwrap_or(entries.len());
        section_header(ui, theme, &humanize_period(&labels[start]), width, false);
        for index in start..end {
            let song = &entries[index].song;
            let url = song.thumb(160);
            let texture = art.texture(ui.ctx(), services, &url);
            let active = current.is_some_and(|current| current.id == song.id);
            let response = song_row(
                ui,
                theme,
                song,
                texture.as_ref(),
                width,
                active,
                SongRowOptions {
                    download: services.download_badge(&song.id),
                    radio: true,
                },
            );
            if response.download.clicked() {
                intents.push(Intent::Download {
                    song: song.clone(),
                    action: response.download_action,
                });
            } else if response.radio.as_ref().is_some_and(|radio| radio.clicked()) {
                intents.push(Intent::StartRadio(song.clone()));
            } else if response.row.clicked() {
                intents.push(Intent::Play {
                    songs: songs.clone(),
                    index,
                    source: ml_core::QueueSource::List,
                });
            }
        }
        ui.add_space(DP8);
    }
    ui.add_space(DP16);
    intents
}

fn humanize_period(label: &str) -> String {
    match label {
        "today" => "Today".to_string(),
        "yesterday" => "Yesterday".to_string(),
        "this_week" => "This week".to_string(),
        "last_week" => "Last week".to_string(),
        other => other.to_string(),
    }
}
