# Proof source freeze — settings numerics

Frozen 2026-11-02. Proof: `candidate.rs`, scaffold: `original.rs`.

## Frozen source

| source | sha256 |
|---|---|
| crates/core/src/settings.rs | 832144ddc45a98993b437b00fc631ccdc61f939e7437276c40c80ab742b15bcc |

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
