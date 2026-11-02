//! WAV loading for the `recognize-probe --input` path.

use std::path::Path;

use hound::{SampleFormat, WavReader};

use crate::{PcmAudio, RecognitionError};

/// Read a mono/stereo WAV file as interleaved PCM16 samples.
///
/// Supports 16-bit integer and 32-bit float WAVs (what ffmpeg produces by
/// default for a cut-out test segment); everything else is refused instead of
/// being guessed at.
pub fn read_wav(path: impl AsRef<Path>) -> Result<PcmAudio, RecognitionError> {
    let path = path.as_ref();
    let mut reader = WavReader::open(path).map_err(|error| wav_error(path, error))?;
    let spec = reader.spec();

    let samples: Vec<i16> = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| wav_error(path, error))?,
        (SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .map(|sample| {
                sample
                    .map(|value| (value.clamp(-1.0, 1.0) * 32_767.0).round() as i16)
                    .map_err(|error| wav_error(path, error))
            })
            .collect::<Result<Vec<_>, _>>()?,
        (format, bits) => {
            return Err(RecognitionError::UnsupportedFormat(format!(
                "{}: {bits}-bit {format:?} WAV is not supported (use 16-bit PCM or 32-bit float)",
                path.display()
            )))
        }
    };

    if spec.channels == 0 || spec.sample_rate == 0 {
        return Err(RecognitionError::UnsupportedFormat(format!(
            "{}: WAV has no channels or sample rate",
            path.display()
        )));
    }

    Ok(PcmAudio {
        samples,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
    })
}

fn wav_error(path: &Path, error: hound::Error) -> RecognitionError {
    match error {
        hound::Error::IoError(error) => {
            RecognitionError::Io(format!("{}: {error}", path.display()))
        }
        other => RecognitionError::UnsupportedFormat(format!("{}: {other}", path.display())),
    }
}
