#!/usr/bin/env bash
# Run every proof gate. Fail closed: drift, freeze, review record, structure,
# the guard self-test or any failing gate makes this script exit non-zero.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck disable=SC1091
. "$here/verus.env"
status=0

# 0. The verification binary must be the pinned build, not whatever is on PATH.
verus_bin=${VERUS:-$HOME/.local/opt/verus-x86-linux/verus}
if ! version_out=$("$verus_bin" --version 2>&1); then
    echo "GATES FAILED: cannot run verus at $verus_bin"
    exit 1
fi
case "$version_out" in
    *"$VERUS_VERSION"*) ;;
    *)
        echo "GATES FAILED: wrong Verus version (want $VERUS_VERSION):"
        printf '%s\n' "$version_out" | sed 's/^/  /'
        exit 1
        ;;
esac

# 1. A changed crates/** file invalidates its frozen proof: block until the
#    re-freeze (SOURCE.md + FREEZE.sha256) lands in the same change.
if ! "$here/check-drift.sh"; then
    echo "GATES FAILED: source drift; re-freeze the stale proof(s) before gating"
    exit 1
fi

# 2. A frozen pair that changed without a manifest update is an untracked edit.
if ! bash "$here/freeze-check.sh"; then
    echo "GATES FAILED: frozen proof pair changed outside a recorded re-freeze"
    exit 1
fi

# 3. A fresh reviewer must have signed the current freeze manifest and gate
#    scripts (paper trail; see the residual-trust note in REVIEW.md).
if ! bash "$here/review-check.sh"; then
    echo "GATES FAILED: review record missing or stale"
    exit 1
fi

# 4. candidate.rs may add only ghost material. Lynette does not reject new
#    exec fns (not even nested in mod/impl), so compare the exec surface first.
count=0
for d in "$here"/*/; do
    [ -f "$d/original.rs" ] && [ -f "$d/candidate.rs" ] || continue
    name=$(basename "$d")
    count=$((count + 1))
    if ! out=$(python3 "$here/spec_guard.py" "$d/original.rs" "$d/candidate.rs" 2>&1); then
        printf '%s\n' "$out"
        echo "FAIL $name: spec_guard"
        status=1
    fi
done
if [ "$count" -eq 0 ]; then
    echo "FAIL no proof pairs found under $here"
    exit 1
fi

# 5. The guard itself must still reject the known evasions before its verdict
#    on this run is trusted.
if ! python3 "$here/spec_guard_selftest.py"; then
    echo "GATES FAILED: gate self-test"
    exit 1
fi

# 6. Verus, Lynette, trust scan and freeze manifest, per pair.
for d in "$here"/*/; do
    [ -f "$d/original.rs" ] && [ -f "$d/candidate.rs" ] || continue
    name=$(basename "$d")
    [ -x "$d/gate" ] || { echo "FAIL $name: gate missing/not executable"; status=1; continue; }
    "$d/gate" || status=1
done

if [ "$status" -eq 0 ]; then
    echo "ALL GATES PASS ($count proofs)"
else
    echo "GATES FAILED"
fi
exit "$status"
