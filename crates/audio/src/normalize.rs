//! Loudness normalization gain, ported from the Android client.
//!
//! Android's `normalizationGainMb` uses YouTube's per-track `loudnessDb` /
//! `perceptualLoudnessDb` from the player response; it never measures the
//! stream locally. The port's `crates/innertube` player parser does not expose
//! those fields yet, so the caller supplies the values (or a manual gain) and
//! [`normalization_gain_db`] only performs the parity arithmetic.
//!
//! Target loudness mirrors the Android `LoudnessLevel` options:
//! `AGGRESSIVE -7`, `LOUD -11`, `BALANCED -14`, `QUIET -19` LUFS.
//!
//! Formula (MusicService.kt:5071):
//!
//! ```text
//! measured = perceptualLoudnessDb ?? loudnessDb + AGGRESSIVE.targetLufs
//! gain_mb  = (int)(-(measured - targetLufs) * 100)
//! gain_mb  = clamp(gain_mb, -1500, 300)      // -15 dB .. +3 dB
//! ```

/// Gain range clamp, in millibels (Android `coerceIn(-1500, 300)`).
const MIN_GAIN_MB: i32 = -1500;
const MAX_GAIN_MB: i32 = 300;

/// Android's `LoudnessLevel.AGGRESSIVE.targetLufs`, added to the relative
/// `loudnessDb` so it can be compared with an absolute LUFS target.
const LOUDNESS_DB_OFFSET: f32 = -7.0;

/// Correction in dB for a track with the given loudness metadata, or `None`
/// when the caller has no loudness value (normalization must then stay off).
///
/// `loudness_db` is YouTube's relative `loudnessDb`; `perceptual_loudness_db`
/// is the absolute `perceptualLoudnessDb` that takes precedence. The result is
/// clamped to `-15.0..=3.0` dB and truncated to centibels, matching Android.
pub fn normalization_gain_db(
    loudness_db: Option<f32>,
    perceptual_loudness_db: Option<f32>,
    target_lufs: f32,
) -> Option<f32> {
    if !target_lufs.is_finite() {
        return None;
    }
    let measured = perceptual_loudness_db
        .filter(|value| value.is_finite())
        .or_else(|| {
            loudness_db
                .filter(|value| value.is_finite())
                .map(|value| value + LOUDNESS_DB_OFFSET)
        })?;
    let gain_mb = (-(measured - target_lufs) * 100.0) as i32;
    Some(gain_mb.clamp(MIN_GAIN_MB, MAX_GAIN_MB) as f32 / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lufs(value: f32) -> Option<f32> {
        Some(value)
    }

    #[test]
    fn relative_loudness_uses_the_aggressive_offset() {
        // loudness -5 + (-7) = -12 LUFS; target -14 -> -2 dB.
        assert_eq!(normalization_gain_db(lufs(-5.0), None, -14.0), Some(-2.0));
        // loudness 0 + (-7) = -7 LUFS; target -14 -> -7 dB.
        assert_eq!(normalization_gain_db(lufs(0.0), None, -14.0), Some(-7.0));
    }

    #[test]
    fn perceptual_loudness_takes_precedence() {
        // Perceptual -10 LUFS wins over relative -5: -10 vs target -14 -> -4 dB.
        assert_eq!(
            normalization_gain_db(lufs(-5.0), lufs(-10.0), -14.0),
            Some(-4.0)
        );
    }

    #[test]
    fn clamp_matches_android_millibel_limits() {
        assert_eq!(normalization_gain_db(lufs(30.0), None, -14.0), Some(-15.0));
        assert_eq!(normalization_gain_db(lufs(-40.0), None, -14.0), Some(3.0));
    }

    #[test]
    fn missing_or_non_finite_metadata_yields_none() {
        assert_eq!(normalization_gain_db(None, None, -14.0), None);
        assert_eq!(
            normalization_gain_db(lufs(f32::NAN), lufs(f32::INFINITY), -14.0),
            None
        );
        assert_eq!(normalization_gain_db(lufs(-5.0), None, f32::NAN), None);
    }

    #[test]
    fn centibel_truncation_is_toward_zero() {
        // loudness -4.995 + (-7) = -11.995; target -14 -> -2.005 dB -> -2.00 dB.
        assert_eq!(normalization_gain_db(lufs(-4.995), None, -14.0), Some(-2.0));
    }
}
