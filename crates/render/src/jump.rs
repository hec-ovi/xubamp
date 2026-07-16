//! The "Jump to file" dialog (classic Winamp `J`): a standalone window with a search field and a
//! filtered list of the matching tracks that you pick to play. Unlike an in-playlist search it does
//! not touch the playlist's own selection or scroll. Winamp draws it with native OS widgets rather
//! than the skin, so we render our own neutral chrome. Pure (returns a `Framebuffer`); the filtering
//! and list navigation are unit-tested.

use xubamp_skin::font;

use crate::adwaita::{self, Palette, UiFont};
use crate::pledit::Row;
use crate::Framebuffer;

/// Default dialog size.
pub const JUMP_W: i32 = 340;
pub const JUMP_H: i32 = 320;

const TITLE_H: i32 = 18;
/// Title-bar band height, exported so the window layer can treat a press there as a drag.
pub const JUMP_TITLE_H: i32 = TITLE_H;
const SEARCH_H: i32 = 16;
const BUTTON_H: i32 = 22;
const ROW_H: i32 = 12;
/// Y at which the results list begins.
const LIST_TOP: i32 = TITLE_H + SEARCH_H + 3;
const PAD: i32 = 6;
const BTN_W: i32 = 64;

// Neutral colors (the dialog is not skinned; the user asked to ignore colors).
const BG: [u8; 3] = [24, 24, 28];
const FG: [u8; 3] = [222, 222, 222];
const DIM: [u8; 3] = [140, 140, 148];
const SEL_BG: [u8; 3] = [40, 90, 180];
const BAR: [u8; 3] = [48, 48, 56];
const FIELD_BG: [u8; 3] = [12, 12, 14];

/// A bottom-bar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpButton {
    /// Play the selected match and close.
    Jump,
    /// Close without changing playback.
    Close,
}

/// Dialog state: the search query plus the full track list to filter. `selected`/`scroll` index into
/// the *filtered* matches (not the full list).
#[derive(Debug, Clone, Default)]
pub struct JumpState {
    pub query: String,
    /// The full playlist, kept in sync so the filter always reflects the current tracks. A row's
    /// position here is its original playlist index (what [`Self::selected_track`] returns).
    pub rows: Vec<Row>,
    /// Selected position within the current matches.
    pub selected: usize,
    /// First visible match position (list scroll).
    pub scroll: usize,
}

impl JumpState {
    /// Original playlist indices of the rows matching the query: every whitespace-separated token
    /// must appear in the shown title OR any of the track's metadata (artist, album, composer,
    /// genre, year, comment, file name, ...), case-insensitive. An empty query matches everything.
    pub fn matches(&self) -> Vec<usize> {
        let tokens: Vec<String> = self.query.split_whitespace().map(str::to_lowercase).collect();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                let title = r.title.to_lowercase();
                tokens
                    .iter()
                    .all(|t| title.contains(t) || r.search.contains(t.as_str()))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// How many result rows fit in a dialog `window_h` px tall.
    pub fn visible_rows(window_h: i32) -> usize {
        ((window_h - LIST_TOP - BUTTON_H) / ROW_H).max(0) as usize
    }

    /// The original playlist index of the currently-selected match, if any.
    pub fn selected_track(&self) -> Option<usize> {
        self.matches().get(self.selected).copied()
    }

    /// Replace the query (a keystroke edited it), resetting the selection to the first match.
    pub fn set_query(&mut self, query: String, window_h: i32) {
        self.query = query;
        self.selected = 0;
        self.scroll = 0;
        self.clamp_and_scroll(window_h);
    }

    /// Move the selection by `delta` rows (arrow keys), keeping it in range and in view.
    pub fn move_selection(&mut self, delta: i32, window_h: i32) {
        let n = self.matches().len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as i32 + delta).clamp(0, n as i32 - 1) as usize;
        self.clamp_and_scroll(window_h);
    }

    /// The match position at window-local (`x`, `y`) (a click), or `None` outside the list rows.
    pub fn row_at(&self, x: i32, y: i32, window_h: i32) -> Option<usize> {
        if x < 0 || y < LIST_TOP {
            return None;
        }
        let row = (y - LIST_TOP) / ROW_H;
        if row < 0 || row as usize >= Self::visible_rows(window_h) {
            return None;
        }
        let pos = self.scroll + row as usize;
        (pos < self.matches().len()).then_some(pos)
    }

    /// The button at window-local (`x`, `y`), or `None`.
    pub fn button_at(&self, x: i32, y: i32, window_w: i32, window_h: i32) -> Option<JumpButton> {
        if y < window_h - BUTTON_H || y >= window_h {
            return None;
        }
        if (PAD..PAD + BTN_W).contains(&x) {
            Some(JumpButton::Jump)
        } else if (window_w - PAD - BTN_W..window_w - PAD).contains(&x) {
            Some(JumpButton::Close)
        } else {
            None
        }
    }

    /// Clamp the selection into range and scroll the minimum needed to keep it visible.
    fn clamp_and_scroll(&mut self, window_h: i32) {
        let n = self.matches().len();
        if n == 0 {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        self.selected = self.selected.min(n - 1);
        let vis = Self::visible_rows(window_h);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if vis > 0 && self.selected >= self.scroll + vis {
            self.scroll = self.selected + 1 - vis;
        }
    }
}

/// Rendering theme for the Jump dialog. `classic()` keeps the original dark neutral chrome (used
/// when no system UI font is available); `adwaita(...)` paints a native GNOME look in the palette
/// with the system font. Interaction geometry is identical, so hit-testing is unaffected.
pub struct JumpTheme<'a> {
    palette: Palette,
    font: Option<&'a UiFont>,
}

impl Default for JumpTheme<'_> {
    fn default() -> Self {
        Self::classic()
    }
}

impl<'a> JumpTheme<'a> {
    pub fn classic() -> Self {
        Self {
            palette: Palette::dark(),
            font: None,
        }
    }

    pub fn adwaita(palette: Palette, font: &'a UiFont) -> Self {
        Self {
            palette,
            font: Some(font),
        }
    }
}

/// Compose the dialog. With a system font it renders the native Adwaita look; otherwise the classic
/// dark chrome.
pub fn compose(state: &JumpState, width: i32, height: i32, theme: &JumpTheme) -> Framebuffer {
    match theme.font {
        Some(font) => compose_adwaita(state, width, height, &theme.palette, font),
        None => compose_classic(state, width, height),
    }
}

fn jtext(fb: &mut Framebuffer, font: &UiFont, x: i32, top_y: i32, s: &str, px: f32, color: [u8; 4]) {
    let baseline = top_y + (px * 0.78).round() as i32;
    font.draw_text(fb, x, baseline, s, px, color);
}

#[allow(clippy::too_many_arguments)]
fn jtext_clipped(
    fb: &mut Framebuffer,
    font: &UiFont,
    x: i32,
    top_y: i32,
    s: &str,
    max_w: i32,
    px: f32,
    color: [u8; 4],
) {
    if font.text_width(s, px) as i32 <= max_w {
        jtext(fb, font, x, top_y, s, px, color);
        return;
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "\u{2026}";
        if font.text_width(&candidate, px) as i32 <= max_w {
            jtext(fb, font, x, top_y, &candidate, px, color);
            return;
        }
    }
}

fn compose_adwaita(
    state: &JumpState,
    width: i32,
    height: i32,
    p: &Palette,
    font: &UiFont,
) -> Framebuffer {
    let (w, h) = (width.max(JUMP_W), height.max(JUMP_H));
    let mut fb = Framebuffer::new(w as u32, h as u32);
    adwaita::fill_rect(&mut fb, 0, 0, w, h, p.window_bg);

    // Headerbar.
    jtext(&mut fb, font, PAD + 2, 2, "Jump to file", 13.0, p.fg);
    adwaita::draw_separator(&mut fb, 0, TITLE_H - 1, w, p);

    // Search entry (view background rounded field with a hairline border).
    let field_y = TITLE_H + 3;
    let field_h = SEARCH_H;
    adwaita::fill_rounded_rect(&mut fb, PAD, field_y, w - 2 * PAD, field_h, 6, p.view_bg);
    adwaita::stroke_rounded_rect(&mut fb, PAD, field_y, w - 2 * PAD, field_h, 6, 1, p.border);
    let matches = state.matches();
    let shown = format!("{}_", state.query);
    jtext(&mut fb, font, PAD + 7, field_y + 2, &shown, 12.0, p.fg);
    let counter = format!("{}/{}", matches.len(), state.rows.len());
    let cw = font.text_width(&counter, 11.0) as i32;
    jtext(&mut fb, font, w - PAD - 7 - cw, field_y + 3, &counter, 11.0, p.dim_fg);

    // Results list.
    let vis = JumpState::visible_rows(h);
    let list_w = w - 2 * PAD;
    for (row, &track) in matches.iter().enumerate().skip(state.scroll).take(vis) {
        let screen = (row - state.scroll) as i32;
        let y = LIST_TOP + screen * ROW_H;
        let title = state.rows.get(track).map(|r| r.title.as_str()).unwrap_or("");
        let selected = row == state.selected;
        if selected {
            adwaita::fill_rounded_rect(&mut fb, PAD - 1, y - 1, list_w, ROW_H, 5, p.accent_bg);
        }
        let color = if selected { p.accent_fg } else { p.fg };
        jtext_clipped(&mut fb, font, PAD + 4, y - 2, title, list_w - 8, 11.0, color);
    }

    // Bottom bar with rounded buttons (Jump is the suggested/primary action).
    let by = h - BUTTON_H;
    adwaita::draw_separator(&mut fb, 0, by, w, p);
    abutton(&mut fb, font, PAD, by + 3, "Jump", p, true);
    abutton(&mut fb, font, w - PAD - BTN_W, by + 3, "Close", p, false);
    fb
}

fn abutton(fb: &mut Framebuffer, font: &UiFont, x: i32, y: i32, label: &str, p: &Palette, primary: bool) {
    let (h, r) = (BUTTON_H - 6, 6);
    let (bg, fg) = if primary { (p.accent_bg, p.accent_fg) } else { (p.view_bg, p.fg) };
    adwaita::fill_rounded_rect(fb, x, y, BTN_W, h, r, bg);
    if !primary {
        adwaita::stroke_rounded_rect(fb, x, y, BTN_W, h, r, 1, p.border);
    }
    let lw = font.text_width(label, 12.0) as i32;
    jtext(fb, font, x + (BTN_W - lw) / 2, y + (h - 12) / 2, label, 12.0, fg);
}

/// Compose the dialog at `width` x `height`.
fn compose_classic(state: &JumpState, width: i32, height: i32) -> Framebuffer {
    let (w, h) = (width.max(JUMP_W), height.max(JUMP_H));
    let mut fb = Framebuffer::new(w as u32, h as u32);
    fill(&mut fb, 0, 0, w, h, BG);

    // Title bar.
    fill(&mut fb, 0, 0, w, TITLE_H, BAR);
    text(&mut fb, PAD, 5, "JUMP TO FILE", FG);

    // Search field with the query and a block cursor, plus the found/total counter on the right.
    let field_y = TITLE_H + 2;
    fill(&mut fb, PAD, field_y, w - 2 * PAD, SEARCH_H - 2, FIELD_BG);
    let matches = state.matches();
    let shown = format!("{}_", state.query);
    text(&mut fb, PAD + 3, field_y + 4, &shown, FG);
    let counter = format!("{}/{}", matches.len(), state.rows.len());
    let cx = w - PAD - 3 - font::text_width(&counter) as i32;
    text(&mut fb, cx, field_y + 4, &counter, DIM);

    // Results list.
    let vis = JumpState::visible_rows(h);
    let list_w = w - 2 * PAD;
    for (row, &track) in matches.iter().enumerate().skip(state.scroll).take(vis) {
        let screen = (row - state.scroll) as i32;
        let y = LIST_TOP + screen * ROW_H;
        let title = state.rows.get(track).map(|r| r.title.as_str()).unwrap_or("");
        if row == state.selected {
            fill(&mut fb, PAD - 1, y - 1, list_w, ROW_H, SEL_BG);
        }
        let max_chars = ((list_w - 4) / font::ADVANCE as i32).max(0) as usize;
        let clipped: String = title.chars().take(max_chars).collect();
        text(&mut fb, PAD + 1, y, &clipped, FG);
    }

    // Bottom buttons.
    let by = h - BUTTON_H;
    fill(&mut fb, 0, by, w, BUTTON_H, BAR);
    button(&mut fb, PAD, by, "JUMP");
    button(&mut fb, w - PAD - BTN_W, by, "CLOSE");
    fb
}

/// Draw a labelled button box.
fn button(fb: &mut Framebuffer, x: i32, y: i32, label: &str) {
    fill(fb, x, y + 3, BTN_W, BUTTON_H - 6, FIELD_BG);
    let tx = x + (BTN_W - font::text_width(label) as i32) / 2;
    text(fb, tx, y + 7, label, FG);
}

/// Fill an opaque rectangle, clipped to the framebuffer.
fn fill(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
    for yy in y.max(0)..(y + h).min(fb.height as i32) {
        for xx in x.max(0)..(x + w).min(fb.width as i32) {
            let o = ((yy as u32 * fb.width + xx as u32) * 4) as usize;
            fb.rgba[o] = c[0];
            fb.rgba[o + 1] = c[1];
            fb.rgba[o + 2] = c[2];
            fb.rgba[o + 3] = 255;
        }
    }
}

/// Draw text with the clean-room 5x7 font.
fn text(fb: &mut Framebuffer, x: i32, y: i32, s: &str, c: [u8; 3]) {
    font::draw_text(&mut fb.rgba, fb.width, fb.height, x, y, s, c);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(titles: &[&str]) -> JumpState {
        JumpState {
            rows: titles.iter().map(|t| Row { title: (*t).into(), ..Default::default() }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn filter_also_matches_hidden_metadata() {
        let mut state = JumpState {
            rows: vec![
                Row {
                    title: "1. Aphex Twin - Xtal".into(),
                    search: "aphex twin xtal selected ambient works 85-92 ambient 1992 xtal.mp3"
                        .into(),
                    ..Default::default()
                },
                Row {
                    title: "2. Boards of Canada - Roygbiv".into(),
                    search: "boards of canada roygbiv music has the right to children 1998".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        // An album word matches even though no row title contains it.
        state.set_query("ambient works".into(), JUMP_H);
        assert_eq!(state.matches(), [0]);
        // A year finds its track.
        state.set_query("1998".into(), JUMP_H);
        assert_eq!(state.matches(), [1]);
        // Tokens may mix shown title and hidden metadata.
        state.set_query("boards children".into(), JUMP_H);
        assert_eq!(state.matches(), [1]);
        state.set_query("nowhere".into(), JUMP_H);
        assert!(state.matches().is_empty());
    }

    #[test]
    fn empty_query_matches_everything() {
        let s = state(&["a", "b", "c"]);
        assert_eq!(s.matches(), vec![0, 1, 2]);
    }

    #[test]
    fn filter_is_token_and_and_case_insensitive() {
        let mut s = state(&["1. Cry Wolf", "2. Take On Me", "3. The Sun Always Shines On TV"]);
        s.query = "take".into();
        assert_eq!(s.matches(), vec![1]);
        s.query = "SUN tv".into();
        assert_eq!(s.matches(), vec![2], "all tokens must appear, any case/order");
        s.query = "the".into();
        assert_eq!(s.matches(), vec![2], "matches the one title containing 'the'");
        s.query = "zzz".into();
        assert!(s.matches().is_empty());
    }

    #[test]
    fn selected_track_maps_to_the_original_index() {
        let mut s = state(&["red apple", "blue sky", "green apple", "yellow sun"]);
        s.set_query("apple".into(), JUMP_H); // original indices 0 and 2
        assert_eq!(s.matches(), vec![0, 2]);
        assert_eq!(s.selected, 0);
        assert_eq!(s.selected_track(), Some(0));
        s.move_selection(1, JUMP_H);
        assert_eq!(s.selected_track(), Some(2), "second match is original index 2");
        s.move_selection(5, JUMP_H); // clamps to the last match
        assert_eq!(s.selected_track(), Some(2));
        s.move_selection(-10, JUMP_H); // clamps to the first
        assert_eq!(s.selected_track(), Some(0));
    }

    #[test]
    fn editing_the_query_resets_the_selection() {
        let mut s = state(&["one", "two", "three"]);
        s.set_query(String::new(), JUMP_H);
        s.move_selection(2, JUMP_H);
        assert_eq!(s.selected, 2);
        s.set_query("t".into(), JUMP_H); // two, three
        assert_eq!(s.selected, 0, "a query edit resets to the first match");
        assert_eq!(s.selected_track(), Some(1));
    }

    #[test]
    fn selection_scrolls_into_view_in_a_short_window() {
        // A window only tall enough for a few rows, with many matches.
        let titles: Vec<String> = (0..50).map(|i| format!("track {i}")).collect();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        let mut s = state(&refs);
        s.set_query(String::new(), JUMP_H);
        let vis = JumpState::visible_rows(JUMP_H);
        assert!(vis > 0 && vis < 50);
        s.move_selection(49, JUMP_H); // to the last match
        assert!(s.scroll + vis > s.selected && s.selected >= s.scroll, "last match is in view");
        assert_eq!(s.selected, 49);
    }

    #[test]
    fn row_at_and_button_at_hit_test() {
        let titles: Vec<String> = (0..30).map(|i| format!("t{i}")).collect();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        let mut s = state(&refs);
        s.set_query(String::new(), JUMP_H);
        // The first list row maps to match position 0.
        assert_eq!(s.row_at(20, LIST_TOP, JUMP_H), Some(0));
        assert_eq!(s.row_at(20, LIST_TOP + ROW_H, JUMP_H), Some(1));
        // Above the list: nothing.
        assert_eq!(s.row_at(20, TITLE_H, JUMP_H), None);
        // Buttons live on the bottom bar.
        assert_eq!(s.button_at(PAD, JUMP_H - 1, JUMP_W, JUMP_H), Some(JumpButton::Jump));
        assert_eq!(s.button_at(JUMP_W - PAD - 1, JUMP_H - 1, JUMP_W, JUMP_H), Some(JumpButton::Close));
        assert_eq!(s.button_at(JUMP_W / 2, JUMP_H - 1, JUMP_W, JUMP_H), None, "gap between buttons");
    }

    #[test]
    fn compose_renders_at_least_the_default_size() {
        let s = state(&["hello world"]);
        let theme = JumpTheme::classic();
        let fb = compose(&s, JUMP_W, JUMP_H, &theme);
        assert_eq!((fb.width, fb.height), (JUMP_W as u32, JUMP_H as u32));
        // A smaller request is clamped up to the minimum.
        let fb = compose(&s, 10, 10, &theme);
        assert_eq!((fb.width, fb.height), (JUMP_W as u32, JUMP_H as u32));
    }

    #[test]
    fn adwaita_theme_is_opaque_and_differs_by_palette_when_a_font_is_present() {
        let Some(font) = UiFont::load_system() else {
            return;
        };
        let s = state(&["hello world", "another track"]);
        let light = compose(&s, JUMP_W, JUMP_H, &JumpTheme::adwaita(Palette::light(), &font));
        let dark = compose(&s, JUMP_W, JUMP_H, &JumpTheme::adwaita(Palette::dark(), &font));
        assert!(light.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255), "opaque");
        assert_ne!(light.rgba, dark.rgba, "light and dark differ");
    }
}
