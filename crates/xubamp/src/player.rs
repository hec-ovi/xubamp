//! The player: ties the playlist to the audio engine. It owns the current [`AudioEngine`] and the
//! [`Playlist`], and switching tracks drops the old engine and starts a fresh one (each track gets
//! its own PipeWire stream at its native rate, so no resampler is needed to move between tracks of
//! different rates). It lives on the main/UI thread, so it needs no locking.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use xubamp_audio::engine::AudioEngine;
use xubamp_audio::playlist::{Playlist, ShuffleCycle, TrackId};
use xubamp_audio::EqSettings;
use xubamp_library::is_audio_path;
use xubamp_render::hit::{ModeButton, Playback, Transport};
use xubamp_render::pledit;

use crate::{track_title, transport_ops, EngineOp, TransportState};

#[derive(Clone, Copy)]
enum Direction {
    Forward,
    Backward,
}

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
    /// Shuffle mode traverses a stable-ID permutation. Repeat mode lives on the playlist.
    shuffle: bool,
    /// The pending permutation is independent of display order, so playlist edits cannot silently
    /// retarget a choice to the entry that inherited an old index.
    shuffle_cycle: ShuffleCycle,
    /// Equalizer controls persist across tracks. Every newly loaded engine starts with this exact
    /// snapshot, while an in-flight engine receives changes through its lock-free control handle.
    equalizer: EqSettings,
}

impl Player {
    /// Construct a player with the classic defaults. Kept for callers and tests that do not restore
    /// persisted settings.
    pub fn new(tracks: Vec<PathBuf>) -> Self {
        let mut player = Self {
            playlist: Playlist::default(),
            engine: None,
            volume: 100,
            balance: 0,
            stopped: false,
            shuffle: false,
            shuffle_cycle: ShuffleCycle::from_entropy(),
            equalizer: EqSettings::default(),
        };
        player.append_paths(tracks);
        player.shuffle_cycle.anchor(&player.playlist);
        player
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
        if shuffle {
            player.shuffle_cycle.anchor(&player.playlist);
        }
        player
    }

    /// Append supported local audio paths in the user's input order without changing transport
    /// state or starting a decoder. Duplicate paths remain distinct playlist entries, matching a
    /// classic playlist. When the playlist was empty, its first accepted entry becomes current; the
    /// stopped, paused, or active transport state is otherwise left exactly as it was.
    ///
    /// The returned stable IDs correspond one-for-one with accepted MP3/WAV paths. Unsupported
    /// extensions are ignored as a final audio-only guard after file or directory selection.
    pub fn append_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> Vec<TrackId> {
        self.playlist
            .extend(paths.into_iter().filter(|path| is_audio_path(path)))
    }

    /// Start playing the current track (called once at startup). No-op on an empty playlist.
    pub fn start(&mut self) {
        let Some(current) = self.playlist.current_id() else {
            self.stopped = true;
            return;
        };

        // Try lazily: precomputing a shuffled candidate list would consume the whole permutation
        // even when the first file loads. A broken first file still scans at most each stable ID
        // once, and repeat can never turn an all-broken playlist into an infinite retry loop.
        if self.try_load(current, true) {
            self.playlist.clear_history();
            return;
        }
        if self.shuffle {
            let mut seen = HashSet::from([current]);
            let max_steps = self.playlist.len().saturating_mul(2);
            for _ in 0..max_steps {
                if seen.len() >= self.playlist.len() {
                    break;
                }
                let Some(id) = self
                    .shuffle_cycle
                    .next(&self.playlist, self.playlist.repeat())
                else {
                    break;
                };
                if !seen.insert(id) {
                    continue;
                }
                if self.try_load(id, true) {
                    self.playlist.select_track(id);
                    self.playlist.clear_history();
                    return;
                }
            }
        } else if let Some(index) = self.playlist.current_index() {
            let candidates = self.linear_candidates(index, Direction::Forward, true);
            for id in candidates.into_iter().skip(1) {
                if self.try_load(id, true) {
                    self.playlist.select_track(id);
                    self.playlist.clear_history();
                    return;
                }
            }
        }
        self.stopped = true;
    }

    /// Drop the old engine and try one stable-ID candidate. When `active` is false the destination
    /// is still decoded and ready, but remains in the classic stopped state at its beginning.
    fn try_load(&mut self, id: TrackId, active: bool) -> bool {
        let Some(path) = self.playlist.path_for_id(id).map(Path::to_path_buf) else {
            return false;
        };
        self.engine = None; // join old workers before opening the replacement stream
        match AudioEngine::play_with_equalizer(&path, self.equalizer_settings()) {
            Ok(engine) => {
                let h = engine.handle();
                h.set_volume(self.volume);
                h.set_balance(self.balance);
                if !active {
                    h.set_active(false);
                    h.seek_to_start();
                }
                self.stopped = !active;
                eprintln!(
                    "xubamp: {} {}",
                    if active { "playing" } else { "loaded" },
                    path.display()
                );
                self.engine = Some(engine);
                true
            }
            Err(e) => {
                eprintln!("xubamp: cannot play {}: {e}", path.display());
                false
            }
        }
    }

    /// Carry out a transport command. Play/Pause/Stop go through the shared [`transport_ops`] policy
    /// applied to the current engine; Prev/Next move through the playlist (loading a new track);
    /// Eject awaits the file dialog.
    pub fn transport(&mut self, t: Transport) {
        match t {
            Transport::Prev => {
                let active = self.navigation_autoplays();
                self.navigate_previous(active);
            }
            Transport::Next => {
                let active = self.navigation_autoplays();
                self.advance(active);
            }
            Transport::Eject => eprintln!("xubamp: Eject (load file) not implemented yet"),
            Transport::Play | Transport::Pause | Transport::Stop => {
                // Stop clears the visualizer (a reset); Play/Pause do not.
                let was_stopped = self.stopped;
                self.stopped = matches!(t, Transport::Stop);
                let Some(engine) = &self.engine else {
                    // A previous decode failure may have left the selection intact but no engine.
                    // Classic Play (and Pause-as-Play) retries from that selection with the same
                    // bounded invalid-file scan used at startup.
                    if matches!(t, Transport::Play | Transport::Pause) {
                        self.start();
                    }
                    return;
                };
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

    /// Advance to the next playable track. In shuffle, a browser-style Forward redo is tried before
    /// consuming the next permutation member. Failed files are skipped transactionally, so they do
    /// not enter actual play history, and the unique-attempt bound prevents repeat from looping.
    fn advance(&mut self, active: bool) -> bool {
        if self.playlist.is_empty() {
            return false;
        }
        if !self.shuffle {
            let Some(index) = self.playlist.current_index() else {
                return false;
            };
            let candidates = self.linear_candidates(index, Direction::Forward, false);
            return self.try_linear_candidates(candidates, active);
        }

        let mut attempted = HashSet::new();
        let mut attempted_load = false;
        // History is capped at 256 and one complete cycle is at most `len`; the extra cycle covers
        // duplicate redo IDs plus crossing a repeat boundary while skipping broken files.
        let max_steps = self.playlist.len().saturating_mul(2).saturating_add(257);
        for _ in 0..max_steps {
            if attempted.len() >= self.playlist.len() {
                break;
            }
            let (id, redo) = if let Some(id) = self.playlist.forward_candidate(None) {
                (id, true)
            } else {
                let Some(id) = self
                    .shuffle_cycle
                    .next(&self.playlist, self.playlist.repeat())
                else {
                    break;
                };
                (id, false)
            };

            if !attempted.insert(id) {
                if redo {
                    self.playlist.discard_forward_candidate(id);
                }
                continue;
            }
            attempted_load = true;
            if self.try_load(id, active) {
                if self.playlist.commit_forward(id).is_some() {
                    return true;
                }
                self.engine = None;
                break;
            }
            if redo {
                self.playlist.discard_forward_candidate(id);
            }
        }
        if attempted_load {
            self.stopped = true;
        }
        false
    }

    /// Previous retraces successfully-played shuffle history. It never manufactures a random or
    /// index-adjacent predecessor when that history is exhausted.
    fn navigate_previous(&mut self, active: bool) -> bool {
        if !self.shuffle {
            let Some(index) = self.playlist.current_index() else {
                return false;
            };
            let candidates = self.linear_candidates(index, Direction::Backward, false);
            return self.try_linear_candidates(candidates, active);
        }

        let mut attempted = HashSet::new();
        let mut attempted_load = false;
        for _ in 0..257 {
            if attempted.len() >= self.playlist.len() {
                break;
            }
            let Some(id) = self.playlist.back_candidate(None) else {
                break;
            };
            if !attempted.insert(id) {
                self.playlist.discard_back_candidate(id);
                continue;
            }
            attempted_load = true;
            if self.try_load(id, active) {
                if self.playlist.commit_back(id).is_some() {
                    return true;
                }
                self.engine = None;
                break;
            }
            self.playlist.discard_back_candidate(id);
        }
        if attempted_load {
            self.stopped = true;
        }
        false
    }

    /// Play the playlist track at index `i` (a double-click in the playlist window). Remembers the
    /// current track for Back and clears the forward stack (a fresh navigation). No-op if the index
    /// is out of range.
    pub fn play_index(&mut self, i: usize) {
        let Some(selected) = self.playlist.track_id(i) else {
            return;
        };

        if self.try_load(selected, true) {
            if self.shuffle {
                self.playlist.jump_to(i);
                self.shuffle_cycle.anchor(&self.playlist);
            } else {
                self.playlist.select_track(selected);
            }
            return;
        }

        // A double-clicked broken entry follows the same bounded skip policy as transport. Build a
        // fresh cycle anchored at the requested row so a successful fallback still starts a manual
        // shuffle traversal rather than leaking the old pending order.
        let mut attempted = HashSet::from([selected]);
        if self.shuffle {
            let mut fresh_cycle = self.shuffle_cycle.clone();
            fresh_cycle.anchor_at(&self.playlist, Some(selected));
            for _ in 1..self.playlist.len() {
                let Some(id) = fresh_cycle.next(&self.playlist, self.playlist.repeat()) else {
                    break;
                };
                if !attempted.insert(id) {
                    continue;
                }
                if self.try_load(id, true) {
                    if let Some(index) = self.playlist.index_of_id(id) {
                        self.playlist.jump_to(index);
                        self.shuffle_cycle = fresh_cycle;
                        return;
                    }
                    self.engine = None;
                    break;
                }
            }
        } else {
            let candidates = self.linear_candidates(i, Direction::Forward, true);
            for id in candidates.into_iter().skip(1) {
                if !attempted.insert(id) {
                    continue;
                }
                if self.try_load(id, true) {
                    self.playlist.select_track(id);
                    return;
                }
            }
        }
        self.stopped = true;
    }

    /// Toggle shuffle or repeat mode.
    pub fn toggle_mode(&mut self, mode: ModeButton) {
        match mode {
            ModeButton::Shuffle => {
                self.shuffle = !self.shuffle;
                // The Back/Forward history only drives navigation in shuffle, so reset it on any
                // mode change so a stale shuffle trail never leaks into sequential play (or back).
                self.playlist.clear_history();
                if self.shuffle {
                    self.shuffle_cycle.anchor(&self.playlist);
                } else {
                    self.shuffle_cycle.clear();
                }
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

    /// Track changes keep playing from Playing and Paused, but a user pressing Next/Previous after
    /// Stop (or before the end poll observes a naturally-finished stream) merely loads the target.
    fn navigation_autoplays(&self) -> bool {
        self.engine
            .as_ref()
            .is_some_and(|engine| !self.stopped && !engine.is_finished())
    }

    /// Stable-ID candidates in classic display order. With repeat off this stops at the relevant
    /// edge. With repeat on it visits exactly one full playlist, including the starting track last
    /// for a transport skip (which restarts a single-track playlist).
    fn linear_candidates(
        &self,
        start: usize,
        direction: Direction,
        include_start: bool,
    ) -> Vec<TrackId> {
        let len = self.playlist.len();
        if start >= len || len == 0 {
            return Vec::new();
        }

        let mut result = Vec::with_capacity(len);
        let first_step = usize::from(!include_start);
        for offset in 0..len {
            let step = first_step + offset;
            let index = match direction {
                Direction::Forward => {
                    let raw = start + step;
                    if raw >= len && !self.playlist.repeat() {
                        break;
                    }
                    raw % len
                }
                Direction::Backward => {
                    if step > start && !self.playlist.repeat() {
                        break;
                    }
                    (start + len - (step % len)) % len
                }
            };
            if let Some(id) = self.playlist.track_id(index) {
                result.push(id);
            }
        }
        result
    }

    fn try_linear_candidates(&mut self, candidates: Vec<TrackId>, active: bool) -> bool {
        let mut attempted = false;
        for id in candidates {
            attempted = true;
            if self.try_load(id, active) {
                if self.playlist.select_track(id).is_some() {
                    return true;
                }
                self.engine = None;
                break;
            }
        }
        if attempted {
            self.stopped = true;
        }
        false
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
        if !self.advance(true) {
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
                stopped: self.stopped,
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

    #[test]
    fn linear_navigation_candidates_stop_or_wrap_exactly_once() {
        fn paths(player: &Player, ids: Vec<TrackId>) -> Vec<PathBuf> {
            ids.into_iter()
                .map(|id| player.playlist.path_for_id(id).unwrap().to_path_buf())
                .collect()
        }

        let mut player = Player::new(["a.mp3", "b.mp3", "c.mp3"].map(PathBuf::from).to_vec());
        player.playlist.select(1);

        assert_eq!(
            paths(
                &player,
                player.linear_candidates(1, Direction::Forward, false),
            ),
            ["c.mp3"].map(PathBuf::from)
        );
        assert_eq!(
            paths(
                &player,
                player.linear_candidates(1, Direction::Backward, false),
            ),
            ["a.mp3"].map(PathBuf::from)
        );

        player.playlist.set_repeat(true);
        assert_eq!(
            paths(
                &player,
                player.linear_candidates(1, Direction::Forward, false),
            ),
            ["c.mp3", "a.mp3", "b.mp3"].map(PathBuf::from),
            "repeat visits one full cycle and restarts current last"
        );
        assert_eq!(
            paths(
                &player,
                player.linear_candidates(1, Direction::Backward, false),
            ),
            ["a.mp3", "c.mp3", "b.mp3"].map(PathBuf::from)
        );
    }

    #[test]
    fn all_invalid_shuffle_with_repeat_is_bounded_at_start_and_next() {
        let paths = [
            "/definitely-missing/xubamp-broken-a.mp3",
            "/definitely-missing/xubamp-broken-b.mp3",
            "/definitely-missing/xubamp-broken-c.mp3",
        ]
        .map(PathBuf::from)
        .to_vec();
        let mut player = Player::with_settings(paths, true, true, EqSettings::default());

        player.start();
        assert!(player.engine.is_none());
        assert!(player.stopped);
        assert!(player.playback().stopped);

        player.transport(Transport::Next);
        assert!(player.engine.is_none());
        assert!(player.stopped);

        player.transport(Transport::Play);
        assert!(player.engine.is_none());
        assert!(player.playback().stopped);
    }

    #[test]
    fn append_paths_populates_an_empty_stopped_playlist_without_autoplay() {
        let mut player = Player::new(Vec::new());
        player.start();
        assert!(player.stopped);

        let added = player.append_paths([
            PathBuf::from("first.mp3"),
            PathBuf::from("movie.mp4"),
            PathBuf::from("first.mp3"),
            PathBuf::from("second.WAV"),
        ]);

        assert_eq!(added.len(), 3, "video is rejected and duplicates are kept");
        assert_eq!(player.playlist.current_id(), Some(added[0]));
        assert_eq!(player.playlist_view().1, Some(0));
        assert_eq!(
            player.playlist.tracks().collect::<Vec<_>>(),
            ["first.mp3", "first.mp3", "second.WAV"].map(Path::new)
        );
        assert!(
            player.engine.is_none(),
            "append does not construct a decoder"
        );
        assert!(
            player.stopped,
            "append preserves the stopped transport state"
        );
        assert!(!player.playback().playing);
    }

    #[test]
    fn append_paths_preserves_current_history_and_joins_the_shuffle_cycle() {
        let mut player = Player::with_settings(
            ["a.mp3", "b.wav", "c.mp3"].map(PathBuf::from).to_vec(),
            true,
            false,
            EqSettings::default(),
        );
        player.shuffle_cycle = ShuffleCycle::with_seed(0xA11D);
        player.shuffle_cycle.anchor(&player.playlist);
        let shuffled = player.shuffle_cycle.next(&player.playlist, false).unwrap();
        player.playlist.commit_forward(shuffled).unwrap();
        let current = player.playlist.current_id();
        let previous = player.playlist.back_candidate(None);
        let was_stopped = player.stopped;

        let added = player.append_paths([
            PathBuf::from("d.wav"),
            PathBuf::from("e.mp3"),
            PathBuf::from("notes.txt"),
        ]);

        assert_eq!(added.len(), 2);
        assert_eq!(player.playlist.current_id(), current);
        assert_eq!(player.playlist.back_candidate(None), previous);
        assert_eq!(player.stopped, was_stopped);
        assert!(player.engine.is_none());

        let mut pending = Vec::new();
        while let Some(id) = player.shuffle_cycle.next(&player.playlist, false) {
            pending.push(id);
        }
        for id in added {
            assert!(
                pending.contains(&id),
                "every newly appended stable ID joins the pending shuffle cycle"
            );
        }
        assert_eq!(
            pending.iter().copied().collect::<HashSet<_>>().len(),
            pending.len(),
            "playlist edits do not duplicate a pending shuffle member"
        );
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

    #[test]
    #[ignore = "needs a running PipeWire session"]
    fn next_from_stop_loads_the_destination_without_starting_it() {
        let a = std::env::temp_dir().join("xubamp_player_stopped_next_a.wav");
        let b = std::env::temp_dir().join("xubamp_player_stopped_next_b.wav");
        write_wav(&a, 48_000, 2);
        write_wav(&b, 48_000, 2);
        let mut player = Player::new(vec![a.clone(), b.clone()]);
        player.start();
        player.transport(Transport::Stop);
        player.transport(Transport::Next);

        let deadline = Instant::now() + Duration::from_secs(2);
        while player.playback().playing && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(player.playlist_view().1, Some(1));
        assert!(player.playback().stopped);
        assert!(!player.playback().playing);
        assert_eq!(player.playback().elapsed, Some(0));
        assert!(player.playback().position.unwrap_or(1.0) < 0.05);

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    #[ignore = "needs a running PipeWire session"]
    fn next_from_pause_starts_the_destination() {
        let a = std::env::temp_dir().join("xubamp_player_paused_next_a.wav");
        let b = std::env::temp_dir().join("xubamp_player_paused_next_b.wav");
        write_wav(&a, 48_000, 2);
        write_wav(&b, 48_000, 2);
        let mut player = Player::new(vec![a.clone(), b.clone()]);
        player.start();
        player.transport(Transport::Pause);
        player.transport(Transport::Next);

        let deadline = Instant::now() + Duration::from_secs(2);
        while !player.playback().playing && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(player.playlist_view().1, Some(1));
        assert!(!player.playback().stopped);
        assert!(player.playback().playing);

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    #[ignore = "needs a running PipeWire session"]
    fn next_from_a_naturally_finished_track_loads_stopped_before_poll() {
        let a = std::env::temp_dir().join("xubamp_player_finished_next_a.wav");
        let b = std::env::temp_dir().join("xubamp_player_finished_next_b.wav");
        write_wav(&a, 48_000, 1);
        write_wav(&b, 48_000, 2);
        let mut player = Player::new(vec![a.clone(), b.clone()]);
        player.start();

        let deadline = Instant::now() + Duration::from_secs(8);
        while !player.engine.as_ref().unwrap().is_finished() {
            assert!(Instant::now() < deadline, "first track never finished");
            thread::sleep(Duration::from_millis(20));
        }
        player.transport(Transport::Next);

        let deadline = Instant::now() + Duration::from_secs(2);
        while player.playback().playing && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(player.playlist_view().1, Some(1));
        assert!(player.playback().stopped);
        assert!(!player.playback().playing);
        assert_eq!(player.playback().elapsed, Some(0));
        assert!(player.playback().position.unwrap_or(1.0) < 0.05);

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    #[ignore = "needs a running PipeWire session"]
    fn shuffle_without_repeat_plays_one_permutation_then_stops() {
        let paths = ["a", "b", "c"]
            .map(|suffix| {
                std::env::temp_dir().join(format!("xubamp_player_shuffle_once_{suffix}.wav"))
            })
            .to_vec();
        for path in &paths {
            write_wav(path, 48_000, 1);
        }
        let mut player = Player::with_settings(paths.clone(), true, false, EqSettings::default());
        player.shuffle_cycle = ShuffleCycle::with_seed(0x0BAD_5EED);
        player.shuffle_cycle.anchor(&player.playlist);
        player.start();

        let mut visited = vec![player.playlist.current_id().unwrap()];
        let deadline = Instant::now() + Duration::from_secs(10);
        while !player.playback().stopped {
            player.poll();
            let current = player.playlist.current_id().unwrap();
            if visited.last().copied() != Some(current) {
                visited.push(current);
            }
            assert!(Instant::now() < deadline, "shuffle cycle never stopped");
            thread::sleep(Duration::from_millis(20));
        }

        assert_eq!(visited.len(), paths.len());
        assert_eq!(
            visited.iter().copied().collect::<HashSet<_>>().len(),
            paths.len()
        );
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    #[ignore = "needs a running PipeWire session"]
    fn shuffle_repeat_begins_a_new_cycle_without_replaying_in_place() {
        let paths = ["a", "b", "c"]
            .map(|suffix| {
                std::env::temp_dir().join(format!("xubamp_player_shuffle_repeat_{suffix}.wav"))
            })
            .to_vec();
        for path in &paths {
            write_wav(path, 48_000, 1);
        }
        let mut player = Player::with_settings(paths.clone(), true, true, EqSettings::default());
        player.shuffle_cycle = ShuffleCycle::with_seed(0x000C_1C1E);
        player.shuffle_cycle.anchor(&player.playlist);
        player.start();

        let mut visited = vec![player.playlist.current_id().unwrap()];
        let deadline = Instant::now() + Duration::from_secs(10);
        while visited.len() < paths.len() + 1 {
            player.poll();
            let current = player.playlist.current_id().unwrap();
            if visited.last().copied() != Some(current) {
                visited.push(current);
            }
            assert!(
                Instant::now() < deadline,
                "second shuffle cycle never began"
            );
            thread::sleep(Duration::from_millis(20));
        }
        player.transport(Transport::Stop);

        assert_eq!(
            visited[..paths.len()]
                .iter()
                .copied()
                .collect::<HashSet<_>>()
                .len(),
            paths.len()
        );
        assert_ne!(visited[paths.len() - 1], visited[paths.len()]);
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    }
}
