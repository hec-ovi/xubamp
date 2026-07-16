//! Symphonia-backed decoding of a track to interleaved f32 PCM. Producer-thread only.

use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey, Tag};
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// A decodable audio source: demuxer plus decoder producing interleaved f32 frames.
pub struct Source {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_buf: Option<SampleBuffer<f32>>,
    buf_cap_frames: usize,
    /// Sample rate of the most recently decoded packet (authoritative once decoding starts).
    pub sample_rate: u32,
    /// Channel count of the most recently decoded packet.
    pub channels: usize,
    /// Total playable length in frames, from the track header, or `None` when the format does
    /// not report one (a headerless, non-seekable stream). WAV always has it; MP3 has it from a
    /// Xing/Info/VBRI tag or a bitrate estimate on a seekable file. In gapless mode it is already
    /// the delay/padding-trimmed length, matching the position clock. Drives the seek bar.
    pub total_frames: Option<u64>,
}

/// Header-only duration probe: open the container, read the default audio track's frame count and
/// sample rate, and return whole seconds. No decoding happens, so it is cheap enough to run over a
/// whole playlist as tracks are added. Returns `None` when the length or rate is not carried in the
/// header (e.g. a VBR MP3 with no Xing header) or the file cannot be read, so a caller shows a blank
/// time rather than a wrong one.
pub fn probe_duration_secs(path: &Path) -> Option<u32> {
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
    let frames = track.codec_params.n_frames?;
    let rate = u64::from(track.codec_params.sample_rate?);
    (rate > 0).then(|| (frames / rate) as u32)
}

/// Header-level stream facts for the file-info box: rate and channel count straight off the
/// container, the header duration, and the codec's short name from the decoder registry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamInfo {
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub duration_secs: Option<u32>,
    pub codec: String,
}

/// Header-only probe of a track's stream facts. No decoding happens. `None` when the file
/// cannot be opened or no audio track is found.
pub fn probe_stream_info(path: &Path) -> Option<StreamInfo> {
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
    let params = &track.codec_params;
    let duration_secs = match (params.n_frames, params.sample_rate) {
        (Some(frames), Some(rate)) if rate > 0 => Some((frames / u64::from(rate)) as u32),
        _ => None,
    };
    let codec = symphonia::default::get_codecs()
        .get_codec(params.codec)
        .map(|descriptor| descriptor.long_name.to_owned())
        .unwrap_or_default();
    Some(StreamInfo {
        sample_rate: params.sample_rate,
        channels: params
            .channels
            .map(|c| c.count().min(u8::MAX as usize) as u8),
        duration_secs,
        codec,
    })
}

/// Artist and title read from a file's embedded tags. Both fields are `None` when the file
/// carries no usable tag, so the caller falls back to the file name, like classic Winamp.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackTags {
    pub artist: Option<String>,
    pub title: Option<String>,
}

impl TrackTags {
    /// The classic display name: `Artist - Title` when both tags are present, either alone
    /// otherwise, `None` when the tags carry nothing (caller falls back to the file name).
    pub fn display_name(&self) -> Option<String> {
        match (&self.artist, &self.title) {
            (Some(artist), Some(title)) => Some(format!("{artist} - {title}")),
            (None, Some(title)) => Some(title.clone()),
            (Some(artist), None) => Some(artist.clone()),
            (None, None) => None,
        }
    }
}

/// Header-only tag probe: read the artist and title from a file's metadata without decoding any
/// audio. Covers an ID3v2 block preceding MP3 frames (surfaced by the probe), and container-level
/// metadata (Vorbis comments in Ogg/FLAC, RIFF INFO in WAV). Returns `None` when the file cannot
/// be opened or probed; a readable file with no tags yields empty [`TrackTags`].
pub fn probe_tags(path: &Path) -> Option<TrackTags> {
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let mut tags = TrackTags::default();
    // Metadata found by the probe outside the container (an ID3v2 block before MP3 frames).
    if let Some(metadata) = probed.metadata.get() {
        if let Some(revision) = metadata.current() {
            collect_tags(revision.tags(), &mut tags);
        }
    }
    // Container-carried metadata (Vorbis comments, RIFF INFO). The first source wins so a
    // dedicated leading tag block is not overridden by a weaker container field.
    if let Some(revision) = probed.format.metadata().current() {
        collect_tags(revision.tags(), &mut tags);
    }
    Some(tags)
}

/// Fold a metadata revision's tags into `out`, keeping the first non-empty artist and title.
fn collect_tags(read: &[Tag], out: &mut TrackTags) {
    for tag in read {
        let slot = match tag.std_key {
            Some(StandardTagKey::Artist) => &mut out.artist,
            Some(StandardTagKey::TrackTitle) => &mut out.title,
            _ => continue,
        };
        if slot.is_none() {
            let value = tag.value.to_string();
            // RIFF INFO strings carry their NUL terminator (and pad byte) into the value.
            let trimmed = value.trim_matches(|c: char| c.is_whitespace() || c == '\0');
            if !trimmed.is_empty() {
                *slot = Some(trimmed.to_owned());
            }
        }
    }
}

impl Source {
    /// Open a file and prepare its default audio track for decoding.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        // Gapless trims LAME/Xing encoder delay and padding on MP3 (no leading click).
        let fmt_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &fmt_opts,
            &MetadataOptions::default(),
        )?;
        let format = probed.format;

        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(Error::Unsupported("no decodable audio track"))?;
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())?;

        // WAV reports these up front; MP3 may leave them None until the first frame decodes.
        let sample_rate = track.codec_params.sample_rate.unwrap_or(0);
        let channels = track.codec_params.channels.map_or(0, |c| c.count());
        // The header-reported length (frames). Present for WAV and most MP3s; used only for the
        // seek bar and the total-time display, never for decode correctness.
        let total_frames = track.codec_params.n_frames;

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_buf: None,
            buf_cap_frames: 0,
            sample_rate,
            channels,
            total_frames,
        })
    }

    /// Next block of interleaved f32 samples (`len == frames * channels`), or `Ok(None)` at
    /// clean end of stream. The returned slice borrows an internal buffer that stays valid
    /// only until the next call.
    pub fn next_interleaved(&mut self) -> Result<Option<&[f32]>, Error> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                // 0.5.x signals EOF as an UnexpectedEof IO error, not Ok(None).
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
                Err(Error::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(e),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec: SignalSpec = *decoded.spec();
                    self.sample_rate = spec.rate;
                    self.channels = spec.channels.count();

                    let cap_frames = decoded.capacity();
                    if self.sample_buf.is_none() || self.buf_cap_frames < cap_frames {
                        // Allocation happens here, on the decode thread, never in the callback.
                        self.sample_buf = Some(SampleBuffer::<f32>::new(cap_frames as u64, spec));
                        self.buf_cap_frames = cap_frames;
                    }
                    let buf = self.sample_buf.as_mut().unwrap();
                    buf.copy_interleaved_ref(decoded);
                    return Ok(Some(buf.samples()));
                }
                Err(Error::DecodeError(_)) => continue, // recoverable: symphonia resyncs
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
                Err(e) => return Err(e),
            }
        }
    }

    /// Seek so decoding resumes at `seconds` from the start of the track, returning the frame it
    /// actually landed on. `SeekMode::Accurate` lands at or just before the request (at most one
    /// packet early, which is imperceptible for a Winamp-style scrub), so the returned frame is
    /// the true resume point and the caller rebases the position clock to it. The decoder is reset
    /// afterwards, as the `FormatReader` contract requires: seeking invalidates the decoder's
    /// carried state (for MP3, the bit reservoir and overlap buffers).
    pub fn seek(&mut self, seconds: f64) -> Result<u64, Error> {
        let time = Time::new(seconds.trunc() as u64, seconds.fract());
        let seeked = self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(self.track_id),
            },
        )?;
        self.decoder.reset();
        Ok(seeked.actual_ts)
    }
}

