# Verus handoff — `ml-lyrics`

For the proof lead. Everything below is pure and total: no I/O, no locks, no
`unwrap`/`expect` on input-dependent paths, no panics. No
`assume`/`admit`/`external_body`/`axiom` is needed — the bodies are small enough
to prove directly, except where noted.

Two bodies call third-party dictionaries (`kakasi`, `pinyin`); those are
**outside the Verus subset** and are listed as such rather than modelled.

## Obligations

| # | file | fn | postcondition to prove |
|---|---|---|---|
| 1 | `crates/lyrics/src/korean.rs` | `parts(c: char) -> Option<(usize, usize, usize)>` | `Some((cho, jung, jong))` ⇒ `cho < 19 ∧ jung < 21 ∧ jong < 28`; `None` ⇔ `c` outside `U+AC00..=U+D7A3`; total |
| 2 | `crates/lyrics/src/korean.rs` | `jong_alone(jong: usize) -> &'static str` | total for every `usize` (out-of-range and index `0` ⇒ `""`); `jong ∈ 1..28` ⇒ the table row |
| 3 | `crates/lyrics/src/korean.rs` | `jong_before_cho(jong, cho) -> Option<&'static str>` | lookup is a binary search over a sorted 117-row table; result is the row value for exactly that key, `None` for absent keys; total |
| 4 | `crates/lyrics/src/korean.rs` | `romanize(text: &str) -> String` | output is a concatenation of table strings and the input's non-Hangul chars in order; every held final is emitted exactly once (both at a syllable boundary and at end of input); total; `""` ⇒ `""` |
| 5 | `crates/lyrics/src/hindi.rs` | `romanize(text: &str) -> String` | two-char nukta row is preferred over the single row; unknown chars pass through verbatim and in order; digits map to ASCII digits; total; `""` ⇒ `""` |
| 6 | `crates/lyrics/src/romanize.rs` | `is_macedonian / spoken_as / detect_cyrillic_language` | existing obligations (already proven as `proofs/util`-adjacent); unchanged by this change |
| 7 | `crates/lyrics/src/romanize.rs` | `detect_script(text, enabled) -> Option<Script>` | result ∈ `enabled`; `None` ⇒ no script in `enabled` has a character in `text`; order is `enabled`'s; total |
| 8 | `crates/lyrics/src/romanize.rs` | `romanize_detected(script, line) -> String` | total for every `Script`; never panics; a line with no target characters is returned unchanged (for Korean/Hindi/Chinese; Japanese is dictionary-backed) |
| 9 | `crates/lyrics/src/romanize.rs` | `trim_space_before_punctuation(text) -> String` | only whitespace runs immediately followed by punctuation are removed; every other character is preserved in order; total |
| 10 | `crates/lyrics/src/romanize.rs` | `romanize_document_enabled(doc, enabled) -> Option<Romanization>` | `Some(r)` ⇒ `r.lines.len() == doc.lines.len()`; per-line `start_ms`/`end_ms`/`words`/`background`/`agent` are preserved identically (only `text` differs); `None` ⇒ undetected |
| 11 | `crates/lyrics/src/lib.rs` | `shift(ms: u64, offset: i64) -> u64` | `offset >= 0 ⇒ r == min(u64::MAX, ms + offset)`; `offset < 0 ⇒ r == saturating_sub(ms, |offset|)`; `r >= 0` always; total (no overflow) |
| 12 | `crates/lyrics/src/lib.rs` | `apply_offset(doc, offset_ms)` | every line and word timestamp is `shift`ed by the same offset; line order is unchanged; `start_ms`/`end_ms` monotonicity per line is preserved when the input had it |
| 13 | `crates/lyrics/src/lib.rs` | `translated(doc, lines)` | `doc.translated.len() == doc.lines.len()`; index `i` keeps the line `i`'s timing fields; missing entries are `""`; extras dropped; total |
| 14 | `crates/lyrics/src/translate.rs` | `fit_to_line_count(lines, count) -> Vec<String>` | `r.len() == count`; `r[i] == lines[i]` for `i < min(len, count)`; the rest are `""`; total |
| 15 | `crates/lyrics/src/translate.rs` | `lines_from_json / parse_translation_content` | `parse_translation_content` never panics on arbitrary input; a `{"lines": [...]}` or bare-array reply yields exactly those strings, in order; otherwise it falls back to one entry per non-empty line |
| 16 | `crates/lyrics/src/translate.rs` | `deepl_language(target) -> String` | total; `en` ⇒ `EN-US`, `zh*` ⇒ `ZH`, `pt` ⇒ `PT-PT`, otherwise `to_uppercase()`; Unicode case mapping (`to_lowercase`/`to_uppercase`) is in std, not the Verus subset — prove the five-way dispatch over an abstract `&str` |
| 17 | `crates/lyrics/src/translate.rs` | `TranslationClient::translate_lines` | unconfigured key ⇒ `Unavailable` **without** any transport call; otherwise the returned `Vec` has exactly `lines.len()` entries, with blank inputs keeping their slot as `""` |

## Outside the Verus subset (documented, not modelled)

- `kakasi::convert` (obligation 8, Japanese) — third-party dictionary + `unicode_normalization`.
- `pinyin::ToPinyin::to_pinyin` (obligation 8, Chinese) — third-party table.
- `serde_json` parsing in obligation 15 — a spec for "parse or fail" is not useful;
  prove the *line-count and ordering* postconditions over an abstract parsed input.
- `HttpTransport::post` and everything in `ReqwestTransport` — I/O.

## Invariants worth freezing (property tests, all present in `tests/`)

1. `romanize_document` preserves `lines.len()` and every timing field.
2. Every romanizer is total over all `char` values (tested with NUL, D800-block
   surrogates' neighbours, `U+10FFFF`, empty and whitespace-only input).
3. `apply_offset` is additive: `apply(a); apply(b)` ≡ `apply(a+b)` where no clamp
   occurs, and order never changes.
4. `translated` is length-preserving in both directions (pad and truncate).
5. `LyricsClient::from_names` never returns an empty ladder.
