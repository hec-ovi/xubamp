//! The player: ties the playlist to the audio engine. It owns the current [`AudioEngine`] and the
//! [`Playlist`], and switching tracks drops the old engine and starts a fresh one (each track gets
//! its own PipeWire stream at its native rate, so no resampler is needed to move between tracks of
//! different rates). It lives on the main/UI thread, so it needs no locking.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use xubamp_audio::engine::AudioEngine;
use xubamp_audio::playlist::Playlist;
use xubamp_render::hit::{ModeButton, Playback, Transport};
use xubamp_render::pledit;

use crate::{track_title, transport_ops, EngineOp};

pub struct Player {
    playlist: Playlist,
    /// The engine for the current track, or `None` before the first track starts / after a failed
    /// load / on an empty playlist.
    engine: Option<AudioEngine>,
    /// Volume and balance persist across tracks: a freshly loaded engine starts at these.
    volume: u8,
    balance: i8,
    /// Whether the last transport was Stop (as opposed to Pause), so the visualizer can clear rather
    /// than freeze. Reset by any action that (re)starts or changes the track.
    stopped: bool,
    /// Shuffle mode: when on, advancing plays a random track. (Repeat mode lives on the playlist.)
    shuffle: bool,
    /// xorshift PRNG state for shuffle, seeded once at startup.
    rng: u64,
}

impl Player {
    pub fn new(tracks: Vec<PathBuf>) -> Self {
        Self {
            playlist: Playlist::new(tracks),
            engine: None,
            volume: 100,
            balance: 0,
            stopped: false,
            shuffle: false,
            // Seed the PRNG from the wall clock; `| 1` avoids the all-zero state xorshift can't leave.
            rng: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15)
                | 1,
        }
    }

    /// Start playing the current track (called once at startup). No-op on an empty playlist.
    pub fn start(&mut self) {
        self.load_current();
    }

    /// Drop the old engine (joining its threads and freeing its stream) and start a fresh one for
    /// the current track, applying the persisted volume/balance. A load failure leaves no engine.
    fn load_current(&mut self) {
        self.stopped = false; // a (re)loaded track is playing, not stopped
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
            Transport::Next => self.advance(),
            Transport::Eject => eprintln!("xubamp: Eject (load file) not implemented yet"),
            Transport::Play | Transport::Pause | Transport::Stop => {
                // Stop clears the visualizer (a reset); Play/Pause do not.
                self.stopped = matches!(t, Transport::Stop);
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

    /// Advance to the next track: a random one in shuffle mode, otherwise the next in order (which
    /// wraps when repeat is on). Loads and plays whatever it lands on; a no-op at a hard end.
    fn advance(&mut self) {
        if self.shuffle && self.playlist.len() > 1 {
            let i = self.next_random();
            if self.playlist.select(i).is_some() {
                self.load_current();
            }
        } else if self.playlist.next().is_some() {
            self.load_current();
        }
    }

    /// Toggle shuffle or repeat mode.
    pub fn toggle_mode(&mut self, mode: ModeButton) {
        match mode {
            ModeButton::Shuffle => self.shuffle = !self.shuffle,
            ModeButton::Repeat => {
                let on = !self.playlist.repeat();
                self.playlist.set_repeat(on);
            }
        }
    }

    /// A xorshift step.
    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// A random track index that is not the current one (so shuffle does not replay in place).
    fn next_random(&mut self) -> usize {
        let n = self.playlist.len();
        let cur = self.playlist.current_index();
        loop {
            let i = (self.rand() % n as u64) as usize;
            if n == 1 || Some(i) != cur {
                return i;
            }
        }
    }

    /// The `x` hotkey: force the current track from the top regardless of play state.
    pub fn restart(&mut self) {
        self.stopped = false;
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
        if self.engine.as_ref().is_some_and(AudioEngine::is_finished) {
            self.advance();
        }
    }

    /// The playlist rows (numbered file stems; durations arrive once tracks are probed) and the
    /// index of the currently-playing track, for the playlist window to render.
    pub fn playlist_view(&self) -> (Vec<pledit::Row>, Option<usize>) {
        let rows = self
            .playlist
            .tracks()
            .iter()
            .enumerate()
            .map(|(i, path)| pledit::Row {
                title: format!("{}. {}", i + 1, track_title(&path.to_string_lossy())),
                duration: String::new(),
            })
            .collect();
        (rows, self.playlist.current_index())
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
                    stopped: self.stopped,
                    kbps: h.bitrate_kbps(),
                    khz: h.khz(),
                    channels: h.channels(),
                    title: self.title(),
                    shuffle: self.shuffle,
                    repeat: self.playlist.repeat(),
                }
            }
            // Nothing loaded, but shuffle/repeat modes still light their buttons.
            None => Playback {
                shuffle: self.shuffle,
                repeat: self.playlist.repeat(),
                ..Default::default()
            },
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
