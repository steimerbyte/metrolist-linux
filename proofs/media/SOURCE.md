# media — frozen source

## Source

| file | sha256 (at freeze) |
|------|--------------------|
| crates/core/src/media.rs | 0d1b309803236d599289cc20d48ac2fc85d8ac1cadc1f68210aa34dc3df05caa |

Toolchain: Verus `0.2026.11.02.8bf68ee` (tag `release/0.2026.11.02.8bf68ee`),
bundled Rust `1.99.1-x86_64-unknown-linux-gnu`; workspace pin `1.99.1`.
Lynette gate: `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`, built against
`verus_syn`/`verus_prettyplease` from the Verus tag above.

## Scope

`StreamInfo::is_expired(now_epoch_secs: i64) -> bool` — the streaming-URL
expiry decision (300 s safety margin, inclusive).

## Model deviations

- `self.expires_at: Option<i64>` is passed as the parameter `expires_at`;
  the remaining `StreamInfo` fields are irrelevant to the decision.
- `i64::saturating_sub(300)` is transcribed as an explicit guarded check
  (`expires < i64::MIN + 300` → `i64::MIN`). `i64::MIN + 300` cannot overflow,
  and the guard makes the subtraction total, so the transcription is exact.
- Spec arithmetic is `int`; `expiry_threshold` returns `int`, so no wrap.

## Proven

- `is_expired`: `r == expired(expires_at, now)` — `None` never expires;
  `Some(e)` expires iff `now >= e - 300` (saturating).
- `lemma_none_never_expires`, `lemma_margin_is_inclusive`,
  `lemma_expiry_monotone`.

`StreamInfo`'s `Serialize`/`Deserialize` derives and the rest of `MediaInfo`
are data-only and out of the Verus subset (serde).
