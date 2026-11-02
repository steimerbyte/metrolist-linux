//! TTML (Apple Music / BetterLyrics) → timed lines, ported from the upstream
//! `JSONParser`/`TTMLParser` pair: agents from the document, `x-bg` background spans,
//! `x-roman`/`x-translation` skipped, split syllables merged, `lyricOffset` applied.

use ml_core::{LyricsDocument, LyricsLine, LyricsWord, MlError, MlResult};
use roxmltree::Node;

use crate::lrc::DEFAULT_WORD_TAIL_MS;

const META_NS: &str = "http://www.w3.org/ns/ttml#metadata";
const PARAM_NS: &str = "http://www.w3.org/ns/ttml#parameter";

/// Parse a TTML document. Lines keep document order (the `{bg}` de-duplication in
/// [`to_lrc`] depends on it).
pub fn parse_ttml(text: &str) -> MlResult<LyricsDocument> {
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| MlError::Parse(format!("invalid TTML: {e}")))?;
    let root = doc.root_element();

    let offset_ms = root
        .children()
        .find(|n| n.has_tag_name("head"))
        .and_then(|head| head.children().find(|n| n.has_tag_name("metadata")))
        .and_then(|meta| meta.children().find(|n| n.has_tag_name("audio")))
        .and_then(|audio| audio.attribute("lyricOffset"))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(0.0);

    let mut lines = Vec::new();
    if let Some(body) = root.children().find(|n| n.has_tag_name("body")) {
        walk(body, &mut lines, offset_ms, None);
    }

    Ok(LyricsDocument {
        provider: "ttml".to_string(),
        lines,
        plain: None,
        translated: None,
        language: None,
    })
}

fn walk(node: Node, lines: &mut Vec<LyricsLine>, offset_ms: f64, parent_agent: Option<&str>) {
    let mut agent = parent_agent;
    if node.has_tag_name("div") {
        if let Some(a) = get_attr(&node, "agent") {
            agent = Some(a);
        }
    }
    if node.has_tag_name("p") {
        parse_p(&node, lines, offset_ms, agent);
        return;
    }
    for child in node.children().filter(|n| n.is_element()) {
        walk(child, lines, offset_ms, agent);
    }
}

fn parse_p(p: &Node, lines: &mut Vec<LyricsLine>, offset_ms: f64, div_agent: Option<&str>) {
    let begin = timing_attr(p, "begin").or_else(|| find_first_span_begin(p));
    let Some(begin) = begin else { return };
    let start_ms = seconds(parse_time(begin) + offset_ms);

    let agent = get_attr(p, "agent").or(div_agent).map(str::to_string);
    let is_p_background = get_attr(p, "role") == Some("x-bg");

    let mut word_spans: Vec<SpanInfo> = Vec::new();
    let mut background: Vec<LyricsLine> = Vec::new();

    for child in p.children().filter(|n| n.is_element()) {
        match get_attr(&child, "role") {
            Some("x-bg") if is_p_background => parse_word_span(&child, offset_ms, &mut word_spans),
            Some("x-bg") => {
                if let Some(bg) = parse_background_span(&child, start_ms, offset_ms) {
                    background.push(bg);
                }
            }
            Some("x-translation") | Some("x-roman") => {}
            _ => parse_word_span(&child, offset_ms, &mut word_spans),
        }
    }

    let mut words = merge_spans(word_spans);
    let text = if words.is_empty() {
        direct_text(p).trim().to_string()
    } else {
        build_text(&words)
    };

    line_ends(&mut words);
    let words_end = words.last().map(|w| w.end_ms);

    if !text.is_empty() {
        lines.push(LyricsLine {
            start_ms,
            end_ms: words_end,
            text,
            words,
            background: is_p_background,
            agent,
        });
        lines.extend(merge_background(background));
    } else if !background.is_empty() {
        lines.extend(merge_background(background));
    }
}

/// Upstream merges every background span of a `<p>` into a single background line.
fn merge_background(background: Vec<LyricsLine>) -> Option<LyricsLine> {
    let mut it = background.into_iter();
    let first = it.next()?;
    let mut merged = LyricsLine {
        start_ms: first.start_ms,
        end_ms: first.end_ms,
        text: first.text,
        words: first.words,
        background: true,
        agent: first.agent,
    };
    for next in it {
        if !next.text.is_empty() {
            if !merged.text.is_empty() {
                merged.text.push(' ');
            }
            merged.text.push_str(&next.text);
        }
        merged.words.extend(next.words);
        merged.start_ms = merged.start_ms.min(next.start_ms);
        merged.end_ms = next.end_ms.or(merged.end_ms);
    }
    Some(merged)
}

fn parse_word_span(span: &Node, offset_ms: f64, spans: &mut Vec<SpanInfo>) {
    let (Some(begin), Some(end)) = (timing_attr(span, "begin"), timing_attr(span, "end")) else {
        return;
    };
    let text = text_content(span);
    let spaced = text.ends_with(char::is_whitespace)
        || span
            .next_sibling()
            .and_then(|n| n.text())
            .is_some_and(|t| t.starts_with(char::is_whitespace));
    spans.push(SpanInfo {
        text,
        start_ms: seconds(parse_time(begin) + offset_ms),
        end_ms: seconds(parse_time(end) + offset_ms),
        has_trailing_space: spaced,
    });
}

fn parse_background_span(span: &Node, parent_start_ms: u64, offset_ms: f64) -> Option<LyricsLine> {
    let start_ms = timing_attr(span, "begin")
        .map(|b| seconds(parse_time(b) + offset_ms))
        .unwrap_or(parent_start_ms);

    let mut spans = Vec::new();
    let mut has_spans = false;
    for child in span.children().filter(|n| n.is_element()) {
        has_spans = true;
        if !matches!(
            get_attr(&child, "role"),
            Some("x-translation") | Some("x-roman")
        ) {
            parse_word_span(&child, offset_ms, &mut spans);
        }
    }

    let mut words = if has_spans {
        merge_spans(spans)
    } else {
        Vec::new()
    };
    line_ends(&mut words);
    let text = if words.is_empty() {
        text_content(span).trim().to_string()
    } else {
        build_text(&words)
    };
    if text.is_empty() {
        return None;
    }
    Some(LyricsLine {
        start_ms,
        end_ms: words.last().map(|w| w.end_ms),
        text,
        words,
        background: true,
        agent: None,
    })
}

/// One timed span before syllables are merged into words.
struct SpanInfo {
    text: String,
    start_ms: u64,
    end_ms: u64,
    has_trailing_space: bool,
}

/// Merge split syllables: a span that does not end a word (no trailing space, no
/// hyphen) continues the previous one.
fn merge_spans(spans: Vec<SpanInfo>) -> Vec<LyricsWord> {
    let mut it = spans.into_iter();
    let Some(first) = it.next() else {
        return Vec::new();
    };
    let mut words = Vec::new();
    let mut text = first.text;
    let mut start = first.start_ms;
    let mut end = first.end_ms;
    let mut prev_spaced = first.has_trailing_space;
    let mut prev_hyphen = text.ends_with('-');

    for span in it {
        let SpanInfo {
            text: span_text,
            start_ms,
            end_ms,
            has_trailing_space,
        } = span;
        let hyphen = span_text.ends_with('-');
        if prev_spaced && !prev_hyphen {
            push_word(&mut words, &text, start, end);
            text = span_text;
            start = start_ms;
        } else {
            text.push_str(&span_text);
        }
        end = end_ms;
        prev_spaced = has_trailing_space;
        prev_hyphen = hyphen;
    }
    push_word(&mut words, &text, start, end);
    words
}

fn push_word(words: &mut Vec<LyricsWord>, text: &str, start_ms: u64, end_ms: u64) {
    let text = text.trim();
    if !text.is_empty() {
        words.push(LyricsWord {
            start_ms,
            end_ms: end_ms.max(start_ms),
            text: text.to_string(),
        });
    }
}

/// Join words the way they were sung; a trailing space is only added when the span
/// had one and the word is not hyphenated.
fn build_text(words: &[LyricsWord]) -> String {
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        out.push_str(&w.text);
        if i + 1 < words.len() && !w.text.ends_with('-') && needs_space(words, i) {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

/// Words produced from a split syllable never end with a space, so a word boundary is
/// enough; upstream's `hasTrailingSpace` decides, which is `true` for every merge
/// boundary and only `false` for the line's last span.
fn needs_space(words: &[LyricsWord], i: usize) -> bool {
    i + 1 < words.len()
}

/// Direct text of an element: its own text nodes plus non-background/non-roman spans.
fn direct_text(node: &Node) -> String {
    let mut out = String::new();
    for child in node.children() {
        if child.is_text() {
            out.push_str(child.text().unwrap_or_default());
        } else if child.is_element()
            && child.has_tag_name("span")
            && !matches!(
                get_attr(&child, "role"),
                Some("x-bg") | Some("x-translation") | Some("x-roman")
            )
        {
            out.push_str(&text_content(&child));
        }
    }
    out
}

/// `textContent` — concatenation of every descendant text node.
fn text_content(node: &Node) -> String {
    node.descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect()
}

/// Post-process word timings: sort by start, then give any word whose `end` is not
/// after its `start` (a span with no `end`) the next word's start or a 500 ms tail.
///
/// Timing attributes are never rewritten — overlapping spans are legitimate.
fn line_ends(words: &mut [LyricsWord]) {
    words.sort_by_key(|w| w.start_ms);
    for i in 0..words.len() {
        if words[i].end_ms <= words[i].start_ms {
            let next_start = words.get(i + 1).map_or(0, |w| w.start_ms);
            let floor = words[i].start_ms.saturating_add(DEFAULT_WORD_TAIL_MS);
            words[i].end_ms = floor.max(next_start);
        }
    }
}

/// `ttm:agent` / plain `agent` / metadata-namespace `agent`.
fn get_attr<'a>(node: &Node<'a, 'a>, local: &str) -> Option<&'a str> {
    attr_ns(node, META_NS, local).or_else(|| node.attribute(local))
}

/// Timing attributes may be unprefixed or in the TTML parameter namespace.
fn timing_attr<'a>(node: &Node<'a, 'a>, local: &str) -> Option<&'a str> {
    node.attribute(local)
        .or_else(|| attr_ns(node, PARAM_NS, local))
}

fn attr_ns<'a>(node: &Node<'a, 'a>, ns: &str, local: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == local && a.namespace() == Some(ns))
        .map(|a| a.value())
}

/// Some exports omit the `<p begin>`; the earliest child `<span begin>` is the line.
fn find_first_span_begin<'a>(p: &Node<'a, 'a>) -> Option<&'a str> {
    p.children()
        .filter(|n| n.is_element() && n.has_tag_name("span"))
        .filter_map(|span| timing_attr(&span, "begin"))
        .min_by(|a, b| parse_time(a).total_cmp(&parse_time(b)))
}

fn seconds(v: f64) -> u64 {
    (v.max(0.0) * 1000.0).round() as u64
}

/// TTML clock: `hh:mm:ss.f`, `mm:ss.f`, `12.5s`, `2000ms`, `3m`.
fn parse_time(time: &str) -> f64 {
    let t = time.trim();
    if t.contains(':') {
        let parts: Vec<&str> = t.split(':').collect();
        let num = |s: &str| s.trim().parse::<f64>().unwrap_or(0.0);
        return match parts.as_slice() {
            [m, s] => num(m) * 60.0 + num(s),
            [h, m, s] => num(h) * 3600.0 + num(m) * 60.0 + num(s),
            _ => 0.0,
        };
    }
    if let Some(ms) = t.strip_suffix("ms") {
        return ms.parse::<f64>().unwrap_or(0.0) / 1000.0;
    }
    for (suffix, scale) in [("h", 3600.0), ("m", 60.0), ("s", 1.0)] {
        if let Some(v) = t.strip_suffix(suffix) {
            return v.parse::<f64>().unwrap_or(0.0) * scale;
        }
    }
    t.parse().unwrap_or(0.0)
}

/// Serialize parsed lines back to the LRC flavor the app consumes: `{agent:vN}`,
/// `{bg}` and a following `<word:start:end|...>` line for word timings.
///
/// Agent ids are remapped in document order — explicit `v1`/`v2`/`v1000` are kept, any
/// other agent takes the first free `v1`/`v2` slot (`v1000` shares `v2` with a
/// primary vocalist).
pub fn to_lrc(lines: &[LyricsLine]) -> String {
    let mut map: std::collections::BTreeMap<String, String> = Default::default();
    for line in lines {
        let Some(raw) = line.agent.as_ref().map(|a| a.to_lowercase()) else {
            continue;
        };
        if matches!(raw.as_str(), "v1" | "v2" | "v1000") {
            map.insert(raw.clone(), raw);
        }
    }
    let mut next = 1;
    for line in lines {
        let Some(raw) = line.agent.as_ref().map(|a| a.to_lowercase()) else {
            continue;
        };
        if map.contains_key(&raw) {
            continue;
        }
        while next <= 2
            && (map.contains_key(&format!("v{next}"))
                || map.values().any(|v| v == &format!("v{next}")))
        {
            next += 1;
        }
        map.insert(
            raw,
            if next <= 2 {
                format!("v{next}")
            } else {
                "v1".into()
            },
        );
    }
    if map.contains_key("v1000") && map.contains_key("v1") {
        map.insert("v1000".into(), "v2".into());
    }

    let has_background = lines.iter().any(|l| l.background);
    let multi = map.len() > 1
        || (map.len() == 1 && !map.contains_key("v1"))
        || (has_background && map.len() == 1 && map.contains_key("v1"));

    let mut out = String::new();
    let mut last_bg = false;
    for line in lines {
        let agent_id = line
            .agent
            .as_ref()
            .map(|a| a.to_lowercase())
            .and_then(|a| map.get(&a).cloned());

        // `{bg}` only on the first of consecutive background lines.
        let tag = if line.background {
            if last_bg {
                None
            } else {
                Some("{bg}".to_string())
            }
        } else if multi {
            agent_id.map(|id| format!("{{agent:{id}}}"))
        } else {
            None
        };

        out.push_str(&format_lrc_time(line.start_ms));
        out.push_str(tag.as_deref().unwrap_or_default());
        out.push_str(&line.text);
        out.push('\n');
        emit_words(&mut out, line);
        last_bg = line.background;
    }
    out
}

fn emit_words(out: &mut String, line: &LyricsLine) {
    if line.words.is_empty() {
        return;
    }
    out.push('<');
    for (i, w) in line.words.iter().enumerate() {
        if i > 0 {
            out.push('|');
        }
        out.push_str(&w.text);
        out.push(':');
        out.push_str(&trim_seconds(w.start_ms));
        out.push(':');
        out.push_str(&trim_seconds(w.end_ms));
    }
    out.push_str(">\n");
}

fn format_lrc_time(ms: u64) -> String {
    format!(
        "[{:02}:{:02}.{:02}]",
        ms / 60_000,
        (ms % 60_000) / 1000,
        (ms % 1000) / 10
    )
}

/// `10.5` seconds, trailing zeros trimmed (`10.0` stays `10.0` like upstream's `Double`).
fn trim_seconds(ms: u64) -> String {
    let mut s = format!("{:.3}", ms as f64 / 1000.0);
    while s.ends_with('0') && !s.ends_with(".0") {
        s.pop();
    }
    s
}
