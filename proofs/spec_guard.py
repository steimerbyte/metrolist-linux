#!/usr/bin/env python3
"""Structural guard for a frozen proof pair.

Lynette's `additions` check is not an exec-surface check (measured: it exits 0
for an added top-level exec fn), and regex/line matching is defeated by
formatting (`fn\nsneaky`, `fn /* c */ sneaky`). This guard therefore lexes
both files like Rust and compares the executable surface at every nesting
depth:

- every exec `fn` in `candidate.rs` must exist in `original.rs` in the **same
  scope path** (`mod`/`impl`/`trait` nesting) with the same tokenized signature
  (name, parameters, return type, `requires`, `ensures`);
- `mod`/`impl`/`trait` blocks may not be added at all;
- `const`/`static`/`struct`/`enum`/`type` items may not be added;
- `spec fn` / `proof fn` items may be added freely inside existing scopes.

The lexer skips `//` and nested `/* … */` comments, normal/byte/raw strings and
char literals, so keywords inside them never declare anything. Signatures are
compared as token sequences, so line breaks, comments and whitespace are
irrelevant. Declarations inside function bodies are ignored because bodies are
skipped wholesale.

Limits (documented, not hidden): macro-generated items are invisible (no
expansion), `mod name;` declarations are recorded as bodyless blocks, the body
opener is found with a small heuristic (a `{` after `if`/`else`/`match`/`=>`, or
one whose group is followed by `,`/`else`, is an expression brace), and an
unparseable declaration fails closed.

Usage: spec_guard.py original.rs candidate.rs
"""
import re
import sys
import pathlib

KEYWORDS = {"fn", "mod", "impl", "trait", "const", "static", "struct", "enum", "type"}
GHOST = ("spec fn", "proof fn")
RAW_STRING = re.compile(r'(?:b|c)?(r)(#*)?"')
PLAIN_STRING = re.compile(r'(?:b|c)?"')
CHAR_LITERAL = re.compile(r"(?:b)?'(?:\\.|[^\\'])'")


class GuardError(Exception):
    pass


def norm(text):
    return re.sub(r"\s+", " ", text).strip()


def lex(text, path):
    """Emit (kind, value) for identifiers, literals and punctuation."""
    tokens = []
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch in " \t\r\n":
            i += 1
            continue
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            depth = 1
            i += 2
            while i < n and depth:
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                raise GuardError(f"{path}: unterminated block comment")
            continue

        match = RAW_STRING.match(text, i)
        if match:
            end = '"' + match.group(2)
            j = text.find(end, match.end())
            if j < 0:
                raise GuardError(f"{path}: unterminated raw string")
            tokens.append(("lit", "<str>"))
            i = j + len(end)
            continue

        match = PLAIN_STRING.match(text, i)
        if match:
            j = match.end()
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            else:
                raise GuardError(f"{path}: unterminated string")
            tokens.append(("lit", "<str>"))
            i = j
            continue

        match = CHAR_LITERAL.match(text, i)
        if match:
            tokens.append(("lit", "<char>"))
            i = match.end()
            continue

        if ch == "'":  # lifetime
            j = i + 1
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            i = j
            continue

        if ch.isdigit():
            j = i + 1
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            tokens.append(("lit", "<num>"))
            i = j
            continue

        if ch.isalpha() or ch == "_":
            j = i + 1
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            tokens.append(("ident", text[i:j]))
            i = j
            continue

        tokens.append(("punct", ch))
        i += 1
    return tokens


def match_brace(tokens, open_index, path):
    depth = 0
    for j in range(open_index, len(tokens)):
        kind, value = tokens[j]
        if kind != "punct":
            continue
        if value == "{":
            depth += 1
        elif value == "}":
            depth -= 1
            if depth == 0:
                return j
    raise GuardError(f"{path}: unbalanced braces after token {open_index}")


def find_body(tokens, fn_index, path):
    """Index of the `{` opening the fn body, or None for a bodyless fn."""
    paren = 0
    i = fn_index + 1
    while i < len(tokens):
        kind, value = tokens[i]
        if kind == "punct":
            if value in "([":
                paren += 1
            elif value in ")]":
                paren -= 1
            elif value == ";" and paren == 0:
                return None
            elif value == "{" and paren == 0:
                previous = tokens[i - 1][1] if i > 0 else None
                close = match_brace(tokens, i, path)
                following = tokens[close + 1][1] if close + 1 < len(tokens) else None
                if previous in ("if", "else", "match", "=>") or following in (",", "else"):
                    i = close + 1
                    continue
                return i
        i += 1
    raise GuardError(f"{path}: no body brace found for the fn at token {fn_index}")


def next_ident(tokens, index, path):
    if index < len(tokens) and tokens[index][0] == "ident":
        return tokens[index][1]
    raise GuardError(f"{path}: expected an identifier after token {index - 1}")


def scan(path):
    lines = pathlib.Path(path).read_text()
    tokens = lex(lines, path)
    fns, blocks, items = [], [], []
    stack = []      # (kind, name, depth inside the block)
    pending = None  # (kind, name) waiting for its `{`
    depth = 0
    i = 0
    while i < len(tokens):
        kind, value = tokens[i]
        scope = tuple(f"{k} {n}" for k, n, _ in stack)
        if kind == "punct" and value == "{":
            depth += 1
            if pending:
                stack.append((pending[0], pending[1], depth))
                pending = None
            i += 1
            continue
        if kind == "punct" and value == "}":
            depth -= 1
            while stack and stack[-1][2] > depth:
                stack.pop()
            i += 1
            continue
        if kind == "punct" and value == ";":
            pending = None
            i += 1
            continue
        if kind == "ident":
            if value in ("mod", "trait"):
                name = next_ident(tokens, i + 1, path)
                blocks.append((scope, value, name))
                pending = (value, name)
                i += 1
                continue
            if value == "impl":
                j = i + 1
                parts = []
                while j < len(tokens) and tokens[j] != ("punct", "{"):
                    if tokens[j][0] == "ident" and tokens[j][1] == ";":
                        break
                    parts.append(tokens[j][1])
                    j += 1
                name = norm(" ".join(parts))
                blocks.append((scope, "impl", name))
                pending = ("impl", name)
                i = j
                continue
            if value == "fn":
                fn_kind = "fn"
                if i > 0 and tokens[i - 1] == ("ident", "proof"):
                    fn_kind = "proof fn"
                elif i > 0 and tokens[i - 1] == ("ident", "spec"):
                    fn_kind = "spec fn"
                name = next_ident(tokens, i + 1, path)
                body = find_body(tokens, i, path)
                if body is None:
                    header = norm(" ".join(t[1] for t in tokens[i + 1:]))
                    fns.append((scope, fn_kind, name, header))
                    i += 1
                    continue
                header = norm(" ".join(t[1] for t in tokens[i + 1:body]))
                fns.append((scope, fn_kind, name, header))
                i = match_brace(tokens, body, path) + 1
                continue
            if value in ("const", "static", "struct", "enum", "type"):
                if i + 1 < len(tokens) and tokens[i + 1][0] == "ident" \
                        and tokens[i + 1][1] not in KEYWORDS:
                    items.append((scope, value, tokens[i + 1][1]))
                i += 1
                continue
        i += 1
    return fns, blocks, items


def compare(original, candidate):
    orig_fns, orig_blocks, orig_items = scan(original)
    cand_fns, cand_blocks, cand_items = scan(candidate)
    problems = []

    orig_map = {(scope, kind, name): header for scope, kind, name, header in orig_fns}
    for scope, kind, name, header in cand_fns:
        if kind in GHOST:
            continue
        where = " > ".join(scope) if scope else "top level"
        key = (scope, kind, name)
        if key not in orig_map:
            problems.append(f"candidate adds exec fn `{name}` at {where} (not in original.rs)")
        elif orig_map[key] != header:
            problems.append(
                f"exec fn `{name}` signature differs at {where}:\n"
                f"    original:  {orig_map[key]}\n"
                f"    candidate: {header}"
            )

    for scope, kind, name in cand_blocks:
        if (scope, kind, name) not in orig_blocks:
            where = " > ".join(scope) if scope else "top level"
            problems.append(f"candidate adds {kind} block `{name}` at {where}")

    for scope, kind, name in cand_items:
        if (scope, kind, name) not in orig_items:
            where = " > ".join(scope) if scope else "top level"
            problems.append(f"candidate adds {kind} item `{name}` at {where}")

    return problems


def main():
    if len(sys.argv) != 3:
        print("usage: spec_guard.py original.rs candidate.rs", file=sys.stderr)
        return 2
    try:
        problems = compare(sys.argv[1], sys.argv[2])
    except GuardError as error:
        print(f"FAIL spec_guard: {error}")
        return 1
    if problems:
        print("FAIL spec_guard:")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print("spec_guard: exec surface matches original.rs at every scope")
    return 0


if __name__ == "__main__":
    sys.exit(main())
