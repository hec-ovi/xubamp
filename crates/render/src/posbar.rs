//! The position (seek) bar.
//!
//! A single-row groove background with a draggable thumb sliding over it, both from `posbar.bmp`.
//! This module holds the pure value math (where the thumb sits for a 0..=1 position, and the
//! inverse: what fraction a click x maps to) and the draw, so the hit-testing in [`crate::hit`]
//! and the compositor agree on one geometry. Unlike the volume/balance sliders, the seek commits
//! on release, so the value here only places and reads the thumb. Geometry matches Webamp's
//! classic main window.

use xubamp_skin::bmp::Image;
use xubamp_skin::sprites;

use crate::{blit, Framebuffer};

/// Pixels the thumb travels across the groove: groove width minus thumb width (219), so the thumb
/// is flush-left at position 0.0 and flush-right at 1.0.
pub const POSBAR_TRAVEL: i32 = sprites::POSBAR_W - sprites::POSBAR_THUMB_W;

/// Thumb x offset (0..=[`POSBAR_TRAVEL`]) for a playback position `fraction` (0..=1), clamped.
pub fn position_thumb_offset(fraction: f32) -> i32 {
    (fraction.clamp(0.0, 1.0) * POSBAR_TRAVEL as f32).round() as i32
}

/// Inverse of [`position_thumb_offset`]: the 0..=1 position for a window-local pointer x, with the
/// thumb centered on the cursor and clamped to the track (so a click or drag past either end pins
/// to 0.0 or 1.0).
pub fn position_from_x(x: i32) -> f32 {
    let offset = (x - sprites::POSBAR_X - sprites::POSBAR_THUMB_W / 2).clamp(0, POSBAR_TRAVEL);
    offset as f32 / POSBAR_TRAVEL as f32
}

/// Draw the seek bar from `sheet` at the current `position` (0..=1), thumb held (`pressed`) while
/// scrubbing. `blit` clips to both the sheet and framebuffer, so a sheet smaller than the classic
/// layout simply drops the missing pixels instead of panicking.
pub fn draw(fb: &mut Framebuffer, sheet: &Image, position: f32, pressed: bool) {
    blit(fb, sheet, sprites::POSBAR_BG, sprites::POSBAR_X, sprites::POSBAR_Y);
    let thumb = if pressed {
        sprites::POSBAR_THUMB_PRESSED
    } else {
        sprites::POSBAR_THUMB_NORMAL
    };
    blit(
        fb,
        sheet,
        thumb,
        sprites::POSBAR_X + position_thumb_offset(position),
        sprites::POSBAR_Y,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_offset_reaches_both_ends() {
        assert_eq!(position_thumb_offset(0.0), 0, "start is flush-left");
        assert_eq!(position_thumb_offset(1.0), POSBAR_TRAVEL, "end is flush-right");
        assert_eq!(position_thumb_offset(0.5), (POSBAR_TRAVEL as f32 * 0.5).round() as i32);
        // Out-of-range fractions clamp instead of overshooting the track.
        assert_eq!(position_thumb_offset(-1.0), 0);
        assert_eq!(position_thumb_offset(2.0), POSBAR_TRAVEL);
    }

    #[test]
    fn position_from_x_clamps_past_the_track_and_round_trips() {
        // Far left / far right of the window pin to the extremes.
        assert_eq!(position_from_x(-1000), 0.0);
        assert_eq!(position_from_x(10_000), 1.0);
        // A cursor over the thumb center at the far right reads back as full; the track start as 0.
        let x_end = sprites::POSBAR_X + POSBAR_TRAVEL + sprites::POSBAR_THUMB_W / 2;
        assert_eq!(position_from_x(x_end), 1.0);
        let x_start = sprites::POSBAR_X + sprites::POSBAR_THUMB_W / 2;
        assert_eq!(position_from_x(x_start), 0.0);
        // A mid-track click reads back near 0.5.
        let mid_x = sprites::POSBAR_X + POSBAR_TRAVEL / 2 + sprites::POSBAR_THUMB_W / 2;
        let f = position_from_x(mid_x);
        assert!((f - 0.5).abs() < 0.01, "mid click reads ~0.5 (got {f})");
    }

    /// A posbar sheet where the groove is RED, the normal thumb GREEN, and the pressed thumb BLUE,
    /// so a draw can be read back to prove which sprite landed where.
    fn posbar_sheet() -> Image {
        let w = 307u32;
        let h = 10u32;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let put = |rgba: &mut [u8], x: u32, y: u32, c: [u8; 4]| {
            let o = ((y * w + x) * 4) as usize;
            rgba[o..o + 4].copy_from_slice(&c);
        };
        for y in 0..h {
            for x in 0..248u32 {
                put(&mut rgba, x, y, [255, 0, 0, 255]); // groove RED
            }
            for x in 248..277u32 {
                put(&mut rgba, x, y, [0, 255, 0, 255]); // normal thumb GREEN
            }
            for x in 278..307u32 {
                put(&mut rgba, x, y, [0, 0, 255, 255]); // pressed thumb BLUE
            }
        }
        Image { width: w, height: h, rgba }
    }

    fn px(fb: &Framebuffer, x: i32, y: i32) -> [u8; 4] {
        let o = ((y as u32 * fb.width + x as u32) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    #[test]
    fn draw_paints_the_groove_with_the_thumb_at_the_position() {
        let sheet = posbar_sheet();
        let y = sprites::POSBAR_Y + 5;
        // Start: the normal thumb (GREEN) is flush-left over the groove; the groove (RED) shows to
        // its right (beyond the 29px thumb).
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &sheet, 0.0, false);
        assert_eq!(px(&fb, sprites::POSBAR_X, y), [0, 255, 0, 255], "thumb at the start");
        assert_eq!(px(&fb, sprites::POSBAR_X + 100, y), [255, 0, 0, 255], "groove to its right");

        // End: the thumb slides flush-right; the start now shows the bare groove.
        let mut fb2 = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb2, &sheet, 1.0, false);
        let tx = sprites::POSBAR_X + POSBAR_TRAVEL;
        assert_eq!(px(&fb2, tx + 14, y), [0, 255, 0, 255], "thumb at the end");
        assert_eq!(px(&fb2, sprites::POSBAR_X, y), [255, 0, 0, 255], "groove at the start");
    }

    #[test]
    fn draw_uses_the_pressed_thumb_while_scrubbing() {
        let sheet = posbar_sheet();
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &sheet, 0.0, true);
        assert_eq!(
            px(&fb, sprites::POSBAR_X, sprites::POSBAR_Y + 5),
            [0, 0, 255, 255],
            "pressed (blue) thumb",
        );
    }

    #[test]
    fn an_undersized_sheet_draws_without_panicking() {
        // A 1x1 sheet has only pixel (0,0), so the clipping blit copies that single pixel and
        // drops the rest of the 248-wide groove instead of panicking. Sampling well inside the
        // bar (a source column the sheet does not have) reads back untouched.
        let sheet = Image { width: 1, height: 1, rgba: vec![9, 9, 9, 255] };
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &sheet, 0.5, false);
        assert_eq!(px(&fb, sprites::POSBAR_X + 100, sprites::POSBAR_Y), [0, 0, 0, 0], "clipped, not drawn");
    }
}
