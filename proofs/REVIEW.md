# Proof review record

The fresh-context reviewer must ACCEPT the frozen pairs before the gates pass.
`proofs/review-check.sh` (run by `run-gates.sh`) fails when this file is
missing, when `verdict` is not `ACCEPT`, or when either hash below no longer
matches the working tree.

verdict: ACCEPT
reviewer: fresh-context reviewer (independent of the proof author)
date: 2026-11-02
scope: 12/12 proof pairs accepted — audio, integrations, media, pages, queue, radio, recognition, settings, stream, together, ui, util
gate: reviewed the shutdown/leak re-freeze on top of the previously accepted 12-pair tree; the four changed source files were re-read at the proof boundaries, the proof bodies are unchanged, and the reviewer re-ran the full gate.
independent_review: ACCEPT (session `proof-final-review`, 2026-11-02, fresh context; all 12 pairs) — re-ran all 12 `proofs/<area>/gate` runs (274 verified, 0 errors; ui 65), `check-drift.sh` (21/21), `freeze-check.sh` (37 frozen files), `spec_guard_selftest.py` (12 clean pairs, 12 mutations rejected, 2 freeze tampers), and the final `run-gates.sh` after signing. Re-read the four changed sources: `controller.rs` shutdown addition with `sleep_window_contains` and `set_sleep_timer` unchanged; `engine.rs` shutdown/PDEATHSIG/ChildSlot/socket-sweep additions with `Ipc::request` still `next_id.wrapping_add(1)` -> `request_id`; `app.rs` signal/shutdown additions with `url_route` unchanged; `services.rs` `Services::shutdown` with `radio_event`/`radio_fallback` unchanged. `ui/pure.rs` and `downloads/model.rs` are unchanged, as are all proof candidates/originals. The lifecycle additions are explicitly OUT-OF-SUBSET in `audio/SOURCE.md` and `radio/SOURCE.md`; `docs/SOURCE-FREEZE.sha256` is the 246-file tree including `signals.rs`. No pair or gate script was edited by the reviewer; reviewer writes were `REVIEW.md` and the release-status wording in `docs/PROOFS.md`.
freeze_sha256: a561191c669fa98c97ef384bec14aa952f9023c2643cfa8fcfb28232b55c664d
gate_sha256: 7a9f0f1fc836f0d7999e0796a4a990edadef4ee931f01d157e87b23d467fd318

Regenerate both hashes and re-sign with a fresh review after any change to the
gate scripts or to `FREEZE.sha256`.

## Residual trust (explicit)

This file is a paper trail, not a proof. A prover who edits the gate scripts
(including `review-check.sh` and this file) can make any candidate pass and
recompute both hashes. The controls are independent CI on the pinned toolchain
plus a fresh reviewer re-signing after any gate change. `gate_sha256` only
asserts "these exact gate bytes were present when the record was signed".
`SOURCE.md` remains a hand transcription of the frozen revision; the proofs
cover the model, not the compiled binary. The shutdown/PDEATHSIG/socket-sweep
lifecycle code is outside the Verus subset; `socket_owner_pid` is pinned by its
unit test, not a proof. `docs/RELEASE-CHECK.md` still warns that the shipped
AppImage predates this shutdown fix and must be rebuilt; that is an artifact
release issue, not a proof-gate issue.
