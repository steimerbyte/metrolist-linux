//! Devanagari → Latin transliteration, ported from upstream `romanizeHindi`
//! (`LyricsUtils.kt`): the app's own 87-row table, so a line reads exactly as it does
//! on Android.
//!
//! A hand-rolled table beats a transliteration crate here: `lipilekhika` was the
//! closest maintained option, but it is `no_std`, adds ~20 transitive crates, emits
//! diacritics (`namastē`) and transliterates `ॐ` as `āūṁ` — none of which match
//! upstream. Pure and total: a two-character row is tried, then one character, then
//! the character passes through.
//!
//! Known upstream limitation, kept for parity: the inherent schwa is not emitted, so
//! `नमस्ते` romanizes to `nmste`, not `namaste`. Fixing that needs a syllable engine
//! with schwa deletion; `lipilekhika` (MIT) is the maintained candidate if the app
//! ever changes its output.

/// Try the longest row first; `ROWS` holds the eight consonant+nukta digraphs ahead of
/// the single-character rows.
const ROWS: &[(&str, &str)] = &[
    ("क़", "q"),
    ("ख़", "kh"),
    ("ग़", "g"),
    ("ज़", "z"),
    ("ड़", "r"),
    ("ढ़", "rh"),
    ("फ़", "f"),
    ("य़", "y"),
    ("अ", "a"),
    ("आ", "aa"),
    ("इ", "i"),
    ("ई", "ee"),
    ("उ", "u"),
    ("ऊ", "oo"),
    ("ऋ", "ri"),
    ("ए", "e"),
    ("ऐ", "ai"),
    ("ओ", "o"),
    ("औ", "au"),
    ("क", "k"),
    ("ख", "kh"),
    ("ग", "g"),
    ("घ", "gh"),
    ("ङ", "ng"),
    ("च", "ch"),
    ("छ", "chh"),
    ("ज", "j"),
    ("झ", "jh"),
    ("ञ", "ny"),
    ("ट", "t"),
    ("ठ", "th"),
    ("ड", "d"),
    ("ढ", "dh"),
    ("ण", "n"),
    ("त", "t"),
    ("थ", "th"),
    ("द", "d"),
    ("ध", "dh"),
    ("न", "n"),
    ("प", "p"),
    ("फ", "ph"),
    ("ब", "b"),
    ("भ", "bh"),
    ("म", "m"),
    ("य", "y"),
    ("र", "r"),
    ("ल", "l"),
    ("व", "v"),
    ("श", "sh"),
    ("ष", "sh"),
    ("स", "s"),
    ("ह", "h"),
    ("ा", "aa"),
    ("ि", "i"),
    ("ी", "ee"),
    ("ु", "u"),
    ("ू", "oo"),
    ("ृ", "ri"),
    ("े", "e"),
    ("ै", "ai"),
    ("ो", "o"),
    ("ौ", "au"),
    ("ं", "n"),
    ("ः", "h"),
    ("ँ", "n"),
    ("़", ""),
    ("्", ""),
    ("०", "0"),
    ("१", "1"),
    ("२", "2"),
    ("३", "3"),
    ("४", "4"),
    ("५", "5"),
    ("६", "6"),
    ("७", "7"),
    ("८", "8"),
    ("९", "9"),
    ("ॐ", "Om"),
    ("ऽ", ""),
    ("क़", "q"),
    ("ख़", "kh"),
    ("ग़", "g"),
    ("ज़", "z"),
    ("ड़", "r"),
    ("ढ़", "rh"),
    ("फ़", "f"),
    ("य़", "y"),
];

/// Romanize Devanagari text; non-Devanagari characters pass through unchanged.
pub fn romanize(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        // A nukta row is always a base consonant followed by U+093C, so the pair is
        // only looked for when the next character is actually a nukta.
        if let Some(next) = chars.get(i + 1).filter(|c| **c == '\u{093C}') {
            let key = [chars[i], *next].iter().collect::<String>();
            if let Some((_, latin)) = ROWS.iter().find(|(from, _)| *from == key) {
                out.push_str(latin);
                i += 2;
                continue;
            }
        }
        let key = chars[i].to_string();
        match ROWS.iter().find(|(from, _)| *from == key) {
            Some((_, latin)) => out.push_str(latin),
            None => out.push(chars[i]),
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::romanize;

    /// Expected values are upstream's, not pronunciation guides: the table maps only
    /// explicit vowel signs, so the inherent schwa never appears.
    #[test]
    fn words_transliterate_like_upstream() {
        assert_eq!(
            romanize("\u{928}\u{92E}\u{938}\u{94D}\u{924}\u{947}"),
            "nmste"
        );
        assert_eq!(romanize("\u{92A}\u{94D}\u{92F}\u{93E}\u{930}"), "pyaar");
        assert_eq!(romanize("\u{926}\u{93F}\u{932}"), "dil");
        assert_eq!(romanize("\u{917}\u{940}\u{924}"), "geet");
        assert_eq!(romanize("\u{926}\u{93F}\u{932} \u{939}\u{948}"), "dil hai");
    }

    #[test]
    fn nukta_pairs_and_digits() {
        // Precomposed (U+0958 क़) and decomposed (क + nukta) spellings agree.
        assert_eq!(romanize("\u{958}"), "q");
        assert_eq!(romanize("\u{915}\u{93C}"), "q");
        assert_eq!(romanize("\u{95B}"), "z");
        assert_eq!(romanize("\u{966}\u{967}\u{968}"), "012");
    }

    #[test]
    fn non_devanagari_text_passes_through() {
        assert_eq!(romanize("hello"), "hello");
        assert_eq!(romanize(""), "");
        assert_eq!(
            romanize("\u{928}\u{92E}\u{938}\u{94D}\u{924}\u{947}, Hello 123"),
            "nmste, Hello 123"
        );
        assert_eq!(romanize("\u{0}\u{1}"), "\u{0}\u{1}");
    }

    /// Every scalar value is total: no panic, and unrelated scripts are untouched.
    #[test]
    fn every_char_is_total() {
        for c in ['\u{900}', '\u{97F}', '\u{93C}', '\u{4E00}', '\u{10FFFF}'] {
            let _ = romanize(&c.to_string());
        }
        assert_eq!(romanize("\u{930}\u{94D}\u{907}"), "ri");
        assert_eq!(romanize("\u{4E2D}\u{6587}"), "\u{4E2D}\u{6587}");
    }
}
