# Proof source freeze — queue state machine

Frozen 2026-11-02. Proof: `candidate.rs` (proven), scaffold: `original.rs`
(same specs + exec, proof annotations removed).

## Frozen source

| source | sha256 |
|---|---|
| crates/core/src/queue.rs | d8cf9a5df28f8665a74d1a53d6426128454956b3ff5ac83ccac7b1f00dab1315 |

## Toolchain

- Verus 0.2026.11.02.8bf68ee, bundled Rust `1.99.1-x86_64-unknown-linux-gnu`
- Workspace `rust-toolchain.toml`: 1.99.1
- Lynette: upstream `ed5855f`, rebuilt against `verus_syn`/`verus_prettyplease`
  from Verus tag `release/0.2026.11.02.8bf68ee` (see `../verus.env`)

## How the model maps to the real code

- `Vec<Song>` becomes `Vec<u32>`: song identity is opaque to the queue
  invariants, only indices matter. `Song` is a serde struct and outside the
  Verus subset.
- `&mut self` methods are kept; postconditions use `final(self)` / `old(self)`.
- `song_at`'s `order.get(..).and_then(..)` becomes explicit bounds checks; same
  result (`None` for out-of-range or dangling indices).
- `(0..len).collect()` becomes a push loop; `for i in &mut order` becomes an
  index loop; `.extend(rest)` becomes an index loop. Same effects.
- `permute`'s `let Some(current) = ... else { identity }` becomes the equivalent
  `if self.order.is_empty()` branch; `rest`'s `.filter(..).collect()` becomes a
  push loop. Fisher-Yates order and `lcg` are copied verbatim.

## Proved obligations

`inv` (order is a permutation of `0..songs.len()`, position bounds),
`next`/`previous` repeat semantics (Off stops without moving, All wraps, One
stays), `remove` successor/previous selection + index remap + pointwise order,
`move_item` index remap, `permute` keeps the current track first and yields a
permutation, `set_shuffle`/`reshuffle`/`replace_songs`, `insert_next`, `append`.
