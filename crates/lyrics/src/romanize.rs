//! Romanization: one script detected per document, then applied line by line.
//!
//! Four scripts are ported, each behind the same total `romanize_detect` entry point:
//!
//! * Cyrillic (Macedonian) — hand-built table, `romanize_macedonian`.
//! * Japanese — `kakasi` (MIT) kanji/kana dictionary, `romanize_japanese`.
//! * Chinese — `pinyin` (MIT) Hanzi dictionary, `romanize_chinese`, tone-free.
//! * Korean — Hangul arithmetic + Revised Romanization table, [`crate::korean`].
//! * Devanagari/Hindi — [`crate::hindi`] (upstream's own transliteration table).
//!
//! Language detection runs **once per document** (`romanize_document`) — reading each
//! line on its own re-detects it as Russian/Serbian and mangles `ц`/`ж`. Upstream only
//! got this right after the bug in `MacedonianRomanizationTest`.
//!
//! Every function here is pure and total: no I/O, no panics, non-target text passes
//! through unchanged, and an unknown script answers [`MlError::Unimplemented`] instead
//! of half-romanizing.

use ml_core::{LyricsDocument, LyricsLine, MlError, MlResult};

/// Script detected for a Cyrillic document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CyrillicLanguage {
    Russian,
    Ukrainian,
    Serbian,
    Bulgarian,
    Belarusian,
    Kyrgyz,
    Macedonian,
}

impl CyrillicLanguage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Russian => "Russian",
            Self::Ukrainian => "Ukrainian",
            Self::Serbian => "Serbian",
            Self::Bulgarian => "Bulgarian",
            Self::Belarusian => "Belarusian",
            Self::Kyrgyz => "Kyrgyz",
            Self::Macedonian => "Macedonian",
        }
    }
}

/// Macedonian letters, including the grave-accented `ѐ`/`ѝ` used for stressed vowels.
///
/// NOTE: upstream's uppercase row omits `Д`; restored here because `Д` is part of the
/// Macedonian alphabet and its absence made detection fail on any line starting with
/// it. Every other letter matches upstream exactly.
const MACEDONIAN_LETTERS: &str = "АБВГДЃЕЖЗЅИЈКЛЉМНЊОПРСТЌУФХЦЧЏШЀЍ\
абвгдѓежзѕијклљмнњопрстќуфхцчџшѐѝ";

/// `Ѐ`/`ѐ` and `Ѝ`/`ѝ` romanize to their plain Latin letters.
const MACEDONIAN_ROMAJI: &[(char, &str)] = &[
    ('Ѓ', "Gj"),
    ('Ѕ', "Dz"),
    ('И', "I"),
    ('Ј', "J"),
    ('Љ', "Lj"),
    ('Њ', "Nj"),
    ('Ќ', "Kj"),
    ('Џ', "Dž"),
    ('Ч', "Č"),
    ('Ш', "Sh"),
    ('Ж', "Zh"),
    ('Ц', "C"),
    ('Х', "H"),
    ('Ѐ', "E"),
    ('Ѝ', "I"),
    ('ѓ', "gj"),
    ('ѕ', "dz"),
    ('и', "i"),
    ('ј', "j"),
    ('љ', "lj"),
    ('њ', "nj"),
    ('ќ', "kj"),
    ('џ', "dž"),
    ('ч', "č"),
    ('ш', "sh"),
    ('ж', "zh"),
    ('ц', "c"),
    ('х', "h"),
    ('ѐ', "e"),
    ('ѝ', "i"),
];

/// Letters Macedonian shares with the general Cyrillic alphabet.
const GENERAL_ROMAJI: &[(char, &str)] = &[
    ('А', "A"),
    ('Б', "B"),
    ('В', "V"),
    ('Г', "G"),
    ('Д', "D"),
    ('Е', "E"),
    ('З', "Z"),
    ('К', "K"),
    ('Л', "L"),
    ('М', "M"),
    ('Н', "N"),
    ('О', "O"),
    ('П', "P"),
    ('Р', "R"),
    ('С', "S"),
    ('Т', "T"),
    ('У', "U"),
    ('Ф', "F"),
    ('а', "a"),
    ('б', "b"),
    ('в', "v"),
    ('г', "g"),
    ('д', "d"),
    ('е', "e"),
    ('з', "z"),
    ('к', "k"),
    ('л', "l"),
    ('м', "m"),
    ('н', "n"),
    ('о', "o"),
    ('п', "p"),
    ('р', "r"),
    ('с', "s"),
    ('т', "t"),
    ('у', "u"),
    ('ф', "f"),
];

fn is_cyrillic(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

/// True when every Cyrillic character in `text` belongs to the Macedonian alphabet and
/// at least one Cyrillic letter is present.
pub fn is_macedonian(text: &str) -> bool {
    let mut any = false;
    for c in text.chars() {
        if !is_cyrillic(c) {
            continue;
        }
        if !MACEDONIAN_LETTERS.contains(c) {
            return false;
        }
        any = true;
    }
    any
}

/// Every language the app knows, in upstream's detection order.
pub const ALL_LANGUAGES: [CyrillicLanguage; 7] = [
    CyrillicLanguage::Russian,
    CyrillicLanguage::Ukrainian,
    CyrillicLanguage::Serbian,
    CyrillicLanguage::Bulgarian,
    CyrillicLanguage::Belarusian,
    CyrillicLanguage::Kyrgyz,
    CyrillicLanguage::Macedonian,
];

fn alphabet(language: CyrillicLanguage) -> &'static str {
    match language {
        CyrillicLanguage::Russian => {
            "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя"
        }
        CyrillicLanguage::Ukrainian => {
            "АБВГҐДЕЄЖЗИІЇЙКЛМНОПРСТУФХЦЧШЩЬЮЯабвгґдеєжзиіїйклмнопрстуфхцчшщьюя"
        }
        CyrillicLanguage::Serbian => "АБВГДЕЖЗИЈКЛЉМНЊОПРСТЋУФХЦЧШабвгдежзијклљмнњопрстћуфхцчш",
        CyrillicLanguage::Bulgarian => {
            "АБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЬЮЯабвгдежзийклмнопрстуфхцчшщъьюя"
        }
        CyrillicLanguage::Belarusian => {
            "АБВГДЕЁЖЗІЙКЛМНОПРСТУЎФХЦЧШЫЬЭЮЯабвгдеёжзійклмнопрстуўфхцчшыьэюя"
        }
        CyrillicLanguage::Kyrgyz => {
            "АБВГДЕЁЖЗИЙКЛМНОҢОӨПРСТУҮФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмноңоөпрстуүфхцчшщъыьэюя"
        }
        CyrillicLanguage::Macedonian => MACEDONIAN_LETTERS,
    }
}

/// Detect the Cyrillic language of a whole document over `enabled`, in that order.
///
/// Call once per document and reuse the answer for every line: re-detecting per line
/// reads a Macedonian line as Russian and turns `ц` into `ts`.
pub fn detect_cyrillic_language(
    text: &str,
    enabled: &[CyrillicLanguage],
) -> Option<CyrillicLanguage> {
    if !text.chars().any(is_cyrillic) {
        return None;
    }
    enabled
        .iter()
        .copied()
        .find(|language| spoken_as(text, alphabet(*language)))
}

/// `text` uses only letters from `alphabet` (non-Cyrillic characters ignored).
fn spoken_as(text: &str, alphabet: &str) -> bool {
    let mut any = false;
    for c in text.chars() {
        if !is_cyrillic(c) {
            continue;
        }
        if !alphabet.contains(c) {
            return false;
        }
        any = true;
    }
    any
}

/// Map every Cyrillic character to Latin; unknown characters pass through unchanged.
///
/// Pure and total: only `char` lookups, no indexing that can fail.
pub fn romanize_macedonian(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match lookup(MACEDONIAN_ROMAJI, c).or_else(|| lookup(GENERAL_ROMAJI, c)) {
            Some(latin) => out.push_str(latin),
            None => out.push(c),
        }
    }
    out
}

fn lookup(table: &[(char, &'static str)], c: char) -> Option<&'static str> {
    table.iter().find(|(from, _)| *from == c).map(|(_, to)| *to)
}

/// Romanize one line with an already-detected Cyrillic language.
///
/// Only Macedonian is implemented; the other branches are honest stubs because their
/// case tables are not ported yet.
pub fn romanize_line(language: CyrillicLanguage, line: &str) -> MlResult<String> {
    match language {
        CyrillicLanguage::Macedonian => Ok(romanize_macedonian(line)),
        other => Err(MlError::unimplemented(format!(
            "romanization for {}",
            other.name()
        ))),
    }
}

/// Every script [`romanize_detected`] can name.
///
/// `ALL_SCRIPTS` is ordered Japanese → Korean → Chinese → Hindi → Cyrillic, but the
/// first-wins order only matters for Japanese vs Chinese: [`speaks`] asks Japanese for
/// *kana*, never Hanzi, so a Hanzi-only document is Chinese even with Japanese enabled
/// (the same rule as upstream `isChinese`), and a document with any kana is Japanese.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Japanese,
    Korean,
    Chinese,
    Hindi,
    Cyrillic(CyrillicLanguage),
}

impl Script {
    pub fn name(self) -> &'static str {
        match self {
            Self::Japanese => "Japanese",
            Self::Korean => "Korean",
            Self::Chinese => "Chinese",
            Self::Hindi => "Hindi",
            Self::Cyrillic(language) => language.name(),
        }
    }
}

/// Detect the script of a whole document (never per line), in upstream's order.
///
/// `enabled` is the caller's allow-list: a script missing from it is never returned,
/// which keeps a document of shared letters from being read with the wrong table.
pub fn detect_script(text: &str, enabled: &[Script]) -> Option<Script> {
    enabled.iter().copied().find(|script| speaks(text, *script))
}

/// Every script the port implements, in upstream's detection order.
pub const ALL_SCRIPTS: [Script; 5] = [
    Script::Japanese,
    Script::Korean,
    Script::Chinese,
    Script::Hindi,
    Script::Cyrillic(CyrillicLanguage::Macedonian),
];

/// `text` contains at least one character the script claims (see [`Script`]).
fn speaks(text: &str, script: Script) -> bool {
    match script {
        // Kana only: ideographs alone cannot prove a Japanese reading (upstream isChinese).
        Script::Japanese => text.chars().any(is_kana),
        Script::Korean => text.chars().any(is_hangul),
        Script::Chinese => text.chars().any(is_hanzi),
        Script::Hindi => text.chars().any(is_devanagari),
        Script::Cyrillic(language) => spoken_as(text, alphabet(language)),
    }
}

fn is_kana(c: char) -> bool {
    ('\u{3040}'..='\u{30FF}').contains(&c)
}

fn is_hangul(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

fn is_hanzi(c: char) -> bool {
    // CJK Unified Ideographs plus the rarer extension-A block.
    ('\u{3400}'..='\u{4DBF}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c)
}

fn is_devanagari(c: char) -> bool {
    ('\u{0900}'..='\u{097F}').contains(&c)
}

/// Romanize one line in a known script; non-target characters pass through.
pub fn romanize_detected(script: Script, line: &str) -> String {
    match script {
        Script::Japanese => romanize_japanese(line),
        Script::Korean => crate::korean::romanize(line),
        Script::Chinese => romanize_chinese(line),
        Script::Hindi => crate::hindi::romanize(line),
        Script::Cyrillic(language) => {
            romanize_line(language, line).unwrap_or_else(|_| line.to_string())
        }
    }
}

/// Everything [`romanize_document`] fills in for a caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Romanization {
    pub script: Script,
    pub lines: Vec<LyricsLine>,
}

/// Detect the document's script once and romanize every line with that one table.
///
/// Returns `None` for Latin text and for scripts [`ALL_SCRIPTS`] does not cover
/// (Greek, Arabic, Thai, …), so a caller can keep the original lines verbatim.
pub fn romanize_document_enabled(doc: &LyricsDocument, enabled: &[Script]) -> Option<Romanization> {
    let sample = doc
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let script = detect_script(&sample, enabled)?;
    Some(Romanization {
        script,
        lines: doc
            .lines
            .iter()
            .map(|line| {
                let mut romanized = line.clone();
                romanized.text = romanize_detected(script, &line.text);
                romanized
            })
            .collect(),
    })
}

/// [`romanize_document_enabled`] over every ported script.
pub fn romanize_document(doc: &LyricsDocument) -> Option<Vec<LyricsLine>> {
    romanize_document_enabled(doc, &ALL_SCRIPTS).map(|romanization| romanization.lines)
}

/// Japanese romanization via `kakasi` (Hepburn); kana and kanji are both read.
///
/// Never fails: a line with no Japanese characters comes back unchanged.
pub fn romanize_japanese(text: &str) -> String {
    kakasi::convert(text).romaji
}

/// Chinese romanization via `pinyin`; tone marks are **dropped** (`nǐ hǎo` → `ni hao`)
/// so the result reads like the app's `tinypinyin` output.
///
/// Each Hanzi is followed by a space unless the text already supplies whitespace or
/// punctuation there, which keeps Latin text inside a Chinese line intact; a space
/// before CJK/ASCII punctuation is then dropped, matching upstream `romanizeChinese`.
pub fn romanize_chinese(text: &str) -> String {
    use pinyin::ToPinyin;

    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() * 2);
    for (i, c) in chars.iter().enumerate() {
        match c.to_pinyin() {
            Some(pinyin) => {
                out.push_str(pinyin.plain());
                let next = chars.get(i + 1);
                let separator =
                    next.is_some_and(|next| !next.is_whitespace() && !is_punctuation(*next));
                if separator {
                    out.push(' ');
                }
            }
            None => out.push(*c),
        }
    }
    trim_space_before_punctuation(out)
}

/// Remove the spaces the syllable separator left directly before punctuation.
///
/// Only whitespace runs that end at punctuation are dropped; every other space is kept
/// byte for byte, so Latin text inside a Chinese line is not reflowed.
fn trim_space_before_punctuation(text: String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            let mut run = String::from(c);
            while let Some(next) = chars.next_if(|next| next.is_whitespace()) {
                run.push(next);
            }
            if !chars.peek().is_some_and(|next| is_punctuation(*next)) {
                out.push_str(&run);
            }
            continue;
        }
        out.push(c);
    }
    out.trim_end().to_string()
}

fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(
            c,
            '\u{3001}'..='\u{3011}' | '\u{3014}'..='\u{301F}' | '\u{FF01}'..='\u{FF0F}' | '\u{FF1A}'..='\u{FF20}' | '\u{FF3B}'..='\u{FF40}' | '\u{FF5B}'..='\u{FF65}'
        )
}

/// Korean romanization (Revised Romanization, syllable-boundary rules applied).
pub fn romanize_korean(text: &str) -> String {
    crate::korean::romanize(text)
}

/// Hindi/Devanagari romanization; other characters pass through unchanged.
pub fn romanize_hindi(text: &str) -> String {
    crate::hindi::romanize(text)
}
