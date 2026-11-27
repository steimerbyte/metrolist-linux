//! `browse FEmusic_explore` / `FEmusic_home` → [`HomeFeed`], narrowed by
//! [`ml_core::HomeView`]; chips and podcast shelves come from `FEmusic_home`.

use ml_core::{HomeFeed, HomeSection, HomeView};
use serde_json::Value;

use crate::parse::renderers::{carousel_items, grid_items, shelf_items};
use crate::parse::{continuation, runs_from, text_of};

fn section_list(value: &Value) -> Option<&Value> {
    value
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer")
        .or_else(|| value.pointer("/contents/sectionListRenderer"))
}

fn carousel_section(carousel: &Value) -> Option<HomeSection> {
    let title = text_of(&runs_from(
        carousel.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")?,
    ));
    let items = carousel_items(carousel);
    if items.is_empty() {
        return None;
    }
    Some(HomeSection {
        kind: super::section_kind(&title),
        title,
        id: header_browse_id(carousel).map(str::to_string),
        items,
        continuation: continuation(carousel.get("continuations")),
    })
}

fn grid_section(grid: &Value) -> Option<HomeSection> {
    let items = grid_items(grid);
    if items.is_empty() {
        return None;
    }
    let title = grid
        .pointer("/header/gridHeaderRenderer/title")
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .unwrap_or_default();
    Some(HomeSection {
        kind: super::section_kind(&title),
        title,
        id: None,
        items,
        continuation: continuation(grid.get("continuations")),
    })
}

fn shelf_section(shelf: &Value) -> Option<HomeSection> {
    let items = shelf_items(shelf);
    if items.is_empty() {
        return None;
    }
    let title = shelf
        .pointer("/title")
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .unwrap_or_default();
    Some(HomeSection {
        kind: super::section_kind(&title),
        title,
        id: None,
        items,
        continuation: continuation(shelf.get("continuations")),
    })
}

/// One `sectionListRenderer` content item → section.
pub fn section_from_content(content: &Value) -> Option<HomeSection> {
    if let Some(carousel) = content.get("musicCarouselShelfRenderer") {
        carousel_section(carousel)
    } else if let Some(grid) = content.get("gridRenderer") {
        grid_section(grid)
    } else if let Some(shelf) = content.get("musicShelfRenderer") {
        shelf_section(shelf)
    } else if let Some(content) = content.get("musicImmersiveCarouselShelfRenderer") {
        carousel_section(content)
    } else {
        None
    }
}

/// One Home header chip (e.g. "Podcasts") with the `params` it browses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chip {
    pub title: String,
    pub params: Option<String>,
}

/// Home header chips from `sectionListRenderer.header.chipCloudRenderer`.
pub fn chips(value: &Value) -> Vec<Chip> {
    let Some(chips) =
        section_list(value).and_then(|list| list.pointer("/header/chipCloudRenderer/chips"))
    else {
        return Vec::new();
    };
    chips
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|chip| {
            let chip = chip.get("chipCloudChipRenderer")?;
            let title = text_of(&runs_from(chip.get("text")?));
            (!title.is_empty()).then(|| Chip {
                title,
                params: chip
                    .pointer("/navigationEndpoint/browseEndpoint/params")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

/// Params of the first chip whose title contains `label`, case-insensitively.
pub fn chip_params(value: &Value, label: &str) -> Option<String> {
    let label = label.to_lowercase();
    chips(value)
        .into_iter()
        .find(|chip| chip.title.to_lowercase().contains(&label))
        .and_then(|chip| chip.params)
}

/// Parse the home browse response.
pub fn from_browse(value: &Value) -> HomeFeed {
    let Some(list) = section_list(value) else {
        return HomeFeed::default();
    };
    let sections = list
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(section_from_content)
        .collect();
    HomeFeed {
        sections,
        continuation: continuation(list.get("continuations")),
    }
}

/// Browse id a carousel header links to: the "Mehr" button first, then the title.
fn header_browse_id(carousel: &Value) -> Option<&str> {
    let header = carousel
        .get("header")?
        .get("musicCarouselShelfBasicHeaderRenderer")?;
    header
        .pointer("/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint/browseId")
        .or_else(|| header.pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId"))
        .and_then(Value::as_str)
}

/// Parse a feed page and keep the shelves `view` asks for: an empty list keeps
/// every shelf, a non-empty one keeps shelves whose browse id matches a prefix.
/// Shelves without an id (grids, `FEmusic_home` rows) always survive.
pub fn filter(mut feed: HomeFeed, view: &HomeView) -> HomeFeed {
    if !view.sections.is_empty() {
        feed.sections.retain(|section| {
            section
                .id
                .as_deref()
                .is_none_or(|id| view.sections.iter().any(|prefix| id.starts_with(prefix)))
        });
    }
    feed
}

/// Parse `browse {continuation}` for more home shelves.
pub fn from_continuation(value: &Value) -> HomeFeed {
    let Some(continuation) = value.pointer("/continuationContents/sectionListContinuation") else {
        return HomeFeed::default();
    };
    let sections = continuation
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(section_from_content)
        .collect();
    HomeFeed {
        sections,
        continuation: continuation_here(continuation),
    }
}

fn continuation_here(value: &Value) -> Option<String> {
    continuation(value.get("continuations"))
}
