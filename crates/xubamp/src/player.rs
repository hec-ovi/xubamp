//! The player: ties the playlist to the audio engine. It owns the current [`AudioEngine`] and the
//! [`Playlist`], and switching tracks drops the old engine and starts a fresh one (each track gets
//! its own PipeWire stream at its native rate, so no resampler is needed to move between tracks of
//! different rates). It lives on the main/UI thread, so it needs no locking.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use xubamp_audio::engine::AudioEngine;
use xubamp_audio::playlist::Playlist;
use xubamp_audio::EqSettings;
use xubamp_render::hit::{ModeButton, Playback, Transport};
use xubamp_render::pledit;

use crate::{track_title, transport_ops, EngineOp, TransportState};

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
    /// Equalizer controls persist across tracks. Every newly loaded engine starts with this exact
    /// snapshot, while an in-flight engine receives changes through its lock-free control handle.
    equalizer: EqSettings,
    /// xorshift PRNG state for shuffle, seeded once at startup.
    rng: u64,
}

impl Player {
    /// Construct a player with the classic defaults. Kept for callers and tests that do not restore
    /// persisted settings.
    pub fn new(tracks: Vec<PathBuf>) -> Self {
        Self {
            playlist: Playlist::new(tracks),
            engine: None,
            volume: 100,
            balance: 0,
            stopped: false,
            shuffle: false,
            equalizer: EqSettings::default(),
            // Seed the PRNG from the wall clock; `| 1` avoids the all-zero state xorshift can't leave.
            rng: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15)
                | 1,
        }
    }

    /// Construct a player from persisted playback and equalizer settings without starting audio.
    /// Delaying playback until [`Self::start`] lets startup finish restoring all state first.
    pub fn with_settings(
        tracks: Vec<PathBuf>,
        shuffle: bool,
        repeat: bool,
        equalizer: EqSettings,
    ) -> Self {
        let mut player = Self::new(tracks);
        player.shuffle = shuffle;
        player.playlist.set_repeat(repeat);
        player.set_equalizer_settings(equalizer);
        player
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
        match AudioEngine::play_with_equalizer(&path, self.equalizer_settings()) {
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
                // In shuffle, Back retraces the real play order (the history stack) so it returns to
                // the track you actually just heard. In order, Prev is simply the previous track, so
                // toggling shuffle off gives plain sequential navigation.
                let moved = if self.shuffle {
                    let fresh = self.playlist.peek_prev();
                    self.playlist.back(fresh).is_some()
                } else {
                    self.playlist.prev().is_some()
                };
                if moved {
                    self.load_current();
                }
            }
            Transport::Next => {
                self.advance();
            }
            Transport::Eject => eprintln!("xubamp: Eject (load file) not implemented yet"),
            Transport::Play | Transport::Pause | Transport::Stop => {
                // Stop clears the visualizer (a reset); Play/Pause do not.
                let was_stopped = self.stopped;
                self.stopped = matches!(t, Transport::Stop);
                if let Some(engine) = &self.engine {
                    let h = engine.handle();
                    let state = if h.is_finished() {
                        TransportState::Finished
                    } else if h.is_playing() {
                        TransportState::Playing
                    } else if was_stopped {
                        TransportState::Stopped
                    } else {
                        TransportState::Paused
                    };
                    for op in transport_ops(t, state) {
                        match op {
                            EngineOp::SeekToStart => h.seek_to_start(),
                            EngineOp::SetActive(active) => h.set_active(active),
                        }
                    }
                }
            }
        }
    }

    /// Advance to the next track (Next button or auto-advance): redo a stepped-back track if any,
    /// else a random one in shuffle mode or the next in order (wrapping when repeat is on). Loads and
    /// plays whatever it lands on; a no-op at a hard end. Remembers the departing track for Back.
    fn advance(&mut self) -> bool {
        let moved = if self.shuffle && self.playlist.len() > 1 {
            // Redo a stepped-back track if any, else a fresh random pick; both remember the trail.
            let fresh = Some(self.next_random());
            self.playlist.forward(fresh).is_some()
        } else {
            // In order, Next is simply the next track (wrapping with repeat).
            self.playlist.next().is_some()
        };
        if moved {
            self.load_current();
        }
        moved
    }

    /// Play the playlist track at index `i` (a double-click in the playlist window). Remembers the
    /// current track for Back and clears the forward stack (a fresh navigation). No-op if the index
    /// is out of range.
    pub fn play_index(&mut self, i: usize) {
        // In shuffle, remember the departing track for Back; in order, a plain select keeps Prev/Next
        // sequential from the new position.
        let moved = if self.shuffle {
            self.playlist.jump_to(i).is_some()
        } else {
            self.playlist.select(i).is_some()
        };
        if moved {
            self.load_current();
        }
    }

    /// Toggle shuffle or repeat mode.
    pub fn toggle_mode(&mut self, mode: ModeButton) {
        match mode {
            ModeButton::Shuffle => {
                self.shuffle = !self.shuffle;
                // The Back/Forward history only drives navigation in shuffle, so reset it on any
                // mode change so a stale shuffle trail never leaks into sequential play (or back).
                self.playlist.clear_history();
            }
            ModeButton::Repeat => {
                let on = !self.playlist.repeat();
                self.playlist.set_repeat(on);
            }
        }
    }

    /// Current shuffle mode, used to persist a user toggle without coupling the player to the
    /// settings file format.
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Current repeat-all mode, used to persist a user toggle.
    pub fn repeat(&self) -> bool {
        self.playlist.repeat()
    }

    /// Return the coherent equalizer snapshot that will carry into the next track.
    pub fn equalizer_settings(&self) -> EqSettings {
        self.equalizer
    }

    /// Update both the current engine and the state inherited by future tracks. Persistence is left
    /// to a future equalizer UI commit so slider motion never writes the settings file per redraw.
    pub fn set_equalizer_settings(&mut self, settings: EqSettings) {
        self.equalizer = settings.sanitized();
        if let Some(engine) = &self.engine {
            engine.handle().set_equalizer_settings(self.equalizer);
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
        if self.stopped || !self.engine.as_ref().is_some_and(AudioEngine::is_finished) {
            return;
        }
        if !self.advance() {
            // Natural end at a hard playlist boundary: leave the decoder and clock at the end. The
            // stopped marker prevents this poll from retrying forever; Play/Pause can restart it.
            self.stopped = true;
        }
    }

    /// The playlist rows (numbered file stems; durations arrive once tracks are probed) and the
    /// index of the currently-playing track, for the playlist window to render.
    pub fn playlist_view(&self) -> (Vec<pledit::Row>, Option<usize>) {
        let rows = self
            .playlist
            .tracks()
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

/// End-to-end tests of the player against the real PipeWire-backed engine: they play short WAVs to
/// their end and assert the playlist behaviour at the boundary. Ignored by default (they need a
/// running PipeWire session); in the dev container route them to a silent sink to stay quiet:
///   cargo test -p xubamp --features audio player -- --ignored --nocapture
/// (set PIPEWIRE_NODE=<null-sink> first for silence.)
#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn constructor_restores_modes_and_equalizer_without_starting_audio() {
        let mut equalizer = EqSettings {
            enabled: false,
            preamp_db: 4.5,
            ..EqSettings::default()
        };
        equalizer.bands_db[3] = -7.25;

        let player = Player::with_settings(Vec::new(), true, true, equalizer);

        assert!(player.shuffle());
        assert!(player.repeat());
        assert_eq!(player.equalizer_settings(), equalizer);
        assert!(
            player.engine.is_none(),
            "construction must not start PipeWire"
        );
    }

    #[test]
    fn default_constructor_and_mode_toggles_remain_compatible() {
        let mut player = Player::new(Vec::new());
        assert!(!player.shuffle());
        assert!(!player.repeat());
        assert_eq!(player.equalizer_settings(), EqSettings::default());

        player.toggle_mode(ModeButton::Shuffle);
        player.toggle_mode(ModeButton::Repeat);
        assert!(player.shuffle());
        assert!(player.repeat());
    }

    #[test]
    fn equalizer_setter_sanitizes_and_persists_without_an_engine() {
        let mut player = Player::new(Vec::new());
        let mut settings = EqSettings {
            preamp_db: f32::NAN,
            ..EqSettings::default()
        };
        settings.bands_db[0] = 50.0;

        player.set_equalizer_settings(settings);

        assert_eq!(player.equalizer_settings().preamp_db, 0.0);
        assert_eq!(player.equalizer_settings().bands_db[0], 12.0);
    }

    /// Write a dependency-free 16-bit PCM stereo WAV of a 440 Hz sine, `seconds` long.
    fn write_wav(path: &Path, rate: u32, seconds: u32) {
        let channels: u16 = 2;
        let bits: u16 = 16;
        let frames = rate * seconds;
        let data_len = frames * channels as u32 * (bits / 8) as u32;
        let byte_rate = rate * channels as u32 * (bits / 8) as u32;
        let block_align = channels * (bits / 8);
        let mut buf = Vec::with_capacity(44 + data_len as usize);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_len).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_len.to_le_bytes());
        let step = std::f64::consts::TAU * 440.0 / rate as f64;
        for i in 0..frames {
            let s = ((i as f64 * step).sin() * 0.2 * 32767.0) as i16;
            buf.extend_from_slice(&s.to_le_bytes()); // L
            buf.extend_from_slice(&s.to_le_bytes()); // R
        }
        std::fs::write(path, buf).expect("write wav");
    }

    #[test]
    #[ignore = "needs a running PipeWire session"]
    fn a_single_track_stops_at_the_end_without_rewinding() {
        let path = std::env::temp_dir().join("xubamp_player_eos.wav");
        write_wav(&path, 48_000, 1); // one second, finishes quickly
        let mut player = Player::new(vec![path.clone()]);
        player.start();

        // Poll as the UI loop does; the track should play out and then STOP (not hang finished).
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            player.poll();
            if player.playback().stopped {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "single track never stopped at its end: {:?}",
                player.playback(),
            );
            thread::sleep(Duration::from_millis(30));
        }

        // The hard end is handled once, but the decoder/clock remains at the final position. This is
        // distinct from pressing Stop, which explicitly rewinds to zero.
        thread::sleep(Duration::from_millis(400));
        player.poll();
        let pb = player.playback();
        assert!(!pb.playing, "stopped at the end, not left playing");
        assert!(pb.stopped, "the end-of-playlist stop holds");
        assert!(
            pb.position.unwrap_or(0.0) > 0.95,
            "position remains at the end ({:?})",
            pb.position
        );
        assert!(
            pb.elapsed.unwrap_or(0) >= 1,
            "clock remains at the end ({:?})",
            pb.elapsed
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[ignore = "needs a running PipeWire session"]
    fn a_finished_track_auto_advances_to_the_next() {
        let a = std::env::temp_dir().join("xubamp_player_adv_a.wav");
        let b = std::env::temp_dir().join("xubamp_player_adv_b.wav");
        write_wav(&a, 48_000, 1);
        write_wav(&b, 48_000, 2); // longer, so there is a window to observe it playing
        let mut player = Player::new(vec![a.clone(), b.clone()]);
        player.start();
        assert_eq!(
            player.playlist_view().1,
            Some(0),
            "starts on the first track"
        );

        // The first track plays out; the auto-advance poll should move to the second and play it.
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            player.poll();
            if player.playlist_view().1 == Some(1) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the first track did not auto-advance to the second: {:?}",
                player.playback(),
            );
            thread::sleep(Duration::from_millis(30));
        }
        // On the second track it is playing again, not in the stopped end state.
        let pb = player.playback();
        assert!(!pb.stopped, "advancing to the next track is not a stop");

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }
}
