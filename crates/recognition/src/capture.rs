//! Microphone capture through cpal, matching the Android recording parameters
//! (mono PCM16, 12 s max). Linux has no runtime microphone permission model, so
//! a device is either present or not.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Host, SampleFormat, StreamConfig};

use crate::{PcmAudio, RecognitionError};

/// Longest recording the fingerprint pipeline uses (same as the Android app).
pub const MAX_RECORD_SECONDS: f32 = 12.0;

/// One input device exposed by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDevice {
    pub name: String,
    pub is_default: bool,
    pub channels: u16,
    pub sample_rate: u32,
}

/// What to record.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureOptions {
    /// Requested duration; clamped to `0.1..=12.0` s.
    pub seconds: f32,
    /// Device name (exact match first, then case-insensitive substring);
    /// `None` uses the system default input.
    pub device: Option<String>,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            seconds: MAX_RECORD_SECONDS,
            device: None,
        }
    }
}

/// List every input device, with the default one flagged.
pub fn list_input_devices() -> Result<Vec<InputDevice>, RecognitionError> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|device| device.id().ok());
    let devices = host
        .input_devices()
        .map_err(|error| RecognitionError::NoInputDevice(error.to_string()))?;

    let mut listed = Vec::new();
    for device in devices {
        let is_default = match (device.id(), &default_id) {
            (Ok(id), Some(default)) => &id == default,
            _ => false,
        };
        let config = device.default_input_config().ok();
        listed.push(InputDevice {
            name: device.to_string(),
            is_default,
            channels: config.as_ref().map_or(0, |config| config.channels()),
            sample_rate: config.as_ref().map_or(0, |config| config.sample_rate()),
        });
    }
    Ok(listed)
}

/// Record with the default input device for `options.seconds`.
pub fn capture(options: &CaptureOptions) -> Result<PcmAudio, RecognitionError> {
    let stop = AtomicBool::new(false);
    capture_until(options, &stop)
}

/// Record until the duration elapses or `stop` is set.
pub fn capture_until(
    options: &CaptureOptions,
    stop: &AtomicBool,
) -> Result<PcmAudio, RecognitionError> {
    let seconds = options.seconds.clamp(0.1, MAX_RECORD_SECONDS);
    let host = cpal::default_host();
    let device = select_device(&host, options.device.as_deref())?;
    let supported = device
        .default_input_config()
        .map_err(|error| RecognitionError::Capture(format!("default input config: {error}")))?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.config();
    let channels = usize::from(config.channels);
    let sample_rate = config.sample_rate;

    let samples: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::with_capacity(
        sample_rate as usize * seconds as usize + 1024,
    )));
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let callback_samples = Arc::clone(&samples);
    let callback_failure = Arc::clone(&failure);
    let on_error = move |error: cpal::Error| {
        if let Ok(mut slot) = callback_failure.lock() {
            *slot = Some(error.to_string());
        }
    };

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            config,
            move |input: &[f32], _| push_f32(&callback_samples, input, channels),
            on_error,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            config,
            move |input: &[i16], _| push_i16(&callback_samples, input, channels),
            on_error,
            None,
        ),
        other => {
            return Err(RecognitionError::Capture(format!(
                "unsupported sample format {other:?} (need f32 or i16)"
            )))
        }
    }
    .map_err(|error| RecognitionError::Capture(format!("could not open input stream: {error}")))?;

    stream.play().map_err(|error| {
        RecognitionError::Capture(format!("could not start recording: {error}"))
    })?;

    let started = Instant::now();
    let wanted = Duration::from_secs_f32(seconds);
    while started.elapsed() < wanted && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(stream);

    if let Some(message) = failure.lock().ok().and_then(|slot| slot.clone()) {
        return Err(RecognitionError::Capture(message));
    }
    let recorded = Arc::try_unwrap(samples)
        .map(|mutex| mutex.into_inner().unwrap_or_default())
        .unwrap_or_default();
    if recorded.is_empty() {
        return Err(RecognitionError::Capture("no audio captured".to_string()));
    }

    Ok(PcmAudio {
        samples: recorded,
        sample_rate,
        // The callbacks above already mixed every frame to mono, like the Android recorder.
        channels: 1,
    })
}

/// Downmix a callback frame to mono and append it.
fn push_f32(target: &Arc<Mutex<Vec<i16>>>, input: &[f32], channels: usize) {
    let Ok(mut target) = target.lock() else {
        return;
    };
    for frame in input.chunks_exact(channels) {
        let mono = frame.iter().sum::<f32>() / channels as f32;
        target.push((mono.clamp(-1.0, 1.0) * 32_767.0).round() as i16);
    }
}

fn push_i16(target: &Arc<Mutex<Vec<i16>>>, input: &[i16], channels: usize) {
    let Ok(mut target) = target.lock() else {
        return;
    };
    for frame in input.chunks_exact(channels) {
        let mono = frame.iter().map(|&sample| i32::from(sample)).sum::<i32>() / channels as i32;
        target.push(mono as i16);
    }
}

fn select_device(host: &Host, wanted: Option<&str>) -> Result<cpal::Device, RecognitionError> {
    let Some(wanted) = wanted else {
        return host
            .default_input_device()
            .ok_or_else(|| RecognitionError::NoInputDevice("no default input device".to_string()));
    };

    let devices = host
        .input_devices()
        .map_err(|error| RecognitionError::NoInputDevice(error.to_string()))?;
    let needle = wanted.to_lowercase();
    let mut partial = None;
    for device in devices {
        let name = device.to_string();
        if name == wanted {
            return Ok(device);
        }
        if partial.is_none() && name.to_lowercase().contains(&needle) {
            partial = Some(device);
        }
    }
    partial.ok_or_else(|| RecognitionError::NoInputDevice(wanted.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_to_twelve_seconds() {
        let options = CaptureOptions::default();
        assert_eq!(options.seconds, MAX_RECORD_SECONDS);
        assert_eq!(options.device, None);
    }

    #[test]
    fn downmix_adds_frames_to_mono() {
        let target = Arc::new(Mutex::new(Vec::new()));
        push_i16(&target, &[100, 200, -100, -300], 2);
        assert_eq!(*target.lock().unwrap(), vec![150, -200]);
        let target = Arc::new(Mutex::new(Vec::new()));
        push_i16(&target, &[5, 6, 7], 2); // incomplete frame is dropped
        assert_eq!(*target.lock().unwrap(), vec![5]);
    }

    #[test]
    fn float_frames_are_clamped() {
        let target = Arc::new(Mutex::new(Vec::new()));
        push_f32(&target, &[1.5, -1.5], 2); // averaged -> 0.0
        assert_eq!(*target.lock().unwrap(), vec![0]);
        let target = Arc::new(Mutex::new(Vec::new()));
        push_f32(&target, &[0.5], 1);
        assert_eq!(*target.lock().unwrap(), vec![16384]);
    }
}
