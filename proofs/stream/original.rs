use vstd::prelude::*;

verus! {

// ===== frozen model of crates/innertube/src/stream.rs and its helpers =====
//
// Expressive-Rust adapters that Verus cannot take are modelled, never
// strengthened: `SystemTime` -> epoch seconds (checked add kept explicit),
// `&str` parsing -> the post-parse decision function, `impl Iterator` filter
// -> an index loop with the same predicate, `String` -> `Vec<u8>`,
// `format!`/`reqwest`/`HashMap`/`Mutex` are absent. See SOURCE.md.

// ---------- chunk_ranges (verbatim) ----------

pub fn chunk_ranges(total: u64, chunk: u64) -> (r: Vec<(u64, u64)>)
    ensures
        (total == 0 || chunk == 0) ==> r@.len() == 0,
        total > 0 && chunk > 0 ==> (
            r@.len() > 0
                && r@[0].0 == 0
                && r@[r@.len() - 1].1 == total - 1
                && forall|i: int| 0 <= i < r@.len() ==> #[trigger] r@[i].0 <= r@[i].1 && r@[i].1 < total
                && forall|i: int| 0 <= i < r@.len() - 1 ==> #[trigger] r@[i + 1].0 == r@[i].1 + 1
                && forall|i: int| 0 <= i < r@.len() ==> #[trigger] r@[i].1 - r@[i].0 + 1 <= chunk
                && forall|i: int| 0 <= i < r@.len() - 1 ==> #[trigger] r@[i].1 - r@[i].0 + 1 == chunk
        ),
{
    let mut ranges = Vec::new();
    if total == 0 || chunk == 0 {
        return ranges;
    }
    let mut offset = 0u64;
    while offset < total
        decreases (total - offset) as int,
    {
        let end = offset.saturating_add(chunk - 1).min(total - 1);
        ranges.push((offset, end));
        offset = end.saturating_add(1);
    }
    ranges
}

// ---------- expiry (SystemTime -> epoch seconds) ----------

/// `now.checked_add(Duration::from_secs(expires_in - margin))` on epoch
/// seconds: `None` when the TTL is inside the margin or the sum overflows.
pub fn expiry(now: u64, expires_in: u64, margin: u64) -> (r: Option<u64>)
    ensures
        expires_in <= margin ==> r.is_none(),
        expires_in > margin ==> r.is_some() == (now as int + (expires_in - margin) as int <= u64::MAX as int),
        r.is_some() ==> r.unwrap() as int == now as int + (expires_in - margin) as int,
        r.is_some() ==> r.unwrap() >= now,
{
    if expires_in <= margin {
        return None;
    }
    let delta = expires_in - margin;
    if now > u64::MAX - delta {
        return None;
    }
    Some(now + delta)
}

// ---------- parse_content_range: the post-parse decision only ----------

/// The checks `parse_content_range` applies after `split_once`/`parse`:
/// `START > END` and `END >= TOTAL` are rejected, so any `Some` is a valid
/// range. The `&str` scanning itself is out of the Verus subset.
pub fn validate_content_range(start: u64, end: u64, total: Option<u64>) -> (r: Option<(u64, u64, Option<u64>)>)
    ensures
        start > end ==> r.is_none(),
        total.is_some() && end >= total.unwrap() ==> r.is_none(),
        r.is_some() ==> r.unwrap().0 <= r.unwrap().1,
        r.is_some() && r.unwrap().2.is_some() ==> r.unwrap().1 < r.unwrap().2.unwrap(),
        r.is_some() ==> r.unwrap().0 == start && r.unwrap().1 == end && r.unwrap().2 == total,
{
    if start > end {
        return None;
    }
    if let Some(t) = total {
        if end >= t {
            return None;
        }
    }
    Some((start, end, total))
}

// ---------- pick_itag (crates/innertube/src/player.rs) ----------

/// `AudioFormat` reduced to the fields the filter reads: `url` is modelled by
/// its length (`Some(0)` is the empty string), `auto_dubbed` is verbatim.
pub struct Format {
    pub itag: u32,
    pub auto_dubbed: bool,
    pub url_len: Option<u32>,
}

pub open spec fn is_playable(f: Format) -> bool {
    !f.auto_dubbed
        && match f.url_len {
            None => false,
            Some(len) => len > 0,
        }
}

pub open spec fn playable_itag(formats: Seq<Format>, want: u32) -> bool {
    exists|i: int| 0 <= i < formats.len() && #[trigger] formats[i].itag == want && is_playable(formats[i])
}

/// `[251u32, 140].into_iter().find(|itag| playable(formats).any(|f| f.itag == itag))`
/// with `playable` = not auto-dubbed and a non-empty direct URL. The two
/// preference passes are unrolled index loops (same order, same short-circuit).
pub fn pick_itag(formats: &[Format]) -> (r: Option<u32>)
    ensures
        r.is_some() ==> (r.unwrap() == 251 || r.unwrap() == 140),
        r.is_some() ==> playable_itag(formats@, r.unwrap()),
        playable_itag(formats@, 251) ==> r == Some(251),
        !playable_itag(formats@, 251) && playable_itag(formats@, 140) ==> r == Some(140),
        !playable_itag(formats@, 251) && !playable_itag(formats@, 140) ==> r.is_none(),
{
    let mut j = 0;
    while j < formats.len()
        decreases formats@.len() - j,
    {
        if formats[j].itag == 251 && !formats[j].auto_dubbed && formats[j].url_len.is_some() && formats[j].url_len.unwrap() > 0 {
            return Some(251);
        }
        j += 1;
    }
    let mut k = 0;
    while k < formats.len()
        decreases formats@.len() - k,
    {
        if formats[k].itag == 140 && !formats[k].auto_dubbed && formats[k].url_len.is_some() && formats[k].url_len.unwrap() > 0 {
            return Some(140);
        }
        k += 1;
    }
    None
}

// ---------- base64url_nopad (crates/innertube/src/context.rs) ----------

pub const ALPHABET: [u8; 64] = *b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub open spec fn alphabet_seq() -> Seq<u8> {
    ALPHABET.view()
}

pub open spec fn in_b64_alphabet(c: u8) -> bool {
    exists|k: int| 0 <= k < 64 && #[trigger] alphabet_seq()[k] == c
}

pub open spec fn encoded_len(n: int) -> int {
    if n <= 0 {
        0
    } else {
        4 * (n / 3) + (if n % 3 == 0 { 0 } else { n % 3 + 1 })
    }
}

/// `bytes.chunks(3)` encoded through the 64-byte table; `String` -> `Vec<u8>`,
/// `chunks` -> an index loop with the same `chunk.len()` checks.
pub fn base64url_nopad(bytes: &[u8]) -> (out: Vec<u8>)
    ensures
        out@.len() as int == encoded_len(bytes@.len() as int),
        forall|i: int| 0 <= i < out@.len() ==> #[trigger] in_b64_alphabet(out@[i]),
{
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len()
        decreases (bytes@.len() as int - i as int + 2) / 3,
    {
        let remaining = bytes.len() - i;
        let b0 = bytes[i] as u32;
        let b1 = if remaining > 1 { bytes[i + 1] as u32 } else { 0 };
        let b2 = if remaining > 2 { bytes[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        let x0 = ((triple >> 18) & 0x3f) as usize;
        let x1 = ((triple >> 12) & 0x3f) as usize;
        let x2 = ((triple >> 6) & 0x3f) as usize;
        let x3 = (triple & 0x3f) as usize;
        out.push(ALPHABET[x0]);
        out.push(ALPHABET[x1]);
        if remaining > 1 {
            out.push(ALPHABET[x2]);
        }
        if remaining > 2 {
            out.push(ALPHABET[x3]);
        }
        i = if remaining <= 3 { bytes.len() } else { i + 3 };
    }
    out
}

} // verus!
