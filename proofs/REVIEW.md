# Proof review record

The fresh-context reviewer must ACCEPT the frozen pairs before the gates pass.
`proofs/review-check.sh` (run by `run-gates.sh`) fails when this file is
missing, when `verdict` is not `ACCEPT`, or when either hash below no longer
matches the working tree.

verdict: ACCEPT
reviewer: fresh-context reviewer (independent of the proof author)
date: 2026-11-27
scope: 12/12 proof pairs accepted for the third 2026-11-27 re-freeze of `radio`: `app.rs` gained only the `DemoMode::Loading` plumbing (skip the boot `load_home` in `App::new`, readiness arm in `after_frame`), so the delta is hash-only for the proof and `url_route` plus every other proven body are byte-identical. This layers on the accepted same-day re-freeze of audio/radio/settings/stream (seven declared sources) and the audio re-prove of the three-argument `should_clear_stream_mute` (297 verified, 0 errors)
gate: `check-drift.sh` 21/21 declared sources match; `freeze-check.sh` 37 frozen files; all 12 area gates PASS (297 verified, 0 errors: audio 16, integrations 13, media 4, pages 7, queue 35, radio 12, recognition 26, settings 7, stream 19, together 64, ui 84, util 10); step-4 `spec_guard` loop clean for 12/12 pairs, `spec_guard_selftest.py` 12 clean pairs accepted / 12 mutations rejected / 2 freeze tampers rejected, `trust_scan.py` PASS 12/12; byte-compared every proven body against `git show HEAD` (identical sha256): `app.rs::url_route` (L164) plus the `pasted_urls_route_like_upstream` MPRE test, `services.rs::radio_event`/`radio_fallback`, `innertube/lib.rs` `SearchFilter::params`/`search_body`, and the `ParsedUrl`/`Route` types with their defining files untouched; `git diff --stat HEAD -- proofs/` touched only `FREEZE.sha256`, the audio pair, the four `SOURCE.md` and this file; Verus `0.2026.11.02.8bf68ee`, Lynette `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`; full `run-gates.sh` exit 0 (ALL GATES PASS, 12 proofs)
independent_review: ACCEPT (fresh context, 2026-11-27; all 12 pairs). The audio re-prove transcribes the frozen `engine.rs::should_clear_stream_mute` body verbatim; `clear_stream_mute_spec` and the five-row `lemma_stream_mute_table` match, and `lemma_muted_stays_muted`/`lemma_no_output_never_unmutes` are sound for every boolean. The stream note now correctly places `parse_visitor_data`/`cookie_value` outside the subset, and no other proven body changed. Third pass: the dither demo-mode delta lives in `App::new`/`after_frame`, outside the route layout; `url_route`, the radio event block, the search body and their input types are byte-identical to the v0.1.3 revision.
freeze_sha256: d153fff8aeac121cfa29fb7375c6a2adb20c866962a4b3e7d6990873db76338c
gate_sha256: e53f35d28b820590e8d6c1ac8f0af65c4f993b37cc45e73e6c771c7e823617c2

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
