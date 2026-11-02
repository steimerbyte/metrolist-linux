//! Port of upstream `MacedonianRomanizationTest` (5 cases), the "one script per song"
//! rule, and the CJK/Indic/translation paths added later.

use ml_core::{LyricsDocument, LyricsLine};
use ml_lyrics::{
    apply_offset, detect_cyrillic_language, detect_script, is_macedonian, romanize_chinese,
    romanize_document, romanize_hindi, romanize_japanese, romanize_korean, romanize_macedonian,
    romanized, translated, CyrillicLanguage, Script, ALL_LANGUAGES, ALL_SCRIPTS,
};

/// "Everything I have, both a house and a field" — the accent is on the first word.
const LINE_WITH_GRAVE_E: &str = "Сѐ што имам и куќа и нива";
/// "her" — the grave separates the pronoun from the conjunction.
const LINE_WITH_GRAVE_I: &str = "Ги виде ѝ рече";

const MACEDONIAN_SONG: [&str; 4] = [
    "Една цреша што мене ме чека",
    "Сѐ што имам и куќа и нива",
    "Но црешата да ја најдам жива",
    "Што е тоа во луѓето тажно",
];

fn doc(texts: &[&str]) -> LyricsDocument {
    LyricsDocument {
        provider: "test".into(),
        lines: texts
            .iter()
            .enumerate()
            .map(|(i, text)| LyricsLine {
                start_ms: i as u64 * 1_000,
                text: text.to_string(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

// ----------------------------------------------------------------- Cyrillic (ported)

#[test]
fn grave_accented_e_is_recognised_as_macedonian() {
    assert!(is_macedonian(LINE_WITH_GRAVE_E));
}

#[test]
fn grave_accented_i_is_recognised_as_macedonian() {
    assert!(is_macedonian(LINE_WITH_GRAVE_I));
}

#[test]
fn accented_lyrics_are_romanised_not_dropped() {
    let romanized = romanize_macedonian(LINE_WITH_GRAVE_E);
    assert!(!romanized.is_empty());
    assert!(
        !romanized
            .chars()
            .any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)),
        "got {romanized}"
    );
}

#[test]
fn grave_accented_vowels_romanise_to_plain_latin_letters() {
    assert_eq!(
        romanize_macedonian(LINE_WITH_GRAVE_E),
        "Se shto imam i kukja i niva"
    );
}

/// Detection runs over the whole lyrics, so every line has to be read with that one
/// alphabet — re-detecting per line reads ц as "ts" (Russian) or ж as "ž" (Serbian).
#[test]
fn every_line_of_a_song_is_read_with_the_same_alphabet() {
    let parsed = romanize_document(&doc(&MACEDONIAN_SONG)).expect("Macedonian detected");
    assert_eq!(parsed.len(), 4);
    for line in &parsed {
        assert!(
            !line
                .text
                .chars()
                .any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)),
            "{}",
            line.text
        );
    }
    assert!(parsed[0].text.contains("cresha"), "{}", parsed[0].text);
    assert!(parsed[2].text.contains("zhiva"), "{}", parsed[2].text);
    assert!(parsed[3].text.contains("tazhno"), "{}", parsed[3].text);
}

#[test]
fn detection_reports_macedonian_not_russian_for_shared_letters() {
    assert_eq!(
        detect_cyrillic_language("Съешь же ещё этих мягких французских булок", &ALL_LANGUAGES),
        Some(CyrillicLanguage::Russian)
    );
    assert_eq!(
        detect_cyrillic_language(LINE_WITH_GRAVE_E, &[CyrillicLanguage::Macedonian]),
        Some(CyrillicLanguage::Macedonian)
    );
    assert_eq!(
        detect_cyrillic_language(
            "Една цреша што мене ме чека",
            &[CyrillicLanguage::Macedonian]
        ),
        Some(CyrillicLanguage::Macedonian)
    );
    assert_eq!(
        detect_cyrillic_language("no cyrillic here", &ALL_LANGUAGES),
        None
    );
}

#[test]
fn non_cyrillic_text_is_not_romanised() {
    assert!(romanize_document(&doc(&["plain english"])).is_none());
}

#[test]
fn romanization_is_idempotent_on_latin_text() {
    assert_eq!(romanize_macedonian("already latin"), "already latin");
    assert_eq!(romanize_macedonian(""), "");
}

// ---------------------------------------------------------------- script detection

#[test]
fn scripts_are_detected_by_their_own_characters() {
    assert_eq!(
        detect_script("君の名は", &ALL_SCRIPTS),
        Some(Script::Japanese)
    );
    assert_eq!(detect_script("사랑해", &ALL_SCRIPTS), Some(Script::Korean));
    assert_eq!(
        detect_script("你好世界", &ALL_SCRIPTS),
        Some(Script::Chinese)
    );
    assert_eq!(detect_script("नमस्ते", &ALL_SCRIPTS), Some(Script::Hindi));
    assert_eq!(
        detect_script("Една цреша", &ALL_SCRIPTS),
        Some(Script::Cyrillic(CyrillicLanguage::Macedonian))
    );
}

/// Hanzi alone cannot separate Japanese from Chinese, so kana decides; a line of pure
/// Hanzi is read as Chinese, and an explicit allow-list overrides that.
#[test]
fn kana_decides_japanese_and_the_allow_list_overrides() {
    assert_eq!(detect_script("東京", &ALL_SCRIPTS), Some(Script::Chinese));
    assert_eq!(
        detect_script("東京タワー", &ALL_SCRIPTS),
        Some(Script::Japanese)
    );
    // A document with only Hanzi and no kana is read as Chinese even when Japanese is
    // the only enabled script: ideographs alone cannot prove the reading, and upstream
    // `isChinese` has the same rule. Callers holding extra knowledge call
    // `romanize_japanese` directly.
    assert_eq!(detect_script("東京", &[Script::Japanese]), None);
    assert_eq!(
        detect_script("日本語", &[Script::Chinese]),
        Some(Script::Chinese)
    );
}

#[test]
fn latin_and_uncovered_scripts_are_not_detected() {
    for text in [
        "hello world",
        "",
        "   ",
        "🎵🎵",
        "Γειά σου",
        "สวัสดี",
        "مرحبا",
        "\u{0}",
    ] {
        assert_eq!(detect_script(text, &ALL_SCRIPTS), None, "{text:?}");
    }
}

#[test]
fn mixed_text_picks_the_script_the_document_mostly_uses() {
    // Latin around it does not change the answer.
    assert_eq!(
        detect_script("Hello 世界 (live)", &ALL_SCRIPTS),
        Some(Script::Chinese)
    );
    assert_eq!(
        detect_script("Live: こんにちは!", &ALL_SCRIPTS),
        Some(Script::Japanese)
    );
    assert_eq!(
        detect_script("안녕 (Hello)", &ALL_SCRIPTS),
        Some(Script::Korean)
    );
}

// ------------------------------------------------------------------------ Japanese

#[test]
fn japanese_reads_kana_and_kanji() {
    assert_eq!(romanize_japanese("こんにちは"), "konnichiha");
    assert_eq!(romanize_japanese("世界"), "sekai");
    assert_eq!(romanize_japanese("東京"), "toukyou");
    assert_eq!(romanize_japanese("カタカナ"), "katakana");
    assert_eq!(romanize_japanese("がっこう"), "gakkou");
    assert_eq!(romanize_japanese("きゃく"), "kyaku");
}

#[test]
fn japanese_preserves_latin_punctuation_and_empty_input() {
    assert_eq!(romanize_japanese(""), "");
    assert_eq!(romanize_japanese("   "), "   ");
    assert_eq!(romanize_japanese("Hello 123"), "Hello 123");
    // Tokens are space-separated, like upstream's `joinToString(" ")`.
    assert_eq!(
        romanize_japanese("こんにちは、世界！"),
        "konnichiha, sekai!"
    );
}

// ------------------------------------------------------------------------- Chinese

#[test]
fn chinese_uses_tone_free_pinyin() {
    assert_eq!(romanize_chinese("你好"), "ni hao");
    assert_eq!(romanize_chinese("中文"), "zhong wen");
    // 乐 is a heteronym (lè/yuè); the plain table picks the first reading, and the
    // heteronym feature would return every candidate — the app emits one line.
    assert_eq!(romanize_chinese("音乐"), "yin le");
}

#[test]
fn chinese_drops_the_space_before_punctuation() {
    // Punctuation itself is kept as-is (upstream does not ASCII-fold it); only the
    // syllable separator before it is dropped.
    assert_eq!(romanize_chinese("你好，世界。"), "ni hao，shi jie。");
    assert_eq!(romanize_chinese("你好, world"), "ni hao, world");
    assert_eq!(romanize_chinese(""), "");
    assert_eq!(romanize_chinese("HELLO"), "HELLO");
    // Spaces inside Latin text are not reflowed; only the separator before
    // punctuation is dropped.
    assert_eq!(
        romanize_chinese("中文 hello world"),
        "zhong wen hello world"
    );
    assert_eq!(romanize_chinese("你好 hello, world"), "ni hao hello, world");
}

// -------------------------------------------------------------------------- Korean

#[test]
fn korean_uses_revised_romanization() {
    assert_eq!(romanize_korean("안녕하세요"), "annyeonghaseyo");
    assert_eq!(romanize_korean("사랑해"), "saranghae");
    assert_eq!(romanize_korean("감사합니다"), "gamsahamnida");
    assert_eq!(romanize_korean("서울"), "seoul");
}

#[test]
fn korean_resyllabifies_across_syllables() {
    assert_eq!(romanize_korean("한국어"), "hangugeo");
    assert_eq!(romanize_korean("한국"), "hanguk");
    assert_eq!(romanize_korean("좋아"), "joha");
    assert_eq!(romanize_korean("없어"), "eopseo");
    assert_eq!(romanize_korean("꽃"), "kkot");
}

#[test]
fn korean_preserves_non_hangul_text() {
    assert_eq!(romanize_korean(""), "");
    assert_eq!(romanize_korean("안녕 Hi 123"), "annyeong Hi 123");
    assert_eq!(romanize_korean("plain"), "plain");
}

// --------------------------------------------------------------------------- Hindi

#[test]
fn hindi_uses_the_app_table() {
    assert_eq!(romanize_hindi("गीत"), "geet");
    assert_eq!(romanize_hindi("दिल है"), "dil hai");
    assert_eq!(romanize_hindi("प्यार"), "pyaar");
    assert_eq!(romanize_hindi(""), "");
    assert_eq!(romanize_hindi("hello"), "hello");
}

#[test]
fn hindi_handles_nukta_and_digits() {
    assert_eq!(romanize_hindi("ज़िंदगी"), "zindgee");
    assert_eq!(romanize_hindi("१२३"), "123");
}

// ---------------------------------------------------------------- document helpers

/// Timing, word timings, agents and background flags survive romanization; only text
/// changes. That is what keeps karaoke rendering aligned.
#[test]
fn romanization_keeps_timing_metadata() {
    let mut original = doc(&["君の名は", "こんにちは"]);
    original.lines[0].end_ms = Some(900);
    original.lines[1].background = true;
    original.lines[1].agent = Some("v2".into());
    original.lines[1].words = vec![ml_core::LyricsWord {
        start_ms: 1_000,
        end_ms: 1_500,
        text: "こんにちは".into(),
    }];

    let romanized_doc = romanized(&original).expect("Japanese detected");
    assert_eq!(romanized_doc.lines[0].start_ms, 0);
    assert_eq!(romanized_doc.lines[0].end_ms, Some(900));
    assert_eq!(romanized_doc.lines[0].text, "kun no mei ha");
    assert!(romanized_doc.lines[1].background);
    assert_eq!(romanized_doc.lines[1].agent.as_deref(), Some("v2"));
    assert_eq!(romanized_doc.lines[1].words.len(), 1);
    assert_eq!(romanized_doc.lines[1].words[0].start_ms, 1_000);
    assert_eq!(romanized_doc.lines[1].words[0].text, "こんにちは");
    // The original document is untouched.
    assert_eq!(original.lines[0].text, "君の名は");
}

#[test]
fn each_script_romanises_a_whole_document() {
    for (text, expected) in [
        ("こんにちは", "konnichiha"),
        ("你好", "ni hao"),
        ("사랑해", "saranghae"),
        ("गीत", "geet"),
    ] {
        let lines = romanize_document(&doc(&[text])).expect("script detected");
        assert_eq!(lines[0].text, expected, "{text}");
    }
}

#[test]
fn empty_document_has_no_script() {
    let empty = LyricsDocument::default();
    assert!(romanize_document(&empty).is_none());
    assert!(romanized(&empty).is_none());
}

#[test]
fn malformed_and_control_text_is_total() {
    for text in ["\u{0}\u{1}\u{2}", "\u{FFFD}", "\u{10FFFF}", "ㅤ", "\u{0301}"] {
        let lines = romanize_document(&doc(&[text]));
        // Either undetected or romanized; never a panic.
        if let Some(lines) = lines {
            assert_eq!(lines.len(), 1);
        }
    }
}

// ------------------------------------------------------------------- offset/translate

#[test]
fn offset_shifts_lines_and_words_and_clamps_at_zero() {
    let mut document = doc(&["a", "b"]);
    document.lines[0].end_ms = Some(500);
    document.lines[1].words = vec![ml_core::LyricsWord {
        start_ms: 1_000,
        end_ms: 1_900,
        text: "b".into(),
    }];

    apply_offset(&mut document, 250);
    assert_eq!(document.lines[0].start_ms, 250);
    assert_eq!(document.lines[0].end_ms, Some(750));
    assert_eq!(document.lines[1].start_ms, 1_250);
    assert_eq!(document.lines[1].words[0].start_ms, 1_250);
    assert_eq!(document.lines[1].words[0].end_ms, 2_150);

    apply_offset(&mut document, -10_000);
    assert!(document.lines.iter().all(|line| line.start_ms == 0));
    assert!(document.lines[1]
        .words
        .iter()
        .all(|word| word.start_ms == 0));
    // Order is preserved, so the timeline still makes sense.
    assert_eq!(document.lines.len(), 2);
}

/// Two offsets compose like one, where no clamp intervenes: the property the UI relies
/// on when the user nudges the sync in two steps.
#[test]
fn offsets_compose() {
    let mut once = doc(&["a", "b", "c"]);
    let mut twice = doc(&["a", "b", "c"]);
    apply_offset(&mut once, 400);
    apply_offset(&mut twice, 150);
    apply_offset(&mut twice, 250);
    assert_eq!(once.lines, twice.lines);
    assert_eq!(
        once.lines.iter().map(|l| l.start_ms).collect::<Vec<_>>(),
        vec![400, 1_400, 2_400]
    );

    // A negative offset larger than the whole document clamps everywhere at zero.
    let mut clamped = doc(&["a", "b"]);
    apply_offset(&mut clamped, i64::MIN);
    assert!(clamped.lines.iter().all(|line| line.start_ms == 0));
    assert!(clamped
        .lines
        .iter()
        .all(|line| line.end_ms.unwrap_or(0) == 0));
}

/// The ladder is never empty, so a bad settings value cannot disable lyrics entirely.
#[test]
fn provider_ladder_is_never_empty() {
    for names in [vec![], vec!["nope"], vec!["  "]] {
        assert_eq!(
            ml_lyrics::LyricsClient::from_names(&names)
                .providers()
                .len(),
            1,
            "{names:?}"
        );
    }
}

#[test]
fn translation_keeps_timings_and_pads_missing_lines() {
    let mut document = doc(&["one", "two", "three"]);
    translated(&mut document, vec!["uno".into()]);
    let lines = document.translated.clone().expect("translated filled");
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].text, "uno");
    assert_eq!(lines[0].start_ms, 0);
    assert_eq!(lines[1].text, "");
    assert_eq!(lines[2].start_ms, 2_000);

    // Extra entries are dropped, never appended past the timeline.
    translated(
        &mut document,
        vec!["a".into(), "b".into(), "c".into(), "d".into()],
    );
    assert_eq!(document.translated.unwrap().len(), 3);
}
