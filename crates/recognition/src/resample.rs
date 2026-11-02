//! PCM helpers: channel downmix and the linear resampler used upstream.
//!
//! `AudioResampler.kt` resamples the raw interleaved PCM stream with linear
//! interpolation and truncates the queried position to an integer sample index.
//! That is reproduced here exactly, including the `(samples * ratio) as int`
//! output length, so the resulting PCM matches the Android pipeline byte for byte.

use crate::signature::pcm16le_to_samples;

/// Resample interleaved PCM16 little-endian audio from `from_rate` to `to_rate`.
///
/// Mirrors `AudioResampler.resample`: no anti-aliasing filter, truncating
/// position lookup, and the last input sample is repeated for the final output
/// samples. Equal rates return the input unchanged, as upstream does.
pub fn resample_pcm16(data: &[u8], from_rate: u32, to_rate: u32) -> Vec<u8> {
    if from_rate == to_rate {
        return data.to_vec();
    }
    if from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }

    let input = pcm16le_to_samples(data);
    if input.is_empty() {
        return Vec::new();
    }

    let ratio = f64::from(to_rate) / f64::from(from_rate);
    let output_len = (input.len() as f64 * ratio) as usize;
    let mut output = Vec::with_capacity(output_len * 2);

    for i in 0..output_len {
        let src_pos = i as f64 / ratio;
        let src_index = src_pos as usize;
        let fraction = src_pos - src_index as f64;
        let sample = if src_index + 1 < input.len() {
            // Kotlin: (a*(1-fraction) + b*fraction).toInt().toShort()
            (f64::from(input[src_index]) * (1.0 - fraction)
                + f64::from(input[src_index + 1]) * fraction) as i32 as i16
        } else {
            input[src_index]
        };
        output.extend_from_slice(&sample.to_le_bytes());
    }

    output
}

/// Average interleaved PCM16 channels down to mono (truncating toward zero).
///
/// Upstream only ever records mono, so this is the Linux-side equivalent for
/// stereo microphone configs and stereo files.
pub fn downmix_to_mono(data: &[u8], channels: u16) -> Vec<u8> {
    if channels <= 1 {
        return data.to_vec();
    }
    let channels = usize::from(channels);
    let samples = pcm16le_to_samples(data);
    let frames = samples.len() / channels;
    let mut output = Vec::with_capacity(frames * 2);

    for frame in 0..frames {
        let sum: i32 = samples[frame * channels..(frame + 1) * channels]
            .iter()
            .map(|&sample| i32::from(sample))
            .sum();
        let mono = (sum / channels as i32) as i16;
        output.extend_from_slice(&mono.to_le_bytes());
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_rate_is_a_passthrough() {
        let data: Vec<u8> = (0..64i16).flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(resample_pcm16(&data, 16_000, 16_000), data);
    }

    #[test]
    fn empty_and_odd_input_do_not_panic() {
        assert!(resample_pcm16(&[], 44_100, 16_000).is_empty());
        assert!(resample_pcm16(&[7], 44_100, 16_000).is_empty());
        assert_eq!(resample_pcm16(&[7], 16_000, 16_000), vec![7]);
        assert!(downmix_to_mono(&[], 2).is_empty());
        assert!(downmix_to_mono(&[1, 2], 2).is_empty()); // half a stereo frame
        assert_eq!(downmix_to_mono(&[10, 0, 20, 0], 2).len(), 2);
    }

    #[test]
    fn output_length_matches_the_kotlin_formula() {
        // 1 s at 44.1 kHz and 48 kHz both land on 16000 samples after resampling.
        assert_eq!(
            resample_pcm16(&vec![0; 44_100 * 2], 44_100, 16_000).len(),
            16_000 * 2
        );
        assert_eq!(
            resample_pcm16(&vec![0; 48_000 * 2], 48_000, 16_000).len(),
            16_000 * 2
        );
        // Arbitrary rate: 12345 samples at 12345 Hz is one second.
        assert_eq!(
            resample_pcm16(&vec![0; 12_345 * 2], 12_345, 16_000).len(),
            16_000 * 2
        );
    }

    #[test]
    fn constant_signal_stays_constant() {
        let pcm: Vec<u8> = (0..1000i16).flat_map(|_| 1234i16.to_le_bytes()).collect();
        let out = resample_pcm16(&pcm, 44_100, 16_000);
        assert!(!out.is_empty());
        assert!(out
            .as_chunks::<2>()
            .0
            .iter()
            .all(|pair| *pair == 1234i16.to_le_bytes()));
    }

    #[test]
    fn stereo_downmix_averages_channels() {
        let frames: [(i16, i16); 4] = [(100, 200), (-100, 100), (i16::MIN, i16::MAX), (7, 8)];
        let data: Vec<u8> = frames
            .iter()
            .flat_map(|&(left, right)| left.to_le_bytes().into_iter().chain(right.to_le_bytes()))
            .collect();
        let mono = downmix_to_mono(&data, 2);
        let samples: Vec<i16> = pcm16le_to_samples(&mono);
        assert_eq!(samples, vec![150, 0, 0, 7]);
    }
}
