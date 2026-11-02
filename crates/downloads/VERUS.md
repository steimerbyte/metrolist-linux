# Verus handoff — `ml-downloads`

For the proof lead (`proofs/downloads/**`). Everything below is pure and total:
no I/O, no locks, no `unwrap`, no panics. No `assume`/`admit`/`external_body`/
`axiom` is needed — the bodies are small enough to prove directly. Model
deviations are listed per obligation.

| # | file | fn | postcondition to prove |
|---|---|---|---|
| 1 | `crates/downloads/src/naming.rs` | `sanitize(input: &str) -> String` | every output char ∈ `[A-Za-z0-9 ._()[]-]`; output never contains `..`; never starts with `.`/space, never ends with `.`/space; non-empty (`"track"` when empty) |
| 2 | `crates/downloads/src/naming.rs` | `truncate_at_boundary(s: &str, max: usize) -> &str` | `r` is a prefix of `s`; `r.len() <= max`; `r` ends on a `char` boundary; total for every `max` (including `0` and `> s.len()`) |
| 3 | `crates/downloads/src/naming.rs` | `extension_for_mime(mime: &str) -> &'static str` | result ∈ `{webm, m4a, opus, mp3, flac}`; unknown ⇒ `webm`; total and deterministic |
| 4 | `crates/downloads/src/naming.rs` | `file_name(title, artist, video_id, extension) -> String` | `len() <= 120`; all chars allowed; contains `[<id>]`; `file_name(t,a,id,e) == file_name(t,a,id,e)` (deterministic); `id1 != id2 ⇒ name1 != name2`; total |
| 5 | `crates/downloads/src/model.rs` | `progress_percent(bytes: u64, total: u64) -> u8` | `total == 0 ⇒ r == 0`; `total > 0 ⇒ r == min(100, bytes*100/total)` in `u128`; result `<= 100` always |
| 6 | `crates/downloads/src/model.rs` | `occupied_bytes(state, size, bytes) -> u64` | `Completed ⇒ r == size`; `Downloading ⇒ r == bytes`; `Queued|Failed|Cancelled ⇒ r == 0` |
| 7 | `crates/downloads/src/model.rs` | `usage_of(entries) -> u64` (and `storage_sum`) | `r == min(u64::MAX, Σ occupied_bytes(entry))`; monotone in the input multiset; `r == 0` for empty |
| 8 | `crates/downloads/src/model.rs` | `clamp_concurrency(requested: usize) -> usize` | `1 <= r <= 8`; `requested ∈ 1..=8 ⇒ r == requested` |
| 9 | `crates/downloads/src/model.rs` | `DownloadState::can_transition(from, to) -> bool` | `r ⇔ from == to ∨ (from,to) ∈ table`; `Completed` has no outgoing edge to `Queued|Downloading`; reflexivity; `is_terminal == Completed|Failed|Cancelled` |
| 10 | `crates/downloads/src/manifest.rs` | `is_plain_file_name(name: &str) -> bool` | `r ⇒ no '/' '\\' '\0', name ∉ {".", "..", ""}, one component, relative`; hence `dir.join(name).parent() == dir` |

Notes for transcription:

1. `sanitize` uses `String::trim_matches`; model the trim as an index loop and
   prove the four boundary facts as separate lemmas (`lemma_no_dot_run`,
   `lemma_trimmed_is_nonempty`). The `for c in input.chars()` loop becomes an
   index loop over a `Seq<char>`.
2. `truncate_at_boundary` walks backwards while `!is_char_boundary(end)`. Model
   `is_char_boundary` as a spec predicate over a byte sequence (`bytes[end]` is
   not a UTF-8 continuation byte, i.e. `b & 0xC0 != 0x80`), then prove the walk
   terminates (decreasing index) and lands on a boundary. This is the one
   obligation that needs a small bytes-level model; keep it in a separate module
   so the rest of the proof stays at `Seq<char>`.
3. `extension_for_mime` lowercases before matching (`to_ascii_lowercase`). Model
   as a total predicate over `Seq<char>` with case folding; the five-way chain is
   a nested `if`, so it proves by case analysis. Order matters (`webm` before
   `mp4`), keep it.
4. `file_name` calls `sanitize`, `truncate_at_boundary` and `blake3::hash`
   (only when the id exceeds 48 bytes). Split the obligation: prove
   length/charset/`[id]`-suffix/determinism for **every** id, and prove
   distinctness only for ids `<= 48` bytes. Do not axiomatize blake3 — the long-id
   distinctness claim is a birthday-bound property, not a theorem, and the real
   code does not rely on it (the manifest is keyed by id, not by file name).
5. `progress_percent` uses `u128` arithmetic to avoid `u64` overflow. Model with
   `int` and prove `bytes*100/total <= 100` from `bytes <= total`; the clamped
   case `bytes > total` is the `min`. No overflow obligation at all.
6. `usage_of` is `fold` with `saturating_add`. Prove `r == sum` as long as the
   sum fits, and `r == u64::MAX` otherwise (the fold saturates); needs one
   induction over the slice.
7. `can_transition` is a finite match table: 25 pairs, prove each row and the
   three "must be false" rows (`Completed→Queued`, `Completed→Downloading`,
   `Queued→Completed`).
10. `is_plain_file_name` composes `Path::components().count() == 1`. Model
    `components` as a spec function over bytes: splitting on `/`, dropping `.`,
    a leading `..` counts as `ParentDir`. The `contains` checks alone do not
    imply "one component" (`a//b`), so the proof must cover the byte predicate
    directly; keep the RHS list as the postcondition.

Callers that rely on these: `manager.rs` (`complete`, `fail`, `cancel`,
`remove`, `path_for`, `enqueue`'s budget check) and `Download::percent`.
