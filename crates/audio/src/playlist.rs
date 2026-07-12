//! The playlist: an ordered list of track paths plus the current position. Pure (no audio), so the
//! next/previous/selection logic is unit-tested without a device. The player layer turns a returned
//! path into actual playback.

use std::path::{Path, PathBuf};

/// An ordered list of tracks with a current selection.
#[derive(Debug, Default, Clone)]
pub struct Playlist {
    tracks: Vec<PathBuf>,
    /// Index of the current track, or `None` when the list is empty.
    current: Option<usize>,
    /// When set, `next`/`prev` wrap around the ends instead of stopping (repeat-all).
    repeat: bool,
}

impl Playlist {
    /// Build a playlist from `tracks`; the first track (if any) becomes current.
    pub fn new(tracks: Vec<PathBuf>) -> Self {
        let current = (!tracks.is_empty()).then_some(0);
        Self { tracks, current, repeat: false }
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
}
