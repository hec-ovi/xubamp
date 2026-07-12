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
pub mod marquee;
pub mod posbar;
pub mod slider;

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
    // Song-title marquee: drawn from the skin's text.bmp font over the display panel. Skins
    // without that sheet (including the built-in default) simply show no marquee here.
    if let Some(text) = &skin.text {
        marquee::draw(&mut fb, text, &state.title, state.marquee_offset);
    }
    // Volume and balance sliders: each drawn from its own sheet at the current value, with the
    // thumb shown pressed while that slider is being dragged. Skins without the sheet skip it.
    if let Some(volume) = &skin.volume {
        let held = state.dragging == Some(hit::Slider::Volume);
        slider::draw_volume(&mut fb, volume, state.volume, held);
    }
    if let Some(balance) = &skin.balance {
        let held = state.dragging == Some(hit::Slider::Balance);
        slider::draw_balance(&mut fb, balance, state.balance, held);
    }
    // Position (seek) bar: the groove and a thumb at the current playback position (0 when nothing
    // is loaded), drawn pressed while the user scrubs. Skins without posbar.bmp show the main
    // background groove instead.
    if let Some(posbar) = &skin.posbar {
        let held = state.dragging == Some(hit::Slider::Position);
        posbar::draw(&mut fb, posbar, state.position.unwrap_or(0.0), held);
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
    fn marquee_draws_over_the_panel_only_with_a_title_and_a_text_sheet() {
        // A text sheet whose glyph cells are all GREEN, over a RED main background.
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            text: Some(solid(155, 18, GREEN)),
            ..Default::default()
        };
        let (mx, my) = (xubamp_skin::sprites::MARQUEE_X as u32, xubamp_skin::sprites::MARQUEE_Y as u32);

        // With a title, the first glyph cell paints the marquee origin green.
        let playing = hit::UiState {
            title: "HELLO".to_string(),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &playing);
        assert_eq!(px(&fb, mx, my), GREEN, "title glyph drawn at the marquee origin");
        // The glyph row is confined to CELL_H pixels: the rows just above and below stay the
        // red background, so a mis-sized cell (drawing above or below the strip) would be caught.
        assert_eq!(px(&fb, mx, my - 1), RED, "nothing drawn above the glyph row");
        assert_eq!(
            px(&fb, mx, my + xubamp_skin::textfont::CELL_H as u32),
            RED,
            "nothing drawn below the glyph row",
        );

        // With no title the strip is untouched: the red background shows through.
        let idle = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(px(&idle, mx, my), RED, "empty title leaves the panel background");

        // A skin without text.bmp never draws a marquee, even with a title set.
        let no_font = Skin {
            main: Some(solid(275, 116, RED)),
            ..Default::default()
        };
        let fb = compose_main_window(&no_font, &playing);
        assert_eq!(px(&fb, mx, my), RED, "no text sheet, no marquee");
    }

    #[test]
    fn sliders_draw_over_the_panel_only_when_their_sheets_are_present() {
        use xubamp_skin::sprites;
        // GREEN volume + balance sheets over a RED main; the background column and thumb are all
        // GREEN, so any slider pixel reads GREEN and the untouched background stays RED.
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            volume: Some(solid(68, 433, GREEN)),
            balance: Some(solid(47, 433, GREEN)),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(px(&fb, sprites::VOLUME_X as u32, sprites::VOLUME_Y as u32), GREEN, "volume drawn");
        assert_eq!(px(&fb, sprites::BALANCE_X as u32, sprites::BALANCE_Y as u32), GREEN, "balance drawn");
        // Between the two sliders the main background shows through.
        assert_eq!(px(&fb, (sprites::VOLUME_X + sprites::VOLUME_W) as u32, sprites::VOLUME_Y as u32), RED);

        // A skin without the slider sheets draws neither.
        let bare = Skin {
            main: Some(solid(275, 116, RED)),
            ..Default::default()
        };
        let fb = compose_main_window(&bare, &hit::UiState::default());
        assert_eq!(px(&fb, sprites::VOLUME_X as u32, sprites::VOLUME_Y as u32), RED, "no volume sheet");
        assert_eq!(px(&fb, sprites::BALANCE_X as u32, sprites::BALANCE_Y as u32), RED, "no balance sheet");
    }

    #[test]
    fn posbar_draws_over_the_panel_only_when_its_sheet_is_present() {
        use xubamp_skin::sprites;
        // A GREEN posbar sheet over a RED main. The whole sheet is GREEN, so any seek-bar pixel
        // (groove or thumb) reads GREEN and the untouched background stays RED.
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            posbar: Some(solid(307, 10, GREEN)),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(px(&fb, sprites::POSBAR_X as u32, sprites::POSBAR_Y as u32), GREEN, "posbar drawn");
        // Just below the 10px-tall bar the main background shows through.
        assert_eq!(
            px(&fb, sprites::POSBAR_X as u32, (sprites::POSBAR_Y + sprites::POSBAR_H) as u32),
            RED,
            "nothing drawn below the bar",
        );

        // A skin without posbar.bmp draws no seek bar.
        let bare = Skin {
            main: Some(solid(275, 116, RED)),
            ..Default::default()
        };
        let fb = compose_main_window(&bare, &hit::UiState::default());
        assert_eq!(px(&fb, sprites::POSBAR_X as u32, sprites::POSBAR_Y as u32), RED, "no posbar sheet");
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
