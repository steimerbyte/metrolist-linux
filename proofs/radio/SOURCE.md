# radio — frozen source

## Source

| file | sha256 (at freeze) |
|------|--------------------|
| crates/ui/src/services.rs | 1434dc75dd41e13d276b596839716aa3bf2202198f86eebcc7a1df1a3402a3f5 |
| crates/ui/src/app.rs | d6d8f5a17c5c1afbba1fb62203bab9b6ce3bea464b436766d628a7d5de8de7cb |
| crates/innertube/src/lib.rs | bfa89ea531da62a31afc59bee4978d51606f9d372c72a3c325a21679650a8890 |

Toolchain: Verus `0.2026.11.02.8bf68ee` (tag `release/0.2026.11.02.8bf68ee`),
bundled Rust `1.99.1-x86_64-unknown-linux-gnu`; workspace pin `1.99.1`.
Lynette gate: `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`.

Scope inside these files: `radio_event` and `radio_fallback`
(`crates/ui/src/services.rs`), `url_route` plus its `MPRE` prefix test
(`crates/ui/src/app.rs`), and `search_body` with `SearchFilter::params`
(`crates/innertube/src/lib.rs`). Everything else in those files — the app
shell, the session/queue plumbing, the HTTP client, auth, cookies, protobuf
helpers — is out of scope and only frozen for drift.

Re-freeze 2026-11-02 (release freeze): crates/ui/src/{app,services}.rs kept
growing while the account/audio/catalog UI landed. The proven bodies were
re-read against the frozen revision and are verbatim (radio_event,
radio_fallback, url_route); the hashes above are that revision's. Only
surrounding code changed. Everything else in these files is outside this area.

## Scope notes (what changed when)

- `services.rs`: the radio event block. `radio_event` maps a `next` queue page
  onto `UiEvent::RadioReady`; the clicked track wins over the panel's
  `current_index`, which is only a fallback, and the index falls back to 0.
- `app.rs`: only `url_route`. `ui-spacing` also changed `center_column`
  (layout only, no proof relevance); `search-radio` owns the search wiring.
  The shutdown/leak fix later added `App::shutdown`/`App::exit`, the
  signal wake-up in `logic`, the `cli_exit` atomic hand-off for the screenshot
  callback, and `Drop for App` — all out of subset, `url_route` untouched.
  v0.1.1 then added Settings nav routing through `shell::{NavAction,
  active_tab, settings_active}` (`apply_nav`), the single-fixture player demo
  (`start_player_demo`, `DEMO_SONG_ID`) and stricter screenshot settle gating —
  again out of subset; `url_route` and its call sites are unchanged.
- `lib.rs`: `SearchFilter`, `SearchFilter::params` and `search_body` are the
  proven items; `cookie-auth` landed the cookie transport in the same file
  (frozen at its `bfa89ea5…` stable point, search body untouched).

## Model deviations

- **Song / page identities** are opaque `u32` tokens; `Song`, `QueuePage`,
  `UiEvent::RadioReady` and `QueueSource` are reduced to the fields the
  functions read (the other fields are irrelevant to these functions).
- **`Vec::iter().position(..)` / `Option::filter`** are transcribed as an index
  loop plus `match` (same policy, total). `Option::or`/`unwrap_or` become the
  equivalent `match`.
- **`ParsedUrl` / `Route` ids** are `Vec<char>` instead of `String`:
  `id.starts_with("MPRE")` becomes four explicit character comparisons. For an
  ASCII prefix this is the same test on any UTF-8 string (a 4-byte prefix that
  is all ASCII can only match at a character boundary); non-ASCII byte-prefix
  corner cases are out of subset. `starts_with_mpre` is an exec helper in both
  the original and the candidate (it is part of the transcription, not ghost
  material).
- **`serde_json::Value`** cannot be modelled: the `search` request body is
  reduced to `SearchBody { query, params }`. The filter `params` strings are
  five distinct opaque tokens; the proof establishes that they are distinct
  and attached exactly when a filter is given, not their bytes.

## Proven

- `radio_event`: origin is the clicked song, the page's song list is passed
  through unchanged, the source is `Radio`; the returned index is in range for
  any non-empty page; when the clicked song appears in the page the index
  points at its **first** occurrence; otherwise it is the panel's
  `current_index` when that is in range and `0` otherwise (including empty
  pages).
- `radio_fallback`: single-track queue — origin is the clicked song, the queue
  is exactly `[song]`, index `0`, and the caller's source is preserved.
- `url_route`: video links are never routed (`None`); playlist links map to the
  playlist route; artist links to the artist route; album links open the album
  page only when the id starts with `MPRE` (`M`,`P`,`R`,`E` as the first four
  characters) and the playlist page otherwise; the route never rewrites the id.
- `is_album_browse_id` / `starts_with_mpre`: the prefix test, with
  `lemma_mpre_needs_four_chars` fixing the four-character boundary.
- `search_body`: the query is copied verbatim (`lemma_query_is_verbatim`), and
  `params` is present exactly when a filter is given
  (`lemma_no_filter_no_params`); the five filters map to five distinct tokens
  (`lemma_filter_params_distinct`); identical inputs produce the identical
  body (`lemma_same_inputs_same_body`).

## Not covered (deliberate)

- The `UiEvent` enum, its dispatch and every side effect; `App::handle_event`,
  clipboard/flash messages, and the layout around `center_column`.
- `serde_json` body construction, the HTTP layer, `InnertubeClient::post`,
  cookies/auth (separate files) and every async path in `lib.rs`.

## Re-freeze 2026-11-02 (shutdown/leak fix)

Reason: `crates/ui/src/{app,services}.rs` changed while the SIGTERM/SIGKILL
leak fix landed (new hashes in the table above; `innertube/src/lib.rs` is
unchanged at `bfa89ea5…`). The delta is process lifecycle only:
`signals::install` / `terminate_requested` wiring in `app.rs`, `App::shutdown`
and `App::exit` (every `std::process::exit` path now flushes and shuts down
first), a `cli_exit` atomic so the screenshot callback cannot borrow the app,
`Drop for App`, and `Services::shutdown` (idempotent: engines, workers,
downloads) reused by `Drop for Services`.

The proven bodies were re-read against the current files and are verbatim, so
`original.rs`/`candidate.rs` and the 12 proofs are unchanged — hash + delta
update only:

- `radio_event` — unchanged (`services.rs`): `.iter().position(...)` over the
  page songs, `.or(page.current_index.filter(|i| *i < page.songs.len()))`,
  `.unwrap_or(0)`.
- `radio_fallback` — unchanged (`services.rs`): `[song]`, `index: 0`, caller's
  source preserved.
- `url_route` — unchanged (`app.rs`): `Video` returns `None`, `Album` with the
  `MPRE` prefix is the album route, other albums/playlists/artists keep their
  route and id.
- `search_body` / `SearchFilter::params` — `lib.rs` did not change.

The new lifecycle code is out of subset (enumerated above and in `ui`'s
source); no new obligation is claimed and no proof text changed.

## Re-freeze 2026-11-02 v0.1.1 (Settings nav routing, demo fixture)

Reason: v0.1.1 routes the Settings entry from the nav rail/bottom bar
(`apply_nav` over `shell::NavAction`) and pins the screenshot demo to one
approved fixture (`DEMO_SONG_ID`, `start_player_demo`, `demo_ready` now also
requires that id, playing state, position > 1500 ms and no pending toast).
`crates/ui/src/services.rs` did not change for v0.1.1 (`17aab95c…`);
`app.rs` is now `d6d8f5a1…` (table above).

- `url_route` — unchanged, verbatim: `Video` returns `None`, `Album` with the
  `MPRE` prefix is the album route, other albums/playlists/artists keep their
  route and id.
- `radio_event` / `radio_fallback` — unchanged (`services.rs` untouched).
- `search_body` / `SearchFilter::params` — unchanged (`innertube/src/lib.rs`
  untouched at `bfa89ea5…`).

The navigation and demo-screenshot code is out of subset (UI state machine);
no proven body moved and no proof text changed.

## Re-freeze 2026-11-02 v0.1.2 (account cookie import)

Reason: `Services::import_cookie_file` now expands the pasted path through
`pure::expand_import_path` (two added lines: read `HOME`, expand). New hash is
in the table above (`1434dc75…`); `crates/ui/src/app.rs` and
`crates/innertube/src/lib.rs` are unchanged for v0.1.2.

- `radio_event` / `radio_fallback` — byte-identical; the diff has no hunk in
  the radio block.
- `url_route` — unchanged (`app.rs` untouched this round).
- `search_body` / `SearchFilter::params` — unchanged (`lib.rs` untouched).

`expand_import_path` itself is proven in `proofs/ui` (it lives in `pure.rs`);
this area only records the call site, which is out of subset.
