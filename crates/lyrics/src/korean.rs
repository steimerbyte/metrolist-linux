//! Hangul → Revised Romanization, transcribed from upstream `HANGUL_ROMAJA_MAP`
//! (`LyricsUtils.kt`) so the port reads Korean exactly like the Android app.
//!
//! A syllable is decomposed arithmetically (`U+AC00` block: 19 initials × 21 vowels ×
//! 28 finals); the final consonant is *held back* until the next syllable is known,
//! because it is resyllabified across the boundary (`한국어` → `hangugeo`, not
//! `hangukeo`). Pure and total: no indexing that can fail, no allocation beyond output.

/// 19 initial consonants (choseong), in Unicode order (`U+1100`+i).
const CHO: [&str; 19] = [
    "g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p",
    "h",
];

/// 21 medial vowels (jungseong), in Unicode order (`U+1161`+i).
const JUNG: [&str; 21] = [
    "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we",
    "wi", "yu", "eu", "eui", "i",
];

/// Final consonants (jongseong) on their own; index = jongseong number, `0` = none.
const JONG: [Option<&str>; 28] = [
    None,
    Some("k"),
    Some("kk"),
    Some("k"),
    Some("n"),
    Some("n"),
    Some("n"),
    Some("t"),
    Some("l"),
    Some("k"),
    Some("m"),
    Some("p"),
    Some("t"),
    Some("t"),
    Some("p"),
    Some("l"),
    Some("m"),
    Some("p"),
    Some("p"),
    Some("t"),
    Some("t"),
    Some("ng"),
    Some("t"),
    Some("t"),
    Some("k"),
    Some("t"),
    Some("p"),
    Some("t"),
];

/// A final consonant followed by the next syllable's initial: `(jong, cho)` → surface.
///
/// Sorted by key so the lookup is a binary search. Transcribed from upstream
/// `HANGUL_ROMAJA_MAP["jong"]`; its `"ᆬㅎ"` row is dropped because it mixes a
/// compatibility jamo (`ㅎ`) into a conjoining-jamo key, so it can never match.
const JONG_BEFORE_CHO: [((u8, u8), &str); 117] = [
    ((1, 2), "ngn"),
    ((1, 5), "ngn"),
    ((1, 6), "ngm"),
    ((1, 11), "g"),
    ((1, 18), "kh"),
    ((2, 2), "ngn"),
    ((2, 5), "ngn"),
    ((2, 6), "ngm"),
    ((2, 11), "kg"),
    ((2, 18), "kh"),
    ((3, 2), "ngn"),
    ((3, 5), "ngn"),
    ((3, 6), "ngm"),
    ((3, 11), "ks"),
    ((3, 18), "kch"),
    ((4, 5), "ll"),
    ((5, 2), "nn"),
    ((5, 5), "nn"),
    ((5, 6), "nm"),
    ((5, 11), "nj"),
    ((6, 5), "nn"),
    ((6, 11), "nh"),
    ((7, 2), "nn"),
    ((7, 5), "nn"),
    ((7, 6), "nm"),
    ((7, 11), "d"),
    ((7, 18), "th"),
    ((8, 2), "ll"),
    ((8, 5), "ll"),
    ((8, 11), "r"),
    ((9, 2), "ngn"),
    ((9, 5), "ngn"),
    ((9, 6), "ngm"),
    ((9, 11), "lg"),
    ((9, 18), "lkh"),
    ((10, 2), "mn"),
    ((10, 5), "mn"),
    ((10, 6), "mm"),
    ((10, 11), "lm"),
    ((10, 18), "lmh"),
    ((11, 2), "mn"),
    ((11, 5), "mn"),
    ((11, 6), "mm"),
    ((11, 11), "lb"),
    ((11, 18), "lph"),
    ((12, 2), "nn"),
    ((12, 5), "nn"),
    ((12, 6), "nm"),
    ((12, 11), "ls"),
    ((12, 18), "lsh"),
    ((13, 2), "nn"),
    ((13, 5), "nn"),
    ((13, 6), "nm"),
    ((13, 11), "lt"),
    ((13, 18), "lth"),
    ((14, 2), "mn"),
    ((14, 5), "mn"),
    ((14, 6), "mm"),
    ((14, 11), "lp"),
    ((14, 18), "lph"),
    ((15, 2), "ll"),
    ((15, 5), "ll"),
    ((15, 6), "lm"),
    ((15, 11), "lh"),
    ((15, 18), "lh"),
    ((16, 5), "mn"),
    ((17, 2), "mn"),
    ((17, 5), "mn"),
    ((17, 6), "mm"),
    ((17, 11), "b"),
    ((17, 18), "ph"),
    ((18, 2), "mn"),
    ((18, 5), "mn"),
    ((18, 6), "mm"),
    ((18, 11), "ps"),
    ((18, 18), "psh"),
    ((19, 2), "nn"),
    ((19, 5), "nn"),
    ((19, 6), "nm"),
    ((19, 11), "s"),
    ((19, 18), "sh"),
    ((20, 2), "tn"),
    ((20, 5), "tn"),
    ((20, 6), "nm"),
    ((20, 11), "ss"),
    ((20, 18), "th"),
    ((22, 2), "nn"),
    ((22, 5), "nn"),
    ((22, 6), "nm"),
    ((22, 11), "j"),
    ((22, 18), "ch"),
    ((23, 2), "nn"),
    ((23, 5), "nn"),
    ((23, 6), "nm"),
    ((23, 11), "ch"),
    ((23, 18), "ch"),
    ((24, 2), "ngn"),
    ((24, 5), "ngn"),
    ((24, 6), "ngm"),
    ((24, 11), "k"),
    ((24, 18), "kh"),
    ((25, 2), "nn"),
    ((25, 5), "nn"),
    ((25, 6), "nm"),
    ((25, 11), "t"),
    ((25, 18), "th"),
    ((26, 2), "mn"),
    ((26, 5), "mn"),
    ((26, 6), "mm"),
    ((26, 11), "p"),
    ((26, 18), "ph"),
    ((27, 0), "k"),
    ((27, 2), "nn"),
    ((27, 5), "nn"),
    ((27, 6), "mm"),
    ((27, 11), "h"),
    ((27, 18), "t"),
];

/// Decompose a precomposed syllable into `(initial, vowel, final)`, `0` = none.
fn parts(c: char) -> Option<(usize, usize, usize)> {
    let index = (c as u32).checked_sub(0xAC00)? as usize;
    (index < 19 * 21 * 28).then_some((index / 588, (index % 588) / 28, index % 28))
}

fn jong_alone(jong: usize) -> &'static str {
    JONG.get(jong).copied().flatten().unwrap_or("")
}

/// The final consonant as it is spoken before `cho` (resyllabification/liaison).
fn jong_before_cho(jong: usize, cho: usize) -> Option<&'static str> {
    let key = (jong as u8, cho as u8);
    JONG_BEFORE_CHO
        .binary_search_by(|(k, _)| k.cmp(&key))
        .ok()
        .and_then(|i| JONG_BEFORE_CHO.get(i))
        .map(|(_, surface)| *surface)
}

/// Romanize Korean text; non-Hangul characters pass through unchanged.
pub fn romanize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut held_final: Option<usize> = None;
    for c in text.chars() {
        let Some((cho, jung, jong)) = parts(c) else {
            if let Some(previous) = held_final.take() {
                out.push_str(jong_alone(previous));
            }
            out.push(c);
            continue;
        };
        let mut spoke_initial = false;
        if let Some(previous) = held_final.take() {
            match jong_before_cho(previous, cho) {
                Some(surface) => {
                    out.push_str(surface);
                    spoke_initial = true;
                }
                None => out.push_str(jong_alone(previous)),
            }
        }
        if !spoke_initial {
            out.push_str(CHO.get(cho).copied().unwrap_or(""));
        }
        out.push_str(JUNG.get(jung).copied().unwrap_or(""));
        if jong != 0 {
            held_final = Some(jong);
        }
    }
    if let Some(previous) = held_final {
        out.push_str(jong_alone(previous));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::romanize;

    #[test]
    fn syllables_resyllabify_across_the_boundary() {
        assert_eq!(romanize("한국어"), "hangugeo");
        assert_eq!(romanize("한국"), "hanguk");
        assert_eq!(romanize("국내"), "gungnae");
        assert_eq!(romanize("신라"), "silla");
        assert_eq!(romanize("물리"), "mulli");
        assert_eq!(romanize("좋아"), "joha");
        assert_eq!(romanize("앉아"), "anja");
        assert_eq!(romanize("없어"), "eopseo");
        assert_eq!(romanize("각하"), "gakha");
        assert_eq!(romanize("읽는"), "ingneun");
    }

    #[test]
    fn common_lines_and_passthrough() {
        assert_eq!(romanize("안녕하세요"), "annyeonghaseyo");
        assert_eq!(romanize("사랑해"), "saranghae");
        assert_eq!(romanize("감사합니다"), "gamsahamnida");
        assert_eq!(romanize("서울"), "seoul");
        assert_eq!(romanize("꽃"), "kkot");
        assert_eq!(romanize("좋은 아침"), "joheun achim");
        assert_eq!(romanize("안녕 Hi 123"), "annyeong Hi 123");
        assert_eq!(romanize(""), "");
    }

    /// Every code point is total: no panics, and non-Hangul text is untouched.
    #[test]
    fn every_char_is_total() {
        for c in ['\u{0}', 'a', '한', '\u{D7A3}', '\u{10FFFF}', 'ᄀ', 'ㅎ'] {
            let _ = romanize(&c.to_string());
        }
        for text in ["", " ", "\u{AC00}", "\u{D7A3}\u{AC00}"] {
            let _ = romanize(text);
        }
    }
}
