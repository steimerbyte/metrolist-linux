# integrations — frozen source

## Source

| file | sha256 (at freeze) |
|------|--------------------|
| crates/integrations/src/mapping.rs | 2fa791a9d1ccde903fa147f42fb406b263d43b973200ee2c10dfe0f0d628b9fc |

Toolchain: Verus `0.2026.11.02.8bf68ee` (tag `release/0.2026.11.02.8bf68ee`),
bundled Rust `1.99.1-x86_64-unknown-linux-gnu`; workspace pin `1.99.1`.
Lynette gate: `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`.

## Scope

The dependency-free MPRIS/Discord mapping core of `ml-integrations`:
`transport_status`, `transport_loop`, `repeat_from_loop`, `ms_to_us`,
`us_to_ms`, `position_ms_from_us`, `can_go_next`, `can_go_previous`.

## Model deviations

- `PlaybackStatus`, `RepeatMode`, `TransportStatus`, `TransportLoop` are
  re-declared with the same variants; `mpris.rs`'s conversion to the
  `mpris-server` types is a total match and out of subset (D-Bus crate).
- `PlayerSnapshot` is reduced to the four fields these predicates read
  (`current`, `queue_index`, `queue_len`, `repeat`) and passed by value; the
  source borrows `&PlayerSnapshot`, which changes no decision.
- `can_go_next`'s `Off` arm is `index.checked_add(1).is_some_and(|next| next < queue_len)`.
  `is_some_and` has no Verus spec and `assume_specification` is banned, so the
  exec is transcribed as `match index.checked_add(1) { Some(next) => next < queue_len, None => false }`
  (identical value; `checked_add` itself is supported). The spec is the plain
  integer comparison `(index as int) + 1 < (queue_len as int)`; the agreement
  with the exec transcription is **not assumed**: `lemma_next_checked_add` in
  `candidate.rs` proves
  `((index as int) + 1 < queue_len) <==> (index != usize::MAX && index + 1 < queue_len)`,
  i.e. `checked_add` cannot change the answer (`None` only at
  `index == usize::MAX`, where the plain comparison is false because
  `queue_len <= usize::MAX`). `lemma_next_wraps` covers the `All`/`One` arms.
- `ms_to_us`: `i64::try_from(ms).unwrap_or(i64::MAX)` is transcribed as an
  explicit `if ms > i64::MAX as u64 { i64::MAX } else { ms as i64 }` (same
  clamp, no `TryFrom` adapter), and `saturating_mul(1000)` as an explicit
  guard on `i64::MAX / 1000`.
- `us_to_ms` keeps Rust's truncation-toward-zero for negative dividends via
  the `trunc_ms` spec (Verus `int` division is Euclidean).
- `position_ms_from_us` clamps negative offsets to 0 before dividing, which
  is *not* truncation; the `position_ms` spec encodes the clamp.
- `volume_of` is out of subset: `f64`/`is_nan` are not Verus values. Its
  contract (muted ⇒ 0.0, NaN ⇒ 0.0, otherwise `0.0..=1.0`) is CANDIDATE.
- `track_path`'s hex loop and `track_metadata`/`snapshot_changes` (String
  concatenation, `Vec` collection, field diffing) are CANDIDATE.

## Proven

All eight functions equal their spec transcription; plus
`lemma_loop_roundtrip` / `lemma_loop_roundtrip_inv` (`transport_loop` and
`repeat_from_loop` are mutually inverse bijections), `lemma_next_wraps`
(All/One ⇒ both predicates hold whenever a track is current),
`lemma_next_checked_add` (`checked_add` cannot change the answer) and
`lemma_exact_us` (`ms_to_us` is exact below the saturation point).

Re-freeze log: 2026-11-02 — source gained the `usize::MAX` `checked_add` fix in
`can_go_next`; model and spec updated to the new semantics, hash above.
