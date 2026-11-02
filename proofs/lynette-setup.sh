#!/usr/bin/env bash
# Build the pinned Lynette gate binary against the pinned Verus parser.
#
# Lynette pins syn_verus 2.0.96, which predates the Verus `final` keyword used
# by the proofs. This script checks Lynette out at the pinned commit, points it
# at verus_syn/verus_prettyplease from the exact Verus release tag, adapts the
# seven AST constructors that gained fields, and builds the release binary.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck disable=SC1091
. "$here/verus.env"
tools=${TOOLS_DIR:-$(cd "$here/../.." && pwd)/tools}
mkdir -p "$tools"

if [ ! -d "$tools/Lynette/.git" ]; then
    git clone --quiet "$LYNETTE_REPO" "$tools/Lynette"
fi
if ! git -C "$tools/Lynette" cat-file -e "$LYNETTE_COMMIT^{commit}" 2>/dev/null; then
    git -C "$tools/Lynette" fetch --quiet origin "$LYNETTE_COMMIT"
fi
git -C "$tools/Lynette" reset --hard --quiet "$LYNETTE_COMMIT"
git -C "$tools/Lynette" clean --quiet -fd

if [ ! -d "$tools/verus-src/dependencies/syn" ]; then
    git clone --quiet --depth 1 --branch "$VERUS_TAG" https://github.com/verus-lang/verus "$tools/verus-src"
fi

# 1. rename the parser crates (idempotent)
grep -rl "syn_verus\|prettyplease_verus" "$tools/Lynette/lynette/src" 2>/dev/null \
    | xargs -r sed -i 's/syn_verus/verus_syn/g; s/prettyplease_verus/verus_prettyplease/g' || true

python3 - "$tools/Lynette/lynette" "$tools/verus-src" <<'PY'
import pathlib, sys

lynette, verus_src = sys.argv[1], sys.argv[2]
cargo = pathlib.Path(lynette) / "Cargo.toml"
text = cargo.read_text()
if "verus_syn = { path" not in text:
    text = text.replace(
        'syn_verus = { git = "https://github.com/verus-lang/verus", version = "2.0.96", features = ["full", "extra-traits", "visit-mut"] }',
        f'verus_syn = {{ path = "{verus_src}/dependencies/syn", default-features = false, features = ["full", "extra-traits", "visit-mut", "parsing", "printing", "clone-impls"] }}',
    )
    text = text.replace(
        'prettyplease_verus = { git = "https://github.com/verus-lang/verus", version = "0.2.29" }',
        f'verus_prettyplease = {{ path = "{verus_src}/dependencies/prettyplease" }}',
    )
    assert "verus_syn = { path" in text, "Cargo.toml dependency rewrite failed"
    cargo.write_text(text)

# 2. seven AST constructors gained fields in the matching verus_syn
def patch(name, needle, insert):
    path = pathlib.Path(lynette) / "src" / name
    src = path.read_text()
    if insert.strip() in src:
        return
    assert needle in src, f"patch anchor missing in {name}: {needle!r}"
    path.write_text(src.replace(needle, needle + insert, 1))

patch("deghost.rs",
      "                args: new_args,\n",
      "                atomically: c.atomically.clone(),\n")
patch("deghost.rs",
      "                ensures: if mode.ensures { c.ensures.clone() } else { None },\n",
      "                invariants: if mode.invariants { c.invariants.clone() } else { None },\n")
patch("deghost.rs",
      "                    invariant: if mode.invariants { f.invariant.clone() } else { None },\n",
      "                    invariant_except_break: None,\n                    ensures: None,\n")
patch("deghost.rs",
      "        spec: verus_syn::SignatureSpec {\n            prover: sig.spec.prover.clone(), // Removed\n",
      "            atomic_spec: sig.spec.atomic_spec.clone(),\n")
patch("deghost.rs",
      "        defaultness: i.defaultness.clone(),\n        unsafety: i.unsafety.clone(),\n",
      "        constness: i.constness.clone(),\n")
patch("deghost.rs",
      "            vis: t.vis.clone(),\n",
      "            constness: t.constness.clone(),\n")
patch("main.rs",
      "                                        spec: verus_syn::SignatureSpec {\n                                            prover: sig.spec.prover.clone(),\n",
      "                                            atomic_spec: sig.spec.atomic_spec.clone(),\n")
print("lynette sources patched")
PY

cargo build --release --manifest-path "$tools/Lynette/Cargo.toml" >/dev/null
echo "lynette built: $tools/Lynette/target/release/lynette"
"$tools/Lynette/target/release/lynette" --version
