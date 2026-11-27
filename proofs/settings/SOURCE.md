# Proof source freeze — settings numerics

Frozen 2026-11-02. Proof: `candidate.rs`, scaffold: `original.rs`.

## Frozen source

| source | sha256 |
|---|---|
| crates/core/src/settings.rs | a574eb05c1ca2dafd33f1eb4fe04b7556da987d5a091358bfbfd92d58a44b6a9 |

## Toolchain

- Verus 0.2026.11.02.8bf68ee, bundled Rust `1.99.1-x86_64-unknown-linux-gnu`
- Workspace `rust-toolchain.toml`: 1.99.1

## Proved obligations

- `seed_rgb` returns exactly the three low bytes of `seed_color`; the channels
  reconstruct `seed_color & 0x00FF_FFFF` (no information loss) and ignore the
  alpha byte.
- `Settings::default().seed_color == 0x00ED_5564` maps to `(0xED, 0x55, 0x64)`,
  matching `ui/theme/Theme.kt`.

Serde fill-from-defaults and `f32` settings are outside the subset.

## Re-freeze 2026-11-03 v0.1.3 (motion settings)

`crates/core/src/settings.rs` gained one serialized field,
`pub reduce_motion: bool` (default `false`); the new hash is in the table
above. `seed_rgb` (L154–L160) and `Settings::default().seed_color`
(`0x00ED_5564`) are byte-identical, and this pair transcribes no `Settings`
struct, so no obligation changed and no proof text changed. Serde
fill-from-defaults (the new field is covered by the struct's `#[serde(default)]`)
remains outside the subset, as does the `f32` settings block.

## Re-freeze 2026-11-27 (home view)

`crates/core/src/settings.rs` gained `HomePage`, `HomeView` and
`Settings.home` (new hash above); the settings screen offers the page and its
shelves. `seed_rgb` and `Settings::default().seed_color` (`0x00ED_5564`) are
byte-identical, and this pair transcribes no `Settings` struct, so no
obligation changed and no proof text changed. The new field is covered by the
struct's `#[serde(default)]`, which stays outside the subset.
