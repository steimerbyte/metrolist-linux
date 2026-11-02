#!/usr/bin/env python3
"""Reject net-new Verus trust markers.

Usage: trust_scan.py ORIGINAL CANDIDATE

Why this exists: `lynette additions` allows the prover to rewrite a target
proof body freely.  That is correct for proof search, but it also lets
`assume(false)` through.  Verus then reports "N verified, 0 errors" for a
vacuous proof.  This scan compares marker counts, ignoring comments, and
fails if the candidate introduces more than the original had.

shortcut: count-based, not an AST.  It catches duplicated/moved markers
conservatively; upgrade to token/AST diff if trusted boundaries grow.
"""

import re
import sys
from pathlib import Path

MARKERS = [
    (r"\bassume\s*\(", "assume"),
    (r"\badmit\s*\(", "admit"),
    (r"\bexternal_body\b", "external_body"),
    (r"\baxiom\s+fn\b", "axiom fn"),
    (r"\bassume_specification\b", "assume_specification"),
    (r"#\s*\[\s*verifier\s*::\s*external", "#[verifier::external...]"),
    (r"\bunimplemented\s*!", "unimplemented!"),
]


def strip_comments(source: str) -> str:
    source = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return re.sub(r"//[^\n]*", "", source)


def counts(path: str) -> dict[str, int]:
    source = strip_comments(Path(path).read_text())
    return {name: len(re.findall(pattern, source)) for pattern, name in MARKERS}


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        return 2

    original, candidate = counts(sys.argv[1]), counts(sys.argv[2])
    bad = [
        (name, original[name], candidate[name])
        for _, name in MARKERS
        if candidate[name] > original[name]
    ]
    if bad:
        for name, before, after in bad:
            print(f"FAIL net-new trust marker: {name} ({before} -> {after})")
        return 1

    print("PASS no net-new trust markers")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
