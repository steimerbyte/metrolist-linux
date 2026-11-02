//! Artist (`UC*`) page: immersive header + sections + see-all endpoints.

use ml_core::{ArtistItem, ArtistPage, HomeFeed, HomeSection, Song};
use serde_json::Value;

use crate::parse::renderers::{
    carousel_items, multi_row_episode, responsive_item, two_row_item, RowOptions,
};
use crate::parse::{array_at, continuation, runs_from, text_of};

/// Extra artist facts the frozen `ml_core::ArtistPage` cannot carry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistExtras {
    pub monthly_listeners: Option<String>,
    pub is_subscribed: bool,
    /// "See all" browse endpoints from carousel headers (`browseId` + `params`).
    pub see_all: Vec<SeeAll>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeeAll {
    pub title: String,
    pub browse_id: String,
    pub params: Option<String>,
}

/// One section-list content item: title, items, see-all endpoint, continuation.
pub type SectionChunk = (String, Vec<ml_core::YtItem>, Option<SeeAll>, Option<String>);

/// Artist page parsed together with its extras.
#[derive(Debug, Clone, Default)]
pub struct ArtistDetails {
    pub page: ArtistPage,
    pub extras: ArtistExtras,
}

fn section_list(value: &Value) -> Option<&Value> {
    value.pointer(
        "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer",
    )
}

fn header(value: &Value) -> Option<&Value> {
    value
        .pointer("/header/musicImmersiveHeaderRenderer")
        .or_else(|| value.pointer("/header/musicVisualHeaderRenderer"))
        .or_else(|| value.pointer("/header/musicHeaderRenderer"))
}

fn header_title(header: &Value) -> String {
    header
        .pointer("/title")
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .unwrap_or_default()
}

fn header_thumbnail(header: &Value) -> Option<String> {
    crate::parse::thumbnail_url(
        header
            .get("thumbnail")
            .or_else(|| header.get("foregroundThumbnail")),
    )
}

fn subscriber_text(header: &Value) -> Option<String> {
    for pointer in [
        "/subscriptionButton2/subscribeButtonRenderer/subscriberCountWithSubscribeText",
        "/subscriptionButton/subscribeButtonRenderer/longSubscriberCountText",
        "/subscriptionButton/subscribeButtonRenderer/shortSubscriberCountText",
    ] {
        let text = header
            .pointer(pointer)
            .map(runs_from)
            .map(|runs| text_of(&runs));
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            return Some(text);
        }
    }
    None
}

fn channel_id_from_header(header: &Value, fallback: &str) -> String {
    header
        .pointer("/subscriptionButton/subscribeButtonRenderer/channelId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn description(value: &Value, header: Option<&Value>) -> Option<String> {
    let section_contents = section_list(value)
        .map(|list| array_at(list, "/contents"))
        .unwrap_or(&[]);
    let from_sections = section_contents
        .iter()
        .find_map(|content| content.get("musicDescriptionShelfRenderer"))
        .and_then(|shelf| shelf.get("description"))
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .filter(|text| !text.is_empty());
    from_sections.or_else(|| {
        header
            .and_then(|header| header.get("description"))
            .map(runs_from)
            .map(|runs| text_of(&runs))
            .filter(|text| !text.is_empty())
    })
}

/// `ArtistPage.fromSectionListRendererContent`: emit items + title + see-all.
pub fn section_from_content(content: &Value) -> Option<SectionChunk> {
    if let Some(shelf) = content.get("musicShelfRenderer") {
        let title = shelf
            .pointer("/title")
            .map(runs_from)
            .map(|runs| text_of(&runs))
            .unwrap_or_default();
        let items = array_at(shelf, "/contents")
            .iter()
            .filter_map(|content| {
                content
                    .get("musicResponsiveListItemRenderer")
                    .and_then(|renderer| responsive_item(renderer, &RowOptions::default()))
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            return None;
        }
        let see_all = shelf
            .pointer("/title/runs/0/navigationEndpoint/browseEndpoint")
            .and_then(|browse| {
                Some(SeeAll {
                    title: title.clone(),
                    browse_id: browse.get("browseId")?.as_str()?.to_string(),
                    params: browse
                        .get("params")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            });
        return Some((
            title,
            items,
            see_all,
            continuation(shelf.get("continuations")),
        ));
    }
    let carousel = content
        .get("musicCarouselShelfRenderer")
        .or_else(|| content.get("musicImmersiveCarouselShelfRenderer"))?;
    let title = carousel
        .pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .unwrap_or_default();
    let items = carousel_items(carousel);
    if items.is_empty() {
        return None;
    }
    let see_all = carousel
        .pointer("/header/musicCarouselShelfBasicHeaderRenderer/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint")
        .and_then(|browse| {
            Some(SeeAll {
                title: title.clone(),
                browse_id: browse.get("browseId")?.as_str()?.to_string(),
                params: browse.get("params").and_then(Value::as_str).map(str::to_string),
            })
        });
    Some((
        title,
        items,
        see_all,
        continuation(carousel.get("continuations")),
    ))
}

/// Parse the artist browse response.
pub fn from_browse(value: &Value, channel_id: &str) -> ArtistDetails {
    let header = header(value);
    let name = header.map(header_title).unwrap_or_default();
    let artist = ArtistItem {
        id: channel_id.to_string(),
        name,
        thumbnail_url: header.and_then(header_thumbnail),
        channel_id: header.map(|header| channel_id_from_header(header, channel_id)),
        subscribers: header.and_then(subscriber_text),
    };
    let mut top_songs: Vec<Song> = Vec::new();
    let mut sections = Vec::new();
    let mut extras = ArtistExtras {
        monthly_listeners: header
            .and_then(|header| header.pointer("/monthlyListenerCount"))
            .map(runs_from)
            .map(|runs| text_of(&runs))
            .filter(|text| !text.is_empty()),
        is_subscribed: header
            .and_then(|header| {
                header.pointer("/subscriptionButton/subscribeButtonRenderer/subscribed")
            })
            .and_then(Value::as_bool)
            .or_else(|| {
                header
                    .and_then(|header| {
                        header.pointer("/subscriptionButton/subscribeButtonRenderer/subscribed")
                    })
                    .and_then(Value::as_bool)
            })
            .unwrap_or(false),
        see_all: Vec::new(),
    };
    let mut page_continuation = None;
    if let Some(list) = section_list(value) {
        for content in array_at(list, "/contents") {
            let Some((title, items, see_all, token)) = section_from_content(content) else {
                continue;
            };
            if let Some(see_all) = see_all {
                extras.see_all.push(see_all);
            }
            if page_continuation.is_none() {
                page_continuation = token.clone();
            }
            if top_songs.is_empty()
                && content.get("musicShelfRenderer").is_some()
                && items
                    .iter()
                    .all(|item| matches!(item, ml_core::YtItem::Song(_)))
            {
                top_songs = items
                    .into_iter()
                    .filter_map(|item| match item {
                        ml_core::YtItem::Song(song) => Some(song),
                        _ => None,
                    })
                    .collect();
            } else {
                sections.push(HomeSection {
                    kind: super::section_kind(&title),
                    title,
                    items,
                    continuation: token,
                });
            }
        }
        if page_continuation.is_none() {
            page_continuation = continuation(list.get("continuations"));
        }
    }
    ArtistDetails {
        page: ArtistPage {
            artist,
            top_songs,
            sections,
            description: description(value, header),
            continuation: page_continuation,
        },
        extras,
    }
}

/// Parse `browse {continuation}` for more artist shelves/items.
pub fn from_continuation(value: &Value) -> HomeFeed {
    // See-all continuations arrive as a shelf, not a section list.
    if let Some(shelf) = value
        .pointer("/continuationContents/musicPlaylistShelfContinuation")
        .or_else(|| value.pointer("/continuationContents/musicShelfContinuation"))
    {
        let items = array_at(shelf, "/contents")
            .iter()
            .filter_map(|content| {
                content
                    .get("musicResponsiveListItemRenderer")
                    .and_then(|renderer| responsive_item(renderer, &RowOptions::default()))
            })
            .collect::<Vec<_>>();
        return HomeFeed {
            sections: (!items.is_empty())
                .then(|| HomeSection {
                    kind: ml_core::SectionKind::Unknown,
                    title: String::new(),
                    items,
                    continuation: continuation(shelf.get("continuations")),
                })
                .into_iter()
                .collect(),
            continuation: continuation(shelf.get("continuations")),
        };
    }
    let Some(continuation) = value.pointer("/continuationContents/sectionListContinuation") else {
        return HomeFeed::default();
    };
    let sections: Vec<HomeSection> = array_at(continuation, "/contents")
        .iter()
        .filter_map(super::home::section_from_content)
        .collect();
    HomeFeed {
        sections,
        continuation: continuation.get("continuations").and_then(|value| {
            value.as_array().and_then(|items| {
                items.iter().find_map(|item| {
                    item.pointer("/nextContinuationData/continuation")
                        .or_else(|| item.pointer("/nextRadioContinuationData/continuation"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
            })
        }),
    }
}

/// Items for a "see all" browse (`browseId` + `params`).
pub fn items_from_browse(value: &Value) -> HomeFeed {
    let Some(list) = section_list(value) else {
        return HomeFeed::default();
    };
    let mut items = Vec::new();
    let mut shelf_continuation = None;
    for content in array_at(list, "/contents") {
        if let Some(shelf) = content.get("musicShelfRenderer") {
            items.extend(
                array_at(shelf, "/contents")
                    .iter()
                    .filter_map(|content| {
                        content
                            .get("musicResponsiveListItemRenderer")
                            .and_then(|renderer| responsive_item(renderer, &RowOptions::default()))
                    })
                    .collect::<Vec<_>>(),
            );
            if shelf_continuation.is_none() {
                shelf_continuation = continuation(shelf.get("continuations"))
                    .or_else(|| crate::parse::shelf_continuation(shelf.get("contents")));
            }
        } else if let Some(shelf) = content.get("musicPlaylistShelfRenderer") {
            // "See all" (Top songs/Videos) returns a playlist shelf, not a list
            // of sections.
            items.extend(
                array_at(shelf, "/contents")
                    .iter()
                    .filter_map(|content| {
                        content
                            .get("musicResponsiveListItemRenderer")
                            .and_then(|renderer| responsive_item(renderer, &RowOptions::default()))
                    })
                    .collect::<Vec<_>>(),
            );
            if shelf_continuation.is_none() {
                shelf_continuation = continuation(shelf.get("continuations"))
                    .or_else(|| crate::parse::shelf_continuation(shelf.get("contents")));
            }
        } else if let Some(carousel) = content
            .get("musicCarouselShelfRenderer")
            .or_else(|| content.get("musicImmersiveCarouselShelfRenderer"))
        {
            items.extend(carousel_items(carousel));
        } else if let Some(content) = content.get("musicCardShelfRenderer") {
            if let Some(item) = crate::parse::renderers::card_item(content) {
                items.push(item);
            }
        } else if let Some(multi) = content.get("musicMultiRowListItemRenderer") {
            if let Some(episode) = multi_row_episode(multi) {
                items.push(ml_core::YtItem::Episode(episode));
            }
        } else if let Some(two_row) = content.get("musicTwoRowItemRenderer") {
            if let Some(item) = two_row_item(two_row) {
                items.push(item);
            }
        }
    }
    let token = continuation(list.get("continuations")).or(shelf_continuation);
    let title = value
        .pointer("/header/musicHeaderRenderer/title")
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .unwrap_or_default();
    HomeFeed {
        sections: (!items.is_empty())
            .then(|| HomeSection {
                kind: ml_core::SectionKind::Unknown,
                title,
                items,
                continuation: token.clone(),
            })
            .into_iter()
            .collect(),
        continuation: token,
    }
}
