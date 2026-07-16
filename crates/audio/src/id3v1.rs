//! ID3v1 / ID3v1.1 tags: the fixed 128-byte "TAG" block at the very end of an MP3 file.
//!
//! This is the tag the classic Winamp file-info box edits. The layout is fixed-width Latin-1:
//! 3 bytes "TAG", 30 title, 30 artist, 30 album, 4 year, 30 comment, 1 genre index. ID3v1.1
//! steals the comment's last two bytes: a zero separator then the track number. Reading and
//! writing are pure byte-level operations on that tail, so they can never corrupt the audio
//! stream in front of it.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// The fixed tag block size, including the "TAG" magic.
pub const TAG_LEN: u64 = 128;

/// The "no genre" byte: 255 means unset/unknown.
pub const GENRE_NONE: u8 = 255;

/// The standard ID3v1 genre names (indices 0..=79). Later indices were Winamp extensions;
/// anything outside this table displays as its raw number.
pub const GENRES: [&str; 80] = [
    "Blues", "Classic Rock", "Country", "Dance", "Disco", "Funk", "Grunge", "Hip-Hop", "Jazz",
    "Metal", "New Age", "Oldies", "Other", "Pop", "R&B", "Rap", "Reggae", "Rock", "Techno",
    "Industrial", "Alternative", "Ska", "Death Metal", "Pranks", "Soundtrack", "Euro-Techno",
    "Ambient", "Trip-Hop", "Vocal", "Jazz+Funk", "Fusion", "Trance", "Classical", "Instrumental",
    "Acid", "House", "Game", "Sound Clip", "Gospel", "Noise", "AlternRock", "Bass", "Soul",
    "Punk", "Space", "Meditative", "Instrumental Pop", "Instrumental Rock", "Ethnic", "Gothic",
    "Darkwave", "Techno-Industrial", "Electronic", "Pop-Folk", "Eurodance", "Dream",
    "Southern Rock", "Comedy", "Cult", "Gangsta", "Top 40", "Christian Rap", "Pop/Funk",
    "Jungle", "Native American", "Cabaret", "New Wave", "Psychadelic", "Rave", "Showtunes",
    "Trailer", "Lo-Fi", "Tribal", "Acid Punk", "Acid Jazz", "Polka", "Retro", "Musical",
    "Rock & Roll", "Hard Rock",
];

/// A decoded (or to-be-written) ID3v1 tag. Strings are already trimmed of the padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Id3v1 {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub comment: String,
    /// Genre index into the classic table; [`GENRE_NONE`] when unset.
    pub genre: u8,
    /// ID3v1.1 track number; `None` writes a plain v1 comment field.
    pub track: Option<u8>,
}

impl Default for Id3v1 {
    fn default() -> Self {
        Self {
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            year: String::new(),
            comment: String::new(),
            // An absent tag has no genre; index 0 would read as "Blues".
            genre: GENRE_NONE,
            track: None,
        }
    }
}

impl Id3v1 {
    /// The genre's display name: the table entry for a known index, empty when unset, the raw
    /// number otherwise.
    pub fn genre_name(&self) -> String {
        match self.genre {
            GENRE_NONE => String::new(),
            g if (g as usize) < GENRES.len() => GENRES[g as usize].to_owned(),
            g => g.to_string(),
        }
    }

    /// The genre index for a user-typed name: empty clears it, a known name (case-insensitive)
    /// maps to its index, a plain number in range is taken as-is, anything else is unset.
    pub fn genre_from_name(name: &str) -> u8 {
        let name = name.trim();
        if name.is_empty() {
            return GENRE_NONE;
        }
        if let Some(index) = GENRES
            .iter()
            .position(|g| g.eq_ignore_ascii_case(name))
        {
            return index as u8;
        }
        match name.parse::<u8>() {
            Ok(n) if n != GENRE_NONE => n,
            _ => GENRE_NONE,
        }
    }
}

/// Read the ID3v1 tag from the end of `path`, `Ok(None)` when the file has none.
pub fn read(path: &Path) -> std::io::Result<Option<Id3v1>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len < TAG_LEN {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-(TAG_LEN as i64)))?;
    let mut block = [0u8; TAG_LEN as usize];
    file.read_exact(&mut block)?;
    Ok(parse(&block))
}

/// Write (replace or append) the ID3v1 tag at the end of `path`.
pub fn write(path: &Path, tag: &Id3v1) -> std::io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let len = file.metadata()?.len();
    let has_tag = if len >= TAG_LEN {
        file.seek(SeekFrom::End(-(TAG_LEN as i64)))?;
        let mut magic = [0u8; 3];
        file.read_exact(&mut magic)?;
        &magic == b"TAG"
    } else {
        false
    };
    if has_tag {
        file.seek(SeekFrom::End(-(TAG_LEN as i64)))?;
    } else {
        file.seek(SeekFrom::End(0))?;
    }
    file.write_all(&encode(tag))?;
    file.flush()
}

/// Decode a 128-byte block, `None` when it does not start with "TAG".
fn parse(block: &[u8; TAG_LEN as usize]) -> Option<Id3v1> {
    if &block[0..3] != b"TAG" {
        return None;
    }
    // ID3v1.1: a zero at comment byte 28 followed by a nonzero byte is the track number.
    let track = (block[125] == 0 && block[126] != 0).then_some(block[126]);
    let comment_len = if track.is_some() { 28 } else { 30 };
    Some(Id3v1 {
        title: field(&block[3..33]),
        artist: field(&block[33..63]),
        album: field(&block[63..93]),
        year: field(&block[93..97]),
        comment: field(&block[97..97 + comment_len]),
        genre: block[127],
        track,
    })
}

/// Encode the fixed 128-byte block.
fn encode(tag: &Id3v1) -> [u8; TAG_LEN as usize] {
    let mut block = [0u8; TAG_LEN as usize];
    block[0..3].copy_from_slice(b"TAG");
    fill(&mut block[3..33], &tag.title);
    fill(&mut block[33..63], &tag.artist);
    fill(&mut block[63..93], &tag.album);
    fill(&mut block[93..97], &tag.year);
    match tag.track {
        Some(track) => {
            fill(&mut block[97..125], &tag.comment);
            block[125] = 0;
            block[126] = track;
        }
        None => fill(&mut block[97..127], &tag.comment),
    }
    block[127] = tag.genre;
    block
}

/// Decode a fixed-width Latin-1 field, trimming NUL padding and surrounding spaces.
fn field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end]
        .iter()
        .map(|&b| b as char)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Encode a string into a fixed-width Latin-1 field, truncating and replacing non-Latin-1
/// characters with '?'.
fn fill(dst: &mut [u8], s: &str) {
    for (slot, ch) in dst.iter_mut().zip(s.chars()) {
        *slot = if (ch as u32) < 256 { ch as u32 as u8 } else { b'?' };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("xubamp-id3v1-{}-{}", std::process::id(), name))
    }

    #[test]
    fn write_appends_once_then_replaces_in_place() {
        let path = temp("roundtrip.mp3");
        std::fs::write(&path, b"not really mpeg audio data").unwrap();
        assert_eq!(read(&path).unwrap(), None, "no tag on a fresh file");

        let tag = Id3v1 {
            title: "Xtal".into(),
            artist: "Aphex Twin".into(),
            album: "Selected Ambient Works 85-92".into(),
            year: "1992".into(),
            comment: "classic".into(),
            genre: 26, // Ambient
            track: Some(1),
        };
        write(&path, &tag).unwrap();
        let after_append = std::fs::metadata(&path).unwrap().len();
        assert_eq!(after_append, 26 + TAG_LEN, "tag appended after the audio");
        assert_eq!(read(&path).unwrap(), Some(tag.clone()));

        // Writing again replaces the block instead of stacking another one.
        let updated = Id3v1 {
            comment: String::new(),
            track: None,
            ..tag
        };
        write(&path, &updated).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), after_append);
        assert_eq!(read(&path).unwrap(), Some(updated));
        // The audio bytes in front are untouched.
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..26], b"not really mpeg audio data");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn long_and_non_latin1_fields_truncate_and_degrade() {
        let path = temp("truncate.mp3");
        std::fs::write(&path, b"x").unwrap();
        let tag = Id3v1 {
            title: "A".repeat(64),
            artist: "Björk — 日本".into(),
            genre: GENRE_NONE,
            ..Default::default()
        };
        write(&path, &tag).unwrap();
        let read_back = read(&path).unwrap().unwrap();
        assert_eq!(read_back.title, "A".repeat(30), "30-byte field truncates");
        assert_eq!(
            read_back.artist, "Björk ? ??",
            "Latin-1 kept, everything else becomes '?'"
        );
        assert_eq!(read_back.track, None);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn genre_names_round_trip_and_reject_junk() {
        let ambient = Id3v1 {
            genre: 26,
            ..Default::default()
        };
        assert_eq!(ambient.genre_name(), "Ambient");
        assert_eq!(Id3v1::genre_from_name("ambient"), 26);
        assert_eq!(Id3v1::genre_from_name(""), GENRE_NONE);
        assert_eq!(Id3v1::genre_from_name("Ambient Jazz Fusion X"), GENRE_NONE);
        assert_eq!(Id3v1::genre_from_name("42"), 42);
        let unknown = Id3v1 {
            genre: 200,
            ..Default::default()
        };
        assert_eq!(unknown.genre_name(), "200", "out-of-table shows the number");
        let unset = Id3v1::default();
        assert_eq!(
            Id3v1 {
                genre: GENRE_NONE,
                ..unset
            }
            .genre_name(),
            ""
        );
    }
}
