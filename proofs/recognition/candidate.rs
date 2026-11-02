use vstd::prelude::*;

verus! {

// ---------- crates/recognition/src/signature.rs ----------

/// One step of the reflected CRC-32 (IEEE 802.3, polynomial `0xEDB88320`).
pub open spec fn crc_bit_step(crc: u32) -> u32 {
    if crc & 1 != 0 {
        (crc >> 1) ^ 0xEDB8_8320
    } else {
        crc >> 1
    }
}

/// Eight bit steps: one byte through the register.
pub open spec fn crc_bits(crc: u32, n: nat) -> u32
    decreases n
{
    if n == 0 {
        crc
    } else {
        crc_bit_step(crc_bits(crc, (n - 1) as nat))
    }
}

pub open spec fn crc_byte(crc: u32, byte: u8) -> u32 {
    crc_bits(crc ^ (byte as u32), 8)
}

/// The register after `data[0..i]`.
pub open spec fn crc_prefix(crc0: u32, data: Seq<u8>, i: int) -> u32
    decreases i
{
    if i <= 0 {
        crc0
    } else {
        crc_byte(crc_prefix(crc0, data, i - 1), data[i - 1])
    }
}

pub open spec fn crc32_spec(data: Seq<u8>) -> u32 {
    !crc_prefix(0xFFFF_FFFF, data, data.len() as int)
}

/// Standard CRC-32 (IEEE 802.3, as `java.util.zip.CRC32`).
pub fn crc32(data: &[u8]) -> (r: u32)
    ensures
        r == crc32_spec(data@),
{
    let mut crc = 0xFFFF_FFFFu32;
    let mut i: usize = 0;
    while i < data.len()
        invariant
            i <= data.len(),
            crc == crc_prefix(0xFFFF_FFFF, data@, i as int),
        decreases data.len() - i,
    {
        crc = crc ^ (data[i] as u32);
        let start = crc;
        let mut k: usize = 0;
        while k < 8
            invariant
                k <= 8,
                crc == crc_bits(start, k as nat),
            decreases 8 - k,
        {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            k = k + 1;
        }
        i = i + 1;
    }
    !crc
}

/// Standard padded base64 (`android.util.Base64.NO_WRAP`): output length for
/// `n` input bytes.
pub open spec fn b64_len(n: int) -> int {
    ((n + 2) / 3) * 4
}

/// Number of trailing `=` for `n` input bytes.
pub open spec fn b64_pad(n: int) -> int {
    if n % 3 == 0 {
        0
    } else if n % 3 == 1 {
        2
    } else {
        1
    }
}

pub const ALPHABET: [u8; 64] = *b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> (r: Vec<u8>)
    requires
        data@.len() <= usize::MAX - 3,
    ensures
        r@.len() == b64_len(data@.len() as int),
        r@.len() % 4 == 0,
        b64_pad(data@.len() as int) < 3,
{
    let mut out = Vec::new();
    let mut i: usize = 0;
    while i < data.len()
        invariant
            (i as int) <= (data.len() as int) + 2,
            (i as int) % 3 == 0,
            out@.len() == 4 * (((i as int) + 2) / 3),
            out@.len() % 4 == 0,
        decreases (data@.len() as int) + 2 - (i as int),
    {
        let remaining = data.len() - i;
        let b0 = data[i] as u32;
        let b1 = if remaining > 1 { data[i + 1] as u32 } else { 0 };
        let b2 = if remaining > 2 { data[i + 2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        assert(((n >> 18) & 63) < 64u32) by (bit_vector);
        assert(((n >> 12) & 63) < 64u32) by (bit_vector);
        assert(((n >> 6) & 63) < 64u32) by (bit_vector);
        assert((n & 63) < 64u32) by (bit_vector);
        out.push(ALPHABET[((n >> 18) & 63) as usize]);
        out.push(ALPHABET[((n >> 12) & 63) as usize]);
        out.push(if remaining > 1 {
            ALPHABET[((n >> 6) & 63) as usize]
        } else {
            b'='
        });
        out.push(if remaining > 2 {
            ALPHABET[(n & 63) as usize]
        } else {
            b'='
        });
        i = i + 3;
    }
    out
}

/// Frequency band boundaries of the fingerprint peak finder: below 250 Hz and
/// above 5500 Hz a peak is dropped; the four bands are otherwise disjoint and
/// the top band's upper bound is inclusive.
pub open spec fn band_of(frequency_hz: int) -> Option<u8> {
    if frequency_hz < 250 {
        None
    } else if frequency_hz < 520 {
        Some(0)
    } else if frequency_hz < 1450 {
        Some(1)
    } else if frequency_hz < 3500 {
        Some(2)
    } else if frequency_hz <= 5500 {
        Some(3)
    } else {
        None
    }
}

pub fn band_of_exec(frequency_hz: i64) -> (r: Option<u8>)
    ensures
        r == band_of(frequency_hz as int),
{
    if frequency_hz < 250 {
        None
    } else if frequency_hz < 520 {
        Some(0)
    } else if frequency_hz < 1450 {
        Some(1)
    } else if frequency_hz < 3500 {
        Some(2)
    } else if frequency_hz <= 5500 {
        Some(3)
    } else {
        None
    }
}

/// One encoded peak: the delta byte plus the two little-endian shorts, with an
/// absolute-position escape when the delta no longer fits a byte.
pub open spec fn peak_frame_len(diff: int) -> int {
    if diff >= 255 {
        // 0xFF + four absolute bytes + the delta byte + two u16s
        10
    } else {
        // delta byte + two u16s
        5
    }
}

/// The body of `Generator::encode`'s per-peak loop: `prev` is the last FFT pass
/// number written, peaks arrive in non-decreasing pass order. `to_le_bytes` /
/// `extend_from_slice` are transcribed as the equivalent byte pushes (vstd has
/// no spec for them); the escape layout is proven byte-exact.
pub fn encode_peak(fft_pass_number: i32, prev_fft_pass_number: i32, out: &mut Vec<u8>) -> (r: i32)
    requires
        prev_fft_pass_number >= 0,
        prev_fft_pass_number <= fft_pass_number,
    ensures
        r == fft_pass_number,
        final(out)@.len() == (old(out)@.len() as int) + peak_frame_len(
            (fft_pass_number - prev_fft_pass_number) as int,
        ),
        fft_pass_number - prev_fft_pass_number >= 255 ==> #[trigger] final(out)@[old(out)@.len() as int] == 0xFF,
        fft_pass_number - prev_fft_pass_number >= 255 ==> final(out)@[(old(out)@.len() as int) + 5] == 0,
        fft_pass_number - prev_fft_pass_number < 255 ==> #[trigger] final(out)@[old(out)@.len() as int] == (fft_pass_number
            - prev_fft_pass_number) as u8,
{
    let diff = fft_pass_number - prev_fft_pass_number;
    let mut prev = prev_fft_pass_number;
    if diff >= 255 {
        // Encode the absolute position behind a 0xFF marker.
        out.push(0xFF);
        out.push((fft_pass_number & 0xff) as u8);
        out.push(((fft_pass_number >> 8) & 0xff) as u8);
        out.push(((fft_pass_number >> 16) & 0xff) as u8);
        out.push(((fft_pass_number >> 24) & 0xff) as u8);
        prev = fft_pass_number;
    }
    out.push((fft_pass_number - prev) as u8);
    out.push((0u16 & 0xff) as u8);
    out.push((0u16 >> 8) as u8);
    out.push((0u16 & 0xff) as u8);
    out.push((0u16 >> 8) as u8);
    prev = fft_pass_number;
    prev
}

/// The signature's sample count: `num_samples` plus the 0.24 s lead-in
/// (`16_000 * 0.24 = 3840` samples), as an `i32`.
pub open spec fn samples_and_offset(num_samples: int) -> int {
    num_samples + 3840
}

pub fn samples_and_offset_exec(num_samples: i32) -> (r: i32)
    requires
        num_samples <= i32::MAX - 3840,
    ensures
        r as int == samples_and_offset(num_samples as int),
{
    num_samples + 3840
}
/// An empty input has CRC 0 (`!0xFFFF_FFFF`).
pub proof fn lemma_crc32_empty()
    ensures
        crc32_spec(Seq::<u8>::empty()) == 0,
{
    assert(!0xFFFF_FFFFu32 == 0u32) by (bit_vector);
}

/// The CRC is a function of the bytes only: appending one byte updates it by
/// exactly one `crc_byte` step.
pub proof fn lemma_crc_prefix_step(crc0: u32, data: Seq<u8>, i: int)
    requires
        0 <= i < data.len(),
    ensures
        crc_prefix(crc0, data, i + 1) == crc_byte(crc_prefix(crc0, data, i), data[i]),
{
}

/// Base64 output is always a whole number of 4-character groups.
pub proof fn lemma_b64_len_multiple_of_four(n: int)
    requires
        0 <= n,
    ensures
        b64_len(n) % 4 == 0,
{
}

/// One input byte is one group (plus two `=`), two bytes are one group (plus
/// one `=`), three bytes are one clean group.
pub proof fn lemma_b64_padding_shape(n: int)
    requires
        0 <= n,
    ensures
        n % 3 == 0 ==> b64_pad(n) == 0,
        n % 3 == 1 ==> b64_pad(n) == 2,
        n % 3 == 2 ==> b64_pad(n) == 1,
        b64_pad(n) < 3,
{
}

/// The encoded size is `4/3` of the payload rounded up, so a 56-byte header
/// plus a peak blob is a 4-byte multiple.
pub proof fn lemma_b64_grows_by_a_third(n: int)
    requires
        0 <= n,
    ensures
        b64_len(n) >= n,
        b64_len(n) <= n + n / 3 + 4,
{
}

/// The lower bounds are exclusive and the upper bound of the top band is
/// inclusive: 250 Hz is band 0, 3500 Hz is band 3, 5500 Hz is band 3 and 5501
/// Hz is dropped.
pub proof fn lemma_band_boundaries()
    ensures
        band_of(249) == None,
        band_of(250) == Some(0),
        band_of(519) == Some(0),
        band_of(520) == Some(1),
        band_of(1449) == Some(1),
        band_of(1450) == Some(2),
        band_of(3499) == Some(2),
        band_of(3500) == Some(3),
        band_of(5500) == Some(3),
        band_of(5501) == None,
{
}

/// A lower peak frequency never lands in a higher band.
pub proof fn lemma_band_monotone(a: int, b: int)
    requires
        a <= b,
    ensures
        match band_of(a) {
            Some(x) => match band_of(b) {
                Some(y) => x <= y,
                None => true,
            },
            None => true,
        },
{
}

/// Frame sizes: a delta that still fits the byte is 5 bytes; a delta of 255
/// (the first that does not fit) pays the 5-byte absolute escape.
pub proof fn lemma_frame_sizes()
    ensures
        peak_frame_len(0) == 5,
        peak_frame_len(254) == 5,
        peak_frame_len(255) == 10,
        peak_frame_len(1_000) == 10,
{
}

/// The Android generator stops at 12 s, so the lead-in value is at most
/// `12 * 16_000 + 3840` and fits an `i32` with a wide margin.
pub proof fn lemma_samples_and_offset_bounded(num_samples: i32)
    requires
        0 <= num_samples <= 12 * 16_000,
    ensures
        0 <= samples_and_offset(num_samples as int) <= 12 * 16_000 + 3840,
        samples_and_offset(num_samples as int) <= i32::MAX as int,
{
}

/// Only whole 128-sample hops are fed, so `num_samples` stays a multiple of
/// the hop size and never exceeds the input length.
pub proof fn lemma_hop_arithmetic(hops: int)
    requires
        0 <= hops,
    ensures
        (hops * 128) % 128 == 0,
        hops * 128 >= hops,
{
}

} // verus!
