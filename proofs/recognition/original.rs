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
        decreases data.len() - i,
    {
        crc = crc ^ (data[i] as u32);
        let start = crc;
        let mut k: usize = 0;
        while k < 8
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
        decreases (data@.len() as int) + 2 - (i as int),
    {
        let remaining = data.len() - i;
        let b0 = data[i] as u32;
        let b1 = if remaining > 1 { data[i + 1] as u32 } else { 0 };
        let b2 = if remaining > 2 { data[i + 2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
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
} // verus!