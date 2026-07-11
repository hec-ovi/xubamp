//! Software sprite compositor.
//!
//! The whole classic UI is bitmap sprites blitted into one CPU framebuffer, which the
//! `wl` crate then hands to the compositor as a `wl_shm` buffer. This crate is pure: a
//! `Framebuffer`, a clipping `blit`, and window-composition functions. No platform code,
//! no allocation per blit beyond the single framebuffer.

use xubamp_skin::bmp::Image;
use xubamp_skin::sprites::{self, Placement, Rect};
use xubamp_skin::Skin;

pub mod hit;

/// A top-down `RGBA8888` framebuffer, 4 bytes per pixel.
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, top-down.
    pub rgba: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; width as usize * height as usize * 4],
        }
    }

    /// The raw pixel bytes, for upload into a `wl_shm` buffer.
    pub fn as_bytes(&self) -> &[u8] {
        &self.rgba
    }
}

/// Copy `rect` from `src` into `fb` at (`dst_x`, `dst_y`), opaque, clipped to both the
/// source image and the destination framebuffer. Regions outside either are skipped, so
/// off-screen or oversized placements never panic.
pub fn blit(fb: &mut Framebuffer, src: &Image, rect: Rect, dst_x: i32, dst_y: i32) {
    for row in 0..rect.h {
        let sy = rect.y + row;
        let dy = dst_y + row;
        if sy < 0 || dy < 0 || sy as u32 >= src.height || dy as u32 >= fb.height {
            continue;
        }
        for col in 0..rect.w {
            let sx = rect.x + col;
            let dx = dst_x + col;
            if sx < 0 || dx < 0 || sx as u32 >= src.width || dx as u32 >= fb.width {
                continue;
            }
            let s_off = ((sy as u32 * src.width + sx as u32) * 4) as usize;
            let d_off = ((dy as u32 * fb.width + dx as u32) * 4) as usize;
            fb.rgba[d_off..d_off + 4].copy_from_slice(&src.rgba[s_off..s_off + 4]);
        }
    }
}

fn blit_placement(fb: &mut Framebuffer, sheet: &Image, p: Placement) {
    blit(fb, sheet, p.src, p.dst_x, p.dst_y);
}

/// Split whole seconds into the four MM:SS digit values (tens then units of minutes, then of
/// seconds) for the time display. Minutes saturate at 99 so the two-digit field never
/// overflows; the classic display has no room to show more.
pub fn mmss_digits(secs: u32) -> [u8; 4] {
    let mins = (secs / 60).min(99);
    let s = secs % 60;
    [
        (mins / 10) as u8,
        (mins % 10) as u8,
        (s / 10) as u8,
        (s % 10) as u8,
    ]
}

/// Compose the main window (275x116): the MAIN background, the active title bar, then the
/// six transport buttons, drawing the pressed sprite for whichever button `state` reports as
/// held. Missing sheets are simply skipped (their pixels stay whatever the lower layer left),
/// which is the default-skin fallback point.
pub fn compose_main_window(skin: &Skin, state: &hit::UiState) -> Framebuffer {
    let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
    if let Some(main) = &skin.main {
        blit_placement(&mut fb, main, sprites::MAIN_BG);
    }
    if let Some(titlebar) = &skin.titlebar {
        blit_placement(&mut fb, titlebar, sprites::TITLEBAR_ACTIVE);
    }
    if let Some(cbuttons) = &skin.cbuttons {
        for ((normal, pressed), id) in sprites::CBUTTONS
            .iter()
            .zip(sprites::CBUTTONS_PRESSED.iter())
            .zip(hit::TRANSPORT_ORDER)
        {
            let placement = if state.pressed == Some(id) {
                *pressed
            } else {
                *normal
            };
            blit_placement(&mut fb, cbuttons, placement);
        }
    }
    // Time display: four digits from the number sheet, but only while a time is set. With no
    // elapsed time (nothing loaded / stopped) the slots stay blank, as on the classic display.
    if let (Some(numbers), Some(secs)) = (&skin.numbers, state.elapsed) {
        for (&(dx, dy), &d) in sprites::TIME_DIGITS.iter().zip(mmss_digits(secs).iter()) {
            blit(&mut fb, numbers, sprites::DIGITS[d as usize], dx, dy);
        }
    }
    fb
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn px(fb: &Framebuffer, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * fb.width + x) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];

    #[test]
    fn compose_fills_from_main_background() {
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!((fb.width, fb.height), (275, 116));
        assert_eq!(px(&fb, 0, 0), RED);
        assert_eq!(px(&fb, 274, 115), RED);
    }

    #[test]
    fn transport_buttons_land_on_their_rects() {
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            cbuttons: Some(solid(136, 36, GREEN)),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &hit::UiState::default());
        // Play button occupies dst x 39..62, y 88..106.
        assert_eq!(px(&fb, 39, 88), GREEN, "play top-left");
        assert_eq!(px(&fb, 61, 105), GREEN, "play bottom-right");
        // Away from any button the main background shows through.
        assert_eq!(px(&fb, 200, 40), RED);
        assert_eq!(px(&fb, 0, 0), RED);
    }

    #[test]
    fn pressed_button_draws_from_the_bottom_row() {
        // A cbuttons sheet split top/bottom: normal row (y 0..18) BLUE, pressed row WHITE.
        let mut sheet = solid(136, 36, [0, 0, 255, 255]); // BLUE top
        for y in 18..36 {
            for x in 0..136 {
                let o = ((y * 136 + x) * 4) as usize;
                sheet.rgba[o..o + 4].copy_from_slice(&[255, 255, 255, 255]); // WHITE bottom
            }
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            cbuttons: Some(sheet),
            ..Default::default()
        };
        let state = hit::UiState {
            pressed: Some(hit::Transport::Play),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &state);
        // Play (dst 39,88) is pressed -> sampled from the WHITE bottom row.
        assert_eq!(px(&fb, 39 + 11, 88 + 9), [255, 255, 255, 255], "play pressed");
        // Stop (dst 85,88) is not pressed -> still the BLUE normal row.
        assert_eq!(px(&fb, 85 + 11, 88 + 9), [0, 0, 255, 255], "stop normal");
    }

    #[test]
    fn mmss_digits_split_and_clamp() {
        assert_eq!(mmss_digits(0), [0, 0, 0, 0]);
        assert_eq!(mmss_digits(65), [0, 1, 0, 5]); // 01:05
        assert_eq!(mmss_digits(3599), [5, 9, 5, 9]); // 59:59
        assert_eq!(mmss_digits(6000), [9, 9, 0, 0]); // 100:00 clamps to 99:00
        assert_eq!(mmss_digits(600_000), [9, 9, 0, 0]); // far past the cap, still 99:xx
    }

    #[test]
    fn time_display_draws_the_elapsed_digits() {
        // A number sheet where digit d's 9px cell is a d-distinct red, so we can read back
        // which digit landed where. (Digit 0 is (10,0,0), digit 5 is (110,0,0), etc.)
        let mut numbers = solid(99, 13, [0, 0, 0, 255]);
        for d in 0..10u32 {
            let color = [(10 + d * 20) as u8, 0, 0, 255];
            for y in 0..13u32 {
                for x in d * 9..d * 9 + 9 {
                    let o = ((y * 99 + x) * 4) as usize;
                    numbers.rgba[o..o + 4].copy_from_slice(&color);
                }
            }
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            numbers: Some(numbers),
            ..Default::default()
        };
        let state = hit::UiState {
            elapsed: Some(65), // 01:05 -> digits [0, 1, 0, 5]
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &state);
        for (&(dx, dy), &d) in xubamp_skin::sprites::TIME_DIGITS.iter().zip([0u32, 1, 0, 5].iter())
        {
            let want = [(10 + d * 20) as u8, 0, 0, 255];
            let (cx, cy) = (dx as u32 + 4, dy as u32 + 6); // sample a pixel inside the cell
            assert_eq!(px(&fb, cx, cy), want, "digit {d} at ({dx},{dy})");
        }

        // With no elapsed time the slots stay blank: the main background shows through.
        let blank = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(px(&blank, 48 + 4, 26 + 6), RED, "blank display draws no digit");
    }

    #[test]
    fn blit_clips_at_the_edge_without_panicking() {
        let mut fb = Framebuffer::new(10, 10);
        let src = solid(5, 5, GREEN);
        // Drawn at (8,8): only the 2x2 top-left corner of src fits.
        blit(&mut fb, &src, Rect::new(0, 0, 5, 5), 8, 8);
        assert_eq!(px(&fb, 9, 9), GREEN); // inside
        assert_eq!(px(&fb, 7, 7), [0, 0, 0, 0]); // untouched
    }
}
