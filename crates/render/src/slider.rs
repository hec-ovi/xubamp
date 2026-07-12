//! The volume and balance sliders.
//!
//! Each is a level-indicator background (one of 28 frames chosen by the value) with a draggable
//! thumb on top, both from the same sheet (`volume.bmp` / `balance.bmp`). This module holds the
//! pure value math (which frame, where the thumb sits, and the inverse: what value a click maps
//! to) and the draw, so the hit-testing in [`crate::hit`] and the compositor agree on one
//! geometry. The frame and travel formulas match Webamp's classic main window.

use xubamp_skin::sprites::{self, Rect};
use xubamp_skin::bmp::Image;

use crate::{blit, Framebuffer};

/// Pixels the thumb travels across each background: the background width minus the thumb width,
/// so the thumb is flush-left at the minimum value and flush-right at the maximum.
pub const VOLUME_TRAVEL: i32 = sprites::VOLUME_W - sprites::SLIDER_THUMB_W;
pub const BALANCE_TRAVEL: i32 = sprites::BALANCE_W - sprites::SLIDER_THUMB_W;

/// Volume (0..=100) to its background frame (0..=27) in `volume.bmp`. Webamp draws
/// `round(volume/100 * 28) - 1`: frame 27 is the full bar at 100, and low volumes collapse to
/// frame 0. Clamped so the degenerate `-1` at volume 0 stays in range.
pub fn volume_frame(volume: u8) -> i32 {
    let sprite = ((volume.min(100) as f32 / 100.0) * sprites::SLIDER_FRAMES as f32).round() as i32;
    (sprite - 1).clamp(0, sprites::SLIDER_FRAMES - 1)
}

/// Balance (-100..=100) to its background frame (0..=27) in `balance.bmp`. Symmetric about
/// center: `floor(|balance|/100 * 27)`, so frame 0 is dead-center and frame 27 is either
/// extreme. Direction (left vs right) is shown by the thumb, not the background frame.
pub fn balance_frame(balance: i8) -> i32 {
    let percent = balance.unsigned_abs().min(100) as f32 / 100.0;
    ((percent * (sprites::SLIDER_FRAMES - 1) as f32).floor() as i32).clamp(0, sprites::SLIDER_FRAMES - 1)
}

/// Thumb x offset (0..=[`VOLUME_TRAVEL`]) from the volume value.
pub fn volume_thumb_offset(volume: u8) -> i32 {
    ((volume.min(100) as f32 / 100.0) * VOLUME_TRAVEL as f32).round() as i32
}

/// Thumb x offset (0..=[`BALANCE_TRAVEL`]) from the balance value: -100 is flush-left, 0 is
/// centered, +100 is flush-right.
pub fn balance_thumb_offset(balance: i8) -> i32 {
    let normalized = (balance.clamp(-100, 100) as i32 + 100) as f32 / 200.0;
    (normalized * BALANCE_TRAVEL as f32).round() as i32
}

/// Inverse of [`volume_thumb_offset`]: the volume for a window-local pointer x, with the thumb
/// centered on the cursor and clamped to the track (so a click or drag past either end pins to
/// 0 or 100).
pub fn volume_from_x(x: i32) -> u8 {
    let offset = (x - sprites::VOLUME_X - sprites::SLIDER_THUMB_W / 2).clamp(0, VOLUME_TRAVEL);
    ((offset as f32 / VOLUME_TRAVEL as f32) * 100.0).round() as u8
}

/// Inverse of [`balance_thumb_offset`]: the balance for a window-local pointer x, clamped to the
/// track. Center (0) is at the track midpoint.
pub fn balance_from_x(x: i32) -> i8 {
    let offset = (x - sprites::BALANCE_X - sprites::SLIDER_THUMB_W / 2).clamp(0, BALANCE_TRAVEL);
    (((offset as f32 / BALANCE_TRAVEL as f32) * 200.0).round() as i32 - 100) as i8
}

/// Draw a slider: the chosen background frame, then the thumb (pressed sprite while held) at its
/// offset. `blit` clips to both sheet and framebuffer, so a sheet smaller than the classic
/// layout simply drops the missing pixels instead of panicking.
fn draw(fb: &mut Framebuffer, sheet: &Image, bg_src_x: i32, frame: i32, x: i32, thumb_offset: i32, pressed: bool) {
    let bg = Rect::new(
        bg_src_x,
        frame * sprites::SLIDER_FRAME_STRIDE,
        // Width from the destination geometry; balance's is narrower than volume's.
        if bg_src_x == sprites::BALANCE_BG_SRC_X { sprites::BALANCE_W } else { sprites::VOLUME_W },
        sprites::SLIDER_BG_H,
    );
    blit(fb, sheet, bg, x, sprites::VOLUME_Y);
    let thumb = if pressed {
        sprites::SLIDER_THUMB_PRESSED
    } else {
        sprites::SLIDER_THUMB_NORMAL
    };
    blit(fb, sheet, thumb, x + thumb_offset, sprites::VOLUME_Y + sprites::SLIDER_THUMB_DY);
}

/// Draw the volume slider from `sheet` at the current `volume`, thumb held (`pressed`) or not.
pub fn draw_volume(fb: &mut Framebuffer, sheet: &Image, volume: u8, pressed: bool) {
    draw(
        fb,
        sheet,
        sprites::VOLUME_BG_SRC_X,
        volume_frame(volume),
        sprites::VOLUME_X,
        volume_thumb_offset(volume),
        pressed,
    );
}

/// Draw the balance slider from `sheet` at the current `balance`, thumb held (`pressed`) or not.
pub fn draw_balance(fb: &mut Framebuffer, sheet: &Image, balance: i8, pressed: bool) {
    draw(
        fb,
        sheet,
        sprites::BALANCE_BG_SRC_X,
        balance_frame(balance),
        sprites::BALANCE_X,
        balance_thumb_offset(balance),
        pressed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_frame_spans_the_column_ends() {
        assert_eq!(volume_frame(0), 0, "silent collapses to the first frame");
        assert_eq!(volume_frame(100), 27, "full volume is the last frame");
        // Monotonic non-decreasing across the range.
        let mut prev = -1;
        for v in 0..=100u8 {
            let f = volume_frame(v);
            assert!((0..=27).contains(&f), "frame {f} for volume {v} in range");
            assert!(f >= prev, "frame does not decrease as volume rises");
            prev = f;
        }
    }

    #[test]
    fn balance_frame_is_symmetric_about_center() {
        assert_eq!(balance_frame(0), 0, "center is the first frame");
        assert_eq!(balance_frame(100), 27, "full right is the last frame");
        assert_eq!(balance_frame(-100), 27, "full left is the last frame");
        // Mirror image: equal deviation either side selects the same frame.
        for mag in 0..=100i8 {
            assert_eq!(balance_frame(mag), balance_frame(-mag), "|{mag}| symmetric");
        }
    }

    #[test]
    fn thumb_offsets_reach_both_ends() {
        assert_eq!(volume_thumb_offset(0), 0);
        assert_eq!(volume_thumb_offset(100), VOLUME_TRAVEL);
        assert_eq!(balance_thumb_offset(-100), 0, "full left is flush-left");
        assert_eq!(balance_thumb_offset(100), BALANCE_TRAVEL, "full right is flush-right");
        assert_eq!(balance_thumb_offset(0), BALANCE_TRAVEL / 2, "center is the midpoint");
    }

    #[test]
    fn value_from_x_clamps_past_the_track_and_round_trips() {
        // Far left / far right of the window pin to the extremes.
        assert_eq!(volume_from_x(-1000), 0);
        assert_eq!(volume_from_x(1000), 100);
        assert_eq!(balance_from_x(-1000), -100);
        assert_eq!(balance_from_x(1000), 100);
        // A cursor over the thumb center at max volume reads back as ~full.
        let x_full = sprites::VOLUME_X + VOLUME_TRAVEL + sprites::SLIDER_THUMB_W / 2;
        assert_eq!(volume_from_x(x_full), 100);
        // Center of the balance track reads back as center (0).
        let x_center = sprites::BALANCE_X + BALANCE_TRAVEL / 2 + sprites::SLIDER_THUMB_W / 2;
        assert_eq!(balance_from_x(x_center), 0);
    }

    /// A slider sheet where background frame `f` is filled with a distinct color and the two
    /// thumb cells are their own colors, so a draw can be read back to prove which frame and
    /// thumb landed where.
    fn slider_sheet() -> Image {
        let w = 68u32;
        let h = 433u32;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let put = |rgba: &mut [u8], x: u32, y: u32, c: [u8; 4]| {
            let o = ((y * w + x) * 4) as usize;
            rgba[o..o + 4].copy_from_slice(&c);
        };
        // Background frames: frame f is color (f+1, 0, 0).
        for f in 0..28u32 {
            for y in f * 15..f * 15 + 15 {
                for x in 0..w {
                    put(&mut rgba, x, y, [(f + 1) as u8, 0, 0, 255]);
                }
            }
        }
        // Thumb cells at y 422..433: normal (x 15..29) GREEN, pressed (x 0..14) BLUE.
        for y in 422..433u32 {
            for x in 15..29u32 {
                put(&mut rgba, x, y, [0, 255, 0, 255]);
            }
            for x in 0..14u32 {
                put(&mut rgba, x, y, [0, 0, 255, 255]);
            }
        }
        Image { width: w, height: h, rgba }
    }

    fn px(fb: &Framebuffer, x: i32, y: i32) -> [u8; 4] {
        let o = ((y as u32 * fb.width + x as u32) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    #[test]
    fn draw_volume_paints_the_value_frame_and_thumb() {
        let sheet = slider_sheet();
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw_volume(&mut fb, &sheet, 100, false);
        // Background at the volume origin is frame 27 -> color (28,0,0).
        assert_eq!(px(&fb, sprites::VOLUME_X, sprites::VOLUME_Y), [28, 0, 0, 255]);
        // The normal (GREEN) thumb sits at the far right (offset VOLUME_TRAVEL).
        let tx = sprites::VOLUME_X + VOLUME_TRAVEL;
        let ty = sprites::VOLUME_Y + sprites::SLIDER_THUMB_DY;
        assert_eq!(px(&fb, tx + 1, ty + 1), [0, 255, 0, 255], "normal thumb drawn");
    }

    #[test]
    fn draw_volume_uses_the_pressed_thumb_when_held() {
        let sheet = slider_sheet();
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw_volume(&mut fb, &sheet, 0, true);
        // Frame 0 -> color (1,0,0) at the origin.
        assert_eq!(px(&fb, sprites::VOLUME_X, sprites::VOLUME_Y), [1, 0, 0, 255]);
        // Held: the pressed (BLUE) thumb, flush-left at offset 0.
        let ty = sprites::VOLUME_Y + sprites::SLIDER_THUMB_DY;
        assert_eq!(px(&fb, sprites::VOLUME_X + 1, ty + 1), [0, 0, 255, 255], "pressed thumb drawn");
    }

    #[test]
    fn draw_balance_reads_its_own_column_and_center_frame() {
        let sheet = slider_sheet();
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw_balance(&mut fb, &sheet, 0, false);
        // Center -> frame 0 -> color (1,0,0) at the balance origin.
        assert_eq!(px(&fb, sprites::BALANCE_X, sprites::BALANCE_Y), [1, 0, 0, 255]);
        // The thumb is centered on the balance track.
        let tx = sprites::BALANCE_X + BALANCE_TRAVEL / 2;
        let ty = sprites::BALANCE_Y + sprites::SLIDER_THUMB_DY;
        assert_eq!(px(&fb, tx + 1, ty + 1), [0, 255, 0, 255], "centered thumb drawn");
    }

    #[test]
    fn an_undersized_sheet_draws_without_panicking() {
        // A 1x1 sheet has none of the frames or thumb; the clipping blit drops every pixel.
        let sheet = Image { width: 1, height: 1, rgba: vec![9, 9, 9, 255] };
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw_volume(&mut fb, &sheet, 50, false);
        draw_balance(&mut fb, &sheet, -30, true);
        // Nothing was drawn into the slider row.
        assert_eq!(px(&fb, sprites::VOLUME_X, sprites::VOLUME_Y), [0, 0, 0, 0]);
    }
}
