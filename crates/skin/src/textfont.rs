//! The classic `TEXT.BMP` bitmap font: which 5x6 cell each character draws from.
//!
//! `TEXT.BMP` is a sprite sheet of 5x6-pixel glyph cells laid out on a fixed grid, three
//! rows tall. A character's cell is `(col * CELL_W, row * CELL_H)`. Glyphs abut with no
//! spacing (the classic `hspacing`/`vspacing` are both 0), so the horizontal advance equals
//! the cell width. Letters are case-folded (both cases share one cell), and characters the
//! sheet has no cell for return `None`, which the renderer draws as a gap. The grid is the
//! documented classic layout (transcribed, not copied from any implementation).

use crate::sprites::Rect;

/// Glyph cell size, in pixels.
pub const CELL_W: i32 = 5;
pub const CELL_H: i32 = 6;
/// Horizontal advance between cells. `TEXT.BMP` glyphs abut, so this equals [`CELL_W`].
pub const ADVANCE: i32 = CELL_W;

/// Source rectangle in `TEXT.BMP` for `c`, or `None` if the sheet has no cell for it (drawn
/// as a blank advance). Case is folded to the single shared letter cell.
pub fn cell(c: char) -> Option<Rect> {
    let (row, col) = pos(c)?;
    Some(Rect::new(col * CELL_W, row * CELL_H, CELL_W, CELL_H))
}

/// The `(row, column)` of `c` on the `TEXT.BMP` grid.
fn pos(c: char) -> Option<(i32, i32)> {
    // Row 0, columns 0-25: the letters (one cell per letter, shared by both cases).
    if c.is_ascii_alphabetic() {
        return Some((0, c.to_ascii_lowercase() as i32 - 'a' as i32));
    }
    // Row 1, columns 0-9: the digits.
    if c.is_ascii_digit() {
        return Some((1, c as i32 - '0' as i32));
    }
    Some(match c {
        // Rest of row 0.
        '"' => (0, 26),
        '@' => (0, 27),
        ' ' => (0, 30),
        // Rest of row 1: punctuation and symbols.
        '\u{2026}' => (1, 10), // horizontal ellipsis
        '.' => (1, 11),
        ':' => (1, 12),
        '(' => (1, 13),
        ')' => (1, 14),
        '-' => (1, 15),
        '\'' => (1, 16),
        '!' => (1, 17),
        '_' => (1, 18),
        '+' => (1, 19),
        '\\' => (1, 20),
        '/' => (1, 21),
        // `<`/`>` have no cells of their own; the classic font reuses the brackets.
        '[' | '<' => (1, 22),
        ']' | '>' => (1, 23),
        '^' => (1, 24),
        '&' => (1, 25),
        '%' => (1, 26),
        ',' => (1, 27),
        '=' => (1, 28),
        '$' => (1, 29),
        '#' => (1, 30),
        // Row 2: the Nordic letters, then `?` and `*`.
        '\u{00C5}' | '\u{00E5}' => (2, 0), // A-ring
        '\u{00D6}' | '\u{00F6}' => (2, 1), // O-umlaut
        '\u{00C4}' | '\u{00E4}' => (2, 2), // A-umlaut
        '?' => (2, 3),
        '*' => (2, 4),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_share_one_case_folded_cell_in_row_zero() {
        assert_eq!(cell('a'), Some(Rect::new(0, 0, 5, 6)));
        assert_eq!(cell('A'), Some(Rect::new(0, 0, 5, 6)), "upper and lower share a cell");
        assert_eq!(cell('z'), Some(Rect::new(25 * 5, 0, 5, 6)));
        assert_eq!(cell('Z'), cell('z'));
    }

    #[test]
    fn digits_are_row_one_columns_zero_through_nine() {
        assert_eq!(cell('0'), Some(Rect::new(0, 6, 5, 6)));
        assert_eq!(cell('9'), Some(Rect::new(9 * 5, 6, 5, 6)));
    }

    #[test]
    fn punctuation_lands_on_its_documented_cells() {
        assert_eq!(cell('.'), Some(Rect::new(11 * 5, 6, 5, 6)));
        assert_eq!(cell(':'), Some(Rect::new(12 * 5, 6, 5, 6)));
        assert_eq!(cell('('), Some(Rect::new(13 * 5, 6, 5, 6)));
        assert_eq!(cell(')'), Some(Rect::new(14 * 5, 6, 5, 6)));
        assert_eq!(cell('-'), Some(Rect::new(15 * 5, 6, 5, 6)));
        assert_eq!(cell('#'), Some(Rect::new(30 * 5, 6, 5, 6)));
        // Angle brackets reuse the square-bracket cells.
        assert_eq!(cell('<'), cell('['));
        assert_eq!(cell('>'), cell(']'));
    }

    #[test]
    fn space_is_the_blank_cell_at_row_zero_column_thirty() {
        assert_eq!(cell(' '), Some(Rect::new(30 * 5, 0, 5, 6)));
    }

    #[test]
    fn row_two_holds_the_question_mark_and_asterisk() {
        assert_eq!(cell('?'), Some(Rect::new(3 * 5, 12, 5, 6)));
        assert_eq!(cell('*'), Some(Rect::new(4 * 5, 12, 5, 6)));
    }

    #[test]
    fn non_ascii_glyphs_map_and_case_fold() {
        // Row 2 Nordic letters, both cases sharing a cell.
        assert_eq!(cell('\u{00C5}'), Some(Rect::new(0, 12, 5, 6)), "A-ring");
        assert_eq!(cell('\u{00E5}'), cell('\u{00C5}'), "a-ring folds to the same cell");
        assert_eq!(cell('\u{00D6}'), Some(Rect::new(5, 12, 5, 6)), "O-umlaut");
        assert_eq!(cell('\u{00F6}'), cell('\u{00D6}'));
        assert_eq!(cell('\u{00C4}'), Some(Rect::new(10, 12, 5, 6)), "A-umlaut");
        assert_eq!(cell('\u{00E4}'), cell('\u{00C4}'));
        // The ellipsis lives in row 1, column 10.
        assert_eq!(cell('\u{2026}'), Some(Rect::new(50, 6, 5, 6)));
    }

    #[test]
    fn characters_without_a_cell_return_none() {
        assert_eq!(cell('~'), None);
        assert_eq!(cell('\u{20AC}'), None); // euro sign: no cell
        assert_eq!(cell('\t'), None);
    }
}
