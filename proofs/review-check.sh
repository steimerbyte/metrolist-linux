#!/usr/bin/env bash
# Review-record gate: a fresh reviewer must have ACCEPTed the current freeze
# manifest (pairs + SOURCE.md + SOURCES.list) and the current gate scripts,
# tool pins and trust scanner. This is a paper trail, not a proof:
# whoever can edit the gate can also edit this record. The controls are
# independent CI plus a fresh review after any gate change.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
cd "$here" || exit 1

fail() { echo "FAIL review: $*"; echo "  a fresh reviewer must ACCEPT and re-sign proofs/REVIEW.md"; exit 1; }

[ -f REVIEW.md ] || fail "REVIEW.md missing"

verdict=$(grep -m1 '^verdict:' REVIEW.md | cut -d: -f2 | tr -d ' \r')
[ "$verdict" = "ACCEPT" ] || fail "verdict is '${verdict:-<missing>}', not ACCEPT"

recorded_freeze=$(grep -m1 '^freeze_sha256:' REVIEW.md | awk '{print $2}')
recorded_gate=$(grep -m1 '^gate_sha256:' REVIEW.md | awk '{print $2}')
case "$recorded_freeze$recorded_gate" in (*[!0-9a-f]*) fail "REVIEW.md hashes are not 64-char lowercase hex";; esac
[ "${#recorded_freeze}" -eq 64 ] && [ "${#recorded_gate}" -eq 64 ] || fail "REVIEW.md hashes are missing"

actual_freeze=$(sha256sum FREEZE.sha256 | cut -d' ' -f1)
actual_gate=$({ sha256sum */gate */SOURCE.md spec_guard.py spec_guard_selftest.py \
    freeze-check.sh freeze.sh run-gates.sh check-drift.sh review-check.sh \
    verus.env lynette-setup.sh install-verus.sh SOURCES.list \
    trust_scan.py; } \
    | LC_ALL=C sort | sha256sum | cut -d' ' -f1)

[ "$actual_freeze" = "$recorded_freeze" ] || fail "FREEZE.sha256 changed since review (recorded $recorded_freeze, now $actual_freeze)"
[ "$actual_gate" = "$recorded_gate" ] || fail "gate scripts changed since review (recorded $recorded_gate, now $actual_gate)"

echo "review: current freeze + gate scripts are signed by REVIEW.md"
