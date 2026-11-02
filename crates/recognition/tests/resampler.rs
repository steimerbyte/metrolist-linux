//! Resampler parity with `AudioResampler.kt` and PCM helpers.

use ml_recognition::{downmix_to_mono, pcm16le_to_samples, resample_pcm16};

struct Lcg(u32);

impl Lcg {
    fn new(seed: u32) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
}

fn noise_pcm(len: usize, seed: u32) -> Vec<u8> {
    let mut rng = Lcg::new(seed);
    let mut pcm = Vec::with_capacity(len * 2);
    for _ in 0..len {
        let sample = ((rng.next() >> 16) as u16 as i16).to_le_bytes();
        pcm.extend_from_slice(&sample);
    }
    pcm
}

#[test]
fn resampled_bytes_match_the_kotlin_reference() {
    let input = noise_pcm(1_000, 0x1357_2468);
    let expected = include_bytes!("fixtures/resample_44100_1000_16k.pcm");
    let actual = resample_pcm16(&input, 44_100, 16_000);
    assert_eq!(actual.as_slice(), expected.as_slice());
}

#[test]
fn one_second_at_any_rate_becomes_16000_samples() {
    // Output lengths observed from the compiled Kotlin `AudioResampler` loop.
    for (rate, samples) in [(44_100u32, 44_100usize), (48_000, 48_000), (12_345, 12_345)] {
        let input = noise_pcm(samples, 0x51A2_B3C4);
        let output = resample_pcm16(&input, rate, 16_000);
        assert_eq!(
            output.len(),
            16_000 * 2,
            "{rate} Hz -> 16 kHz should produce one second of output"
        );
        assert_eq!(pcm16le_to_samples(&output).len(), 16_000);
    }
}

#[test]
fn resampling_changes_sample_values() {
    let input = noise_pcm(4_410, 0x0BAD_F00D);
    let output = resample_pcm16(&input, 44_100, 16_000);
    assert_eq!(output.len(), 1_600 * 2);
    assert_ne!(output, input[..output.len()].to_vec());
}

#[test]
fn empty_odd_and_degenerate_inputs_are_safe() {
    assert!(resample_pcm16(&[], 44_100, 16_000).is_empty());
    assert!(resample_pcm16(&[0x7F], 44_100, 16_000).is_empty());
    assert!(resample_pcm16(&[], 0, 16_000).is_empty());
    assert!(resample_pcm16(&[1, 2, 3, 4], 22_050, 0).is_empty());
    // Odd trailing byte is dropped, not read out of bounds.
    let output = resample_pcm16(&[1, 0, 2, 0, 3], 44_100, 16_000);
    assert!(output.len().is_multiple_of(2));
}

#[test]
fn downmix_averages_interleaved_channels() {
    let stereo: Vec<u8> = [1000i16, 2000, -1000, 1000, -32768, 32767]
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    let mono = downmix_to_mono(&stereo, 2);
    assert_eq!(pcm16le_to_samples(&mono), vec![1500, 0, 0]);
    assert_eq!(downmix_to_mono(&stereo, 1), stereo);
}
