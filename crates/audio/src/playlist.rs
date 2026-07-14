//! The playlist: an ordered list of stable track identities plus the current position. Pure (no
//! audio), so mutation and navigation are unit-tested without a device. The player layer turns a
//! returned path into actual playback.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The most recent tracks remembered for Back/Forward navigation. Bounds memory over a long session
/// (older entries fall off the far end of the back history).
const HISTORY_MAX: usize = 256;

/// Identity of one playlist entry. IDs do not change when entries are inserted, removed, or moved,
/// and are never reused during a playlist's lifetime. Duplicate paths therefore remain distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrackId(u64);

impl TrackId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A shuffled, stable-ID traversal of a playlist.
///
/// The current track is the first member of a newly anchored cycle, so it is excluded from the
/// initial pending permutation. Every other live entry is returned at most once. When repeat is
/// enabled, exhausting the permutation starts a new one containing every live entry; its first
/// result avoids the track that just played whenever another choice exists. Removed IDs are
/// discarded, moved IDs keep their place in the cycle, and newly-added IDs join the pending set.
///
/// Randomness is deliberately self-contained rather than pulled from a global RNG. Supplying a
/// seed makes the navigation policy reproducible in tests while [`Self::from_entropy`] gives the
/// application a different order each run.
#[derive(Debug, Clone)]
pub struct ShuffleCycle {
    remaining: Vec<TrackId>,
    members: HashSet<TrackId>,
    rng: u64,
    anchored: bool,
}

impl ShuffleCycle {
    /// Construct a deterministic cycle. An all-zero seed is remapped because xorshift cannot
    /// escape zero.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            remaining: Vec::new(),
            members: HashSet::new(),
            rng: nonzero_seed(seed),
            anchored: false,
        }
    }

    /// Construct a cycle seeded from the wall clock.
    pub fn from_entropy() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};

        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self::with_seed(seed)
    }

    /// Start a fresh traversal with the playlist's current entry as its already-played anchor.
    pub fn anchor(&mut self, playlist: &Playlist) {
        self.anchor_at(playlist, playlist.current_id());
    }

    /// Forget pending traversal state. The next call to [`Self::next`] anchors at whatever is then
    /// current.
    pub fn clear(&mut self) {
        self.remaining.clear();
        self.members.clear();
        self.anchored = false;
    }

    /// Return the next stable track ID. With repeat off, an exhausted cycle stays exhausted. With
    /// repeat on, a new complete permutation begins.
    pub fn next(&mut self, playlist: &Playlist, repeat: bool) -> Option<TrackId> {
        if !self.anchored {
            self.anchor(playlist);
        }
        self.sync_edits(playlist);
        if let Some(id) = self.remaining.pop() {
            return Some(id);
        }
        if !repeat || playlist.is_empty() {
            return None;
        }

        self.members = playlist.ids().collect();
        self.remaining = playlist.ids().collect();
        self.shuffle_remaining();

        // `pop` is the next result. Keep the just-played track away from that position when there
        // is any alternative; a one-entry repeating playlist necessarily returns itself.
        if self.remaining.len() > 1 && self.remaining.last().copied() == playlist.current_id() {
            if let Some(other) = self
                .remaining
                .iter()
                .position(|id| Some(*id) != playlist.current_id())
            {
                let last = self.remaining.len() - 1;
                self.remaining.swap(other, last);
            }
        }
        self.remaining.pop()
    }

    /// Anchor at an explicit entry. This is used by a manual playlist jump after that entry has
    /// become current.
    pub fn anchor_at(&mut self, playlist: &Playlist, current: Option<TrackId>) {
        self.members = playlist.ids().collect();
        self.remaining = playlist.ids().filter(|id| Some(*id) != current).collect();
        self.shuffle_remaining();
        self.anchored = true;
    }

    fn sync_edits(&mut self, playlist: &Playlist) {
        let live: HashSet<_> = playlist.ids().collect();
        self.remaining.retain(|id| live.contains(id));
        self.members.retain(|id| live.contains(id));

        // If removing the current row caused Playlist to select its neighbor, that neighbor is now
        // the cycle anchor and must not be returned immediately as a pending choice.
        if let Some(current) = playlist.current_id() {
            self.remaining.retain(|id| *id != current);
            self.members.insert(current);
        }

        let old_len = self.remaining.len();
        for id in playlist.ids() {
            if self.members.insert(id) && Some(id) != playlist.current_id() {
                self.remaining.push(id);
            }
        }
        if self.remaining.len() != old_len {
            self.shuffle_remaining();
        }
    }

    fn shuffle_remaining(&mut self) {
        for i in (1..self.remaining.len()).rev() {
            let j = (self.rand() % (i as u64 + 1)) as usize;
            self.remaining.swap(i, j);
        }
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
}

impl Default for ShuffleCycle {
    fn default() -> Self {
        Self::from_entropy()
    }
}

const fn nonzero_seed(seed: u64) -> u64 {
    if seed == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        seed
    }
}

#[derive(Debug, Clone)]
struct Track {
    id: TrackId,
    path: PathBuf,
}

/// An ordered list of tracks with a current selection.
#[derive(Debug, Default, Clone)]
pub struct Playlist {
    tracks: Vec<Track>,
    /// Stable identity of the current track, or `None` when the list is empty.
    current: Option<TrackId>,
    /// When set, `next`/`prev` wrap around the ends instead of stopping (repeat-all).
    repeat: bool,
    /// Back stack: previously-played stable IDs, most recent last. `back` returns to these, so Prev
    /// retraces the real play order even after playlist edits and under shuffle.
    history: Vec<TrackId>,
    /// Forward stack: tracks stepped away from by `back`, so a following `forward` redoes them
    /// (browser-style). Cleared by a fresh jump (double-click / jump-to-file).
    future: Vec<TrackId>,
    next_id: u64,
}

impl Playlist {
    /// Build a playlist from `tracks`; the first track (if any) becomes current.
    pub fn new(tracks: Vec<PathBuf>) -> Self {
        let mut playlist = Self::default();
        playlist.extend(tracks);
        playlist
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// The track paths in display order.
    pub fn tracks(&self) -> impl ExactSizeIterator<Item = &Path> + DoubleEndedIterator {
        self.tracks.iter().map(|track| track.path.as_path())
    }

    /// Index of the current track, or `None` when the list is empty.
    pub fn current_index(&self) -> Option<usize> {
        self.current.and_then(|id| self.index_of(id))
    }

    /// Stable identity of the current track.
    pub fn current_id(&self) -> Option<TrackId> {
        self.current
    }

    /// Stable identity of the entry currently at `index`.
    pub fn track_id(&self, index: usize) -> Option<TrackId> {
        self.tracks.get(index).map(|track| track.id)
    }

    /// All stable IDs in display order.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = TrackId> + DoubleEndedIterator + '_ {
        self.tracks.iter().map(|track| track.id)
    }

    /// The display index of a stable ID, or `None` if that entry has been removed.
    pub fn index_of_id(&self, id: TrackId) -> Option<usize> {
        self.index_of(id)
    }

    /// The path belonging to a stable ID, or `None` if that entry has been removed.
    pub fn path_for_id(&self, id: TrackId) -> Option<&Path> {
        self.track(id).map(|track| track.path.as_path())
    }

    /// The current track's path, or `None` when the list is empty.
    pub fn current(&self) -> Option<&Path> {
        self.current
            .and_then(|id| self.track(id))
            .map(|track| track.path.as_path())
    }

    /// Append one entry, selecting it only when the playlist was empty.
    pub fn add(&mut self, path: PathBuf) -> TrackId {
        let id = self.allocate_id();
        self.tracks.push(Track { id, path });
        self.current.get_or_insert(id);
        id
    }

    /// Append entries in input order.
    pub fn extend(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            self.add(path);
        }
    }

    /// Insert an entry before `index`. `index == len` appends; larger indices are rejected.
    pub fn insert(&mut self, index: usize, path: PathBuf) -> Option<TrackId> {
        if index > self.tracks.len() {
            return None;
        }
        let id = self.allocate_id();
        self.tracks.insert(index, Track { id, path });
        self.current.get_or_insert(id);
        Some(id)
    }

    /// Remove the entry at `index`. If it was current, the following entry becomes current, or the
    /// previous final entry when the removed item was last. Navigation history forgets only the
    /// removed identity; every surviving entry keeps its meaning.
    pub fn remove(&mut self, index: usize) -> Option<PathBuf> {
        if index >= self.tracks.len() {
            return None;
        }
        let removed = self.tracks.remove(index);
        self.history.retain(|&id| id != removed.id);
        self.future.retain(|&id| id != removed.id);
        if self.current == Some(removed.id) {
            self.current = self
                .tracks
                .get(index)
                .or_else(|| self.tracks.last())
                .map(|track| track.id);
        }
        Some(removed.path)
    }

    /// Remove all entries without reusing their stable IDs if more tracks are added later.
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current = None;
        self.clear_history();
    }

    /// Move one entry to its final display index. The current entry and navigation history follow
    /// its stable identity rather than whichever track takes over the old index.
    pub fn move_track(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tracks.len() || to >= self.tracks.len() {
            return false;
        }
        if from != to {
            let track = self.tracks.remove(from);
            self.tracks.insert(to, track);
        }
        true
    }

    /// Whether repeat-all is on.
    pub fn repeat(&self) -> bool {
        self.repeat
    }

    /// Turn repeat-all on/off. When on, `next`/`prev` wrap around the ends.
    pub fn set_repeat(&mut self, on: bool) {
        self.repeat = on;
    }

    /// Advance to the next track and return it. At the end of the list this returns `None` (the
    /// caller stops), or wraps to the first track when repeat is on.
    #[allow(clippy::should_implement_trait)] // a playlist "next", not an iterator
    pub fn next(&mut self) -> Option<PathBuf> {
        let i = self.current_index()?;
        let n = self.tracks.len();
        let j = if i + 1 < n {
            Some(i + 1)
        } else if self.repeat {
            Some(0)
        } else {
            None
        };
        j.and_then(|j| self.select(j))
    }

    /// Go to the previous track and return it. At the start this returns `None` (stays put), or
    /// wraps to the last track when repeat is on.
    pub fn prev(&mut self) -> Option<PathBuf> {
        let i = self.current_index()?;
        let n = self.tracks.len();
        let j = if i > 0 {
            Some(i - 1)
        } else if self.repeat {
            Some(n - 1)
        } else {
            None
        };
        j.and_then(|j| self.select(j))
    }

    /// Select track `i` and return it, or `None` if `i` is out of range (selection unchanged).
    pub fn select(&mut self, i: usize) -> Option<PathBuf> {
        let track = self.tracks.get(i)?;
        self.current = Some(track.id);
        Some(track.path.clone())
    }

    /// Select a stable track identity without recording navigation history.
    pub fn select_track(&mut self, id: TrackId) -> Option<PathBuf> {
        self.select_id(id)
    }

    /// The next index in linear order (wraps to the first with repeat), without moving. `None` at a
    /// hard end. The player passes this (or a shuffle-chosen index) to [`Self::forward`].
    pub fn peek_next(&self) -> Option<usize> {
        let i = self.current_index()?;
        if i + 1 < self.tracks.len() {
            Some(i + 1)
        } else if self.repeat {
            Some(0)
        } else {
            None
        }
    }

    /// The previous index in linear order (wraps to the last with repeat), without moving. `None` at
    /// the start.
    pub fn peek_prev(&self) -> Option<usize> {
        let i = self.current_index()?;
        if i > 0 {
            Some(i - 1)
        } else if self.repeat {
            Some(self.tracks.len() - 1)
        } else {
            None
        }
    }

    /// Go forward (the Next button / auto-advance): redo a track from the forward stack if the user
    /// has stepped back, otherwise move to `fresh` (the caller-computed next: a shuffle pick or
    /// [`Self::peek_next`]). Remembers the track left behind for Back. Returns the new current track,
    /// or `None` at a hard end.
    pub fn forward(&mut self, fresh: Option<usize>) -> Option<PathBuf> {
        let fresh = fresh.and_then(|i| self.track_id(i));
        let target = self.forward_candidate(fresh)?;
        self.commit_forward(target)
    }

    /// Return the stable-ID candidate for Forward without changing selection or history. A valid
    /// browser-style redo takes priority over `fresh`.
    pub fn forward_candidate(&mut self, fresh: Option<TrackId>) -> Option<TrackId> {
        self.purge_invalid_future();
        self.future
            .last()
            .copied()
            .or_else(|| fresh.filter(|id| self.track(*id).is_some()))
    }

    /// Commit a candidate returned by [`Self::forward_candidate`]. The departing current track is
    /// recorded for Previous. If a redo is pending, only that redo can be committed.
    pub fn commit_forward(&mut self, target: TrackId) -> Option<PathBuf> {
        self.purge_invalid_future();
        if let Some(redo) = self.future.last().copied() {
            if redo != target {
                return None;
            }
            self.future.pop();
        } else if self.track(target).is_none() {
            return None;
        }
        if let Some(cur) = self.current {
            self.history.push(cur);
            self.cap_history();
        }
        self.select_id(target)
    }

    /// Forget a redo candidate that could not be loaded. It was never played, so it must not enter
    /// actual play history or be offered repeatedly during the same navigation.
    pub fn discard_forward_candidate(&mut self, target: TrackId) {
        self.purge_invalid_future();
        if self.future.last().copied() == Some(target) {
            self.future.pop();
        }
    }

    /// Go back (the Prev button): return to the last remembered track, or `fresh` (typically
    /// [`Self::peek_prev`]) when the back stack is empty. Remembers the current track on the forward
    /// stack so a following [`Self::forward`] redoes it. Returns the new current track, or `None`.
    pub fn back(&mut self, fresh: Option<usize>) -> Option<PathBuf> {
        let fresh = fresh.and_then(|i| self.track_id(i));
        let target = self.back_candidate(fresh)?;
        self.commit_back(target)
    }

    /// Return the stable-ID candidate for Back without changing selection or history. Actual play
    /// history takes priority over `fresh`.
    pub fn back_candidate(&mut self, fresh: Option<TrackId>) -> Option<TrackId> {
        self.purge_invalid_history();
        self.history
            .last()
            .copied()
            .or_else(|| fresh.filter(|id| self.track(*id).is_some()))
    }

    /// Commit a candidate returned by [`Self::back_candidate`]. The track being left becomes a
    /// Forward redo.
    pub fn commit_back(&mut self, target: TrackId) -> Option<PathBuf> {
        self.purge_invalid_history();
        if let Some(previous) = self.history.last().copied() {
            if previous != target {
                return None;
            }
            self.history.pop();
        } else if self.track(target).is_none() {
            return None;
        }
        if let Some(cur) = self.current {
            self.future.push(cur);
        }
        self.select_id(target)
    }

    /// Forget a Previous candidate that could not be loaded. Failed files are not actual playback
    /// history and must not be retried forever.
    pub fn discard_back_candidate(&mut self, target: TrackId) {
        self.purge_invalid_history();
        if self.history.last().copied() == Some(target) {
            self.history.pop();
        }
    }

    /// Jump straight to track `i` (double-click / jump-to-file): a fresh navigation that remembers
    /// the current track for Back and invalidates the forward stack. Returns it, or `None` if out of
    /// range.
    pub fn jump_to(&mut self, i: usize) -> Option<PathBuf> {
        if i >= self.tracks.len() {
            return None;
        }
        let target = self.track_id(i)?;
        if let Some(cur) = self.current {
            if cur != target {
                self.history.push(cur);
                self.cap_history();
            }
        }
        self.future.clear();
        self.select(i)
    }

    /// Drop the Back/Forward navigation history. Called on a shuffle-mode change, since the history
    /// only drives navigation in shuffle (in order, Prev/Next are plainly sequential), so a stale
    /// shuffle trail must not leak across the mode switch.
    pub fn clear_history(&mut self) {
        self.history.clear();
        self.future.clear();
    }

    /// Cap the back history so it cannot grow without bound over a long session.
    fn cap_history(&mut self) {
        let overflow = self.history.len().saturating_sub(HISTORY_MAX);
        if overflow > 0 {
            self.history.drain(..overflow);
        }
    }

    fn allocate_id(&mut self) -> TrackId {
        loop {
            self.next_id = self.next_id.wrapping_add(1);
            if self.next_id == 0 {
                continue;
            }
            let id = TrackId(self.next_id);
            if self.track(id).is_none() {
                return id;
            }
        }
    }

    fn index_of(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|track| track.id == id)
    }

    fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }

    fn select_id(&mut self, id: TrackId) -> Option<PathBuf> {
        let track = self.track(id)?;
        let path = track.path.clone();
        self.current = Some(id);
        Some(path)
    }

    fn purge_invalid_history(&mut self) {
        while self
            .history
            .last()
            .is_some_and(|id| self.track(*id).is_none())
        {
            self.history.pop();
        }
    }

    fn purge_invalid_future(&mut self) {
        while self
            .future
            .last()
            .is_some_and(|id| self.track(*id).is_none())
        {
            self.future.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(names: &[&str]) -> Playlist {
        Playlist::new(names.iter().map(PathBuf::from).collect())
    }

    #[test]
    fn a_fresh_playlist_starts_on_the_first_track() {
        let p = pl(&["a.mp3", "b.mp3", "c.mp3"]);
        assert_eq!(p.len(), 3);
        assert_eq!(p.current_index(), Some(0));
        assert_eq!(p.current(), Some(Path::new("a.mp3")));
        assert!(!p.is_empty());
    }

    #[test]
    fn an_empty_playlist_has_no_current() {
        let p = Playlist::default();
        assert!(p.is_empty());
        assert_eq!(p.current_index(), None);
        assert_eq!(p.current(), None);
    }

    #[test]
    fn next_advances_and_stops_at_the_end() {
        let mut p = pl(&["a", "b", "c"]);
        assert_eq!(p.next().as_deref(), Some(Path::new("b")));
        assert_eq!(p.current_index(), Some(1));
        assert_eq!(p.next().as_deref(), Some(Path::new("c")));
        assert_eq!(p.current_index(), Some(2));
        // At the end: no next, selection unchanged (the caller stops).
        assert_eq!(p.next(), None);
        assert_eq!(p.current_index(), Some(2));
    }

    #[test]
    fn prev_goes_back_and_stays_at_the_start() {
        let mut p = pl(&["a", "b", "c"]);
        p.select(2);
        assert_eq!(p.prev().as_deref(), Some(Path::new("b")));
        assert_eq!(p.prev().as_deref(), Some(Path::new("a")));
        assert_eq!(p.current_index(), Some(0));
        // At the start: no prev, stays put.
        assert_eq!(p.prev(), None);
        assert_eq!(p.current_index(), Some(0));
    }

    #[test]
    fn select_jumps_in_range_and_ignores_out_of_range() {
        let mut p = pl(&["a", "b", "c"]);
        assert_eq!(p.select(2).as_deref(), Some(Path::new("c")));
        assert_eq!(p.current_index(), Some(2));
        // Out of range: no change.
        assert_eq!(p.select(9), None);
        assert_eq!(p.current_index(), Some(2));
    }

    #[test]
    fn repeat_wraps_at_both_ends() {
        let mut p = pl(&["a", "b", "c"]);
        p.set_repeat(true);
        assert!(p.repeat());
        p.select(2);
        assert_eq!(
            p.next().as_deref(),
            Some(Path::new("a")),
            "end wraps to the first with repeat"
        );
        assert_eq!(p.current_index(), Some(0));
        assert_eq!(
            p.prev().as_deref(),
            Some(Path::new("c")),
            "start wraps to the last with repeat"
        );
        assert_eq!(p.current_index(), Some(2));
    }

    #[test]
    fn next_and_prev_on_an_empty_list_are_inert() {
        let mut p = Playlist::default();
        assert_eq!(p.next(), None);
        assert_eq!(p.prev(), None);
        assert_eq!(p.select(0), None);
    }

    #[test]
    fn peek_next_and_prev_report_linear_neighbors() {
        let mut p = pl(&["a", "b", "c"]);
        assert_eq!(p.peek_prev(), None, "at the start");
        assert_eq!(p.peek_next(), Some(1));
        p.select(2);
        assert_eq!(p.peek_next(), None, "at the end without repeat");
        assert_eq!(p.peek_prev(), Some(1));
        p.set_repeat(true);
        assert_eq!(p.peek_next(), Some(0), "wraps with repeat");
        p.select(0);
        assert_eq!(p.peek_prev(), Some(2), "wraps with repeat");
    }

    #[test]
    fn back_retraces_the_real_play_order() {
        // Simulate shuffle by stepping forward to non-adjacent tracks (a -> c -> a -> d).
        let mut p = pl(&["a", "b", "c", "d"]);
        assert_eq!(p.forward(Some(2)).as_deref(), Some(Path::new("c")));
        assert_eq!(p.forward(Some(0)).as_deref(), Some(Path::new("a")));
        assert_eq!(p.forward(Some(3)).as_deref(), Some(Path::new("d")));
        assert_eq!(p.current_index(), Some(3));
        // Back retraces that exact order, not index-1.
        assert_eq!(p.back(None).as_deref(), Some(Path::new("a")));
        assert_eq!(p.back(None).as_deref(), Some(Path::new("c")));
        assert_eq!(
            p.back(None).as_deref(),
            Some(Path::new("a")),
            "back to the very first track"
        );
        assert_eq!(
            p.back(None),
            None,
            "no history and no linear previous: inert"
        );
        assert_eq!(p.current_index(), Some(0));
    }

    #[test]
    fn forward_redoes_a_stepped_back_track() {
        let mut p = pl(&["a", "b", "c"]);
        p.forward(Some(1)); // a -> b
        p.forward(Some(2)); // b -> c
        p.back(None); // c -> b
        assert_eq!(p.current_index(), Some(1));
        // Forward redoes c from the forward stack, ignoring the passed-in fresh index.
        assert_eq!(p.forward(Some(0)).as_deref(), Some(Path::new("c")));
        assert_eq!(
            p.current_index(),
            Some(2),
            "redo used the forward stack, not the fresh index"
        );
    }

    #[test]
    fn jump_to_records_history_and_clears_the_forward_stack() {
        let mut p = pl(&["a", "b", "c", "d"]);
        p.forward(Some(1)); // a -> b
        p.back(None); // b -> a, so c-side forward stack now holds b
        assert_eq!(
            p.jump_to(3).as_deref(),
            Some(Path::new("d")),
            "a -> d, a fresh jump"
        );
        assert_eq!(p.current_index(), Some(3));
        // The forward stack was cleared, so forward uses the fresh index instead of redoing b.
        assert_eq!(p.forward(Some(2)).as_deref(), Some(Path::new("c")));
        // Back returns to d, then to a (jump_to remembered both).
        assert_eq!(p.back(None).as_deref(), Some(Path::new("d")));
        assert_eq!(p.back(None).as_deref(), Some(Path::new("a")));
        // Out of range: inert.
        let cur = p.current_index();
        assert_eq!(p.jump_to(9), None);
        assert_eq!(p.current_index(), cur);
    }

    #[test]
    fn forward_stops_at_a_hard_end() {
        let mut p = pl(&["a", "b"]);
        p.select(1);
        assert_eq!(
            p.forward(None),
            None,
            "end of list, no repeat, nothing to redo"
        );
        assert_eq!(p.current_index(), Some(1));
    }

    #[test]
    fn clear_history_drops_back_and_forward() {
        let mut p = pl(&["a", "b", "c"]);
        p.forward(Some(1)); // a -> b, history=[0]
        p.forward(Some(2)); // b -> c, history=[0,1]
        p.clear_history();
        // With the trail gone, Back falls to the linear previous (b) rather than retracing history.
        let fresh = p.peek_prev();
        assert_eq!(
            p.back(fresh).as_deref(),
            Some(Path::new("b")),
            "linear previous, not history"
        );
    }

    #[test]
    fn insert_and_move_keep_the_current_entry_identity() {
        let mut p = pl(&["a", "b", "c"]);
        p.select(1);
        let current = p.current_id().unwrap();

        let inserted = p.insert(0, PathBuf::from("new")).unwrap();
        assert_ne!(inserted, current);
        assert_eq!(p.current_id(), Some(current));
        assert_eq!(p.current_index(), Some(2));
        assert_eq!(p.current(), Some(Path::new("b")));

        assert!(p.move_track(2, 0));
        assert_eq!(p.current_id(), Some(current));
        assert_eq!(p.current_index(), Some(0));
        assert_eq!(p.current(), Some(Path::new("b")));
        assert_eq!(
            p.tracks().collect::<Vec<_>>(),
            ["b", "new", "a", "c"].map(Path::new)
        );
    }

    #[test]
    fn edits_preserve_surviving_shuffle_history_and_purge_removed_entries() {
        let mut p = pl(&["a", "b", "c", "d"]);
        p.forward(Some(1)); // a -> b, history=[a]
        p.forward(Some(2)); // b -> c, history=[a,b]
        assert_eq!(p.back(None).as_deref(), Some(Path::new("b"))); // future=[c]

        assert_eq!(
            p.remove(2).as_deref(),
            Some(Path::new("c")),
            "remove the future entry"
        );
        assert_eq!(
            p.forward(Some(2)).as_deref(),
            Some(Path::new("d")),
            "removed future is skipped"
        );
        assert_eq!(p.back(None).as_deref(), Some(Path::new("b")));
        assert_eq!(
            p.back(None).as_deref(),
            Some(Path::new("a")),
            "surviving history is intact"
        );
    }

    #[test]
    fn removing_current_selects_the_next_entry_then_the_previous_final_entry() {
        let mut p = pl(&["a", "b", "c"]);
        p.select(1);
        assert_eq!(p.remove(1).as_deref(), Some(Path::new("b")));
        assert_eq!(p.current(), Some(Path::new("c")), "same display slot wins");
        assert_eq!(p.remove(1).as_deref(), Some(Path::new("c")));
        assert_eq!(
            p.current(),
            Some(Path::new("a")),
            "last removal falls back to previous"
        );
        assert_eq!(p.remove(0).as_deref(), Some(Path::new("a")));
        assert!(p.current().is_none());
        assert!(p.is_empty());
    }

    #[test]
    fn duplicate_paths_have_unique_non_reused_ids_and_invalid_edits_are_inert() {
        let mut p = pl(&["same", "same"]);
        let first = p.track_id(0).unwrap();
        let second = p.track_id(1).unwrap();
        assert_ne!(first, second);
        assert!(first.get() > 0 && second.get() > first.get());
        assert_eq!(p.insert(9, PathBuf::from("nope")), None);
        assert_eq!(p.remove(9), None);
        assert!(!p.move_track(0, 9));

        p.clear();
        let after_clear = p.add(PathBuf::from("same"));
        assert!(
            after_clear.get() > second.get(),
            "clear does not recycle identities"
        );
        assert_eq!(p.current_id(), Some(after_clear));
    }

    #[test]
    fn shuffle_cycle_visits_each_other_entry_once_then_exhausts() {
        let p = pl(&["a", "b", "c", "d", "e"]);
        let current = p.current_id().unwrap();
        let mut cycle = ShuffleCycle::with_seed(7);
        cycle.anchor(&p);

        let mut visited = Vec::new();
        while let Some(id) = cycle.next(&p, false) {
            visited.push(id);
        }

        assert_eq!(visited.len(), p.len() - 1);
        assert!(!visited.contains(&current));
        assert_eq!(visited.iter().copied().collect::<HashSet<_>>().len(), 4);
        assert_eq!(cycle.next(&p, false), None, "repeat-off stays exhausted");
    }

    #[test]
    fn shuffle_repeat_starts_a_full_new_cycle_without_an_immediate_repeat() {
        let mut p = pl(&["a", "b", "c", "d"]);
        let mut cycle = ShuffleCycle::with_seed(91);
        cycle.anchor(&p);

        let first_cycle: Vec<_> = (0..p.len() - 1)
            .map(|_| cycle.next(&p, false).unwrap())
            .collect();
        let last = *first_cycle.last().unwrap();
        p.select_track(last);

        let next = cycle.next(&p, true).unwrap();
        assert_ne!(next, last, "a new cycle avoids the track just played");
        let mut new_cycle = vec![next];
        p.select_track(next);
        for _ in 1..p.len() {
            let id = cycle.next(&p, true).unwrap();
            p.select_track(id);
            new_cycle.push(id);
        }
        assert_eq!(
            new_cycle.iter().copied().collect::<HashSet<_>>().len(),
            p.len()
        );
    }

    #[test]
    fn single_track_shuffle_repeats_only_when_repeat_is_on() {
        let p = pl(&["only"]);
        let only = p.current_id().unwrap();
        let mut cycle = ShuffleCycle::with_seed(1);
        cycle.anchor(&p);

        assert_eq!(cycle.next(&p, false), None);
        assert_eq!(cycle.next(&p, true), Some(only));
    }

    #[test]
    fn shuffle_cycle_survives_remove_move_and_add_by_stable_identity() {
        let mut p = pl(&["a", "b", "c", "d"]);
        let removed = p.track_id(1).unwrap();
        let moved = p.track_id(3).unwrap();
        let mut cycle = ShuffleCycle::with_seed(1234);
        cycle.anchor(&p);

        assert!(p.move_track(3, 1));
        assert_eq!(p.remove(2).as_deref(), Some(Path::new("b")));
        let added = p.add(PathBuf::from("new"));

        let mut visited = Vec::new();
        while let Some(id) = cycle.next(&p, false) {
            visited.push(id);
        }
        assert!(!visited.contains(&removed));
        assert!(visited.contains(&moved));
        assert!(visited.contains(&added));
        assert_eq!(
            visited.iter().copied().collect::<HashSet<_>>().len(),
            visited.len()
        );
        assert_eq!(visited.len(), p.len() - 1);
    }

    #[test]
    fn equal_shuffle_seeds_produce_equal_permutations() {
        let p = pl(&["a", "b", "c", "d", "e", "f"]);
        let mut a = ShuffleCycle::with_seed(0xCAFE);
        let mut b = ShuffleCycle::with_seed(0xCAFE);
        a.anchor(&p);
        b.anchor(&p);

        let order_a: Vec<_> = (1..p.len()).map(|_| a.next(&p, false).unwrap()).collect();
        let order_b: Vec<_> = (1..p.len()).map(|_| b.next(&p, false).unwrap()).collect();
        assert_eq!(order_a, order_b);
    }

    #[test]
    fn transactional_redo_and_back_do_not_record_discarded_failures() {
        let mut p = pl(&["a", "b", "c"]);
        p.forward(Some(1)); // a -> b
        p.forward(Some(2)); // b -> c
        p.back(None); // c -> b, future=[c]

        let failed_redo = p.forward_candidate(None).unwrap();
        assert_eq!(p.path_for_id(failed_redo), Some(Path::new("c")));
        p.discard_forward_candidate(failed_redo);
        assert_eq!(p.forward_candidate(None), None);
        assert_eq!(p.current(), Some(Path::new("b")));

        let previous = p.back_candidate(None).unwrap();
        assert_eq!(p.path_for_id(previous), Some(Path::new("a")));
        p.discard_back_candidate(previous);
        assert_eq!(p.back_candidate(None), None);
        assert_eq!(p.current(), Some(Path::new("b")));
    }

    #[test]
    fn forward_after_back_redoes_history_without_consuming_shuffle_cycle() {
        let mut p = pl(&["a", "b", "c", "d", "e"]);
        let mut cycle = ShuffleCycle::with_seed(0x1234);
        cycle.anchor(&p);

        let first = cycle.next(&p, false).unwrap();
        p.commit_forward(first);
        let second = cycle.next(&p, false).unwrap();
        p.commit_forward(second);

        let mut expected_cycle = cycle.clone();
        let expected_fresh = expected_cycle.next(&p, false).unwrap();
        let previous = p.back_candidate(None).unwrap();
        p.commit_back(previous);
        let redo = p.forward_candidate(None).unwrap();
        assert_eq!(redo, second);
        p.commit_forward(redo);

        assert_eq!(
            cycle.next(&p, false),
            Some(expected_fresh),
            "redoing Forward must leave the pending permutation untouched"
        );
    }
}
