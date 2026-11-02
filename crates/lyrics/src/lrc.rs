//! LRC parsing: plain `[mm:ss.xx]text`, rich sync (`[mm:ss.xx]<mm:ss.xx>word`),
//! inline `{agent:vN}` / `{bg}` markers, `[bg: ...]` lines and the `[offset:]` tag.
//!
//! Plain and rich-sync lines are parsed in one pass because callers consume both in the
//! same list (KuGou serves plain LRC, a TTML dump is rich sync).
//!
//! Every timestamp is total arithmetic: `parse_stamp` returns `None` instead of
//! panicking or overflowing, and offsets clamp at zero.

use base64::Engine as _;
use ml_core::{LyricsDocument, LyricsLine, LyricsWord, MlResult};

/// `[offset:]` value meaning: positive shifts every timestamp **earlier**.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LrcMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub author: Option<String>,
    pub length: Option<String>,
    /// Raw `[offset:n]` in milliseconds, already applied to the returned timestamps.
    pub offset_ms: i64,
}

/// Parsed LRC document plus its metadata tags.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lrc {
    pub meta: LrcMeta,
    pub doc: LyricsDocument,
}

/// Parse an LRC document. Malformed lines are skipped, never fatal.
pub fn parse_lrc(text: &str) -> MlResult<LyricsDocument> {
    Ok(parse_lrc_with_meta(text)?.doc)
}

/// Parse an LRC document, keeping the metadata tags.
pub fn parse_lrc_with_meta(text: &str) -> MlResult<Lrc> {
    let body = unescape(text);
    let lines: Vec<&str> = body.lines().collect();

    let mut meta = LrcMeta::default();
    for line in &lines {
        if let Some((key, value)) = metadata_tag(line) {
            match key {
                "ti" => meta.title = Some(value),
                "ar" => meta.artist = Some(value),
                "al" => meta.album = Some(value),
                "by" => meta.author = Some(value),
                "length" => meta.length = Some(value),
                "offset" => meta.offset_ms = value.parse().unwrap_or(0),
                _ => {}
            }
        }
    }

    let mut entries: Vec<Entry> = Vec::new();
    let mut last: Option<usize> = None;

    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        // A standalone `<word:start:end|...>` line carries the previous line's words.
        if line.starts_with('<') && line.ends_with('>') {
            if let Some(words) = parse_word_list(&line[1..line.len() - 1]) {
                if let Some(idx) = last {
                    let entry = &mut entries[idx];
                    if entry.words.is_empty() {
                        if entry.start_from_hint {
                            entry.start_ms = words[0].start_ms;
                            entry.start_from_hint = false;
                        }
                        entry.words = words;
                    }
                }
                continue;
            }
        }

        // `[bg: ...]` — a background line whose timings are inline or on the next line.
        if let Some(payload) = line
            .strip_prefix("[bg:")
            .and_then(|rest| rest.strip_suffix(']'))
        {
            let parsed = parse_timed_content(payload.trim(), None);
            let from_hint = parsed.words.is_empty();
            let start = parsed
                .words
                .first()
                .map(|w| w.start_ms)
                .unwrap_or_else(|| next_start_hint(&lines[i + 1..]));
            entries.push(Entry {
                start_ms: start,
                text: parsed.text,
                words: parsed.words,
                agent: inherited_agent(&entries),
                background: true,
                start_from_hint: from_hint,
            });
            last = Some(entries.len() - 1);
            continue;
        }

        let Some((times, content)) = split_timestamps(line) else {
            continue;
        };

        let mut content = content.trim_start();
        let mut agent = None;
        let mut background = false;

        if let Some(rest) = content.strip_prefix('{') {
            if let Some(end) = rest.find('}') {
                if let Some(a) = rest[..end].strip_prefix("agent:") {
                    agent = Some(a.trim().to_string());
                    content = rest[end + 1..].trim_start();
                }
            }
        }
        // `v1:` / `v2:` prefix from Paxsenix-shaped dumps.
        if let Some((prefix_agent, rest)) = split_agent_prefix(content) {
            agent = Some(prefix_agent);
            content = rest;
        }
        if let Some(rest) = content.strip_prefix("{bg}") {
            background = true;
            content = rest.trim_start();
        }
        if background && agent.is_none() {
            agent = inherited_agent(&entries);
        }

        let parsed = parse_timed_content(content, Some(&lines[i + 1..]));
        if parsed.text.is_empty() && parsed.words.is_empty() {
            continue;
        }

        for ts in times {
            entries.push(Entry {
                start_ms: ts,
                text: parsed.text.clone(),
                words: parsed.words.clone(),
                agent: agent.clone(),
                background,
                start_from_hint: false,
            });
        }
        last = Some(entries.len() - 1);
    }

    // Stable sort: duplicate timestamps keep their document order.
    entries.sort_by_key(|e| e.start_ms);
    let offset_ms = meta.offset_ms;
    let mut lines: Vec<LyricsLine> = entries
        .into_iter()
        .map(|e| e.into_line(offset_ms))
        .collect();

    derive_end_times(&mut lines);

    Ok(Lrc {
        meta,
        doc: LyricsDocument {
            provider: "lrc".to_string(),
            lines,
            plain: None,
            translated: None,
            language: None,
        },
    })
}

/// Drop the credit block providers prepend (`Synced by…`, `Lyrics by：…`, `Composed by…`).
///
/// Upstream cuts every such line before the first real lyric, matching `.+].+[:：].+`:
/// a `word: value` line in the header. Only the head of the document is inspected, so a
/// sung line containing a colon later on survives.
pub fn filter_credits(lines: &mut Vec<LyricsLine>) {
    const HEAD_LIMIT: usize = 30;
    let credit = |line: &LyricsLine| {
        let text = line.text.trim();
        let lower = text.to_lowercase();
        let named_credit = [
            "synced by",
            "lyrics by",
            "music by",
            "arranged by",
            "composed by",
            "written by",
            "produced by",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix));
        // `word: value` — KuGou separates with a fullwidth colon.
        let labelled = text.split_once([':', '：']).is_some_and(|(head, tail)| {
            !head.is_empty() && !tail.trim().is_empty() && head.len() <= 40
        });
        // `Title - Artist` header line, as KuGou emits above the first lyric.
        let title_artist = text
            .split_once(" - ")
            .is_some_and(|(title, artist)| !title.trim().is_empty() && !artist.trim().is_empty());
        named_credit || labelled || title_artist
    };
    // Only strip a leading credit block: once a non-credit line appears, stop looking.
    let mut cut = 0;
    for (i, line) in lines.iter().enumerate().take(HEAD_LIMIT) {
        if credit(line) {
            cut = i + 1;
        } else {
            break;
        }
    }
    if cut > 0 {
        lines.drain(..cut);
    }
    lines.retain(|line| !line.text.trim().is_empty());
}

/// Decode the KuGou payload: base64 of an LRC document.
pub fn decode_kugou(content: &str) -> MlResult<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content.trim())
        .map_err(|e| ml_core::MlError::Parse(format!("kugou base64: {e}")))?;
    String::from_utf8(bytes).map_err(|e| ml_core::MlError::Parse(format!("kugou utf8: {e}")))
}

/// One parsed line, before offsets are applied.
#[derive(Debug, Clone)]
struct Entry {
    start_ms: u64,
    text: String,
    words: Vec<LyricsWord>,
    agent: Option<String>,
    background: bool,
    /// Start came from a following line's hint, not from this line's own timing.
    start_from_hint: bool,
}

impl Entry {
    fn into_line(self, offset_ms: i64) -> LyricsLine {
        LyricsLine {
            start_ms: shift(self.start_ms, offset_ms),
            end_ms: None,
            text: self.text,
            words: self
                .words
                .into_iter()
                .map(|w| LyricsWord {
                    start_ms: shift(w.start_ms, offset_ms),
                    end_ms: shift(w.end_ms, offset_ms),
                    text: w.text,
                })
                .collect(),
            background: self.background,
            agent: self.agent,
        }
    }
}

/// Agent of the most recent main (non-background) line.
fn inherited_agent(entries: &[Entry]) -> Option<String> {
    entries
        .iter()
        .rev()
        .find(|e| !e.background)
        .and_then(|e| e.agent.clone())
}

/// Leading `[mm:ss.xx]` timestamps plus the remaining content.
fn split_timestamps(line: &str) -> Option<(Vec<u64>, &str)> {
    let mut rest = line;
    let mut times = Vec::new();
    while let Some(after_open) = rest.strip_prefix('[') {
        let Some(close) = after_open.find(']') else {
            break;
        };
        match parse_stamp(&after_open[..close]) {
            Some(ms) => {
                times.push(ms);
                rest = after_open[close + 1..].trim_start();
            }
            None => break,
        }
    }
    (!times.is_empty()).then_some((times, rest))
}

/// `mm:ss`, `mm:ss.xx`, `mm:ss.xxx`, optionally `hh:mm:ss.xx`.
///
/// Total: non-numeric parts, `ss >= 60`, an over-long field or overflow give `None`.
fn parse_stamp(stamp: &str) -> Option<u64> {
    let stamp = stamp.trim();
    if !stamp.as_bytes().first()?.is_ascii_digit() {
        return None;
    }
    let (clock, frac_ms) = match stamp.rsplit_once('.') {
        Some((clock, frac)) => {
            if frac.is_empty() || frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (clock, fraction_ms(frac))
        }
        None => (stamp, 0),
    };
    let mut parts = clock.split(':');
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let (hours, minutes, seconds): (u64, u64, u64) = match third {
        Some(secs) => (
            first.parse().ok()?,
            second.parse().ok()?,
            secs.parse().ok()?,
        ),
        None => (0, first.parse().ok()?, second.parse().ok()?),
    };
    if hours > 24 || minutes >= 60 || seconds >= 60 {
        return None;
    }
    let total = hours
        .checked_mul(3600)?
        .checked_add(minutes.checked_mul(60)?)?
        .checked_add(seconds)?;
    total.checked_mul(1000)?.checked_add(frac_ms)
}

/// `5` → 500 ms, `50` → 50 ms, `500`/`5000` → 500 ms.
fn fraction_ms(fraction: &str) -> u64 {
    match fraction.len() {
        1 => fraction.parse::<u64>().unwrap_or(0) * 100,
        2 => fraction.parse::<u64>().unwrap_or(0) * 10,
        _ => fraction.get(..3).and_then(|f| f.parse().ok()).unwrap_or(0),
    }
}

/// Result of parsing the content after a line timestamp.
#[derive(Debug, Default)]
struct TimedContent {
    /// Spoken text with inline timestamps removed.
    text: String,
    words: Vec<LyricsWord>,
}

/// Parse `text` with `<mm:ss.xx>word` markers; each marker applies to the text after it.
///
/// `following` is the rest of the document, used to derive the last word's end.
fn parse_timed_content(content: &str, following: Option<&[&str]>) -> TimedContent {
    let mut words: Vec<LyricsWord> = Vec::new();
    let mut text = String::with_capacity(content.len());
    let mut word_text = String::new();
    let mut current: Option<u64> = None;
    let mut rest = content;

    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open + 1..].find('>') else {
            break;
        };
        let inner = &rest[open + 1..open + 1 + close];
        let Some(ms) = parse_stamp(inner) else {
            // Not a timestamp (`<3`): keep it as literal text and move on.
            let literal = &rest[..open + close + 2];
            text.push_str(literal);
            word_text.push_str(literal);
            rest = &rest[open + close + 2..];
            continue;
        };
        let before = &rest[..open];
        text.push_str(before);
        word_text.push_str(before);
        flush_word(&mut words, &mut word_text, current);
        current = Some(ms);
        rest = &rest[open + close + 2..];
    }
    text.push_str(rest);
    word_text.push_str(rest);
    flush_word(&mut words, &mut word_text, current);

    // A trailing clock value after the last word is that word's end.
    if let Some(last) = words.last_mut() {
        if last.end_ms <= last.start_ms {
            if let Some(end) = current {
                if end > last.start_ms {
                    last.end_ms = end;
                }
            }
        }
    }

    // Unset ends run to the next word, then to the next main line.
    for i in 0..words.len().saturating_sub(1) {
        if words[i].end_ms <= words[i].start_ms {
            words[i].end_ms = words[i + 1].start_ms;
        }
    }
    if let Some(last) = words.last_mut() {
        if last.end_ms <= last.start_ms {
            let next = following.and_then(next_main_line_start);
            last.end_ms = match next {
                Some(n) if n > last.start_ms => n,
                _ => last.start_ms.saturating_add(DEFAULT_WORD_TAIL_MS),
            };
        }
    }

    TimedContent {
        text: text.trim().to_string(),
        words,
    }
}

fn flush_word(words: &mut Vec<LyricsWord>, pending: &mut String, start: Option<u64>) {
    if let Some(start) = start {
        let text = pending.trim();
        if !text.is_empty() {
            words.push(LyricsWord {
                start_ms: start,
                end_ms: start,
                text: text.to_string(),
            });
        }
    }
    pending.clear();
}

/// Parse `<text:start:end|text:start:end>` (seconds, no wrapping brackets).
fn parse_word_list(payload: &str) -> Option<Vec<LyricsWord>> {
    let mut words = Vec::new();
    for chunk in payload.split('|') {
        let parts: Vec<&str> = chunk.split(':').collect();
        if parts.len() < 3 {
            return None;
        }
        let start: f64 = parts[parts.len() - 2].trim().parse().ok()?;
        let end: f64 = parts[parts.len() - 1].trim().parse().ok()?;
        let text = parts[..parts.len() - 2].join(":").trim().to_string();
        if text.is_empty() {
            continue;
        }
        words.push(LyricsWord {
            start_ms: seconds_to_ms(start),
            end_ms: seconds_to_ms(end),
            text,
        });
    }
    (!words.is_empty()).then_some(words)
}

/// `f64` seconds → ms, saturating; NaN and negatives give 0.
fn seconds_to_ms(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * 1000.0).round() as u64
}

/// `v1:`-style agent prefix.
fn split_agent_prefix(content: &str) -> Option<(String, &str)> {
    let (agent, rest) = content.split_once(':')?;
    let digits = agent.trim().strip_prefix('v')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((agent.trim().to_string(), rest.trim_start()))
}

/// Parse `[key:value]` metadata; `None` for timestamps and unknown keys.
fn metadata_tag(line: &str) -> Option<(&str, String)> {
    let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (key, value) = inner.split_once(':')?;
    let key = key.trim();
    matches!(key, "ti" | "ar" | "al" | "by" | "offset" | "length")
        .then(|| (key, value.trim().to_string()))
}

/// Start of the next line that can end the previous word's timing.
///
/// Background lines overlap the line they accompany, so they are not "next". A
/// background line without inline timings hands its words to a following standalone
/// `<...>` line, which has no `[timestamp]` and is skipped by the same rule.
fn next_main_line_start(lines: &[&str]) -> Option<u64> {
    for line in lines {
        let Some((times, content)) = split_timestamps(line.trim()) else {
            continue;
        };
        if content.trim_start().starts_with("{bg}") {
            continue;
        }
        return times.first().copied();
    }
    None
}

/// First timestamp found in `lines`: start hint for a `[bg:]` line without timing.
fn next_start_hint(lines: &[&str]) -> u64 {
    lines
        .iter()
        .find_map(|l| split_timestamps(l.trim()))
        .and_then(|(times, _)| times.first().copied())
        .unwrap_or(0)
}

/// Fill each line's `end_ms`: last word end when timed, else the next main line start.
fn derive_end_times(lines: &mut [LyricsLine]) {
    for i in 0..lines.len() {
        let end = match lines[i].words.last() {
            Some(word) => Some(word.end_ms),
            None => lines[i + 1..]
                .iter()
                .find(|l| l.start_ms > lines[i].start_ms && (lines[i].background || !l.background))
                .map(|l| l.start_ms),
        };
        lines[i].end_ms = end.filter(|e| *e > lines[i].start_ms);
    }
}

/// Apply the `[offset:]` value; positive offset moves lyrics earlier. Clamps at zero.
fn shift(ms: u64, offset_ms: i64) -> u64 {
    (ms as i64).saturating_sub(offset_ms).max(0) as u64
}

/// Longest tail handed to a word whose end cannot be derived.
pub const DEFAULT_WORD_TAIL_MS: u64 = 500;

/// Decode `\n` escapes and HTML entities some providers ship inside JSON.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '&' => out.push_str(&decode_entity(&mut chars)),
            other => out.push(other),
        }
    }
    out
}

/// Decode `&name;` / `&#123;` / `&#x1F;` starting after the `&`; unknown stays literal.
fn decode_entity(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut name = String::new();
    while let Some(&c) = chars.peek() {
        if c == ';' {
            chars.next();
            return match name.as_str() {
                "amp" => "&".to_string(),
                "lt" => "<".to_string(),
                "gt" => ">".to_string(),
                "quot" => "\"".to_string(),
                "apos" => "'".to_string(),
                "nbsp" => " ".to_string(),
                _ => {
                    let code = name
                        .strip_prefix("#x")
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                        .and_then(char::from_u32);
                    match code {
                        Some(c) => c.to_string(),
                        None => format!("&{name};"),
                    }
                }
            };
        }
        if c.is_alphanumeric() && name.len() < 12 {
            name.push(c);
            chars.next();
        } else {
            break;
        }
    }
    format!("&{name}")
}
