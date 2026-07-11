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

/// Time-display digit cell size in the number sheets (`NUMBERS.BMP` / `NUMS_EX.BMP`).
pub const DIGIT_W: i32 = 9;
pub const DIGIT_H: i32 = 13;

/// Source rects for digits 0-9 in the number sheet. Both sheets place the ten digits at the
/// same cells: digit `d` at x = d*9, y = 0, sized 9x13. (They differ only in the trailing
/// blank and minus cells, which the elapsed-time display does not use.)
pub const DIGITS: [Rect; 10] = [
    Rect::new(0, 0, DIGIT_W, DIGIT_H),
    Rect::new(9, 0, DIGIT_W, DIGIT_H),
    Rect::new(18, 0, DIGIT_W, DIGIT_H),
    Rect::new(27, 0, DIGIT_W, DIGIT_H),
    Rect::new(36, 0, DIGIT_W, DIGIT_H),
    Rect::new(45, 0, DIGIT_W, DIGIT_H),
    Rect::new(54, 0, DIGIT_W, DIGIT_H),
    Rect::new(63, 0, DIGIT_W, DIGIT_H),
    Rect::new(72, 0, DIGIT_W, DIGIT_H),
    Rect::new(81, 0, DIGIT_W, DIGIT_H),
];

/// The song-title marquee region on the main window: a `MARQUEE_W`-wide strip whose glyph
/// rows start at (`MARQUEE_X`, `MARQUEE_Y`). Classic skins draw the title here from `text.bmp`
/// (5x6 cells), scrolling it when it overruns the width. `MARQUEE_Y` is the top of the 6px
/// glyph row (the classic element sits at y=24 with 3px of top padding above the glyphs).
pub const MARQUEE_X: i32 = 111;
pub const MARQUEE_Y: i32 = 27;
pub const MARQUEE_W: i32 = 154;

/// Destination top-lefts of the four time-display digits on the main window, in order:
/// tens-of-minutes, units-of-minutes, tens-of-seconds, units-of-seconds. Digits within a pair
/// step by 12px; the MM and SS pairs are 18px apart, the extra 6px being where the background
/// colon sits (the colon is part of MAIN.BMP, not a digit). Coordinates are the classic layout.
/// (The countdown minus sign, added with a later remaining-time toggle, is a 9x13 cell at 39,26.)
pub const TIME_DIGITS: [(i32, i32); 4] = [(48, 26), (60, 26), (78, 26), (90, 26)];

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
    fn digit_cells_tile_across_the_top_row() {
        assert_eq!(DIGITS.len(), 10);
        for (d, r) in DIGITS.iter().enumerate() {
            assert_eq!(*r, Rect::new(d as i32 * DIGIT_W, 0, DIGIT_W, DIGIT_H), "digit {d}");
        }
    }

    #[test]
    fn marquee_region_matches_the_classic_layout() {
        assert_eq!((MARQUEE_X, MARQUEE_Y), (111, 27));
        assert_eq!(MARQUEE_W, 154);
        // The strip stays inside the 275px-wide window, ending 10px shy of the right edge.
        assert_eq!(MARQUEE_X + MARQUEE_W, MAIN_W - 10);
    }

    #[test]
    fn time_digits_layout_leaves_room_for_the_colon() {
        assert_eq!(TIME_DIGITS, [(48, 26), (60, 26), (78, 26), (90, 26)]);
        assert_eq!(TIME_DIGITS[1].0 - TIME_DIGITS[0].0, 12, "step within the MM pair");
        assert_eq!(TIME_DIGITS[3].0 - TIME_DIGITS[2].0, 12, "step within the SS pair");
        assert_eq!(TIME_DIGITS[2].0 - TIME_DIGITS[1].0, 18, "MM->SS spans the colon gap");
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
