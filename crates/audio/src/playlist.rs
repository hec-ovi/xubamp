//! The playlist: an ordered list of stable track identities plus the current position. Pure (no
//! audio), so mutation and navigation are unit-tested without a device. The player layer turns a
//! returned path into actual playback.

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
        let target = self
            .pop_valid_future()
            .or_else(|| fresh.and_then(|i| self.track_id(i)))?;
        if let Some(cur) = self.current {
            self.history.push(cur);
            self.cap_history();
        }
        self.select_id(target)
    }

    /// Go back (the Prev button): return to the last remembered track, or `fresh` (typically
    /// [`Self::peek_prev`]) when the back stack is empty. Remembers the current track on the forward
    /// stack so a following [`Self::forward`] redoes it. Returns the new current track, or `None`.
    pub fn back(&mut self, fresh: Option<usize>) -> Option<PathBuf> {
        let target = self
            .pop_valid_history()
            .or_else(|| fresh.and_then(|i| self.track_id(i)))?;
        if let Some(cur) = self.current {
            self.future.push(cur);
        }
        self.select_id(target)
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

    fn pop_valid_history(&mut self) -> Option<TrackId> {
        while let Some(id) = self.history.pop() {
            if self.track(id).is_some() {
                return Some(id);
            }
        }
        None
    }

    fn pop_valid_future(&mut self) -> Option<TrackId> {
        while let Some(id) = self.future.pop() {
            if self.track(id).is_some() {
                return Some(id);
            }
        }
        None
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
}
