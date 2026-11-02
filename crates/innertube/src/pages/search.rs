//! `search` / `music/get_search_suggestions` parsing.

use std::collections::HashSet;

use ml_core::{SearchPage, YtItem};
use serde_json::Value;

use crate::parse::renderers::{card_item, grid_items, responsive_item, shelf_items, RowOptions};
use crate::parse::{array_at, continuation, runs_from, shelf_continuation, text_of};

fn section_list(value: &Value) -> Option<&Value> {
    value.pointer(
        "/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer",
    )
}

fn push_items(items: &mut Vec<YtItem>, new_items: Vec<YtItem>) {
    for item in new_items {
        let id = item.id().to_string();
        if !id.is_empty() && items.iter().any(|existing| existing.id() == id) {
            continue;
        }
        items.push(item);
    }
}

fn items_from_content(content: &Value, items: &mut Vec<YtItem>) {
    if let Some(shelf) = content.get("musicShelfRenderer") {
        push_items(items, shelf_items(shelf));
    }
    if let Some(section) = content.get("itemSectionRenderer") {
        for content in array_at(section, "/contents") {
            if let Some(renderer) = content.get("musicResponsiveListItemRenderer") {
                if let Some(item) = responsive_item(renderer, &RowOptions::default()) {
                    push_items(items, vec![item]);
                }
            }
            if let Some(renderer) = content.get("musicCardShelfRenderer") {
                if let Some(item) = card_item(renderer) {
                    push_items(items, vec![item]);
                }
            }
            if let Some(shelf) = content.get("musicShelfRenderer") {
                push_items(items, shelf_items(shelf));
            }
            if let Some(grid) = content.get("gridRenderer") {
                push_items(items, grid_items(grid));
            }
        }
    }
    if let Some(card) = content.get("musicCardShelfRenderer") {
        if let Some(item) = card_item(card) {
            push_items(items, vec![item]);
        }
    }
    if let Some(grid) = content.get("gridRenderer") {
        push_items(items, grid_items(grid));
    }
}

fn first_shelf_continuation(content: &Value) -> Option<String> {
    content
        .get("musicShelfRenderer")
        .and_then(|shelf| continuation(shelf.get("continuations")))
        .or_else(|| {
            array_at(content, "/itemSectionRenderer/contents")
                .iter()
                .find_map(|content| {
                    content
                        .get("musicShelfRenderer")
                        .and_then(|shelf| continuation(shelf.get("continuations")))
                })
        })
        .or_else(|| {
            content
                .get("musicShelfRenderer")
                .and_then(|shelf| shelf_continuation(shelf.get("contents")))
        })
}

/// Parse a first-page search response.
pub fn from_response(value: &Value, query: &str) -> SearchPage {
    let mut items = Vec::new();
    let mut continuation_token = None;
    if let Some(list) = section_list(value) {
        for content in array_at(list, "/contents") {
            items_from_content(content, &mut items);
            if continuation_token.is_none() {
                continuation_token = first_shelf_continuation(content);
            }
        }
        if continuation_token.is_none() {
            continuation_token = continuation(list.get("continuations"));
        }
    }
    let songs = items
        .iter()
        .filter_map(|item| match item {
            YtItem::Song(song) => Some(song.clone()),
            _ => None,
        })
        .collect();
    SearchPage {
        query: query.to_string(),
        items,
        songs,
        continuation: continuation_token,
    }
}

fn continuation_items(value: &Value) -> &[Value] {
    value
        .pointer("/continuationContents/musicShelfContinuation/contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .or_else(|| {
            value
                .pointer(
                    "/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems",
                )
                .and_then(Value::as_array)
                .map(Vec::as_slice)
        })
        .unwrap_or(&[])
}

/// Parse `search {continuation}`.
pub fn from_continuation(value: &Value) -> SearchPage {
    let mut items = Vec::new();
    for content in continuation_items(value) {
        if let Some(renderer) = content.get("musicResponsiveListItemRenderer") {
            if let Some(item) = responsive_item(renderer, &RowOptions::default()) {
                push_items(&mut items, vec![item]);
            }
        }
    }
    let token =
        continuation(value.pointer("/continuationContents/musicShelfContinuation/continuations"));
    let songs = items
        .iter()
        .filter_map(|item| match item {
            YtItem::Song(song) => Some(song.clone()),
            _ => None,
        })
        .collect();
    SearchPage {
        query: String::new(),
        items,
        songs,
        continuation: token,
    }
}

/// `music/get_search_suggestions` → plain query strings.
pub fn suggestions(value: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut suggestions = Vec::new();
    for section in array_at(value, "/contents") {
        for content in array_at(section, "/searchSuggestionsSectionRenderer/contents") {
            let Some(suggestion) = content.get("searchSuggestionRenderer") else {
                continue;
            };
            let text = text_of(&runs_from(&suggestion["suggestion"]));
            if !text.is_empty() && seen.insert(text.clone()) {
                suggestions.push(text);
            }
        }
    }
    suggestions
}
