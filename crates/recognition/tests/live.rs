//! Live end-to-end recognition against the real Shazam API.
//!
//! Ignored by default because it needs the network and a known track:
//!
//! ```text
//! ffmpeg -ss 40 -t 12 -i song.mp3 -ac 1 -ar 44100 -sample_fmt s16 /tmp/segment.wav
//! ML_RECOGNITION_WAV=/tmp/segment.wav cargo test -p ml-recognition --test live -- --ignored --nocapture
//! ```

use ml_recognition::{read_wav, signature_uri_from_pcm16le, ShazamClient, SAMPLE_RATE};

#[test]
#[ignore = "hits the live Shazam API; set ML_RECOGNITION_WAV first"]
fn recognizes_a_twelve_second_wav_segment() {
    let path =
        std::env::var("ML_RECOGNITION_WAV").expect("set ML_RECOGNITION_WAV to a 12 s WAV segment");
    let audio = read_wav(&path).expect("read WAV");
    let pcm = audio.to_16k_mono_pcm16();
    let signature = signature_uri_from_pcm16le(&pcm).expect("fingerprint");
    let sample_duration_ms = (pcm.len() as u64 / 2) * 1000 / u64::from(SAMPLE_RATE);

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = runtime
        .block_on(ShazamClient::new().recognize(&signature, sample_duration_ms))
        .expect("live recognition");

    println!("{result:?}");
    assert!(!result.title.is_empty(), "matched track has no title");
    assert!(!result.artist.is_empty(), "matched track has no artist");
}
