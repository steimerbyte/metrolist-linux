//! `FEmusic_moods_and_genres` → category buttons with their browse params.
//!
//! Button clicks browse `FEmusic_moods_and_genres_category` with each button's
//! `params` (see [`crate::InnertubeClient::mood_genre_page`]).

use serde_json::Value;

use crate::parse::{runs_from, text_of};

use super::browse::section_list;

/// One mood/genre landing group ("Moods & moments", "Genres").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoodGroup {
    pub title: String,
    pub items: Vec<MoodGenre>,
}

/// One colored category button.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoodGenre {
    pub title: String,
    pub browse_id: String,
    pub params: Option<String>,
    /// `leftStripeColor` (ARGB) for the button tint.
    pub stripe_color: Option<u32>,
}

/// The mood/genre landing page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoodsAndGenres {
    pub groups: Vec<MoodGroup>,
}

fn grid_group(grid: &Value) -> Option<MoodGroup> {
    let title = grid
        .pointer("/header/gridHeaderRenderer/title")
        .map(runs_from)
        .map(|runs| text_of(&runs))
        .unwrap_or_default();
    let items: Vec<MoodGenre> = grid
        .get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|item| item.get("musicNavigationButtonRenderer"))
        .filter_map(mood_genre)
        .collect();
    (!items.is_empty()).then_some(MoodGroup { title, items })
}

/// `MoodAndGenres.fromMusicNavigationButtonRenderer`: title, stripe color and
/// the click command's browse endpoint are all required.
fn mood_genre(button: &Value) -> Option<MoodGenre> {
    let title = text_of(&runs_from(button.get("buttonText")?));
    if title.is_empty() {
        return None;
    }
    let endpoint = button.pointer("/clickCommand/browseEndpoint")?;
    let browse_id = endpoint.get("browseId")?.as_str()?.to_string();
    Some(MoodGenre {
        title,
        browse_id,
        params: endpoint
            .get("params")
            .and_then(Value::as_str)
            .map(str::to_string),
        stripe_color: button
            .pointer("/solid/leftStripeColor")
            .and_then(Value::as_u64)
            .map(|color| color as u32),
    })
}

/// Parse the mood/genre landing browse response.
pub fn from_browse(value: &Value) -> MoodsAndGenres {
    let groups = section_list(value)
        .and_then(|list| list.get("contents"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|content| content.get("gridRenderer"))
        .filter_map(grid_group)
        .collect();
    MoodsAndGenres { groups }
}
