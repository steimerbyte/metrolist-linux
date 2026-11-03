# Machine-checked proofs (`proofs/`)

`proofs/` holds Verus proofs for the pure, total logic of Metrolist. Each area
is a frozen pair:

- `original.rs` — the *spec + exec scaffold* transcribed from the real source:
  data types, `spec fn`s and the executable functions with their contracts,
  without loop invariants, `assert`s or `proof { … }` blocks. It is never
  edited to make a proof pass; a change to the source requires a re-freeze
  (new `SOURCE.md` hash + `proofs/freeze.sh` + re-prove). `proofs/derive_original.py`
  derives it from the candidate (dropping `proof fn`s, invariants, `assert`s
  and proof blocks): `together`, `radio`, `recognition` and `ui` regenerate
  byte-for-byte, the other pairs differ only in blank-line placement, and the
  older `settings`/`util` originals additionally keep a few ghost `proof fn`
  helpers from their transcription (ghost code cannot change exec behaviour;
  the gate still requires the exec surface and every contract to match). Nine
  originals (`queue`, `util`, `settings`, `stream`, `pages`, `together`,
  `radio`, `recognition`, `ui`) deliberately do **not** verify because the
  invariants live in the candidate; three (`audio`, `media`, `integrations`)
  happen to verify on their own. Every original compiles (checked: no
  resolution/type errors anywhere, only expected verification errors).
  Neither fact is required: the gate only ever verifies `candidate.rs`.
- `candidate.rs` — the same file plus only loop invariants, `decreases`,
  `assert`s, `proof { … }` blocks, and `spec fn`/`proof fn` helpers. No new
  exec fn, no signature, `requires` or `ensures` differences.
- `gate` — Verus, the Lynette additive-only check, the trust scan and the
  freeze-manifest check.
- `SOURCE.md` — source hashes, toolchain pins, and every modeling deviation.
- `FREEZE.sha256` — sha256 of every `original.rs`/`candidate.rs`, every
  `SOURCE.md` and `SOURCES.list`; `proofs/freeze.sh` rewrites it,
  `proofs/freeze-check.sh` fails when anything changed without it.
- `SOURCES.list` — the frozen `<area> <crates path>` declaration that
  `check-drift.sh` iterates.
- `REVIEW.md` — the fresh reviewer's `ACCEPT`, plus the sha256 of
  `FREEZE.sha256` and of the gate scripts. The gate hash covers the per-pair
  `gate` scripts, `spec_guard.py`, the self-test, the freeze/drift/review/run
  scripts, `verus.env`, `lynette-setup.sh`, `install-verus.sh`, every
  `SOURCE.md`, `SOURCES.list` and the trust scanner. `proofs/review-check.sh`
  fails when the record is missing, not `ACCEPT`, or stale.

```bash
./proofs/run-gates.sh        # drift + freeze + review + self-test + structure + all gates
./proofs/check-drift.sh      # exit 1 when a frozen source hash changed
./proofs/freeze.sh           # ONLY after a reviewed re-freeze (rewrites FREEZE.sha256)
./proofs/spec_guard_selftest.py  # asserts the guard still rejects the known evasions
```

A run is green only when all of the checks below pass. Checks 1–4 are executed
by each `proofs/<area>/gate`; the review record and the exec-surface check are
executed by `run-gates.sh` for the whole tree.

1. **Verus** prints exactly `verification results:: N verified, 0 errors`
   (`--crate-type=lib --expand-errors --rlimit 100`).
2. **Lynette** (`additions original.rs candidate.rs`) exits 0: the candidate
   adds only ghost/proof material. Measured limits: it exits **0** for an added
   top-level exec fn, and its rejection of a `mod sneaky` insertion is
   incidental (macro-level diff), so it is not an exec-surface check.
   `run-gates.sh` therefore also runs `proofs/spec_guard.py`, which **lexes**
   both files like Rust — skipping `//` and nested `/* … */` comments,
   normal/byte/raw strings and char literals — and binds each `fn`/`mod`/
   `impl`/`trait`/`const`/`static`/`struct`/`enum`/`type` keyword to the next
   identifier **across arbitrary whitespace, newlines and comments**. Scopes are
   tracked with `{`/`}` tokens, so every `fn` is compared at its `mod`/`impl`/
   `trait` path by tokenized signature (name, parameters, return type,
   `requires`, `ensures`); new `mod`/`impl`/`trait` blocks and new exec fns are
   rejected regardless of formatting. Limits: macro-generated items are
   invisible (no expansion), fn bodies are skipped wholesale, `mod name;` is
   recorded as a bodyless block, the body opener uses a small heuristic (a `{`
   after `if`/`else`/`match`/`=>`, or one whose group is followed by `,`/`else`,
   is an expression), and an unparseable declaration fails closed.
   `proofs/spec_guard_selftest.py` re-proves every run that the guard still
   rejects (a) a top-level exec fn, (b) an exec fn in a new `mod`, (c) a method
   in an existing `impl`, (d) a removed `ensures`, (e) an added `ensures`,
   (f) `fn\nsneaky`, `fn /* c */ sneaky` and `fn /* } { */ sneaky`,
   (g) `mod\nsneaky { fn\nhidden }`, (h) `pub(crate) fn` and `pub(in …) fn`,
   (i) a `pub(crate) mod` with a hidden fn, (j) a deleted `SOURCE.md` row and a
   changed/missing source, (k) a tampered `candidate.rs`/`original.rs`, while
   still accepting keywords that appear only inside strings or comments.
3. **Trust scan** (`proofs/trust_scan.py`, vendored so a public checkout does
   not need tooling outside the repo) finds no `assume`, `admit`,
   `external_body`, `axiom`, `assume_specification`,
   `#[verifier::external…]` or `unimplemented!`.
4. **Freeze** (`proofs/freeze-check.sh`) finds `FREEZE.sha256` matching every
   frozen pair, so an edit outside a recorded re-freeze fails the gate.
5. **Review** (`proofs/review-check.sh`) finds `REVIEW.md` with `verdict:
   ACCEPT` and matching sha256 values for `FREEZE.sha256` and the gate scripts.

`run-gates.sh` runs drift, freeze, review, the per-pair exec-surface check and
the guard self-test **before** any Verus gate, so a stale or unguarded gate
cannot report green. Order matters for the failure message: a mutated tree is
reported as `FAIL <area>: spec_guard`, not as a self-test anomaly.

The workspace is not a git checkout, so `SOURCE.md` pins each frozen file by
content hash (`sha256`) rather than by commit; when the tree gains history the
commit is recorded next to the hash and the gates re-run.

Tool locations are resolved without host paths: each gate defaults to
`$HOME/.local/opt/verus-x86-linux/verus` and to the sibling
`tools/Lynette/target/release/lynette`, and both can be overridden with
`VERUS=` / `LYNETTE=` (or by `proofs/install-verus.sh` /
`proofs/lynette-setup.sh`). `proofs/trust_scan.py` is a byte-for-byte vendored
copy of the `.omp` skill scanner (sha256
`96f6548fcc29b0ab1bde3d6e8f6c2e5191a5898b217d0a7f62f4bd9d14915414`); it is
hashed by the review record, so update it deliberately.

Pins live in `proofs/verus.env` (including the release ZIP sha256);
`proofs/install-verus.sh` downloads, hash-checks and version-checks the pinned
Verus, and `run-gates.sh` refuses to run under any other version.
`proofs/lynette-setup.sh` rebuilds the gate binary from the pinned Lynette
commit against the parser from the pinned Verus release (Lynette's own
`syn_verus` predates the `final` keyword). CI:
`.github/workflows/proofs.yml` runs `proofs/run-gates.sh`, which now blocks on
source drift (`crates/**` changed without a re-freeze) as well as on a failing
gate; CI duplicates no version pins, it sources `proofs/verus.env`.

Current status: **12 proofs, 277 functions verified, 0 errors**.

| proof | source | verified | log |
|-------|--------|---------:|-----|
| `proofs/queue` | `crates/core/src/queue.rs` | 35 | `proofs/logs/queue.log` |
| `proofs/util` | `crates/core/src/util.rs` | 10 | `proofs/logs/util.log` |
| `proofs/settings` | `crates/core/src/settings.rs` | 7 | `proofs/logs/settings.log` |
| `proofs/stream` | `crates/innertube/src/{stream.rs, player.rs, context.rs}` | 19 | `proofs/logs/stream.log` |
| `proofs/media` | `crates/core/src/media.rs` | 4 | `proofs/logs/media.log` |
| `proofs/integrations` | `crates/integrations/src/mapping.rs` | 13 | `proofs/logs/integrations.log` |
| `proofs/audio` | `crates/audio/src/{controller,engine}.rs` | 12 | `proofs/logs/audio.log` |
| `proofs/pages` | `crates/innertube/src/pages/next.rs` | 7 | `proofs/logs/pages.log` |
| `proofs/together` | `crates/together/src/{clock,sync,codec,model}.rs` | 64 | `proofs/logs/together.log` |
| `proofs/radio` | `crates/ui/src/{services,app}.rs`, `crates/innertube/src/lib.rs` | 12 | `proofs/logs/radio.log` |
| `proofs/recognition` | `crates/recognition/src/signature.rs` | 26 | `proofs/logs/recognition.log` |
| `proofs/ui` | `crates/ui/src/pure.rs`, `crates/downloads/src/model.rs` | 65 | `proofs/logs/ui.log` |

Freeze integrity: `proofs/FREEZE.sha256` covers both files of every pair, every
`SOURCE.md` and `SOURCES.list`; it is checked by every gate and by
`run-gates.sh`. `proofs/SOURCES.list` is the frozen declaration of sources
(`<area> <crates path>`); `check-drift.sh` iterates **it**, so deleting a row
from a `SOURCE.md` cannot disable drift, and it also fails when a `SOURCE.md`
declares a `crates/**` path that is not listed.

What the green gates say, concretely:

- **Queue** (`Queue`): index/selection invariants, `next`/`previous` under
  `RepeatMode::{Off,All,One}`, `remove` returns the successor (or previous at
  the tail) and remaps `current`, `move_item` remaps `current`, `permute`
  keeps the current song first and is a permutation, `lcg` verbatim,
  `reshuffle`/`set_shuffle`/`replace_songs`/`insert_next`/`append`/`clear`.
- **Util**: the models `duration_parts` (exact and reconstructing,
  `h*3600+m*60+s`), saturating `position_seconds`, `millis_to_seconds`,
  `thumb_bucket` monotonicity and the exact 11-byte video-id predicate; the
  `String`/`format!`/`&str` wrappers around them are explicitly out of subset.
- **Settings**: `seed_rgb` extracts `0xED5564 → (0xED,0x55,0x64)`, ignores the
  alpha byte, reconstructs exactly.
- **Stream**: `chunk_ranges` covers `0..total` exactly once, last end
  `total-1`, every non-last range exactly `chunk` bytes (last ≤ `chunk`);
  `expiry` (`None` iff within the margin, checked-add overflow ⇒ `None`);
  `validate_content_range` rejects `start>end`/`end>=total`;
  `pick_itag` prefers 251 then 140, considering only non-auto-dubbed formats
  with a non-empty URL; `base64url_nopad` output length `ceil(4n/3)` and every
  byte from the 64-byte base64url table. Injectivity is CANDIDATE (decode
  lemma), not claimed.
- **Media**: `StreamInfo::is_expired` — `None` never expires, `Some(e)`
  expires iff `now >= e-300` (saturating), monotone in `now`.
- **Audio**: `set_sleep_timer`'s seconds value (`m*60` clamped to 86 400,
  exact for the 1..=1440 min the UI offers) and `MpvEngine::request`'s
  wrapping request id (consecutive ids always differ).
- **Integrations**: `transport_status`, `transport_loop`/`repeat_from_loop`
  are mutually inverse bijections, `can_go_next`/`can_go_previous` under
  `Off`/`All`/`One`, `ms_to_us` saturating (exact below the saturation
  point), `us_to_ms` sign-preserving truncation, `position_ms_from_us`
  clamps negatives to 0.

## Coverage matrix

Statuses: **VERIFIED** (gate green, see table above) · **CANDIDATE** (pure and
total, proof not written; blocker named) · **OUT-OF-SUBSET** (needs something
Verus cannot take: I/O, async, floats, serde, FFI/D-Bus, `format!`, iterators,
`str` method adapters, wall clock).

### `ml-core`

| unit | status | evidence / reason |
|------|--------|-------------------|
| `Queue` playback/index state machine (`new`, `len`, `is_empty`, `position`, `is_shuffled`, `repeat_mode`, `set_repeat`, `cycle_repeat`, `song_at`, `current`, `jump_to`, `append`, `insert_next`, `clear`, `remove`, `move_item`, `next`, `previous`, `permute`, `set_shuffle`, `reshuffle`, `replace_songs`) | VERIFIED | `proofs/queue` |
| `RepeatMode::cycle`, `RepeatMode::as_str` | VERIFIED | `proofs/queue` |
| `millis_to_seconds` | VERIFIED | `proofs/util` (real fn) |
| `duration_parts` (model of `format_duration`'s arithmetic) | VERIFIED | `proofs/util`; `format_duration`/`format_duration_opt` themselves are OUT-OF-SUBSET (`String`/`format!`) |
| `position_seconds` (model of `format_position`'s clamp+divide) | VERIFIED | `proofs/util`; `format_position` is OUT-OF-SUBSET (`String`/`format!`) |
| `thumb_bucket` (model of `thumbnail_url`'s size selection) + monotonicity | VERIFIED | `proofs/util`; `thumbnail_url`/`resize_thumbnail_url` are OUT-OF-SUBSET (`format!`, `&str` rewriting) |
| `is_video_id_bytes`, `exec_valid_id_byte` (model of `parse_video_id`/`is_video_id`'s id predicate) | VERIFIED | `proofs/util`; the `&str`/`split_once` scaffolding is OUT-OF-SUBSET |
| `now_millis` | OUT-OF-SUBSET | wall clock |
| `settings::seed_rgb` (+`default().seed_color`) | VERIFIED | `proofs/settings` |
| `StreamInfo::is_expired` | VERIFIED | `proofs/media` |
| `Queue::source`, `Queue::set_source` | CANDIDATE | plain field copy (no arithmetic); one-line addition to `proofs/queue` |
| `Song::is_video_song` | CANDIDATE | one-arm field predicate; trivial, no gate yet |
| `Song::thumb` | OUT-OF-SUBSET | `format!` + `String` |
| `Song::artist_names`, `AlbumRef/Album/Artist/Playlist::artist_names` | OUT-OF-SUBSET | `Vec<String>`, `join`, `format!` |
| `Song::duration_text`, `share_url` and every other `share_url` | OUT-OF-SUBSET | `format!`/`url` crate |
| `AlbumRef::as_ref`, `PlaylistTrack::as_song`, `PlaylistEntry` accessors | CANDIDATE | field copies / `Option` plumbing, no arithmetic |
| `SearchEntry::id/title/thumbnail/explicit/as_song` | CANDIDATE | field returns, `Option` |
| `Lyrics::is_synced` | CANDIDATE | `Vec` length predicate |
| `PlaybackStatus`, `PlayerSnapshot::is_playing` | CANDIDATE | enum match on one field |
| `MlError` constructors (`unimplemented`, `Api`, …) | OUT-OF-SUBSET | owns `String`s, enum data only |
| `Queue::iter`, `songs()` | OUT-OF-SUBSET | `impl Iterator` adapter / slice view |
| `models::{new name/id}` constructors | OUT-OF-SUBSET | `impl Into<String>` |
| `events.rs`, `error.rs`, `lib.rs` re-exports | OUT-OF-SUBSET | data definitions only |

### `ml-innertube`

| unit | status | evidence / reason |
|------|--------|-------------------|
| `stream::chunk_ranges` | VERIFIED | `proofs/stream` (as `chunk_ranges`) |
| `stream::expiry` (margin) | VERIFIED | `proofs/stream` (as `expiry`, `SystemTime` → epoch seconds) |
| `stream::parse_content_range` (consistency checks only) | VERIFIED | `proofs/stream` (`validate_content_range`: `start>end` / `end>=total`; the `&str` scanning is OUT-OF-SUBSET) |
| `player::pick_itag` | VERIFIED | `proofs/stream` (two unrolled passes over `!auto_dubbed` + non-empty URL, same order) |
| `context::base64url_nopad` length + alphabet | VERIFIED | `proofs/stream` (real 64-byte table, `String`→`Vec<u8>`, `chunks(3)`→index loop) |
| `base64url_nopad` injectivity | CANDIDATE | needs a decoder round-trip lemma (inverse over 4-char groups + tail); output alphabet/length proven |
| `player::validate_tracking_url`, `parse_player`, `stream_info`, `number`, `player_body`, `video_thumbnail` | OUT-OF-SUBSET | `serde_json::Value`, `reqwest::Url`, regex-free string scanning |
| `parse::*` (`text_of`, `runs_from`, `extract_artists`, `parse_time`, `extract_duration`, `split_by_separator`, `odd_elements`, `ownership`, `thumbnail_url`, `largest_thumbnail`, `has_explicit`, `continuation`, `shelf_continuation`, `opt_str`, `array_at`) | CANDIDATE | pure over `serde_json::Value`; a Verus model needs a JSON tree type (next target: `parse_time`, `extract_duration`, `split_by_separator` only) |
| `context::is_stale`, `ClientKind::{host,client_id,user_agent,version}`, `cookie_value`, `parse_visitor_data`, `sapisidhash` | OUT-OF-SUBSET | clock/regex/`sha1` hex; `cookie_value`'s `split_once` is out of subset |
| `pot::is_expired`, `PoTokens` | OUT-OF-SUBSET | `SystemTime` + `Mutex` |
| `Innertube` client methods (`home` … `next`, `charts`, `moods_and_genres`, `new_releases`, `podcasts`, `podcast`, `browse`, `queue_append`, `refresh_client_config`, `send`) | OUT-OF-SUBSET | `async` + `reqwest`; `queue_append`'s only pure part is `pages::next::append` (VERIFIED) |
| `stream::content_total`, `parse_content_range_total` | CANDIDATE | total and pure over `(u16, Option<&str>, Option<u64>)` but the decision runs through `&str` scanning (`split_once`, `trim`, `parse`, `starts_with`), which has no Verus model — needs a `str`-as-`Seq<u8>` scanner (no `reqwest` in these signatures) |
| `pages::next::append` (continuation append: dedup by id, order, count, continuation, `current_index`) | VERIFIED | `proofs/pages`; model deviations named in its `SOURCE.md`: `Song.id`→`u32` token, `for`→indexed `while`, `iter().any(id ==)`→`contains_id` helper proven equal to `Seq::contains`; lemmas `lemma_len_monotone`, `lemma_no_duplicates` |
| `pages::podcast::attach_show` (Option fills only) | CANDIDATE | total and pure, but needs the `Episode`/`Playlist` field model (same id-token style as `proofs/pages`) |
| `pages::browse::albums` (`filter_map` over a `Vec`) | CANDIDATE | needs the `BrowsePage`/`Album` model (id tokens), then the same fold proof as `append` |
| `pages::{browse,moods,home,podcast}::{from_browse,…,chip_params}` | CANDIDATE | pure over `serde_json::Value`; needs a JSON tree model (same blocker as `parse::*`); `mood_genre`'s `u64 as u32` truncation is the only arithmetic |
| `lib.rs` `search_body` + `SearchFilter::params` | VERIFIED | `proofs/radio`: query copied verbatim, `params` present iff a filter is given, the five filters map to five distinct tokens (`lemma_filter_params_distinct`, `lemma_no_filter_no_params`, `lemma_query_is_verbatim`, `lemma_same_inputs_same_body`); `serde_json` body construction is OUT-OF-SUBSET (model: `SearchBody { query, params }`, tokens opaque) |
| `lib.rs` `CHART_PARAMS`, `MOOD_CATEGORY_BROWSE_ID` | OUT-OF-SUBSET | string constants (param mapping), no behaviour to prove |
| `lib.rs` client config (`params`, `Default`) | OUT-OF-SUBSET | `serde_json::Value` |

### `ml-audio` (audio2 obligations)

| unit | status | evidence / reason |
|------|--------|-------------------|
| `PlaybackController::play_queue` index clamp (`start_index.min(len-1)`) | CANDIDATE | pure; needs the `Queue` model lifted into the controller (queue half already VERIFIED) |
| `set_volume` clamp `0.0..=1.0` | OUT-OF-SUBSET | `f32`; contract frozen (clamp, then publish) |
| `set_sleep_timer` seconds (`m*60` min 86 400) | VERIFIED | `proofs/audio` transcribes the expression as `sleep_seconds`; `Instant`/`Duration` themselves are OUT-OF-SUBSET |
| `sleep_window_contains` (daily window, wrapping) | VERIFIED | real `controller.rs` body verbatim (`% 1440`, `start == end` whole day); `lemma_full_day_always_contains`, `lemma_ui_windows`, `lemma_ordered_step` |
| `Ipc::request` id wrap (`wrapping_add(1)`) | VERIFIED | `proofs/audio` models the update as `next_request_id` ((id+1) mod 2^64) |
| `engine::should_clear_stream_mute` (`allow_unmute && !intended_muted`, v0.1.1) | VERIFIED | `proofs/audio`: exact predicate plus `lemma_stream_mute_table` (four-case truth table) and `lemma_muted_stays_muted` |
| `clear_stream_mute`, `allow_unmute`/`unmute_checks` plumbing, `STREAM_UNMUTE_TICKS` | OUT-OF-SUBSET | `Ipc` socket/property state machine; only the pure predicate is proven |
| `is_current_entry` / `play_queue` keep-stream click (v0.1.1) | CANDIDATE | controller state machine over `fade`/`current_url`/queue contents; pinned by three regression tests |
| `seek`, `set_position_ms`, `position_ms` clamps | CANDIDATE | `u64` saturating arithmetic, no floats |
| `restore_queue` | CANDIDATE | field copy + UI emits (no arithmetic); needs the controller-state model |
| `append_queue` (dedup by id, append, count) | CANDIDATE | `HashSet<String>` + `Queue` model; the same fold over id tokens is VERIFIED in `proofs/pages` |
| `move_queue_item` / `remove_queue_item` | CANDIDATE | delegate to the proven `Queue` `move_item`/`remove`, then controller state machine |
| `set_crossfade` clamp (`min(30s)`) | CANDIDATE | pure `Duration` clamp (the rest of crossfade is `Instant`/`f32`); could reuse the `sleep_seconds` treatment |
| `attach_crossfade_engine`, `set_crossfade_gapless`, `set_device`, `set_muted`, `set_skip_silence`, `set_speed`, `set_normalization`, `set_eq`, `stop`, `submit_resolved`, `poll`, `snapshot`, `queue_view` | OUT-OF-SUBSET | channels/`Instant`/`f32` plumbing |
| sleep-timer `EndOfTrack` vs preloaded advance | CANDIDATE | controller state machine over `AudioEvent`; needs the event source injected as a pure step function |
| `eq::build_af` | OUT-OF-SUBSET | `f32` bands + `format!` |
| `devices::parse_devices` / engine arg assembly | OUT-OF-SUBSET | `str` splitting, `Vec<String>`, sockets/processes |
| `MpvEngine` (`spawn`, `send`, `try_recv`), `list_devices` | OUT-OF-SUBSET | subprocess + Unix sockets + `serde_json` |
| mpv `EndFile` reason parsing | OUT-OF-SUBSET | `serde_json` |

### `ml-integrations` (integrations2 obligations)

| unit | status | evidence / reason |
|------|--------|-------------------|
| `mapping::transport_status` | VERIFIED | `proofs/integrations` |
| `mapping::transport_loop`, `repeat_from_loop` (mutual inverses) | VERIFIED | `proofs/integrations` |
| `mapping::ms_to_us` (saturating), `us_to_ms`, `position_ms_from_us` | VERIFIED | `proofs/integrations` |
| `mapping::can_go_next`, `can_go_previous` | VERIFIED | `proofs/integrations` (`checked_add(1)` on the `Off` arm: no wrap at `usize::MAX`, answer unchanged) |
| `mapping::volume_of` (muted/NaN ⇒ 0, else clamp) | CANDIDATE | `f64`; blocked on a Verus float encoding — contract frozen in `SOURCE.md` |
| `mapping::track_path` (hex, prefix, empty-id rule) | CANDIDATE | exec loop provable, but injectivity needs the hex decode lemma (same class as base64) |
| `mapping::track_metadata`, `track_identity` | OUT-OF-SUBSET | `String`/`Vec<String>` collection, `Option` chains |
| `mapping::snapshot_changes` (7 property diffs) | CANDIDATE | pure diff of `Published`; needs the `TrackMetadata`/float equality model |
| `notify::should_notify` | CANDIDATE | keyed by `(id,title)` on the snapshot; string equality only |
| `discord::encode/read_frame`, `candidate_sockets` | OUT-OF-SUBSET | Unix sockets, `serde_json`, `format!` |
| `instance::encode_argv/decode_argv/socket_path` | CANDIDATE | pure `u32` length codec; round-trip lemma not written |
| `mpris`, `tray`, `notify` I/O paths | OUT-OF-SUBSET | D-Bus (`mpris-server`, `ksni`, `notify-rust`), `zbus` |

### `ml-together` (listen-together core)

| unit | status | evidence / reason |
|------|--------|-------------------|
| `model::song_to_track_info` duration (`(duration_seconds * 1000) as i64`) | VERIFIED | `proofs/together` (`ms_of`/`seconds_of` round trip, `lemma_duration_round_trip`, `lemma_zero_second_duration_is_absent`); the `String`/`Option<u32>` plumbing of `TrackInfo` is modelled as the `u32` fields only |
| `model::track_info_to_song` duration (`(duration_ms / 1000) as u32`) | VERIFIED | `proofs/together`, same round-trip lemma (`0 ms` ⇒ `None`, truncation pinned) |
| `clock::ServerClock::record_pong` sample policy (RTT, best pick, stale repair) | VERIFIED | `proofs/together`: `sample_accepted` (receive-vs-send inversion ±45 s, `lemma_stale_sample_rejected`), `network_round_trip` (`lemma_network_rtt_bounded`), `best_after` = `min` (`lemma_best_after_is_min`, `lemma_first_sample_records_best`); the `Instant`/`SystemTime` arithmetic is modelled as elapsed milliseconds |
| `clock::ServerClock::now_at` / `position_at` | VERIFIED | `proofs/together` (`now_at`: offset present iff a sample landed; `position_at` saturating advance, `lemma_advance_no_rewind`, `lemma_advance_nothing_to_add`, `lemma_advance_monotone`, `lemma_advance_saturates`) |
| `sync::drift_decision` (50 ms play / 750 ms seek thresholds) | VERIFIED | `proofs/together`: `drift_decision_spec` + `drift_abs`; exact boundaries and the `ready` (buffered) gate |
| `sync::should_seek_during_active_playback` | VERIFIED | `proofs/together` |
| `sync::position_at` (effective-at stamping) | VERIFIED | `proofs/together` (`sync_position_at_spec`) |
| `sync::RevisionGate` (`last`/`reset`/`set`/`accept`) | VERIFIED | `proofs/together`: unversioned `0` always accepted, stale rejected (`lemma_accept_stale_rejected`), advance (`lemma_accept_advances`), idempotent (`lemma_accept_idempotent`), no replay (`lemma_no_replay`, `lemma_set_then_accept`) |
| `sync::backoff_base` / `backoff_delay_ms` / `should_reconnect` | VERIFIED | `proofs/together`: 5-entry table for the attempt exponent (`lemma_backoff_table`), far below the nominal cap (`lemma_backoff_far_below_nominal_cap`), monotone (`lemma_backoff_monotone`), jitter bounded (`lemma_jitter_bounded`), reconnect boundary/monotone |
| `sync::canonical_playback_queue` (dedup of current + upcoming) | VERIFIED | `proofs/together`: `canonical_ids` prefix model, `contains_id` = `Seq::contains`, first-occurrence order preserved, current always first and present |
| `codec::should_compress` | VERIFIED | `proofs/together` (`compress_spec`, `envelope_compresses`): never compress unless strictly smaller and above the envelope (`lemma_small_payload_never_compressed`, `lemma_compression_never_inflates`, `lemma_equal_size_not_compressed`) |
| `ServerClock` `Instant`/`SystemTime` plumbing, `Transport` (WebSocket/`tokio`), message framing, presence and chat state, reconnection I/O | OUT-OF-SUBSET | wall clock, async I/O, `serde_json`; `f64` offsets/rtt are carried through the model but no float identity is claimed |
| `f64` server offset (`server_offset_ms: Option<f64>`) | OUT-OF-SUBSET | Verus has no float reasoning: the model keeps the `f64` field and proves only presence (`is_some()` iff a sample landed); every arithmetic fact is stated over `int`/`i64` milliseconds (`now_at_model`, `network_round_trip`, `best_after`) and documented as a deviation in `proofs/together/SOURCE.md` |

### `ml-recognition` (signature generation)

| unit | status | evidence / reason |
|------|--------|-------------------|
| `signature::crc32` | VERIFIED | `proofs/recognition`: `crc32` = `crc32_spec` (eight reflected `0xEDB88320` bit steps per byte, final inversion), `lemma_crc32_empty`, `lemma_crc_prefix_step` |
| `signature::base64_encode` shape | VERIFIED | `proofs/recognition`: `len == 4*ceil(n/3)`, multiple of 4, `pad < 3` with the exact per-residue counts, growth `4/3 + slack`; the padding **characters' positions** are pinned by the golden test, not proven |
| `signature::band_of` mapping | VERIFIED | `proofs/recognition` (`band_of_exec`): `<250`, `<520`, `<1450`, `<3500`, `<=5500`, else `None`; `lemma_band_boundaries`, `lemma_band_monotone`; integer Hz, the float multiply `bin * BIN_HZ` is OUT-OF-SUBSET |
| `signature::encode_peak` (0xFF escape, `prev` re-anchor) | VERIFIED | `proofs/recognition`: 5 bytes, 10 with escape (`lemma_frame_sizes`), escape iff delta ≥ 255, the byte after `0xFF` is `0`, `prev` is the peak just written |
| `signature::samples_and_offset` (12 s window, `0.24 s` lead-in) | VERIFIED | `proofs/recognition` (`samples_and_offset_exec`): exact `num_samples + 3840` (`16_000 * 0.24 = 3840`), i32-safe for the window (`lemma_samples_and_offset_bounded`), hop accounting (`lemma_hop_arithmetic`) |
| `process` (rustfft), peak picking over float magnitudes, windowing, resampler, WAV read, HTTP upload | OUT-OF-SUBSET | `rustfft`/`f32`/`f64`, I/O |

### `ml-lyrics`, `ml-store`, `ml-ui`

| crate | status | reason |
|-------|--------|--------|
| `ml-lyrics` | OUT-OF-SUBSET | `reqwest`, XML/JSON parsing, `regex`; its `invariants.rs` already asserts sorting/monotonicity/prefix bounds with property tests |
| `ml-store` | OUT-OF-SUBSET | `rusqlite` (SQLite FFI) |
| `ml-ui` | OUT-OF-SUBSET | `eframe`/`egui`/`wgpu` state, except the pure radio/route logic below (VERIFIED) |

### `ml-ui` (pure radio and routing logic)

| unit | status | evidence / reason |
|------|--------|-------------------|
| `services::radio_event` (queue page → `RadioReady`) | VERIFIED | `proofs/radio`: origin is the clicked song, songs pass through unchanged, source `Radio`, index in range for a non-empty page, the clicked song's **first** occurrence wins, else the panel `current_index` when in range, else `0` |
| `services::radio_fallback` (single-track queue) | VERIFIED | `proofs/radio`: queue is exactly `[song]`, index `0`, caller source preserved |
| `app::url_route` (`ParsedUrl` → `Option<Route>`) | VERIFIED | `proofs/radio`: `Video` → `None`, `Album` with an `MPRE` id → `Album` page else `Playlist`, `Playlist`/`Artist` map to themselves, id never rewritten; `lemma_mpre_needs_four_chars` |
| `App` event loop, `eframe`/`egui` state, the radio fetcher (async `next` + generation checks), layout (`center_column`) | OUT-OF-SUBSET | `eframe`/`egui`/`wgpu`, `tokio`/`reqwest`, `Mutex` generation state; the generation-gated writes are a concurrency property, not a pure function |
| `pure::expiry_hint` | VERIFIED | `proofs/ui` (message-kind model): `None` without dated cookies, `Expired` for `expires_at <= now`, `UnderDay` for 1..=86_399 s, `OneDay` for 86_400..=172_799 s, `Days(n)` above; boundaries and monotonicity proven |
| `pure::back_route` | VERIFIED | `proofs/ui` (route tokens): empty stack ⇒ `None` and unchanged; otherwise returns the previous top, leaves the old prefix, length drops by exactly one (`lemma_back_route_is_a_pop`) |
| `pure::session_file_path` | VERIFIED | `proofs/ui` (PathBuf → `&[u8]`/`Vec<u8>`, Unix `join` transcribed): result is `dir + (optional '/') + "session-cookies.txt"` |
| `pure::queue_index_for_id` | VERIFIED | `proofs/ui` (id tokens): `Some(i)` ⇔ `i` is the first index with that id and in range, `None` ⇔ absent |
| `pure::playlist_move_target` | VERIFIED | `proofs/ui`: `t < len`, `t != index`, `t == index + delta`, direction matches the sign; `index >= len`/`delta == 0` ⇒ `None`; no overflow |
| `pure::group_starts` | VERIFIED | `proofs/ui` (`T = u8`): result is exactly `starts_prefix`, strictly increasing, all `< keys.len()` |
| `pure::radio_should_extend` | VERIFIED | `proofs/ui`: true iff position known, queue non-empty and `position + 5 >= len` (with saturation) |
| `pure::sleep_seconds` | VERIFIED | `proofs/ui`: `min(minutes*60, 86_400)`, exact for 1..=1440 min, monotone, never above 24 h |
| `pure::seek_relative` | VERIFIED | `proofs/ui`: `min(saturating target, duration_ms)` for `duration_ms > 0`, no underflow, result inside the track |
| `pure::seed_color_value` | VERIFIED | `proofs/ui`: masks to the low 24 bits, high byte zero |
| `pure::progress_percent` (`downloads::model`) | VERIFIED | `proofs/ui`: `0` for a zero total, else `min(bytes*100/total, 100)`; always `<= 100`; `bytes >= total` ⇒ `100` |
| `pure::storage_percent` | VERIFIED | `proofs/ui`: `None` iff no limit, `Some(100)` for a zero limit, else `progress_percent`, `<= 100` |
| `pure::download_badge` | VERIFIED | `proofs/ui` (state + percent model, `percent <= 100`): `None` entry ⇒ `None` badge, `Downloading(p)` carries exactly the percent |
| `pure::download_icon`, `pure::download_click` | VERIFIED | `proofs/ui`: icon ids are the real table (one per badge, `1..=6`); click table proven and `Completed` is the only inert click |
| `pure::cookie_import_format` | VERIFIED | `proofs/ui` (UTF-8 bytes; ASCII `trim_start`; `lines()` = runs of non-`\n`): JSON wins on a trimmed `[`/`{`, then a `# Netscape` header or a ≥7-field tab line, then any `=`, else unknown |
| `pure::{seek_target, volume_target, crossfade_seconds, playback_speed, speed_percent, normalization_gain, volume_step, clamp_eq_bands}` | OUT-OF-SUBSET | `f32`/`f64` clamp+round; no float encoding in vstd, so no bound on the returned float is claimed (the source doc comments' postconditions are unproven here) |
| `pure::{wrap_vinyl_angle, advance_vinyl_angle}` (v0.1.1) | OUT-OF-SUBSET | `f32` modulo/`TAU` wrap and `dt.clamp(0.0, 0.05)`; same float policy, pinned by the UI rotation tests |
| `shell::{NavAction, active_tab, settings_active}` nav routing (v0.1.1) | OUT-OF-SUBSET | UI routing state machine over the current route; `apply_nav` is a table dispatch with side effects |
| `pure::{crossfade_label, gain_label, bytes_text, download_progress_text, account_error_message, local_file_url, parse_deep_link, first_deep_link, sanitize_proxy_url, valid_search_query}` | OUT-OF-SUBSET | `String`/`format!`/`&str` scanning (`trim`, `chars`, `strip_prefix`) and `Path` percent-encoding; `account_error_message` additionally maps error payload strings, so only its arm selection could be modelled |
| `pure::{download_hint, tray_command, stats_period_label, month_short}` | CANDIDATE | literal enum/table → text (`&'static str`); no arithmetic, pinned by the unit tests, not transcribed into this pair |
| `pure::parse_persisted_queue` | OUT-OF-SUBSET | `serde_json` + `HashSet` |

## Candidate backlog (ordered)

1. `parse_time` / `extract_duration` — small `&str` scanners; the only thing
   keeping them out is a `str`-as-`Seq<u8>` model.
2. `content_total` / `StreamCache` freshness — pure arithmetic on
   `Option<u64>`; needs a header-slice model to keep the source shape.
3. `track_path` injectivity and `base64url_nopad` injectivity — both need one
   decode/round-trip lemma over fixed-size output groups.
4. `mapping::volume_of` / `snapshot_changes` — blocked on a float encoding.
5. `EndOfTrack` vs preloaded advance — needs the audio event loop restated
   as a pure step function over `AudioEvent`.
6. `together::ServerClock` float offset — blocked on a Verus float encoding;
   the integer/lifetime model (`now_at_model`, `best_after`) is what is proven.
7. `recognition::base64_encode` padding positions — needs a byte-level output
   equality statement (the length/shape lemmas are proven today).

## Honest limits

- A green gate proves the *frozen model* equals its spec, not the binary. The
  transcription boundary is written down per area in `SOURCE.md`; anything the
  model does not contain (I/O, floats, `String` construction) is outside the
  proof, not "proven correct".
- `original.rs` is the frozen statement of what was transcribed. Nine
  originals (`queue`, `util`, `settings`, `stream`, `pages`, `together`,
  `radio`, `recognition`, `ui`) do not verify by design (their invariants live
  in the candidate); three (`audio`, `media`, `integrations`) happen to verify
  on their own. All twelve originals compile. Either way the gate verifies
  only `candidate.rs`, and `spec_guard.py` plus Lynette enforce that the
  candidate cannot weaken or extend its exec surface.
- Source drift never silently invalidates a proof: `run-gates.sh` runs
  `check-drift.sh` first and blocks (exit 1) when a file under `crates/**`
  changed without a re-freeze. A re-freeze is a reviewed change: update
  `SOURCE.md` (new hash + reason), run `proofs/freeze.sh`, re-prove, and record
  the semantic delta in `SOURCE.md` (see `proofs/integrations` for the
  `usize::MAX` `checked_add` re-freeze).
- `spec_guard.py` is a token scanner, not a Rust parser: it lexes away
  comments and strings, compares tokenized exec signatures at every
  `mod`/`impl`/`trait` scope path, and ignores bodies (Verus proves the frozen
  contract). It cannot see macro-generated items, and its body-opener/scanner
  heuristics are described in item 2 of **The gate** above; the self-test pins
  the accept/reject behaviour.
- `matches!` with a guard cannot be used in a frozen exec body: Verus
  `0.2026.11.02.8bf68ee` panics on it, so such predicates are written as
  `is_some() && unwrap()` with the same value (documented in `SOURCE.md`).
- `REVIEW.md` is a **paper trail, not a proof**. A prover who edits the gate
  scripts can also edit this record and recompute both hashes; the review-check
  gate only guarantees that the exact gate bytes and freeze manifest in the
  record are the ones that ran. The real controls are independent CI on the
  pinned toolchain plus a fresh reviewer re-signing after any gate change. Do
  not claim this boundary is mechanically enforced.

## Independent review (2026-11-02)

Reviewer session `proof-final-review`, fresh context, independent of the proof
author. **Verdict: ACCEPT** of the full gate, including the rewritten
token-lexer `spec_guard.py` and its 12-mutation self-test. That ACCEPT covers
the freeze as it stood then: 7 pairs, 95 verified.

Post-review changes (same day, **not covered by that ACCEPT**):

- parity-audio landed crossfade/sleep-fade/normalization work, so
  `proofs/audio` was re-frozen to the new `controller.rs`/`engine.rs` hashes
  (the two proven expressions kept their exact shape) and a new real function
  `sleep_window_contains` was transcribed and proven: audio 7 → 12 verified.
- parity-catalog landed `crates/innertube/src/pages/*`; a new `proofs/pages`
  pair proves `pages::next::append` (dedup by id, order, count, continuation,
  `current_index`) over an id-token model, with its model deviations named in
  `proofs/pages/SOURCE.md`: 7 verified, 8 proofs / 107 functions total.
- Three new pairs for the newly landed parity modules, all green:
  `proofs/together` (clock/sync/codec/model: sample policy, drift/seek,
  revision gate, backoff, canonical queue, compression decision — 64 verified),
  `proofs/radio` (`radio_event`, `radio_fallback`, `url_route`,
  `search_body`/`SearchFilter::params` — 12 verified) and `proofs/recognition`
  (`crc32`, base64 shape, band mapping, 0xFF escape, sample/lead-in arithmetic
  — 26 verified). 11 proofs / 209 functions total. Model deviations are in
  each `SOURCE.md`: `f64` offsets are carried but never reasoned about
  (integer models only), `String` ids become `char` sequences / `u32` tokens,
  `serde_json::Value` becomes `SearchBody { query, params }` with opaque
  params tokens, `to_le_bytes`/`extend_from_slice` become explicit byte
  pushes, and the recognition band thresholds are compared on integer Hz.
- The 15:56 `crates/audio` delta (controller/engine frozen hashes) was a
  `rustfmt` reflow of `controller,engine,eq,normalize,tests/live.rs`, not a
  semantic change: parity-audio confirmed no post-FINAL edits, ui-lead owns
  the additive controller queue/crossfade API edit, the proven `request`,
  `set_sleep_timer` and `sleep_window_contains` shapes are verbatim, and
  `cargo test -p ml-audio` (55 unit + 11 live) plus clippy are green at the
  frozen hashes. The engine delta that note recorded as "unattributed" was later
  identified as the shutdown/leak work and is now attributed in
  `proofs/audio/SOURCE.md` (see the final re-freeze below).
- `crates/ui/src/services.rs` was edited by the search/radio wiring after the
  first hash was recorded; `radio/SOURCE.md` carries the new hash and the
  proven `radio_event`/`radio_fallback` bodies were re-checked verbatim
  against it.

### Release freeze 2026-11-02 (superseded by the v0.1.1 freeze below)

Twelve pairs are frozen: `proofs/FREEZE.sha256` =
`a561191c669fa98c97ef384bec14aa952f9023c2643cfa8fcfb28232b55c664d`
(37 files), `gate_sha256` =
`7a9f0f1fc836f0d7999e0796a4a990edadef4ee931f01d157e87b23d467fd318`.
274 functions verified / 0 errors; Lynette `additions` and `compare` green;
trust scan clean; `check-drift.sh` 21/21; the gate self-test covers all twelve
pairs (12 accepted, 12 mutations rejected). The gate scripts no longer contain
host paths (`VERUS`/`LYNETTE` default to `$HOME`/sibling-checkout and can be
overridden) and `proofs/trust_scan.py` is vendored so a public checkout needs
no tooling outside the repo. `docs/SOURCE-FREEZE.sha256` (246 files, packaging
provenance) was regenerated for the same revision.

### Re-freeze after the shutdown/leak fix (2026-11-02)

The app could outlive its `mpv` children and IPC sockets on `SIGTERM`/`SIGKILL`.
The fix touched `crates/audio/src/{engine,controller}.rs`,
`crates/ui/src/{app,services,signals,desktop,lib}.rs` and `Cargo.toml`/
`Cargo.lock`; `crates/ui/src/pure.rs` (`29342d9a…`) and
`crates/downloads/src/model.rs` (`73e3b754…`) are unchanged, so the `ui` pair
carries no source change. The proven bodies (`sleep_window_contains`,
`set_sleep_timer`, `Ipc::request`, `radio_event`, `radio_fallback`,
`url_route`, `search_body`) were re-read at the new revision and are verbatim;
the new lifecycle code is outside the Verus subset and is enumerated in
`proofs/audio/SOURCE.md` and `proofs/radio/SOURCE.md`. Counts therefore stay
**274 verified / 0 errors** across the same 12 pairs; only the frozen hashes
moved. `docs/SOURCE-FREEZE.sha256` was regenerated for the new tree (246 files,
now including `crates/ui/src/signals.rs`).

`FREEZE.sha256` was regenerated for this revision and `REVIEW.md` was **not
touched by the author**; the independent reviewer has now signed this
revision. Per the policy above these additions need that review, and CI is the
other control.

Status of the twelve-pair review (audio, integrations, media, pages, queue,
radio, recognition, settings, stream, together, ui, util): **ACCEPTED** by
independent reviewer session `proof-final-review` for the shutdown/leak
re-freeze above (`freeze_sha256` a561191c669fa98c97ef384bec14aa952f9023c2643cfa8fcfb28232b55c664d,
`gate_sha256` 7a9f0f1fc836f0d7999e0796a4a990edadef4ee931f01d157e87b23d467fd318).
The earlier freeze (`freeze_sha256` 5725611a091d5dde168c33aef3d57f1e792123add4621c446f6de63347874763,
`gate_sha256` 11a31e4ad06ed1a576cac81057bd5a56d110281332a619d0a426b6d2e6405141)
was also ACCEPTED. Two earlier review rounds returned REJECT for documentation
fidelity only: a claimed-but-missing `back_route`, a "verbatim" wording on
`progress_percent`, copied cross-area function lists, and count/status drift
between this file and `ui/SOURCE.md`. All are fixed and the proof content
remains green (274 verified / 0 errors, 12 pairs); `REVIEW.md` now carries the
current ACCEPT.

What green proves, per the gate definition above: each frozen `candidate.rs`
verifies under the pinned Verus (`0.2026.11.02.8bf68ee`, 277 verified / 0
errors across the 12 pairs); Lynette `additions` accepts it; `trust_scan.py`
finds no `assume`/`admit`/`external_body`/`axiom`/`assume_specification`/
`unimplemented!`; the pair matches `FREEZE.sha256`; and `spec_guard.py` plus
the self-test show the candidate cannot add or change an exec surface at any
`mod`/`impl`/`trait` depth, including across newlines, comments, raw strings
and split keywords. Source drift is blocked, not warned (`check-drift.sh`
exits 1 unless all 21 declared sources match the frozen hashes).

Residual trust is unchanged by this review and remains explicit:

- `SOURCE.md` is a hand transcription of `crates/**`; the proofs are about the
  frozen transcription, not the compiled binary. A wrong transcription that
  is internally consistent would verify — the drift hashes pin the revision,
  not the semantic fidelity.
- CANDIDATE and OUT-OF-SUBSET rows in the matrix are not proofs: they name
  functions or models whose real statements are not verified, with the
  blocking reason recorded. Models (`sleep_seconds`, `expiry`, `chunk_ranges`
  models, …) are labelled as models, never as real functions.
- `spec_guard.py` cannot see macro-generated items, so a proc-macro or
  `macro_rules!` that emits an exec fn is outside its scope. No such macro is
  present in the frozen pairs.
- The lexer/scanner is heuristic, not a Rust parser: nested block comments,
  raw strings and split keywords are handled and self-tested, but an
  unparseable declaration fails closed rather than parsing per the reference
  compiler.
- `REVIEW.md` is a paper trail, not a proof. A prover who edits the gate
  scripts can edit this record and recompute both hashes; the real controls
  are independent CI on the pinned toolchain plus a fresh reviewer re-signing
  after any gate change.

### Release freeze 2026-11-02 v0.1.1

Twelve pairs are frozen at version `0.1.1`: `proofs/FREEZE.sha256` =
`1a07e2b574959b8316e97dc64fa1d407b614e07fea315e895bd2d86b45c01676`
(37 files), `gate_sha256` =
`9ce7fb1a802dc0600d1dee8cc5df85919a85cd6da3111fb47b2ba22d6e4889ea`.
**277 functions verified / 0 errors** (audio 12 -> 15 with the new
`should_clear_stream_mute` obligation); Lynette `additions` and `compare`
green; trust scan clean; `check-drift.sh` 21/21; the gate self-test covers all
twelve pairs. `docs/SOURCE-FREEZE.sha256` (246 files) was regenerated for this
tree: `345a3ebb21a15064ca99c1cbbc77c8bbc94f53a8c672b2ecd3dd9a80eebb0485`.

Source delta since the previously accepted freeze (documented per pair, no
proven body changed except the new predicate):

- `crates/audio/src/{engine,controller}.rs` — stream-level `ao-mute` clear with
  a four-tick reassert and test-mute opt-in (new pure `should_clear_stream_mute`
  is now proven), plus keep-stream play when the clicked entry is already
  loaded; `set_sleep_timer`, `sleep_window_contains` and `Ipc::request` were
  re-read verbatim.
- `crates/ui/src/app.rs` — Settings nav routing (`apply_nav` over
  `shell::NavAction`) and the single-fixture player demo; `url_route` verbatim.
- `crates/ui/src/pure.rs` — two additive `f32` vinyl-rotation helpers
  (`wrap_vinyl_angle`, `advance_vinyl_angle`), OUT-OF-SUBSET; the pair's 16
  source functions are byte-identical.
- `crates/lyrics/src/providers.rs` — LRCLIB User-Agent from
  `CARGO_PKG_VERSION`/`CARGO_PKG_REPOSITORY`; not a proof source.
- `Cargo.toml`/`Cargo.lock` — workspace version `0.1.1` and repository URL.

`REVIEW.md` was **not touched by the author**; the fresh independent review
has signed `v0.1.1` for this freeze (`freeze_sha256`
`1a07e2b574959b8316e97dc64fa1d407b614e07fea315e895bd2d86b45c01676`,
`gate_sha256` `9ce7fb1a802dc0600d1dee8cc5df85919a85cd6da3111fb47b2ba22d6e4889ea`).
The earlier freeze (`a561191c…` / `7a9f0f1f…`) remains recorded as previously
ACCEPTED; the current ACCEPT covers v0.1.1 and the release gate now passes.
