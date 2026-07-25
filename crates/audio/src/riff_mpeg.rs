//! Unwrapping MPEG audio stored inside a RIFF/WAVE container.
//!
//! Some MP3s in the wild are not bare MPEG streams: the frames sit in the `data` chunk of a
//! RIFF/WAVE file whose `fmt ` chunk declares `wFormatTag` 0x0055 (`WAVE_FORMAT_MPEGLAYER3`) or
//! 0x0050 (`WAVE_FORMAT_MPEG`, layers I and II). Encoders of the late nineties produced them and
//! they usually still carry the `.mp3` extension, so nothing about the file announces the wrapper.
//! Symphonia routes them to its WAV reader, which only understands PCM-shaped formats and gives up
//! with "wav: unsupported wave format", so the track will not probe, tag, or play at all.
//!
//! The fix is to hand the decoder the bytes it can read and nothing else: the leading ID3v2 tag,
//! if the file has one, followed by the payload of the `data` chunk. Splicing those two ranges
//! yields exactly the bare MP3 the file would have been without the wrapper, tags included.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

/// `WAVE_FORMAT_MPEG`: MPEG-1 layers I and II.
const WAVE_FORMAT_MPEG: u16 = 0x0050;
/// `WAVE_FORMAT_MPEGLAYER3`: MPEG-1 layer III, the common case.
const WAVE_FORMAT_MPEGLAYER3: u16 = 0x0055;

/// A RIFF chunk header plus the minimum `fmt ` body we read (`wFormatTag`).
const CHUNK_HEADER: u64 = 8;

/// Chunks are scanned only this far into the file. A `fmt ` chunk lives within the first few
/// hundred bytes of the RIFF body in every real file; the bound keeps a corrupt size field from
/// walking the whole file.
const MAX_CHUNK_SCAN: u64 = 64 * 1024;

/// A read-only view over an ordered list of byte ranges of one file, presented as a single
/// contiguous stream. Seekable and of known length, so Symphonia treats it exactly like a plain
/// file: the demuxer can seek, and the engine's seek bar keeps working.
pub struct SplicedFile {
    file: File,
    /// `(offset in the file, length)`, in stream order. Non-overlapping and non-empty.
    ranges: Vec<(u64, u64)>,
    len: u64,
    pos: u64,
}

impl SplicedFile {
    /// `ranges` are taken in the order given; empty ranges are dropped.
    fn new(file: File, ranges: Vec<(u64, u64)>) -> Self {
        let ranges: Vec<(u64, u64)> = ranges.into_iter().filter(|&(_, len)| len > 0).collect();
        let len = ranges.iter().map(|&(_, len)| len).sum();
        Self {
            file,
            ranges,
            len,
            pos: 0,
        }
    }

    /// The file offset the current stream position maps to, and how many bytes remain in the range
    /// it falls in. `None` at or past the end of the stream.
    fn locate(&self) -> Option<(u64, u64)> {
        let mut remaining = self.pos;
        for &(start, len) in &self.ranges {
            if remaining < len {
                return Some((start + remaining, len - remaining));
            }
            remaining -= len;
        }
        None
    }
}

impl Read for SplicedFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some((offset, left_in_range)) = self.locate() else {
            return Ok(0); // end of stream
        };
        let want = buf.len().min(left_in_range.min(usize::MAX as u64) as usize);
        if want == 0 {
            return Ok(0);
        }
        self.file.seek(SeekFrom::Start(offset))?;
        let read = self.file.read(&mut buf[..want])?;
        self.pos += read as u64;
        Ok(read)
    }
}

impl Seek for SplicedFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => self.len as i64 + n,
            SeekFrom::Current(n) => self.pos as i64 + n,
        };
        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the stream",
            ));
        }
        // Seeking past the end is legal and reads return 0 there, matching a plain file.
        self.pos = target as u64;
        Ok(self.pos)
    }
}

impl MediaSource for SplicedFile {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

/// Length of the ID3v2 tag at the start of `file`, including its 10-byte header, or 0 when there
/// is none. The size field is syncsafe: seven bits per byte.
fn id3v2_len(file: &mut File) -> io::Result<u64> {
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; 10];
    if file.read_exact(&mut header).is_err() || &header[..3] != b"ID3" {
        return Ok(0);
    }
    let size = header[6..10]
        .iter()
        .fold(0u64, |acc, &byte| (acc << 7) | u64::from(byte & 0x7f));
    Ok(10 + size)
}

/// If `file` is a RIFF/WAVE container holding an MPEG audio stream, the ranges that make it up: the
/// leading ID3v2 tag (when present) followed by the `data` chunk payload. `None` for everything
/// else, including ordinary PCM WAVs and bare MP3s, which go to Symphonia's probe untouched.
pub fn mpeg_ranges(file: &mut File) -> io::Result<Option<Vec<(u64, u64)>>> {
    let file_len = file.metadata()?.len();
    let tag_len = id3v2_len(file)?;

    let mut riff = [0u8; 12];
    file.seek(SeekFrom::Start(tag_len))?;
    if file.read_exact(&mut riff).is_err() || &riff[..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Ok(None);
    }

    // Walk the chunk list for `fmt ` (to learn the codec) and `data` (the payload). The two can
    // appear in either order in principle, but `fmt ` always precedes `data` in practice, so a
    // single pass that stops at `data` is enough.
    let mut mpeg = false;
    let mut cursor = tag_len + 12;
    let scan_end = file_len.min(tag_len + 12 + MAX_CHUNK_SCAN);
    while cursor + CHUNK_HEADER <= scan_end {
        let mut header = [0u8; 8];
        file.seek(SeekFrom::Start(cursor))?;
        if file.read_exact(&mut header).is_err() {
            return Ok(None);
        }
        let id = &header[..4];
        let size = u64::from(u32::from_le_bytes([
            header[4], header[5], header[6], header[7],
        ]));
        let body = cursor + CHUNK_HEADER;
        match id {
            b"fmt " if size >= 2 => {
                let mut tag = [0u8; 2];
                if file.read_exact(&mut tag).is_err() {
                    return Ok(None);
                }
                mpeg = matches!(
                    u16::from_le_bytes(tag),
                    WAVE_FORMAT_MPEG | WAVE_FORMAT_MPEGLAYER3
                );
                if !mpeg {
                    return Ok(None); // an ordinary WAV: Symphonia handles it
                }
            }
            b"data" => {
                if !mpeg {
                    return Ok(None);
                }
                // A truncated file declares more than it holds; clamp so the stream ends with the
                // file rather than reading short forever.
                let len = size.min(file_len.saturating_sub(body));
                let mut ranges = Vec::with_capacity(2);
                if tag_len > 0 {
                    ranges.push((0, tag_len));
                }
                ranges.push((body, len));
                return Ok(Some(ranges));
            }
            _ => {}
        }
        // Chunks are word aligned: an odd size is followed by a pad byte.
        cursor = body + size + (size & 1);
    }
    Ok(None)
}

/// Open `path`, returning a media source over just its MPEG stream when the file turns out to be a
/// RIFF/WAVE wrapper, and `None` when it is anything else (the caller opens it normally).
pub fn open(path: &std::path::Path) -> io::Result<Option<SplicedFile>> {
    let mut file = File::open(path)?;
    let Some(ranges) = mpeg_ranges(&mut file)? else {
        return Ok(None);
    };
    Ok(Some(SplicedFile::new(file, ranges)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Wrap `payload` in a RIFF/WAVE container declaring `format_tag`, optionally behind an ID3v2
    /// tag of `tag_body` bytes, exactly as the real files are laid out.
    pub(crate) fn wrap(payload: &[u8], format_tag: u16, id3: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(body) = id3 {
            out.extend_from_slice(b"ID3");
            out.extend_from_slice(&[3, 0, 0]);
            let size = body.len() as u32;
            out.extend_from_slice(&[
                ((size >> 21) & 0x7f) as u8,
                ((size >> 14) & 0x7f) as u8,
                ((size >> 7) & 0x7f) as u8,
                (size & 0x7f) as u8,
            ]);
            out.extend_from_slice(body);
        }
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&format_tag.to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes()); // channels
        fmt.extend_from_slice(&44100u32.to_le_bytes());
        fmt.extend_from_slice(&16000u32.to_le_bytes()); // avg bytes per second
        fmt.extend_from_slice(&1u16.to_le_bytes()); // block align
        fmt.extend_from_slice(&0u16.to_le_bytes()); // bits per sample
        fmt.extend_from_slice(&[0u8; 16]); // the MPEGLAYER3 extension block

        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        body.extend_from_slice(&fmt);
        body.extend_from_slice(b"fact");
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(payload);

        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        path
    }

    #[test]
    fn a_wrapped_mpeg_stream_reads_back_as_the_bare_file() {
        let payload: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let tag = b"TAGBODY".repeat(3);
        let path = temp("xubamp_riff_mpeg.mp3", &wrap(&payload, 0x0055, Some(&tag)));

        let mut spliced = open(&path).unwrap().expect("a wrapped MPEG file is recognised");
        // The stream is the ID3v2 tag (10 byte header + body) followed by the payload, and nothing
        // of the RIFF container in between.
        assert_eq!(spliced.byte_len(), Some((10 + tag.len() + payload.len()) as u64));
        let mut all = Vec::new();
        spliced.read_to_end(&mut all).unwrap();
        assert_eq!(&all[..3], b"ID3");
        assert_eq!(&all[10..10 + tag.len()], &tag[..]);
        assert_eq!(&all[10 + tag.len()..], &payload[..]);

        // Seeking lands in the right range on both sides of the splice, which is what lets the
        // demuxer scrub.
        let payload_start = (10 + tag.len()) as u64;
        spliced.seek(SeekFrom::Start(10)).unwrap();
        let mut head = [0u8; 4];
        spliced.read_exact(&mut head).unwrap();
        assert_eq!(&head, &tag[..4], "seek into the tag body");
        spliced.seek(SeekFrom::Start(payload_start + 7)).unwrap();
        let mut one = [0u8; 1];
        spliced.read_exact(&mut one).unwrap();
        assert_eq!(one[0], payload[7], "seek into the payload");

        // A read spanning the boundary stitches the two ranges. Each `read` stops at the end of a
        // range, so loop like any caller of `Read` must.
        spliced.seek(SeekFrom::Start(payload_start - 2)).unwrap();
        let mut across = [0u8; 4];
        let mut got = 0;
        while got < across.len() {
            let n = spliced.read(&mut across[got..]).unwrap();
            assert!(n > 0, "short read before the end of the stream");
            got += n;
        }
        assert_eq!(&across[..2], &tag[tag.len() - 2..]);
        assert_eq!(&across[2..], &payload[..2]);

        // Past the end reads nothing rather than erroring, like a plain file.
        spliced.seek(SeekFrom::End(0)).unwrap();
        assert_eq!(spliced.read(&mut [0u8; 8]).unwrap(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_wrapped_stream_without_an_id3_tag_is_just_the_payload() {
        let payload: Vec<u8> = (0..200u8).collect();
        let path = temp("xubamp_riff_mpeg_notag.mp3", &wrap(&payload, 0x0050, None));
        let mut spliced = open(&path).unwrap().expect("layer II wrapper is recognised too");
        let mut all = Vec::new();
        spliced.read_to_end(&mut all).unwrap();
        assert_eq!(all, payload);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ordinary_files_are_left_to_symphonia() {
        // A PCM WAV: Symphonia reads these natively, so unwrapping would only get in the way.
        let pcm = temp("xubamp_riff_pcm.wav", &wrap(&[0u8; 64], 0x0001, None));
        assert!(open(&pcm).unwrap().is_none());
        std::fs::remove_file(&pcm).ok();

        // A bare MP3: no RIFF header at all.
        let bare = temp("xubamp_riff_bare.mp3", &[0xff, 0xfb, 0x90, 0x44, 0, 0, 0, 0]);
        assert!(open(&bare).unwrap().is_none());
        std::fs::remove_file(&bare).ok();

        // A file too short to hold a header of any kind.
        let tiny = temp("xubamp_riff_tiny.mp3", b"RIF");
        assert!(open(&tiny).unwrap().is_none());
        std::fs::remove_file(&tiny).ok();
    }

    #[test]
    fn a_truncated_data_chunk_stops_at_the_end_of_the_file() {
        let mut bytes = wrap(&[7u8; 1000], 0x0055, None);
        bytes.truncate(bytes.len() - 400); // the data chunk now claims more than the file holds
        let path = temp("xubamp_riff_truncated.mp3", &bytes);
        let mut spliced = open(&path).unwrap().expect("still recognised");
        let mut all = Vec::new();
        spliced.read_to_end(&mut all).unwrap();
        assert_eq!(all.len(), 600, "reads what is there, not what is declared");
        assert_eq!(spliced.byte_len(), Some(600));
        std::fs::remove_file(&path).ok();
    }
}
