//! Defensive JSON primitives shared by every page parser.
//!
//! Renderers vary by locale and client, so parsing walks `serde_json::Value`
//! instead of pinning exhaustive structs. Missing fields become `None`; unknown
//! renderers are skipped ([`MlError::Parse`] is only for the whole-envelope
//! shape).

pub mod renderers;
pub mod url;

use serde_json::Value;

use ml_core::Artist;

/// One text run plus the endpoint carried by that run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub browse_id: Option<String>,
    pub page_type: Option<String>,
    pub watch_video_id: Option<String>,
    pub watch_playlist_id: Option<String>,
    pub watch_set_video_id: Option<String>,
    pub music_video_type: Option<String>,
}

impl Run {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn linked(text: impl Into<String>, browse_id: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            browse_id: Some(browse_id.into()),
            ..Self::default()
        }
    }

    pub fn from_value(value: &Value) -> Self {
        let text = value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let endpoint = value.get("navigationEndpoint");
        let browse = endpoint.and_then(|e| e.get("browseEndpoint"));
        let watch = endpoint.and_then(|e| e.get("watchEndpoint"));
        let config = browse
            .and_then(|b| b.get("browseEndpointContextSupportedConfigs"))
            .and_then(|c| c.get("browseEndpointContextMusicConfig"));
        Self {
            text,
            browse_id: browse
                .and_then(|b| b.get("browseId"))
                .and_then(Value::as_str)
                .map(str::to_string),
            page_type: config
                .and_then(|c| c.get("pageType"))
                .and_then(Value::as_str)
                .map(str::to_string),
            watch_video_id: watch
                .and_then(|w| w.get("videoId"))
                .and_then(Value::as_str)
                .map(str::to_string),
            watch_playlist_id: watch
                .and_then(|w| w.get("playlistId"))
                .and_then(Value::as_str)
                .map(str::to_string),
            watch_set_video_id: watch
                .and_then(|w| w.get("playlistSetVideoId"))
                .and_then(Value::as_str)
                .map(str::to_string),
            music_video_type: watch
                .and_then(|w| w.get("watchEndpointMusicSupportedConfigs"))
                .and_then(|c| c.get("watchEndpointMusicConfig"))
                .and_then(|c| c.get("musicVideoType"))
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    }
}

pub const PAGE_ALBUM: &str = "MUSIC_PAGE_TYPE_ALBUM";
pub const PAGE_AUDIOBOOK: &str = "MUSIC_PAGE_TYPE_AUDIOBOOK";
pub const PAGE_ARTIST: &str = "MUSIC_PAGE_TYPE_ARTIST";
pub const PAGE_LIBRARY_ARTIST: &str = "MUSIC_PAGE_TYPE_LIBRARY_ARTIST";
pub const PAGE_USER_CHANNEL: &str = "MUSIC_PAGE_TYPE_USER_CHANNEL";
pub const PAGE_PLAYLIST: &str = "MUSIC_PAGE_TYPE_PLAYLIST";
pub const PAGE_PODCAST: &str = "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE";
pub const PAGE_AUDIO_TRACK: &str = "MUSIC_PAGE_TYPE_NON_MUSIC_AUDIO_TRACK_PAGE";

/// `{ "runs": [...] }`, `{ "simpleText": "…" }`, or a bare string.
pub fn runs_from(value: &Value) -> Vec<Run> {
    if let Some(runs) = value.get("runs").and_then(Value::as_array) {
        return runs.iter().map(Run::from_value).collect();
    }
    if let Some(text) = value.get("simpleText").and_then(Value::as_str) {
        return vec![Run::new(text)];
    }
    if let Some(text) = value.as_str() {
        return vec![Run::new(text)];
    }
    Vec::new()
}

/// Runs at `value.path`; empty when absent.
pub fn runs_at(value: &Value, path: &str) -> Vec<Run> {
    value.pointer(path).map(runs_from).unwrap_or_default()
}

/// Concatenate run text, matching `runs.joinToString("")`.
pub fn text_of(runs: &[Run]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

/// Split on bullet separators, keeping every run of every group.
pub fn split_by_separator(runs: &[Run]) -> Vec<Vec<Run>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    for run in runs {
        if run.text.trim() == "•" {
            groups.push(std::mem::take(&mut current));
        } else {
            current.push(run.clone());
        }
    }
    groups.push(current);
    groups
}

/// Even-indexed runs; the album/performer column interleaves names and `", "`.
pub fn odd_elements(runs: &[Run]) -> Vec<Run> {
    runs.iter()
        .enumerate()
        .filter(|(index, _)| index % 2 == 0)
        .map(|(_, run)| run.clone())
        .collect()
}

fn is_artist_page(page_type: Option<&str>, browse_id: &str) -> bool {
    matches!(
        page_type,
        Some(PAGE_ARTIST | PAGE_LIBRARY_ARTIST | PAGE_USER_CHANNEL)
    ) || (page_type.is_none() && browse_id.starts_with("UC"))
}

/// Artists only from linked runs with a trusted page type; never from
/// durations, play counts or localized labels (`PageHelperTest`).
pub fn extract_artists(runs: &[Run]) -> Vec<Artist> {
    runs.iter()
        .filter(|run| !run.text.trim().is_empty())
        .filter_map(|run| {
            let browse_id = run.browse_id.as_deref()?;
            is_artist_page(run.page_type.as_deref(), browse_id)
                .then(|| Artist::new(run.text.trim(), Some(browse_id.to_string())))
        })
        .collect()
}

pub fn is_podcast_run(run: &Run) -> bool {
    run.page_type.as_deref() == Some(PAGE_PODCAST) && run.browse_id.is_some()
}

/// Podcast show credit from a byline: linked artist, then podcast-show link,
/// then the first unlinked section.
pub fn extract_podcast_byline(runs: &[Run]) -> Vec<Artist> {
    let linked = extract_artists(runs);
    if !linked.is_empty() {
        return linked;
    }
    if let Some(podcast) = runs
        .iter()
        .find(|run| !run.text.trim().is_empty() && is_podcast_run(run))
    {
        return vec![Artist::new(podcast.text.trim(), None)];
    }
    split_by_separator(runs)
        .first()
        .and_then(|group| group.first())
        .filter(|run| !run.text.trim().is_empty() && run.browse_id.is_none())
        .map(|run| vec![Artist::new(run.text.trim(), None)])
        .unwrap_or_default()
}

/// `PlaylistPage.ownerFromByline`.
pub fn owner_from_byline(runs: &[Run]) -> Option<Artist> {
    extract_artists(runs).into_iter().next().or_else(|| {
        split_by_separator(runs)
            .first()
            .and_then(|group| group.first())
            .filter(|run| run.browse_id.is_none() && !run.text.trim().is_empty())
            .map(|run| Artist::new(run.text.trim(), None))
    })
}

/// Legacy playlist subtitle: `Playlist • Owner • 14 songs` → second section.
pub fn owner_from_detail_subtitle(runs: &[Run]) -> Option<Artist> {
    let groups = split_by_separator(runs);
    owner_from_byline(groups.get(1)?)
}

/// `String.parseTime`: `m:ss`, `m.ss`, `m,ss`, `h:mm:ss`.
pub fn parse_time(text: &str) -> Option<u32> {
    let parts: Vec<&str> = text.trim().split([':', '.', ',']).collect();
    let numbers: Vec<u32> = parts
        .iter()
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    match numbers.as_slice() {
        [minutes, seconds] => minutes.checked_mul(60)?.checked_add(*seconds),
        [hours, minutes, seconds] => hours
            .checked_mul(3600)?
            .checked_add(minutes.checked_mul(60)?)?
            .checked_add(*seconds),
        _ => None,
    }
}

/// Duration text only: 1–2 digit first part, two digits after every separator.
fn is_duration_text(text: &str) -> bool {
    let text = text.trim();
    let parts: Vec<&str> = text.split([':', '.', ',']).collect();
    if !(parts.len() == 2 || parts.len() == 3) {
        return false;
    }
    let first_ok =
        (1..=2).contains(&parts[0].len()) && parts[0].chars().all(|c| c.is_ascii_digit());
    first_ok
        && parts[1..]
            .iter()
            .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_digit()))
}

/// First run that is a duration, never a play count or label.
pub fn extract_duration(runs: &[Run]) -> Option<u32> {
    runs.iter()
        .find(|run| is_duration_text(&run.text))
        .and_then(|run| parse_time(&run.text))
}

/// Both library feedback tokens from `toggleMenuServiceItemRenderer` items.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryTokens {
    pub add: Option<String>,
    pub remove: Option<String>,
}

pub fn library_tokens(menu: Option<&Value>) -> LibraryTokens {
    let mut tokens = LibraryTokens::default();
    let Some(items) = menu
        .and_then(|m| m.pointer("/menuRenderer/items"))
        .and_then(Value::as_array)
    else {
        return tokens;
    };
    for item in items {
        let Some(toggle) = item.get("toggleMenuServiceItemRenderer") else {
            continue;
        };
        let icon = toggle
            .pointer("/defaultIcon/iconType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        // KEEP/KEEP_OFF are "Pin to Listen Again", not library membership.
        if matches!(icon, "KEEP" | "KEEP_OFF") {
            continue;
        }
        let is_add = matches!(icon, "LIBRARY_ADD" | "BOOKMARK_BORDER");
        let is_saved = matches!(
            icon,
            "LIBRARY_SAVED" | "BOOKMARK" | "LIBRARY_REMOVE" | "LIBRARY_ADD"
        ) || icon.starts_with("LIBRARY_");
        if !is_add && !is_saved {
            continue;
        }
        let default = toggle
            .pointer("/defaultServiceEndpoint/feedbackEndpoint/feedbackToken")
            .and_then(Value::as_str)
            .map(str::to_string);
        let toggled = toggle
            .pointer("/toggledServiceEndpoint/feedbackEndpoint/feedbackToken")
            .and_then(Value::as_str)
            .map(str::to_string);
        if is_add {
            tokens.add = tokens.add.or(default);
            tokens.remove = tokens.remove.or(toggled);
        } else {
            tokens.remove = tokens.remove.or(default);
            tokens.add = tokens.add.or(toggled);
        }
    }
    tokens
}

/// Largest thumbnail; API order is usually ascending but locales differ.
pub fn thumbnail_url(renderer: Option<&Value>) -> Option<String> {
    let renderer = renderer?;
    for pointer in [
        "/musicThumbnailRenderer/thumbnail/thumbnails",
        "/musicAnimatedThumbnailRenderer/backupRenderer/thumbnail/thumbnails",
        "/croppedSquareThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/thumbnails",
        "/thumbnails",
    ] {
        if let Some(url) = renderer.pointer(pointer).and_then(largest_thumbnail) {
            return Some(url);
        }
    }
    None
}

fn largest_thumbnail(thumbnails: &Value) -> Option<String> {
    thumbnails
        .as_array()?
        .iter()
        .filter_map(|thumbnail| {
            let url = thumbnail.get("url").and_then(Value::as_str)?;
            let width = thumbnail.get("width").and_then(Value::as_u64).unwrap_or(0);
            Some((width, url))
        })
        .max_by_key(|(width, _)| *width)
        .map(|(_, url)| url.to_string())
}

pub fn has_explicit(badges: Option<&Value>) -> bool {
    badges
        .and_then(Value::as_array)
        .map(|badges| {
            badges.iter().any(|badge| {
                badge
                    .pointer("/musicInlineBadgeRenderer/icon/iconType")
                    .and_then(Value::as_str)
                    == Some("MUSIC_EXPLICIT_BADGE")
            })
        })
        .unwrap_or(false)
}

/// Continuation token from a `continuations` array; supports radio keys.
pub fn continuation(continuations: Option<&Value>) -> Option<String> {
    continuations
        .and_then(Value::as_array)?
        .iter()
        .find_map(|item| {
            item.pointer("/nextContinuationData/continuation")
                .or_else(|| item.pointer("/nextRadioContinuationData/continuation"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

/// Continuation token inside shelf `contents[]` (`continuationItemRenderer`).
pub fn shelf_continuation(contents: Option<&Value>) -> Option<String> {
    contents.and_then(Value::as_array)?.iter().find_map(|item| {
        item.pointer("/continuationItemRenderer/continuationEndpoint/continuationCommand/token")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

pub fn opt_str(value: Option<&Value>, key: &str) -> Option<String> {
    value?.get(key).and_then(Value::as_str).map(str::to_string)
}

pub fn array_at<'a>(value: &'a Value, path: &str) -> &'a [Value] {
    value
        .pointer(path)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_text_is_never_a_duration() {
        assert_eq!(parse_time("3:04"), Some(184));
        assert_eq!(parse_time("1:00:00"), Some(3600));
        assert_eq!(parse_time("3,3"), Some(183));
        assert_eq!(parse_time("3.42"), Some(222));
        assert_eq!(parse_time("Yesterday"), None);
        assert_eq!(extract_duration(&[Run::new("14 m reproducciones")]), None);
    }

    #[test]
    fn artist_extraction_requires_trusted_page_type() {
        let runs = vec![
            Run::new("canción"),
            Run::new("14 m reproducciones"),
            Run::linked("21 Savage", "UC21"),
            Run::linked("Album title", "MPREb_album"),
        ];
        assert_eq!(
            extract_artists(&runs),
            vec![Artist::new("21 Savage", Some("UC21".into()))]
        );
    }
}
