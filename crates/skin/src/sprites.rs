//! Static sprite-coordinate tables for classic skins.
//!
//! Every element Winamp draws comes from a fixed sub-rectangle of a sheet, drawn at a
//! fixed destination on the window. These numbers are facts about the skin format,
//! transcribed from the documented classic layout (not copied from any implementation).
//! This module holds the main-window set; more sheets are added as later phases render
//! them. The coordinates are validated against real skins in the render-diff pass.

/// A source rectangle within a sheet, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
}

/// A sprite to blit: a source rect from a sheet, drawn at a window destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub src: Rect,
    pub dst_x: i32,
    pub dst_y: i32,
}

impl Placement {
    pub const fn new(src: Rect, dst_x: i32, dst_y: i32) -> Self {
        Self { src, dst_x, dst_y }
    }
}

/// Main window size, in pixels.
pub const MAIN_W: i32 = 275;
pub const MAIN_H: i32 = 116;

/// MAIN.BMP is the full 275x116 background, drawn at the origin.
pub const MAIN_BG: Placement = Placement::new(Rect::new(0, 0, MAIN_W, MAIN_H), 0, 0);

/// Title-bar strips from TITLEBAR.BMP, drawn at the origin (275x14).
pub const TITLEBAR_ACTIVE: Placement = Placement::new(Rect::new(27, 0, 275, 14), 0, 0);
pub const TITLEBAR_INACTIVE: Placement = Placement::new(Rect::new(27, 15, 275, 14), 0, 0);

/// The six transport buttons from CBUTTONS.BMP (normal state, top row), in order:
/// previous, play, pause, stop, next, eject.
pub const CBUTTONS: [Placement; 6] = [
    Placement::new(Rect::new(0, 0, 23, 18), 16, 88),    // previous
    Placement::new(Rect::new(23, 0, 23, 18), 39, 88),   // play
    Placement::new(Rect::new(46, 0, 23, 18), 62, 88),   // pause
    Placement::new(Rect::new(69, 0, 23, 18), 85, 88),   // stop
    Placement::new(Rect::new(92, 0, 22, 18), 108, 88),  // next
    Placement::new(Rect::new(114, 0, 22, 16), 136, 89), // eject
];

/// The same six buttons in their pressed state (the bottom row of CBUTTONS.BMP), same
/// destinations. Each source rect is the normal one shifted down by its own height, so the
/// pressed art sits directly below the normal art: 18px for the first five, 16px for the
/// shorter eject button (whose pressed art is one pixel higher, per the classic sheet).
pub const CBUTTONS_PRESSED: [Placement; 6] = [
    Placement::new(Rect::new(0, 18, 23, 18), 16, 88),    // previous
    Placement::new(Rect::new(23, 18, 23, 18), 39, 88),   // play
    Placement::new(Rect::new(46, 18, 23, 18), 62, 88),   // pause
    Placement::new(Rect::new(69, 18, 23, 18), 85, 88),   // stop
    Placement::new(Rect::new(92, 18, 22, 18), 108, 88),  // next
    Placement::new(Rect::new(114, 16, 22, 16), 136, 89), // eject
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_window_geometry() {
        assert_eq!((MAIN_W, MAIN_H), (275, 116));
        assert_eq!(MAIN_BG.src, Rect::new(0, 0, 275, 116));
    }

    #[test]
    fn six_transport_buttons_in_order() {
        assert_eq!(CBUTTONS.len(), 6);
        // play is the second button, drawn just right of previous.
        assert_eq!(CBUTTONS[1].dst_x, 39);
        assert_eq!(CBUTTONS[1].src, Rect::new(23, 0, 23, 18));
        // eject is the narrow, shorter button at the far right.
        assert_eq!(CBUTTONS[5].src, Rect::new(114, 0, 22, 16));
    }

    #[test]
    fn pressed_buttons_share_destinations_and_sit_below_normal() {
        assert_eq!(CBUTTONS_PRESSED.len(), 6);
        for (normal, pressed) in CBUTTONS.iter().zip(CBUTTONS_PRESSED.iter()) {
            // Same on-window position and same size as the normal state.
            assert_eq!((pressed.dst_x, pressed.dst_y), (normal.dst_x, normal.dst_y));
            assert_eq!((pressed.src.w, pressed.src.h), (normal.src.w, normal.src.h));
            assert_eq!(pressed.src.x, normal.src.x);
            // Pressed art is directly below the normal art, offset by the button's height.
            assert_eq!(pressed.src.y, normal.src.y + normal.src.h);
        }
    }
}
