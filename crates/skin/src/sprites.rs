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

/// Volume and balance sliders share a sheet layout: a column of `SLIDER_FRAMES` background
/// frames stacked `SLIDER_FRAME_STRIDE` px apart (the level indicator), then the draggable
/// thumb below them. The background is drawn `SLIDER_BG_H` px tall (the classic container
/// height) even though the frame cells stride by 15, so only the top of each cell shows.
pub const SLIDER_FRAME_STRIDE: i32 = 15;
pub const SLIDER_FRAMES: i32 = 28;
pub const SLIDER_BG_H: i32 = 13;

/// The slider thumb sprite, 14x11, in the same two cells of both `volume.bmp` and `balance.bmp`:
/// the normal state at x=15 and the pressed (held) state at x=0, both at y=422 (just below the
/// 420px-tall background column).
pub const SLIDER_THUMB_W: i32 = 14;
pub const SLIDER_THUMB_H: i32 = 11;
/// The thumb sits 1px below the background's top edge (the classic CSS `top: 1px`).
pub const SLIDER_THUMB_DY: i32 = 1;
pub const SLIDER_THUMB_NORMAL: Rect = Rect::new(15, 422, SLIDER_THUMB_W, SLIDER_THUMB_H);
pub const SLIDER_THUMB_PRESSED: Rect = Rect::new(0, 422, SLIDER_THUMB_W, SLIDER_THUMB_H);

/// The volume slider: a 68x13 background at (107, 57) drawn from `volume.bmp` (background column
/// starts at x=0), with the thumb travelling `VOLUME_W - SLIDER_THUMB_W` px across it.
pub const VOLUME_X: i32 = 107;
pub const VOLUME_Y: i32 = 57;
pub const VOLUME_W: i32 = 68;
pub const VOLUME_BG_SRC_X: i32 = 0;

/// The balance slider: a 38x13 background at (177, 57) drawn from `balance.bmp`, whose background
/// column starts 9px in (`BALANCE_BG_SRC_X`), with the thumb travelling `BALANCE_W - thumb` px.
pub const BALANCE_X: i32 = 177;
pub const BALANCE_Y: i32 = 57;
pub const BALANCE_W: i32 = 38;
pub const BALANCE_BG_SRC_X: i32 = 9;

/// The position (seek) bar from POSBAR.BMP (307x10): a 248x10 groove background on the left, then
/// the two thumb states to its right. Unlike the volume/balance sheets (a column of level frames)
/// this is a single row: one groove sprite with the thumb sliding over it. The container is at
/// (16, 72) on the main window. Coordinates cross-checked against Webamp's classic main window.
pub const POSBAR_X: i32 = 16;
pub const POSBAR_Y: i32 = 72;
pub const POSBAR_W: i32 = 248;
pub const POSBAR_H: i32 = 10;

/// The groove background: the left 248x10 of POSBAR.BMP, drawn at the container origin.
pub const POSBAR_BG: Rect = Rect::new(0, 0, POSBAR_W, POSBAR_H);

/// The 29x10 thumb in its normal (released) and pressed (held while scrubbing) cells, to the
/// right of the groove. It travels `POSBAR_W - POSBAR_THUMB_W` (219) px across the groove.
pub const POSBAR_THUMB_W: i32 = 29;
pub const POSBAR_THUMB_H: i32 = 10;
pub const POSBAR_THUMB_NORMAL: Rect = Rect::new(248, 0, POSBAR_THUMB_W, POSBAR_THUMB_H);
pub const POSBAR_THUMB_PRESSED: Rect = Rect::new(278, 0, POSBAR_THUMB_W, POSBAR_THUMB_H);

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
    fn slider_geometry_fits_the_window_and_sheet() {
        // Both sliders sit on the same row, inside the window, and their thumbs travel a
        // positive distance across the background (background wider than the thumb).
        assert_eq!(VOLUME_Y, BALANCE_Y, "volume and balance share a row");
        // Geometry invariants over compile-time constants: static-assert them so a bad edit
        // fails to compile rather than at test time.
        const { assert!(VOLUME_X + VOLUME_W <= BALANCE_X, "volume ends before balance begins") };
        const { assert!(BALANCE_X + BALANCE_W < MAIN_W, "balance stays inside the window") };
        const { assert!(VOLUME_W - SLIDER_THUMB_W > 0, "volume thumb travels a positive distance") };
        const { assert!(BALANCE_W - SLIDER_THUMB_W > 0, "balance thumb travels a positive distance") };
        // The background column is exactly SLIDER_FRAMES frames of SLIDER_FRAME_STRIDE px, and
        // the thumb sits just below it (y=422 = 28*15 + 2px gap).
        assert_eq!(SLIDER_FRAMES * SLIDER_FRAME_STRIDE, 420);
        assert_eq!(SLIDER_THUMB_NORMAL.y, 422);
        assert_eq!(SLIDER_THUMB_PRESSED.y, 422);
        assert_ne!(SLIDER_THUMB_NORMAL.x, SLIDER_THUMB_PRESSED.x, "held thumb is a distinct cell");
    }

    #[test]
    fn position_bar_geometry_matches_the_classic_layout() {
        // Sits inside the window with a groove wider than the thumb (a positive travel).
        const { assert!(POSBAR_X + POSBAR_W <= MAIN_W, "position bar stays inside the window") };
        const { assert!(POSBAR_W - POSBAR_THUMB_W > 0, "the thumb travels a positive distance") };
        assert_eq!(POSBAR_W - POSBAR_THUMB_W, 219, "classic travel");
        // The groove is the left edge of the sheet; the two thumb cells sit to its right, 30px
        // apart, distinct, and together imply the classic 307px sheet width.
        assert_eq!(POSBAR_BG, Rect::new(0, 0, 248, 10));
        assert_eq!(POSBAR_THUMB_NORMAL, Rect::new(248, 0, 29, 10));
        assert_eq!(POSBAR_THUMB_PRESSED, Rect::new(278, 0, 29, 10));
        assert_ne!(POSBAR_THUMB_NORMAL.x, POSBAR_THUMB_PRESSED.x, "held thumb is a distinct cell");
        assert_eq!(POSBAR_THUMB_PRESSED.x + POSBAR_THUMB_W, 307, "sheet is 307px wide");
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
