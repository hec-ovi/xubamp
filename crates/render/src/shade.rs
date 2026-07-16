//! Windowshade (collapsed) mode of the main window.
//!
//! Classic Winamp's windowshade collapses the main window to just its title strip (275x14): a
//! compact bar with the transport glyphs, a mini MM:SS clock, and a mini seek bar. Most of it is
//! baked into one background sprite; this module blits that strip plus the few live elements (a
//! held title button, the mini clock digits, the mini seek thumb) and owns the mini seek's value
//! math so the hit-testing in [`crate::hit`] and this compositor agree on one geometry. The toggle
//! itself (the windowshade title button, or a double-click on the title bar) lives in the `wl`
//! layer, which also resizes the toplevel.

use xubamp_skin::sprites::{self, Rect};
use xubamp_skin::{textfont, Skin};

use crate::hit::{self, UiState};
use crate::{blit, blit_placement, darken_rect, marquee, mmss_digits, Framebuffer};

/// Pixels the mini seek thumb travels across its trough: trough width minus thumb width, so the
/// thumb is flush-left at position 0.0 and flush-right at 1.0.
pub const SEEK_TRAVEL: i32 = sprites::SHADE_POSBAR_W - sprites::SHADE_POSBAR_THUMB_W;

/// Thumb x offset (0..=[`SEEK_TRAVEL`]) for a playback position `fraction` (0..=1), clamped.
pub fn seek_thumb_offset(fraction: f32) -> i32 {
    (fraction.clamp(0.0, 1.0) * SEEK_TRAVEL as f32).round() as i32
}

/// Inverse of [`seek_thumb_offset`]: the 0..=1 position for a window-local pointer x over the mini
/// seek bar, thumb centered on the cursor and clamped to the track (past either end pins to 0/1).
pub fn seek_from_x(x: i32) -> f32 {
    let offset =
        (x - sprites::SHADE_POSBAR_X - sprites::SHADE_POSBAR_THUMB_W / 2).clamp(0, SEEK_TRAVEL);
    offset as f32 / SEEK_TRAVEL as f32
}

/// Which mini-thumb cell suits `fraction`: the left/right end variants near the extremes, the
/// centre cell in between, matching Winamp's three-cell mini thumb.
fn seek_thumb(fraction: f32) -> Rect {
    if fraction <= 0.33 {
        sprites::SHADE_POSBAR_THUMB_LEFT
    } else if fraction >= 0.66 {
        sprites::SHADE_POSBAR_THUMB_RIGHT
    } else {
        sprites::SHADE_POSBAR_THUMB
    }
}

/// Compose the collapsed (windowshade) main window: the 275x14 title strip, a held title button's
/// pressed sprite, pressed feedback for the baked mini transport, the song title (scrolling in
/// its narrower strip when it overruns), the mini seek bar at the current position, and the mini
/// MM:SS clock. Missing sheets are skipped, as in the full compose.
pub fn compose(skin: &Skin, state: &UiState) -> Framebuffer {
    let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_SHADE_H as u32);
    // A malformed/incomplete skin must never map an all-transparent toplevel. Production's built-in
    // skin bakes its title bar into MAIN.BMP, so seed an opaque neutral strip, then use that baked
    // title when TITLEBAR.BMP is absent. A complete classic skin replaces it with the real shade
    // strip below.
    for pixel in fb.rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[14, 26, 34, 255]);
    }
    if let Some(titlebar) = &skin.titlebar {
        blit_placement(&mut fb, titlebar, sprites::SHADE_BG);
        // A held title button shows its shade-mode pressed sprite over the strip (the up art is
        // baked into the strip; the windowshade button's cell is the "restore" variant here).
        if let Some(b) = state.pressed_title {
            let idx = hit::TITLE_BUTTON_ORDER
                .iter()
                .position(|&t| t == b)
                .unwrap();
            blit_placement(&mut fb, titlebar, sprites::SHADE_TITLE_BUTTONS_PRESSED[idx]);
        }
        // Mini seek bar: the trough, then the thumb at the current position (0 when nothing loaded).
        let frac = state.position.unwrap_or(0.0);
        blit(
            &mut fb,
            titlebar,
            sprites::SHADE_POSBAR_BG,
            sprites::SHADE_POSBAR_X,
            sprites::SHADE_POSBAR_Y,
        );
        blit(
            &mut fb,
            titlebar,
            seek_thumb(frac),
            sprites::SHADE_POSBAR_X + seek_thumb_offset(frac),
            sprites::SHADE_POSBAR_Y,
        );
    } else if let Some(main) = &skin.main {
        // The clean-room built-in skin has no overlay sheets. Its top 14 rows are already an opaque
        // title strip, and make a safe compact fallback even though it cannot expose the optional
        // classic mini transport/seek artwork.
        blit(
            &mut fb,
            main,
            Rect::new(0, 0, sprites::MAIN_W, sprites::MAIN_SHADE_H),
            0,
            0,
        );
    }
    // A held mini transport button: the strip bakes only the up art, so darken its footprint for
    // pressed feedback, like the other artless pressed states.
    if let Some(held) = state.pressed {
        if let Some((&(bx, by, bw, bh), _)) = sprites::SHADE_TRANSPORT
            .iter()
            .zip(hit::TRANSPORT_ORDER)
            .find(|(_, t)| *t == held)
        {
            darken_rect(&mut fb, bx, by, bw, bh);
        }
    }
    // The song title in its strip between the menu button and the mini clock: left-aligned when
    // it fits, the looping marquee otherwise, like the expanded window.
    if let Some(text) = &skin.text {
        marquee::draw_in(
            &mut fb,
            text,
            &state.title,
            if state.scroll_title { state.marquee_offset } else { 0 },
            sprites::SHADE_TITLE_X,
            sprites::SHADE_TIME_Y,
            sprites::SHADE_TITLE_W,
        );
    }
    // Mini clock: the selected MM:SS representation in the small text.bmp font, blank when that
    // value is unavailable. Remaining mode uses the compact font's leading minus glyph. The four
    // digit cells sit at fixed x offsets, minutes then seconds. While paused the digits share the
    // classic blink with the expanded clock.
    if let (Some(text), Some(secs)) = (&skin.text, state.displayed_time()) {
        if !state.blink_hides() {
            if state.time_display == hit::TimeDisplay::Remaining {
                if let Some(cell) = textfont::cell('-') {
                    blit(&mut fb, text, cell, 128, sprites::SHADE_TIME_Y);
                }
            }
            for (&x, &d) in sprites::SHADE_TIME_DIGITS_X
                .iter()
                .zip(mmss_digits(secs).iter())
            {
                if let Some(cell) = textfont::cell((b'0' + d) as char) {
                    blit(&mut fb, text, cell, x, sprites::SHADE_TIME_Y);
                }
            }
        }
    }
    fb
}

#[cfg(test)]
mod tests {
    use super::*;
    use xubamp_skin::bmp::Image;
    use xubamp_skin::default_skin;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Image {
        Image {
            width: w,
            height: h,
            rgba: rgba
                .iter()
                .copied()
                .cycle()
                .take(w as usize * h as usize * 4)
                .collect(),
        }
    }

    fn px(fb: &Framebuffer, x: i32, y: i32) -> [u8; 4] {
        let o = ((y as u32 * fb.width + x as u32) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn shade_draws_the_title_clipped_to_its_strip() {
        // GREEN glyph cells over a BLUE strip: any drawn glyph pixel reads GREEN.
        let skin = Skin {
            titlebar: Some(solid(344, 87, BLUE)),
            text: Some(solid(155, 18, GREEN)),
            ..Default::default()
        };
        let state = UiState {
            shade: true,
            // 40 chars * 6px overruns the 118px strip, so the tail must clip.
            title: "A".repeat(40),
            ..Default::default()
        };
        let fb = compose(&skin, &state);
        assert_eq!(
            px(&fb, sprites::SHADE_TITLE_X, sprites::SHADE_TIME_Y),
            GREEN,
            "first glyph column drawn"
        );
        assert_eq!(
            px(
                &fb,
                sprites::SHADE_TITLE_X + sprites::SHADE_TITLE_W - 1,
                sprites::SHADE_TIME_Y
            ),
            GREEN,
            "strip filled to its clip edge"
        );
        assert_eq!(
            px(
                &fb,
                sprites::SHADE_TITLE_X + sprites::SHADE_TITLE_W,
                sprites::SHADE_TIME_Y
            ),
            BLUE,
            "nothing leaks past the clip edge"
        );
        // No title: the strip shows the background.
        let empty = compose(
            &skin,
            &UiState {
                shade: true,
                ..Default::default()
            },
        );
        assert_eq!(
            px(&empty, sprites::SHADE_TITLE_X, sprites::SHADE_TIME_Y),
            BLUE,
            "empty title draws nothing"
        );
    }

    #[test]
    fn shade_darkens_a_held_mini_transport_button() {
        let skin = Skin {
            titlebar: Some(solid(344, 87, WHITE)),
            ..Default::default()
        };
        let (bx, by, bw, bh) = sprites::SHADE_TRANSPORT[1]; // play
        let held = compose(
            &skin,
            &UiState {
                shade: true,
                pressed: Some(hit::Transport::Play),
                ..Default::default()
            },
        );
        let pressed_px = px(&held, bx + bw / 2, by + bh / 2);
        assert!(
            pressed_px[0] < 250 && pressed_px[0] > 0,
            "held button footprint darkened, got {pressed_px:?}"
        );
        assert_eq!(
            px(&held, bx - 2, by + bh / 2),
            WHITE,
            "neighbouring strip pixels untouched"
        );
        let released = compose(
            &skin,
            &UiState {
                shade: true,
                ..Default::default()
            },
        );
        assert_eq!(
            px(&released, bx + bw / 2, by + bh / 2),
            WHITE,
            "no feedback without a held button"
        );
    }

    #[test]
    fn seek_math_round_trips_and_clamps_to_the_mini_track() {
        assert_eq!(seek_thumb_offset(0.0), 0, "start is flush-left");
        assert_eq!(seek_thumb_offset(1.0), SEEK_TRAVEL, "end is flush-right");
        assert_eq!(seek_thumb_offset(-1.0), 0, "clamps below");
        assert_eq!(seek_thumb_offset(2.0), SEEK_TRAVEL, "clamps above");
        // Past either end pins to 0/1; the ends round-trip through the thumb-centred inverse.
        assert_eq!(seek_from_x(-1000), 0.0);
        assert_eq!(seek_from_x(10_000), 1.0);
        let x_start = sprites::SHADE_POSBAR_X + sprites::SHADE_POSBAR_THUMB_W / 2;
        let x_end = sprites::SHADE_POSBAR_X + SEEK_TRAVEL + sprites::SHADE_POSBAR_THUMB_W / 2;
        assert_eq!(seek_from_x(x_start), 0.0);
        assert_eq!(seek_from_x(x_end), 1.0);
    }

    #[test]
    fn thumb_cell_switches_at_the_ends() {
        assert_eq!(
            seek_thumb(0.0),
            sprites::SHADE_POSBAR_THUMB_LEFT,
            "left at the start"
        );
        assert_eq!(
            seek_thumb(0.5),
            sprites::SHADE_POSBAR_THUMB,
            "centre in the middle"
        );
        assert_eq!(
            seek_thumb(1.0),
            sprites::SHADE_POSBAR_THUMB_RIGHT,
            "right at the end"
        );
    }

    #[test]
    fn compose_returns_the_title_strip_size_even_with_a_bare_skin() {
        let fb = compose(&Skin::default(), &UiState::default());
        assert_eq!(
            (fb.width, fb.height),
            (275, 14),
            "shade collapses to the title strip"
        );
        assert!(
            fb.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255),
            "even an incomplete skin maps an opaque strip"
        );
        let builtin = compose(&default_skin(), &UiState::default());
        assert!(
            builtin.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255),
            "the package's built-in skin stays visible when shaded"
        );
    }

    #[test]
    fn compose_draws_the_strip_and_the_pressed_shade_button() {
        // A titlebar sheet: all BLUE, with the shade-strip source region (27,29,275,14) painted
        // GREEN and the shade-mode "restore" pressed cell (9,27,9,9) painted WHITE, so we can read
        // back which sprite landed where.
        let mut sheet = solid(344, 87, BLUE);
        let put = |sheet: &mut Image, x: u32, y: u32, c: [u8; 4]| {
            let o = ((y * sheet.width + x) * 4) as usize;
            sheet.rgba[o..o + 4].copy_from_slice(&c);
        };
        for y in 29..43u32 {
            for x in 27..302u32 {
                put(&mut sheet, x, y, GREEN);
            }
        }
        for y in 27..36u32 {
            for x in 9..18u32 {
                put(&mut sheet, x, y, WHITE);
            }
        }
        let skin = Skin {
            titlebar: Some(sheet),
            ..Default::default()
        };

        // Idle: the strip background shows through where no control sits, and the shade button area
        // shows the (green) strip, not a pressed sprite.
        let idle = compose(&skin, &UiState::default());
        assert_eq!(px(&idle, 100, 5), GREEN, "strip background drawn");
        assert_eq!(
            px(&idle, 254 + 4, 3 + 4),
            GREEN,
            "shade button idle shows the strip"
        );

        // Held shade button: its restore (WHITE) pressed sprite is drawn at (254,3,9,9).
        let held = UiState {
            shade: true,
            pressed_title: Some(hit::TitleButton::Shade),
            ..Default::default()
        };
        let fb = compose(&skin, &held);
        assert_eq!(
            px(&fb, 254 + 4, 3 + 4),
            WHITE,
            "held shade shows the restore sprite"
        );
    }

    #[test]
    fn compose_draws_the_mini_clock_digits() {
        // A text sheet where digit d's 5x6 cell (row 1: x=d*5, y=6) is a d-distinct red, so we can
        // read back which digit landed where.
        let mut text = solid(160, 18, [0, 0, 0, 255]);
        for d in 0..10u32 {
            let color = [(10 + d * 20) as u8, 0, 0, 255];
            for y in 6..12u32 {
                for x in d * 5..d * 5 + 5 {
                    let o = ((y * 160 + x) * 4) as usize;
                    text.rgba[o..o + 4].copy_from_slice(&color);
                }
            }
        }
        let minus = textfont::cell('-').expect("TEXT.BMP has a minus cell");
        for y in minus.y as u32..(minus.y + minus.h) as u32 {
            for x in minus.x as u32..(minus.x + minus.w) as u32 {
                let o = ((y * text.width + x) * 4) as usize;
                text.rgba[o..o + 4].copy_from_slice(&GREEN);
            }
        }
        let skin = Skin {
            text: Some(text),
            ..Default::default()
        };
        let color = |d: u32| [(10 + d * 20) as u8, 0, 0, 255];
        // elapsed 65s -> 01:05 -> digits [0,1,0,5] at the four x offsets, y=SHADE_TIME_Y.
        let state = UiState {
            shade: true,
            elapsed: Some(65),
            ..Default::default()
        };
        let fb = compose(&skin, &state);
        let xs = sprites::SHADE_TIME_DIGITS_X;
        let y = sprites::SHADE_TIME_Y + 2;
        assert_eq!(px(&fb, xs[0] + 2, y), color(0), "tens of minutes");
        assert_eq!(px(&fb, xs[1] + 2, y), color(1), "units of minutes");
        assert_eq!(px(&fb, xs[2] + 2, y), color(0), "tens of seconds");
        assert_eq!(px(&fb, xs[3] + 2, y), color(5), "units of seconds");

        // Nothing loaded: the clock stays blank (the background shows through).
        let blank = compose(
            &skin,
            &UiState {
                shade: true,
                ..Default::default()
            },
        );
        assert_eq!(
            px(&blank, xs[0] + 2, y),
            [14, 26, 34, 255],
            "blank clock leaves the opaque fallback strip visible"
        );

        // Remaining mode uses the same derived countdown and puts the font's minus glyph at x=128.
        let remaining = compose(
            &skin,
            &UiState {
                shade: true,
                time_display: hit::TimeDisplay::Remaining,
                elapsed: Some(135),
                duration: Some(200), // 01:05 remaining
                ..Default::default()
            },
        );
        assert_eq!(
            px(&remaining, 128 + 2, sprites::SHADE_TIME_Y + 2),
            GREEN,
            "compact remaining clock has a leading minus"
        );
        assert_eq!(px(&remaining, xs[0] + 2, y), color(0));
        assert_eq!(px(&remaining, xs[1] + 2, y), color(1));
        assert_eq!(px(&remaining, xs[2] + 2, y), color(0));
        assert_eq!(px(&remaining, xs[3] + 2, y), color(5));

        let unknown = compose(
            &skin,
            &UiState {
                shade: true,
                time_display: hit::TimeDisplay::Remaining,
                elapsed: Some(135),
                duration: None,
                ..Default::default()
            },
        );
        assert_eq!(
            px(&unknown, 128 + 2, sprites::SHADE_TIME_Y + 2),
            [14, 26, 34, 255],
            "unknown countdown has neither sign nor digits"
        );
    }

    #[test]
    fn mini_seek_thumb_moves_with_the_position() {
        // A titlebar sheet: BLUE, with the mini-thumb cells (x 17..26, y 36..43) painted GREEN so we
        // can find the thumb, and the trough (0..17) left BLUE.
        let mut sheet = solid(344, 87, BLUE);
        for y in 36..43u32 {
            for x in 17..26u32 {
                let o = ((y * sheet.width + x) * 4) as usize;
                sheet.rgba[o..o + 4].copy_from_slice(&GREEN);
            }
        }
        let skin = Skin {
            titlebar: Some(sheet),
            ..Default::default()
        };
        let y = sprites::SHADE_POSBAR_Y + 3;
        // Start: the thumb sits flush-left at the trough origin.
        let at_start = UiState {
            shade: true,
            position: Some(0.0),
            ..Default::default()
        };
        let fb = compose(&skin, &at_start);
        assert_eq!(
            px(&fb, sprites::SHADE_POSBAR_X + 1, y),
            GREEN,
            "thumb at the start"
        );
        // End: the thumb slides to the far end of the mini track.
        let at_end = UiState {
            shade: true,
            position: Some(1.0),
            ..Default::default()
        };
        let fb = compose(&skin, &at_end);
        let tx = sprites::SHADE_POSBAR_X + SEEK_TRAVEL;
        assert_eq!(px(&fb, tx + 1, y), GREEN, "thumb at the end");
        assert_ne!(
            px(&fb, sprites::SHADE_POSBAR_X + 1, y),
            GREEN,
            "start now bare trough"
        );
    }
}
