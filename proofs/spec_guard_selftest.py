#!/usr/bin/env python3
"""Self-test for the gate's structural and freeze checks.

Every mutation below is applied to a temporary copy; the repository is never
modified. Each case must make the corresponding checker exit non-zero, and the
unmodified pairs must pass. Exit 0 only if all cases behave.

Cases:
  1. clean pairs are accepted
  2. a new top-level exec fn is rejected
  3. a new exec fn inside a new `mod` is rejected        (reviewer evasion)
  4. a new method inside an existing `impl` is rejected
  5. a removed `ensures` clause is rejected
  6. an added `ensures` clause is rejected
  7. formatting cannot hide a declaration: `fn\nsneaky`, `fn /* c */ sneaky`,
     `fn /* } { */ sneaky`, `mod\nsneaky { fn\nhidden }`
  8. `pub(crate) fn` / `pub(in …) fn` are rejected
  9. a `pub(crate) mod` with a hidden fn is rejected
 10. keywords inside raw strings / nested block comments are not declarations
 11. deleting a SOURCE.md row does not disable check-drift.sh, and a missing
     declared source is also rejected
 10. a tampered candidate.rs is rejected by freeze-check.sh
 11. a tampered original.rs is rejected by freeze-check.sh
"""
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent
GUARD = HERE / "spec_guard.py"
AREAS = sorted(
    d.name
    for d in HERE.iterdir()
    if (d / "original.rs").is_file() and (d / "candidate.rs").is_file()
)


def run(cmd, cwd=None):
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)


def mutate(text, old, new):
    assert old in text, f"self-test setup is stale: {old!r} not found"
    return text.replace(old, new, 1)


def guard(original, candidate):
    return run([sys.executable, str(GUARD), str(original), str(candidate)])


def main():
    failures = []

    # 1. clean pairs
    for area in AREAS:
        result = guard(HERE / area / "original.rs", HERE / area / "candidate.rs")
        if result.returncode != 0:
            failures.append(f"clean pair {area} was rejected:\n{result.stdout}")

    stream_original = HERE / "stream" / "original.rs"
    stream_candidate = (HERE / "stream" / "candidate.rs").read_text()
    queue_original = HERE / "queue" / "original.rs"
    queue_candidate = (HERE / "queue" / "candidate.rs").read_text()
    media_original = HERE / "media" / "original.rs"
    media_candidate = (HERE / "media" / "candidate.rs").read_text()

    cases = [
        (
            "new top-level exec fn",
            mutate(stream_candidate, "} // verus!",
                   "pub fn sneaky(bytes: &[u8]) -> (r: u64)\n    ensures r == 0,\n{\n    0\n}\n\n} // verus!"),
            stream_original,
        ),
        (
            "exec fn inside a new mod",
            mutate(stream_candidate, "verus! {\n",
                   "verus! {\n\nmod sneaky {\n    pub fn evil(bytes: &[u8]) -> (r: u64)\n        ensures r == 0,\n    {\n        0\n    }\n}\n"),
            stream_original,
        ),
        (
            "new method inside an existing impl",
            mutate(queue_candidate, "impl Queue {\n",
                   "impl Queue {\n    pub fn sneaky(&self) -> (r: usize)\n        ensures r == 0,\n    {\n        0\n    }\n"),
            queue_original,
        ),
        (
            "removed ensures",
            mutate(stream_candidate, "        r.is_some() ==> r.unwrap() >= now,\n", ""),
            stream_original,
        ),
        (
            "added ensures",
            mutate(stream_candidate, "        r.is_some() ==> r.unwrap() >= now,\n",
                   "        r.is_some() ==> r.unwrap() >= now,\n        r.is_some() ==> r.unwrap() == now,\n"),
            stream_original,
        ),
        (
            "pub(crate) fn added to media",
            mutate(media_candidate, "} // verus!",
                   "pub(crate) fn sneaky(x: u8) -> (r: u8)\n    ensures r == x,\n{\n    x\n}\n\n} // verus!"),
            media_original,
        ),
        (
            "pub(in crate::x) fn added",
            mutate(stream_candidate, "} // verus!",
                   "pub(in crate::x) fn sneaky2(x: u8) -> (r: u8)\n    ensures r == x,\n{\n    x\n}\n\n} // verus!"),
            stream_original,
        ),
        (
            "fn split from its name",
            mutate(media_candidate, "} // verus!",
                   "pub fn\nsneaky(x: u8) -> (r: u8)\n    ensures r == x,\n{\n    x\n}\n\n} // verus!"),
            media_original,
        ),
        (
            "block comment between fn and name",
            mutate(media_candidate, "} // verus!",
                   "pub fn /* c */ sneaky(x: u8) -> (r: u8)\n    ensures r == x,\n{\n    x\n}\n\n} // verus!"),
            media_original,
        ),
        (
            "brace-bearing block comment between fn and name",
            mutate(media_candidate, "} // verus!",
                   "pub fn /* } { */ sneaky(x: u8) -> (r: u8)\n    ensures r == x,\n{\n    x\n}\n\n} // verus!"),
            media_original,
        ),
        (
            "split mod and split fn",
            mutate(stream_candidate, "verus! {\n",
                   "verus! {\n\npub(crate) mod\nsneaky {\n    pub fn\n    hidden(x: u8) -> (r: u8)\n        ensures r == x,\n    {\n        x\n    }\n}\n"),
            stream_original,
        ),
        (
            "pub(crate) mod with a hidden fn",
            mutate(stream_candidate, "verus! {\n",
                   "verus! {\n\npub(crate) mod hidden {\n    pub(crate) fn h(x: u8) -> (r: u8)\n        ensures r == x,\n    {\n        x\n    }\n}\n"),
            stream_original,
        ),
    ]

    with tempfile.TemporaryDirectory() as tmp:
        for index, (name, text, original) in enumerate(cases, start=2):
            candidate = pathlib.Path(tmp) / f"case{index}.rs"
            candidate.write_text(text)
            result = guard(original, candidate)
            if result.returncode == 0:
                failures.append(f"case {index} ({name}) was NOT rejected")
            elif "FAIL spec_guard" not in result.stdout:
                failures.append(f"case {index} ({name}) failed without a guard message:\n{result.stdout}")

        # 10. keywords inside strings / comments are not declarations: the
        #     verdict must be the same as for the unmodified pair.
        noise = mutate(media_candidate, "    now_epoch_secs >= threshold\n",
                       "    let _ = r#\"fn sneaky() { }\"#;\n    // fn also_sneaky()\n    now_epoch_secs >= threshold\n")
        noise = "/* fn /* nested */ sneaky */\n" + noise
        clean_path = pathlib.Path(tmp) / "media_clean.rs"
        clean_path.write_text(media_candidate)
        noise_path = pathlib.Path(tmp) / "media_noise.rs"
        noise_path.write_text(noise)
        base = guard(media_original, clean_path)
        result = guard(media_original, noise_path)
        if base.returncode != result.returncode:
            failures.append(
                "keywords inside strings/comments changed the guard verdict:\n"
                f"  clean: {base.stdout}  noisy: {result.stdout}"
            )

        # 11. deleting a SOURCE.md row must not disable check-drift.sh
        repo = pathlib.Path(tmp) / "repo"
        proofs_copy = repo / "proofs"
        shutil.copytree(HERE, proofs_copy, ignore=shutil.ignore_patterns("logs", "__pycache__"))
        for line in (HERE / "SOURCES.list").read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            _, rel = line.split()
            destination = repo / rel
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO / rel, destination)
        clean = run(["bash", str(proofs_copy / "check-drift.sh")])
        if clean.returncode != 0:
            failures.append(f"clean check-drift failed:\n{clean.stdout}")
        media_md = proofs_copy / "media" / "SOURCE.md"
        original_md = media_md.read_text()
        media_md.write_text(re.sub(r"^\| crates/core/src/media\.rs \|.*\n", "", original_md, flags=re.M))
        result = run(["bash", str(proofs_copy / "check-drift.sh")])
        if result.returncode == 0:
            failures.append("deleted SOURCE.md row did NOT fail check-drift.sh")
        media_md.write_text(original_md)
        (repo / "crates/core/src/media.rs").write_text("// changed\n")
        result = run(["bash", str(proofs_copy / "check-drift.sh")])
        if result.returncode == 0:
            failures.append("changed source did NOT fail check-drift.sh")
        (repo / "crates/core/src/media.rs").unlink()
        result = run(["bash", str(proofs_copy / "check-drift.sh")])
        if result.returncode == 0:
            failures.append("missing declared source did NOT fail check-drift.sh")

        # 12. tampered frozen pair -> freeze-check.sh
        copy = pathlib.Path(tmp) / "proofs"
        shutil.copytree(HERE, copy, ignore=shutil.ignore_patterns("logs", "__pycache__"))
        clean = run(["bash", str(copy / "freeze-check.sh")])
        if clean.returncode != 0:
            failures.append(f"clean freeze-check failed:\n{clean.stdout}")
        for target in ("candidate.rs", "original.rs"):
            tampered = copy / "media" / target
            tampered.write_text(tampered.read_text() + "\n// tamper\n")
            result = run(["bash", str(copy / "freeze-check.sh")])
            if result.returncode == 0:
                failures.append(f"tampered media/{target} was NOT rejected by freeze-check.sh")

    if failures:
        print(f"FAIL gate self-test ({len(failures)} problems)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print(f"gate self-test: {len(AREAS)} clean pairs accepted, {len(cases)} mutations rejected, "
          "string/comment keywords ignored, SOURCE.md-row deletion + changed + missing source rejected, "
          "2 freeze tampers rejected")
    return 0


if __name__ == "__main__":
    sys.exit(main())
