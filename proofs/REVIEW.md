# Proof review record

The fresh-context reviewer must ACCEPT the frozen pairs before the gates pass.
`proofs/review-check.sh` (run by `run-gates.sh`) fails when this file is
missing, when `verdict` is not `ACCEPT`, or when either hash below no longer
matches the working tree.

verdict: ACCEPT
reviewer: fresh-context reviewer (independent of the proof author)
date: 2026-11-27
scope: 12/12 proof pairs accepted for the fourth 2026-11-27 re-freeze of `radio`: `app.rs` gained only the queue-thumbnail wiring (`seed_demo_art` uploads a procedural `ArtImage` for `DEMO_ART_URL`, `demo_song`/`demo_queue` carry that thumbnail with the last of three demo rows dropped, and `queue_overlay` passes `&mut self.art` into the queue screen), so `url_route` (L164), its `pasted_urls_route_like_upstream` MPRE test and every other proven body are byte-identical. This layers on the accepted same-day re-freezes of audio/radio/settings/stream and the audio re-prove of the three-argument `should_clear_stream_mute` (297 verified, 0 errors)
gate: `check-drift.sh` 21/21 declared sources match; `freeze-check.sh` 37 frozen files; all 12 area gates PASS (297 verified, 0 errors: audio 16, integrations 13, media 4, pages 7, queue 35, radio 12, recognition 26, settings 7, stream 19, together 64, ui 84, util 10); step-4 `spec_guard` loop clean for 12/12 pairs, `spec_guard_selftest.py` 12 clean pairs accepted / 12 mutations rejected / 2 freeze tampers rejected, per-pair `trust_scan.py` PASS 12/12; byte-compared every proven body against `git show HEAD` (identical sha256): `app.rs::url_route` (L164) plus the `pasted_urls_route_like_upstream` MPRE test, `services.rs::radio_event`/`radio_fallback` (file untouched), `innertube/lib.rs` `SearchFilter::params`/`search_body` (file untouched); `git diff --stat HEAD -- proofs/` touched only `FREEZE.sha256` and `radio/SOURCE.md` (hash + queue-thumbnail re-freeze note); Verus `0.2026.11.02.8bf68ee`, Lynette `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`; full `run-gates.sh` exit 0 (ALL GATES PASS, 12 proofs)
independent_review: ACCEPT (fresh context, 2026-11-27; all 12 pairs). Fourth pass: the queue-thumbnail delta is demo-only art seeding plus the queue screen's art wiring, outside the route layout; `url_route`, the radio event block, the search body and their input types are byte-identical to the v0.1.3 revision, and the previously accepted audio/stream/settings obligations are unchanged. The `radio` re-freeze note now records the delta and the re-read that produced it.
freeze_sha256: b20cee1e4510b3e940c9aec71c6b828b5c6960e771505889debd17e857974e58
gate_sha256: 3f0a5e7fe8af6b4497d000aced1e2b9561b6c1e5d815aab0629c4e762e4c210c

Regenerate both hashes and re-sign with a fresh review after any change to the
gate scripts or to `FREEZE.sha256`.

## Residual trust (explicit)

This file is a paper trail, not a proof. A prover who edits the gate scripts
(including `review-check.sh` and this file) can make any candidate pass and
recompute both hashes. The controls are independent CI on the pinned toolchain
plus a fresh reviewer re-signing after any gate change. `gate_sha256` only
asserts "these exact gate bytes were present when the record was signed".
`SOURCE.md` remains a hand transcription of the frozen revision; the proofs
cover the model, not the compiled binary. The re-proven `should_clear_stream_mute`
obligation is proven about its three boolean arguments; the `ao_open` state
machine that feeds them (`file-loaded`/`audio-reconfig` set it, `start-file`/
`end-file`/`idle` clear it, and `handle_message`'s benign-error filter) is
OUT-OF-SUBSET and pinned by unit tests only. The v0.1.3 obligations
(`QueueSource`/`SortMode`/`PlaylistOrder`, reduced to their variants) remain as
accepted: the derives other than `PartialEq`/`Eq` and `#[derive(Default)]` are
pinned by unit tests, not proven. The display sorters (`sort_songs`,
`playlist_songs`, `sort_albums`, `sort_artists`, `sort_playlists`), the `label`
tables, the four `f32` progress helpers, `motion.rs`,
`screens::search::pill_geometry` and the rest of the UI tranche (`app.rs`
egui/side-effect code) are OUT-OF-SUBSET and are not proofs. The earlier
exclusions stand: `sapisidhash`, the stream-mute plumbing, keep-stream state
machine, lifecycle code and f32 vinyl helpers are pinned by tests only.

Documentation erratum (resolved): the v0.1.3 `url_route` locator is corrected
in `proofs/radio/SOURCE.md` (2026-11-27 note: L164); `docs/PROOFS.md` still
carries the v0.1.3 record (L160–L168 for `app.rs` `4a5fbf4f…`) and is not
gate-hashed. `docs/SOURCE-FREEZE.sha256` was regenerated in the 2026-11-27
tree; it tracks release provenance and is outside this review.
