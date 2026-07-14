//! The built-in default skin: an original, clean-room main window drawn in code.
//!
//! xubamp ships no third-party skin art. Every classic `.wsz` (the Winamp base skin, the
//! XMMS default, the SpyAMP set) is someone's copyrighted work, so none can live in this
//! repo. When the user passes no skin, we draw our own 275x116 window instead: a classic
//! layout in a cyan/blue palette, entirely from these routines. It is baked into the
//! `main` sheet (title bar, display, visualiser, sliders and transport row all painted in),
//! with the overlay sheets left `None` so nothing double-draws over it. Real skins loaded
//! at runtime replace it wholesale.

use crate::bmp::Image;
use crate::font;
use crate::sprites::{MAIN_H, MAIN_W};

// Cyan/blue palette. Original values; not sampled from any skin.
const BODY_TOP: [u8; 3] = [30, 52, 66];
const BODY_BOT: [u8; 3] = [14, 26, 34];
const EDGE_LIGHT: [u8; 3] = [72, 120, 142];
const EDGE_DARK: [u8; 3] = [8, 16, 22];
const TITLE_TOP: [u8; 3] = [26, 112, 138];
const TITLE_BOT: [u8; 3] = [12, 54, 70];
const PANEL_BG: [u8; 3] = [10, 30, 40];
const LCD_BG: [u8; 3] = [3, 14, 20];
const CYAN: [u8; 3] = [42, 208, 236];
const CYAN_DIM: [u8; 3] = [26, 132, 152];
const TRACK_BG: [u8; 3] = [7, 18, 26];
const KNOB: [u8; 3] = [96, 156, 176];
const BTN_FACE: [u8; 3] = [34, 62, 78];

/// A tiny top-down RGBA8888 drawing surface used only to author the default skin.
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![0; (w * h * 4) as usize],
        }
    }

    fn put(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x < 0 || y < 0 || x as u32 >= self.w || y as u32 >= self.h {
            return;
        }
        let o = ((y as u32 * self.w + x as u32) * 4) as usize;
        self.px[o] = c[0];
        self.px[o + 1] = c[1];
        self.px[o + 2] = c[2];
        self.px[o + 3] = 255;
    }

    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
        for j in 0..h {
            for i in 0..w {
                self.put(x + i, y + j, c);
            }
        }
    }

    /// Fill a vertical gradient from `top` at the first row to `bot` at the last.
    fn vgrad(&mut self, x: i32, y: i32, w: i32, h: i32, top: [u8; 3], bot: [u8; 3]) {
        for j in 0..h {
            let t = if h <= 1 {
                0.0
            } else {
                j as f32 / (h - 1) as f32
            };
            let c = lerp(top, bot, t);
            for i in 0..w {
                self.put(x + i, y + j, c);
            }
        }
    }

    fn hline(&mut self, x: i32, y: i32, w: i32, c: [u8; 3]) {
        for i in 0..w {
            self.put(x + i, y, c);
        }
    }

    fn vline(&mut self, x: i32, y: i32, h: i32, c: [u8; 3]) {
        for j in 0..h {
            self.put(x, y + j, c);
        }
    }

    /// One-pixel bevel around a rect: `top_left` along the top and left edges, `bot_right`
    /// along the bottom and right. Raised looks lit from the top-left; swap the colours for
    /// a sunken (recessed) look.
    fn bevel(&mut self, x: i32, y: i32, w: i32, h: i32, top_left: [u8; 3], bot_right: [u8; 3]) {
        self.hline(x, y, w, top_left);
        self.vline(x, y, h, top_left);
        self.hline(x, y + h - 1, w, bot_right);
        self.vline(x + w - 1, y, h, bot_right);
    }

    fn raised(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.bevel(x, y, w, h, EDGE_LIGHT, EDGE_DARK);
    }

    fn sunken(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.bevel(x, y, w, h, EDGE_DARK, EDGE_LIGHT);
    }

    fn text(&mut self, x: i32, y: i32, s: &str, c: [u8; 3]) {
        font::draw_text(&mut self.px, self.w, self.h, x, y, s, c);
    }

    /// Right-pointing filled triangle (play glyph) inside a `size`-tall box at (x, y).
    fn tri_right(&mut self, x: i32, y: i32, size: i32, c: [u8; 3]) {
        for row in 0..size {
            let span = size - 2 * (row - size / 2).abs();
            self.hline(x, y + row, span.max(0), c);
        }
    }

    fn tri_left(&mut self, x: i32, y: i32, size: i32, c: [u8; 3]) {
        for row in 0..size {
            let span = (size - 2 * (row - size / 2).abs()).max(0);
            self.hline(x + size - span, y + row, span, c);
        }
    }

    fn tri_up(&mut self, x: i32, y: i32, size: i32, c: [u8; 3]) {
        for row in 0..size {
            let span = (2 * row + 1).min(size);
            self.hline(x + (size - span) / 2, y + row, span, c);
        }
    }
}

fn lerp(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let m = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8;
    [m(0), m(1), m(2)]
}

/// Draw one raised transport button with a centred cyan glyph.
fn button(c: &mut Canvas, x: i32, kind: Glyph) {
    let (y, w, h) = (88, 22, 18);
    c.fill(x, y, w, h, BTN_FACE);
    c.raised(x, y, w, h);
    let cx = x + w / 2;
    let cy = y + h / 2;
    match kind {
        Glyph::Prev => {
            c.fill(x + 6, cy - 4, 2, 8, CYAN);
            c.tri_left(x + 9, cy - 4, 8, CYAN);
        }
        Glyph::Play => c.tri_right(cx - 4, cy - 4, 8, CYAN),
        Glyph::Pause => {
            c.fill(cx - 4, cy - 4, 3, 8, CYAN);
            c.fill(cx + 1, cy - 4, 3, 8, CYAN);
        }
        Glyph::Stop => c.fill(cx - 4, cy - 4, 8, 8, CYAN),
        Glyph::Next => {
            c.tri_right(x + 6, cy - 4, 8, CYAN);
            c.fill(x + 14, cy - 4, 2, 8, CYAN);
        }
        Glyph::Eject => {
            c.tri_up(cx - 4, cy - 5, 8, CYAN);
            c.fill(cx - 4, cy + 4, 8, 2, CYAN);
        }
    }
}

enum Glyph {
    Prev,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
}

/// Build the built-in default [`Skin`]: a fully drawn 275x116 main window, no overlays.
pub fn default_skin() -> crate::Skin {
    let mut c = Canvas::new(MAIN_W as u32, MAIN_H as u32);

    // Window body and outer frame.
    c.vgrad(0, 0, MAIN_W, MAIN_H, BODY_TOP, BODY_BOT);
    c.raised(0, 0, MAIN_W, MAIN_H);

    // Title bar with a recessed centre plate carrying the wordmark.
    c.vgrad(2, 2, MAIN_W - 4, 11, TITLE_TOP, TITLE_BOT);
    for i in 0..4 {
        c.hline(6, 4 + i * 2, MAIN_W - 12, TITLE_BOT);
    }
    let word = "XUBAMP";
    let ww = font::text_width(word) as i32;
    let plate_x = (MAIN_W - (ww + 10)) / 2;
    c.fill(plate_x, 2, ww + 10, 11, PANEL_BG);
    c.sunken(plate_x, 2, ww + 10, 11);
    c.text(plate_x + 5, 4, word, CYAN);
    // Mock window controls at the far right of the title bar.
    for i in 0..3 {
        let bx = MAIN_W - 10 - i * 9;
        c.fill(bx, 3, 7, 7, BTN_FACE);
        c.raised(bx, 3, 7, 7);
    }

    // Big time readout.
    c.fill(24, 26, 62, 13, LCD_BG);
    c.sunken(24, 26, 62, 13);
    c.text(30, 29, "00:00", CYAN);

    // Song-title marquee panel.
    c.fill(92, 26, 158, 13, LCD_BG);
    c.sunken(92, 26, 158, 13);
    c.text(97, 29, "XUBAMP DEFAULT SKIN", CYAN_DIM);

    // Small stereo indicator.
    c.text(255, 29, "ST", CYAN_DIM);

    // Spectrum-analyser box with a few static cyan bars.
    c.fill(24, 43, 76, 16, [0, 0, 0]);
    c.sunken(24, 43, 76, 16);
    let bars = [4, 7, 10, 12, 9, 13, 8, 6, 11, 14, 10, 7, 5, 9, 12, 6, 3];
    for (i, &hgt) in bars.iter().enumerate() {
        let bx = 26 + i as i32 * 4;
        let top = 57 - hgt;
        for yy in top..57 {
            let t = (57 - yy) as f32 / 14.0;
            c.hline(bx, yy, 3, lerp(CYAN_DIM, CYAN, t));
        }
    }

    // Volume and balance slider tracks with knobs.
    c.fill(107, 57, 68, 8, TRACK_BG);
    c.sunken(107, 57, 68, 8);
    c.fill(158, 55, 6, 12, KNOB);
    c.raised(158, 55, 6, 12);
    c.fill(177, 57, 38, 8, TRACK_BG);
    c.sunken(177, 57, 38, 8);
    c.fill(193, 55, 6, 12, KNOB);
    c.raised(193, 55, 6, 12);

    // Position (seek) bar.
    c.fill(16, 72, 248, 10, TRACK_BG);
    c.sunken(16, 72, 248, 10);
    c.fill(22, 71, 8, 12, KNOB);
    c.raised(22, 71, 8, 12);

    // Transport button row.
    button(&mut c, 16, Glyph::Prev);
    button(&mut c, 39, Glyph::Play);
    button(&mut c, 62, Glyph::Pause);
    button(&mut c, 85, Glyph::Stop);
    button(&mut c, 108, Glyph::Next);
    button(&mut c, 136, Glyph::Eject);

    // Equaliser / playlist toggles at the bottom right.
    c.fill(219, 89, 22, 12, BTN_FACE);
    c.raised(219, 89, 22, 12);
    c.text(223, 91, "EQ", CYAN);
    c.fill(242, 89, 22, 12, BTN_FACE);
    c.raised(242, 89, 22, 12);
    c.text(246, 91, "PL", CYAN);

    crate::Skin {
        main: Some(Image {
            width: MAIN_W as u32,
            height: MAIN_H as u32,
            rgba: c.px,
        }),
        // Ship the classic visualization palette so the built-in skin's spectrum/oscilloscope
        // animates from live audio like a real skin (the render layer draws its dynamic feedback,
        // clock, and slider thumbs procedurally when the other overlay sheets are absent).
        viscolor: Some(crate::viscolor::VisColor::default()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_full_main_window_and_no_overlays() {
        let skin = default_skin();
        let main = skin.main.expect("default skin has a main sheet");
        assert_eq!((main.width, main.height), (275, 116));
        assert_eq!(main.rgba.len(), 275 * 116 * 4);
        assert!(skin.titlebar.is_none(), "default bakes the title bar into main");
        assert!(skin.cbuttons.is_none(), "default bakes buttons into main");
        // Every pixel is fully opaque: the whole window is painted, no gaps.
        assert!(
            main.rgba.chunks_exact(4).all(|p| p[3] == 255),
            "no transparent holes in the default window"
        );
    }

    #[test]
    fn top_left_corner_is_the_light_bevel() {
        let main = default_skin().main.unwrap();
        assert_eq!(&main.rgba[0..3], &EDGE_LIGHT, "raised top-left highlight");
    }
}
