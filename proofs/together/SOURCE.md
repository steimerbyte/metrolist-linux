# together — frozen source

## Source

| file | sha256 (at freeze) |
|------|--------------------|
| crates/together/src/clock.rs | 5c4bda35dd5f7e21a5dd1b6be4c262013ddedec2b5200799440a2344d75dab0c |
| crates/together/src/sync.rs | 77ec42f205ebcf7af93313d41201ef62f461ec808218bca1f6f6f6fbc6621ec0 |
| crates/together/src/codec.rs | 0dccfac4c32a8dc391398c4f247f8da9cb18444614321718a287f9857fc92c1b |
| crates/together/src/model.rs | 21118a72019f5a5592c38070fb4c4432575e3086e1c016be93f33f0653b37319 |

Toolchain: Verus `0.2026.11.02.8bf68ee` (tag `release/0.2026.11.02.8bf68ee`),
bundled Rust `1.99.1-x86_64-unknown-linux-gnu`; workspace pin `1.99.1`.
Lynette gate: `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`.

## Scope

The deterministic policy layer of Listen Together, the part that decides what
the player does (not the part that talks to the server):

- `clock.rs` — `ServerClock`: sample acceptance guards, best-RTT tracking,
  `has_offset`/`best_round_trip_ms`, `now_at` presence, and the guard/saturation
  arithmetic of `position_at`.
- `sync.rs` — `drift_decision`, `should_seek_during_active_playback`,
  `position_at`, `RevisionGate::{last,reset,set,accept}`, the integer backbone
  of `backoff_delay_ms` (`backoff_base`), `should_reconnect`, and
  `canonical_playback_queue`.
- `codec.rs` — `should_compress` and the envelope compression policy.
- `model.rs` — `song_to_track_info` / `track_info_to_song` duration, artist,
  album and thumbnail mapping.

Out of subset and not covered: the WebSocket client, prost/protobuf encode and
decode, gzip (`flate2`), the `RoomSession` state machine, `session.rs`,
`client.rs`, `messages.rs`, and the `Queue` traversal in `queue_to_room` /
`queue_from_room` (`Queue::iter()`/`skip()` iterator adaptors have no vstd
spec – see "Not covered" below).

## Model deviations

- **f64.** vstd has no float reasoning. `ServerClock.server_offset_ms` keeps
  its real `Option<f64>` type, `record_pong` is transcribed with the real f64
  filter expression, and only the *guards*, the RTT bound, the min-tracking and
  the first-sample flag are proven; the filtered offset value is opaque.
  `now_at`'s f64→i64 cast is likewise opaque; its *presence* (`Some` iff an
  offset was recorded) is proven, and the position tail it feeds is proven
  separately as `saturating_advance` in exact integers (the same expression,
  factored because the f64-derived `server_now` cannot be reasoned about).
- **`Option::filter`, `saturating_add/sub`, `unsigned_abs`, `then_some`,
  `clamp`, `str::starts_with`** are transcribed as the equivalent `match` /
  guarded if-chains (same results, total). `i64::MIN + delta` is never formed.
- **`drift_decision`** is `target - current`; the transcription carries the
  precondition that this difference fits in `i64` (±2^60 ms bounds), because
  Verus requires the overflow proof the real code leaves implicit (a debug
  build would panic on such an input).
- **`DriftDecision::Speed(f32)`** becomes `Speed(i32)` percent (`102`/`98`):
  `1.0 ± DRIFT_CORRECTION_SPEED`. The player only consumes the multiplier.
- **`backoff_delay_ms`** is transcribed as `backoff_base(attempt)` (the real
  `2u64 << min(attempt-1, 4)` written as the equivalent 5-entry table, because
  Verus's bit-vector tactic does not reason about a variable shift) plus
  `base * jitter_percent / 500`; `jitter_percent` is the integer model of
  `jitter_sample * 100` (`0.2 * p / 100 = p / 500`). The real argument is an
  `f64` in `[0, 1)`; its truncation is out of subset, the bound is proven for
  the integer model.
- **`canonical_playback_queue`** takes `current` by value and the `Vec<u32>`
  id list (real code: `&proto::TrackInfo`); ids are opaque `u32` tokens.
  `canonical_prefix` is the prefix-length form of `canonical_fold`, chosen so
  the loop invariant unfolds in one step.
- **`Song`/`TrackInfo`** are reduced to id plus the four converted fields.
  Strings are modelled by presence plus an opaque symbol: `Option<u32>` is
  "empty string" (None) vs "non-empty, symbol s" (Some(s)). Byte content and
  the `", "` join are out of subset; presence, identity and the millisecond
  conversion are proven.

## Proven

- `song_to_track_info` / `track_info_to_song`: field-wise mapping; seconds →
  ms is `*1000`, ms → seconds is `/1000` clamped at `u32::MAX`; non-positive
  wire durations decode as `None`; `lemma_duration_round_trip` (every real
  duration round-trips) and `lemma_zero_second_duration_is_absent` (`Some(0)`
  seconds is the one duration that decodes as "unknown").
- `sample_accepted`: the exact guard chain; `lemma_stale_sample_rejected`
  (age > 60 s rejected), `lemma_age_bound_inclusive` (exactly 60 s accepted).
- `network_round_trip` / `best_after`: accepted samples have RTT in
  `[0, 60_000]` (`lemma_network_rtt_bounded`); best RTT is the running minimum
  and never grows (`lemma_best_after_is_min`), and a first sample replaces the
  `u64::MAX` sentinel (`lemma_first_sample_records_best`).
- `ServerClock::{new,reset,has_offset,server_offset_ms,best_round_trip_ms}` and
  `record_pong`: returns `true` iff the sample was accepted and it was the
  first; an accepted sample always sets an offset; a rejected sample changes
  nothing; `now_at` is `Some` iff an offset exists.
- `position_at` (clock): unchanged when paused, without an anchor (`None` or
  `<= 0`) or before the first pong; never below `position`.
- `saturating_advance`: the exact saturating tail; `lemma_advance_no_rewind`,
  `lemma_advance_nothing_to_add` (future anchor adds nothing),
  `lemma_advance_monotone`, `lemma_advance_saturates`.
- `drift_decision`: Hold below/at 50 ms, Seek at/above 750 ms, ±2 % Speed
  between them with the sign of the drift; `should_seek_during_active_playback`:
  host heartbeat seeks only above the 2 s tolerance.
- `RevisionGate`: revision 0 always accepted and state untouched; stale
  revisions rejected and the gate never moves backwards; fresh revisions
  accepted and become the high-water mark; re-presenting the current revision
  is accepted; `lemma_no_replay` (an accepted revision is rejected afterwards
  until `set`/`reset`); `set` reopens exactly its revision.
- `backoff_base`: equals the table (`lemma_backoff_table`, 2 s, 4 s, 8 s,
  16 s, 32 s, sticky above attempt 5); `lemma_backoff_far_below_nominal_cap`
  (the 120 s cap never binds – it is dead code); `lemma_backoff_monotone`;
  `lemma_jitter_bounded` (≤ 38.4 s with 20 % jitter).
- `should_reconnect`: `attempts < 15`, with the boundary and monotonicity
  lemmas.
- `canonical_playback_queue`: result equals `canonical_ids`, is non-empty and
  starts with the current track; ids already queued are not repeated and
  upcoming ids keep server order.
- `should_compress`: `compressed < original`; the envelope never inflates,
  and nothing at or below the 100-byte threshold is compressed
  (`lemma_compression_never_inflates`,
  `lemma_small_payload_never_compressed`, `lemma_equal_size_not_compressed`).

## Not covered (deliberate)

- `queue_to_room` / `queue_from_room`: the real bodies are
  `queue.iter().skip(queue.position() + 1)` and
  `Queue::new(canonical_playback_queue(..))`, i.e. an iterator adaptor chain
  over `Queue` plus the `Song` conversion. The `Queue` invariants are already
  frozen in `proofs/queue`; re-modelling `Queue` here would duplicate them
  (and the `iter()/skip()` spec is missing from vstd). The dedup/order/count
  core they rely on — `canonical_playback_queue` — is proven above.
- The f64 filter value, gzip, prost, WebSocket, `RoomSession`, and every
  async/IO path in the crate.
