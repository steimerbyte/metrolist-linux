# Proof source freeze — settings numerics

Frozen 2026-11-02. Proof: `candidate.rs`, scaffold: `original.rs`.

## Frozen source

| source | sha256 |
|---|---|
| crates/core/src/settings.rs | e3a390e746ab8fb8da61d1982b57d5a02dc3ac4e8974a6390275ea9e129bec6b |

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
