//! File naming: `<title> - <artist> [<videoId>].<ext>`, sanitized and bounded.
//!
//! The allowed character set is exactly `[A-Za-z0-9 ._()[]-]`; everything else
//! becomes `_` (whitespace becomes a space) and the video id guarantees that two
//! different songs can never share a name.

use std::path::{Path, PathBuf};

/// Hard cap on the byte length of a generated file name.
pub const MAX_FILE_NAME_BYTES: usize = 120;

/// Longest video id kept verbatim in the name; longer ids are shortened with a
/// blake3 prefix so the suffix always fits.
const MAX_ID_BYTES: usize = 48;

fn allowed(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '(' | ')' | '[' | ']' | '-')
}

/// Sanitize one path component.
///
/// Postconditions (proved by hand in the Verus handoff):
/// every output char is in the allowed set, so `/` and `\` can never appear;
/// the result never contains `..`; it never starts with `.` or a space and never
/// ends with `.` or a space; and the result is never empty (`"track"`).
pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len().min(MAX_FILE_NAME_BYTES));
    for c in input.chars() {
        let mapped = if allowed(c) {
            c
        } else if c.is_whitespace() {
            ' '
        } else {
            '_'
        };
        // Collapse a dot run: forbids ".." and keeps names tidy.
        if mapped == '.' && out.ends_with('.') {
            continue;
        }
        out.push(mapped);
    }
    let trimmed = out.trim_matches(|c: char| c == ' ' || c == '.');
    if trimmed.is_empty() {
        "track".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Cut `s` to at most `max` bytes on a char boundary (no panic).
fn truncate_at_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn clean_extension(extension: &str) -> String {
    let cleaned: String = extension
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(5)
        .collect::<String>()
        .to_ascii_lowercase();
    if cleaned.is_empty() {
        "webm".to_string()
    } else {
        cleaned
    }
}

/// Map a stream mime type to a file extension. Unknown mime types fall back to
/// `webm`, the measured VISIONOS `audio/webm; codecs="opus"` container.
///
/// Postcondition: the result is one of `webm`, `m4a`, `opus`, `mp3`, `flac`.
pub fn extension_for_mime(mime: &str) -> &'static str {
    let mime = mime.to_ascii_lowercase();
    if mime.contains("webm") {
        "webm"
    } else if mime.contains("mp4") || mime.contains("m4a") || mime.contains("aac") {
        "m4a"
    } else if mime.contains("ogg") || mime.contains("opus") {
        "opus"
    } else if mime.contains("mpeg") || mime.contains("mp3") {
        "mp3"
    } else if mime.contains("flac") {
        "flac"
    } else {
        "webm"
    }
}

/// Build the final file name for one track.
///
/// Postconditions: byte length `<= MAX_FILE_NAME_BYTES`, every char is in the
/// allowed set, no `..`, and the name ends with ` [<videoId>].<ext>` (a
/// sanitized/capped id) so different songs cannot collide. Total for every input.
pub fn file_name(title: &str, artist: &str, video_id: &str, extension: &str) -> String {
    let extension = clean_extension(extension);
    let id = {
        let sanitized = sanitize(video_id);
        if sanitized.len() > MAX_ID_BYTES {
            let digest = blake3::hash(video_id.as_bytes()).to_hex();
            format!(
                "{}-{}",
                truncate_at_boundary(&sanitized, MAX_ID_BYTES - 9),
                &digest[..8]
            )
        } else {
            sanitized
        }
    };
    let suffix = format!(" [{id}].{extension}");
    // Cannot underflow: the suffix is at most 4 + 48 + 5 = 57 bytes.
    let budget = MAX_FILE_NAME_BYTES.saturating_sub(suffix.len());
    let base = sanitize(&format!("{title} - {artist}"));
    let base = truncate_at_boundary(&base, budget);
    let base = base.trim_end_matches([' ', '.']);
    // A base of pure punctuation (`- `, `...`) reads as an empty title.
    let base = if base.chars().any(|c| c.is_ascii_alphanumeric()) {
        base
    } else {
        "track"
    };
    format!("{base}{suffix}")
}

/// Scratch file for an in-flight transfer; always inside the download dir.
pub(crate) fn part_path(dir: &Path, video_id: &str) -> PathBuf {
    dir.join(format!("dl_{}.part", sanitize(video_id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_safe(name: &str, id: &str) {
        assert!(name.len() <= MAX_FILE_NAME_BYTES, "{name:?} too long");
        assert!(!name.contains('/') && !name.contains('\\'), "{name:?}");
        assert!(!name.contains(".."), "{name:?}");
        assert!(!name.starts_with('.') && !name.starts_with(' '), "{name:?}");
        assert!(name.contains(&format!("[{id}]")), "{name:?} lacks id");
        assert!(
            name.chars().all(|c| allowed(c) || c == '[' || c == ']'),
            "{name:?}"
        );
    }

    #[test]
    fn traversal_cannot_escape_the_directory() {
        let name = file_name("../../etc/passwd", "../root", "abc123", "webm");
        assert_safe(&name, "abc123");
        assert_eq!(name, "_._etc_passwd - ._root [abc123].webm");
        assert!(!name.contains('/'));
        assert!(!Path::new(&name)
            .components()
            .any(|c| c == std::path::Component::ParentDir));
    }

    #[test]
    fn unicode_is_replaced_not_dropped() {
        assert_eq!(sanitize("日本語 – title"), "___ _ title");
        assert_eq!(sanitize("Beyoncé"), "Beyonc_");
        let name = file_name("歌", "歌手", "vid00000001", "opus");
        assert_safe(&name, "vid00000001");
    }

    #[test]
    fn long_titles_are_capped_and_keep_the_suffix() {
        let title = "x".repeat(1000);
        let artist = "y".repeat(500);
        let name = file_name(&title, &artist, "kJQP7kiw5Fk", "webm");
        assert_safe(&name, "kJQP7kiw5Fk");
        assert!(name.len() <= MAX_FILE_NAME_BYTES);
        assert!(name.ends_with(" [kJQP7kiw5Fk].webm"));
    }

    #[test]
    fn very_long_video_ids_are_hashed() {
        let id = "z".repeat(400);
        let name = file_name("t", "a", &id, "m4a");
        assert!(name.len() <= MAX_FILE_NAME_BYTES);
        assert!(name.ends_with(".m4a"));
        assert!(!name.contains(&id));
        // Deterministic and distinct per id.
        assert_eq!(name, file_name("t", "a", &id, "m4a"));
        assert_ne!(name, file_name("t", "a", &format!("{id}0"), "m4a"));
    }

    #[test]
    fn different_songs_never_collide() {
        let first = file_name("Song", "Artist", "aaaaaaaaaaa", "webm");
        let second = file_name("Song", "Artist", "bbbbbbbbbbb", "webm");
        assert_ne!(first, second);
        // Same song, same name (idempotent).
        assert_eq!(
            file_name("Song", "Artist", "aaaaaaaaaaa", "webm"),
            file_name("Song", "Artist", "aaaaaaaaaaa", "webm")
        );
    }

    #[test]
    fn empty_and_hostile_inputs_stay_valid() {
        assert_eq!(file_name("", "", "abc", "webm"), "track [abc].webm");
        assert_eq!(sanitize(""), "track");
        assert_eq!(sanitize("   "), "track");
        assert_eq!(sanitize("..."), "track");
        assert_eq!(sanitize("a..b"), "a.b");
        assert_eq!(file_name("t", "a", "abc", ""), "t - a [abc].webm");
        assert_eq!(file_name("t", "a", "abc", "../x"), "t - a [abc].x");
    }

    #[test]
    fn extensions_follow_mime() {
        assert_eq!(extension_for_mime("audio/webm; codecs=\"opus\""), "webm");
        assert_eq!(extension_for_mime("audio/mp4"), "m4a");
        assert_eq!(extension_for_mime("audio/mp4; codecs=\"mp4a.40.2\""), "m4a");
        assert_eq!(extension_for_mime("audio/ogg; codecs=opus"), "opus");
        assert_eq!(extension_for_mime("audio/mpeg"), "mp3");
        assert_eq!(extension_for_mime("audio/flac"), "flac");
        assert_eq!(extension_for_mime(""), "webm");
        assert_eq!(extension_for_mime("application/octet-stream"), "webm");
    }

    #[test]
    fn part_files_stay_in_the_directory() {
        let path = part_path(Path::new("/music"), "../../etc/passwd");
        assert_eq!(path.parent(), Some(Path::new("/music")));
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("part"));
    }
}
