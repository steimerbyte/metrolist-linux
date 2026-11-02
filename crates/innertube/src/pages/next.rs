//! `next` (radio / album / playlist queue) parsing.

use ml_core::{QueuePage, Song};
use serde_json::Value;

use crate::parse::array_at;
use crate::parse::continuation;
use crate::parse::renderers::panel_song;

fn panel(value: &Value) -> Option<&Value> {
    value
        .pointer("/continuationContents/playlistPanelContinuation")
        .or_else(|| {
            value.pointer(
                "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer",
            )
        })
}

/// Title from the watch-next queue header subtitle.
pub fn title(value: &Value) -> Option<String> {
    value
        .pointer(
            "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/header/musicQueueHeaderRenderer/subtitle/runs/0/text",
        )
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Channel owner from the watch-next `videoSecondaryInfoRenderer` block, used
/// to fill podcast episode credits when the queue itself carries none.
pub fn owner(value: &Value) -> Option<ml_core::Artist> {
    let owner = array_at(
        value,
        "/contents/twoColumnWatchNextResults/results/results/contents",
    )
    .iter()
    .find_map(|content| content.pointer("/videoSecondaryInfoRenderer/owner/videoOwnerRenderer"))?;
    let name = owner
        .pointer("/title/runs/0/text")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())?;
    let id = owner
        .pointer("/navigationEndpoint/browseEndpoint/browseId")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(ml_core::Artist::new(name, id))
}

fn songs_from_panel(panel: &Value) -> (Vec<Song>, Option<usize>) {
    let mut songs = Vec::new();
    let mut current_index = None;
    for content in panel
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let Some(renderer) = content.get("playlistPanelVideoRenderer") else {
            continue;
        };
        if let Some((song, selected)) = panel_song(renderer) {
            if selected && current_index.is_none() {
                current_index = Some(songs.len());
            }
            songs.push(song);
        }
    }
    (songs, current_index)
}

/// Parse the first `next` response.
pub fn from_response(value: &Value) -> QueuePage {
    let Some(panel) = panel(value) else {
        return QueuePage::default();
    };
    let (songs, current_index) = songs_from_panel(panel);
    QueuePage {
        title: title(value),
        songs,
        current_index,
        continuation: continuation(panel.get("continuations")),
    }
}

/// Parse `next {continuation}` for more radio/queue rows.
pub fn from_continuation(value: &Value) -> QueuePage {
    let Some(panel) = panel(value) else {
        return QueuePage::default();
    };
    let (songs, current_index) = songs_from_panel(panel);
    QueuePage {
        title: None,
        songs,
        current_index,
        continuation: continuation(panel.get("continuations")),
    }
}

/// Append one continuation page in place: songs whose id is not queued yet are
/// appended in server order, existing order and `current_index` are kept, and
/// `page.continuation` becomes the next token (`None` = queue exhausted).
/// Returns the number of appended songs.
pub fn append(page: &mut QueuePage, next: QueuePage) -> usize {
    let before = page.songs.len();
    for song in next.songs {
        if !page.songs.iter().any(|queued| queued.id == song.id) {
            page.songs.push(song);
        }
    }
    page.continuation = next.continuation;
    page.songs.len() - before
}
