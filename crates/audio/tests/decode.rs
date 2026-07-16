//! End-to-end decode tests: real files through the real open -> demux -> decode path.

use std::path::{Path, PathBuf};

use xubamp_audio::decode::{probe_tags, Source, TrackTags};

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

/// An ID3v2.3 tag block carrying a TPE1 (artist) and TIT2 (title) text frame, byte-exact per the
/// spec: a 10-byte header with a syncsafe size, then plain big-endian-sized frames whose text
/// payloads start with a 0x00 (Latin-1) encoding byte.
fn id3v2_block(artist: &str, title: &str) -> Vec<u8> {
    fn frame(id: &[u8; 4], text: &str) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(id);
        f.extend_from_slice(&(text.len() as u32 + 1).to_be_bytes());
        f.extend_from_slice(&[0, 0]); // frame flags
        f.push(0); // Latin-1 text encoding
        f.extend_from_slice(text.as_bytes());
        f
    }
    let mut body = frame(b"TPE1", artist);
    body.extend_from_slice(&frame(b"TIT2", title));
    let mut tag = Vec::new();
    tag.extend_from_slice(b"ID3");
    tag.extend_from_slice(&[3, 0, 0]); // v2.3, no flags
    let size = body.len() as u32;
    // Syncsafe: 7 bits per byte, high bit clear.
    tag.extend_from_slice(&[
        ((size >> 21) & 0x7f) as u8,
        ((size >> 14) & 0x7f) as u8,
        ((size >> 7) & 0x7f) as u8,
        (size & 0x7f) as u8,
    ]);
    tag.extend_from_slice(&body);
    tag
}

#[test]
fn probes_id3v2_tags_on_an_mp3() {
    let fixture: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.mp3");
    let mp3 = std::fs::read(&fixture).unwrap();
    let mut tagged = id3v2_block("Aphex Twin", "Xtal");
    tagged.extend_from_slice(&mp3);
    let path = std::env::temp_dir().join("xubamp_probe_tags_test.mp3");
    std::fs::write(&path, tagged).unwrap();

    let tags = probe_tags(&path).expect("tagged MP3 probes");
    assert_eq!(tags.artist.as_deref(), Some("Aphex Twin"));
    assert_eq!(tags.title.as_deref(), Some("Xtal"));
    assert_eq!(tags.display_name().as_deref(), Some("Aphex Twin - Xtal"));
    std::fs::remove_file(&path).ok();
}

#[test]
fn probes_riff_info_tags_on_a_wav_and_reads_empty_tags_as_none() {
    // A WAV with a LIST INFO chunk (IART artist + INAM title) appended after the data chunk.
    let plain = std::env::temp_dir().join("xubamp_probe_tags_plain.wav");
    write_wav_s16_stereo(&plain, 48000, 480);
    let tags = probe_tags(&plain).expect("a plain WAV still probes");
    assert_eq!(tags, TrackTags::default(), "no tags reads as empty");
    assert_eq!(tags.display_name(), None, "empty tags fall back to the name");

    fn info_entry(id: &[u8; 4], text: &str) -> Vec<u8> {
        let mut z = text.as_bytes().to_vec();
        z.push(0); // NUL terminator
        if z.len() % 2 == 1 {
            z.push(0); // RIFF chunks are word-aligned
        }
        let mut e = id.to_vec();
        e.extend_from_slice(&(z.len() as u32).to_le_bytes());
        e.extend_from_slice(&z);
        e
    }
    let mut wav = std::fs::read(&plain).unwrap();
    let mut list = b"INFO".to_vec();
    list.extend_from_slice(&info_entry(b"IART", "Boards of Canada"));
    list.extend_from_slice(&info_entry(b"INAM", "Roygbiv"));
    let mut chunk = b"LIST".to_vec();
    chunk.extend_from_slice(&(list.len() as u32).to_le_bytes());
    chunk.extend_from_slice(&list);
    // The reader collects INFO while walking chunks toward `data`, so the LIST goes between the
    // fmt chunk (ends at byte 36 of this fixed-layout file) and the data chunk.
    wav.splice(36..36, chunk);
    let riff_len = (wav.len() - 8) as u32;
    wav[4..8].copy_from_slice(&riff_len.to_le_bytes());
    let tagged = std::env::temp_dir().join("xubamp_probe_tags_info.wav");
    std::fs::write(&tagged, wav).unwrap();

    let tags = probe_tags(&tagged).expect("tagged WAV probes");
    assert_eq!(tags.artist.as_deref(), Some("Boards of Canada"));
    assert_eq!(tags.title.as_deref(), Some("Roygbiv"));
    assert_eq!(
        tags.display_name().as_deref(),
        Some("Boards of Canada - Roygbiv")
    );
    std::fs::remove_file(&plain).ok();
    std::fs::remove_file(&tagged).ok();
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
