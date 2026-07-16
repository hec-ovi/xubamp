//! Geometry shared by the Wayland pane surfaces.
//!
//! Classic Winamp windows snap when matching edges are less than 15 pixels apart.  Keeping the
//! policy independent of Wayland makes the exact boundary and the graph behavior testable without
//! a compositor.

/// The classic/Webamp edge attraction distance.  The comparison is deliberately strict: a
/// fourteen-pixel gap snaps and a fifteen-pixel gap does not.
pub const SNAP_DISTANCE: i32 = 15;

/// The active snap threshold. The classic 15px is the default; the Preferences Display page can
/// change it (0 disables snapping). Read on the single UI thread only.
use std::sync::atomic::{AtomicI32, Ordering};
static SNAP_PX: AtomicI32 = AtomicI32::new(SNAP_DISTANCE);

/// Set the edge-snap threshold in pixels (0 disables).
pub fn set_snap_px(px: i32) {
    SNAP_PX.store(px.clamp(0, 64), Ordering::Relaxed);
}

fn snap_px() -> i32 {
    SNAP_PX.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn at(position: Point, width: i32, height: i32) -> Self {
        Self {
            x: position.x,
            y: position.y,
            width,
            height,
        }
    }

    pub const fn right(self) -> i32 {
        self.x + self.width
    }

    pub const fn bottom(self) -> i32 {
        self.y + self.height
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Snap {
    x: Option<i32>,
    y: Option<i32>,
}

fn near(a: i32, b: i32) -> bool {
    (a - b).abs() < snap_px()
}

fn overlaps_x(a: Rect, b: Rect) -> bool {
    a.x <= b.right() + snap_px() && b.x <= a.right() + snap_px()
}

fn overlaps_y(a: Rect, b: Rect) -> bool {
    a.y <= b.bottom() + snap_px() && b.y <= a.bottom() + snap_px()
}

/// Return the coordinates that would align `moving` to `stationary`.  Axis choices follow the
/// classic order: outside edges first, then matching left/top edges, then matching right/bottom.
fn snap(moving: Rect, stationary: Rect) -> Snap {
    let mut result = Snap::default();
    if overlaps_y(moving, stationary) {
        result.x = if near(moving.x, stationary.right()) {
            Some(stationary.right())
        } else if near(moving.right(), stationary.x) {
            Some(stationary.x - moving.width)
        } else if near(moving.x, stationary.x) {
            Some(stationary.x)
        } else if near(moving.right(), stationary.right()) {
            Some(stationary.right() - moving.width)
        } else {
            None
        };
    }
    if overlaps_x(moving, stationary) {
        result.y = if near(moving.y, stationary.bottom()) {
            Some(stationary.bottom())
        } else if near(moving.bottom(), stationary.y) {
            Some(stationary.y - moving.height)
        } else if near(moving.y, stationary.y) {
            Some(stationary.y)
        } else if near(moving.bottom(), stationary.bottom()) {
            Some(stationary.bottom() - moving.height)
        } else {
            None
        };
    }
    result
}

/// Snap a proposed pane rectangle to the first matching edge on each axis.  The caller controls the
/// priority by ordering `stationary`; main, equalizer, playlist is the normal order.
pub fn snap_to_many(proposed: Rect, stationary: &[Rect]) -> Point {
    let mut dx = 0;
    let mut dy = 0;
    for other in stationary {
        let candidate = snap(proposed, *other);
        if dx == 0 {
            dx = candidate.x.map_or(0, |x| x - proposed.x);
        }
        if dy == 0 {
            dy = candidate.y.map_or(0, |y| y - proposed.y);
        }
        if dx != 0 && dy != 0 {
            break;
        }
    }
    Point {
        x: proposed.x + dx,
        y: proposed.y + dy,
    }
}

/// Whether two panes count as connected for a main-window cluster drag.  This deliberately uses
/// the same attraction test as dragging, including aligned edges and a sub-15-pixel gap.
#[cfg(test)]
fn connected(a: Rect, b: Rect) -> bool {
    let result = snap(a, b);
    result.x.is_some() || result.y.is_some()
}

/// Return all rectangles transitively connected to `anchor`, in input order.  This is useful when
/// the main pane drags an equalizer that in turn has the playlist attached below it.
#[cfg(test)]
fn connected_component(rects: &[Rect], anchor: usize) -> Vec<usize> {
    if anchor >= rects.len() {
        return Vec::new();
    }
    let mut selected = vec![false; rects.len()];
    selected[anchor] = true;
    let mut pending = vec![anchor];
    while let Some(current) = pending.pop() {
        for candidate in 0..rects.len() {
            if !selected[candidate] && connected(rects[candidate], rects[current]) {
                selected[candidate] = true;
                pending.push(candidate);
            }
        }
    }
    selected
        .into_iter()
        .enumerate()
        .filter_map(|(i, selected)| selected.then_some(i))
        .collect()
}

/// Adjust a pane directly attached to the right or bottom of `anchor` after the anchor changes
/// size. Matching far edges alone do not form a resize relationship.
pub fn preserve_resize_attachment(pane: Rect, old_anchor: Rect, new_anchor: Rect) -> Point {
    let overlaps_anchor_x = pane.x <= old_anchor.right() && old_anchor.x <= pane.right();
    let overlaps_anchor_y = pane.y <= old_anchor.bottom() && old_anchor.y <= pane.bottom();
    let dx = if overlaps_anchor_y && pane.x == old_anchor.right() {
        new_anchor.right() - old_anchor.right()
    } else {
        0
    };
    let dy = if overlaps_anchor_x && pane.y == old_anchor.bottom() {
        new_anchor.bottom() - old_anchor.bottom()
    } else {
        0
    };
    Point {
        x: pane.x + dx,
        y: pane.y + dy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect {
        x: 0,
        y: 0,
        width: 275,
        height: 116,
    };

    #[test]
    fn snap_threshold_is_strictly_less_than_fifteen() {
        let fourteen = Rect {
            x: 0,
            y: MAIN.bottom() + 14,
            width: 275,
            height: 116,
        };
        let fifteen = Rect {
            y: MAIN.bottom() + 15,
            ..fourteen
        };
        assert_eq!(snap_to_many(fourteen, &[MAIN]).y, MAIN.bottom());
        assert_eq!(snap_to_many(fifteen, &[MAIN]).y, fifteen.y);
    }

    #[test]
    fn snaps_outside_and_matching_edges_on_both_axes() {
        let proposed = Rect {
            x: MAIN.right() + 7,
            y: 9,
            width: 275,
            height: 116,
        };
        assert_eq!(
            snap_to_many(proposed, &[MAIN]),
            Point {
                x: MAIN.right(),
                y: 0
            }
        );

        let below = Rect {
            x: -8,
            y: MAIN.bottom() + 4,
            ..proposed
        };
        assert_eq!(
            snap_to_many(below, &[MAIN]),
            Point {
                x: 0,
                y: MAIN.bottom()
            }
        );
    }

    #[test]
    fn distant_perpendicular_ranges_do_not_attract() {
        let proposed = Rect {
            x: MAIN.right() + 3,
            y: 1_000,
            width: 275,
            height: 116,
        };
        assert_eq!(
            snap_to_many(proposed, &[MAIN]),
            Point {
                x: proposed.x,
                y: proposed.y
            }
        );
    }

    #[test]
    fn connected_component_is_transitive() {
        let eq = Rect {
            y: MAIN.bottom(),
            ..MAIN
        };
        let playlist = Rect {
            y: eq.bottom(),
            ..MAIN
        };
        let detached = Rect {
            x: 900,
            y: 900,
            ..MAIN
        };
        assert_eq!(
            connected_component(&[MAIN, eq, playlist, detached], 0),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn shade_resize_keeps_a_bottom_attached_pane_attached() {
        let playlist = Rect {
            y: MAIN.bottom(),
            ..MAIN
        };
        let shaded = Rect { height: 14, ..MAIN };
        assert_eq!(
            preserve_resize_attachment(playlist, MAIN, shaded),
            Point {
                x: 0,
                y: shaded.bottom()
            }
        );
    }

    #[test]
    fn shade_resize_does_not_move_a_right_attached_pane_when_width_is_unchanged() {
        let playlist = Rect {
            x: MAIN.right(),
            ..MAIN
        };
        let shaded = Rect { height: 14, ..MAIN };
        assert_eq!(
            preserve_resize_attachment(playlist, MAIN, shaded),
            Point {
                x: playlist.x,
                y: playlist.y
            }
        );
    }
}
