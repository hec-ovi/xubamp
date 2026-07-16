//! A compact 5x7 bitmap font, drawn straight into an RGBA8888 buffer.
//!
//! The glyph shapes are original (clean-room), covering only the characters xubamp draws
//! for itself: the built-in default skin's wordmark and LCD-style readouts. Real skins
//! carry their own number and text sheets; this font is only for pixels we author.

/// Glyph cell size, in pixels. Glyph ink may be narrower than the cell; drawing is
/// proportional (each glyph advances by its ink width plus a one-pixel gap), so `GLYPH_W`
/// and [`ADVANCE`] are the widest case.
pub const GLYPH_W: u32 = 5;
pub const GLYPH_H: u32 = 7;
/// The widest horizontal advance (a full cell plus its one blank column). An upper bound:
/// narrow glyphs like `-`, `1`, or a space advance less.
pub const ADVANCE: u32 = GLYPH_W + 1;
/// A space's clear width: a narrow word gap, like the proportional system font this
/// stands in for.
const SPACE_W: u32 = 2;

/// Rows are top to bottom; within a row bit 4 (`0x10`) is the leftmost of five columns.
/// Unknown characters render blank.
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '%' => [0x19, 0x19, 0x02, 0x04, 0x08, 0x13, 0x13],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        '-' => [0x00, 0x00, 0x00, 0x0E, 0x00, 0x00, 0x00],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        _ => [0, 0, 0, 0, 0, 0, 0],
    }
}

/// Ink extent of `c` in its cell: (first lit column, ink width). A space is [`SPACE_W`] of
/// clear width; a character with no glyph keeps the full cell as a visible placeholder gap.
fn extent(c: char) -> (i32, u32) {
    if c == ' ' {
        return (0, SPACE_W);
    }
    let rows = glyph(c);
    let mut mask = 0u8;
    for &bits in &rows {
        mask |= bits;
    }
    if mask == 0 {
        return (0, GLYPH_W);
    }
    let mut first = 0i32;
    while mask & (0x10 >> first) == 0 {
        first += 1;
    }
    let mut last = GLYPH_W as i32 - 1;
    while mask & (0x10 >> last) == 0 {
        last -= 1;
    }
    (first, (last - first + 1) as u32)
}

/// Draw `text` at (`x`, `y`) into a top-down RGBA8888 `buf` that is `stride_px` wide and
/// `height_px` tall. One opaque pixel of `color` per set bit; pixels outside the buffer are
/// clipped. Proportional: each glyph advances by its ink width plus a one-pixel gap.
pub fn draw_text(
    buf: &mut [u8],
    stride_px: u32,
    height_px: u32,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 3],
) {
    let mut cx = x;
    for ch in text.chars() {
        let rows = glyph(ch);
        let (first, width) = extent(ch);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..GLYPH_W as i32 {
                if bits & (0x10 >> col) == 0 {
                    continue;
                }
                let px = cx + col - first;
                let py = y + row as i32;
                if px < 0 || py < 0 || px as u32 >= stride_px || py as u32 >= height_px {
                    continue;
                }
                let off = ((py as u32 * stride_px + px as u32) * 4) as usize;
                buf[off] = color[0];
                buf[off + 1] = color[1];
                buf[off + 2] = color[2];
                buf[off + 3] = 255;
            }
        }
        cx += width as i32 + 1;
    }
}

/// Pixel width `text` occupies when drawn: each glyph's ink width plus a one-pixel gap
/// between glyphs, with no trailing gap. Empty text is zero.
pub fn text_width(text: &str) -> u32 {
    let mut total = 0u32;
    for c in text.chars() {
        total += extent(c).1 + 1;
    }
    total.saturating_sub(1)
}

/// How many leading characters of `text` fit when drawn into `max_w` pixels: the longest
/// prefix whose [`text_width`] stays within the budget.
pub fn chars_fitting(text: &str, max_w: u32) -> usize {
    let mut used = 0u32;
    let mut n = 0usize;
    for c in text.chars() {
        let advance = extent(c).1 + 1;
        if (used + advance).saturating_sub(1) > max_w {
            break;
        }
        used += advance;
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(w: u32, h: u32) -> Vec<u8> {
        vec![0; (w * h * 4) as usize]
    }
    fn on(b: &[u8], w: u32, x: u32, y: u32) -> bool {
        b[((y * w + x) * 4 + 3) as usize] != 0
    }

    #[test]
    fn text_width_is_proportional_to_glyph_ink() {
        assert_eq!(text_width(""), 0);
        assert_eq!(text_width("A"), 5);
        assert_eq!(text_width("AB"), 11); // 5 + 1 + 5
        // ':' has one column of ink, so a clock reads narrower than five full cells.
        assert_eq!(text_width("00:00"), 5 + 1 + 5 + 1 + 1 + 1 + 5 + 1 + 5);
        // The dash is 3px of ink and a space is a narrow word gap, so "A - B" reads like
        // the proportional system font, not three empty cells.
        assert_eq!(text_width("A - B"), 5 + 1 + 2 + 1 + 3 + 1 + 2 + 1 + 5);
    }

    #[test]
    fn chars_fitting_measures_the_longest_prefix() {
        assert_eq!(chars_fitting("AB", 11), 2);
        assert_eq!(chars_fitting("AB", 10), 1);
        assert_eq!(chars_fitting("AB", 4), 0);
        assert_eq!(chars_fitting("", 100), 0);
    }

    #[test]
    fn draws_a_known_glyph_and_clips_out_of_bounds() {
        let (w, h) = (8, 8);
        let mut b = buf(w, h);
        // 'I' is 01110 / 00100... / 01110 in its cell; proportional drawing shifts the ink
        // to the pen position, so the bars span columns 0..=2 and the stem is column 1.
        draw_text(&mut b, w, h, 0, 0, "I", [10, 20, 30]);
        assert!(on(&b, w, 0, 0) && on(&b, w, 2, 0), "top bar of I at the pen");
        assert!(!on(&b, w, 3, 0), "top bar is three columns of ink");
        assert!(on(&b, w, 1, 3) && !on(&b, w, 0, 3), "stem of I, sides clear");
        assert!(on(&b, w, 0, 6) && on(&b, w, 2, 6), "bottom bar of I");
        // Pixel (0, 0) is the first set bit of 'I'; its RGBA starts at byte 0.
        assert_eq!(&b[0..4], &[10, 20, 30, 255]);

        // Drawing off the right edge must not panic and must not wrap.
        draw_text(&mut b, w, h, 6, 0, "M", [1, 2, 3]);
        assert!(!on(&b, w, 0, 1), "no wrap into column 0");
    }

    #[test]
    fn unknown_char_is_blank() {
        let (w, h) = (8, 8);
        let mut b = buf(w, h);
        draw_text(&mut b, w, h, 0, 0, "~", [255, 255, 255]);
        assert!(b.iter().all(|&x| x == 0), "unknown glyph draws nothing");
    }
}
