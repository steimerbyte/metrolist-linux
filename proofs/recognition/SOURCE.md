# recognition — frozen source

## Source

| file | sha256 (at freeze) |
|------|--------------------|
| crates/recognition/src/signature.rs | ca4477245067d2c69f8e51de2b8097ec30c07cb503a9c97ad78140b9a7e23e32 |

Toolchain: Verus `0.2026.11.02.8bf68ee` (tag `release/0.2026.11.02.8bf68ee`),
bundled Rust `1.99.1-x86_64-unknown-linux-gnu`; workspace pin `1.99.1`.
Lynette gate: `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`.

Scope: the bit-exact parts of the Shazam signature generator — `crc32`,
`base64_encode` (shape), the band mapping, the per-peak 0xFF escape encoder, and
the sample-count/lead-in arithmetic. The FFT (rustfft), the resampler, the WAV
reader and the libm-shaped peak picking that depends on float magnitudes are
out of scope.

## Model deviations

- **`base64_encode`** is transcribed with an index loop instead of
  `for chunk in data.chunks(3)`; chunking is identical. `ALPHABET[i] as char`
  is modelled as `ALPHABET[i] as u8` (ASCII-only alphabet). The `requires
  data@.len() <= usize::MAX - 3` keeps `i + 3` total; no real slice can reach
  it. The byte-level *placement* of the `=` padding is pinned by the golden
  tests, not proven here (the contract proves the output **length** and that it
  is a 4-byte multiple, plus `b64_pad < 3`).
- **`band_of`** takes integer Hz. The real code multiplies a float bin by
  `BIN_HZ` and compares against `f64` thresholds; the comparison structure is
  mirrored one-for-one on integers (a peak frequency is an exact multiple of
  7.8125 Hz only in theory — the float comparison itself is out of subset).
- **`encode_peak`** replaces `i32::to_le_bytes` and `extend_from_slice` with
  four/two explicit byte pushes (vstd has no spec for those two std calls) and
  fixes the magnitude/bin shorts at 0 (they are caller data, not arithmetic).
  The escape decision, the re-anchoring of `prev`, and the resulting frame
  length are proven.
- **`samples_and_offset`** models
  `(num_samples as f64 + SAMPLE_RATE as f64 * 0.24) as i32` as the exact
  integer `num_samples + 3840` (`16_000 * 0.24 = 3840`, exactly representable;
  the golden test pins the real f64 result to the same value). i32 overflow is
  excluded by a documented bound (`num_samples <= i32::MAX - 3840`).

## Proven

- `crc32` = `crc32_spec`: `crc_prefix` folds `crc_byte` over the prefix, each
  `crc_byte` is eight reflected `0xEDB88320` bit steps, and the final register
  is bit-inverted. `lemma_crc32_empty` (`[]` → `0`),
  `lemma_crc_prefix_step` (appending one byte is one `crc_byte` step), so the
  checksum is a pure function of the bytes.
- `base64_encode`: `r.len() == b64_len(n) == 4 * ceil(n/3)`, always a multiple
  of 4 (`lemma_b64_len_multiple_of_four`), `b64_pad(n) < 3` and the exact
  padding counts per residue (`lemma_b64_padding_shape`), and the
  `4/3 + slack` growth bound (`lemma_b64_grows_by_a_third`).
- `band_of` / `band_of_exec`: the four bands with exclusive lower bounds and
  the inclusive 5500 Hz upper bound; `lemma_band_boundaries` pins 249/250,
  519/520, 1449/1450, 3499/3500, 5500/5501; `lemma_band_monotone` says a lower
  frequency never lands in a higher band.
- `encode_peak`: the escape fires exactly at a delta of 255, the frame is 5
  bytes without escape and 10 with it (`lemma_frame_sizes`), the byte after a
  0xFF marker is 0 (the escape re-anchors `prev`), and `prev` is always the
  peak just written so the next delta is measured from the right base.
- `samples_and_offset`: `num_samples + 3840`, bounded for the 12 s window
  (`lemma_samples_and_offset_bounded`, no i32 overflow);
  `lemma_hop_arithmetic` for the 128-sample hop accounting.

## Not covered (deliberate)

- rustfft/`process`, the peak picking over float magnitudes, the windowing, the
  resampler, WAV parsing, the HTTP call to the recognition endpoint, and the
  `0.24 * 16_000` float multiply itself.
