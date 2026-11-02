//! Lavfi filter chain used for the 10-band EQ and normalization gain.
//!
//! The chain is applied through mpv's `af` property as a single `lavfi=[…]`
//! graph, which keeps commas inside the graph from being split into separate
//! mpv audio filters (verified against mpv 0.41: `get_property af` echoes the
//! graph back and playback continues).

/// Graphic-EQ band centers in Hz, matching the Android 10-band UI.
pub const EQ_BANDS_HZ: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// One-octave peaking width (Q = sqrt(2)), as used by Android's default bands.
const BAND_Q: f32 = 1.41;

/// Gains below this are inaudible and omitted so an unmodified EQ leaves the
/// audio path untouched.
const MIN_GAIN_DB: f32 = 0.05;

/// Mid-stream silence longer than `stop_duration` and below `stop_threshold`
/// is dropped, keeping 50 ms so cuts do not click. `stop_periods=-1` trims all
/// silence after the first non-silent sample (leading/trailing silence is left
/// alone). Measured on mpv 0.41: a 7 s tone/silence/tone file finishes in
/// 4.05 s with this filter.
const SILENCE_REMOVE: &str =
    "silenceremove=stop_periods=-1:stop_duration=0.5:stop_threshold=-50dB:stop_silence=0.05:detection=peak";

/// Build the `af` value for `eq_bands` (dB, zipped with [`EQ_BANDS_HZ`]), an
/// optional replay-gain correction in dB and the skip-silence filter. Returns
/// `None` when nothing needs to be applied (the caller clears `af`).
pub fn build_af(eq_bands: &[f32], gain_db: f32, skip_silence: bool) -> Option<String> {
    let mut filters = Vec::new();
    if skip_silence {
        filters.push(SILENCE_REMOVE.to_string());
    }
    if gain_db.is_finite() && gain_db.abs() >= MIN_GAIN_DB {
        filters.push(format!("volume={gain_db}dB"));
    }
    for (hz, gain) in EQ_BANDS_HZ.iter().zip(eq_bands) {
        if gain.is_finite() && gain.abs() >= MIN_GAIN_DB {
            filters.push(format!("equalizer=f={hz}:t=q:w={BAND_Q}:g={gain}"));
        }
    }
    if filters.is_empty() {
        None
    } else {
        Some(format!("lavfi=[{}]", filters.join(",")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_eq_and_zero_gain_is_none() {
        assert_eq!(build_af(&[0.0; 10], 0.0, false), None);
        assert_eq!(build_af(&[], 0.0, false), None);
    }

    #[test]
    fn builds_equalizer_chain_in_band_order() {
        let bands = [3.0, 0.0, 0.0, 0.0, 0.0, -4.0, 0.0, 0.0, 0.0, 2.0];
        assert_eq!(
            build_af(&bands, 0.0, false).unwrap(),
            "lavfi=[equalizer=f=31:t=q:w=1.41:g=3,\
             equalizer=f=1000:t=q:w=1.41:g=-4,\
             equalizer=f=16000:t=q:w=1.41:g=2]"
        );
    }

    #[test]
    fn gain_prefixes_the_chain() {
        assert_eq!(
            build_af(&[0.0; 10], -11.0, false).unwrap(),
            "lavfi=[volume=-11dB]"
        );
        assert_eq!(
            build_af(
                &[1.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                3.0,
                false
            )
            .unwrap(),
            "lavfi=[volume=3dB,equalizer=f=31:t=q:w=1.41:g=1.5]"
        );
    }

    #[test]
    fn non_finite_and_tiny_gains_are_ignored() {
        assert_eq!(build_af(&[f32::NAN, f32::INFINITY, 0.01], 0.0, false), None);
        assert_eq!(
            build_af(&[1.0, 2.0], 0.0, false)
                .unwrap()
                .matches("equalizer=")
                .count(),
            2
        );
    }

    #[test]
    fn extra_bands_are_ignored() {
        let mut bands = vec![0.0; 14];
        bands[9] = 4.0;
        assert_eq!(
            build_af(&bands, 0.0, false).unwrap(),
            "lavfi=[equalizer=f=16000:t=q:w=1.41:g=4]"
        );
    }

    #[test]
    fn skip_silence_leads_the_chain() {
        assert_eq!(
            build_af(&[0.0; 10], 0.0, true).unwrap(),
            format!("lavfi=[{SILENCE_REMOVE}]")
        );
        assert_eq!(
            build_af(&[0.0; 10], -3.0, true).unwrap(),
            format!("lavfi=[{SILENCE_REMOVE},volume=-3dB]")
        );
    }
}
