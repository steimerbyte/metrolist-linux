//! Internal consistency checks for the parser: the invariants the Verus proof lead will
//! freeze into spec/exec pairs. Everything here is pure and total — no I/O, no panics on
//! any input.

#[cfg(test)]
mod tests {
    use crate::{active_line_index, active_line_indices, active_word_index, parse_lrc, parse_ttml};
    use ml_core::{LyricsDocument, LyricsLine, LyricsWord};

    fn doc(lines: impl IntoIterator<Item = LyricsLine>) -> LyricsDocument {
        LyricsDocument {
            provider: "test".into(),
            lines: lines.into_iter().collect(),
            ..Default::default()
        }
    }

    fn line(start_ms: u64, end_ms: Option<u64>, background: bool) -> LyricsLine {
        LyricsLine {
            start_ms,
            end_ms,
            text: format!("line@{start_ms}"),
            background,
            ..Default::default()
        }
    }

    /// `parse_lrc` output is sorted, and every derived end is after its start.
    #[test]
    fn lrc_output_is_sorted_with_monotonic_ends() {
        let parsed = parse_lrc("[00:20.00]b\n[00:10.00]<00:10.00>a <00:11.00>z\n[00:20.00]c")
            .expect("LRC parsing never fails");
        let starts: Vec<u64> = parsed.lines.iter().map(|l| l.start_ms).collect();
        assert!(starts.windows(2).all(|w| w[0] <= w[1]), "{starts:?}");
        for line in &parsed.lines {
            if let Some(end) = line.end_ms {
                assert!(end > line.start_ms, "{line:?}");
            }
            for word in &line.words {
                assert!(word.end_ms >= word.start_ms, "{word:?}");
            }
        }
    }

    /// The active set is ordered and never reaches past `active_line_index`'s prefix.
    ///
    /// It is deliberately *not* monotonic in time: a line with an explicit end stops
    /// being active again once it has finished.
    #[test]
    fn active_line_selection_is_ordered_and_prefix_bounded() {
        let d = doc([line(1_000, Some(2_000), false), line(3_000, None, false)]);
        assert_eq!(active_line_index(&d.lines, 0), None);
        assert_eq!(active_line_index(&d.lines, 1_000), Some(0));
        assert_eq!(active_line_index(&d.lines, 3_000), Some(1));
        assert_eq!(active_line_index(&d.lines, u64::MAX), Some(1));

        for position in (0..6_000).step_by(250) {
            let active = active_line_indices(&d.lines, position);
            assert!(active.windows(2).all(|w| w[0] < w[1]), "{active:?}");
            let last = active_line_index(&d.lines, position);
            for index in &active {
                assert!(Some(*index) <= last, "{active:?} vs {last:?}");
            }
        }
        // The line with an explicit end goes inactive again; the open-ended one does not.
        assert!(active_line_indices(&d.lines, 2_500).is_empty());
        assert_eq!(active_line_indices(&d.lines, 99_000), vec![1]);
    }

    /// Selecting words never runs past the last word, and never before the first.
    #[test]
    fn active_word_selection_is_bounded() {
        let words = vec![
            LyricsWord {
                start_ms: 1_000,
                end_ms: 1_500,
                text: "a".into(),
            },
            LyricsWord {
                start_ms: 2_000,
                end_ms: 2_500,
                text: "b".into(),
            },
        ];
        assert_eq!(active_word_index(&words, 0), None);
        assert_eq!(active_word_index(&words, 1_000), Some(0));
        assert_eq!(active_word_index(&words, u64::MAX), Some(1));
        assert_eq!(active_word_index(&[], 5_000), None);
    }

    /// A background line never terminates the main line above it.
    #[test]
    fn background_lines_do_not_end_their_main_line() {
        let parsed = parse_lrc(
            "[00:10.00]<00:10.00>Main <00:10.50>line\n[bg: <00:10.20>ooh]\n[00:12.00]Next",
        )
        .expect("LRC parsing never fails");
        let main = &parsed.lines[0];
        assert_eq!(main.words.last().unwrap().end_ms, 12_000);
        assert!(parsed.lines.iter().any(|l| l.background));
    }

    /// A hostile document is skipped line by line, never fatal and never panicking.
    #[test]
    fn malformed_documents_are_total() {
        for text in [
            "",
            "\u{0}\u{0}",
            "[99:99.99]bad\n[xx:yy.zz]worse\n[00:01.00]ok",
            "[[[[[]]]]]",
            "[offset:not-a-number]\n[00:01.00]x",
            "\n\n\n",
        ] {
            let parsed = parse_lrc(text).expect("parse_lrc is total");
            assert!(parsed
                .lines
                .windows(2)
                .all(|w| w[0].start_ms <= w[1].start_ms));
        }

        for ttml in ["", "<tt>", "not xml", "<tt><body><div><p><span>no timing"] {
            let _ = parse_ttml(ttml);
        }
    }
}
