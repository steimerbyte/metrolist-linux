# Proof review record

The fresh-context reviewer must ACCEPT the frozen pairs before the gates pass.
`proofs/review-check.sh` (run by `run-gates.sh`) fails when this file is
missing, when `verdict` is not `ACCEPT`, or when either hash below no longer
matches the working tree.

verdict: ACCEPT
reviewer: fresh-context reviewer (independent of the proof author)
date: 2026-11-02
scope: 12/12 proof pairs accepted — audio, integrations, media, pages, queue, radio, recognition, settings, stream, together, ui, util
gate: reviewed the v0.1.2 account-cookie re-freeze on top of the previously accepted v0.1.1 tree; the moved sources were re-read at the proof boundaries, the new `expand_import_path` obligation was independently checked against `PathBuf::join`, and the reviewer re-ran the full gate.
independent_review: ACCEPT (session `proof-final-review`, 2026-11-02, fresh context; all 12 pairs) — re-ran all 12 `proofs/<area>/gate` runs (289 verified, 0 errors; ui 77 with `expand_import_path`, audio 15), `check-drift.sh` (21/21), `freeze-check.sh` (37 frozen files), `spec_guard_selftest.py` (12 clean pairs, 12 mutations rejected, 2 freeze tampers), and the final `run-gates.sh` after signing. Re-read the moved sources: `pure.rs` adds `expand_import_path` with `trim_span`/`path_join`/`copy_range`; `path_join_spec` matches measured Rust `Path::join` for empty home/rest, absolute rest, trailing separators and `.`/`..`; ASCII-only trim and Unix paths are documented deviations. `services.rs` only routes the import through `expand_import_path`; `radio_event`/`radio_fallback` have no diff. `context.rs` changes only `sapisidhash` to `ts sapisid origin` (OUT-OF-SUBSET, SHA-1/format/test-pinned) and its vector test; `base64url_nopad` and the rest of the stream proof are unchanged. ui original 26 verified / 20 expected scaffold errors, no resolution errors; ui candidate 77. The account hash-order fix is not claimed as a proof; `stream/SOURCE.md` records it as test-pinned. `docs/SOURCE-FREEZE.sha256` is the 246-file v0.1.2 tree. No pair or gate script was edited by the reviewer; reviewer writes were `REVIEW.md` and the v0.1.2 release-status wording in `docs/PROOFS.md`.
freeze_sha256: bfbfbd84561bda4c1c46ab47a80b51bdb89455d29ba016d08082f5c52875cd56
gate_sha256: 19c01a9b464d244af51d8e290535a2120a5d9ffb0650c47ac206686e972edd90

Regenerate both hashes and re-sign with a fresh review after any change to the
gate scripts or to `FREEZE.sha256`.

## Residual trust (explicit)

This file is a paper trail, not a proof. A prover who edits the gate scripts
(including `review-check.sh` and this file) can make any candidate pass and
recompute both hashes. The controls are independent CI on the pinned toolchain
plus a fresh reviewer re-signing after any gate change. `gate_sha256` only
asserts "these exact gate bytes were present when the record was signed".
`SOURCE.md` remains a hand transcription of the frozen revision; the proofs
cover the model, not the compiled binary. `expand_import_path` is modelled on
Unix bytes with ASCII `trim`; Unicode whitespace, Windows separators and
non-UTF-8 `OsStr` are out of subset. `sapisidhash`, the stream-mute plumbing,
keep-stream state machine, lifecycle code and f32 vinyl helpers are outside the
Verus subset and are pinned by tests, not proofs. `docs/RELEASE-CHECK.md` still
warns that the shipped AppImage is 0.1.1 and predates v0.1.2; it must be rebuilt.
