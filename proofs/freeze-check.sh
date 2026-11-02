#!/usr/bin/env bash
# Fail if a frozen original.rs/candidate.rs changed without regenerating
# FREEZE.sha256 (i.e. an edit that was not recorded as a re-freeze).
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
cd "$here" || exit 1
if [ ! -f FREEZE.sha256 ]; then
    echo "FAIL freeze: FREEZE.sha256 missing (run proofs/freeze.sh after a reviewed re-freeze)"
    exit 1
fi
if out=$(sha256sum --quiet -c FREEZE.sha256 2>&1); then
    echo "freeze: manifest matches ($(grep -c . FREEZE.sha256) frozen files)"
else
    printf '%s\n' "$out" | sed 's/^/  /'
    echo "FAIL freeze: frozen file changed without a manifest update"
    echo "  re-freeze (SOURCE.md + proofs/freeze.sh) and re-prove; never edit a proved pair silently"
    exit 1
fi
