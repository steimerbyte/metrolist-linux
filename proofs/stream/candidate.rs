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
    assert(chunk > 0);
    assert(total > 0);
    let mut offset = 0u64;
    while offset < total
        invariant
            chunk > 0,
            total > 0,
            offset <= total,
            ranges@.len() == 0 ==> offset == 0,
            forall|i: int| 0 <= i < ranges@.len() ==> #[trigger] ranges@[i].0 <= ranges@[i].1 && ranges@[i].1 < total,
            ranges@.len() == 0 || ranges@[0].0 == 0,
            forall|i: int| 0 <= i < ranges@.len() - 1 ==> #[trigger] ranges@[i + 1].0 == ranges@[i].1 + 1,
            ranges@.len() == 0 || ranges@[ranges@.len() - 1].1 + 1 == offset,
            forall|i: int| 0 <= i < ranges@.len() ==> #[trigger] ranges@[i].1 - ranges@[i].0 + 1 <= chunk,
            forall|i: int| 0 <= i < ranges@.len() ==> #[trigger] ranges@[i].1 as int == total as int - 1 || ranges@[i].1 as int == ranges@[i].0 as int + (chunk as int) - 1,
            ranges@.len() as int <= offset as int,
        decreases (total - offset) as int,
    {
        let end = offset.saturating_add(chunk - 1).min(total - 1);
        assert(end >= offset);
        if end < total - 1 {
            assert(offset.saturating_add((chunk - 1) as u64) as int
                == if offset as int + (chunk as int) - 1 > u64::MAX as int {
                    u64::MAX as int
                } else {
                    offset as int + (chunk as int) - 1
                });
            assert(offset as int + (chunk as int) - 1 <= u64::MAX as int);
            assert(end as int == offset as int + (chunk as int) - 1);
        }
        ranges.push((offset, end));
        offset = end.saturating_add(1);
    }
    proof {
        assert forall|i: int| 0 <= i < ranges@.len() - 1 implies
            #[trigger] ranges@[i].1 - ranges@[i].0 + 1 == chunk by {
            assert(ranges@[i].1 as int == total as int - 1
                || ranges@[i].1 as int == ranges@[i].0 as int + (chunk as int) - 1);
            assert(ranges@[i + 1].0 == ranges@[i].1 + 1);
            assert(ranges@[i + 1].1 < total);
            assert(ranges@[i + 1].0 <= ranges@[i + 1].1);
        }
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
    assert(now as int + delta as int <= u64::MAX as int ==> delta <= u64::MAX);
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

proof fn lemma_not_playable(formats: Seq<Format>, want: u32)
    requires forall|l: int| 0 <= l < formats.len() ==> !(#[trigger] formats[l].itag == want && is_playable(formats[l])),
    ensures !playable_itag(formats, want),
{
    if playable_itag(formats, want) {
        let i = choose|i: int| 0 <= i < formats.len() && formats[i].itag == want && is_playable(formats[i]);
        assert(formats[i].itag == want && is_playable(formats[i]));
    }
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
        invariant
            j <= formats@.len(),
            forall|l: int| 0 <= l < j ==> !(#[trigger] formats@[l].itag == 251 && is_playable(formats@[l])),
        decreases formats@.len() - j,
    {
        if formats[j].itag == 251 && !formats[j].auto_dubbed && formats[j].url_len.is_some() && formats[j].url_len.unwrap() > 0 {
            return Some(251);
        }
        j += 1;
    }
    proof {
        lemma_not_playable(formats@, 251);
    }
    let mut k = 0;
    while k < formats.len()
        invariant
            k <= formats@.len(),
            !playable_itag(formats@, 251),
            forall|l: int| 0 <= l < k ==> !(#[trigger] formats@[l].itag == 140 && is_playable(formats@[l])),
        decreases formats@.len() - k,
    {
        if formats[k].itag == 140 && !formats[k].auto_dubbed && formats[k].url_len.is_some() && formats[k].url_len.unwrap() > 0 {
            return Some(140);
        }
        k += 1;
    }
    proof {
        lemma_not_playable(formats@, 140);
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

proof fn lemma_table_byte_in_alphabet(k: int)
    requires 0 <= k < 64,
    ensures in_b64_alphabet(alphabet_seq()[k]),
{
}

proof fn lemma_encoded_len_group(i: int)
    requires 0 <= i,
    ensures encoded_len(i + 3) == encoded_len(i) + 4,
{
    assert(i % 3 == 0 || i % 3 == 1 || i % 3 == 2) by {
        assert((i / 3) * 3 + i % 3 == i) by (nonlinear_arith);
    }
}

proof fn lemma_encoded_len_tail(i: int, remaining: int)
    requires 1 <= remaining <= 3, 0 <= i, i % 3 == 0,
    ensures
        encoded_len(i + remaining) == encoded_len(i) + 2
            + (if remaining > 1 { 1int } else { 0int })
            + (if remaining > 2 { 1int } else { 0int }),
{
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
        invariant
            i <= bytes@.len(),
            i as int == bytes@.len() as int || i as int % 3 == 0,
            out@.len() as int == encoded_len(i as int),
            forall|k: int| 0 <= k < out@.len() ==> #[trigger] in_b64_alphabet(out@[k]),
        decreases (bytes@.len() as int - i as int + 2) / 3,
    {
        let remaining = bytes.len() - i;
        let b0 = bytes[i] as u32;
        let b1 = if remaining > 1 { bytes[i + 1] as u32 } else { 0 };
        let b2 = if remaining > 2 { bytes[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        assert(((triple >> 18) & 0x3f) < 64) by (bit_vector);
        assert(((triple >> 12) & 0x3f) < 64) by (bit_vector);
        assert(((triple >> 6) & 0x3f) < 64) by (bit_vector);
        assert((triple & 0x3f) < 64) by (bit_vector);
        let x0 = ((triple >> 18) & 0x3f) as usize;
        let x1 = ((triple >> 12) & 0x3f) as usize;
        let x2 = ((triple >> 6) & 0x3f) as usize;
        let x3 = (triple & 0x3f) as usize;
        proof { lemma_table_byte_in_alphabet(x0 as int); }
        out.push(ALPHABET[x0]);
        proof { lemma_table_byte_in_alphabet(x1 as int); }
        out.push(ALPHABET[x1]);
        if remaining > 1 {
            proof { lemma_table_byte_in_alphabet(x2 as int); }
            out.push(ALPHABET[x2]);
        }
        if remaining > 2 {
            proof { lemma_table_byte_in_alphabet(x3 as int); }
            out.push(ALPHABET[x3]);
        }
        proof {
            if remaining <= 3 {
                lemma_encoded_len_tail(i as int, remaining as int);
            } else {
                lemma_encoded_len_group(i as int);
            }
        }
        i = if remaining <= 3 { bytes.len() } else { i + 3 };
    }
    out
}

} // verus!
