# ui — frozen source

## Source

| file | sha256 (at freeze) |
|------|--------------------|
| crates/ui/src/pure.rs | bd69302e719e139de95fd2194b7d0c015a5d03068a1f4b560b9214018802c808 |
| crates/downloads/src/model.rs | 73e3b75407ab16e97435933955a007571a830a0ec415095534110b4eeff3b648 |

Toolchain: Verus `0.2026.11.02.8bf68ee` (tag `release/0.2026.11.02.8bf68ee`),
bundled Rust `1.99.1-x86_64-unknown-linux-gnu`; workspace pin `1.99.1`.
Lynette gate: `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`.

## Scope — what this pair covers

`crates/ui/src/pure.rs` declares its helpers as "deterministic, total and
panic-free so the proof lead can freeze them into Verus spec/exec pairs". This
pair proves the integer/byte-shaped ones. It does **not** cover the file: 22
exec functions are in the pair — 16 core functions plus six transcription
helpers — and everything else is listed under "Not covered (full inventory)"
below and is *not* claimed.

Proven (16 core functions + 6 transcription helpers):

1. `session_file_path`
2. `queue_index_for_id`
3. `playlist_move_target`
4. `group_starts`
5. `expiry_hint`
6. `radio_should_extend`
7. `sleep_seconds`
8. `seek_relative`
9. `progress_percent` (`crates/downloads/src/model.rs`)
10. `storage_percent`
11. `seed_color_value`
12. `download_badge`
13. `download_icon`
14. `download_click`
15. `back_route`
16. `cookie_import_format`
17. `trim_start_span`, `starts_with_at`, `count_tabs`, `line_ok_exec`,
    `has_netscape_line`, `contains_byte` (six helpers; they exist only to model
    `trim_start`/`starts_with`/`lines`/`split('\t')` and have their own
    contracts).
18. `lemma_back_route_is_a_pop` and the other `proof fn` helpers are ghost
    lemmas; they are not part of the exec surface.

## Model deviations

- **`session_file_path(&Path) -> PathBuf`** is modelled on bytes:
  `session_file_path(dir: &[u8]) -> Vec<u8>`. Only the Unix branch of
  `Path::join` is transcribed: empty dir ⇒ name, trailing `/` ⇒ no doubled
  separator, otherwise one `/`; the name is `session-cookies.txt` byte for byte.
  Windows separators, `.`/`..` normalization and non-UTF-8 `OsStr` are out of
  subset.
- **`cookie_import_format(&str)`** is modelled on UTF-8 bytes (`&[u8]`).
  `trim_start` uses Rust's Unicode `White_Space`; the model trims the six ASCII
  whitespace bytes. `str::lines()` becomes maximal non-`\n` runs (a trailing
  `\r` affects neither predicate). `line.split('\t').count() >= 7` is
  `>= 6` tabs in the line. The check order is the real one: JSON first, then
  Netscape, then header, then unknown.
- **`expiry_hint(Option<i64>, i64) -> Option<String>`** returns the *message
  kind* (`ExpiryHint`) instead of the `String`; the real function returns five
  fixed sentences. `i64::saturating_sub` is transcribed as the guarded
  difference with `requires now_unix >= 0` (a Unix timestamp), so the
  difference cannot overflow; a non-positive remainder is `Expired`, exactly
  what saturation yields.
- **`queue_index_for_id(&[Song], &str)`** is `queue_index_for_id(ids: &[u32], id: u32)`:
  song ids become opaque tokens. `iter().position(..)` becomes an index loop
  with the same first-match order.
- **`playlist_move_target`**: `i8::unsigned_abs` becomes `0i16 - (delta as i16)`
  in the negative branch; `checked_add`/`checked_sub` become explicit bound
  guards; `then_some` becomes `if`.
- **`group_starts<T: PartialEq>`** is instantiated at `T = u8` (history keys are
  day buckets). The loop is the `enumerate` + `index == 0 || keys[index-1] != key`
  transcription.
- **`back_route(&mut Vec<Route>)`** is `back_route(stack: &mut Vec<u32>)`: the
  route type becomes an opaque token. `Vec::pop` is transcribed as index +
  `truncate` because vstd's `pop` on `&mut Vec` hits the mutable-reference
  restriction; the contract states the pop (returned top, remaining prefix,
  length, empty case).
- **`radio_should_extend`**: `usize::saturating_add(5)` becomes
  `p > usize::MAX - 5` (then `usize::MAX >= len` always) with a plain add
  otherwise; the guard is part of the transcription.
- **`sleep_seconds`**: `u64::from(minutes).saturating_mul(60)` cannot saturate
  for a `u32` and is transcribed as an explicit `u64::MAX / 60` guard (kept and
  proven dead), then `.min(86_400)` becomes an `if`.
- **`seek_relative`**: `u64::saturating_sub/saturating_add` become guarded
  arithmetic (`i128` for the `i64::unsigned_abs` step, so `i64::MIN` is
  handled); `.min(duration_ms)` becomes an `if`; the `duration_ms == 0` early
  exit is kept.
- **`progress_percent`**: the real body is
  `((bytes as u128 * 100) / total as u128).min(100) as u8`; the transcription
  keeps the `u128` multiply and divide and writes the `min` as an equivalent
  `if raw > 100 { 100 } else { raw as u8 }` (vstd has no `Ord::min` on `u128`
  in this position), so it is *semantically* identical, not textually.
- **`seed_color_value`** is `value & 0x00FF_FFFF` verbatim.
- **`download_badge(Option<&Download>)`** is
  `download_badge(state: Option<DownloadState>, percent: u8)`: the real code
  reads `entry.state` and `Download::percent()`; the transcription takes the
  computed percent and adds `requires percent <= 100`, whose source is
  `progress_percent`, proven in this same pair.
- **`download_icon` / `download_click`** return `&'static str` / `DownloadClick`.
  The icon table is modelled as opaque symbol ids (`1..=6`, one per row, in the
  real order); the click table is the real enum. `download_hint` has the same
  shape as `download_icon` and is **not** transcribed.

## Proven

- `session_file_path`: result is `dir + (optional '/') + "session-cookies.txt"`
  (`join_spec`); the name appears once and the path ends with it.
- `queue_index_for_id`: `Some(i)` ⇔ `i` is the **first** index with
  `ids[i] == id` and `i < len`; `None` ⇔ the id is absent.
- `playlist_move_target`: `Some(t)` has `t < len`, `t != index`,
  `t == index + delta`, `delta > 0 ⇒ t > index`, `delta < 0 ⇒ t < index`;
  `index >= len` or `delta == 0` ⇒ `None`; no overflow for any input.
- `group_starts`: result is exactly `starts_prefix`, strictly increasing, all
  indices `< keys.len()`.
- `expiry_hint`: `None` without dated cookies; `Expired` exactly for
  `expires_at <= now` (`lemma_expiry_expired`); `UnderDay` for 1..=86_399 s,
  `OneDay` for 86_400..=172_799 s, `Days(n)` with `n = remaining / 86_400`
  above (`lemma_expiry_boundaries`); monotone in the expiry (`expiry_rank`,
  `lemma_expiry_monotone`).
- `radio_should_extend`: true exactly when the position is known, the queue is
  non-empty and `position + 5 >= len` (with `usize::MAX` saturation); `None`
  never extends (`lemma_extend_never_empty`), and extending needs a non-empty
  queue (`lemma_extend_needs_queue`).
- `sleep_seconds`: `min(minutes * 60, 86_400)`, never above 24 h, exact for
  1..=1440 min (`lemma_sleep_seconds_minutes`), monotone.
- `seek_relative`: `min(saturating target, duration_ms)` for `duration_ms > 0`;
  no underflow (`lemma_seek_relative_no_underflow`) and always inside the track
  (`lemma_seek_relative_bounded`).
- `progress_percent`: `0` for a zero total, else `min(bytes*100/total, 100)`;
  always `<= 100`; `bytes >= total ⇒ 100` (`lemma_percent_full`);
  `bytes == 0 ⇒ 0` (`lemma_percent_zero_bytes`).
- `storage_percent`: `None` iff no limit; `Some(100)` for a zero limit;
  otherwise the `progress_percent` value, `<= 100`.
- `seed_color_value`: masks to the low 24 bits, high byte zero
  (`lemma_seed_color_masked`).
- `download_badge`: `None` entry ⇒ `None` badge; `Downloading(p)` carries
  exactly the percent (proven `<= 100`); every other state maps to its own
  badge.
- `download_icon`: one distinct id per badge class, `1..=6`
  (`lemma_icon_table_distinct`).
- `download_click`: the whole table, and `Completed` is the only badge whose
  click is inert (`(click == None) ⇔ (badge == Completed)`).
- `back_route`: empty stack ⇒ `None` and unchanged; non-empty stack ⇒ the
  previous top is returned, the stack is the old prefix and its length drops by
  exactly one; `lemma_back_route_is_a_pop` states the stack discipline
  (`s.push(x)` pops back to `x` with `s` underneath).
- `cookie_import_format`: JSON wins for a trimmed leading `[`/`{`; otherwise
  the first accepted line (`# Netscape` header or a line with ≥ 7
  tab-separated fields) selects Netscape; otherwise any `=` selects the header
  format; otherwise unknown. `trim_start_span` computes the leading-whitespace
  length exactly (`lemma_ws_prefix_split`, `lemma_ws_prefix_stops`),
  `starts_with_at` matches the marker byte-for-byte (`matches_at`),
  `count_tabs` counts tabs to the line end (`lemma_line_end`,
  `lemma_tabs_in_extend`), and `has_netscape_line`/`contains_byte` scan exactly
  the documented ranges (`lines_ok`, `has_byte_in`).

## Not covered (full inventory)

Every public/total helper of `pure.rs` that this pair does **not** prove, and
why — none of these is claimed as verified anywhere:

| function | status | reason |
|----------|--------|--------|
| `recognition_progress`, `recognition_timer_text`, `recognition_timing_text`, `recognition_subtitle`, `recognition_meta`, `recognition_search_query`, `recognition_youtube_music_url`, `recognition_copy_text`, `recognition_error_message` | OUT-OF-SUBSET | `f32`/`String`/`format!` |
| `percent_encode`, `folded`, `redact_secrets`, `local_file_url` | OUT-OF-SUBSET | `String`/`chars`/`OsStr` byte traversal |
| `best_recognition_match`, `recognition_fallback_song` | OUT-OF-SUBSET | string folding + `Song` ownership; CANDIDATE if the fold were modelled on bytes |
| `played_duration_ms` | CANDIDATE | pure `u64` clamp (`if duration_ms == 0 { position_ms } else { position_ms.min(duration_ms) }`); easy to add, not part of this freeze |
| `lyrics_offset_after` | CANDIDATE | `saturating_add` + `clamp(-3000, 3000)` on `i64`; easy to add, not part of this freeze |
| `translation_endpoint_valid`, `lastfm_connect_mode` | OUT-OF-SUBSET | `&str` trim/prefix logic and string emptiness |
| `lyrics_offset_label`, `crossfade_label`, `gain_label`, `listening_time_text`, `stats_date_text`, `wrapped_summary`, `stats_summary_cards`, `bytes_text`, `download_progress_text` | OUT-OF-SUBSET | `String`/`format!`/calendar arithmetic over i128 mixed with formatting |
| `seek_target`, `volume_target`, `crossfade_seconds`, `playback_speed`, `speed_percent`, `normalization_gain`, `volume_step`, `clamp_eq_bands` | OUT-OF-SUBSET | `f32`/`f64` clamp+round; no float encoding in vstd, so no bound on the returned float is claimed |
| `wrap_vinyl_angle`, `advance_vinyl_angle` (v0.1.1) | OUT-OF-SUBSET | `f32` modulo/`TAU` wrap plus `dt.clamp(0.0, 0.05)`; same float policy, pinned by the UI's rotation tests |
| `stats_period_label`, `month_short`, `download_hint`, `tray_command` | CANDIDATE | enum/table → literal text, no arithmetic; the tables are pinned by the unit tests, not by this pair (only `download_icon`/`download_click` are modelled as id/enum tables) |
| `account_error_message` | OUT-OF-SUBSET | `String` + payload matching; only its arm selection could be modelled |
| `valid_search_query`, `sanitize_proxy_url`, `parse_deep_link`, `first_deep_link`, `sanitize_video_id` | OUT-OF-SUBSET | `&str` `trim`/`strip_prefix`/`chars` scanning |
| `parse_persisted_queue` | OUT-OF-SUBSET | `serde_json` + `HashSet` |
| `samples_and_offset`-style helpers elsewhere in `pure.rs` | n/a | not present in this file |

## Re-freeze note (2026-11-02, release freeze)

`crates/ui/src/pure.rs` grew around the frozen functions while the account /
audio / recognition / stats UI landed. The proven bodies were re-read against
the frozen revision and are the same text; the hashes above are that revision's
(`29342d9a…`). `back_route` was added to this pair at the reviewer's request
(it had been claimed but not transcribed) — no source file changed for that.

## Re-freeze 2026-11-02 v0.1.1 (vinyl rotation helpers)

`crates/ui/src/pure.rs` gained two `f32` helpers at the top of the file
(`VINYL_RADIANS_PER_SEC`, `wrap_vinyl_angle`, `advance_vinyl_angle`) for the
spinning-vinyl animation; the new hash is in the table above
(`bd69302e…`). The delta is purely additive, so the pair's 16 source
functions were re-read and are byte-identical (the six transcription helpers
are unchanged too) — no `candidate.rs`/`original.rs` change and no re-prove. `crates/downloads/src/model.rs` is unchanged
(`73e3b754…`), so `progress_percent` is untouched.

The two new helpers are `f32` and fall under the documented float policy
(OUT-OF-SUBSET, listed in the inventory above). `crates/ui/src/app.rs` also
changed for the Settings nav routing and the single-fixture player demo; that
file is owned by `proofs/radio` and its delta is recorded there — this pair
does not prove anything in `app.rs`.
