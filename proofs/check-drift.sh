#!/usr/bin/env bash
# Fail (exit 1) when a declared proof source no longer matches its SOURCE.md
# hash. Iterates proofs/SOURCES.list, so removing a row from a SOURCE.md cannot
# disable the check. run-gates.sh treats a non-zero exit as a hard block.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
list="$here/SOURCES.list"
status=0
checked=0

[ -f "$list" ] || { echo "FAIL drift: SOURCES.list missing"; exit 1; }

while read -r area path rest; do
    [ -n "${area:-}" ] || continue
    case "$area" in \#*) continue;; esac
    [ -n "${path:-}" ] || { echo "FAIL drift: SOURCES.list line without a path: $area"; status=1; continue; }

    source_md="$here/$area/SOURCE.md"
    if [ ! -f "$source_md" ]; then
        echo "FAIL drift: $area/SOURCE.md missing (declares $path)"
        status=1
        continue
    fi

    want=$(awk -F'|' -v p="$path" '
        { gsub(/^ +| +$/, "", $2); gsub(/^ +| +$/, "", $3)
          if ($2 == p && length($3) == 64) { print $3; exit } }' "$source_md")
    case "$want" in
        *[!0-9a-f]*)
            echo "FAIL drift: $path has a non-hex hash in $area/SOURCE.md"
            status=1
            continue
            ;;
    esac
    if [ -z "$want" ] || [ "${#want}" -ne 64 ]; then
        echo "FAIL drift: $path is declared in SOURCES.list but has no 64-hex row in $area/SOURCE.md"
        status=1
        continue
    fi

    if [ ! -f "$repo/$path" ]; then
        echo "FAIL drift: $path is missing"
        status=1
        continue
    fi

    got=$(sha256sum "$repo/$path" | cut -d' ' -f1)
    if [ "$got" != "$want" ]; then
        echo "FAIL drift: $path changed since freeze ($area/SOURCE.md)"
        echo "  frozen: $want"
        echo "  now:    $got"
        status=1
    fi
    checked=$((checked + 1))
done < "$list"

# A SOURCE.md row for crates/** that is not declared here is an unpinned claim.
for source_md in "$here"/*/SOURCE.md; do
    [ -f "$source_md" ] || continue
    area=$(basename "$(dirname "$source_md")")
    while IFS= read -r line; do
        path=$(printf '%s\n' "$line" | cut -d'|' -f2 | tr -d ' `')
        if ! awk -v a="$area" -v p="$path" '$1 == a && $2 == p { found = 1 } END { exit !found }' "$list"; then
            echo "FAIL drift: $area/SOURCE.md declares $path which is not in SOURCES.list"
            status=1
        fi
    done < <(grep -E '^\| crates/[^|]+\.rs \| [0-9a-f]{64} \|' "$source_md")
done

[ "$status" -eq 0 ] && echo "no source drift ($checked declared sources match)"
exit "$status"
