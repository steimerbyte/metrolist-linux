//! Response-shape parsers: one module per InnerTube surface.

pub mod album;
pub mod artist;
pub mod browse;
pub mod home;
pub mod moods;
pub mod next;
pub mod playlist;
pub mod podcast;
pub mod search;

use ml_core::SectionKind;

/// Best-effort shelf classification by localized title (UI hint only).
pub fn section_kind(title: &str) -> SectionKind {
    let title = title.to_lowercase();
    let contains = |needles: &[&str]| needles.iter().any(|needle| title.contains(needle));
    if contains(&["quick pick", "schnellauswahl"]) {
        SectionKind::QuickPicks
    } else if contains(&["keep listening", "weiterhören"]) {
        SectionKind::KeepListening
    } else if contains(&["new release", "neuerscheinung"]) {
        SectionKind::NewReleases
    } else if contains(&["album"]) {
        SectionKind::Albums
    } else if contains(&["artist"]) {
        SectionKind::Artists
    } else if contains(&["playlist", "wiedergabeliste"]) {
        SectionKind::Playlists
    } else if contains(&["video"]) {
        SectionKind::Videos
    } else if contains(&["episode", "podcast"]) {
        SectionKind::Episodes
    } else if contains(&["song", "track", "titel"]) {
        SectionKind::Songs
    } else if contains(&["related", "similar", "ähnlich"]) {
        SectionKind::Related
    } else {
        SectionKind::Unknown
    }
}
