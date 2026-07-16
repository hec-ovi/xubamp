//! A compact 5x7 bitmap font, drawn straight into an RGBA8888 buffer.
//!
//! The glyph shapes are original (clean-room), covering only the characters xubamp draws
//! for itself: the built-in default skin's wordmark and LCD-style readouts. Real skins
//! carry their own number and text sheets; this font is only for pixels we author.

/// Glyph cell size, in pixels.
pub const GLYPH_W: u32 = 5;
pub const GLYPH_H: u32 = 7;
/// Horizontal advance between glyph cells (one blank column between glyphs).
pub const ADVANCE: u32 = GLYPH_W + 1;

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

/// Draw `text` at (`x`, `y`) into a top-down RGBA8888 `buf` that is `stride_px` wide and
/// `height_px` tall. One opaque pixel of `color` per set bit; pixels outside the buffer are
/// clipped. Glyphs advance by [`ADVANCE`].
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
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..GLYPH_W as i32 {
                if bits & (0x10 >> col) == 0 {
                    continue;
                }
                let px = cx + col;
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
        cx += ADVANCE as i32;
    }
}

/// Pixel width `text` occupies when drawn: `GLYPH_W` per glyph plus a one-pixel gap
/// between glyphs, with no trailing gap. Empty text is zero.
pub fn text_width(text: &str) -> u32 {
    let n = text.chars().count() as u32;
    if n == 0 {
        0
    } else {
        n * GLYPH_W + (n - 1)
    }
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
    fn text_width_counts_glyphs_and_gaps() {
        assert_eq!(text_width(""), 0);
        assert_eq!(text_width("A"), 5);
        assert_eq!(text_width("AB"), 11); // 5 + 1 + 5
        assert_eq!(text_width("00:00"), 5 * 5 + 4);
    }

    #[test]
    fn draws_a_known_glyph_and_clips_out_of_bounds() {
        let (w, h) = (8, 8);
        let mut b = buf(w, h);
        // 'I' here is 01110 / 00100.../ 01110: bars span columns 1..=3, stem is column 2.
        draw_text(&mut b, w, h, 0, 0, "I", [10, 20, 30]);
        assert!(on(&b, w, 1, 0) && on(&b, w, 3, 0), "top bar of I");
        assert!(!on(&b, w, 0, 0) && !on(&b, w, 4, 0), "top bar stops short of the edges");
        assert!(on(&b, w, 2, 3) && !on(&b, w, 0, 3), "stem of I, sides clear");
        assert!(on(&b, w, 1, 6) && on(&b, w, 3, 6), "bottom bar of I");
        // Pixel (1, 0) is the first set bit of 'I'; its RGBA starts at byte 4.
        assert_eq!(&b[4..8], &[10, 20, 30, 255]);

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
