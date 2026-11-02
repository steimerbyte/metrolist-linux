//! YouTube / YouTube Music URL parsing (`YouTubeUrlParser`).

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedUrl {
    Video(String),
    Playlist(String),
    Album(String),
    Artist(String),
}

impl ParsedUrl {
    pub fn id(&self) -> &str {
        match self {
            Self::Video(id) | Self::Playlist(id) | Self::Album(id) | Self::Artist(id) => id,
        }
    }
}

static VIDEO_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?:https?://)?(?:www\.)?(?:music\.)?youtube\.com/watch\?.*v=([a-zA-Z0-9_-]{11})",
        r"(?:https?://)?youtu\.be/([a-zA-Z0-9_-]{11})",
        r"(?:https?://)?(?:www\.)?youtube\.com/shorts/([a-zA-Z0-9_-]{11})",
    ]
    .iter()
    .filter_map(|pattern| Regex::new(pattern).ok())
    .collect()
});

static PLAYLIST_PATTERN: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/playlist\?.*list=([a-zA-Z0-9_-]+)").ok()
});

static ALBUM_PATTERN: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?:https?://)?(?:www\.)?music\.youtube\.com/playlist\?.*list=([a-zA-Z0-9_-]+)")
        .ok()
});

static ARTIST_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?:https?://)?(?:www\.)?music\.youtube\.com/channel/([a-zA-Z0-9_-]+)",
        r"(?:https?://)?(?:www\.)?music\.youtube\.com/browse/(MPRE[a-zA-Z0-9_-]+)",
    ]
    .iter()
    .filter_map(|pattern| Regex::new(pattern).ok())
    .collect()
});

fn capture(pattern: &Regex, url: &str) -> Option<String> {
    pattern
        .captures(url)?
        .get(1)
        .map(|capture| capture.as_str().to_string())
}

/// Mirrors the upstream order: video, then playlist (non-music), then album and
/// artist (music.youtube.com only).
pub fn parse_url(url: &str) -> Option<ParsedUrl> {
    let url = url.trim();
    for pattern in VIDEO_PATTERNS.iter() {
        if let Some(id) = capture(pattern, url) {
            return Some(ParsedUrl::Video(id));
        }
    }
    if !url.contains("music.youtube.com") {
        if let Some(pattern) = PLAYLIST_PATTERN.as_ref() {
            if let Some(id) = capture(pattern, url) {
                return Some(ParsedUrl::Playlist(id));
            }
        }
    }
    if url.contains("music.youtube.com") {
        if let Some(pattern) = ALBUM_PATTERN.as_ref() {
            if let Some(id) = capture(pattern, url) {
                return Some(ParsedUrl::Album(id));
            }
        }
        for pattern in ARTIST_PATTERNS.iter() {
            if let Some(id) = capture(pattern, url) {
                return Some(ParsedUrl::Artist(id));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn videos_from_every_supported_form() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RDAMVM",
            "https://youtu.be/dQw4w9WgXcQ?si=x",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "  https://youtube.com/watch?v=dQw4w9WgXcQ  ",
        ] {
            assert_eq!(
                parse_url(url),
                Some(ParsedUrl::Video("dQw4w9WgXcQ".into())),
                "{url}"
            );
        }
    }

    #[test]
    fn playlists_albums_and_artists() {
        assert_eq!(
            parse_url("https://www.youtube.com/playlist?list=PL123"),
            Some(ParsedUrl::Playlist("PL123".into()))
        );
        assert_eq!(
            parse_url("https://music.youtube.com/playlist?list=OLAK5uy_album"),
            Some(ParsedUrl::Album("OLAK5uy_album".into()))
        );
        assert_eq!(
            parse_url("https://music.youtube.com/channel/UCabc"),
            Some(ParsedUrl::Artist("UCabc".into()))
        );
        assert_eq!(
            parse_url("https://music.youtube.com/browse/MPREb_album"),
            Some(ParsedUrl::Artist("MPREb_album".into()))
        );
    }

    #[test]
    fn unrelated_urls_are_rejected() {
        for url in [
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/channel/UCabc",
            "not a url",
            "",
        ] {
            assert_eq!(parse_url(url), None, "{url}");
        }
    }
}
