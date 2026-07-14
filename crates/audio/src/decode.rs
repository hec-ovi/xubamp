//! Symphonia-backed decoding of a track to interleaved f32 PCM. Producer-thread only.

use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
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
