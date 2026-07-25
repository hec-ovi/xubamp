//! End-to-end decode tests: real files through the real open -> demux -> decode path.

use std::path::{Path, PathBuf};

use xubamp_audio::decode::{probe_stream_info, probe_tags, Source, TrackTags};

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

/// Wrap MPEG frames in the RIFF/WAVE container some MP3s ship in: `wFormatTag` 0x0055
/// (`WAVE_FORMAT_MPEGLAYER3`), the frames in the `data` chunk, an ID3v2 tag in front of the lot.
fn riff_wrapped_mp3(frames: &[u8], id3: &[u8]) -> Vec<u8> {
    let mut out = id3.to_vec();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0x0055u16.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes()); // channels
    fmt.extend_from_slice(&44100u32.to_le_bytes());
    fmt.extend_from_slice(&16000u32.to_le_bytes()); // average bytes per second
    fmt.extend_from_slice(&1u16.to_le_bytes()); // block align
    fmt.extend_from_slice(&0u16.to_le_bytes()); // bits per sample
    fmt.extend_from_slice(&[0u8; 16]); // MPEGLAYER3 extension

    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    body.extend_from_slice(frames);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

#[test]
fn probes_plays_and_tags_an_mp3_stored_inside_a_riff_wave_container() {
    // Real files in the wild: an ID3v2 tag, then a RIFF/WAVE wrapper whose data chunk holds the
    // MPEG stream, still named .mp3. Symphonia routes them to its WAV reader, which rejects the
    // format outright, so the track showed a blank duration, no metadata, and would not play.
    let fixture: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.mp3");
    let frames = std::fs::read(&fixture).unwrap();
    let tag = id3v2_block("France Galle", "Elle Elle La");
    let path = std::env::temp_dir().join("xubamp_riff_wrapped_test.mp3");
    std::fs::write(&path, riff_wrapped_mp3(&frames, &tag)).unwrap();

    let info = probe_stream_info(&path).expect("stream facts, not a blank file-info box");
    assert_eq!(info.sample_rate, Some(48000));
    assert_eq!(info.channels, Some(2));
    assert!(info.codec.contains("MP3") || info.codec.contains("MPEG"), "{}", info.codec);

    let tags = probe_tags(&path).expect("the leading ID3v2 tag survives the unwrap");
    assert_eq!(tags.artist.as_deref(), Some("France Galle"));
    assert_eq!(tags.title.as_deref(), Some("Elle Elle La"));

    // And it decodes: same audio as the bare file it was built from.
    let mut wrapped_frames = 0u64;
    let mut peak = 0.0f32;
    let mut src = Source::open(&path).unwrap();
    while let Some(s) = src.next_interleaved().unwrap() {
        wrapped_frames += (s.len() / 2) as u64;
        for &x in s {
            peak = peak.max(x.abs());
        }
    }
    assert_eq!(src.sample_rate, 48000);
    assert!(peak > 0.05, "the tone decodes, peak {peak}");

    let mut bare = Source::open(&fixture).unwrap();
    let mut bare_frames = 0u64;
    while let Some(s) = bare.next_interleaved().unwrap() {
        bare_frames += (s.len() / 2) as u64;
    }
    assert_eq!(wrapped_frames, bare_frames, "unwrapping loses no audio");
    std::fs::remove_file(&path).ok();
}

#[test]
fn decodes_an_mp3_whose_silent_part_uses_empty_granules() {
    // `silence.mp3` is 0.1 s of tone then 0.3 s of exact digital silence. The encoder writes the
    // silent stretch as granules with `part2_3_length == 0`, which stock symphonia 0.5.5 rejects;
    // the rejection clears the bit reservoir, so every later frame fails too and the rest of the
    // file never decodes. `next_interleaved` skips undecodable packets silently, so the frame
    // count is what catches it: the tail must still be there.
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/silence.mp3");
    let mut src = Source::open(&path).unwrap();

    let mut frames = 0u64;
    let mut tone_peak = 0.0f32;
    let mut tail_peak = 0.0f32;
    // The tone stops at 0.1 s; leave a frame of slack around the boundary for the encoder's
    // filter ramp before calling anything "tail".
    let tone_end = 44100 / 10;
    let tail_start = tone_end + 1152;
    while let Some(s) = src.next_interleaved().unwrap() {
        for (i, frame) in s.chunks_exact(2).enumerate() {
            let level = frame[0].abs().max(frame[1].abs());
            let at = frames + i as u64;
            if at < tone_end {
                tone_peak = tone_peak.max(level);
            } else if at >= tail_start {
                tail_peak = tail_peak.max(level);
            }
        }
        frames += (s.len() / 2) as u64;
    }

    assert_eq!(src.sample_rate, 44100);
    assert_eq!(src.channels, 2);
    // 0.4 s at 44100. Stock symphonia stops after the last frame before the silence (~5760).
    assert!(
        frames >= 44100 * 4 / 10,
        "the silent tail was dropped: decoded {frames} frames of 17640"
    );
    assert!(tone_peak > 0.05, "the tone should carry energy, {tone_peak}");
    assert!(
        tail_peak < 0.01,
        "the tail should decode as silence, not noise, peak {tail_peak}"
    );
}
