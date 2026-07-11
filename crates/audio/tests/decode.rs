//! End-to-end decode tests: real files through the real open -> demux -> decode path.

use std::path::{Path, PathBuf};

use xubamp_audio::decode::Source;

/// Write a 16-bit PCM stereo WAV of a 440 Hz sine (dependency-free RIFF).
fn write_wav_s16_stereo(path: &Path, rate: u32, frames: u32) {
    let block_align: u16 = 4; // 2 channels * 2 bytes
    let byte_rate = rate * block_align as u32;
    let data_len = frames * block_align as u32;

    let mut d = Vec::with_capacity(44 + data_len as usize);
    d.extend_from_slice(b"RIFF");
    d.extend_from_slice(&(36 + data_len).to_le_bytes());
    d.extend_from_slice(b"WAVE");
    d.extend_from_slice(b"fmt ");
    d.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    d.extend_from_slice(&1u16.to_le_bytes()); // PCM
    d.extend_from_slice(&2u16.to_le_bytes()); // channels
    d.extend_from_slice(&rate.to_le_bytes());
    d.extend_from_slice(&byte_rate.to_le_bytes());
    d.extend_from_slice(&block_align.to_le_bytes());
    d.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    d.extend_from_slice(b"data");
    d.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let t = i as f32 / rate as f32;
        let v = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
        let s = (v * 30000.0) as i16;
        d.extend_from_slice(&s.to_le_bytes());
        d.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, d).unwrap();
}

#[test]
fn decodes_generated_wav() {
    let path = std::env::temp_dir().join("xubamp_decode_wav_test.wav");
    write_wav_s16_stereo(&path, 48000, 4800); // 0.1 s
    let mut src = Source::open(&path).unwrap();

    let mut frames = 0u64;
    let mut first: Option<f32> = None;
    let mut peak = 0.0f32;
    while let Some(s) = src.next_interleaved().unwrap() {
        assert_eq!(s.len() % 2, 0, "interleaved stereo");
        if first.is_none() && !s.is_empty() {
            first = Some(s[0]);
            assert_eq!(s[0], s[1], "L and R identical for a mono-source sine");
        }
        frames += (s.len() / 2) as u64;
        for &x in s {
            peak = peak.max(x.abs());
        }
    }

    assert_eq!(src.sample_rate, 48000);
    assert_eq!(src.channels, 2);
    assert_eq!(frames, 4800);
    assert!(first.unwrap().abs() < 0.02, "a sine starts near zero");
    assert!(peak > 0.5, "real signal present, peak {peak}");
    std::fs::remove_file(&path).ok();
}

#[test]
fn decodes_mp3_fixture() {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.mp3");
    let mut src = Source::open(&path).unwrap();

    let mut frames = 0u64;
    let mut peak = 0.0f32;
    while let Some(s) = src.next_interleaved().unwrap() {
        frames += (s.len() / 2) as u64;
        for &x in s {
            peak = peak.max(x.abs());
        }
    }

    assert_eq!(src.channels, 2);
    assert!(
        src.sample_rate == 48000 || src.sample_rate == 44100,
        "unexpected rate {}",
        src.sample_rate
    );
    assert!(frames > 1000, "decoded only {frames} frames");
    assert!(peak > 0.05, "MP3 tone should carry energy, peak {peak}");
}
