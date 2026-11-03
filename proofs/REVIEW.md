# Proof review record

The fresh-context reviewer must ACCEPT the frozen pairs before the gates pass.
`proofs/review-check.sh` (run by `run-gates.sh`) fails when this file is
missing, when `verdict` is not `ACCEPT`, or when either hash below no longer
matches the working tree.

verdict: ACCEPT
reviewer: fresh-context reviewer (independent of the proof author)
date: 2026-11-03
scope: 12/12 proof pairs accepted for the v0.1.3 freeze — audio, integrations, media, pages, queue, radio, recognition, settings, stream, together, ui, util (296 verified, 0 errors)
gate: reviewed the v0.1.3 UI/motion re-freeze on top of the accepted v0.1.2 tree: recomputed every changed `SOURCE.md` source hash, checked the frozen pair delta against git HEAD (`proofs/ui/original.rs` +95/-0, `proofs/ui/candidate.rs` +132/-0; the other 11 pairs untouched), re-derived `original.rs` from `candidate.rs` with `derive_original.py`, re-read `play_starts_radio` and `PlaylistOrder::sort_mode` against `crates/ui/src/pure.rs`, ran `run-gates.sh` (only the stale review record failed), each `proofs/<area>/gate`, `check-drift.sh` (21/21), `freeze-check.sh` (37 files), `spec_guard.py`, its self-test and the trust scan; re-ran `run-gates.sh` after signing.
independent_review: ACCEPT (fresh context, 2026-11-03; all 12 pairs). `pure::play_starts_radio(song_count, source, play_all)` is the source predicate `!play_all && song_count == 1 && source == QueueSource::List` verbatim in `play_starts_radio_spec`; the exec body tests the fieldless six-variant `QueueSource` by variant (all six real variants, same order as `ml_core::QueueSource`), which is the same decision as the source's derived `PartialEq`; the ensures add `r ⇒ one song`, `r ⇒ List`, `play_all ⇒ !r`, `0 songs ⇒ !r`, and `lemma_play_all_is_never_radio`, `lemma_radio_needs_exactly_one`, `lemma_radio_needs_list` are sound for every variant. `PlaylistOrder::sort_mode` transcribes the private source `fn sort_mode(self) -> Option<SortMode>` arm for arm (`Default` ⇒ `None`, the four orders to their own mode, no `Album` arm); `lemma_default_only_none` and `lemma_album_unreachable` hold over all five orders. Model reductions (enums to variants, `#[derive(Default)]` not modelled, labels/sorters/progress helpers out of subset) are the ones documented in `proofs/ui/SOURCE.md`. `candidate.rs` adds only five ghost lemmas; `original.rs` carries no invariant/assert/proof block, regenerates byte-identically for `ui`/`together`/`radio`/`recognition`, compiles with 28 verified / 20 expected scaffold errors, and Lynette `additions` + `compare`, `spec_guard.py` and `trust_scan.py` are clean. Every previously proven body is unchanged: `crates/ui/src/pure.rs` (`692cda7e…`) is additive apart from three `use` lines, `crates/core/src/settings.rs` (`e3a390e7…`) only gains `reduce_motion` + its default (`seed_rgb` L154–L160 verbatim), `crates/ui/src/app.rs` (`4a5fbf4f…`) keeps `url_route` byte-identical at L160–L168, and `services.rs`/`lib.rs`/`downloads::model` are byte-unchanged. Counts: audio 15, integrations 13, media 4, pages 7, queue 35, radio 12, recognition 26, settings 7, stream 19, together 64, ui 84, util 10.
freeze_sha256: 17e030a70a3b9eca90aff1bfb5b7a9b666dc76f424c09e20665c5b13feffa8e3
gate_sha256: 09f035e735f511c1f4b097e2f003a3732056a9f034793cf31f3901cd9759ac4b

Regenerate both hashes and re-sign with a fresh review after any change to the
gate scripts or to `FREEZE.sha256`.

## Residual trust (explicit)

This file is a paper trail, not a proof. A prover who edits the gate scripts
(including `review-check.sh` and this file) can make any candidate pass and
recompute both hashes. The controls are independent CI on the pinned toolchain
plus a fresh reviewer re-signing after any gate change. `gate_sha256` only
asserts "these exact gate bytes were present when the record was signed".
`SOURCE.md` remains a hand transcription of the frozen revision; the proofs
cover the model, not the compiled binary. The two new obligations are proven
about the frozen model: `QueueSource`/`SortMode`/`PlaylistOrder` are reduced to
their variants, so the `QueueSource` derives other than `PartialEq`/`Eq` and
`#[derive(Default)]` are pinned by the unit tests, not proven. The display
sorters (`sort_songs`, `playlist_songs`, `sort_albums`, `sort_artists`,
`sort_playlists`), the `label` tables, the four `f32` progress helpers,
`motion.rs`, `screens::search::pill_geometry` and the rest of the UI tranche
(`app.rs` egui/side-effect code) are OUT-OF-SUBSET and are not proofs. The
earlier exclusions stand: `sapisidhash`, the stream-mute plumbing, keep-stream
state machine, lifecycle code and f32 vinyl helpers are pinned by tests only.

Documentation erratum (non-blocking, disclosed rather than silently accepted):
`proofs/radio/SOURCE.md` (re-freeze note) and `docs/PROOFS.md` (v0.1.3 section)
locate the verbatim `url_route` at "L157–L165"; in the frozen `app.rs`
(`4a5fbf4f…`) it is L160–L168 (the body is verifiably byte-identical, so only
the locator is wrong). `docs/PROOFS.md` is not gate-hashed; the `SOURCE.md`
parenthetical should be corrected in the next re-freeze. `docs/SOURCE-FREEZE.sha256`
was deliberately not regenerated by this freeze; release provenance is owned by
the release step and is outside this review.
