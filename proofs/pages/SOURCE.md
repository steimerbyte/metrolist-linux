# pages — frozen source

## Source

| file | sha256 (at freeze) |
|------|--------------------|
| crates/innertube/src/pages/next.rs | 0c354a0565f2c1c4214dd60dc003715f25d1c0a0084e69a425c8e0d901a98124 |

Toolchain: Verus `0.2026.11.02.8bf68ee` (tag `release/0.2026.11.02.8bf68ee`),
bundled Rust `1.99.1-x86_64-unknown-linux-gnu`; workspace pin `1.99.1`.
Lynette gate: `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`.

## Scope

`pages::next::append(page: &mut QueuePage, next: QueuePage) -> usize` — the
continuation-page append used by `queue_append`: every song of the fetched
page whose id is not already queued is appended in server order, the existing
order and `current_index` are untouched, `page.continuation` takes the next
token, and the return value is the number of appended songs.

The rest of `next.rs` (`panel`, `title`, `owner`, `songs_from_panel`,
`from_response`, `from_continuation`) is `serde_json::Value` parsing and is
out of subset. The async `queue_append` client method is reqwest/async and is
out of subset; only the pure append is modelled.

## Model deviations

- `Song.id` (`String`) is modelled as a `u32` id token; `Song` is not a Verus
  value. Only id equality/order is proven, which is exactly what dedup and
  ordering depend on.
- `QueuePage` is modelled as `Page { songs: Vec<u32>, current_index:
  Option<usize>, continuation: Option<u32> }`; `title` is out of subset and
  omitted, the continuation `String` token becomes `Option<u32>`.
- The real `for song in next.songs` is transcribed as an indexed `while`
  over `0 .. next.songs.len()` with the same order and the same dedup
  predicate. Verus for-loop invariants do not expose the consumed suffix, so
  the fold cannot be related to its post-state with the `for` form.
- The real predicate `page.songs.iter().any(|queued| queued.id == song.id)`
  is transcribed as `contains_id(&page.songs, song)`; Verus has no spec
  linking `Iterator::any` to `Seq::contains`, so `contains_id` is an exec
  helper proven to return `ids@.contains(id)`.
- `usize` length subtraction is guarded by the proven monotonicity fact
  (`lemma_len_monotone`), so the transcription cannot underflow on a queue
  that only grows.

## Proven

- `append`: equals `append_ids(old(page).songs@, next.songs@)` — exact order,
  dedup (repeat ids inside `next` are dropped after the first), count
  `r == new.len() - old.len()`, `continuation == next.continuation`,
  `current_index` unchanged.
- `contains_id`: returns `ids@.contains(id)` (indexed scan with a forall
  invariant over the scanned prefix).
- `lemma_len_monotone`: `append_ids` never shrinks the queue.
- `lemma_no_duplicates`: a duplicate-free queue stays duplicate-free after
  the append.
