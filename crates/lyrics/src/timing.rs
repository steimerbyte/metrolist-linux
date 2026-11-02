//! Playback-side queries over a [`LyricsDocument`]: which line/word is active at a
//! position. Pure functions over the parsed data; no allocation except the result.
//!
//! Ported from `LyricsUtils.findCurrentLineIndex` / `findActiveLineIndices`: the app
//! keeps a 100 ms threshold so a line highlights slightly before it is sung, and a
//! background line that overlaps its main line is active at the same time.

use ml_core::{LyricsDocument, LyricsLine, LyricsWord};

/// Look-ahead used by the app before a line is considered current.
pub const ACTIVE_THRESHOLD_MS: u64 = 100;

/// Index of the line that should be highlighted at `position_ms`, if any.
///
/// The last line starting within `ACTIVE_THRESHOLD_MS` of the position. Simultaneous
/// lines are resolved by [`active_lines_at`].
pub fn active_line_at(doc: &LyricsDocument, position_ms: u64) -> Option<usize> {
    active_line_index(&doc.lines, position_ms)
}

/// Same as [`active_line_at`] over a bare line slice.
pub fn active_line_index(lines: &[LyricsLine], position_ms: u64) -> Option<usize> {
    let target = position_ms.saturating_add(ACTIVE_THRESHOLD_MS);
    let mut found = None;
    for (i, line) in lines.iter().enumerate() {
        if line.start_ms > target {
            break;
        }
        found = Some(i);
    }
    found
}

/// Every line being sung at `position_ms` (simultaneous singers overlap).
///
/// A line runs from its start to its derived end (last word end, else the next line's
/// start). Concurrent main lines keep only the latest; a background line stays active
/// alongside its main line.
pub fn active_lines_at(doc: &LyricsDocument, position_ms: u64) -> Vec<usize> {
    active_line_indices(&doc.lines, position_ms)
}

/// Same as [`active_lines_at`] over a bare line slice.
pub fn active_line_indices(lines: &[LyricsLine], position_ms: u64) -> Vec<usize> {
    let mut active = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.start_ms > position_ms {
            break;
        }
        if line_end(lines, i).is_none_or(|end| position_ms <= end) {
            active.push(i);
        }
    }

    if lines.iter().all(|l| l.words.is_empty()) && active.len() > 1 {
        let latest_main = active
            .iter()
            .filter(|&&i| !lines[i].background)
            .map(|&i| lines[i].start_ms)
            .max();
        if let Some(latest) = latest_main {
            active.retain(|&i| lines[i].background || lines[i].start_ms == latest);
        }
    }
    active
}

/// End of line `i`: its derived `end_ms`, else the next line's start.
fn line_end(lines: &[LyricsLine], i: usize) -> Option<u64> {
    lines[i].end_ms.or_else(|| {
        lines[i + 1..]
            .iter()
            .find(|l| l.start_ms > lines[i].start_ms)
            .map(|l| l.start_ms)
    })
}

/// The word being sung at `position_ms` inside `line`, if the line has word timings.
///
/// `None` before the first word starts; between words the previous word stays selected,
/// matching the renderer, which fills every word whose start has passed.
pub fn active_word_at(line: &LyricsLine, position_ms: u64) -> Option<usize> {
    active_word_index(&line.words, position_ms)
}

/// Same as [`active_word_at`] over a bare word slice.
pub fn active_word_index(words: &[LyricsWord], position_ms: u64) -> Option<usize> {
    let mut found = None;
    for (i, word) in words.iter().enumerate() {
        if word.start_ms > position_ms {
            break;
        }
        found = Some(i);
    }
    found
}
