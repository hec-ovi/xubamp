//! The playlist editor (PLEDIT) window: composited from `pledit.bmp` tiles in either its expanded,
//! resizable form or its 14px windowshade form. Pure (returns a `Framebuffer`), like the main
//! window. Track rows use the clean-room 5x7 font (Winamp uses the skin's system font; we
//! approximate with our own bitmap font for now) coloured from `pledit.txt`.

use std::collections::HashSet;

use xubamp_skin::bmp::Image;
use xubamp_skin::color::Rgb;
use xubamp_skin::sprites::{self, Rect};
use xubamp_skin::{font, Skin};

use crate::{blit, Framebuffer};

/// Format a whole-second count as Winamp's `M:SS` (or `MM:SS`, `MMM:SS` for long values): minutes
/// with no leading zero, seconds always two digits. There is no hours field, matching the classic
/// playlist readout.
fn mmss(total_secs: u32) -> String {
    format!("{}:{:02}", total_secs / 60, total_secs % 60)
}

/// The bottom-bar running-time readout: `selected/total`, where the left side sums the durations of
/// the selected rows and the right side sums every row, each as `M:SS`. Unknown row durations count
/// as zero (the player does not probe lengths yet), so a fresh list reads `0:00/0:00`.
pub fn running_time_message(state: &PlState) -> String {
    let selected: HashSet<usize> = state.selected.iter().copied().collect();
    let mut sel_secs = 0u32;
    let mut total_secs = 0u32;
    for (i, row) in state.rows.iter().enumerate() {
        let secs = row.duration_secs.unwrap_or(0);
        total_secs = total_secs.saturating_add(secs);
        if selected.contains(&i) {
            sel_secs = sel_secs.saturating_add(secs);
        }
    }
    format!("{}/{}", mmss(sel_secs), mmss(total_secs))
}

/// One playlist row: its already-formatted title (`"N. Name"`) and duration (`"M:SS"` or empty).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub title: String,
    pub duration: String,
    /// Track length in seconds when known, for the bottom-bar selected/total readout. Rows created
    /// from a plain path carry `None` (the player does not probe durations yet); a real probe is a
    /// later follow-up, at which point the per-row `duration` string can be derived from this too.
    pub duration_secs: Option<u32>,
}

/// The two buttons at the playlist title bar's right edge. Both expanded and shaded forms use the
/// same dynamic destination rectangles; only the shade button's pressed sprite changes from
/// collapse to restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleButton {
    Shade,
    Close,
}

/// Interactive playlist regions. Keeping this geometry pure makes the title buttons, drag band,
/// and the different expanded/shaded resize targets testable without a Wayland compositor.
/// The five bottom-bar menu buttons. Each opens a flyout: ADD (url/dir/file), REM (remove), SEL
/// (selection), MISC (sort/info), LIST (new/save/load). Winamp's exact positions: all are 22x18 and
/// 12px off the bottom; ADD/REM/SEL/MISC are left-anchored, LIST is anchored to the right edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomButton {
    Add,
    Rem,
    Sel,
    Misc,
    List,
}

impl BottomButton {
    /// The five buttons in left-to-right order, so hit-testing and drawing can iterate them.
    pub const ALL: [BottomButton; 5] = [
        BottomButton::Add,
        BottomButton::Rem,
        BottomButton::Sel,
        BottomButton::Misc,
        BottomButton::List,
    ];

    /// The three-letter face label the fallback (no-skin) frame draws on the button.
    fn label(self) -> &'static str {
        match self {
            BottomButton::Add => "ADD",
            BottomButton::Rem => "REM",
            BottomButton::Sel => "SEL",
            BottomButton::Misc => "MSC",
            BottomButton::List => "LST",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    TitleButton(TitleButton),
    TitleBar,
    Resize,
    /// One of the five bottom-bar cluster buttons, each of which opens its flyout menu.
    BottomMenu(BottomButton),
    /// The scrollbar track down the right edge: a press or drag here sets the scroll position.
    Scrollbar,
    /// The live mini clock in the bottom bar; clicking it toggles elapsed/remaining, like the
    /// main window's clock.
    MiniTime,
    /// The list area proper (rows, and the empty space under the last row).
    Body,
    /// Dead window chrome (the side frames and the bottom bar between its controls). Pressing
    /// here drags the pane, like the title bar, but never joins the double-click shade toggle.
    Frame,
    None,
}

/// The scrollbar sits `SCROLLBAR_RIGHT` px in from the right edge and is `SCROLLBAR_W` px wide,
/// running down the list area between the title and bottom bars.
const SCROLLBAR_RIGHT: i32 = 15;
const SCROLLBAR_W: i32 = 8;
const SCROLLBAR_MIN_THUMB: i32 = 8;

/// The scrollbar track rectangle (x, y, w, h) for the current window size.
fn scrollbar_track(width: i32, height: i32) -> (i32, i32, i32, i32) {
    let x = width - SCROLLBAR_RIGHT;
    let y = sprites::PLEDIT_TITLE_H;
    let h = (height - sprites::PLEDIT_TITLE_H - sprites::PLEDIT_BOTTOM_H).max(0);
    (x, y, SCROLLBAR_W, h)
}

/// Is a window-local point inside the scrollbar track?
fn in_scrollbar_track(width: i32, height: i32, x: i32, y: i32) -> bool {
    let (sx, sy, sw, sh) = scrollbar_track(width, height);
    x >= sx && x < sx + sw && y >= sy && y < sy + sh
}

/// The live mini clock's rectangle (x, y, w, h) in the bottom bar: the classic slot left of the
/// LIST cluster, anchored to the bottom-right like the rest of the bar. Sized for `-MM:SS` in the
/// built-in 5x7 font.
pub fn mini_time_rect(width: i32, height: i32) -> (i32, i32, i32, i32) {
    (width - MINI_TIME_RIGHT, height - MINI_TIME_BOTTOM, 36, 8)
}

/// Is a window-local point inside the mini clock's click target?
fn in_mini_time(width: i32, height: i32, x: i32, y: i32) -> bool {
    let (mx, my, mw, mh) = mini_time_rect(width, height);
    x >= mx && x < mx + mw && y >= my && y < my + mh
}

/// The mini clock's distance in from the right edge and up from the bottom, from the classic
/// playlist bottom-right layout (Webamp: `.mini-time` at (66,23) of the right-anchored 150px
/// section).
const MINI_TIME_RIGHT: i32 = 84;
const MINI_TIME_BOTTOM: i32 = 15;

/// The scrollbar thumb rectangle, or `None` when the list fits and there is nothing to scroll.
pub fn scrollbar_thumb_rect(state: &PlState, width: i32, height: i32) -> Option<(i32, i32, i32, i32)> {
    let total = state.rows.len();
    let visible = PlState::visible_rows(height);
    if visible == 0 || total <= visible {
        return None;
    }
    let (x, y, w, h) = scrollbar_track(width, height);
    let thumb_h = (((visible as f32 / total as f32) * h as f32).round() as i32)
        .clamp(SCROLLBAR_MIN_THUMB, h.max(SCROLLBAR_MIN_THUMB));
    let travel = (h - thumb_h).max(0);
    let offset = ((state.scroll.clamp(0.0, 100.0) / 100.0) * travel as f32).round() as i32;
    Some((x, y + offset, w, thumb_h))
}

pub const ADD_BUTTON_X: i32 = 14;
pub const BOTTOM_BUTTON_BOTTOM: i32 = 12;
pub const BOTTOM_BUTTON_W: i32 = 22;
pub const BOTTOM_BUTTON_H: i32 = 18;
/// Left offsets of the left-anchored buttons and the right offset of the right-anchored LIST button.
const REM_BUTTON_X: i32 = 43;
const SEL_BUTTON_X: i32 = 72;
const MISC_BUTTON_X: i32 = 101;
const LIST_BUTTON_RIGHT: i32 = 22;

/// The window-local rectangle of a bottom-bar button, given the current window size. All buttons
/// sit `BOTTOM_BUTTON_BOTTOM` px off the bottom; LIST is measured from the right edge so it tracks a
/// resized window.
pub fn bottom_button_rect(button: BottomButton, width: i32, height: i32) -> (i32, i32, i32, i32) {
    let y = height - BOTTOM_BUTTON_BOTTOM - BOTTOM_BUTTON_H;
    let x = match button {
        BottomButton::Add => ADD_BUTTON_X,
        BottomButton::Rem => REM_BUTTON_X,
        BottomButton::Sel => SEL_BUTTON_X,
        BottomButton::Misc => MISC_BUTTON_X,
        BottomButton::List => width - LIST_BUTTON_RIGHT - BOTTOM_BUTTON_W,
    };
    (x, y, BOTTOM_BUTTON_W, BOTTOM_BUTTON_H)
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
    /// Whether the playlist is collapsed to its 14px windowshade strip. Its expanded size is owned
    /// by the platform layer so restoring does not lose a user resize.
    pub shade: bool,
    /// Held title button, for depressed feedback until release. The action fires only when release
    /// lands on this same button.
    pub pressed_title: Option<TitleButton>,
    /// Whichever bottom cluster button stays depressed while its popup menu is open.
    pub pressed_menu: Option<BottomButton>,
    /// The live playback clock for the bottom-bar mini time, in whole seconds, already in the
    /// main window's selected elapsed/remaining representation. `None` draws nothing (nothing
    /// loaded, or the paused blink's hidden beat). Fed by the platform layer each tick.
    pub clock: Option<u32>,
    /// Whether the mini clock leads with the remaining-time minus sign.
    pub clock_negative: bool,
}

/// The x coordinate of a 9px playlist title button at `right` pixels from the window's right edge.
fn title_button_x(width: i32, right: i32) -> i32 {
    width - right - sprites::PLEDIT_TITLE_BUTTON_W
}

/// Map a playlist-local point to its interaction region in either expanded or shaded mode.
pub fn region_at(state: &PlState, width: i32, height: i32, x: i32, y: i32) -> Region {
    let min_h = if state.shade {
        sprites::PLEDIT_SHADE_H
    } else {
        sprites::PLEDIT_H
    };
    let (width, height) = (width.max(sprites::PLEDIT_W), height.max(min_h));
    if x < 0 || y < 0 || x >= width || y >= height {
        return Region::None;
    }
    let button_y = sprites::PLEDIT_TITLE_BUTTON_Y;
    let button_h = sprites::PLEDIT_TITLE_BUTTON_W;
    if y >= button_y && y < button_y + button_h {
        let shade_x = title_button_x(width, sprites::PLEDIT_SHADE_BUTTON_RIGHT);
        if x >= shade_x && x < shade_x + sprites::PLEDIT_TITLE_BUTTON_W {
            return Region::TitleButton(TitleButton::Shade);
        }
        let close_x = title_button_x(width, sprites::PLEDIT_CLOSE_BUTTON_RIGHT);
        if x >= close_x && x < close_x + sprites::PLEDIT_TITLE_BUTTON_W {
            return Region::TitleButton(TitleButton::Close);
        }
    }

    if state.shade {
        let resize_x = title_button_x(width, sprites::PLEDIT_SHADE_RESIZE_RIGHT);
        if y >= button_y
            && y < button_y + button_h
            && x >= resize_x
            && x < resize_x + sprites::PLEDIT_TITLE_BUTTON_W
        {
            return Region::Resize;
        }
        if y < sprites::PLEDIT_SHADE_H {
            Region::TitleBar
        } else {
            Region::None
        }
    } else if x >= width - 20 && y >= height - 20 {
        Region::Resize
    } else if let Some(button) = bottom_button_at(width, height, x, y) {
        Region::BottomMenu(button)
    } else if in_scrollbar_track(width, height, x, y) {
        Region::Scrollbar
    } else if in_mini_time(width, height, x, y) {
        Region::MiniTime
    } else if y < sprites::PLEDIT_TITLE_H {
        Region::TitleBar
    } else if x >= sprites::PLEDIT_LIST_X
        && x < width - SCROLLBAR_RIGHT
        && y >= sprites::PLEDIT_LIST_Y
        && y < height - sprites::PLEDIT_BOTTOM_H
    {
        Region::Body
    } else {
        // Side frames, the strip between the title and the first row, and the bottom bar's dead
        // space: all draggable chrome.
        Region::Frame
    }
}

/// Which bottom cluster button, if any, is under a window-local point.
fn bottom_button_at(width: i32, height: i32, x: i32, y: i32) -> Option<BottomButton> {
    BottomButton::ALL.into_iter().find(|&button| {
        let (bx, by, bw, bh) = bottom_button_rect(button, width, height);
        x >= bx && x < bx + bw && y >= by && y < by + bh
    })
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

    /// SEL cluster "Select All": select every row. Returns whether the selection changed.
    pub fn select_all(&mut self) -> bool {
        let all: Vec<usize> = (0..self.rows.len()).collect();
        if self.selected.len() == all.len() {
            return false;
        }
        self.selected = all;
        true
    }

    /// SEL cluster "Select None". Returns whether the selection changed.
    pub fn select_none(&mut self) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        self.selected.clear();
        true
    }

    /// SEL cluster "Invert Selection": every previously-unselected row becomes selected and vice
    /// versa. Returns whether anything changed (only an empty playlist leaves it unchanged).
    pub fn invert_selection(&mut self) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        let selected: HashSet<usize> = self.selected.iter().copied().collect();
        self.selected = (0..self.rows.len())
            .filter(|i| !selected.contains(i))
            .collect();
        true
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

    /// Set the scroll position from a scrollbar press or drag at window-local `y`, mapping the track
    /// span linearly to 0..=100. A no-op when the list fits (nothing to scroll).
    pub fn set_scroll_from_y(&mut self, y: i32, width: i32, height: i32) {
        if self.rows.len() <= Self::visible_rows(height) {
            return;
        }
        let (_, track_y, _, track_h) = scrollbar_track(width, height);
        if track_h <= 0 {
            return;
        }
        let frac = ((y - track_y) as f32 / track_h as f32).clamp(0.0, 1.0);
        self.scroll = frac * 100.0;
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
}

/// Compose the playlist window at `width` x `height`. Expanded geometry is clamped to the classic
/// minimum; shaded geometry preserves the width but collapses completely to 14px. Incomplete skins
/// get an opaque clean-room fallback frame instead of an invisible/invalid surface.
pub fn compose(skin: &Skin, state: &PlState, width: i32, height: i32) -> Framebuffer {
    if state.shade {
        return compose_shade(skin, state, width);
    }

    let (w, h) = (width.max(sprites::PLEDIT_W), height.max(sprites::PLEDIT_H));
    let mut fb = Framebuffer::new(w as u32, h as u32);
    let colors = skin.pledit_colors.clone().unwrap_or_default();
    draw_fallback_expanded(&mut fb, skin, state, &colors);

    let Some(sheet) = &skin.pledit else {
        return fb;
    };
    let mid_y0 = sprites::PLEDIT_TITLE_H;
    let mid_y1 = h - sprites::PLEDIT_BOTTOM_H;
    // The list content stretches with the window: from the left inset to the right edge tile.
    let list_w = w - sprites::PLEDIT_LIST_X - sprites::PLEDIT_RIGHT_TILE.w;

    // Title bar: corners, the centered "PLAYLIST" title, and the repeating fill between them.
    blit(&mut fb, sheet, sprites::PLEDIT_TOP_LEFT, 0, 0);
    blit(
        &mut fb,
        sheet,
        sprites::PLEDIT_TOP_RIGHT,
        w - sprites::PLEDIT_TOP_RIGHT.w,
        0,
    );
    let title_x = (w - sprites::PLEDIT_TITLE.w) / 2;
    tile_h(
        &mut fb,
        sheet,
        sprites::PLEDIT_TOP_TILE,
        sprites::PLEDIT_TOP_LEFT.w,
        title_x,
        0,
    );
    tile_h(
        &mut fb,
        sheet,
        sprites::PLEDIT_TOP_TILE,
        title_x + sprites::PLEDIT_TITLE.w,
        w - sprites::PLEDIT_TOP_RIGHT.w,
        0,
    );
    blit(&mut fb, sheet, sprites::PLEDIT_TITLE, title_x, 0);
    draw_pressed_title(&mut fb, sheet, state, w, false);

    // Middle band: the list background fill, then the side edges over it (tiled vertically so a
    // taller window just adds more edge tiles and list rows).
    fill_rect(
        &mut fb,
        sprites::PLEDIT_LIST_X,
        mid_y0,
        list_w,
        mid_y1 - mid_y0,
        colors.normal_bg,
    );
    tile_v(&mut fb, sheet, sprites::PLEDIT_LEFT_TILE, 0, mid_y0, mid_y1);
    tile_v(
        &mut fb,
        sheet,
        sprites::PLEDIT_RIGHT_TILE,
        w - sprites::PLEDIT_RIGHT_TILE.w,
        mid_y0,
        mid_y1,
    );

    // Bottom bar: the two corners, with the repeating fill tile between them when the window is wider
    // than the default (at the default width the corners meet and the fill loop is empty).
    blit(&mut fb, sheet, sprites::PLEDIT_BOTTOM_LEFT, 0, mid_y1);
    tile_h(
        &mut fb,
        sheet,
        sprites::PLEDIT_BOTTOM_TILE,
        sprites::PLEDIT_BOTTOM_LEFT.w,
        w - sprites::PLEDIT_BOTTOM_RIGHT.w,
        mid_y1,
    );
    blit(
        &mut fb,
        sheet,
        sprites::PLEDIT_BOTTOM_RIGHT,
        w - sprites::PLEDIT_BOTTOM_RIGHT.w,
        mid_y1,
    );
    if let Some(button) = state.pressed_menu {
        let (bx, by, bw, bh) = bottom_button_rect(button, w, h);
        if button == BottomButton::Add {
            // The skin bakes an ADD-pressed sprite; the other clusters have none, so darken them.
            blit(
                &mut fb,
                sheet,
                Rect::new(23, 149, BOTTOM_BUTTON_W, BOTTOM_BUTTON_H),
                bx,
                by,
            );
        } else {
            darken_rect(&mut fb, bx, by, bw, bh);
        }
    }

    draw_running_time(&mut fb, state, &colors, w, h);
    draw_mini_time(&mut fb, state, &colors, w, h);
    draw_rows(&mut fb, state, &colors, h, list_w);
    draw_scrollbar(&mut fb, state, &colors, w, h);
    fb
}

/// Multiply a rectangle's pixels toward black, a skin-independent "pressed" cue for the cluster
/// buttons that have no baked pressed sprite (everything but ADD).
fn darken_rect(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32) {
    for yy in y.max(0)..(y + h).min(fb.height as i32) {
        for xx in x.max(0)..(x + w).min(fb.width as i32) {
            let o = ((yy as u32 * fb.width + xx as u32) * 4) as usize;
            for c in 0..3 {
                fb.rgba[o + c] = (fb.rgba[o + c] as u32 * 5 / 8) as u8;
            }
        }
    }
}

/// Draw the `selected/total` running-time readout, right-aligned in the open bottom-bar space just
/// left of the LIST button, in the playlist's normal text colour.
fn draw_running_time(
    fb: &mut Framebuffer,
    state: &PlState,
    colors: &xubamp_skin::pledit::PlEdit,
    width: i32,
    height: i32,
) {
    let text = running_time_message(state);
    let right = width - LIST_BUTTON_RIGHT - BOTTOM_BUTTON_W - 5;
    let x = right - font::text_width(&text) as i32;
    let y = height - BOTTOM_BUTTON_BOTTOM - BOTTOM_BUTTON_H + 5;
    font::draw_text(
        &mut fb.rgba,
        fb.width,
        fb.height,
        x,
        y,
        &text,
        [colors.normal.r, colors.normal.g, colors.normal.b],
    );
}

/// Draw the live mini clock in its classic bottom-bar slot: `MM:SS` of the current track (a
/// leading minus in remaining mode), in the playlist's current-track colour. Blank when nothing
/// is loaded or during the paused blink's hidden beat (`state.clock` is `None` then).
fn draw_mini_time(
    fb: &mut Framebuffer,
    state: &PlState,
    colors: &xubamp_skin::pledit::PlEdit,
    width: i32,
    height: i32,
) {
    let Some(secs) = state.clock else {
        return;
    };
    let sign = if state.clock_negative { "-" } else { "" };
    let text = format!("{}{:02}:{:02}", sign, secs / 60, secs % 60);
    let (x, y, _, _) = mini_time_rect(width, height);
    font::draw_text(
        &mut fb.rgba,
        fb.width,
        fb.height,
        x,
        y,
        &text,
        [colors.current.r, colors.current.g, colors.current.b],
    );
}

/// Compose the 14px playlist windowshade strip. The width is independent of the expanded/shaded
/// transition and remains horizontally resizable.
pub fn compose_shade(skin: &Skin, state: &PlState, width: i32) -> Framebuffer {
    let w = width.max(sprites::PLEDIT_W);
    let mut fb = Framebuffer::new(w as u32, sprites::PLEDIT_SHADE_H as u32);
    let colors = skin.pledit_colors.clone().unwrap_or_default();

    // Always establish an opaque base. Complete skins cover it with their PLEDIT tiles; the
    // clean-room built-in skin deliberately has no PLEDIT sheet and keeps this fallback.
    fill_rect(
        &mut fb,
        0,
        0,
        w,
        sprites::PLEDIT_SHADE_H,
        fallback_frame_color(skin),
    );
    if let Some(sheet) = &skin.pledit {
        tile_h(&mut fb, sheet, sprites::PLEDIT_SHADE_TILE, 0, w, 0);
        blit(&mut fb, sheet, sprites::PLEDIT_SHADE_LEFT, 0, 0);
        blit(
            &mut fb,
            sheet,
            sprites::PLEDIT_SHADE_RIGHT,
            w - sprites::PLEDIT_SHADE_RIGHT.w,
            0,
        );
        draw_pressed_title(&mut fb, sheet, state, w, true);
    } else {
        draw_fallback_title_buttons(&mut fb, state, w);
    }
    draw_shade_track(&mut fb, state, &colors, w);
    fb
}

/// Pick a stable frame colour from the loaded main sheet when possible, so the no-PLEDIT fallback
/// belongs visually to the current skin. Fall back to a clean-room dark blue for an empty skin.
fn fallback_frame_color(skin: &Skin) -> Rgb {
    let Some(main) = &skin.main else {
        return Rgb::new(24, 48, 62);
    };
    if main.width == 0 || main.height == 0 || main.rgba.len() < 4 {
        return Rgb::new(24, 48, 62);
    }
    let x = 2.min(main.width - 1);
    let y = 2.min(main.height - 1);
    let o = ((y * main.width + x) * 4) as usize;
    if o + 3 >= main.rgba.len() || main.rgba[o + 3] == 0 {
        Rgb::new(24, 48, 62)
    } else {
        Rgb::new(main.rgba[o], main.rgba[o + 1], main.rgba[o + 2])
    }
}

fn draw_fallback_expanded(
    fb: &mut Framebuffer,
    skin: &Skin,
    state: &PlState,
    colors: &xubamp_skin::pledit::PlEdit,
) {
    let (w, h) = (fb.width as i32, fb.height as i32);
    let frame = fallback_frame_color(skin);
    fill_rect(fb, 0, 0, w, h, frame);
    let list_w = w - sprites::PLEDIT_LIST_X - sprites::PLEDIT_RIGHT_TILE.w;
    fill_rect(
        fb,
        sprites::PLEDIT_LIST_X,
        sprites::PLEDIT_TITLE_H,
        list_w,
        h - sprites::PLEDIT_TITLE_H - sprites::PLEDIT_BOTTOM_H,
        colors.normal_bg,
    );
    let title = "PLAYLIST";
    let title_x = (w - font::text_width(title) as i32) / 2;
    font::draw_text(
        &mut fb.rgba,
        fb.width,
        fb.height,
        title_x,
        6,
        title,
        [colors.normal.r, colors.normal.g, colors.normal.b],
    );
    draw_fallback_title_buttons(fb, state, w);
    draw_fallback_bottom_buttons(fb, state, w, h, colors);
    draw_running_time(fb, state, colors, w, h);
    draw_mini_time(fb, state, colors, w, h);
    draw_rows(fb, state, colors, h, list_w);
    draw_scrollbar(fb, state, colors, w, h);
}

/// Draw the scrollbar thumb over the right edge when the list overflows, as a bevelled cap so the
/// grab target is obvious. Nothing is drawn when everything fits.
fn draw_scrollbar(
    fb: &mut Framebuffer,
    state: &PlState,
    colors: &xubamp_skin::pledit::PlEdit,
    width: i32,
    height: i32,
) {
    let Some((x, y, w, h)) = scrollbar_thumb_rect(state, width, height) else {
        return;
    };
    fill_rect(fb, x, y, w, h, colors.selected_bg);
    // A one-pixel light top/left and the darker list background on the bottom/right reads as a
    // raised cap against the recessed track.
    fill_rect(fb, x, y, w, 1, colors.current);
    fill_rect(fb, x, y, 1, h, colors.current);
    fill_rect(fb, x, y + h - 1, w, 1, colors.normal_bg);
    fill_rect(fb, x + w - 1, y, 1, h, colors.normal_bg);
}

/// Draw all five bottom cluster buttons for the no-skin fallback frame, each a small bevelled face
/// with its three-letter label, so the base skin's playlist exposes the same working controls.
fn draw_fallback_bottom_buttons(
    fb: &mut Framebuffer,
    state: &PlState,
    width: i32,
    height: i32,
    colors: &xubamp_skin::pledit::PlEdit,
) {
    let light = Rgb::new(92, 146, 158);
    let dark = Rgb::new(5, 18, 24);
    for button in BottomButton::ALL {
        let (x, y, w, h) = bottom_button_rect(button, width, height);
        let face = if state.pressed_menu == Some(button) {
            colors.selected_bg
        } else {
            Rgb::new(34, 64, 76)
        };
        fill_rect(fb, x, y, w, h, face);
        fill_rect(fb, x, y, w, 1, light);
        fill_rect(fb, x, y, 1, h, light);
        fill_rect(fb, x, y + h - 1, w, 1, dark);
        fill_rect(fb, x + w - 1, y, 1, h, dark);
        font::draw_text(
            &mut fb.rgba,
            fb.width,
            fb.height,
            x + 2,
            y + 6,
            button.label(),
            [colors.normal.r, colors.normal.g, colors.normal.b],
        );
    }
}

fn draw_pressed_title(
    fb: &mut Framebuffer,
    sheet: &Image,
    state: &PlState,
    width: i32,
    shaded: bool,
) {
    let Some(button) = state.pressed_title else {
        return;
    };
    let (src, right) = match button {
        TitleButton::Shade if shaded => (
            sprites::PLEDIT_EXPAND_PRESSED,
            sprites::PLEDIT_SHADE_BUTTON_RIGHT,
        ),
        TitleButton::Shade => (
            sprites::PLEDIT_COLLAPSE_PRESSED,
            sprites::PLEDIT_SHADE_BUTTON_RIGHT,
        ),
        TitleButton::Close => (
            sprites::PLEDIT_CLOSE_PRESSED,
            sprites::PLEDIT_CLOSE_BUTTON_RIGHT,
        ),
    };
    blit(
        fb,
        sheet,
        src,
        title_button_x(width, right),
        sprites::PLEDIT_TITLE_BUTTON_Y,
    );
}

fn draw_fallback_title_buttons(fb: &mut Framebuffer, state: &PlState, width: i32) {
    for (button, right) in [
        (TitleButton::Shade, sprites::PLEDIT_SHADE_BUTTON_RIGHT),
        (TitleButton::Close, sprites::PLEDIT_CLOSE_BUTTON_RIGHT),
    ] {
        let x = title_button_x(width, right);
        let y = sprites::PLEDIT_TITLE_BUTTON_Y;
        let pressed = state.pressed_title == Some(button);
        let face = if pressed {
            Rgb::new(8, 24, 32)
        } else {
            Rgb::new(52, 88, 104)
        };
        fill_rect(
            fb,
            x,
            y,
            sprites::PLEDIT_TITLE_BUTTON_W,
            sprites::PLEDIT_TITLE_BUTTON_W,
            face,
        );
        let glyph = Rgb::new(150, 224, 236);
        match button {
            TitleButton::Shade => {
                fill_rect(fb, x + 2, y + 3, 5, 1, glyph);
                fill_rect(fb, x + 2, y + 5, 5, 1, glyph);
            }
            TitleButton::Close => {
                for i in 2..7 {
                    fill_rect(fb, x + i, y + i, 1, 1, glyph);
                    fill_rect(fb, x + 8 - i, y + i, 1, 1, glyph);
                }
            }
        }
    }
}

fn draw_shade_track(
    fb: &mut Framebuffer,
    state: &PlState,
    colors: &xubamp_skin::pledit::PlEdit,
    width: i32,
) {
    let Some(row) = state.current.and_then(|i| state.rows.get(i)) else {
        return;
    };
    let c = [colors.normal.r, colors.normal.g, colors.normal.b];
    let duration_w = font::text_width(&row.duration) as i32;
    if !row.duration.is_empty() {
        font::draw_text(
            &mut fb.rgba,
            fb.width,
            fb.height,
            width - 30 - duration_w,
            4,
            &row.duration,
            c,
        );
    }
    let available = (width - 5 - 35 - duration_w).max(0) as u32;
    let max_chars = (available / font::ADVANCE.max(1)) as usize;
    let title: String = row.title.chars().take(max_chars).collect();
    font::draw_text(&mut fb.rgba, fb.width, fb.height, 5, 4, &title, c);
}

/// Draw the visible track rows over the list area of a window `window_h` px tall whose list content
/// is `list_w` px wide.
fn draw_rows(
    fb: &mut Framebuffer,
    state: &PlState,
    colors: &xubamp_skin::pledit::PlEdit,
    window_h: i32,
    list_w: i32,
) {
    let offset = state.scroll_offset(window_h);
    let visible = PlState::visible_rows(window_h);
    for (i, row) in state.rows.iter().enumerate().skip(offset).take(visible) {
        let screen_row = (i - offset) as i32;
        let y = sprites::PLEDIT_LIST_Y + screen_row * sprites::PLEDIT_ROW_H;
        if state.selected.contains(&i) {
            fill_rect(
                fb,
                sprites::PLEDIT_LIST_X,
                y - 2,
                list_w,
                sprites::PLEDIT_ROW_H,
                colors.selected_bg,
            );
        }
        let rgb = if state.current == Some(i) {
            colors.current
        } else {
            colors.normal
        };
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
        font::draw_text(
            &mut fb.rgba,
            fb.width,
            fb.height,
            sprites::PLEDIT_LIST_X + 1,
            y,
            &title,
            c,
        );
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
        Image {
            width: w,
            height: h,
            rgba: c
                .iter()
                .copied()
                .cycle()
                .take((w * h * 4) as usize)
                .collect(),
        }
    }
    fn px(fb: &Framebuffer, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * fb.width + x) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    #[test]
    fn compose_draws_the_frame_and_list_background() {
        let sheet = solid_sheet(300, 120, [200, 0, 0, 255]); // the whole frame source is red
        let colors = PlEdit {
            normal_bg: Rgb::new(10, 20, 30),
            ..PlEdit::default()
        };
        let skin = Skin {
            pledit: Some(sheet),
            pledit_colors: Some(colors),
            ..Default::default()
        };
        let fb = compose(
            &skin,
            &PlState::default(),
            sprites::PLEDIT_W,
            sprites::PLEDIT_H,
        );
        assert_eq!(fb.width, sprites::PLEDIT_W as u32);
        assert_eq!(fb.height, sprites::PLEDIT_H as u32);
        // A title-bar corner pixel comes from the (red) sheet: the frame was drawn.
        assert_eq!(
            px(&fb, 2, 2),
            [200, 0, 0, 255],
            "title-bar frame drawn from the sheet"
        );
        // A middle-band interior pixel is the NormalBG list fill.
        assert_eq!(
            px(&fb, 100, 40),
            [10, 20, 30, 255],
            "list background filled with NormalBG"
        );
        // A bottom-bar pixel comes from the sheet corners.
        assert_eq!(px(&fb, 2, 100), [200, 0, 0, 255], "bottom bar drawn");
        // Without a pledit sheet the renderer supplies an opaque clean-room fallback, so the
        // package's built-in skin never maps an invisible playlist toplevel.
        let fallback = compose(
            &Skin::default(),
            &PlState::default(),
            sprites::PLEDIT_W,
            sprites::PLEDIT_H,
        );
        assert!(
            fallback.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255),
            "no pledit.bmp: fallback frame is fully opaque"
        );
    }

    #[test]
    fn region_at_prioritizes_dynamic_title_buttons_and_mode_specific_resize() {
        let (w, h) = (sprites::PLEDIT_W + 50, sprites::PLEDIT_H + 29);
        let expanded = PlState::default();
        let shade_x = w - sprites::PLEDIT_SHADE_BUTTON_RIGHT - 9;
        let close_x = w - sprites::PLEDIT_CLOSE_BUTTON_RIGHT - 9;
        assert_eq!(
            region_at(&expanded, w, h, shade_x + 4, 7),
            Region::TitleButton(TitleButton::Shade)
        );
        assert_eq!(
            region_at(&expanded, w, h, close_x + 4, 7),
            Region::TitleButton(TitleButton::Close)
        );
        assert_eq!(region_at(&expanded, w, h, 40, 7), Region::TitleBar);
        assert_eq!(region_at(&expanded, w, h, 40, 40), Region::Body);
        for button in BottomButton::ALL {
            let (bx, by, bw, bh) = bottom_button_rect(button, w, h);
            assert_eq!(
                region_at(&expanded, w, h, bx + bw / 2, by + bh / 2),
                Region::BottomMenu(button),
                "the resized bottom bar keeps every cluster target, including right-anchored LIST"
            );
        }
        assert_eq!(region_at(&expanded, w, h, w - 1, h - 1), Region::Resize);

        // Dead chrome is the draggable frame: the side borders, the strip between the title bar
        // and the first row, and the bottom bar between its controls.
        assert_eq!(
            region_at(&expanded, w, h, 5, 40),
            Region::Frame,
            "left border"
        );
        assert_eq!(
            region_at(&expanded, w, h, w - 3, 40),
            Region::Frame,
            "right border, right of the scrollbar"
        );
        assert_eq!(
            region_at(&expanded, w, h, 40, sprites::PLEDIT_TITLE_H + 1),
            Region::Frame,
            "the 3px strip above the first row"
        );
        assert_eq!(
            region_at(&expanded, w, h, ADD_BUTTON_X + BOTTOM_BUTTON_W + 3, h - 4),
            Region::Frame,
            "bottom bar between the clusters"
        );

        let shaded = PlState {
            shade: true,
            ..Default::default()
        };
        let resize_x = w - sprites::PLEDIT_SHADE_RESIZE_RIGHT - 9;
        assert_eq!(
            region_at(&shaded, w, sprites::PLEDIT_SHADE_H, resize_x + 4, 7),
            Region::Resize,
            "shade has a width-only grip immediately left of its shade button"
        );
        assert_eq!(
            region_at(&shaded, w, sprites::PLEDIT_SHADE_H, shade_x + 4, 7),
            Region::TitleButton(TitleButton::Shade),
            "restore button keeps the dynamic right-edge target"
        );
        assert_eq!(
            region_at(&shaded, w, sprites::PLEDIT_SHADE_H, 40, 7),
            Region::TitleBar
        );
        assert_eq!(
            region_at(&shaded, w, sprites::PLEDIT_SHADE_H, 40, 14),
            Region::None,
            "nothing exists below the fully collapsed strip"
        );
        assert_eq!(
            region_at(&shaded, w, sprites::PLEDIT_SHADE_H, -1, 7),
            Region::None
        );
    }

    #[test]
    fn mini_time_draws_the_live_clock_in_the_current_color_and_has_a_click_target() {
        let colors = PlEdit {
            normal_bg: Rgb::new(0, 0, 0),
            current: Rgb::new(250, 250, 250),
            ..PlEdit::default()
        };
        let skin = Skin {
            pledit_colors: Some(colors),
            ..Default::default()
        };
        let (w, h) = (sprites::PLEDIT_W, sprites::PLEDIT_H);
        let (mx, my, mw, mh) = mini_time_rect(w, h);

        // With a clock value, some pixel of the "00:07" glyphs is the Current colour.
        let state = PlState {
            clock: Some(7),
            ..Default::default()
        };
        let fb = compose(&skin, &state, w, h);
        let lit = (my..my + mh).any(|y| {
            (mx..mx + mw).any(|x| {
                let o = ((y as u32 * fb.width + x as u32) * 4) as usize;
                fb.rgba[o..o + 3] == [250, 250, 250]
            })
        });
        assert!(lit, "mini clock glyphs drawn in the Current colour");

        // No clock (nothing loaded, or the blink's hidden beat): nothing lit there.
        let blank = compose(&skin, &PlState::default(), w, h);
        let lit = (my..my + mh).any(|y| {
            (mx..mx + mw).any(|x| {
                let o = ((y as u32 * blank.width + x as u32) * 4) as usize;
                blank.rgba[o..o + 3] == [250, 250, 250]
            })
        });
        assert!(!lit, "no clock draws nothing");

        // The slot is its own click target (it toggles the time mode in the platform layer).
        assert_eq!(
            region_at(&PlState::default(), w, h, mx + mw / 2, my + mh / 2),
            Region::MiniTime
        );
    }

    #[test]
    fn scrollbar_appears_only_on_overflow_and_maps_the_drag() {
        fn rows(n: usize) -> Vec<Row> {
            (0..n)
                .map(|i| Row {
                    title: format!("{i}"),
                    duration: String::new(),
                    duration_secs: None,
                })
                .collect()
        }
        let (w, h) = (sprites::PLEDIT_W, sprites::PLEDIT_H);

        // A list that fits shows no thumb.
        let small = PlState {
            rows: rows(2),
            ..Default::default()
        };
        assert!(scrollbar_thumb_rect(&small, w, h).is_none());

        // A long list gets a thumb shorter than the track, at the top for scroll 0.
        let mut big = PlState {
            rows: rows(200),
            ..Default::default()
        };
        let (tx, ty, _tw, th) = scrollbar_track(w, h);
        let (thumb_x, thumb_y, _, thumb_h) = scrollbar_thumb_rect(&big, w, h).unwrap();
        assert_eq!(thumb_x, tx);
        assert_eq!(thumb_y, ty, "scroll 0 puts the thumb at the top");
        assert!(thumb_h < th, "the thumb is shorter than the track");
        assert_eq!(region_at(&big, w, h, tx + 1, ty + 5), Region::Scrollbar);

        // Dragging to the track bottom scrolls to the end and moves the thumb down.
        big.set_scroll_from_y(ty + th, w, h);
        assert!(big.scroll > 99.0);
        let (_, moved_y, _, _) = scrollbar_thumb_rect(&big, w, h).unwrap();
        assert!(moved_y > thumb_y, "the thumb tracks the scroll");

        // A press back at the top returns to scroll 0.
        big.set_scroll_from_y(ty, w, h);
        assert_eq!(big.scroll, 0.0);
    }

    #[test]
    fn selection_ops_cover_all_none_and_invert() {
        let mut state = PlState {
            rows: rows(4),
            ..Default::default()
        };
        assert!(state.select_all());
        assert_eq!(state.selected, [0, 1, 2, 3]);
        assert!(!state.select_all(), "already all-selected is a no-op");
        assert!(state.invert_selection());
        assert!(state.selected.is_empty(), "inverting a full selection clears it");
        assert!(state.invert_selection());
        assert_eq!(state.selected, [0, 1, 2, 3], "inverting nothing selects everything");
        assert!(state.select_none());
        assert!(state.selected.is_empty());
        assert!(!state.select_none(), "already empty is a no-op");

        // Invert a partial selection.
        state.selected = vec![1, 3];
        assert!(state.invert_selection());
        assert_eq!(state.selected, [0, 2]);

        // An empty playlist has nothing to invert.
        let mut empty = PlState::default();
        assert!(!empty.invert_selection());
    }

    #[test]
    fn running_time_shows_selected_over_total() {
        let mut state = PlState {
            rows: vec![
                Row {
                    title: "a".into(),
                    duration: String::new(),
                    duration_secs: Some(90),
                },
                Row {
                    title: "b".into(),
                    duration: String::new(),
                    duration_secs: Some(30),
                },
                Row {
                    title: "c".into(),
                    duration: String::new(),
                    duration_secs: None,
                },
            ],
            ..Default::default()
        };
        assert_eq!(running_time_message(&state), "0:00/2:00", "unknown counts as zero");
        state.selected = vec![0];
        assert_eq!(running_time_message(&state), "1:30/2:00");
    }

    #[test]
    fn held_add_control_uses_the_selected_skin_cell() {
        let mut sheet = solid_sheet(300, 170, [20, 30, 40, 255]);
        for y in 149..167 {
            for x in 23..45 {
                let offset = ((y * sheet.width as i32 + x) * 4) as usize;
                sheet.rgba[offset..offset + 4].copy_from_slice(&[220, 40, 90, 255]);
            }
        }
        let state = PlState {
            pressed_menu: Some(BottomButton::Add),
            ..Default::default()
        };
        let fb = compose(
            &Skin {
                pledit: Some(sheet),
                ..Default::default()
            },
            &state,
            sprites::PLEDIT_W,
            sprites::PLEDIT_H,
        );
        let (x, y, w, h) = bottom_button_rect(BottomButton::Add, sprites::PLEDIT_W, sprites::PLEDIT_H);
        assert_eq!(
            px(&fb, (x + w / 2) as u32, (y + h / 2) as u32),
            [220, 40, 90, 255]
        );
    }

    #[test]
    fn compose_shade_tiles_to_width_and_draws_the_restore_pressed_cell() {
        let mut sheet = solid_sheet(300, 170, [200, 0, 0, 255]);
        let paint = |sheet: &mut Image, rect: Rect, color: [u8; 4]| {
            for y in rect.y..rect.y + rect.h {
                for x in rect.x..rect.x + rect.w {
                    let o = ((y as u32 * sheet.width + x as u32) * 4) as usize;
                    sheet.rgba[o..o + 4].copy_from_slice(&color);
                }
            }
        };
        paint(&mut sheet, sprites::PLEDIT_SHADE_TILE, [0, 0, 200, 255]);
        paint(&mut sheet, sprites::PLEDIT_SHADE_LEFT, [0, 200, 0, 255]);
        paint(&mut sheet, sprites::PLEDIT_SHADE_RIGHT, [200, 200, 0, 255]);
        paint(
            &mut sheet,
            sprites::PLEDIT_EXPAND_PRESSED,
            [255, 255, 255, 255],
        );
        paint(
            &mut sheet,
            sprites::PLEDIT_COLLAPSE_PRESSED,
            [255, 0, 255, 255],
        );
        let skin = Skin {
            pledit: Some(sheet),
            ..Default::default()
        };
        let w = sprites::PLEDIT_W + 50;
        let state = PlState {
            shade: true,
            pressed_title: Some(TitleButton::Shade),
            ..Default::default()
        };
        let fb = compose(&skin, &state, w, 999);
        assert_eq!(
            (fb.width, fb.height),
            (w as u32, sprites::PLEDIT_SHADE_H as u32),
            "shade ignores the old expanded height"
        );
        assert_eq!(px(&fb, 2, 2), [0, 200, 0, 255], "left cap");
        assert_eq!(px(&fb, 100, 2), [0, 0, 200, 255], "middle tile");
        assert_eq!(
            px(&fb, (w - 2) as u32, 2),
            [200, 200, 0, 255],
            "focused right cap"
        );
        let button_x = w - sprites::PLEDIT_SHADE_BUTTON_RIGHT - 9;
        assert_eq!(
            px(&fb, (button_x + 4) as u32, 7),
            [255, 255, 255, 255],
            "held shade button uses the restore cell"
        );

        let expanded = PlState {
            pressed_title: Some(TitleButton::Shade),
            ..Default::default()
        };
        let fb = compose(&skin, &expanded, w, sprites::PLEDIT_H);
        assert_eq!(fb.height, sprites::PLEDIT_H as u32);
        assert_eq!(
            px(&fb, (button_x + 4) as u32, 7),
            [255, 0, 255, 255],
            "expanded middle button uses the collapse cell"
        );
    }

    #[test]
    fn compose_shade_without_pledit_is_opaque_and_shows_current_track() {
        let state = PlState {
            shade: true,
            rows: vec![Row {
                title: "1. current song".into(),
                duration: "3:21".into(),
                duration_secs: Some(201),
            }],
            current: Some(0),
            ..Default::default()
        };
        let fb = compose(
            &Skin::default(),
            &state,
            sprites::PLEDIT_W,
            sprites::PLEDIT_H,
        );
        assert_eq!(fb.height, sprites::PLEDIT_SHADE_H as u32);
        assert!(fb.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert!(
            fb.rgba
                .chunks_exact(4)
                .any(|pixel| pixel[..3] == [0, 255, 0]),
            "the default playlist text colour draws current-track text"
        );
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
        let colors = PlEdit {
            normal_bg: Rgb::new(10, 20, 30),
            ..PlEdit::default()
        };
        let skin = Skin {
            pledit: Some(sheet),
            pledit_colors: Some(colors),
            ..Default::default()
        };

        // One segment wider and taller than the default.
        let (w, h) = (
            sprites::PLEDIT_W + sprites::PLEDIT_SEGMENT_W,
            sprites::PLEDIT_H + sprites::PLEDIT_SEGMENT_H,
        );
        let fb = compose(&skin, &PlState::default(), w, h);
        assert_eq!(
            (fb.width, fb.height),
            (w as u32, h as u32),
            "frame sized to the request"
        );
        // The gap the corners used to leave (just past the 125px left corner, in the bottom band) is
        // now painted by the bottom fill tile.
        let bottom_y = (h - sprites::PLEDIT_BOTTOM_H + 5) as u32;
        assert_eq!(
            px(&fb, 130, bottom_y),
            [0, 0, 200, 255],
            "widened bottom bar filled by the tile"
        );
        // The list background still fills to the new right edge.
        assert_eq!(
            px(&fb, (w - 25) as u32, 40),
            [10, 20, 30, 255],
            "list background stretched to the wider width"
        );
    }

    #[test]
    fn visible_rows_and_scroll_offset_track_the_size() {
        // Default height: 4 rows. A taller window shows more.
        assert_eq!(
            PlState::visible_rows(sprites::PLEDIT_H),
            4,
            "(116-20-38)/13 = 4 rows fit"
        );
        assert_eq!(
            PlState::visible_rows(sprites::PLEDIT_H + sprites::PLEDIT_SEGMENT_H),
            6,
            "one taller segment (+29px) fits 2 more rows",
        );
        let mut s = PlState {
            rows: vec![Row::default(); 10],
            ..Default::default()
        };
        assert_eq!(s.scroll_offset(sprites::PLEDIT_H), 0, "scroll 0 -> top");
        s.scroll = 100.0;
        assert_eq!(
            s.scroll_offset(sprites::PLEDIT_H),
            6,
            "scroll 100 -> overflow (10-4)"
        );
        s.scroll = 50.0;
        assert_eq!(
            s.scroll_offset(sprites::PLEDIT_H),
            3,
            "scroll 50 -> round(0.5*6)"
        );
    }

    fn rows(n: usize) -> Vec<Row> {
        (0..n)
            .map(|i| Row {
                title: format!("track {i}"),
                duration: String::new(),
                duration_secs: None,
            })
            .collect()
    }

    #[test]
    fn row_at_maps_clicks_to_indices_with_scroll() {
        let h = sprites::PLEDIT_H; // 4 visible rows
        let mut s = PlState {
            rows: rows(20),
            ..Default::default()
        };
        // Above the list, or left of it: no row.
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y - 1, h), None);
        assert_eq!(
            s.row_at(sprites::PLEDIT_LIST_X - 1, sprites::PLEDIT_LIST_Y, h),
            None
        );
        // The four visible rows, top to bottom.
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y, h), Some(0));
        assert_eq!(
            s.row_at(50, sprites::PLEDIT_LIST_Y + sprites::PLEDIT_ROW_H, h),
            Some(1)
        );
        // Past the last visible row: none.
        assert_eq!(
            s.row_at(50, sprites::PLEDIT_LIST_Y + 4 * sprites::PLEDIT_ROW_H, h),
            None
        );
        // With scroll to the bottom, the top visible row shifts to the overflow (20-4 = 16).
        s.scroll = 100.0;
        assert_eq!(s.row_at(50, sprites::PLEDIT_LIST_Y, h), Some(16));
        assert_eq!(
            s.row_at(50, sprites::PLEDIT_LIST_Y + 3 * sprites::PLEDIT_ROW_H, h),
            Some(19)
        );
    }

    #[test]
    fn row_at_returns_none_past_the_last_track() {
        let h = sprites::PLEDIT_H;
        let s = PlState {
            rows: rows(2),
            ..Default::default()
        }; // 2 tracks in 4 visible slots
        assert_eq!(
            s.row_at(50, sprites::PLEDIT_LIST_Y + sprites::PLEDIT_ROW_H, h),
            Some(1)
        );
        assert_eq!(
            s.row_at(50, sprites::PLEDIT_LIST_Y + 2 * sprites::PLEDIT_ROW_H, h),
            None
        );
    }

    #[test]
    fn selection_click_ctrl_shift_match_winamp() {
        // Plain click selects only that row and sets the anchor.
        let mut s = PlState {
            rows: rows(10),
            ..Default::default()
        };
        s.click_select(3);
        assert_eq!(s.selected, vec![3]);
        assert_eq!(s.anchor, Some(3));
        // Plain click on an already-selected row is a no-op (a multi-selection survives for a drag).
        s.selected = vec![3, 4, 5];
        s.click_select(4);
        assert_eq!(s.selected, vec![3, 4, 5]);

        // Ctrl+click toggles a single row and moves the anchor, even when un-selecting.
        let mut s = PlState {
            rows: rows(10),
            ..Default::default()
        };
        s.click_select(2);
        s.ctrl_select(5);
        assert_eq!(s.selected, vec![2, 5]);
        assert_eq!(s.anchor, Some(5));
        s.ctrl_select(2);
        assert_eq!(s.selected, vec![5]);
        assert_eq!(s.anchor, Some(2), "anchor moves even on un-select");

        // Shift+click replaces the selection with the range from the anchor; the anchor stays put.
        let mut s = PlState {
            rows: rows(10),
            ..Default::default()
        };
        s.click_select(2);
        s.shift_select(5);
        assert_eq!(s.selected, vec![2, 3, 4, 5]);
        assert_eq!(s.anchor, Some(2));
        s.shift_select(0);
        assert_eq!(
            s.selected,
            vec![0, 1, 2],
            "successive shift-clicks pivot around the same anchor"
        );

        // Shift with no anchor is inert.
        let mut s = PlState {
            rows: rows(10),
            ..Default::default()
        };
        s.shift_select(4);
        assert!(s.selected.is_empty());
    }

    #[test]
    fn clearing_selection_leaves_the_anchor() {
        let mut s = PlState {
            rows: rows(5),
            ..Default::default()
        };
        s.click_select(2);
        s.clear_selection();
        assert!(s.selected.is_empty());
        assert_eq!(s.anchor, Some(2));
    }

    #[test]
    fn wheel_scroll_moves_within_bounds() {
        let h = sprites::PLEDIT_H; // 4 visible
        let mut s = PlState {
            rows: rows(14),
            ..Default::default()
        }; // overflow 10
        s.scroll_by_tracks(2.0, h); // +2/10 = 20%
        assert!((s.scroll - 20.0).abs() < 0.01);
        s.scroll_by_tracks(-100.0, h);
        assert_eq!(s.scroll, 0.0, "clamps at the top");
        s.scroll_by_tracks(1000.0, h);
        assert_eq!(s.scroll, 100.0, "clamps at the bottom");
        // No overflow: inert.
        let mut s = PlState {
            rows: rows(2),
            ..Default::default()
        };
        s.scroll_by_tracks(5.0, h);
        assert_eq!(s.scroll, 0.0);
    }

    #[test]
    fn scroll_to_brings_a_row_into_view() {
        let h = sprites::PLEDIT_H; // 4 visible, overflow 16 for 20 rows
        let mut s = PlState {
            rows: rows(20),
            ..Default::default()
        };
        s.scroll_to(19, h);
        assert_eq!(
            s.scroll_offset(h),
            16,
            "last row lands at the bottom of the view"
        );
        s.scroll_to(0, h);
        assert_eq!(s.scroll_offset(h), 0);
        // An already-visible row does not move the scroll.
        s.scroll = 25.0;
        let before = s.scroll;
        s.scroll_to(s.scroll_offset(h), h);
        assert_eq!(s.scroll, before);
    }
}
