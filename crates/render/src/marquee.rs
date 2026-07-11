//! The song-title marquee: draw the title from a skin's `text.bmp` font into the fixed strip
//! on the main window, scrolling it when it is too long to fit.
//!
//! Classic behaviour: a title that fits the strip sits still, left-aligned. A wider one scrolls
//! left, looping with a [`SEPARATOR`] between the tail and the head, advanced [`STEP_PX`] pixels
//! per platform tick. All glyphs are clipped to the marquee rectangle so a character straddling
//! either edge is cut cleanly, matching the classic clipped display.

use xubamp_skin::bmp::Image;
use xubamp_skin::sprites;
use xubamp_skin::textfont::{self, ADVANCE};

use crate::hit::UiState;
use crate::Framebuffer;

/// Inserted between the end of a scrolling title and its repeat, so the loop reads clearly.
pub const SEPARATOR: &str = "  ***  ";

/// Pixels the marquee advances per platform tick while scrolling.
pub const STEP_PX: u32 = 2;

/// Pixel width `text` occupies in the marquee font: one [`ADVANCE`] per character, no trailing
/// gap (`text.bmp` glyphs abut). Counts characters, not bytes, so multibyte titles are correct.
pub fn text_px_width(text: &str) -> u32 {
    text.chars().count() as u32 * ADVANCE as u32
}

/// Whether `text` is too wide for the strip and must scroll rather than sit static. A title
/// scrolls exactly when it does not fit [`sprites::MARQUEE_W`], which for the 5px cell means 31
/// characters or more (31 glyphs span 155px against a 154px strip), matching the classic display.
pub fn is_scrolling(text: &str) -> bool {
    text_px_width(text) > sprites::MARQUEE_W as u32
}

/// Width of one full scroll loop: the title plus the separator.
fn loop_px_width(title: &str) -> u32 {
    text_px_width(title) + text_px_width(SEPARATOR)
}

/// Advance the marquee one tick. If the title scrolls, step [`STEP_PX`] and wrap over the loop
/// width, returning `true` (it moved). Otherwise pin the offset to 0 and return `false`, so the
/// caller can slow its timer back down when nothing is animating.
pub fn advance(state: &mut UiState) -> bool {
    if !is_scrolling(&state.title) {
        state.marquee_offset = 0;
        return false;
    }
    let loop_w = loop_px_width(&state.title);
    state.marquee_offset = (state.marquee_offset + STEP_PX) % loop_w;
    true
}

/// Draw the title into the marquee strip of `fb`, sampling glyph cells from the skin's `sheet`
/// (`text.bmp`). A short title is drawn once, left-aligned; a long one is drawn as two loop
/// copies so the wrap is seamless. Everything is clipped to the marquee rectangle.
pub fn draw(fb: &mut Framebuffer, sheet: &Image, title: &str, offset: u32) {
    if title.is_empty() {
        return;
    }
    let clip0 = sprites::MARQUEE_X;
    let clip1 = sprites::MARQUEE_X + sprites::MARQUEE_W;
    let y = sprites::MARQUEE_Y;

    if !is_scrolling(title) {
        draw_str(fb, sheet, title, clip0, y, clip0, clip1);
        return;
    }

    let loop_w = loop_px_width(title) as i32;
    let title_w = text_px_width(title) as i32;
    let start = clip0 - offset as i32;
    // Two copies span the whole strip for any offset in `[0, loop_w)`: the first can begin as far
    // left as `clip0 - loop_w`, the second picks up exactly one loop to its right.
    for copy in 0..2 {
        let base = start + copy * loop_w;
        draw_str(fb, sheet, title, base, y, clip0, clip1);
        draw_str(fb, sheet, SEPARATOR, base + title_w, y, clip0, clip1);
    }
}

/// Draw `s` starting at (`x`, `y`), advancing one [`ADVANCE`] per character, clipping every
/// pixel to destination columns `[clip0, clip1)`. Characters with no cell advance blank.
fn draw_str(fb: &mut Framebuffer, sheet: &Image, s: &str, mut x: i32, y: i32, clip0: i32, clip1: i32) {
    for ch in s.chars() {
        if let Some(rect) = textfont::cell(ch) {
            blit_clipped(fb, sheet, rect, x, y, clip0, clip1);
        }
        x += ADVANCE;
    }
}

/// Blit `rect` from `src` to (`dst_x`, `dst_y`), opaque, keeping only destination columns in
/// `[clip0, clip1)` and clipping to both bitmaps. Off-region or off-image pixels are skipped.
fn blit_clipped(
    fb: &mut Framebuffer,
    src: &Image,
    rect: xubamp_skin::sprites::Rect,
    dst_x: i32,
    dst_y: i32,
    clip0: i32,
    clip1: i32,
) {
    for row in 0..rect.h {
        let sy = rect.y + row;
        let dy = dst_y + row;
        if sy < 0 || dy < 0 || sy as u32 >= src.height || dy as u32 >= fb.height {
            continue;
        }
        for col in 0..rect.w {
            let dx = dst_x + col;
            if dx < clip0 || dx >= clip1 {
                continue;
            }
            let sx = rect.x + col;
            if sx < 0 || dx < 0 || sx as u32 >= src.width || dx as u32 >= fb.width {
                continue;
            }
            let s_off = ((sy as u32 * src.width + sx as u32) * 4) as usize;
            let d_off = ((dy as u32 * fb.width + dx as u32) * 4) as usize;
            fb.rgba[d_off..d_off + 4].copy_from_slice(&src.rgba[s_off..s_off + 4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `text.bmp`-shaped sheet where every cell is a solid, cell-distinct colour, so a drawn
    /// pixel identifies which glyph landed. Cell `(row, col)` is filled with `(col+1, row+1, 128)`.
    fn text_sheet() -> Image {
        let (w, h) = (155u32, 18u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for row in 0..3u32 {
            for col in 0..31u32 {
                let color = [(col + 1) as u8, (row + 1) as u8, 128, 255];
                for yy in row * 6..row * 6 + 6 {
                    for xx in col * 5..col * 5 + 5 {
                        let o = ((yy * w + xx) * 4) as usize;
                        rgba[o..o + 4].copy_from_slice(&color);
                    }
                }
            }
        }
        Image { width: w, height: h, rgba }
    }

    fn fb() -> Framebuffer {
        Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32)
    }

    fn px(fb: &Framebuffer, x: i32, y: i32) -> [u8; 4] {
        let o = ((y as u32 * fb.width + x as u32) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    #[test]
    fn scrolls_exactly_when_the_title_overruns_the_strip() {
        // 30 glyphs span 150px and fit the 154px strip; 31 span 155px and must scroll. This is
        // the geometry boundary (and Webamp's length >= 31 rule).
        assert!(!is_scrolling(&"a".repeat(30)), "30 chars (150px) fits");
        assert!(is_scrolling(&"a".repeat(31)), "31 chars (155px) overruns 154px and scrolls");
        assert!(is_scrolling(&"a".repeat(32)), "32 scrolls");
        assert!(!is_scrolling(""), "empty never scrolls");
    }

    #[test]
    fn width_and_scroll_count_characters_not_bytes() {
        assert_eq!(text_px_width(""), 0);
        assert_eq!(text_px_width("A"), 5);
        assert_eq!(text_px_width("ABC"), 15);
        assert_eq!(text_px_width(SEPARATOR), 35); // "  ***  " is 7 cells
        // Multibyte: three 2-byte Nordic letters are 3 glyphs (15px), not 6 bytes' worth.
        assert_eq!(text_px_width("\u{00C4}\u{00D6}\u{00C5}"), 15);
        // 30 multibyte chars (60 bytes) still fit: scrolling keys off char count, not byte len.
        assert!(!is_scrolling(&"\u{00C4}".repeat(30)), "byte length must not force a scroll");
        assert!(is_scrolling(&"\u{00C4}".repeat(31)), "31 multibyte chars overrun the strip");
    }

    #[test]
    fn advance_wraps_a_long_title_and_pins_a_short_one() {
        let mut s = UiState {
            title: "a".repeat(40), // scrolls
            ..Default::default()
        };
        let loop_w = loop_px_width(&s.title);
        assert!(advance(&mut s));
        assert_eq!(s.marquee_offset, STEP_PX);
        // Drive it right up to the wrap point and confirm it rolls back into range.
        s.marquee_offset = loop_w - 1;
        assert!(advance(&mut s));
        assert_eq!(s.marquee_offset, (loop_w - 1 + STEP_PX) % loop_w);
        assert!(s.marquee_offset < loop_w);

        let mut short = UiState {
            title: "short".to_string(),
            marquee_offset: 40, // stale value from a previous long title
            ..Default::default()
        };
        assert!(!advance(&mut short), "a fitting title does not scroll");
        assert_eq!(short.marquee_offset, 0, "offset is pinned back to zero");
    }

    #[test]
    fn draws_a_static_title_left_aligned_at_the_region_origin() {
        let sheet = text_sheet();
        let mut f = fb();
        draw(&mut f, &sheet, "A", 0);
        // 'A' is cell (0,0) -> colour (1,1,128). It lands at the region's top-left glyph.
        assert_eq!(px(&f, sprites::MARQUEE_X, sprites::MARQUEE_Y), [1, 1, 128, 255]);
        assert_eq!(px(&f, sprites::MARQUEE_X + 4, sprites::MARQUEE_Y + 5), [1, 1, 128, 255]);
        // Just left of the region is untouched (still the cleared framebuffer).
        assert_eq!(px(&f, sprites::MARQUEE_X - 1, sprites::MARQUEE_Y), [0, 0, 0, 0]);
    }

    #[test]
    fn clips_the_right_edge_of_a_scrolling_title() {
        let sheet = text_sheet();
        let mut f = fb();
        // A long title at offset 0 begins at the region origin and runs off the right: the last
        // in-region column is drawn, the first column past the strip is never touched.
        draw(&mut f, &sheet, &"a".repeat(40), 0);
        let last_visible = sprites::MARQUEE_X + sprites::MARQUEE_W - 1;
        assert_ne!(px(&f, last_visible, sprites::MARQUEE_Y), [0, 0, 0, 0], "in-region pixel drawn");
        assert_eq!(
            px(&f, sprites::MARQUEE_X + sprites::MARQUEE_W, sprites::MARQUEE_Y),
            [0, 0, 0, 0],
            "the column past the region is never touched",
        );
    }

    #[test]
    fn clips_the_left_edge_of_a_scrolling_title() {
        let sheet = text_sheet();
        let mut f = fb();
        // At offset 2 the first glyph starts at x = MARQUEE_X - 2, straddling the left edge. Its
        // two columns left of the region must be clipped; the right portion paints from the edge.
        draw(&mut f, &sheet, &"a".repeat(40), 2);
        assert_eq!(px(&f, sprites::MARQUEE_X - 1, sprites::MARQUEE_Y), [0, 0, 0, 0], "left of strip clear");
        assert_eq!(
            px(&f, sprites::MARQUEE_X, sprites::MARQUEE_Y),
            [1, 1, 128, 255],
            "right portion of the straddling 'a' cell paints from the edge",
        );
    }

    #[test]
    fn a_scrolling_title_covers_every_column_at_every_offset() {
        let sheet = text_sheet();
        // A 32-char title gives loop_w = (32 + 7) * 5 = 195, short enough that the separator and
        // the wrap seam both pass through the 154px strip. Sweeping all offsets proves the two
        // loop copies plus separator leave no uncovered column at any scroll position; a broken
        // second copy or a misplaced seam would leave a gap at some offset.
        let title = "a".repeat(32);
        let loop_w = loop_px_width(&title);
        for offset in 0..loop_w {
            let mut f = fb();
            draw(&mut f, &sheet, &title, offset);
            for x in sprites::MARQUEE_X..sprites::MARQUEE_X + sprites::MARQUEE_W {
                assert_ne!(
                    px(&f, x, sprites::MARQUEE_Y + 2),
                    [0, 0, 0, 0],
                    "offset {offset}: column {x} left uncovered",
                );
            }
        }
    }

    #[test]
    fn an_empty_title_draws_nothing() {
        let sheet = text_sheet();
        let mut f = fb();
        draw(&mut f, &sheet, "", 0);
        assert!(f.rgba.iter().all(|&b| b == 0), "empty title leaves the strip clear");
    }

    #[test]
    fn a_text_sheet_too_small_for_a_cell_skips_it_without_panicking() {
        // A sheet with only the first row and a few columns: '?' (row 2) and '#' (row 0 col 30)
        // reference cells past its bounds. The source-bounds guards must skip them, not panic.
        let tiny = Image {
            width: 20,
            height: 6,
            rgba: vec![255u8; 20 * 6 * 4],
        };
        let mut f = fb();
        draw(&mut f, &tiny, "?#", 0); // both cells fall outside the 20x6 sheet
        assert!(
            f.rgba.iter().all(|&b| b == 0),
            "out-of-range glyphs draw nothing rather than panicking",
        );
    }
}
