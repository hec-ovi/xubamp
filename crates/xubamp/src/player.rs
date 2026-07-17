//! The player: ties the playlist to the audio engine. It owns the current [`AudioEngine`] and the
//! [`Playlist`], and switching tracks drops the old engine and starts a fresh one (each track gets
//! its own PipeWire stream at its native rate, so no resampler is needed to move between tracks of
//! different rates). It lives on the main/UI thread, so it needs no locking.

use std::collections::{HashMap, HashSet};
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

/// A nonzero seed for the one-shot Randomize shuffle, taken from the wall clock so successive
/// randomizes differ. Only drives cosmetic playlist ordering, never anything reproducible.
fn random_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
        | 1
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
    /// Header-probed track lengths in seconds, keyed by path, so the playlist window can show each
    /// row's duration and the selected/total running time without decoding. Filled as tracks are
    /// added; a path that has no header length simply stays absent (shown blank).
    durations: HashMap<PathBuf, u32>,
    /// Tag-derived display names (`Artist - Title`), keyed by path, probed alongside the
    /// durations. A path with no usable tags stays absent and shows its file stem instead.
    names: HashMap<PathBuf, String>,
    /// Lowercased searchable text per path: every tag value (artist, album, composer, genre,
    /// year, comment, ...) plus the file name, for the Jump dialog.
    search: HashMap<PathBuf, String>,
    /// The classic Options-page behaviours.
    options: PlayerOptions,
}

/// The classic Options-page behaviours the player honors, restored from settings and updated
/// live from the Preferences window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerOptions {
    /// Read titles (tags and durations) when tracks are added; off defers to first play.
    pub read_titles_on_load: bool,
    /// Sort each added batch of files alphabetically before appending.
    pub sort_on_load: bool,
    /// Stop at the end of a track instead of advancing.
    pub manual_advance: bool,
    /// Prefix playlist rows with their 1-based number.
    pub playlist_numbers: bool,
    /// Show underscores / `%20` in filename-derived titles as spaces.
    pub convert_underscores: bool,
    pub convert_percent20: bool,
}

impl Default for PlayerOptions {
    fn default() -> Self {
        Self {
            read_titles_on_load: true,
            sort_on_load: false,
            manual_advance: false,
            playlist_numbers: true,
            convert_underscores: false,
            convert_percent20: false,
        }
    }
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
            durations: HashMap::new(),
            names: HashMap::new(),
            search: HashMap::new(),
            options: PlayerOptions::default(),
        };
        player.append_paths(tracks);
        player.shuffle_cycle.anchor(&player.playlist);
        player
    }

    /// Construct a player from persisted playback and equalizer settings without starting audio.
    /// Delaying playback until [`Self::start`] lets startup finish restoring all state first.
    /// Production goes through [`Self::with_settings_and_options`]; the tests keep this shorthand.
    #[cfg(test)]
    pub fn with_settings(
        tracks: Vec<PathBuf>,
        shuffle: bool,
        repeat: bool,
        shuffle_morph_rate: u8,
        equalizer: EqSettings,
    ) -> Self {
        Self::with_settings_and_options(
            tracks,
            shuffle,
            repeat,
            shuffle_morph_rate,
            equalizer,
            PlayerOptions::default(),
        )
    }

    /// [`Self::with_settings`] plus the Options-page behaviours, applied before the initial
    /// tracks are appended so read-titles-on-play defers their probing too.
    pub fn with_settings_and_options(
        tracks: Vec<PathBuf>,
        shuffle: bool,
        repeat: bool,
        shuffle_morph_rate: u8,
        equalizer: EqSettings,
        options: PlayerOptions,
    ) -> Self {
        let mut player = Self::new(Vec::new());
        player.options = options;
        player.append_paths(tracks);
        player.shuffle_cycle.anchor(&player.playlist);
        player.shuffle = shuffle;
        player.playlist.set_repeat(repeat);
        player.set_shuffle_morph_rate(shuffle_morph_rate);
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
        let mut accepted: Vec<PathBuf> =
            paths.into_iter().filter(|path| is_audio_path(path)).collect();
        if self.options.sort_on_load {
            sort_batch(&mut accepted);
        }
        if self.options.read_titles_on_load {
            self.probe_durations(&accepted);
        }
        self.playlist.extend(accepted)
    }

    /// Header-probe any not-yet-known paths and cache their lengths and tag names for the playlist
    /// and marquee. Errors, headerless, and tagless files are simply skipped, so one bad file never
    /// blocks the rest.
    fn probe_durations(&mut self, paths: &[PathBuf]) {
        for path in paths {
            if !self.durations.contains_key(path) {
                if let Some(secs) = xubamp_audio::decode::probe_duration_secs(path) {
                    self.durations.insert(path.clone(), secs);
                }
            }
            if !self.names.contains_key(path) && !self.search.contains_key(path) {
                if let Some(tags) = xubamp_audio::decode::probe_tags(path) {
                    if let Some(name) = tags.display_name() {
                        self.names.insert(path.clone(), name);
                    }
                    if !tags.all_text.is_empty() {
                        let mut haystack = tags.all_text.to_lowercase();
                        let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase());
                        if let Some(name) = name {
                            haystack.push(' ');
                            haystack.push_str(&name);
                        }
                        self.search.insert(path.clone(), haystack);
                    }
                }
            }
        }
    }

    /// A track's display name: its tag-derived `Artist - Title` when the file carries tags, else
    /// the file stem, Winamp's classic fallback.
    fn display_name(&self, path: &Path) -> String {
        if let Some(name) = self.names.get(path) {
            return name.clone();
        }
        let mut stem = track_title(&path.to_string_lossy());
        if self.options.convert_percent20 {
            stem = stem.replace("%20", " ");
        }
        if self.options.convert_underscores {
            stem = stem.replace('_', " ");
        }
        stem
    }

    /// Update the Options-page behaviours live. Turning read-titles back to on-load probes
    /// everything still unknown so the playlist fills in.
    pub fn set_options(&mut self, options: PlayerOptions) {
        let probe_all = options.read_titles_on_load && !self.options.read_titles_on_load;
        self.options = options;
        if probe_all {
            let paths: Vec<PathBuf> = self.playlist.tracks().map(Path::to_path_buf).collect();
            self.probe_durations(&paths);
        }
    }

    /// The current Options-page behaviours (for folding one change back in).
    pub fn options(&self) -> PlayerOptions {
        self.options
    }

    /// The currently loaded track's path, if any.
    pub fn current_path(&self) -> Option<PathBuf> {
        self.playlist.current().map(Path::to_path_buf)
    }

    /// The current track's display index, for session persistence.
    pub fn current_index(&self) -> Option<usize> {
        self.playlist.current_index()
    }

    /// Load a saved session's playlist exactly as it was left. The sidecar file records the order
    /// the user arranged (sorted or not), so sort-on-load must not reorder it here: that would
    /// silently shift which track the saved current-row index points at. Titles and durations
    /// still probe per the read-titles option.
    pub fn restore_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let accepted: Vec<PathBuf> =
            paths.into_iter().filter(|path| is_audio_path(path)).collect();
        if self.options.read_titles_on_load {
            self.probe_durations(&accepted);
        }
        self.playlist.extend(accepted);
        self.shuffle_cycle.anchor(&self.playlist);
    }

    /// Select playlist row `index` without touching playback: the session-restore path, so the
    /// remembered track is highlighted and armed. Whether playback then resumes is the caller's
    /// call (it does when the previous session ended while playing).
    pub fn restore_selection(&mut self, index: usize) {
        let Some((id, _)) = self.playlist.entries().nth(index).map(|(id, p)| (id, p.to_owned()))
        else {
            return;
        };
        self.playlist.select_track(id);
        self.stopped = true;
    }

    /// The path of playlist row `index`, if it exists.
    pub fn track_path(&self, index: usize) -> Option<PathBuf> {
        self.playlist.tracks().nth(index).map(Path::to_path_buf)
    }

    /// Drop and re-probe one track's cached duration and tag name (its file just changed, e.g.
    /// the file-info box wrote a tag), so the playlist rows and marquee pick up the new values.
    pub fn refresh_metadata(&mut self, path: &Path) {
        self.durations.remove(path);
        self.names.remove(path);
        self.search.remove(path);
        self.probe_durations(std::slice::from_ref(&path.to_path_buf()));
    }

    /// Replace the playlist with supported local audio paths, preserving player-wide modes,
    /// volume, balance, and equalizer settings. The accepted first entry becomes current but is not
    /// decoded until [`Self::start`], allowing a cancelled or invalid picker result to leave the
    /// existing playlist untouched.
    pub fn replace_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> usize {
        let mut accepted: Vec<_> = paths
            .into_iter()
            .filter(|path| is_audio_path(path))
            .collect();
        if accepted.is_empty() {
            return 0;
        }
        if self.options.sort_on_load {
            sort_batch(&mut accepted);
        }
        if self.options.read_titles_on_load {
            self.probe_durations(&accepted);
        }
        let repeat = self.playlist.repeat();
        self.engine = None;
        self.playlist = Playlist::new(accepted);
        self.playlist.set_repeat(repeat);
        self.shuffle_cycle.anchor(&self.playlist);
        self.stopped = true;
        self.playlist.len()
    }

    pub fn is_empty(&self) -> bool {
        self.playlist.is_empty()
    }

    /// Remove the playlist rows at the given display indices (editor "Remove Selected" / Del).
    pub fn remove_indices(&mut self, indices: &[usize]) {
        let ids: Vec<TrackId> = indices
            .iter()
            .filter_map(|&i| self.playlist.track_id(i))
            .collect();
        self.playlist.remove_ids(&ids);
    }

    /// Keep only the rows at the given display indices, removing the rest (editor "Crop").
    pub fn crop_indices(&mut self, indices: &[usize]) {
        let ids: Vec<TrackId> = indices
            .iter()
            .filter_map(|&i| self.playlist.track_id(i))
            .collect();
        self.playlist.retain_ids(&ids);
    }

    /// Clear the whole playlist and stop playback (editor "New List" / "Remove All").
    pub fn clear_playlist(&mut self) {
        self.engine = None;
        self.playlist.clear();
        self.shuffle_cycle.clear();
        self.stopped = true;
    }

    /// Drop entries whose file no longer exists on disk (editor "Remove all dead files").
    pub fn remove_dead_tracks(&mut self) {
        let dead: Vec<TrackId> = self
            .playlist
            .entries()
            .filter(|(_, path)| !path.exists())
            .map(|(id, _)| id)
            .collect();
        self.playlist.remove_ids(&dead);
    }

    /// Stable id and path of every entry in display order, so the editor can compute a sort key
    /// (title / filename / path) without reaching into the playlist internals.
    pub fn playlist_entries(&self) -> Vec<(TrackId, PathBuf)> {
        self.playlist
            .entries()
            .map(|(id, path)| (id, path.to_owned()))
            .collect()
    }

    /// Reorder the playlist to a caller-computed permutation of stable ids (the sort primitive).
    pub fn reorder_playlist(&mut self, order: &[TrackId]) {
        self.playlist.set_order(order);
    }

    /// Reorder the playlist to a permutation given as display indices (the editor's
    /// drag-to-reorder): the entry shown at `order[i]` moves to row `i`. Out-of-range indices
    /// are skipped, and any entries not named keep their order after the named ones.
    pub fn reorder_indices(&mut self, order: &[usize]) {
        let ids: Vec<TrackId> = order
            .iter()
            .filter_map(|&i| self.playlist.track_id(i))
            .collect();
        self.playlist.set_order(&ids);
    }

    /// Reverse the playlist display order (editor "Reverse list").
    pub fn reverse_playlist(&mut self) {
        self.playlist.reverse();
    }

    /// Randomly shuffle the playlist display order (editor "Randomize list"). This changes the shown
    /// order, distinct from Shuffle-mode playback which keeps its own permutation.
    pub fn randomize_playlist(&mut self) {
        let mut ids: Vec<TrackId> = self.playlist.ids().collect();
        let mut x = random_seed();
        for i in (1..ids.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let j = (x % (i as u64 + 1)) as usize;
            ids.swap(i, j);
        }
        self.playlist.set_order(&ids);
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
        // Read-titles-on-play: a deferred title is read the first time its track loads.
        self.probe_durations(std::slice::from_ref(&path));
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
    /// Eject is intercepted by the application layer to open the desktop file chooser.
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
            Transport::Eject => {}
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
            // Retrace real history first; with none left, original Winamp's shuffle Previous
            // jumps to a random other track, and the jump is committed like a Back so Next
            // replays forward from there.
            let fresh = self.random_back_fresh(&attempted);
            let Some(id) = self.playlist.back_candidate(fresh) else {
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

    /// A random non-current, not-yet-attempted track, the shuffle Previous fallback when there
    /// is no history left to retrace. `None` on a single-track (or exhausted) list.
    fn random_back_fresh(&self, attempted: &HashSet<TrackId>) -> Option<TrackId> {
        let current = self
            .playlist
            .current_index()
            .and_then(|index| self.playlist.track_id(index));
        let candidates: Vec<TrackId> = self
            .playlist
            .entries()
            .map(|(id, _)| id)
            .filter(|id| Some(*id) != current && !attempted.contains(id))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        Some(candidates[random_seed() as usize % candidates.len()])
    }

    /// Play the playlist track at index `i` (a double-click in the playlist window). Remembers the
    /// current track for Back and clears the forward stack (a fresh navigation). No-op if the index
    /// is out of range.
    /// Jump `delta` entries from the current track (clamped to the ends) and play the landing
    /// track, matching Winamp's "10 tracks back/forward". A no-op on an empty playlist.
    pub fn skip_tracks(&mut self, delta: i32) {
        if let Some(target) =
            skip_target(self.playlist.current_index().unwrap_or(0), self.playlist.len(), delta)
        {
            self.play_index(target);
        }
    }

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

    /// Current repeat-boundary mutation rate. Updating it leaves the active shuffle permutation
    /// untouched and takes effect only when a repeated cycle is created. A symmetric accessor to
    /// [`Self::set_shuffle_morph_rate`], used by tests; the running app persists the rate from the
    /// Preferences sink and seeds the model from settings, so it does not read this back.
    #[allow(dead_code)]
    pub fn shuffle_morph_rate(&self) -> u8 {
        self.shuffle_cycle.morph_rate()
    }

    pub fn set_shuffle_morph_rate(&mut self, rate: u8) {
        self.shuffle_cycle.set_morph_rate(rate);
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
        if self.options.manual_advance {
            // Classic manual advance: a finished track parks like a hard playlist end.
            self.stopped = true;
            return;
        }
        if !self.advance(true) {
            // Natural end at a hard playlist boundary: leave the decoder and clock at the end. The
            // stopped marker prevents this poll from retrying forever; Play/Pause can restart it.
            self.stopped = true;
        }
    }

    /// The playlist rows (numbered tag names or file stems; durations arrive once tracks are
    /// probed) and the index of the currently-playing track, for the playlist window to render.
    pub fn playlist_view(&self) -> (Vec<pledit::Row>, Option<usize>) {
        let rows = self
            .playlist
            .tracks()
            .enumerate()
            .map(|(i, path)| {
                let duration_secs = self.durations.get(path).copied();
                pledit::Row {
                    title: if self.options.playlist_numbers {
                        format!("{}. {}", i + 1, self.display_name(path))
                    } else {
                        self.display_name(path)
                    },
                    duration: duration_secs.map(fmt_mmss).unwrap_or_default(),
                    duration_secs,
                    search: self.search.get(path).cloned().unwrap_or_else(|| {
                        path.file_name()
                            .map(|n| n.to_string_lossy().to_lowercase())
                            .unwrap_or_default()
                    }),
                }
            })
            .collect();
        (rows, self.playlist.current_index())
    }

    /// The current track's marquee title in the classic Winamp format
    /// `N. Artist - Title (M:SS)`: the 1-based playlist number, the tag name (or file-stem
    /// fallback), and the track length when its header carries one. Empty when nothing is loaded.
    pub fn title(&self) -> String {
        let Some(path) = self.playlist.current() else {
            return String::new();
        };
        let number = self.playlist.current_index().map_or(0, |i| i + 1);
        let name = self.display_name(path);
        match self.durations.get(path) {
            Some(&secs) => format!("{}. {} ({})", number, name, fmt_mmss(secs)),
            None => format!("{number}. {name}"),
        }
    }

    /// A clock + track-info snapshot for the window's redraw tick.
    pub fn playback(&self) -> Playback {
        match &self.engine {
            Some(engine) => {
                let h = engine.handle();
                // Once a track has played out at a hard playlist end, show 0:00 with the seek bar
                // rewound rather than parked at the end. The decoder itself stays at the end (the
                // next Play restarts from the top), so this is a display-only reset with no seek of
                // a drained ring. Auto-advance replaces the finished engine before this is read, so
                // `is_finished` is only true at a real end-of-playlist stop.
                let finished = h.is_finished();
                let stopped = self.stopped || finished;
                Playback {
                    // A stopped (or naturally finished) deck shows a BLANK clock, like classic
                    // Winamp: 0:00 after a Stop or a removed track reads as a lie. The seek thumb
                    // still rests at the start.
                    elapsed: if stopped { None } else { Some(h.elapsed_secs()) },
                    position: if stopped { Some(0.0) } else { h.position_fraction() },
                    duration: h.duration_secs(),
                    playing: h.is_playing(),
                    // A naturally finished track reads as stopped (classic "ended" state), so the
                    // status indicator shows stop and the visualizer settles, not a paused freeze.
                    stopped,
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

/// The clamped landing index for a `skip_tracks(delta)` from `current`, or `None` on an empty list.
/// Pure so the boundary arithmetic is unit-tested without a real engine.
fn skip_target(current: usize, len: usize, delta: i32) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some((current as i32 + delta).clamp(0, len as i32 - 1) as usize)
}

/// Sort one added batch of paths by file name (case-insensitive), the classic sort-on-load.
fn sort_batch(paths: &mut [PathBuf]) {
    paths.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
}

/// Format a whole-second track length as classic Winamp M:SS (minutes are not zero-padded).
fn fmt_mmss(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// End-to-end tests of the player against the real PipeWire-backed engine: they play short WAVs to
/// their end and assert the playlist behaviour at the boundary. Ignored by default (they need a
/// running PipeWire session); in the dev container route them to a silent sink to stay quiet:
///   cargo test -p xubamp --features audio player -- --ignored --nocapture
/// (set PIPEWIRE_NODE=<null-sink> first for silence.)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_mmss_matches_classic_winamp() {
        assert_eq!(fmt_mmss(0), "0:00");
        assert_eq!(fmt_mmss(9), "0:09");
        assert_eq!(fmt_mmss(65), "1:05");
        assert_eq!(fmt_mmss(600), "10:00");
        assert_eq!(fmt_mmss(3599), "59:59");
    }

    #[test]
    fn playlist_view_shows_probed_durations() {
        let dir = std::env::temp_dir().join(format!("xubamp-dur-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        write_wav(&path, 48_000, 2); // ~2 seconds at 48 kHz
        let player = Player::new(vec![path.clone()]);
        let (rows, _) = player.playlist_view();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].duration_secs, Some(2), "header duration probed");
        assert_eq!(rows[0].duration, "0:02");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shuffle_previous_without_history_offers_a_random_other_track() {
        let dir = std::env::temp_dir().join(format!("xubamp-prev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<PathBuf> = (0..3)
            .map(|i| {
                let p = dir.join(format!("t{i}.wav"));
                write_wav(&p, 48_000, 1);
                p
            })
            .collect();
        let mut player = Player::new(paths);
        player.shuffle = true;
        let current = player
            .playlist
            .current_index()
            .and_then(|i| player.playlist.track_id(i));
        // With no history, the Previous fallback offers a random track that is never the
        // current one, and the pool respects already-attempted candidates.
        let mut attempted = HashSet::new();
        for _ in 0..64 {
            if attempted.len() == 2 {
                break;
            }
            let fresh = player.random_back_fresh(&attempted).expect("a candidate");
            assert_ne!(Some(fresh), current, "never re-picks the current track");
            attempted.insert(fresh);
        }
        assert_eq!(attempted.len(), 2, "both other tracks eventually offered");
        assert_eq!(
            player.random_back_fresh(&attempted),
            None,
            "exhausted pool offers nothing"
        );
        // And the playlist commits such a fresh candidate like a Back: current becomes a
        // Forward redo, so Next replays forward from the jump.
        let fresh = *attempted.iter().next().unwrap();
        assert_eq!(player.playlist.back_candidate(Some(fresh)), Some(fresh));
        player.playlist.commit_back(fresh).expect("selects the jump");
        assert_eq!(
            player.playlist.current_index().and_then(|i| player.playlist.track_id(i)),
            Some(fresh)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_paths_keeps_the_sidecar_order_despite_sort_on_load() {
        let dir = std::env::temp_dir().join(format!("xubamp-restore-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let names = ["zebra.wav", "alpha.wav", "middle.wav"];
        let paths: Vec<PathBuf> = names
            .iter()
            .map(|n| {
                let p = dir.join(n);
                write_wav(&p, 48_000, 1);
                p
            })
            .collect();
        let mut player = Player::with_settings_and_options(
            Vec::new(),
            false,
            false,
            50,
            EqSettings::default(),
            PlayerOptions {
                sort_on_load: true,
                ..PlayerOptions::default()
            },
        );
        player.restore_paths(paths.clone());
        for (i, expected) in paths.iter().enumerate() {
            assert_eq!(
                player.track_path(i).as_ref(),
                Some(expected),
                "row {i} keeps the saved order, not the alphabetical one"
            );
        }
        // The saved index therefore still points at the same song, and resume plays it.
        player.restore_selection(0);
        player.start();
        assert_eq!(player.current_path(), Some(paths[0].clone()));
        assert!(!player.playback().stopped, "session resume leaves the stopped state");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn play_after_restore_starts_the_armed_track() {
        let dir = std::env::temp_dir().join(format!("xubamp-restore-play-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<PathBuf> = (0..6)
            .map(|i| {
                let p = dir.join(format!("t{i}.wav"));
                write_wav(&p, 48_000, 1);
                p
            })
            .collect();
        let mut player = Player::new(paths.clone());
        player.restore_selection(1);
        player.transport(Transport::Play);
        // `playing` is published by the PipeWire thread, absent in the test container, so the
        // deck state and the loaded path are what this can assert.
        let pb = player.playback();
        assert!(!pb.stopped, "play after restore leaves the stopped state");
        assert_eq!(
            player.current_path(),
            Some(paths[1].clone()),
            "the armed row is what plays, not the first row"
        );

        // The same restore with shuffle and repeat active (a real session's settings): the armed
        // row must still be what the first Play starts, not a shuffle pick.
        let mut player = Player::with_settings_and_options(
            paths.clone(),
            true,
            true,
            50,
            EqSettings::default(),
            PlayerOptions::default(),
        );
        player.restore_selection(4);
        player.transport(Transport::Play);
        assert!(!player.playback().stopped);
        assert_eq!(
            player.current_path(),
            Some(paths[4].clone()),
            "shuffle does not steal the first Play from the armed row"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_selection_arms_a_row_without_playing() {
        let dir = std::env::temp_dir().join(format!("xubamp-restore-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<PathBuf> = (0..3)
            .map(|i| {
                let p = dir.join(format!("t{i}.wav"));
                write_wav(&p, 48_000, 1);
                p
            })
            .collect();
        let mut player = Player::new(paths.clone());
        player.restore_selection(2);
        assert_eq!(player.current_index(), Some(2));
        assert_eq!(player.current_path(), Some(paths[2].clone()));
        let pb = player.playback();
        assert!(pb.stopped && !pb.playing, "restored session stays stopped");
        assert_eq!(pb.elapsed, None, "stopped deck shows a blank clock");
        // An out-of-range index leaves the selection alone.
        player.restore_selection(99);
        assert_eq!(player.current_index(), Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_sort_numbers_conversions_and_deferred_titles_apply() {
        let dir = std::env::temp_dir().join(format!("xubamp-opts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let later = dir.join("b_track%20x.wav");
        let earlier = dir.join("a song.wav");
        write_wav(&later, 48_000, 1);
        write_wav(&earlier, 48_000, 1);

        let options = PlayerOptions {
            read_titles_on_load: false,
            sort_on_load: true,
            playlist_numbers: false,
            convert_underscores: true,
            convert_percent20: true,
            ..Default::default()
        };
        let mut player = Player::with_settings_and_options(
            vec![later.clone(), earlier.clone()],
            false,
            false,
            xubamp_config::DEFAULT_SHUFFLE_MORPH_RATE,
            EqSettings::default(),
            options,
        );
        let (rows, _) = player.playlist_view();
        assert_eq!(rows[0].title, "a song", "sorted, unnumbered");
        assert_eq!(
            rows[1].title, "b track x",
            "underscores and %20 read as spaces"
        );
        assert_eq!(rows[0].duration_secs, None, "read-on-play defers probing");

        // Flipping back to read-on-load probes everything still unknown.
        player.set_options(PlayerOptions {
            read_titles_on_load: true,
            ..options
        });
        let (rows, _) = player.playlist_view();
        assert_eq!(rows[0].duration_secs, Some(1));
        assert_eq!(rows[1].duration_secs, Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Splice a RIFF INFO LIST chunk (IART + INAM) into a `write_wav` file, in front of its data
    /// chunk (byte 36 of the fixed header layout), so tag probing has something to read.
    fn tag_wav(path: &Path, artist: &str, title: &str) {
        fn info_entry(id: &[u8; 4], text: &str) -> Vec<u8> {
            let mut z = text.as_bytes().to_vec();
            z.push(0);
            if z.len() % 2 == 1 {
                z.push(0);
            }
            let mut e = id.to_vec();
            e.extend_from_slice(&(z.len() as u32).to_le_bytes());
            e.extend_from_slice(&z);
            e
        }
        let mut wav = std::fs::read(path).unwrap();
        let mut list = b"INFO".to_vec();
        list.extend_from_slice(&info_entry(b"IART", artist));
        list.extend_from_slice(&info_entry(b"INAM", title));
        let mut chunk = b"LIST".to_vec();
        chunk.extend_from_slice(&(list.len() as u32).to_le_bytes());
        chunk.extend_from_slice(&list);
        wav.splice(36..36, chunk);
        let riff_len = (wav.len() - 8) as u32;
        wav[4..8].copy_from_slice(&riff_len.to_le_bytes());
        std::fs::write(path, wav).unwrap();
    }

    #[test]
    fn marquee_and_rows_use_tag_names_with_the_classic_format() {
        let dir = std::env::temp_dir().join(format!("xubamp-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tagged = dir.join("tagged.wav");
        write_wav(&tagged, 48_000, 2);
        tag_wav(&tagged, "Aphex Twin", "Xtal");
        let plain = dir.join("plain stem.wav");
        write_wav(&plain, 48_000, 3);

        let player = Player::new(vec![tagged, plain]);
        assert_eq!(
            player.title(),
            "1. Aphex Twin - Xtal (0:02)",
            "marquee: number, tag name, and length"
        );
        let (rows, current) = player.playlist_view();
        assert_eq!(current, Some(0));
        assert_eq!(rows[0].title, "1. Aphex Twin - Xtal");
        assert_eq!(
            rows[1].title, "2. plain stem",
            "tagless file falls back to its stem"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_a_tag_and_refreshing_updates_the_playlist_row() {
        let dir = std::env::temp_dir().join(format!("xubamp-tagsave-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cruel summer.wav");
        write_wav(&path, 48_000, 2);
        let mut player = Player::new(vec![path.clone()]);
        let (rows, _) = player.playlist_view();
        assert_eq!(rows[0].title, "1. cruel summer", "untagged file shows its stem");

        // What the file-info box does on save: write the ID3v1 tag, then refresh the caches.
        xubamp_audio::id3v1::write(
            &path,
            &xubamp_audio::id3v1::Id3v1 {
                title: "Cruel Summer".to_owned(),
                artist: "Ace of Base".to_owned(),
                ..Default::default()
            },
        )
        .unwrap();
        player.refresh_metadata(&path);
        let (rows, _) = player.playlist_view();
        assert_eq!(
            rows[0].title, "1. Ace of Base - Cruel Summer",
            "the row shows the freshly saved tag"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reorder_indices_permutes_rows_and_current_follows_its_track() {
        let mut player = Player::new(
            ["a", "b", "c", "d"]
                .iter()
                .map(|n| PathBuf::from(format!("/nowhere/{n}.wav")))
                .collect(),
        );
        assert_eq!(player.playlist_view().1, Some(0), "current starts at a");

        // The drag permutation the playlist editor emits: order[i] is the old display index.
        player.reorder_indices(&[2, 0, 1, 3]);
        let (rows, current) = player.playlist_view();
        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["1. c", "2. a", "3. b", "4. d"]);
        assert_eq!(current, Some(1), "current follows the track, not the row number");

        // Out-of-range indices are skipped rather than corrupting the order.
        player.reorder_indices(&[9, 3, 2, 1, 0]);
        let (rows, _) = player.playlist_view();
        assert_eq!(rows[0].title, "1. d");
        assert_eq!(rows[3].title, "4. c");
    }

    #[test]
    fn skip_target_clamps_to_the_playlist_ends() {
        assert_eq!(skip_target(2, 10, 10), Some(9), "forward clamps to the last");
        assert_eq!(skip_target(5, 10, -10), Some(0), "back clamps to the first");
        assert_eq!(skip_target(3, 10, 4), Some(7), "in range moves exactly");
        assert_eq!(skip_target(0, 10, -1), Some(0), "already at the start");
        assert_eq!(skip_target(0, 0, 10), None, "empty playlist is a no-op");
    }
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

        let player = Player::with_settings(Vec::new(), true, true, 17, equalizer);

        assert!(player.shuffle());
        assert!(player.repeat());
        assert_eq!(player.shuffle_morph_rate(), 17);
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
        assert_eq!(
            player.shuffle_morph_rate(),
            xubamp_config::DEFAULT_SHUFFLE_MORPH_RATE
        );
        assert_eq!(player.equalizer_settings(), EqSettings::default());

        player.set_shuffle_morph_rate(12);
        player.toggle_mode(ModeButton::Shuffle);
        player.toggle_mode(ModeButton::Repeat);
        assert!(player.shuffle());
        assert!(player.repeat());
        assert_eq!(
            player.shuffle_morph_rate(),
            12,
            "mode toggles do not reset the independent morph preference"
        );
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
        let mut player = Player::with_settings(
            paths,
            true,
            true,
            xubamp_config::DEFAULT_SHUFFLE_MORPH_RATE,
            EqSettings::default(),
        );

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
            xubamp_config::DEFAULT_SHUFFLE_MORPH_RATE,
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

    #[test]
    fn replace_paths_is_transactional_and_preserves_player_wide_settings() {
        let equalizer = EqSettings {
            enabled: false,
            preamp_db: 5.0,
            bands_db: [1.0; 10],
        };
        let mut player = Player::with_settings(
            ["old-a.mp3", "old-b.wav"].map(PathBuf::from).to_vec(),
            true,
            true,
            23,
            equalizer,
        );
        player.set_volume(37);
        player.set_balance(-22);

        assert_eq!(player.replace_paths([PathBuf::from("movie.mp4")]), 0);
        assert_eq!(
            player.playlist.tracks().collect::<Vec<_>>(),
            ["old-a.mp3", "old-b.wav"].map(Path::new),
            "an invalid picker result leaves the old playlist intact"
        );

        assert_eq!(
            player.replace_paths([
                PathBuf::from("new-a.WAV"),
                PathBuf::from("notes.txt"),
                PathBuf::from("new-b.mp3"),
            ]),
            2
        );
        assert_eq!(
            player.playlist.tracks().collect::<Vec<_>>(),
            ["new-a.WAV", "new-b.mp3"].map(Path::new)
        );
        assert_eq!(player.playlist.current_index(), Some(0));
        assert!(player.shuffle());
        assert!(player.repeat());
        assert_eq!(player.shuffle_morph_rate(), 23);
        assert_eq!(player.equalizer_settings(), equalizer);
        assert_eq!(player.volume, 37);
        assert_eq!(player.balance, -22);
        assert!(player.engine.is_none());
        assert!(player.stopped);
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
    fn a_single_track_stops_and_resets_the_shown_clock_at_the_end() {
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

        // The hard end is handled once. The decoder stays parked at the end (the next Play restarts
        // from the top), but the shown clock and seek bar reset to zero rather than sitting at the
        // end, so a finished playlist reads as 0:00 with the thumb rewound.
        thread::sleep(Duration::from_millis(400));
        player.poll();
        let pb = player.playback();
        assert!(!pb.playing, "stopped at the end, not left playing");
        assert!(pb.stopped, "the end-of-playlist stop holds");
        assert_eq!(
            pb.position,
            Some(0.0),
            "the seek bar resets to the start on finish ({:?})",
            pb.position
        );
        assert_eq!(
            pb.elapsed,
            Some(0),
            "the clock resets to 0:00 on finish ({:?})",
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
        let mut player = Player::with_settings(
            paths.clone(),
            true,
            false,
            xubamp_config::DEFAULT_SHUFFLE_MORPH_RATE,
            EqSettings::default(),
        );
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
        let mut player = Player::with_settings(
            paths.clone(),
            true,
            true,
            xubamp_config::DEFAULT_SHUFFLE_MORPH_RATE,
            EqSettings::default(),
        );
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
