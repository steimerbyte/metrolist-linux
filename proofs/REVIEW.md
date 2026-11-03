# Proof review record

The fresh-context reviewer must ACCEPT the frozen pairs before the gates pass.
`proofs/review-check.sh` (run by `run-gates.sh`) fails when this file is
missing, when `verdict` is not `ACCEPT`, or when either hash below no longer
matches the working tree.

verdict: ACCEPT
reviewer: fresh-context reviewer (independent of the proof author)
date: 2026-11-02
scope: 12/12 proof pairs accepted — audio, integrations, media, pages, queue, radio, recognition, settings, stream, together, ui, util
gate: reviewed the v0.1.1 stream-mute/keep-stream re-freeze on top of the previously accepted shutdown/leak tree; the four moved sources were re-read at the proof boundaries, the audio candidate gained the new pure predicate/lemmas without changing any existing exec body, and the reviewer re-ran the full gate.
independent_review: ACCEPT (session `proof-final-review`, 2026-11-02, fresh context; all 12 pairs) — re-ran all 12 `proofs/<area>/gate` runs (277 verified, 0 errors; audio 15 including `should_clear_stream_mute` and its two lemmas), `check-drift.sh` (21/21), `freeze-check.sh` (37 frozen files), `spec_guard_selftest.py` (12 clean pairs, 12 mutations rejected, 2 freeze tampers), and the final `run-gates.sh` after signing. Re-read the moved sources: `controller.rs` keep-stream `is_current_entry`/`play_queue` with `sleep_window_contains` and `set_sleep_timer` still verbatim; `engine.rs` stream-mute/`clear_stream_mute`/`allow_unmute` with `Ipc::request` still `next_id.wrapping_add(1)` -> `request_id`, and the new `should_clear_stream_mute` (`allow_unmute && !intended_muted`) transcribed exactly; `app.rs` Settings-nav/demo additions with `url_route` unchanged; `pure.rs` additive f32 vinyl helpers only, with the pair's 16 source functions unchanged and `downloads/model.rs` unchanged. `services.rs` and `innertube/lib.rs` are unchanged. The audio pair's ghost lemmas are now one trailing block; `original.rs` regenerated and verifies 5, `candidate.rs` 15. The stream-mute plumbing, keep-stream state machine and lifecycle code are explicitly OUT-OF-SUBSET/CANDIDATE; `docs/SOURCE-FREEZE.sha256` is the 246-file v0.1.1 tree including `signals.rs` and `providers.rs`. No pair or gate script was edited by the reviewer; reviewer writes were `REVIEW.md` and the v0.1.1 release-status wording in `docs/PROOFS.md`.
freeze_sha256: 1a07e2b574959b8316e97dc64fa1d407b614e07fea315e895bd2d86b45c01676
gate_sha256: 9ce7fb1a802dc0600d1dee8cc5df85919a85cd6da3111fb47b2ba22d6e4889ea

Regenerate both hashes and re-sign with a fresh review after any change to the
gate scripts or to `FREEZE.sha256`.

## Residual trust (explicit)

This file is a paper trail, not a proof. A prover who edits the gate scripts
(including `review-check.sh` and this file) can make any candidate pass and
recompute both hashes. The controls are independent CI on the pinned toolchain
plus a fresh reviewer re-signing after any gate change. `gate_sha256` only
asserts "these exact gate bytes were present when the record was signed".
`SOURCE.md` remains a hand transcription of the frozen revision; the proofs
cover the model, not the compiled binary. The stream-mute plumbing, keep-stream
controller state machine, shutdown/PDEATHSIG/socket-sweep lifecycle code and
f32 vinyl helpers are outside the Verus subset and are pinned by unit/integration
tests, not proofs. `docs/RELEASE-CHECK.md` still warns that the shipped AppImage
predates the v0.1.1 fixes and must be rebuilt; that is an artifact-release issue,
not a proof-gate issue.
