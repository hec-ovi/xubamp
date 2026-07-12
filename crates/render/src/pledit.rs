//! The playlist editor (PLEDIT) window: composited from `pledit.bmp` tiles at the classic collapsed
//! size (275x116), with the track list drawn over the middle band. Pure (returns a `Framebuffer`),
//! like the main window. Track rows use the clean-room 5x7 font (Winamp uses the skin's system font;
//! we approximate with our own bitmap font for now) coloured from `pledit.txt`.

use xubamp_skin::bmp::Image;
use xubamp_skin::color::Rgb;
use xubamp_skin::sprites::{self, Rect};
use xubamp_skin::{font, Skin};

use crate::{blit, Framebuffer};

/// One playlist row: its already-formatted title (`"N. Name"`) and duration (`"M:SS"` or empty).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub title: String,
    pub duration: String,
}

/// Playlist-window UI state: the rows to show, which track is playing, which rows are selected, and
/// the scroll position (0..=100, a percentage, matching Winamp). Survives the window closing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlState {
    pub rows: Vec<Row>,
    /// Index of the currently-playing track (drawn in the `Current` colour), if any.
    pub current: Option<usize>,
    /// Selected row indices (drawn over the `SelectedBG` colour).
    pub selected: Vec<usize>,
    /// The anchor row for shift-click range selection (the last plainly-clicked row).
    pub anchor: Option<usize>,
    /// Scroll position as a 0..=100 percentage of the overflow (Winamp's model).
    pub scroll: f32,
    /// When `Some`, the playlist is in "jump to file" mode (the classic `J` hotkey) showing this
    /// live search query; the first matching row is selected and scrolled into view. Winamp opens a
    /// separate dialog; we do an in-place incremental search instead.
    pub jump: Option<String>,
}

impl PlState {
    /// How many whole rows fit in the list area of a window `window_h` px tall.
    pub fn visible_rows(window_h: i32) -> usize {
        let list_h = window_h - sprites::PLEDIT_TITLE_H - sprites::PLEDIT_BOTTOM_H;
        (list_h / sprites::PLEDIT_ROW_H).max(0) as usize
    }

    /// Index of the first visible row, from the scroll percentage (Webamp's `percentToIndex`), for a
    /// window `window_h` px tall.
    pub fn scroll_offset(&self, window_h: i32) -> usize {
        let overflow = self.rows.len().saturating_sub(Self::visible_rows(window_h));
        ((self.scroll.clamp(0.0, 100.0) / 100.0) * overflow as f32).round() as usize
    }

    /// The track index at window-local pixel (`x`, `y`) in a window `window_h` px tall, or `None`
    /// when the point is not over a track row (left of the list, above the first row, below the last
    /// visible row, or past the final track).
    pub fn row_at(&self, x: i32, y: i32, window_h: i32) -> Option<usize> {
        if x < sprites::PLEDIT_LIST_X || y < sprites::PLEDIT_LIST_Y {
            return None;
        }
        let screen_row = (y - sprites::PLEDIT_LIST_Y) / sprites::PLEDIT_ROW_H;
        if screen_row >= Self::visible_rows(window_h) as i32 {
            return None;
        }
        let idx = self.scroll_offset(window_h) + screen_row as usize;
        (idx < self.rows.len()).then_some(idx)
    }

    /// Plain click on row `i`: select only it (clearing others) and make it the shift-anchor. If the
    /// row is already selected this is left alone, so a multi-selection survives (Winamp keeps it for
    /// a potential drag).
    pub fn click_select(&mut self, i: usize) {
        if !self.selected.contains(&i) {
            self.selected = vec![i];
            self.anchor = Some(i);
        }
    }

    /// Ctrl+click on row `i`: toggle it in the selection; it becomes the shift-anchor either way
    /// (Winamp sets the anchor even when un-selecting).
    pub fn ctrl_select(&mut self, i: usize) {
        if let Some(pos) = self.selected.iter().position(|&x| x == i) {
            self.selected.remove(pos);
        } else {
            self.selected.push(i);
        }
        self.anchor = Some(i);
    }

    /// Shift+click on row `i`: replace the selection with the contiguous range from the anchor to
    /// `i`. A no-op without an anchor; the anchor is left unchanged so successive shift-clicks all
    /// pivot around it.
    pub fn shift_select(&mut self, i: usize) {
        if let Some(a) = self.anchor {
            self.selected = (a.min(i)..=a.max(i)).collect();
        }
    }

    /// Click on the empty list area below the last track: clear the selection (Winamp's `SELECT_ZERO`;
    /// the anchor is left in place).
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Scroll by `tracks` rows (positive scrolls toward the end), in a window `window_h` px tall. A
    /// no-op when the whole list already fits.
    pub fn scroll_by_tracks(&mut self, tracks: f32, window_h: i32) {
        let overflow = self.rows.len().saturating_sub(Self::visible_rows(window_h));
        if overflow == 0 {
            return;
        }
        self.scroll = (self.scroll + tracks / overflow as f32 * 100.0).clamp(0.0, 100.0);
    }

    /// Adjust the scroll so row `i` is visible in a window `window_h` px tall (scrolls the minimum
    /// needed: to the top if above the view, to the bottom if below it, otherwise unchanged).
    pub fn scroll_to(&mut self, i: usize, window_h: i32) {
        let visible = Self::visible_rows(window_h);
        let overflow = self.rows.len().saturating_sub(visible);
        if overflow == 0 {
            self.scroll = 0.0;
            return;
        }
        let offset = self.scroll_offset(window_h);
        let target = if i < offset {
            i
        } else if i >= offset + visible {
            i + 1 - visible
        } else {
            return; // already visible
        };
        self.scroll = target.min(overflow) as f32 / overflow as f32 * 100.0;
    }

    /// The index of the first row matching the current jump query: every whitespace-separated token
    /// must appear in the row title (case-insensitive). `None` if there is no query or no match.
    pub fn jump_match(&self) -> Option<usize> {
        let query = self.jump.as_deref()?;
        let tokens: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if tokens.is_empty() {
            return None;
        }
        self.rows.iter().position(|r| {
            let title = r.title.to_lowercase();
            tokens.iter().all(|t| title.contains(t))
        })
    }

    /// Recompute the jump match after the query changed: select and scroll to the first matching row
    /// (in a window `window_h` px tall). Returns the matched index, if any.
    pub fn jump_refresh(&mut self, window_h: i32) -> Option<usize> {
        let m = self.jump_match();
        if let Some(i) = m {
            self.selected = vec![i];
            self.scroll_to(i, window_h);
        }
        m
    }
}

/// Compose the playlist window at `width` x `height` (already snapped to the resize grid and clamped
/// to at least the default size by the caller). Returns an empty frame of that size if the skin
/// ships no `pledit.bmp`.
pub fn compose(skin: &Skin, state: &PlState, width: i32, height: i32) -> Framebuffer {
    let (w, h) = (width.max(sprites::PLEDIT_W), height.max(sprites::PLEDIT_H));
    let mut fb = Framebuffer::new(w as u32, h as u32);
    let Some(sheet) = &skin.pledit else {
        return fb;
    };
    let colors = skin.pledit_colors.clone().unwrap_or_default();
    let mid_y0 = sprites::PLEDIT_TITLE_H;
    let mid_y1 = h - sprites::PLEDIT_BOTTOM_H;
    // The list content stretches with the window: from the left inset to the right edge tile.
    let list_w = w - sprites::PLEDIT_LIST_X - sprites::PLEDIT_RIGHT_TILE.w;

    // Title bar: corners, the centered "PLAYLIST" title, and the repeating fill between them.
    blit(&mut fb, sheet, sprites::PLEDIT_TOP_LEFT, 0, 0);
    blit(&mut fb, sheet, sprites::PLEDIT_TOP_RIGHT, w - sprites::PLEDIT_TOP_RIGHT.w, 0);
    let title_x = (w - sprites::PLEDIT_TITLE.w) / 2;
    tile_h(&mut fb, sheet, sprites::PLEDIT_TOP_TILE, sprites::PLEDIT_TOP_LEFT.w, title_x, 0);
    tile_h(&mut fb, sheet, sprites::PLEDIT_TOP_TILE, title_x + sprites::PLEDIT_TITLE.w, w - sprites::PLEDIT_TOP_RIGHT.w, 0);
    blit(&mut fb, sheet, sprites::PLEDIT_TITLE, title_x, 0);

    // Middle band: the list background fill, then the side edges over it (tiled vertically so a
    // taller window just adds more edge tiles and list rows).
    fill_rect(&mut fb, sprites::PLEDIT_LIST_X, mid_y0, list_w, mid_y1 - mid_y0, colors.normal_bg);
    tile_v(&mut fb, sheet, sprites::PLEDIT_LEFT_TILE, 0, mid_y0, mid_y1);
    tile_v(&mut fb, sheet, sprites::PLEDIT_RIGHT_TILE, w - sprites::PLEDIT_RIGHT_TILE.w, mid_y0, mid_y1);

    // Bottom bar: the two corners, with the repeating fill tile between them when the window is wider
    // than the default (at the default width the corners meet and the fill loop is empty).
    blit(&mut fb, sheet, sprites::PLEDIT_BOTTOM_LEFT, 0, mid_y1);
    tile_h(&mut fb, sheet, sprites::PLEDIT_BOTTOM_TILE, sprites::PLEDIT_BOTTOM_LEFT.w, w - sprites::PLEDIT_BOTTOM_RIGHT.w, mid_y1);
    blit(&mut fb, sheet, sprites::PLEDIT_BOTTOM_RIGHT, w - sprites::PLEDIT_BOTTOM_RIGHT.w, mid_y1);

    draw_rows(&mut fb, state, &colors, h, list_w);

    // Jump-to-file mode: black out the title bar's baked "PLAYLIST" and show the live query there.
    if let Some(query) = &state.jump {
        let x0 = sprites::PLEDIT_TOP_LEFT.w;
        let bar_w = w - x0 - sprites::PLEDIT_TOP_RIGHT.w;
        fill_rect(&mut fb, x0, 2, bar_w, sprites::PLEDIT_TITLE_H - 4, Rgb::new(0, 0, 0));
        let rgb = colors.current;
        let text = format!("JUMP: {query}");
        font::draw_text(&mut fb.rgba, fb.width, fb.height, x0 + 4, 7, &text, [rgb.r, rgb.g, rgb.b]);
    }
    fb
}

/// Draw the visible track rows over the list area of a window `window_h` px tall whose list content
/// is `list_w` px wide.
fn draw_rows(fb: &mut Framebuffer, state: &PlState, colors: &xubamp_skin::pledit::PlEdit, window_h: i32, list_w: i32) {
    let offset = state.scroll_offset(window_h);
    let visible = PlState::visible_rows(window_h);
    for (i, row) in state.rows.iter().enumerate().skip(offset).take(visible) {
        let screen_row = (i - offset) as i32;
        let y = sprites::PLEDIT_LIST_Y + screen_row * sprites::PLEDIT_ROW_H;
        if state.selected.contains(&i) {
            fill_rect(fb, sprites::PLEDIT_LIST_X, y - 2, list_w, sprites::PLEDIT_ROW_H, colors.selected_bg);
        }
        let rgb = if state.current == Some(i) { colors.current } else { colors.normal };
        let c = [rgb.r, rgb.g, rgb.b];
        // Right-aligned duration first, so we know how much room the title has.
        let dur_w = if row.duration.is_empty() {
            0
        } else {
            let dw = font::text_width(&row.duration) as i32;
            let dx = sprites::PLEDIT_LIST_X + list_w - dw - 3;
            font::draw_text(&mut fb.rgba, fb.width, fb.height, dx, y, &row.duration, c);
            dw + 4
        };
        // Title, truncated to the remaining width so it never runs into the duration.
        let avail = (list_w - 2 - dur_w).max(0) as u32;
        let max_chars = (avail / font::ADVANCE.max(1)) as usize;
        let title: String = row.title.chars().take(max_chars).collect();
        font::draw_text(&mut fb.rgba, fb.width, fb.height, sprites::PLEDIT_LIST_X + 1, y, &title, c);
    }
}

/// Fill a horizontal band [`x0`, `x1`) at row `y` by repeating `src` from `sheet`, clipping the last
/// tile.
fn tile_h(fb: &mut Framebuffer, sheet: &Image, src: Rect, x0: i32, x1: i32, y: i32) {
    let mut x = x0;
    while x < x1 {
        let clip = src.w.min(x1 - x);
        blit(fb, sheet, Rect::new(src.x, src.y, clip, src.h), x, y);
        x += src.w;
    }
}

/// Fill a vertical band [`y0`, `y1`) at column `x` by repeating `src`, clipping the last tile.
fn tile_v(fb: &mut Framebuffer, sheet: &Image, src: Rect, x: i32, y0: i32, y1: i32) {
    let mut y = y0;
    while y < y1 {
        let clip = src.h.min(y1 - y);
        blit(fb, sheet, Rect::new(src.x, src.y, src.w, clip), x, y);
        y += src.h;
    }
}

/// Fill the rectangle (`x`, `y`, `w`, `h`) with a solid opaque `color`, clipped to the framebuffer.
fn fill_rect(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, color: Rgb) {
    for yy in y.max(0)..(y + h).min(fb.height as i32) {
        for xx in x.max(0)..(x + w).min(fb.width as i32) {
            let o = ((yy as u32 * fb.width + xx as u32) * 4) as usize;
            fb.rgba[o] = color.r;
            fb.rgba[o + 1] = color.g;
            fb.rgba[o + 2] = color.b;
            fb.rgba[o + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xubamp_skin::pledit::PlEdit;

    fn solid_sheet(w: u32, h: u32, c: [u8; 4]) -> Image {
        Image { width: w, height: h, rgba: c.iter().copied().cycle().take((w * h * 4) as usize).collect() }
    }
    fn px(fb: &Framebuffer, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * fb.width + x) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    #[test]
    fn compose_draws_the_frame_and_list_background() {
        let sheet = solid_sheet(300, 120, [200, 0, 0, 255]); // the whole frame source is red
        let colors = PlEdit { normal_bg: Rgb::new(10, 20, 30), ..PlEdit::default() };
        let skin = Skin { pledit: Some(sheet), pledit_colors: Some(colors), ..Default::default() };
        let fb = compose(&skin, &PlState::default(), sprites::PLEDIT_W, sprites::PLEDIT_H);
        assert_eq!(fb.width, sprites::PLEDIT_W as u32);
        assert_eq!(fb.height, sprites::PLEDIT_H as u32);
        // A title-bar corner pixel comes from the (red) sheet: the frame was drawn.
        assert_eq!(px(&fb, 2, 2), [200, 0, 0, 255], "title-bar frame drawn from the sheet");
        // A middle-band interior pixel is the NormalBG list fill.
        assert_eq!(px(&fb, 100, 40), [10, 20, 30, 255], "list background filled with NormalBG");
        // A bottom-bar pixel comes from the sheet corners.
        assert_eq!(px(&fb, 2, 100), [200, 0, 0, 255], "bottom bar drawn");
        // Without a pledit sheet nothing is drawn (transparent), but the frame is still sized.
        let empty = compose(&Skin::default(), &PlState::default(), sprites::PLEDIT_W, sprites::PLEDIT_H);
        assert_eq!(px(&empty, 2, 2), [0, 0, 0, 0], "no pledit.bmp: an empty frame");
    }

    #[test]
    fn compose_fills_a_resized_window() {
        // A sheet whose bottom-tile source (x 179..204, y 0..38) is a distinct colour, so we can
        // check the widened bottom bar is filled by repeating it (not left transparent).
        let mut sheet = solid_sheet(300, 120, [200, 0, 0, 255]);
        for y in 0..38 {
            for x in 179..204 {
                let o = ((y * 300 + x) * 4) as usize;
                sheet.rgba[o..o + 4].copy_from_slice(&[0, 0, 200, 255]);
            }
        }
        let colors = PlEdit { normal_bg: Rgb::new(10, 20, 30), ..PlEdit::default() };
        let skin = Skin { pledit: Some(sheet), pledit_colors: Some(colors), ..Default::default() };

        // One segment wider and taller than the default.
        let (w, h) = (sprites::PLEDIT_W + sprites::PLEDIT_SEGMENT_W, sprites::PLEDIT_H + sprites::PLEDIT_SEGMENT_H);
        let fb = compose(&skin, &PlState::default(), w, h);
        assert_eq!((fb.width, fb.height), (w as u32, h as u32), "frame sized to the request");
        // The gap the corners used to leave (just past the 125px left corner, in the bottom band) is
        // now painted by the bottom fill tile.
        let bottom_y = (h - sprites::PLEDIT_BOTTOM_H + 5) as u32;
        assert_eq!(px(&fb, 130, bottom_y), [0, 0, 200, 255], "widened bottom bar filled by the tile");
        // The list background still fills to the new right edge.
        assert_eq!(px(&fb, (w - 25) as u32, 40), [10, 20, 30, 255], "list background stretched to the wider width");
    }

    #[test]
    fn visible_rows_and_scroll_offset_track_the_size() {
        // Default height: 4 rows. A taller window shows more.
        assert_eq!(PlState::visible_rows(sprites::PLEDIT_H), 4, "(116-20-38)/13 = 4 rows fit");
        assert_eq!(
            PlState::visible_rows(sprites::PLEDIT_H + sprites::PLEDIT_SEGMENT_H),
            6,
            "one taller segment (+29px) fits 2 more rows",
        );
        let mut s = PlState { rows: vec![Row::default(); 10], ..Default::default() };
        assert_eq!(s.scroll_offset(sprites::PLEDIT_H), 0, "scroll 0 -> top");
        s.scroll = 100.0;
        assert_eq!(s.scroll_offset(sprites::PLEDIT_H), 6, "scroll 100 -> overflow (10-4)");
        s.scroll = 50.0;
        assert_eq!(s.scroll_offset(sprites::PLEDIT_H), 3, "scroll 50 -> round(0.5*6)");
    }

    fn rows(n: usize) -> Vec<Row> {
        (0..n).map(|i| Row { title: format!("track {i}"), duration: String::new() }).collect()
    }

    #[test]
    fn row_at_maps_clicks_to_indices_with_scroll() {
        let h = sprites::PLEDIT_H; // 4 visible rows
        let mut s = PlState { rows: rows(20), ..Default::default() };
        // Above the list, or left of it: no row.
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y - 1, h), None);
        assert_eq!(s.row_at(sprites::PLEDIT_LIST_X - 1, sprites::PLEDIT_LIST_Y, h), None);
        // The four visible rows, top to bottom.
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y, h), Some(0));
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y + sprites::PLEDIT_ROW_H, h), Some(1));
        // Past the last visible row: none.
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y + 4 * sprites::PLEDIT_ROW_H, h), None);
        // With scroll to the bottom, the top visible row shifts to the overflow (20-4 = 16).
        s.scroll = 100.0;
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y, h), Some(16));
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y + 3 * sprites::PLEDIT_ROW_H, h), Some(19));
    }

    #[test]
    fn row_at_returns_none_past_the_last_track() {
        let h = sprites::PLEDIT_H;
        let s = PlState { rows: rows(2), ..Default::default() }; // 2 tracks in 4 visible slots
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y + sprites::PLEDIT_ROW_H, h), Some(1));
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y + 2 * sprites::PLEDIT_ROW_H, h), None);
    }

    #[test]
    fn selection_click_ctrl_shift_match_winamp() {
        // Plain click selects only that row and sets the anchor.
        let mut s = PlState { rows: rows(10), ..Default::default() };
        s.click_select(3);
        assert_eq!(s.selected, vec![3]);
        assert_eq!(s.anchor, Some(3));
        // Plain click on an already-selected row is a no-op (a multi-selection survives for a drag).
        s.selected = vec![3, 4, 5];
        s.click_select(4);
        assert_eq!(s.selected, vec![3, 4, 5]);

        // Ctrl+click toggles a single row and moves the anchor, even when un-selecting.
        let mut s = PlState { rows: rows(10), ..Default::default() };
        s.click_select(2);
        s.ctrl_select(5);
        assert_eq!(s.selected, vec![2, 5]);
        assert_eq!(s.anchor, Some(5));
        s.ctrl_select(2);
        assert_eq!(s.selected, vec![5]);
        assert_eq!(s.anchor, Some(2), "anchor moves even on un-select");

        // Shift+click replaces the selection with the range from the anchor; the anchor stays put.
        let mut s = PlState { rows: rows(10), ..Default::default() };
        s.click_select(2);
        s.shift_select(5);
        assert_eq!(s.selected, vec![2, 3, 4, 5]);
        assert_eq!(s.anchor, Some(2));
        s.shift_select(0);
        assert_eq!(s.selected, vec![0, 1, 2], "successive shift-clicks pivot around the same anchor");

        // Shift with no anchor is inert.
        let mut s = PlState { rows: rows(10), ..Default::default() };
        s.shift_select(4);
        assert!(s.selected.is_empty());
    }

    #[test]
    fn clearing_selection_leaves_the_anchor() {
        let mut s = PlState { rows: rows(5), ..Default::default() };
        s.click_select(2);
        s.clear_selection();
        assert!(s.selected.is_empty());
        assert_eq!(s.anchor, Some(2));
    }

    #[test]
    fn wheel_scroll_moves_within_bounds() {
        let h = sprites::PLEDIT_H; // 4 visible
        let mut s = PlState { rows: rows(14), ..Default::default() }; // overflow 10
        s.scroll_by_tracks(2.0, h); // +2/10 = 20%
        assert!((s.scroll - 20.0).abs() < 0.01);
        s.scroll_by_tracks(-100.0, h);
        assert_eq!(s.scroll, 0.0, "clamps at the top");
        s.scroll_by_tracks(1000.0, h);
        assert_eq!(s.scroll, 100.0, "clamps at the bottom");
        // No overflow: inert.
        let mut s = PlState { rows: rows(2), ..Default::default() };
        s.scroll_by_tracks(5.0, h);
        assert_eq!(s.scroll, 0.0);
    }

    #[test]
    fn scroll_to_brings_a_row_into_view() {
        let h = sprites::PLEDIT_H; // 4 visible, overflow 16 for 20 rows
        let mut s = PlState { rows: rows(20), ..Default::default() };
        s.scroll_to(19, h);
        assert_eq!(s.scroll_offset(h), 16, "last row lands at the bottom of the view");
        s.scroll_to(0, h);
        assert_eq!(s.scroll_offset(h), 0);
        // An already-visible row does not move the scroll.
        s.scroll = 25.0;
        let before = s.scroll;
        s.scroll_to(s.scroll_offset(h), h);
        assert_eq!(s.scroll, before);
    }

    #[test]
    fn jump_match_finds_tokens_case_insensitively() {
        let mk = |t: &str| Row { title: t.into(), duration: String::new() };
        let mut s = PlState {
            rows: vec![mk("1. Cry Wolf"), mk("2. Take On Me"), mk("3. The Sun Always Shines")],
            ..Default::default()
        };
        s.jump = Some("take".into());
        assert_eq!(s.jump_match(), Some(1));
        s.jump = Some("TAKE me".into());
        assert_eq!(s.jump_match(), Some(1), "all tokens, any order, case-insensitive");
        s.jump = Some("sun shines".into());
        assert_eq!(s.jump_match(), Some(2));
        s.jump = Some("nope".into());
        assert_eq!(s.jump_match(), None);
        s.jump = Some("   ".into());
        assert_eq!(s.jump_match(), None, "whitespace-only query matches nothing");
        s.jump = None;
        assert_eq!(s.jump_match(), None);
    }

    #[test]
    fn jump_refresh_selects_and_scrolls_to_the_match() {
        let h = sprites::PLEDIT_H;
        let mut rs = rows(20);
        rs[15].title = "15. needle".into();
        let mut s = PlState { rows: rs, ..Default::default() };
        s.jump = Some("needle".into());
        assert_eq!(s.jump_refresh(h), Some(15));
        assert_eq!(s.selected, vec![15]);
        let off = s.scroll_offset(h);
        assert!(off <= 15 && 15 < off + PlState::visible_rows(h), "match scrolled into view");
    }
}
