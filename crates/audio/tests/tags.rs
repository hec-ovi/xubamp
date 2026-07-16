//! Header-only tag probing against real files on disk: the ID3v1 fallback that names the many
//! older files (and this player's own tag editor output) symphonia's probe cannot see.

use std::path::Path;

use xubamp_audio::decode::probe_tags;
use xubamp_audio::id3v1::{self, Id3v1};

/// A minimal silent PCM WAV, enough for the probe to accept the file.
fn write_wav(path: &Path) {
    let rate: u32 = 8_000;
    let frames = rate / 10;
    let data_len = frames * 2;
    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&rate.to_le_bytes());
    buf.extend_from_slice(&(rate * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    buf.resize(44 + data_len as usize, 0);
    std::fs::write(path, buf).unwrap();
}

#[test]
fn probe_tags_falls_back_to_the_id3v1_block() {
    let dir = std::env::temp_dir().join(format!("xubamp-tagprobe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("v1only.wav");
    write_wav(&path);

    // Untagged: nothing to report.
    let tags = probe_tags(&path).expect("probe accepts the wav");
    assert_eq!(tags.title, None);
    assert_eq!(tags.artist, None);

    // With a trailing ID3v1 block (what the file-info editor writes): the fallback surfaces it.
    id3v1::write(
        &path,
        &Id3v1 {
            title: "Xtal".to_owned(),
            artist: "Aphex Twin".to_owned(),
            album: "SAW 85-92".to_owned(),
            ..Id3v1::default()
        },
    )
    .unwrap();
    let tags = probe_tags(&path).expect("probe still accepts the tagged wav");
    assert_eq!(tags.title.as_deref(), Some("Xtal"));
    assert_eq!(tags.artist.as_deref(), Some("Aphex Twin"));
    assert!(
        tags.all_text.contains("SAW 85-92"),
        "v1 album lands in the search text"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
