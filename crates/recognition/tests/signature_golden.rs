//! Byte-for-byte parity with the upstream Kotlin fingerprint generator.
//!
//! The `.sig` fixtures were produced by compiling
//! `app/src/main/kotlin/com/metrolist/music/recognition/ShazamSignatureGenerator.kt`
//! with kotlinc (Android/Timber stubs for `Base64` and `Timber`) and running it
//! on the exact integer PCM these helpers regenerate, so a mismatch here means
//! the Rust port drifted from what the Android app sends to Shazam.

use ml_recognition::{signature_bytes, signature_uri};

/// Numerical Recipes LCG, matching the integer generator used for the fixtures.
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

fn noise(len: usize, seed: u32) -> Vec<i16> {
    let mut rng = Lcg::new(seed);
    (0..len).map(|_| (rng.next() >> 16) as u16 as i16).collect()
}

fn square(index: usize, period: usize) -> i32 {
    if (index / period).is_multiple_of(2) {
        1
    } else {
        -1
    }
}

fn mix(len: usize, seed: u32) -> Vec<i16> {
    let mut rng = Lcg::new(seed);
    (0..len)
        .map(|index| {
            let raw = (rng.next() >> 16) as i32 - 32_768;
            let value = square(index, 29) * 7000
                + square(index, 73) * 5000
                + square(index, 181) * 3000
                + raw.div_euclid(8);
            value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
        })
        .collect()
}

fn chirp(len: usize) -> Vec<i16> {
    let mut phase: i32 = 0;
    let mut step: i32 = 20;
    (0..len)
        .map(|index| {
            phase = (phase + step) & 0xFFFF;
            step = 20 + 380 * index as i32 / len as i32;
            if phase & 0x8000 == 0 {
                12_000
            } else {
                -12_000
            }
        })
        .collect()
}

fn full_scale(len: usize) -> Vec<i16> {
    (0..len)
        .map(|index| {
            if (index / 16) % 2 == 0 {
                i16::MAX
            } else {
                i16::MIN
            }
        })
        .collect()
}

fn assert_case(name: &str, samples: Vec<i16>, expected: &[u8]) {
    let actual = signature_bytes(&samples);
    assert_eq!(
        actual.as_slice(),
        expected,
        "{name}: signature bytes differ from the Kotlin reference"
    );
}

#[test]
fn signatures_match_the_kotlin_reference() {
    assert_case(
        "chirp12s",
        chirp(192_000),
        include_bytes!("fixtures/chirp12s.sig"),
    );
    assert_case(
        "fullscale12s",
        full_scale(192_000),
        include_bytes!("fixtures/fullscale12s.sig"),
    );
    assert_case(
        "mix12s",
        mix(192_000, 0x5EED_1234),
        include_bytes!("fixtures/mix12s.sig"),
    );
    assert_case(
        "noise12s",
        noise(192_000, 0x1234_5678),
        include_bytes!("fixtures/noise12s.sig"),
    );
    assert_case(
        "noise13s",
        noise(208_000, 0x2468_ACE0),
        include_bytes!("fixtures/noise13s.sig"),
    );
    assert_case(
        "noise3s",
        noise(48_000, 0x0BAD_F00D),
        include_bytes!("fixtures/noise3s.sig"),
    );
    assert_case(
        "noise_short07s",
        noise(11_200, 0xDEAD_BEEF),
        include_bytes!("fixtures/noise_short07s.sig"),
    );
    // The fixture PCM files hold bytes: 12 800 B / 1 600 B are 6400 / 800 samples.
    assert_case(
        "silence04s",
        vec![0; 6_400],
        include_bytes!("fixtures/silence04s.sig"),
    );
    assert_case(
        "tiny01s",
        vec![0; 800],
        include_bytes!("fixtures/tiny01s.sig"),
    );
}

#[test]
fn short_silence_uri_matches_the_kotlin_uri() {
    let samples = vec![0i16; 6_400];
    assert_eq!(
        signature_uri(&samples),
        include_str!("fixtures/silence04s.uri")
    );
}

#[test]
fn audio_beyond_twelve_seconds_does_not_change_the_signature() {
    let long = noise(208_000, 0x2468_ACE0);
    let truncated = &long[..192_000];
    assert_eq!(signature_bytes(&long), signature_bytes(truncated));
}
