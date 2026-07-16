//! The classic "file info box": stream facts plus an editable ID3 tag form for one track.
//!
//! Winamp 2.x drew this with native widgets, so like the Jump and Preferences windows it is
//! rendered in the Adwaita style rather than from the skin. The dialog model here is pure and
//! unit-tested: field focus, single-line text editing, hit-testing, and the save payload. The
//! platform layer owns the toplevel window, feeds the data in, and carries out the save.

use std::path::PathBuf;

use crate::adwaita::{self, Palette, UiFont};
use crate::Framebuffer;
use xubamp_skin::font;

/// Default (and fixed) dialog size. The height clears the form (7 rows below the facts block)
/// plus the button bar.
pub const FILEINFO_W: i32 = 440;
pub const FILEINFO_H: i32 = 424;

/// Title-bar band height; a press there drags the window.
pub const FILEINFO_TITLE_H: i32 = 34;

const PAD: i32 = 14;
const ROW_H: i32 = 30;
const LABEL_W: i32 = 86;
const BUTTON_W: i32 = 84;
const BUTTON_H: i32 = 30;
const FACT_ROW_H: i32 = 17;

/// An axis-aligned rectangle: (x, y, w, h).
type Rect = (i32, i32, i32, i32);

/// The editable tag fields, in their on-screen order.
pub const FIELD_LABELS: [&str; 7] = [
    "Title", "Artist", "Album", "Year", "Comment", "Genre", "Track #",
];

/// Stream facts shown in the read-only block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackFacts {
    /// Full path, shown at the top.
    pub path: String,
    pub size_bytes: u64,
    pub duration_secs: Option<u32>,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u8>,
    /// Codec long name ("MP3 (MPEG audio layer 3)", "Free Lossless Audio Codec", ...).
    pub codec: String,
}

impl TrackFacts {
    /// The read-only lines of the facts block, in display order.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!("Size: {} bytes", self.size_bytes)];
        if let Some(secs) = self.duration_secs {
            lines.push(format!("Length: {}:{:02} ({} seconds)", secs / 60, secs % 60, secs));
        }
        if let Some(kbps) = self.bitrate_kbps {
            lines.push(format!("Average bitrate: {kbps} kbit/s"));
        }
        if let Some(hz) = self.sample_rate_hz {
            let channels = match self.channels {
                Some(1) => " mono",
                Some(2) => " stereo",
                _ => "",
            };
            lines.push(format!("{hz} Hz{channels}"));
        }
        if !self.codec.is_empty() {
            lines.push(self.codec.clone());
        }
        lines
    }
}

/// Everything the platform layer feeds the dialog when it opens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInfoData {
    /// The described track, carried through to the save request.
    pub path: PathBuf,
    pub facts: TrackFacts,
    /// Initial values for [`FIELD_LABELS`], in order.
    pub fields: [String; 7],
    /// Whether the Save button is live (an MP3, whose ID3v1 tail we can write).
    pub editable: bool,
}

/// The tag values to write on Save, in [`FIELD_LABELS`] order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveRequest {
    pub path: PathBuf,
    pub fields: [String; 7],
}

/// What a pointer press or key did, for the platform layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcome {
    pub redraw: bool,
    /// Save was activated: write the tag, then show the result via [`FileInfoState::set_status`].
    pub save: bool,
    pub close: bool,
    /// Start an interactive window move (a title-bar band press).
    pub start_move: bool,
}

/// The dialog's pure interaction state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInfoState {
    pub data: FileInfoData,
    /// Focused field index (into [`FIELD_LABELS`]), or `None` when nothing has focus.
    pub focus: Option<usize>,
    /// Cursor position (in chars) within the focused field.
    pub cursor: usize,
    /// One-line result message ("Saved." / an error) under the buttons.
    pub status: Option<String>,
}

/// A single key for the dialog, decoded by the platform layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Tab,
    ShiftTab,
    Enter,
    Escape,
}

impl FileInfoState {
    pub fn new(data: FileInfoData) -> Self {
        let focus = data.editable.then_some(0);
        Self {
            data,
            focus,
            cursor: 0,
            status: None,
        }
    }

    /// The save payload: the current field values for the described track.
    pub fn save_request(&self) -> SaveRequest {
        SaveRequest {
            path: self.data.path.clone(),
            fields: self.data.fields.clone(),
        }
    }

    /// Show the save result line.
    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = Some(status.into());
    }

    /// Digest a save attempt: success dismisses the box (like Winamp's info box, whose save
    /// button is also its close), failure keeps it open with the error on the status line.
    /// Returns whether the caller should close the window.
    pub fn save_result(&mut self, result: Result<(), String>) -> bool {
        match result {
            Ok(()) => true,
            Err(error) => {
                self.set_status(error);
                false
            }
        }
    }

    fn focus_field(&mut self, index: usize) {
        self.focus = Some(index);
        self.cursor = self.data.fields[index].chars().count();
    }

    fn focus_step(&mut self, forward: bool) {
        if !self.data.editable {
            return;
        }
        let n = FIELD_LABELS.len();
        let next = match (self.focus, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        self.focus_field(next);
    }

    /// Handle a decoded key. Editing applies to the focused field.
    pub fn key(&mut self, key: Key) -> Outcome {
        let mut out = Outcome {
            redraw: true,
            ..Default::default()
        };
        match key {
            Key::Escape => {
                out.close = true;
                out.redraw = false;
                return out;
            }
            Key::Enter => {
                if self.data.editable {
                    out.save = true;
                }
                return out;
            }
            Key::Tab => {
                self.focus_step(true);
                return out;
            }
            Key::ShiftTab => {
                self.focus_step(false);
                return out;
            }
            _ => {}
        }
        let Some(focus) = self.focus.filter(|_| self.data.editable) else {
            out.redraw = false;
            return out;
        };
        let field = &mut self.data.fields[focus];
        let chars: Vec<char> = field.chars().collect();
        let cursor = self.cursor.min(chars.len());
        match key {
            Key::Char(c) if !c.is_control() => {
                let mut next: String = chars[..cursor].iter().collect();
                next.push(c);
                next.extend(&chars[cursor..]);
                *field = next;
                self.cursor = cursor + 1;
            }
            Key::Backspace if cursor > 0 => {
                let mut next: String = chars[..cursor - 1].iter().collect();
                next.extend(&chars[cursor..]);
                *field = next;
                self.cursor = cursor - 1;
            }
            Key::Delete if cursor < chars.len() => {
                let mut next: String = chars[..cursor].iter().collect();
                next.extend(&chars[cursor + 1..]);
                *field = next;
            }
            Key::Left => self.cursor = cursor.saturating_sub(1),
            Key::Right => self.cursor = (cursor + 1).min(chars.len()),
            Key::Home => self.cursor = 0,
            Key::End => self.cursor = chars.len(),
            _ => out.redraw = false,
        }
        out
    }

    /// Handle a left press at window-local (`x`, `y`).
    pub fn press(&mut self, x: i32, y: i32, width: i32, height: i32) -> Outcome {
        let mut out = Outcome::default();
        if let Some(index) = field_index_at(x, y, width, height) {
            if self.data.editable {
                self.focus_field(index);
                out.redraw = true;
            }
            return out;
        }
        let (save_rect, close_rect) = button_rects(width, height);
        if in_rect(x, y, save_rect) {
            if self.data.editable {
                out.save = true;
                out.redraw = true;
            }
            return out;
        }
        if in_rect(x, y, close_rect) {
            out.close = true;
            return out;
        }
        if y < FILEINFO_TITLE_H {
            out.start_move = true;
        }
        out
    }
}

/// The y of the top of the tag-field rows.
fn fields_top(_height: i32) -> i32 {
    // Title bar, path line, facts block, then the form.
    FILEINFO_TITLE_H + 24 + 5 * FACT_ROW_H + 14
}

/// The field row rect (the editable box, not the label) for `index`.
fn field_rect(index: usize, width: i32, _height: i32) -> Rect {
    let y = fields_top(0) + index as i32 * ROW_H;
    (
        PAD + LABEL_W,
        y,
        width - PAD * 2 - LABEL_W,
        ROW_H - 6,
    )
}

/// Which field's editable box is at a point.
fn field_index_at(x: i32, y: i32, width: i32, height: i32) -> Option<usize> {
    (0..FIELD_LABELS.len()).find(|&i| in_rect(x, y, field_rect(i, width, height)))
}

/// (Save, Close) button rects, bottom-right.
fn button_rects(width: i32, height: i32) -> (Rect, Rect) {
    let y = height - PAD - BUTTON_H;
    let close = (width - PAD - BUTTON_W, y, BUTTON_W, BUTTON_H);
    let save = (width - PAD * 2 - BUTTON_W * 2, y, BUTTON_W, BUTTON_H);
    (save, close)
}

fn in_rect(x: i32, y: i32, (rx, ry, rw, rh): Rect) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

/// Elide `text` from the front so its tail fits `max` px (paths keep their filename visible).
fn elide_front(font: Option<&UiFont>, text: &str, px: f32, max: f32) -> String {
    let width = |s: &str| match font {
        Some(f) => f.text_width(s, px),
        None => font::text_width(s) as f32,
    };
    if width(text) <= max {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 1..chars.len() {
        let candidate: String = std::iter::once('…').chain(chars[start..].iter().copied()).collect();
        if width(&candidate) <= max {
            return candidate;
        }
    }
    "…".to_owned()
}

/// Compose the dialog. `ui_font` gives native text; without one the built-in 5x7 bitmap font
/// keeps the dialog usable.
pub fn compose(
    state: &FileInfoState,
    ui_font: Option<&UiFont>,
    palette: &Palette,
    width: i32,
    height: i32,
) -> Framebuffer {
    let mut fb = Framebuffer::new(width as u32, height as u32);
    adwaita::fill_rounded_rect(
        &mut fb,
        0,
        0,
        width,
        height,
        adwaita::WINDOW_RADIUS,
        palette.window_bg,
    );
    let text = |fb: &mut Framebuffer, x: i32, y: i32, s: &str, px: f32, color: [u8; 4]| {
        match ui_font {
            Some(f) => f.draw_text(fb, x, y + px as i32, s, px, color),
            None => font::draw_text(
                &mut fb.rgba,
                fb.width,
                fb.height,
                x,
                y + 2,
                s,
                [color[0], color[1], color[2]],
            ),
        }
    };

    // Title bar.
    text(
        &mut fb,
        PAD,
        (FILEINFO_TITLE_H - 20) / 2,
        "File Info",
        14.0,
        palette.fg,
    );
    adwaita::draw_separator(&mut fb, 0, FILEINFO_TITLE_H - 1, width, palette);

    // Path line (front-elided so the filename stays visible).
    let path = elide_front(
        ui_font,
        &state.data.facts.path,
        12.0,
        (width - PAD * 2) as f32,
    );
    text(&mut fb, PAD, FILEINFO_TITLE_H + 4, &path, 12.0, palette.dim_fg);

    // Facts block.
    let facts_top = FILEINFO_TITLE_H + 24;
    for (i, line) in state.data.facts.lines().iter().take(5).enumerate() {
        text(
            &mut fb,
            PAD,
            facts_top + i as i32 * FACT_ROW_H,
            line,
            12.0,
            palette.fg,
        );
    }

    // Tag form.
    for (i, label) in FIELD_LABELS.iter().enumerate() {
        let (fx, fy, fw, fh) = field_rect(i, width, height);
        text(&mut fb, PAD, fy + 5, label, 12.0, palette.dim_fg);
        adwaita::fill_rounded_rect(&mut fb, fx, fy, fw, fh, 6, palette.view_bg);
        adwaita::stroke_rounded_rect(&mut fb, fx, fy, fw, fh, 6, 1, palette.border);
        if state.focus == Some(i) && state.data.editable {
            adwaita::draw_focus_ring(&mut fb, fx, fy, fw, fh, 6, palette);
        }
        let value = &state.data.fields[i];
        let color = if state.data.editable {
            palette.fg
        } else {
            palette.dim_fg
        };
        text(&mut fb, fx + 6, fy + 4, value, 12.0, color);
        // A simple cursor bar after the glyphs left of it.
        if state.focus == Some(i) && state.data.editable {
            let before: String = value.chars().take(state.cursor).collect();
            let cursor_x = fx
                + 6
                + match ui_font {
                    Some(f) => f.text_width(&before, 12.0).round() as i32,
                    None => font::text_width(&before) as i32,
                };
            adwaita::fill_rect(&mut fb, cursor_x, fy + 4, 1, fh - 8, palette.accent_bg);
        }
    }

    // Buttons.
    let (save, close) = button_rects(width, height);
    let button = |fb: &mut Framebuffer, rect: Rect, label: &str, live: bool| {
        let (bx, by, bw, bh) = rect;
        let bg = if live { palette.accent_bg } else { palette.hover };
        let fg = if live { palette.accent_fg } else { palette.dim_fg };
        adwaita::fill_rounded_rect(fb, bx, by, bw, bh, 8, bg);
        let label_w = match ui_font {
            Some(f) => f.text_width(label, 13.0).round() as i32,
            None => font::text_width(label) as i32,
        };
        text(fb, bx + (bw - label_w) / 2, by + (bh - 16) / 2, label, 13.0, fg);
    };
    button(&mut fb, save, "Save", state.data.editable);
    button(&mut fb, close, "Close", true);

    // Status line (save result), left of the buttons.
    if let Some(status) = &state.status {
        text(
            &mut fb,
            PAD,
            height - PAD - BUTTON_H + 6,
            status,
            12.0,
            palette.dim_fg,
        );
    }
    fb
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> FileInfoState {
        FileInfoState::new(FileInfoData {
            path: PathBuf::from("/music/xtal.mp3"),
            facts: TrackFacts {
                path: "/music/xtal.mp3".into(),
                size_bytes: 3_045_043,
                duration_secs: Some(190),
                bitrate_kbps: Some(128),
                sample_rate_hz: Some(44_100),
                channels: Some(2),
                codec: "MP3".into(),
            },
            fields: [
                "Xtal".into(),
                "Aphex Twin".into(),
                String::new(),
                "1992".into(),
                String::new(),
                "Ambient".into(),
                "1".into(),
            ],
            editable: true,
        })
    }

    #[test]
    fn facts_lines_include_size_length_and_rate() {
        let lines = state().data.facts.lines();
        assert_eq!(lines[0], "Size: 3045043 bytes");
        assert_eq!(lines[1], "Length: 3:10 (190 seconds)");
        assert_eq!(lines[2], "Average bitrate: 128 kbit/s");
        assert_eq!(lines[3], "44100 Hz stereo");
        assert_eq!(lines[4], "MP3");
    }

    #[test]
    fn save_result_closes_on_success_and_shows_the_error_otherwise() {
        let mut s = state();
        assert!(
            s.save_result(Ok(())),
            "a successful save dismisses the box"
        );
        assert_eq!(s.status, None, "no status line needed on the way out");
        assert!(
            !s.save_result(Err("Cannot write the tag: read-only".to_owned())),
            "a failed save keeps the box open"
        );
        assert_eq!(
            s.status.as_deref(),
            Some("Cannot write the tag: read-only"),
            "the failure lands on the status line"
        );
    }

    #[test]
    fn typing_edits_the_focused_field_at_the_cursor() {
        let mut s = state();
        assert_eq!(s.focus, Some(0), "editable dialog focuses the title");
        s.cursor = s.data.fields[0].chars().count();
        s.key(Key::Char('!'));
        assert_eq!(s.data.fields[0], "Xtal!");
        s.key(Key::Backspace);
        s.key(Key::Backspace);
        assert_eq!(s.data.fields[0], "Xta");
        s.key(Key::Home);
        s.key(Key::Char('>'));
        assert_eq!(s.data.fields[0], ">Xta");
        s.key(Key::Delete);
        assert_eq!(s.data.fields[0], ">ta", "delete removes after the cursor");
        // Multibyte safety.
        s.key(Key::Char('ö'));
        assert_eq!(s.data.fields[0], ">öta");
    }

    #[test]
    fn tab_cycles_fields_and_enter_saves() {
        let mut s = state();
        s.key(Key::Tab);
        assert_eq!(s.focus, Some(1));
        s.key(Key::ShiftTab);
        s.key(Key::ShiftTab);
        assert_eq!(s.focus, Some(FIELD_LABELS.len() - 1), "wraps backward");
        let out = s.key(Key::Enter);
        assert!(out.save);
        assert_eq!(
            s.save_request().fields[1],
            "Aphex Twin",
            "save carries the current values"
        );
        let out = s.key(Key::Escape);
        assert!(out.close);
    }

    #[test]
    fn read_only_dialogs_ignore_editing_and_save() {
        let mut s = state();
        s.data.editable = false;
        s.focus = None;
        assert!(!s.key(Key::Char('x')).redraw);
        assert!(!s.key(Key::Enter).save, "no ID3v1 target, no save");
        let (save, _) = button_rects(FILEINFO_W, FILEINFO_H);
        let out = s.press(save.0 + 2, save.1 + 2, FILEINFO_W, FILEINFO_H);
        assert!(!out.save, "the dead Save button does nothing");
    }

    #[test]
    fn presses_focus_fields_hit_buttons_and_drag_the_title() {
        let mut s = state();
        let (fx, fy, _, _) = field_rect(2, FILEINFO_W, FILEINFO_H);
        let out = s.press(fx + 3, fy + 3, FILEINFO_W, FILEINFO_H);
        assert!(out.redraw);
        assert_eq!(s.focus, Some(2));
        let (_, close) = button_rects(FILEINFO_W, FILEINFO_H);
        let out = s.press(close.0 + 1, close.1 + 1, FILEINFO_W, FILEINFO_H);
        assert!(out.close);
        let out = s.press(100, 5, FILEINFO_W, FILEINFO_H);
        assert!(out.start_move, "title band drags");
        let out = s.press(5, fields_top(0) + 2, FILEINFO_W, FILEINFO_H);
        assert!(
            !out.start_move && !out.close && !out.save,
            "label column is inert"
        );
    }

    #[test]
    fn compose_renders_without_a_ui_font_and_marks_focus() {
        let s = state();
        let fb = compose(
            &s,
            None,
            &Palette::dark(),
            FILEINFO_W,
            FILEINFO_H,
        );
        assert_eq!(fb.width, FILEINFO_W as u32);
        assert_eq!(fb.height, FILEINFO_H as u32);
        // The focused field carries the focus ring color somewhere on its border.
        let palette = Palette::dark();
        let (fx, fy, fw, fh) = field_rect(0, FILEINFO_W, FILEINFO_H);
        let mut found = false;
        for y in fy - 3..fy + fh + 3 {
            for x in fx - 3..fx + fw + 3 {
                let o = ((y as u32 * fb.width + x as u32) * 4) as usize;
                if fb.rgba[o..o + 4] == palette.focus_ring {
                    found = true;
                }
            }
        }
        assert!(found, "focused field draws its focus ring");
    }
}
