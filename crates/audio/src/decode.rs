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

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_buf: None,
            buf_cap_frames: 0,
            sample_rate,
            channels,
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

    /// Seek to `seconds` from the start of the track.
    pub fn seek(&mut self, seconds: f64) -> Result<(), Error> {
        let time = Time::new(seconds.trunc() as u64, seconds.fract());
        self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(self.track_id),
            },
        )?;
        self.decoder.reset();
        Ok(())
    }
}
