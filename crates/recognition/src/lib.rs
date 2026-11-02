//! `ml-recognition` — Shazam-compatible song recognition.
//!
//! The pipeline mirrors the Android app (`recognition/*` + `com.metrolist.shazamkit`):
//!
//! 1. [`capture::capture`] records up to 12 s from a cpal input device (or
//!    [`wav::read_wav`] loads a file), returning [`PcmAudio`].
//! 2. [`PcmAudio::to_16k_mono_pcm16`] downmixes to mono and resamples to 16 kHz
//!    with the same linear interpolation as `AudioResampler.kt`.
//! 3. [`signature_uri_from_pcm16le`] builds the `data:audio/vnd.shazam.sig;base64,...`
//!    fingerprint with a byte-exact port of `ShazamSignatureGenerator.kt`.
//! 4. [`ShazamClient::recognize`] posts it to Shazam's discovery endpoint and
//!    maps the reply to [`RecognitionResult`].
//!
//! The fingerprint bytes are pinned against signatures produced by the upstream
//! Kotlin code in `tests/signature_golden.rs`.

#![forbid(unsafe_code)]

pub mod capture;
pub mod error;
pub mod resample;
pub mod shazam;
pub mod signature;
pub mod wav;

pub use capture::{capture, capture_until, list_input_devices, CaptureOptions, InputDevice};
pub use error::RecognitionError;
pub use resample::{downmix_to_mono, resample_pcm16};
pub use shazam::{RecognitionResult, ShazamClient};
pub use signature::{
    bytes as signature_bytes, pcm16le_to_samples, samples_to_pcm16le, uri as signature_uri,
    uri_from_pcm16le as signature_uri_from_pcm16le, SAMPLE_RATE,
};
pub use wav::read_wav;

/// Decoded PCM audio: interleaved signed 16-bit samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmAudio {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmAudio {
    /// Duration of the buffer in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        if self.channels == 0 {
            return 0;
        }
        let frames = self.samples.len() as u64 / u64::from(self.channels);
        frames * 1000 / u64::from(self.sample_rate.max(1))
    }

    /// Interleaved PCM16 little-endian bytes.
    pub fn to_pcm16le(&self) -> Vec<u8> {
        samples_to_pcm16le(&self.samples)
    }

    /// Mono 16 kHz PCM16 little-endian, ready for [`signature_uri_from_pcm16le`].
    pub fn to_16k_mono_pcm16(&self) -> Vec<u8> {
        let pcm = self.to_pcm16le();
        let mono = if self.channels <= 1 {
            pcm
        } else {
            downmix_to_mono(&pcm, self.channels)
        };
        resample_pcm16(&mono, self.sample_rate, SAMPLE_RATE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_uses_frames_not_samples() {
        let stereo = PcmAudio {
            samples: vec![0; 96_000],
            sample_rate: 48_000,
            channels: 2,
        };
        assert_eq!(stereo.duration_ms(), 1000);
    }

    #[test]
    fn mono_audio_is_not_downmixed_twice() {
        let audio = PcmAudio {
            samples: vec![1000; 48_000],
            sample_rate: 48_000,
            channels: 1,
        };
        let pcm = audio.to_16k_mono_pcm16();
        assert_eq!(pcm.len(), 16_000 * 2);
        assert!(pcm16le_to_samples(&pcm)
            .iter()
            .all(|&sample| (999..=1000).contains(&sample)));
    }

    #[test]
    fn stereo_audio_is_downmixed_before_resampling() {
        let frames = 48_000;
        let mut samples = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            samples.push(1000);
            samples.push(3000);
        }
        let audio = PcmAudio {
            samples,
            sample_rate: 48_000,
            channels: 2,
        };
        let pcm = audio.to_16k_mono_pcm16();
        let mono = pcm16le_to_samples(&pcm);
        assert_eq!(mono.len(), 16_000);
        assert!(mono.iter().all(|&sample| (1999..=2000).contains(&sample)));
    }
}
