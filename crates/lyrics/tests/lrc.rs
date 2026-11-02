//! Port of upstream `LyricsBackgroundTimingTest` (6 cases) plus LRC parsing edge cases
//! from `LyricsUtils` and the playback helpers.

use ml_lyrics::{active_line_at, active_lines_at, active_word_at, parse_lrc, parse_lrc_with_meta};

fn parse(text: &str) -> Vec<ml_core::LyricsLine> {
    parse_lrc(text).expect("LRC never hard-fails").lines
}

fn main_lines(text: &str) -> Vec<ml_core::LyricsLine> {
    parse(text).into_iter().filter(|l| !l.background).collect()
}

// ---------------------------------------------------------------- background timing

const LYRICS_WITH_BACKGROUND: &str = "[00:10.00]<00:10.00>Main <00:10.50>line
[bg: <00:10.20>ooh<00:11.00>]
[00:12.00]<00:12.00>Next <00:12.40>line";

#[test]
fn last_word_does_not_end_before_it_starts() {
    let lines = main_lines(LYRICS_WITH_BACKGROUND);
    let last_word = lines[0].words.last().expect("word timings");
    assert_eq!(last_word.text, "line");
    assert!(
        last_word.end_ms > last_word.start_ms,
        "ends at {} but starts at {}",
        last_word.end_ms,
        last_word.start_ms
    );
}

#[test]
fn last_word_runs_to_the_next_main_line() {
    let lines = main_lines(LYRICS_WITH_BACKGROUND);
    assert_eq!(lines[0].words.last().unwrap().end_ms, 12_000);
}

#[test]
fn background_line_keeps_its_own_timings() {
    let background: Vec<_> = parse(LYRICS_WITH_BACKGROUND)
        .into_iter()
        .filter(|l| l.background)
        .collect();
    assert_eq!(background.len(), 1);
    assert_eq!(background[0].text, "ooh");
    assert_eq!(background[0].start_ms, 10_200);
    assert_eq!(background[0].words[0].start_ms, 10_200);
}

#[test]
fn bracketed_bg_spelling_is_skipped_too() {
    let brace_style = "[00:10.00]<00:10.00>Main <00:10.50>line
[00:10.20]{bg}<00:10.20>ooh
[00:12.00]<00:12.00>Next <00:12.40>line";
    let lines = main_lines(brace_style);
    let last_word = lines[0].words.last().unwrap();
    assert!(last_word.end_ms > last_word.start_ms);
    assert_eq!(last_word.end_ms, 12_000);
}

#[test]
fn background_with_continuation_line_is_skipped_whole() {
    let with_continuation = "[00:10.00]<00:10.00>Main <00:10.50>line
[bg: ooh]
<ooh:10.2:11.0>
[00:12.00]<00:12.00>Next <00:12.40>line";
    let all = parse(with_continuation);
    let last_word = all
        .iter()
        .find(|l| !l.background)
        .unwrap()
        .words
        .last()
        .unwrap();
    assert_eq!(last_word.text, "line");
    assert_eq!(last_word.end_ms, 12_000);

    let background: Vec<_> = all.into_iter().filter(|l| l.background).collect();
    assert_eq!(background.len(), 1);
    assert_eq!(background[0].text, "ooh");
    assert_eq!(background[0].words[0].start_ms, 10_200);
}

#[test]
fn plain_line_pair_is_unaffected() {
    let plain = "[00:10.00]<00:10.00>Main <00:10.50>line
[00:12.00]<00:12.00>Next <00:12.40>line";
    let lines = main_lines(plain);
    assert_eq!(lines[0].words.last().unwrap().end_ms, 12_000);
}

// ------------------------------------------------------------------- LRC edge cases

#[test]
fn multiple_timestamps_duplicate_the_line() {
    let lines = parse("[00:10.00][00:20.00][01:00.00]chorus");
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines.iter().map(|l| l.start_ms).collect::<Vec<_>>(),
        vec![10_000, 20_000, 60_000]
    );
    assert!(lines.iter().all(|l| l.text == "chorus"));
}

#[test]
fn hundredths_and_thousandths_both_parse() {
    let lines = parse("[00:01.50]a\n[00:02.005]b\n[00:03.5]c");
    assert_eq!(lines[0].start_ms, 1_500);
    assert_eq!(lines[1].start_ms, 2_005);
    assert_eq!(lines[2].start_ms, 3_500);
}

#[test]
fn metadata_tags_are_read_and_offset_applied() {
    let text = "[ti:Song]\n[ar:Artist]\n[al:Album]\n[by:Uploader]\n[length:03:21]\n[offset:+500]\n[00:10.00]line";
    let parsed = parse_lrc_with_meta(text).unwrap();
    assert_eq!(parsed.meta.title.as_deref(), Some("Song"));
    assert_eq!(parsed.meta.artist.as_deref(), Some("Artist"));
    assert_eq!(parsed.meta.album.as_deref(), Some("Album"));
    assert_eq!(parsed.meta.author.as_deref(), Some("Uploader"));
    assert_eq!(parsed.meta.length.as_deref(), Some("03:21"));
    assert_eq!(parsed.meta.offset_ms, 500);
    // Positive offset shows lyrics earlier: 10.00 s - 0.5 s.
    assert_eq!(parsed.doc.lines[0].start_ms, 9_500);
}

#[test]
fn negative_offset_shifts_later_and_clamps_at_zero() {
    // `[offset:n]` is `displayed = timestamp - n`, so a negative n delays the lyrics.
    let text = "[offset:-1000]\n[00:00.20]first\n[00:01.00]second";
    let lines = parse(text);
    assert_eq!(lines[0].start_ms, 1_200);
    assert_eq!(lines[1].start_ms, 2_000);

    // Pulling past the start of the track clamps at zero instead of wrapping.
    let clamped = parse("[offset:+5000]\n[00:01.00]early");
    assert_eq!(clamped[0].start_ms, 0);
}

#[test]
fn unsorted_input_is_sorted_and_empties_skipped() {
    let lines = parse("[00:30.00]third\n\n[00:10.00]first\n[00:20.00]second\n\n");
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        vec!["first", "second", "third"]
    );
}

#[test]
fn duplicate_timestamps_keep_document_order() {
    let lines = parse("[00:10.00]far\n[00:10.00]near");
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        vec!["far", "near"]
    );
}

#[test]
fn malformed_lines_are_skipped_not_fatal() {
    let lines = parse("not a timestamp\n[99:99.99]bad seconds\n[00:01.00]good\n[x:y.z]bad");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "good");
    // Empty document and whitespace are fine too.
    assert!(parse("").is_empty());
    assert!(parse("   \n\n").is_empty());
}

#[test]
fn escaped_newlines_and_entities_are_decoded() {
    let lines = parse(r"[00:01.00]rock &amp; roll\n[00:02.00]line two");
    assert_eq!(lines[0].text, "rock & roll");
    assert_eq!(lines[1].text, "line two");
}

#[test]
fn plain_lrc_without_words_has_line_ends() {
    let lines = parse("[00:01.00]one\n[00:02.00]two");
    assert_eq!(lines[0].end_ms, Some(2_000));
    assert_eq!(lines[1].end_ms, None);
}

#[test]
fn agent_markers_are_parsed_and_inherited() {
    let lines = parse("[00:01.00]{agent:v1}main\n[00:02.00]{bg}background");
    assert_eq!(lines[0].agent.as_deref(), Some("v1"));
    assert!(lines[1].background);
    assert_eq!(lines[1].agent.as_deref(), Some("v1"));
}

#[test]
fn paxsenix_agent_prefix_is_parsed() {
    let lines = parse("[00:00.000]v2: <00:00.000>hello <00:00.500>there");
    assert_eq!(lines[0].agent.as_deref(), Some("v2"));
    assert_eq!(lines[0].text, "hello there");
    assert_eq!(lines[0].words.len(), 2);
}

// ----------------------------------------------------------------------- helpers

#[test]
fn active_line_lookup_follows_the_100ms_threshold() {
    let doc = parse_lrc("[00:01.00]one\n[00:05.00]two\n[00:09.00]three").unwrap();
    assert_eq!(active_line_at(&doc, 0), None);
    assert_eq!(active_line_at(&doc, 900), Some(0));
    assert_eq!(active_line_at(&doc, 1_000), Some(0));
    assert_eq!(active_line_at(&doc, 4_950), Some(1));
    assert_eq!(active_line_at(&doc, 60_000), Some(2));
}

#[test]
fn active_word_lookup_walks_word_starts() {
    let doc = parse_lrc("[00:01.00]<00:01.00>one <00:02.00>two").unwrap();
    let line = &doc.lines[0];
    assert_eq!(active_word_at(line, 500), None);
    assert_eq!(active_word_at(line, 1_000), Some(0));
    assert_eq!(active_word_at(line, 1_900), Some(0));
    assert_eq!(active_word_at(line, 2_000), Some(1));
    assert_eq!(active_word_at(line, 99_000), Some(1));
}

#[test]
fn active_lines_keep_background_alongside_main() {
    let doc = parse_lrc(LYRICS_WITH_BACKGROUND).unwrap();
    let active = active_lines_at(&doc, 10_600);
    let texts: Vec<&str> = active.iter().map(|&i| doc.lines[i].text.as_str()).collect();
    assert!(texts.contains(&"Main line"), "{texts:?}");
    assert!(texts.contains(&"ooh"), "{texts:?}");
}

#[test]
fn simultaneous_plain_lines_keep_only_the_latest() {
    let doc = parse_lrc("[00:01.00]first\n[00:01.50]second\n[00:09.00]third").unwrap();
    let active = active_lines_at(&doc, 1_600);
    assert_eq!(active, vec![1]);
}

#[test]
fn leading_credit_block_is_dropped() {
    // KuGou-style header: a `Title - Artist` line, then `word: value` credits.
    let text = "[00:00.29]Never Gonna Give You Up - Rick Astley
[00:01.09]Lyrics by：Mike Stock
[00:02.15]Composed by：Peter Waterman
[00:19.31]We're no strangers to love
[00:23.03]You know the rules and so do I";
    let mut lines = parse(text);
    ml_lyrics::lrc::filter_credits(&mut lines);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0].text, "We're no strangers to love");
}

#[test]
fn a_colon_inside_a_lyric_line_survives() {
    // Only the head of the document is a credit block.
    let text = "[00:01.00]verse one
[00:05.00]she said: hello
[00:09.00]and left";
    let mut lines = parse(text);
    ml_lyrics::lrc::filter_credits(&mut lines);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[1].text, "she said: hello");
}
