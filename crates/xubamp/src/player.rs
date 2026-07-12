//! The player: ties the playlist to the audio engine. It owns the current [`AudioEngine`] and the
//! [`Playlist`], and switching tracks drops the old engine and starts a fresh one (each track gets
//! its own PipeWire stream at its native rate, so no resampler is needed to move between tracks of
//! different rates). It lives on the main/UI thread, so it needs no locking.

use std::path::{Path, PathBuf};

use xubamp_audio::engine::AudioEngine;
use xubamp_audio::playlist::Playlist;
use xubamp_render::hit::{Playback, Transport};

use crate::{track_title, transport_ops, EngineOp};

pub struct Player {
    playlist: Playlist,
    /// The engine for the current track, or `None` before the first track starts / after a failed
    /// load / on an empty playlist.
    engine: Option<AudioEngine>,
    /// Volume and balance persist across tracks: a freshly loaded engine starts at these.
    volume: u8,
    balance: i8,
}

impl Player {
    pub fn new(tracks: Vec<PathBuf>) -> Self {
        Self {
            playlist: Playlist::new(tracks),
            engine: None,
            volume: 100,
            balance: 0,
        }
    }

    /// Start playing the current track (called once at startup). No-op on an empty playlist.
    pub fn start(&mut self) {
        self.load_current();
    }

    /// Drop the old engine (joining its threads and freeing its stream) and start a fresh one for
    /// the current track, applying the persisted volume/balance. A load failure leaves no engine.
    fn load_current(&mut self) {
        self.engine = None; // drop first: join the old threads before opening a new stream
        let Some(path) = self.playlist.current().map(Path::to_path_buf) else {
            return;
        };
        match AudioEngine::play(&path) {
            Ok(engine) => {
                let h = engine.handle();
                h.set_volume(self.volume);
                h.set_balance(self.balance);
                eprintln!("xubamp: playing {}", path.display());
                self.engine = Some(engine);
            }
            Err(e) => eprintln!("xubamp: cannot play {}: {e}", path.display()),
        }
    }

    /// Carry out a transport command. Play/Pause/Stop go through the shared [`transport_ops`] policy
    /// applied to the current engine; Prev/Next move through the playlist (loading a new track);
    /// Eject awaits the file dialog.
    pub fn transport(&mut self, t: Transport) {
        match t {
            Transport::Prev => {
                if self.playlist.prev().is_some() {
                    self.load_current();
                }
            }
            Transport::Next => {
                if self.playlist.next().is_some() {
                    self.load_current();
                }
            }
            Transport::Eject => eprintln!("xubamp: Eject (load file) not implemented yet"),
            Transport::Play | Transport::Pause | Transport::Stop => {
                if let Some(engine) = &self.engine {
                    let h = engine.handle();
                    for op in transport_ops(t, h.is_playing(), h.is_finished()) {
                        match op {
                            EngineOp::SeekToStart => h.seek_to_start(),
                            EngineOp::SetActive(active) => h.set_active(active),
                        }
                    }
                }
            }
        }
    }

    /// The `x` hotkey: force the current track from the top regardless of play state.
    pub fn restart(&mut self) {
        if let Some(engine) = &self.engine {
            let h = engine.handle();
            h.seek_to_start();
            h.set_active(true);
        }
    }

    pub fn set_volume(&mut self, volume: u8) {
        self.volume = volume;
        if let Some(engine) = &self.engine {
            engine.handle().set_volume(volume);
        }
    }

    pub fn set_balance(&mut self, balance: i8) {
        self.balance = balance;
        if let Some(engine) = &self.engine {
            engine.handle().set_balance(balance);
        }
    }

    pub fn seek_fraction(&mut self, fraction: f32) {
        if let Some(engine) = &self.engine {
            engine.handle().seek_fraction(fraction);
        }
    }

    /// Called each UI tick: auto-advance to the next track when the current one has played out.
    pub fn poll(&mut self) {
        if self.engine.as_ref().is_some_and(AudioEngine::is_finished) && self.playlist.next().is_some()
        {
            self.load_current();
        }
    }

    /// The current track's marquee title (its file stem), or empty when nothing is loaded.
    pub fn title(&self) -> String {
        self.playlist
            .current()
            .map(|p| track_title(&p.to_string_lossy()))
            .unwrap_or_default()
    }

    /// A clock + track-info snapshot for the window's redraw tick.
    pub fn playback(&self) -> Playback {
        match &self.engine {
            Some(engine) => {
                let h = engine.handle();
                Playback {
                    elapsed: Some(h.elapsed_secs()),
                    position: h.position_fraction(),
                    duration: h.duration_secs(),
                    playing: h.is_playing(),
                    kbps: h.bitrate_kbps(),
                    khz: h.khz(),
                    channels: h.channels(),
                    title: self.title(),
                }
            }
            None => Playback::default(),
        }
    }

    /// Fill `out` with the current track's most recent output samples for the visualizer (silence
    /// when nothing is loaded).
    pub fn read_scope(&self, out: &mut [f32]) {
        match &self.engine {
            Some(engine) => engine.handle().read_scope(out),
            None => out.iter_mut().for_each(|s| *s = 0.0),
        }
    }
}
