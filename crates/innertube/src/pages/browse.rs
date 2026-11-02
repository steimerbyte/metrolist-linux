//! Generic `browse {browseId, params}` → [`BrowsePage`].
//!
//! Mood/genre category pages, the "YouTube browse" screen and library lists
//! answer with the same single-column section list as Home plus a header
//! title, so the section parser is shared and only the title differs.

use ml_core::{HomeSection, YtItem};
use serde_json::Value;

use crate::parse::renderers::shelf_items;
use crate::parse::{continuation, runs_from, text_of};

use super::home;

/// One generic browse response.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowsePage {
    pub title: Option<String>,
    pub sections: Vec<HomeSection>,
    pub continuation: Option<String>,
}

/// Section list of a single- or two-column browse response.
pub(crate) fn section_list(value: &Value) -> Option<&Value> {
    value
        .pointer(
            "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer",
        )
        .or_else(|| {
            value.pointer("/contents/twoColumnBrowseResultsRenderer/primaryContents/sectionListRenderer")
        })
        .or_else(|| value.pointer("/contents/sectionListRenderer"))
}

fn page_title(value: &Value) -> Option<String> {
    for path in [
        "/header/musicHeaderRenderer/title",
        "/header/musicImmersiveHeaderRenderer/title",
        "/header/musicVisualHeaderRenderer/title",
    ] {
        if let Some(title) = value.pointer(path) {
            let text = text_of(&runs_from(title));
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Playlist shelves inside a generic browse carry no header of their own.
fn playlist_shelf_section(shelf: &Value) -> Option<HomeSection> {
    let items = shelf_items(shelf);
    if items.is_empty() {
        return None;
    }
    Some(HomeSection {
        title: String::new(),
        items,
        continuation: continuation(shelf.get("continuations")),
        kind: ml_core::SectionKind::Unknown,
    })
}

fn section_from_content(content: &Value) -> Option<HomeSection> {
    if let Some(shelf) = content.get("musicPlaylistShelfRenderer") {
        return playlist_shelf_section(shelf);
    }
    home::section_from_content(content)
}

/// Parse the first `browse {browseId, params}` response.
pub fn from_browse(value: &Value) -> BrowsePage {
    let Some(list) = section_list(value) else {
        return BrowsePage::default();
    };
    let sections = list
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(section_from_content)
        .collect();
    BrowsePage {
        title: page_title(value),
        sections,
        continuation: continuation(list.get("continuations")),
    }
}

/// Parse `browse {continuation}`: the same section-list continuation Home uses.
pub fn from_continuation(value: &Value) -> BrowsePage {
    let feed = home::from_continuation(value);
    BrowsePage {
        title: None,
        sections: feed.sections,
        continuation: feed.continuation,
    }
}

/// Flatten the album cards of a flat grid page (`FEmusic_new_releases_albums`).
pub fn albums(page: BrowsePage) -> Vec<ml_core::Album> {
    page.sections
        .into_iter()
        .flat_map(|section| section.items)
        .filter_map(|item| match item {
            YtItem::Album(album) => Some(album),
            _ => None,
        })
        .collect()
}
