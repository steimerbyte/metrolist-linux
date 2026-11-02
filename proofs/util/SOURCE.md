# Proof source freeze — core utilities

Frozen 2026-11-02. Proof: `candidate.rs`, scaffold: `original.rs`.

## Frozen source

| source | sha256 |
|---|---|
| crates/core/src/util.rs | 56990c5700fc22a511e28be738f3947b62dbb04c17930a48930c8f3514c02c89 |

## Toolchain

- Verus 0.2026.11.02.8bf68ee, bundled Rust `1.99.1-x86_64-unknown-linux-gnu`
- Workspace `rust-toolchain.toml`: 1.99.1

## Boundary (what is proven, what is not)

- `format_duration` / `format_position`: the arithmetic core
  (`duration_parts`, `position_seconds`) is proven total and exact, including
  `h*3600 + m*60 + s == total`, `m < 60`, `s < 60`, saturation at `u32::MAX`.
  The `format!` rendering is outside the Verus subset (`std::fmt`), so the
  string shape (`{minutes:02}`, `{seconds:02}`, no hour padding) is not
  machine-checked; the field widths follow from `m < 60`, `s < 60`.
- `thumbnail_url`: bucket selection is proven (including monotonicity: a larger
  request never picks a smaller bucket). `format!` URL assembly is outside.
- `is_video_id`: proven over `&[u8]`; the real code's
  `value.bytes().all(..)`/`value.len()` are the UTF-8 byte view, mapped by
  `is_video_id_bytes`. `is_ascii_alphanumeric` is transcribed to explicit
  ranges (same set).
- `parse_video_id`: the `url` crate is outside the Verus subset. The final
  `is_video_id(&candidate).then_some(candidate)` filter is proven by
  `is_video_id_bytes`: any returned id has the exactly-11-byte shape.
- `resize_thumbnail_url`: `url::Url` parsing is outside the subset.
- `now_millis`: clock source, outside the subset (non-deterministic).
- `millis_to_seconds`: proven (`millis / 1000` floor).
