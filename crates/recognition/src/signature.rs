//! Shazam-compatible audio fingerprinting.
//!
//! A line-by-line port of the upstream pure-Kotlin `ShazamSignatureGenerator`
//! (itself a port of SongRec/vibra's C++ implementation). The arithmetic order
//! is kept identical on purpose: the output bytes are only useful if they match
//! what the Android app sends, so the FFT here is the same iterative
//! Cooley-Tukey radix-2 DIT loop with the same accumulation order as the
//! Kotlin, not a general-purpose FFT. `tests/signature_golden.rs` pins the
//! result against signatures produced by compiling and running that Kotlin.

use crate::error::RecognitionError;

/// Sample rate the fingerprint algorithm expects (mono PCM16).
pub const SAMPLE_RATE: u32 = 16_000;
/// FFT window size.
pub const FFT_SIZE: usize = 2048;
/// Number of real-FFT bins kept from the 2048-point transform.
pub const FFT_OUTPUT_SIZE: usize = FFT_SIZE / 2 + 1;
/// Hard cap the Android generator stops at (combined with the 12 s limit).
pub const MAX_PEAKS: usize = 255;
/// Longest audio the Android generator processes.
pub const MAX_TIME_SECONDS: f64 = 12.0;

/// Spread ring buffer depth (FFT frames).
const RING_BUF_SIZE: usize = 256;
/// Samples fed per FFT pass.
const HOP: usize = 128;

const BAND_250_520: usize = 0;
const BAND_520_1450: usize = 1;
const BAND_1450_3500: usize = 2;
const BAND_3500_5500: usize = 3;

/// Frequency of one corrected bin in Hz (`16000 / 2 / 1024 / 64`).
const BIN_HZ: f64 = SAMPLE_RATE as f64 / 2.0 / 1024.0 / 64.0;

/// Decode little-endian PCM16 bytes into samples (a trailing odd byte is dropped).
pub fn pcm16le_to_samples(pcm: &[u8]) -> Vec<i16> {
    pcm.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes(*pair))
        .collect()
}

/// Encode samples as little-endian PCM16 bytes.
pub fn samples_to_pcm16le(samples: &[i16]) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    pcm
}

/// Generate the raw Shazam signature from 16 kHz mono PCM16 bytes.
///
/// Fails only on empty or odd-length input, mirroring the Kotlin `require`.
pub fn uri_from_pcm16le(pcm: &[u8]) -> Result<String, RecognitionError> {
    if pcm.is_empty() || !pcm.len().is_multiple_of(2) {
        return Err(RecognitionError::InvalidAudio);
    }
    Ok(uri(&pcm16le_to_samples(pcm)))
}

/// Generate the `data:audio/vnd.shazam.sig;base64,...` URI from samples.
pub fn uri(samples: &[i16]) -> String {
    let mut out = String::from("data:audio/vnd.shazam.sig;base64,");
    out.push_str(&base64_encode(&bytes(samples)));
    out
}

/// Generate the raw signature bytes from 16 kHz mono PCM16 samples.
pub fn bytes(samples: &[i16]) -> Vec<u8> {
    Generator::new().process(samples).encode()
}

/// Frequency peak recorded in a band bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrequencyPeak {
    fft_pass_number: i32,
    peak_magnitude: i32,
    corrected_peak_frequency_bin: i32,
}

struct Generator {
    samples_ring: [i32; FFT_SIZE],
    samples_pos: usize,
    fft_outputs: Vec<f64>,
    fft_pos: usize,
    spread_ffts: Vec<f64>,
    spread_pos: usize,
    spread_num_written: u32,
    num_samples: i32,
    band_peaks: [Vec<FrequencyPeak>; 4],
    total_peaks: usize,
    hanning: [f64; FFT_SIZE],
}

impl Generator {
    fn new() -> Self {
        let mut hanning = [0.0f64; FFT_SIZE];
        for (i, value) in hanning.iter_mut().enumerate() {
            // Same expression as the Kotlin/FFTW HANNIG_MATRIX: 0.5*(1-cos(2π(i+1)/2049)).
            *value = 0.5
                * (1.0
                    - (2.0 * std::f64::consts::PI * (i + 1) as f64 / (FFT_SIZE + 1) as f64).cos());
        }
        Self {
            samples_ring: [0; FFT_SIZE],
            samples_pos: 0,
            fft_outputs: vec![0.0; RING_BUF_SIZE * FFT_OUTPUT_SIZE],
            fft_pos: 0,
            spread_ffts: vec![0.0; RING_BUF_SIZE * FFT_OUTPUT_SIZE],
            spread_pos: 0,
            spread_num_written: 0,
            num_samples: 0,
            band_peaks: Default::default(),
            total_peaks: 0,
            hanning,
        }
    }

    fn process(&mut self, samples: &[i16]) -> &mut Self {
        let mut offset = 0;
        while offset + HOP <= samples.len() {
            // Match the C++ stopping condition: stop only when both 12 s and 255 peaks are reached.
            let elapsed_sec = self.num_samples as f64 / SAMPLE_RATE as f64;
            if elapsed_sec >= MAX_TIME_SECONDS && self.total_peaks >= MAX_PEAKS {
                break;
            }
            self.num_samples += HOP as i32;
            self.feed_samples(&samples[offset..offset + HOP]);
            self.do_fft();
            self.do_peak_spreading_and_recognition();
            offset += HOP;
        }
        self
    }

    fn feed_samples(&mut self, samples: &[i16]) {
        for &sample in samples {
            self.samples_ring[self.samples_pos] = i32::from(sample);
            self.samples_pos = (self.samples_pos + 1) % FFT_SIZE;
        }
    }

    fn do_fft(&mut self) {
        let mut windowed = [0.0f64; FFT_SIZE];
        for (i, value) in windowed.iter_mut().enumerate() {
            *value = self.samples_ring[(self.samples_pos + i) % FFT_SIZE] as f64 * self.hanning[i];
        }
        let mut magnitudes = [0.0f64; FFT_OUTPUT_SIZE];
        compute_rfft(&windowed, &mut magnitudes);
        let base = self.fft_pos * FFT_OUTPUT_SIZE;
        self.fft_outputs[base..base + FFT_OUTPUT_SIZE].copy_from_slice(&magnitudes);
        self.fft_pos = (self.fft_pos + 1) % RING_BUF_SIZE;
    }

    fn do_peak_spreading_and_recognition(&mut self) {
        self.do_peak_spreading();
        if self.spread_num_written >= 47 {
            self.do_peak_recognition();
        }
    }

    fn do_peak_spreading(&mut self) {
        let last_fft_idx = (self.fft_pos + RING_BUF_SIZE - 1) % RING_BUF_SIZE;
        let mut spread = [0.0f64; FFT_OUTPUT_SIZE];
        spread.copy_from_slice(
            &self.fft_outputs[last_fft_idx * FFT_OUTPUT_SIZE..][..FFT_OUTPUT_SIZE],
        );

        // Frequency spreading: in-place 3-point running max (forward pass).
        for pos in 0..FFT_OUTPUT_SIZE - 2 {
            spread[pos] = spread[pos].max(spread[pos + 1]).max(spread[pos + 2]);
        }

        // Time spreading: propagate the max into older entries at -1, -3, -6.
        // The new entry keeps only frequency spreading (matches the C++ vibra).
        for (pos, &value) in spread.iter().enumerate() {
            let mut max_val = value;
            for offset in [-1i32, -3, -6] {
                let idx =
                    (self.spread_pos as i32 + offset).rem_euclid(RING_BUF_SIZE as i32) as usize;
                let old_val = self.spread_ffts[idx * FFT_OUTPUT_SIZE + pos];
                if old_val > max_val {
                    max_val = old_val;
                }
                self.spread_ffts[idx * FFT_OUTPUT_SIZE + pos] = max_val;
            }
        }

        let base = self.spread_pos * FFT_OUTPUT_SIZE;
        self.spread_ffts[base..base + FFT_OUTPUT_SIZE].copy_from_slice(&spread);
        self.spread_pos = (self.spread_pos + 1) % RING_BUF_SIZE;
        self.spread_num_written += 1;
    }

    fn do_peak_recognition(&mut self) {
        let fft_minus_46 = (self.fft_pos + RING_BUF_SIZE * 2 - 46) % RING_BUF_SIZE;
        let spread_minus_49 = (self.spread_pos + RING_BUF_SIZE * 2 - 49) % RING_BUF_SIZE;

        let other_offsets = [
            -53i32, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249,
        ];
        let neighbor_offsets = [-10i32, -7, -4, -3, 1, 2, 5, 8];

        for bin_pos in 10..FFT_OUTPUT_SIZE - 8 {
            let fft_val = self.fft_outputs[fft_minus_46 * FFT_OUTPUT_SIZE + bin_pos];
            if fft_val < 1.0 / 64.0
                || fft_val < self.spread_ffts[spread_minus_49 * FFT_OUTPUT_SIZE + bin_pos]
            {
                continue;
            }

            let mut max_neighbor_spread49 = 0.0f64;
            for offset in neighbor_offsets {
                let value = self.spread_ffts
                    [spread_minus_49 * FFT_OUTPUT_SIZE + (bin_pos as i32 + offset) as usize];
                if value > max_neighbor_spread49 {
                    max_neighbor_spread49 = value;
                }
            }
            if fft_val <= max_neighbor_spread49 {
                continue;
            }

            let mut max_neighbor_other = max_neighbor_spread49;
            for offset in other_offsets {
                let spread_idx =
                    (self.spread_pos as i32 + offset).rem_euclid(RING_BUF_SIZE as i32) as usize;
                let value = self.spread_ffts[spread_idx * FFT_OUTPUT_SIZE + bin_pos - 1];
                if value > max_neighbor_other {
                    max_neighbor_other = value;
                }
            }
            if fft_val <= max_neighbor_other {
                continue;
            }

            let fft_number = self.spread_num_written as i32 - 46;

            let peak_mag = (fft_val.max(1.0 / 64.0)).ln() * 1477.3 + 6144.0;
            let peak_mag_before = (self.fft_outputs[fft_minus_46 * FFT_OUTPUT_SIZE + bin_pos - 1]
                .max(1.0 / 64.0))
            .ln()
                * 1477.3
                + 6144.0;
            let peak_mag_after = (self.fft_outputs[fft_minus_46 * FFT_OUTPUT_SIZE + bin_pos + 1]
                .max(1.0 / 64.0))
            .ln()
                * 1477.3
                + 6144.0;

            let peak_variation1 = peak_mag * 2.0 - peak_mag_before - peak_mag_after;
            let peak_variation2 = (peak_mag_after - peak_mag_before) * 32.0 / peak_variation1;

            let corrected_bin = bin_pos as f64 * 64.0 + peak_variation2;
            let frequency_hz = corrected_bin * BIN_HZ;

            let band = if frequency_hz < 250.0 {
                continue;
            } else if frequency_hz < 520.0 {
                BAND_250_520
            } else if frequency_hz < 1450.0 {
                BAND_520_1450
            } else if frequency_hz < 3500.0 {
                BAND_1450_3500
            } else if frequency_hz <= 5500.0 {
                BAND_3500_5500
            } else {
                continue;
            };

            self.band_peaks[band].push(FrequencyPeak {
                fft_pass_number: fft_number,
                peak_magnitude: peak_mag as i32,
                corrected_peak_frequency_bin: corrected_bin as i32,
            });
            self.total_peaks += 1;
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut contents = Vec::new();

        for (band_id, peaks) in self.band_peaks.iter().enumerate() {
            if peaks.is_empty() {
                continue;
            }

            let mut peak_buf = Vec::new();
            let mut prev_fft_pass_number = 0i32;

            for peak in peaks {
                let diff = peak.fft_pass_number - prev_fft_pass_number;
                if diff >= 255 {
                    // Encode the absolute position behind a 0xFF marker.
                    peak_buf.push(0xFF);
                    peak_buf.extend_from_slice(&peak.fft_pass_number.to_le_bytes());
                    prev_fft_pass_number = peak.fft_pass_number;
                }
                peak_buf.push((peak.fft_pass_number - prev_fft_pass_number) as u8);
                peak_buf.extend_from_slice(&(peak.peak_magnitude as u16).to_le_bytes());
                peak_buf
                    .extend_from_slice(&(peak.corrected_peak_frequency_bin as u16).to_le_bytes());
                prev_fft_pass_number = peak.fft_pass_number;
            }

            // Band tag: 0x60030040 + band id, then the peak blob, padded to 4 bytes.
            contents.extend_from_slice(&(0x6003_0040u32 + band_id as u32).to_le_bytes());
            contents.extend_from_slice(&(peak_buf.len() as u32).to_le_bytes());
            contents.extend_from_slice(&peak_buf);
            let pad_bytes = (4 - peak_buf.len() % 4) % 4;
            contents.extend(std::iter::repeat_n(0u8, pad_bytes));
        }

        let size_minus_header = contents.len() as i32 + 8;
        let samples_and_offset = (self.num_samples as f64 + SAMPLE_RATE as f64 * 0.24) as i32;

        let mut full = Vec::with_capacity(56 + contents.len());
        // 48-byte header, every field little-endian; layout matches the Kotlin ByteBuffer.
        full.extend_from_slice(&0xcafe_2580u32.to_le_bytes()); // magic1
        full.extend_from_slice(&0u32.to_le_bytes()); // crc32 placeholder
        full.extend_from_slice(&size_minus_header.to_le_bytes()); // size_minus_header
        full.extend_from_slice(&0x9411_9c00u32.to_le_bytes()); // magic2
        full.extend_from_slice(&[0u8; 12]); // void1[3]
        full.extend_from_slice(&(3i32 << 27).to_le_bytes()); // shifted_sample_rate_id
        full.extend_from_slice(&[0u8; 8]); // void2[2]
        full.extend_from_slice(&samples_and_offset.to_le_bytes());
        full.extend_from_slice(&((15i32 << 19) + 0x40000).to_le_bytes()); // fixed_value
        full.extend_from_slice(&0x4000_0000u32.to_le_bytes());
        full.extend_from_slice(&(contents.len() as u32 + 8).to_le_bytes());
        full.extend_from_slice(&contents);

        // CRC32 over bytes [8..], stored at offset 4 (little-endian).
        let crc = crc32(&full[8..]).to_le_bytes();
        full[4..8].copy_from_slice(&crc);
        full
    }
}

/// Iterative Cooley-Tukey radix-2 DIT FFT, identical in operation order to the
/// Kotlin generator. `out[k] = max((re[k]² + im[k]²) / 2^17, 1e-10)`.
fn compute_rfft(windowed: &[f64; FFT_SIZE], out: &mut [f64; FFT_OUTPUT_SIZE]) {
    let n = FFT_SIZE;
    let mut re = *windowed;
    let mut im = [0.0f64; FFT_SIZE];

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterfly stages (11 stages for n=2048).
    let mut len = 2usize;
    while len <= n {
        let half_len = len >> 1;
        let ang = -std::f64::consts::PI / half_len as f64;
        let w_base_re = ang.cos();
        let w_base_im = ang.sin();
        let mut i = 0usize;
        while i < n {
            let mut w_re = 1.0f64;
            let mut w_im = 0.0f64;
            for k in 0..half_len {
                let u = i + k;
                let v = u + half_len;
                let even_re = re[u];
                let even_im = im[u];
                let odd_re = re[v] * w_re - im[v] * w_im;
                let odd_im = re[v] * w_im + im[v] * w_re;
                re[u] = even_re + odd_re;
                im[u] = even_im + odd_im;
                re[v] = even_re - odd_re;
                im[v] = even_im - odd_im;
                let new_w_re = w_re * w_base_re - w_im * w_base_im;
                w_im = w_re * w_base_im + w_im * w_base_re;
                w_re = new_w_re;
            }
            i += len;
        }
        len <<= 1;
    }

    let scale_factor = 1.0 / (1u64 << 17) as f64;
    let min_val = 1e-10;
    for (idx, value) in out.iter_mut().enumerate() {
        let mag = (re[idx] * re[idx] + im[idx] * im[idx]) * scale_factor;
        *value = if mag < min_val { min_val } else { mag };
    }
}

/// Standard CRC-32 (IEEE 802.3, as `java.util.zip.CRC32`).
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Standard padded base64 (`android.util.Base64.NO_WRAP`).
pub(crate) fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_java_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn invalid_pcm_is_rejected() {
        assert_eq!(uri_from_pcm16le(&[]), Err(RecognitionError::InvalidAudio));
        assert_eq!(
            uri_from_pcm16le(&[1, 2, 3]),
            Err(RecognitionError::InvalidAudio)
        );
    }

    #[test]
    fn uri_prefix_and_base64_length() {
        let samples = vec![0i16; 1600];
        let sig = bytes(&samples);
        let uri = uri(&samples);
        assert_eq!(sig.len(), 56);
        assert_eq!(
            uri,
            format!("data:audio/vnd.shazam.sig;base64,{}", base64_encode(&sig))
        );
        let payload_len = uri.len() - "data:audio/vnd.shazam.sig;base64,".len();
        assert_eq!(payload_len, sig.len().div_ceil(3) * 4);
    }

    #[test]
    fn hand_rolled_fft_agrees_with_rustfft() {
        use rustfft::num_complex::Complex;
        use rustfft::FftPlanner;

        let mut input = [0.0f64; FFT_SIZE];
        let mut state = 0x1234_5678u32;
        for value in &mut input {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *value = f64::from((state >> 16) as u16 as i16);
        }

        let mut ours = [0.0f64; FFT_OUTPUT_SIZE];
        compute_rfft(&input, &mut ours);

        let mut buffer: Vec<Complex<f64>> = input
            .iter()
            .map(|&value| Complex::new(value, 0.0))
            .collect();
        FftPlanner::<f64>::new()
            .plan_fft_forward(FFT_SIZE)
            .process(&mut buffer);

        for bin in 0..FFT_OUTPUT_SIZE {
            let magnitude = (buffer[bin].re * buffer[bin].re + buffer[bin].im * buffer[bin].im)
                / (1u64 << 17) as f64;
            let expected = magnitude.max(1e-10);
            let relative = (ours[bin] - expected).abs() / expected;
            assert!(
                relative < 1e-12,
                "bin {bin}: ported FFT {} vs rustfft {expected}",
                ours[bin]
            );
        }
    }

    #[test]
    fn header_fields_of_a_silent_signature() {
        let sig = bytes(&vec![0i16; 1600]);
        assert_eq!(&sig[0..4], &0xcafe_2580u32.to_le_bytes());
        assert_eq!(&sig[12..16], &0x9411_9c00u32.to_le_bytes());
        assert_eq!(&sig[48..52], &0x4000_0000u32.to_le_bytes());
        let size_minus_header = u32::from_le_bytes(sig[8..12].try_into().unwrap());
        assert_eq!(size_minus_header as usize, sig.len() - 48);
        let samples_and_offset = i32::from_le_bytes(sig[40..44].try_into().unwrap());
        // Only full 128-sample hops are fed (1600 / 128 = 12 hops), plus 0.24 s.
        assert_eq!(samples_and_offset, 12 * 128 + 3840);
        // A silent buffer has no peaks: only the two descriptor ints follow the header.
        assert_eq!(sig.len(), 48 + 8);
    }
}
