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

/// The four title-bar buttons, 9x9 each at y=3, from TITLEBAR.BMP. Their released (up) graphics are
/// baked into the title-bar strip above; only the pressed (down) sprite is blitted while a button
/// is held, so these placements carry the DOWN source rect and the on-window destination. Order:
/// options (main menu), minimize, windowshade, close. Source columns 0/9/18 at row 9 (down), except
/// the windowshade down sprite at (9,18). Destinations and sources cross-checked against Webamp.
pub const TITLE_BUTTONS_PRESSED: [Placement; 4] = [
    Placement::new(Rect::new(0, 9, 9, 9), 6, 3),    // options
    Placement::new(Rect::new(9, 9, 9, 9), 244, 3),  // minimize
    Placement::new(Rect::new(9, 18, 9, 9), 254, 3), // windowshade
    Placement::new(Rect::new(18, 9, 9, 9), 264, 3), // close
];

// --- Windowshade (collapsed) mode of the main window ---
//
// In shade mode the window is just the title strip: MAIN_W wide x MAIN_SHADE_H tall. The
// transport-button glyphs, the menu/minimize/shade/close up-art, and the mini clock/seek recesses
// are all baked into SHADE_BG; only a held title button's pressed sprite, the mini seek thumb, and
// the small text.bmp clock digits are drawn on top. Coordinates from TITLEBAR.BMP, cross-checked
// against Webamp's skinSprites.ts and main-window.css.

/// Shade strip height (the width stays [`MAIN_W`]).
pub const MAIN_SHADE_H: i32 = 14;

/// The shade title strip background (the focused variant, the one normally shown), from TITLEBAR.BMP.
pub const SHADE_BG: Placement = Placement::new(Rect::new(27, 29, 275, 14), 0, 0);

/// Title buttons in shade mode, parallel to [`TITLE_BUTTONS_PRESSED`] (same order and the same 9x9
/// on-window destinations, so the expanded hit regions carry over unchanged). Only the windowshade
/// button differs: in shade mode its pressed sprite is the "restore" cell at (9,27) rather than the
/// "collapse" cell at (9,18).
pub const SHADE_TITLE_BUTTONS_PRESSED: [Placement; 4] = [
    Placement::new(Rect::new(0, 9, 9, 9), 6, 3), // options (menu)
    Placement::new(Rect::new(9, 9, 9, 9), 244, 3), // minimize
    Placement::new(Rect::new(9, 27, 9, 9), 254, 3), // windowshade (restore)
    Placement::new(Rect::new(18, 9, 9, 9), 264, 3), // close
];

/// Click regions for the six transport buttons in shade mode, in TRANSPORT order (previous, play,
/// pause, stop, next, eject), each `(x, y, w, h)`. Their artwork is baked into [`SHADE_BG`], so
/// these are hit targets only: nothing extra is blitted and there is no pressed state. Coordinates
/// from Webamp's shade-view CSS.
pub const SHADE_TRANSPORT: [(i32, i32, i32, i32); 6] = [
    (169, 2, 7, 10),  // previous
    (176, 2, 10, 10), // play
    (186, 2, 9, 10),  // pause
    (195, 2, 9, 10),  // stop
    (204, 2, 10, 10), // next
    (215, 2, 10, 10), // eject
];

/// The mini seek/position bar in shade mode: a 17px trough at (226,4) with a 3px thumb sliding
/// across it (travel = width - thumb). The thumb swaps to a left/right end variant near the
/// extremes, matching Winamp's three-cell mini thumb. All from TITLEBAR.BMP.
pub const SHADE_POSBAR_X: i32 = 226;
pub const SHADE_POSBAR_Y: i32 = 4;
pub const SHADE_POSBAR_W: i32 = 17;
pub const SHADE_POSBAR_H: i32 = 7;
pub const SHADE_POSBAR_BG: Rect = Rect::new(0, 36, 17, 7);
pub const SHADE_POSBAR_THUMB_W: i32 = 3;
pub const SHADE_POSBAR_THUMB_H: i32 = 7;
pub const SHADE_POSBAR_THUMB: Rect = Rect::new(20, 36, 3, 7); // centre (34%-65%)
pub const SHADE_POSBAR_THUMB_LEFT: Rect = Rect::new(17, 36, 3, 7); // <=33%
pub const SHADE_POSBAR_THUMB_RIGHT: Rect = Rect::new(23, 36, 3, 7); // >=66%

/// The mini MM:SS clock in shade mode, drawn from the small TEXT.BMP font (5x6 digits). Window x of
/// the four digits (tens/units of minutes, then of seconds); the classic layout leaves a colon-width
/// gap between the minute and second pairs. Elapsed only, so there is no leading sign cell.
pub const SHADE_TIME_Y: i32 = 4;
pub const SHADE_TIME_DIGITS_X: [i32; 4] = [134, 139, 147, 152];

/// The song-title strip in shade mode: the classic layout shows the title in the small TEXT.BMP
/// font between the menu button and the mini clock, clipped (not scrolled). The glyph row shares
/// the clock's y.
pub const SHADE_TITLE_X: i32 = 8;
pub const SHADE_TITLE_W: i32 = 118;

/// The six transport buttons from CBUTTONS.BMP (normal state, top row), in order:
/// previous, play, pause, stop, next, eject.
pub const CBUTTONS: [Placement; 6] = [
    Placement::new(Rect::new(0, 0, 23, 18), 16, 88), // previous
    Placement::new(Rect::new(23, 0, 23, 18), 39, 88), // play
    Placement::new(Rect::new(46, 0, 23, 18), 62, 88), // pause
    Placement::new(Rect::new(69, 0, 23, 18), 85, 88), // stop
    Placement::new(Rect::new(92, 0, 22, 18), 108, 88), // next
    Placement::new(Rect::new(114, 0, 22, 16), 136, 89), // eject
];

/// The same six buttons in their pressed state (the bottom row of CBUTTONS.BMP), same
/// destinations. Each source rect is the normal one shifted down by its own height, so the
/// pressed art sits directly below the normal art: 18px for the first five, 16px for the
/// shorter eject button (whose pressed art is one pixel higher, per the classic sheet).
pub const CBUTTONS_PRESSED: [Placement; 6] = [
    Placement::new(Rect::new(0, 18, 23, 18), 16, 88), // previous
    Placement::new(Rect::new(23, 18, 23, 18), 39, 88), // play
    Placement::new(Rect::new(46, 18, 23, 18), 62, 88), // pause
    Placement::new(Rect::new(69, 18, 23, 18), 85, 88), // stop
    Placement::new(Rect::new(92, 18, 22, 18), 108, 88), // next
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

/// The kbps (bitrate) and kHz (sample-rate) readouts are drawn with the small `text.bmp` font
/// (5x6 digits, abutting), NOT the big time digits. `kbps` is a 3-digit field at (111,43), `kHz` a
/// 2-digit field at (156,43); each digit advances 5px and is left-aligned in its field, clipped to
/// the digit count. Coordinates cross-checked against Webamp's main-window CSS.
pub const KBPS_X: i32 = 111;
pub const KBPS_Y: i32 = 43;
pub const KBPS_DIGITS: usize = 3;
pub const KHZ_X: i32 = 156;
pub const KHZ_Y: i32 = 43;
pub const KHZ_DIGITS: usize = 2;

/// The play/pause/stop status indicator (`playpaus.bmp`, 42x9). The three 9x9 status glyphs sit
/// at source columns 0 (play), 9 (pause), and 18 (stop) and draw at (26,28), left of the time
/// display. The 3px-wide work-indicator columns sit at source x=36 (idle) and x=39 (busy) and
/// draw at (24,28), butted against the status glyph. Cross-checked against Webamp's
/// skinSprites.ts and main-window.css (which clips the work cell to 3px).
pub const STATUS_PLAYING: Placement = Placement::new(Rect::new(0, 0, 9, 9), 26, 28);
pub const STATUS_PAUSED: Placement = Placement::new(Rect::new(9, 0, 9, 9), 26, 28);
pub const STATUS_STOPPED: Placement = Placement::new(Rect::new(18, 0, 9, 9), 26, 28);
pub const STATUS_WORK_IDLE: Placement = Placement::new(Rect::new(36, 0, 3, 9), 24, 28);

/// The mono/stereo indicator (`monoster.bmp`, 56x24): the lit words are the top row (y=0), the dim
/// words the bottom row (y=12); the left block (29px) is "stereo", the right (27px) is "mono". On
/// the window, "mono" sits at (212,41) and "stereo" at (239,41). Both are always drawn; the one
/// matching the track's channel count is lit, the other dim. Each entry is (source rect, dest).
pub const MONO_LIT: Placement = Placement::new(Rect::new(29, 0, 27, 12), 212, 41);
pub const MONO_UNLIT: Placement = Placement::new(Rect::new(29, 12, 27, 12), 212, 41);
pub const STEREO_LIT: Placement = Placement::new(Rect::new(0, 0, 29, 12), 239, 41);
pub const STEREO_UNLIT: Placement = Placement::new(Rect::new(0, 12, 29, 12), 239, 41);

/// The EQ and PL toggle buttons on the main window, from `shufrep.bmp` (23x12 each). The "lit"
/// variant is the lower row (the window is open); each has a pressed column at +46px. Dests are the
/// classic (219,58) for EQ and (242,58) for PL. Cross-checked against Webamp.
pub const EQ_OFF: Placement = Placement::new(Rect::new(0, 61, 23, 12), 219, 58);
pub const EQ_OFF_PRESSED: Placement = Placement::new(Rect::new(46, 61, 23, 12), 219, 58);
pub const EQ_ON: Placement = Placement::new(Rect::new(0, 73, 23, 12), 219, 58);
pub const EQ_ON_PRESSED: Placement = Placement::new(Rect::new(46, 73, 23, 12), 219, 58);
pub const PL_OFF: Placement = Placement::new(Rect::new(23, 61, 23, 12), 242, 58);
pub const PL_OFF_PRESSED: Placement = Placement::new(Rect::new(69, 61, 23, 12), 242, 58);
pub const PL_ON: Placement = Placement::new(Rect::new(23, 73, 23, 12), 242, 58);
pub const PL_ON_PRESSED: Placement = Placement::new(Rect::new(69, 73, 23, 12), 242, 58);

/// The shuffle (47x15) and repeat (28x15) mode buttons on the main window, from `shufrep.bmp`. The
/// "on" variant is the lower rows (the mode is enabled); each has a pressed row just below it. Dests
/// are the classic (164,89) for shuffle and (210,89) for repeat.
pub const SHUFFLE_OFF: Placement = Placement::new(Rect::new(28, 0, 47, 15), 164, 89);
pub const SHUFFLE_OFF_PRESSED: Placement = Placement::new(Rect::new(28, 15, 47, 15), 164, 89);
pub const SHUFFLE_ON: Placement = Placement::new(Rect::new(28, 30, 47, 15), 164, 89);
pub const SHUFFLE_ON_PRESSED: Placement = Placement::new(Rect::new(28, 45, 47, 15), 164, 89);
pub const REPEAT_OFF: Placement = Placement::new(Rect::new(0, 0, 28, 15), 210, 89);
pub const REPEAT_OFF_PRESSED: Placement = Placement::new(Rect::new(0, 15, 28, 15), 210, 89);
pub const REPEAT_ON: Placement = Placement::new(Rect::new(0, 30, 28, 15), 210, 89);
pub const REPEAT_ON_PRESSED: Placement = Placement::new(Rect::new(0, 45, 28, 15), 210, 89);

// --- The equalizer (EQMAIN / optional EQ_EX) window. ---
//
// The expanded equalizer has the same fixed dimensions as the main window. EQMAIN holds a complete
// background followed by title/control sprites and a 2x14 grid of value-dependent slider frames.
// EQ_EX is optional and supplies the compact title strip, its tiny volume/balance thumbs, and the
// pressed shade/close buttons. Coordinates match the classic skin sheet layout.

/// Expanded equalizer size and its fully collapsed height.
pub const EQ_W: i32 = 275;
pub const EQ_H: i32 = 116;
pub const EQ_SHADE_H: i32 = 14;

/// Expanded background and focused/unfocused title strips from EQMAIN.BMP.
pub const EQ_BACKGROUND: Placement = Placement::new(Rect::new(0, 0, EQ_W, EQ_H), 0, 0);
pub const EQ_TITLE_ACTIVE: Placement = Placement::new(Rect::new(0, 134, EQ_W, 14), 0, 0);
pub const EQ_TITLE_INACTIVE: Placement = Placement::new(Rect::new(0, 149, EQ_W, 14), 0, 0);

/// Expanded title-button hit/draw geometry. Released artwork is present in the title strip.
pub const EQ_TITLE_BUTTON_Y: i32 = 3;
pub const EQ_TITLE_BUTTON_W: i32 = 9;
pub const EQ_SHADE_BUTTON_X: i32 = 254;
pub const EQ_CLOSE_BUTTON_X: i32 = 264;
pub const EQ_CLOSE: Rect = Rect::new(0, 116, 9, 9);
pub const EQ_CLOSE_PRESSED: Rect = Rect::new(0, 125, 9, 9);
/// Used when an expanded skin omits EQ_EX: this cell exposes the pressed-looking shade area from
/// EQMAIN itself instead of borrowing unrelated default-skin art.
pub const EQ_SHADE_PRESSED_FALLBACK: Rect = Rect::new(254, 152, 9, 9);

/// Expanded ON and AUTO buttons. AUTO is drawn from its ordinary cell but intentionally has no
/// action in xubamp; the renderer dims it so unsupported automatic presets are unambiguous.
pub const EQMAIN_ON: Placement = Placement::new(Rect::new(10, 119, 26, 12), 14, 18);
pub const EQMAIN_ON_PRESSED: Placement = Placement::new(Rect::new(128, 119, 26, 12), 14, 18);
pub const EQMAIN_ON_SELECTED: Placement = Placement::new(Rect::new(69, 119, 26, 12), 14, 18);
pub const EQMAIN_ON_SELECTED_PRESSED: Placement =
    Placement::new(Rect::new(187, 119, 26, 12), 14, 18);
pub const EQ_AUTO: Placement = Placement::new(Rect::new(36, 119, 32, 12), 40, 18);

/// Presets button, which opens the native preset menu/dialog flow in the platform layer.
pub const EQ_PRESETS: Placement = Placement::new(Rect::new(224, 164, 44, 12), 217, 18);
pub const EQ_PRESETS_PRESSED: Placement = Placement::new(Rect::new(224, 176, 44, 12), 217, 18);

/// The 113x19 response graph and its palette/one-pixel preamp line sources.
pub const EQ_GRAPH: Placement = Placement::new(Rect::new(0, 294, 113, 19), 86, 17);
pub const EQ_GRAPH_COLORS: Rect = Rect::new(115, 294, 1, 19);
pub const EQ_PREAMP_LINE: Rect = Rect::new(0, 314, 113, 1);

/// Slider containers: preamp followed by the ten frequency bands. Each is 14x63 on screen.
pub const EQ_SLIDER_Y: i32 = 38;
pub const EQ_PREAMP_X: i32 = 21;
pub const EQ_BAND_X: [i32; 10] = [78, 96, 114, 132, 150, 168, 186, 204, 222, 240];
pub const EQ_SLIDER_W: i32 = 14;
pub const EQ_SLIDER_H: i32 = 63;
pub const EQ_SLIDER_TRACK_H: i32 = 62;

/// EQMAIN's 28 slider-background frames: 14 columns by 2 rows, spaced 15px horizontally and 65px
/// vertically. A frame itself is 14x63. Frame zero is -12 dB; frame 27 is +12 dB.
pub const EQ_SLIDER_GRID: Rect = Rect::new(13, 164, 209, 129);
pub const EQ_SLIDER_FRAMES: i32 = 28;
pub const EQ_SLIDER_COLUMNS: i32 = 14;
pub const EQ_SLIDER_X_STRIDE: i32 = 15;
pub const EQ_SLIDER_Y_STRIDE: i32 = 65;
pub const EQ_SLIDER_THUMB: Rect = Rect::new(0, 164, 11, 11);
pub const EQ_SLIDER_THUMB_PRESSED: Rect = Rect::new(0, 176, 11, 11);
pub const EQ_SLIDER_THUMB_DX: i32 = 1;
pub const EQ_SLIDER_THUMB_TRAVEL: i32 = EQ_SLIDER_TRACK_H - EQ_SLIDER_THUMB.h;

/// Optional EQ_EX sheet: focused/unfocused compact backgrounds and value-segmented tiny thumbs.
pub const EQ_EX_SHADE_ACTIVE: Placement = Placement::new(Rect::new(0, 0, EQ_W, 14), 0, 0);
pub const EQ_EX_SHADE_INACTIVE: Placement = Placement::new(Rect::new(0, 15, EQ_W, 14), 0, 0);
pub const EQ_EX_VOLUME_THUMBS: [Rect; 3] = [
    Rect::new(1, 30, 3, 7),
    Rect::new(4, 30, 3, 7),
    Rect::new(7, 30, 3, 7),
];
pub const EQ_EX_BALANCE_THUMBS: [Rect; 3] = [
    Rect::new(11, 30, 3, 7),
    Rect::new(14, 30, 3, 7),
    Rect::new(17, 30, 3, 7),
];
pub const EQ_EX_SHADE_PRESSED: Rect = Rect::new(1, 38, 9, 9);
pub const EQ_EX_RESTORE_PRESSED: Rect = Rect::new(1, 47, 9, 9);
pub const EQ_EX_CLOSE: Rect = Rect::new(11, 38, 9, 9);
pub const EQ_EX_CLOSE_PRESSED: Rect = Rect::new(11, 47, 9, 9);

/// Compact volume/balance tracks. Their background is baked into the EQ_EX shade strip; only a
/// 3x7 thumb moves across each track.
pub const EQ_SHADE_VOLUME_X: i32 = 61;
pub const EQ_SHADE_VOLUME_Y: i32 = 4;
pub const EQ_SHADE_VOLUME_W: i32 = 97;
pub const EQ_SHADE_BALANCE_X: i32 = 164;
pub const EQ_SHADE_BALANCE_Y: i32 = 4;
pub const EQ_SHADE_BALANCE_W: i32 = 43;
pub const EQ_SHADE_THUMB_W: i32 = 3;
pub const EQ_SHADE_THUMB_H: i32 = 7;

// --- The playlist editor (PLEDIT) window, from pledit.bmp. Built from tiles so it can resize.
// Coordinates cross-checked against Webamp. ---

/// The playlist window's default expanded size, same width and height as the main window. This is
/// also the minimum expanded size; the window only ever grows from here.
pub const PLEDIT_W: i32 = 275;
pub const PLEDIT_H: i32 = 116;

/// The playlist window's fully collapsed height. Its width is preserved while shaded and can still
/// be resized horizontally.
pub const PLEDIT_SHADE_H: i32 = 14;

/// Classic Winamp resizes the playlist in whole segments: 25px wider or 29px taller at a time
/// (`WINDOW_RESIZE_SEGMENT_WIDTH`/`_HEIGHT` in Webamp). We render at whatever size the Wayland
/// compositor hands us (the tiles clip cleanly to any size), so these document the classic segment
/// dimensions rather than gating the resize.
pub const PLEDIT_SEGMENT_W: i32 = 25;
pub const PLEDIT_SEGMENT_H: i32 = 29;

/// Title-bar band height, side-edge band height (a vertical tile), and bottom-bar height.
pub const PLEDIT_TITLE_H: i32 = 20;
pub const PLEDIT_BOTTOM_H: i32 = 38;

/// Title bar (focused variant, y=0 row): corners, the centered "PLAYLIST" title, and the repeating
/// fill tile between them.
pub const PLEDIT_TOP_LEFT: Rect = Rect::new(0, 0, 25, 20);
pub const PLEDIT_TITLE: Rect = Rect::new(26, 0, 100, 20);
pub const PLEDIT_TOP_TILE: Rect = Rect::new(127, 0, 25, 20);
pub const PLEDIT_TOP_RIGHT: Rect = Rect::new(153, 0, 25, 20);

/// Pressed title-button cells. Their released artwork is baked into the expanded top-right tile and
/// the shaded right cap. Destinations are relative to the right edge because the playlist is
/// horizontally resizable: close is 2px from the edge, shade/restore is 12px from it.
pub const PLEDIT_CLOSE_PRESSED: Rect = Rect::new(52, 42, 9, 9);
pub const PLEDIT_COLLAPSE_PRESSED: Rect = Rect::new(62, 42, 9, 9);
pub const PLEDIT_EXPAND_PRESSED: Rect = Rect::new(150, 42, 9, 9);
pub const PLEDIT_TITLE_BUTTON_Y: i32 = 3;
pub const PLEDIT_TITLE_BUTTON_W: i32 = 9;
pub const PLEDIT_CLOSE_BUTTON_RIGHT: i32 = 2;
pub const PLEDIT_SHADE_BUTTON_RIGHT: i32 = 12;

/// Side edges, repeated vertically down the middle band (left 12px wide, right 20px wide).
pub const PLEDIT_LEFT_TILE: Rect = Rect::new(0, 42, 12, 29);
pub const PLEDIT_RIGHT_TILE: Rect = Rect::new(31, 42, 20, 29);

/// Bottom bar: the left corner (125px) holds the button cluster, the right corner (150px) the
/// time/scroll area. At the default width they meet exactly; when the window is wider,
/// `PLEDIT_BOTTOM_TILE` repeats between them.
pub const PLEDIT_BOTTOM_LEFT: Rect = Rect::new(0, 72, 125, 38);
pub const PLEDIT_BOTTOM_RIGHT: Rect = Rect::new(126, 72, 150, 38);
pub const PLEDIT_BOTTOM_TILE: Rect = Rect::new(179, 0, 25, 38);

/// Playlist windowshade tiles. The 25px background repeats across the strip; the 25px left cap and
/// 50px focused right cap overlay its ends. All live in `pledit.bmp`.
pub const PLEDIT_SHADE_TILE: Rect = Rect::new(72, 57, 25, 14);
pub const PLEDIT_SHADE_LEFT: Rect = Rect::new(72, 42, 25, 14);
pub const PLEDIT_SHADE_RIGHT: Rect = Rect::new(99, 42, 50, 14);

/// The width-only resize target in playlist windowshade mode: a 9px square 20px from the right.
/// It sits immediately left of the shade button and has no separate artwork.
pub const PLEDIT_SHADE_RESIZE_RIGHT: i32 = 20;

/// The track-list content rectangle within the window (between the edges and the title/bottom
/// bands): x 12..255 (width 243), y from 23, rows [`PLEDIT_ROW_H`] tall.
pub const PLEDIT_LIST_X: i32 = 12;
pub const PLEDIT_LIST_Y: i32 = 23;
pub const PLEDIT_LIST_W: i32 = PLEDIT_W - 12 - 20; // right edge is 20px wide
pub const PLEDIT_ROW_H: i32 = 13;

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

/// The visualizer region on the main window: a 76x16 recess at (24, 43), between the title bar and
/// the transport buttons, where MAIN.BMP leaves a dark panel. The spectrum/oscilloscope draw the
/// left 75 columns (the 76th stays background), coloured from `viscolor.txt`. Coordinates
/// cross-checked against Webamp.
pub const VIS_X: i32 = 24;
pub const VIS_Y: i32 = 43;
pub const VIS_W: i32 = 76;
pub const VIS_H: i32 = 16;

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
            assert_eq!(
                *r,
                Rect::new(d as i32 * DIGIT_W, 0, DIGIT_W, DIGIT_H),
                "digit {d}"
            );
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
        assert_eq!(
            TIME_DIGITS[1].0 - TIME_DIGITS[0].0,
            12,
            "step within the MM pair"
        );
        assert_eq!(
            TIME_DIGITS[3].0 - TIME_DIGITS[2].0,
            12,
            "step within the SS pair"
        );
        assert_eq!(
            TIME_DIGITS[2].0 - TIME_DIGITS[1].0,
            18,
            "MM->SS spans the colon gap"
        );
    }

    #[test]
    fn slider_geometry_fits_the_window_and_sheet() {
        // Both sliders sit on the same row, inside the window, and their thumbs travel a
        // positive distance across the background (background wider than the thumb).
        assert_eq!(VOLUME_Y, BALANCE_Y, "volume and balance share a row");
        // Geometry invariants over compile-time constants: static-assert them so a bad edit
        // fails to compile rather than at test time.
        const {
            assert!(
                VOLUME_X + VOLUME_W <= BALANCE_X,
                "volume ends before balance begins"
            )
        };
        const {
            assert!(
                BALANCE_X + BALANCE_W < MAIN_W,
                "balance stays inside the window"
            )
        };
        const {
            assert!(
                VOLUME_W - SLIDER_THUMB_W > 0,
                "volume thumb travels a positive distance"
            )
        };
        const {
            assert!(
                BALANCE_W - SLIDER_THUMB_W > 0,
                "balance thumb travels a positive distance"
            )
        };
        // The background column is exactly SLIDER_FRAMES frames of SLIDER_FRAME_STRIDE px, and
        // the thumb sits just below it (y=422 = 28*15 + 2px gap).
        assert_eq!(SLIDER_FRAMES * SLIDER_FRAME_STRIDE, 420);
        assert_eq!(SLIDER_THUMB_NORMAL.y, 422);
        assert_eq!(SLIDER_THUMB_PRESSED.y, 422);
        assert_ne!(
            SLIDER_THUMB_NORMAL.x, SLIDER_THUMB_PRESSED.x,
            "held thumb is a distinct cell"
        );
    }

    #[test]
    fn title_bar_buttons_sit_in_the_top_strip() {
        assert_eq!(TITLE_BUTTONS_PRESSED.len(), 4);
        for p in &TITLE_BUTTONS_PRESSED {
            // Each button is 9x9 within the 14px title-bar band.
            assert_eq!((p.src.w, p.src.h), (9, 9));
            assert!(
                p.dst_y >= 0 && p.dst_y + p.src.h <= TITLEBAR_ACTIVE.src.h,
                "inside the band"
            );
            assert!(
                p.dst_x >= 0 && p.dst_x + p.src.w <= MAIN_W,
                "inside the window"
            );
        }
        // Close is the far-right button; options the far-left.
        assert_eq!(
            (
                TITLE_BUTTONS_PRESSED[3].dst_x,
                TITLE_BUTTONS_PRESSED[3].dst_y
            ),
            (264, 3)
        );
        assert_eq!(
            (
                TITLE_BUTTONS_PRESSED[0].dst_x,
                TITLE_BUTTONS_PRESSED[0].dst_y
            ),
            (6, 3)
        );
        // Minimize sits just left of close.
        assert_eq!(TITLE_BUTTONS_PRESSED[1].dst_x, 244);
    }

    #[test]
    fn windowshade_strip_geometry_stays_inside_the_collapsed_bar() {
        assert_eq!(MAIN_SHADE_H, 14);
        assert_eq!(SHADE_BG.src, Rect::new(27, 29, 275, 14));
        // The shade title buttons mirror the expanded ones in count, order and destination; only the
        // windowshade button's source cell differs (restore vs collapse), so the expanded hit regions
        // carry over unchanged.
        assert_eq!(
            SHADE_TITLE_BUTTONS_PRESSED.len(),
            TITLE_BUTTONS_PRESSED.len()
        );
        for (shade, normal) in SHADE_TITLE_BUTTONS_PRESSED
            .iter()
            .zip(TITLE_BUTTONS_PRESSED.iter())
        {
            assert_eq!(
                (shade.dst_x, shade.dst_y),
                (normal.dst_x, normal.dst_y),
                "same destination"
            );
            assert!(
                shade.dst_y + shade.src.h <= MAIN_SHADE_H,
                "button inside the strip"
            );
        }
        assert_eq!(
            SHADE_TITLE_BUTTONS_PRESSED[2].src,
            Rect::new(9, 27, 9, 9),
            "shade uses the restore cell"
        );
        assert_ne!(
            SHADE_TITLE_BUTTONS_PRESSED[2].src, TITLE_BUTTONS_PRESSED[2].src,
            "distinct from collapse"
        );
        // The six mini transport targets sit in the strip, in order, and do not overlap.
        assert_eq!(SHADE_TRANSPORT.len(), 6);
        for &(_, ry, _, rh) in &SHADE_TRANSPORT {
            assert!(ry + rh <= MAIN_SHADE_H, "transport target inside the strip");
        }
        for pair in SHADE_TRANSPORT.windows(2) {
            assert!(
                pair[0].0 + pair[0].2 <= pair[1].0,
                "transport targets are disjoint and ordered"
            );
        }
        // The mini seek bar sits in the strip and its thumb travels a positive distance.
        const {
            assert!(
                SHADE_POSBAR_Y + SHADE_POSBAR_H <= MAIN_SHADE_H,
                "seek bar inside the strip"
            )
        };
        const {
            assert!(
                SHADE_POSBAR_W - SHADE_POSBAR_THUMB_W > 0,
                "the mini thumb travels a positive distance"
            )
        };
        // The mini clock digits sit in the strip, clear of both the seek bar and the transport row.
        assert_eq!(SHADE_TIME_DIGITS_X.len(), 4);
        for &x in &SHADE_TIME_DIGITS_X {
            assert!(
                x + 5 <= SHADE_TRANSPORT[0].0,
                "clock is left of the transport glyphs"
            );
        }
    }

    #[test]
    fn playlist_windowshade_cells_and_dynamic_controls_fit_the_strip() {
        assert_eq!(PLEDIT_SHADE_H, 14);
        assert_eq!(PLEDIT_SHADE_TILE, Rect::new(72, 57, 25, 14));
        assert_eq!(PLEDIT_SHADE_LEFT, Rect::new(72, 42, 25, 14));
        assert_eq!(PLEDIT_SHADE_RIGHT, Rect::new(99, 42, 50, 14));
        for cell in [
            PLEDIT_CLOSE_PRESSED,
            PLEDIT_COLLAPSE_PRESSED,
            PLEDIT_EXPAND_PRESSED,
        ] {
            assert_eq!((cell.w, cell.h), (9, 9));
            assert!(PLEDIT_TITLE_BUTTON_Y + cell.h <= PLEDIT_SHADE_H);
        }
        assert_eq!(PLEDIT_CLOSE_BUTTON_RIGHT, 2);
        assert_eq!(PLEDIT_SHADE_BUTTON_RIGHT, 12);
        assert_eq!(PLEDIT_SHADE_RESIZE_RIGHT, 20);
    }

    #[test]
    fn equalizer_geometry_matches_the_classic_sheets() {
        assert_eq!((EQ_W, EQ_H, EQ_SHADE_H), (275, 116, 14));
        assert_eq!(EQ_BACKGROUND.src, Rect::new(0, 0, 275, 116));
        assert_eq!(EQ_TITLE_ACTIVE.src, Rect::new(0, 134, 275, 14));
        assert_eq!((EQMAIN_ON.dst_x, EQMAIN_ON.dst_y), (14, 18));
        assert_eq!((EQ_AUTO.dst_x, EQ_AUTO.dst_y), (40, 18));
        assert_eq!((EQ_PRESETS.dst_x, EQ_PRESETS.dst_y), (217, 18));
        assert_eq!((EQ_GRAPH.dst_x, EQ_GRAPH.dst_y), (86, 17));
        assert_eq!(EQ_PREAMP_X, 21);
        assert_eq!(EQ_BAND_X, [78, 96, 114, 132, 150, 168, 186, 204, 222, 240]);
        assert_eq!(EQ_SLIDER_GRID, Rect::new(13, 164, 209, 129));
        assert_eq!(EQ_SLIDER_THUMB_TRAVEL, 51);

        for &x in EQ_BAND_X.iter().chain(core::iter::once(&EQ_PREAMP_X)) {
            assert!(x >= 0 && x + EQ_SLIDER_W <= EQ_W);
        }
        const { assert!(EQ_SLIDER_Y + EQ_SLIDER_H <= EQ_H) };
    }

    #[test]
    fn equalizer_extension_cells_fit_the_compact_strip() {
        assert_eq!(EQ_EX_SHADE_ACTIVE.src, Rect::new(0, 0, 275, 14));
        assert_eq!(EQ_EX_SHADE_INACTIVE.src, Rect::new(0, 15, 275, 14));
        assert_eq!(EQ_EX_SHADE_PRESSED, Rect::new(1, 38, 9, 9));
        assert_eq!(EQ_EX_RESTORE_PRESSED, Rect::new(1, 47, 9, 9));
        assert_eq!(EQ_EX_CLOSE_PRESSED, Rect::new(11, 47, 9, 9));
        for thumb in EQ_EX_VOLUME_THUMBS
            .iter()
            .chain(EQ_EX_BALANCE_THUMBS.iter())
        {
            assert_eq!((thumb.w, thumb.h), (EQ_SHADE_THUMB_W, EQ_SHADE_THUMB_H));
        }
        const { assert!(EQ_SHADE_VOLUME_Y + EQ_SHADE_THUMB_H <= EQ_SHADE_H) };
        const { assert!(EQ_SHADE_BALANCE_Y + EQ_SHADE_THUMB_H <= EQ_SHADE_H) };
        const { assert!(EQ_SHADE_BALANCE_X + EQ_SHADE_BALANCE_W < EQ_SHADE_BUTTON_X) };
    }

    #[test]
    fn visualizer_region_sits_between_the_title_bar_and_transport() {
        // Inside the window, below the title-bar band, above the transport buttons (y 88).
        const {
            assert!(
                VIS_X + VIS_W <= MAIN_W,
                "visualizer stays inside the window"
            )
        };
        const { assert!(VIS_Y >= 14, "below the title-bar band") };
        const { assert!(VIS_Y + VIS_H <= 88, "above the transport button row") };
        // It does not overlap the volume slider's column (they share rows but not x).
        const { assert!(VIS_X + VIS_W <= VOLUME_X, "clear of the volume slider in x") };
        assert_eq!((VIS_X, VIS_Y, VIS_W, VIS_H), (24, 43, 76, 16));
    }

    #[test]
    fn position_bar_geometry_matches_the_classic_layout() {
        // Sits inside the window with a groove wider than the thumb (a positive travel).
        const {
            assert!(
                POSBAR_X + POSBAR_W <= MAIN_W,
                "position bar stays inside the window"
            )
        };
        const {
            assert!(
                POSBAR_W - POSBAR_THUMB_W > 0,
                "the thumb travels a positive distance"
            )
        };
        assert_eq!(POSBAR_W - POSBAR_THUMB_W, 219, "classic travel");
        // The groove is the left edge of the sheet; the two thumb cells sit to its right, 30px
        // apart, distinct, and together imply the classic 307px sheet width.
        assert_eq!(POSBAR_BG, Rect::new(0, 0, 248, 10));
        assert_eq!(POSBAR_THUMB_NORMAL, Rect::new(248, 0, 29, 10));
        assert_eq!(POSBAR_THUMB_PRESSED, Rect::new(278, 0, 29, 10));
        assert_ne!(
            POSBAR_THUMB_NORMAL.x, POSBAR_THUMB_PRESSED.x,
            "held thumb is a distinct cell"
        );
        assert_eq!(
            POSBAR_THUMB_PRESSED.x + POSBAR_THUMB_W,
            307,
            "sheet is 307px wide"
        );
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
