//! The playlist: an ordered list of track paths plus the current position. Pure (no audio), so the
//! next/previous/selection logic is unit-tested without a device. The player layer turns a returned
//! path into actual playback.

use std::path::{Path, PathBuf};

/// The most recent tracks remembered for Back/Forward navigation. Bounds memory over a long session
/// (older entries fall off the far end of the back history).
const HISTORY_MAX: usize = 256;

/// An ordered list of tracks with a current selection.
#[derive(Debug, Default, Clone)]
pub struct Playlist {
    tracks: Vec<PathBuf>,
    /// Index of the current track, or `None` when the list is empty.
    current: Option<usize>,
    /// When set, `next`/`prev` wrap around the ends instead of stopping (repeat-all).
    repeat: bool,
    /// Back stack: previously-played track indices, most recent last. `back` returns to these, so
    /// Prev retraces the real play order (essential under shuffle, where it is not just index-1).
    history: Vec<usize>,
    /// Forward stack: tracks stepped away from by `back`, so a following `forward` redoes them
    /// (browser-style). Cleared by a fresh jump (double-click / jump-to-file).
    future: Vec<usize>,
}

impl Playlist {
    /// Build a playlist from `tracks`; the first track (if any) becomes current.
    pub fn new(tracks: Vec<PathBuf>) -> Self {
        let current = (!tracks.is_empty()).then_some(0);
        Self { tracks, current, repeat: false, history: Vec::new(), future: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// The tracks, in order (for a future playlist window to render).
    pub fn tracks(&self) -> &[PathBuf] {
        &self.tracks
    }

    /// Index of the current track, or `None` when the list is empty.
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    /// The current track's path, or `None` when the list is empty.
    pub fn current(&self) -> Option<&Path> {
        self.current.map(|i| self.tracks[i].as_path())
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
        let i = self.current?;
        let n = self.tracks.len();
        let j = if i + 1 < n {
            Some(i + 1)
        } else if self.repeat {
            Some(0)
        } else {
            None
        };
        j.map(|j| {
            self.current = Some(j);
            self.tracks[j].clone()
        })
    }

    /// Go to the previous track and return it. At the start this returns `None` (stays put), or
    /// wraps to the last track when repeat is on.
    pub fn prev(&mut self) -> Option<PathBuf> {
        let i = self.current?;
        let n = self.tracks.len();
        let j = if i > 0 {
            Some(i - 1)
        } else if self.repeat {
            Some(n - 1)
        } else {
            None
        };
        j.map(|j| {
            self.current = Some(j);
            self.tracks[j].clone()
        })
    }

    /// Select track `i` and return it, or `None` if `i` is out of range (selection unchanged).
    pub fn select(&mut self, i: usize) -> Option<PathBuf> {
        if i < self.tracks.len() {
            self.current = Some(i);
            Some(self.tracks[i].clone())
        } else {
            None
        }
    }

    /// The next index in linear order (wraps to the first with repeat), without moving. `None` at a
    /// hard end. The player passes this (or a shuffle-chosen index) to [`Self::forward`].
    pub fn peek_next(&self) -> Option<usize> {
        let i = self.current?;
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
        let i = self.current?;
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
        let target = self.future.pop().or(fresh)?;
        if let Some(cur) = self.current {
            self.history.push(cur);
            self.cap_history();
        }
        self.select(target)
    }

    /// Go back (the Prev button): return to the last remembered track, or `fresh` (typically
    /// [`Self::peek_prev`]) when the back stack is empty. Remembers the current track on the forward
    /// stack so a following [`Self::forward`] redoes it. Returns the new current track, or `None`.
    pub fn back(&mut self, fresh: Option<usize>) -> Option<PathBuf> {
        let target = self.history.pop().or(fresh)?;
        if let Some(cur) = self.current {
            self.future.push(cur);
        }
        self.select(target)
    }

    /// Jump straight to track `i` (double-click / jump-to-file): a fresh navigation that remembers
    /// the current track for Back and invalidates the forward stack. Returns it, or `None` if out of
    /// range.
    pub fn jump_to(&mut self, i: usize) -> Option<PathBuf> {
        if i >= self.tracks.len() {
            return None;
        }
        if let Some(cur) = self.current {
            if cur != i {
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
        assert_eq!(p.next().as_deref(), Some(Path::new("a")), "end wraps to the first with repeat");
        assert_eq!(p.current_index(), Some(0));
        assert_eq!(p.prev().as_deref(), Some(Path::new("c")), "start wraps to the last with repeat");
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
        assert_eq!(p.back(None).as_deref(), Some(Path::new("a")), "back to the very first track");
        assert_eq!(p.back(None), None, "no history and no linear previous: inert");
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
        assert_eq!(p.current_index(), Some(2), "redo used the forward stack, not the fresh index");
    }

    #[test]
    fn jump_to_records_history_and_clears_the_forward_stack() {
        let mut p = pl(&["a", "b", "c", "d"]);
        p.forward(Some(1)); // a -> b
        p.back(None); // b -> a, so c-side forward stack now holds b
        assert_eq!(p.jump_to(3).as_deref(), Some(Path::new("d")), "a -> d, a fresh jump");
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
        assert_eq!(p.forward(None), None, "end of list, no repeat, nothing to redo");
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
        assert_eq!(p.back(fresh).as_deref(), Some(Path::new("b")), "linear previous, not history");
    }
}
