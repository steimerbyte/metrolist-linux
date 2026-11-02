#!/usr/bin/env bash
# Regenerate FREEZE.sha256 for every frozen proof artifact: both files of each
# pair, each SOURCE.md, and SOURCES.list. Only run this after SOURCE.md
# explains the change and the gate is green; re-sign REVIEW.md afterwards.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
cd "$here"
sha256sum */original.rs */candidate.rs */SOURCE.md SOURCES.list > FREEZE.sha256
echo "wrote FREEZE.sha256 ($(wc -l < FREEZE.sha256) files)"
