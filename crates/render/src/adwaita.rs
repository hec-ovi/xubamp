//! Native GNOME (libadwaita) drawing primitives and a system-UI-font text rasterizer.
//!
//! The classic UI is bitmap sprites; the non-skin menus and dialogs instead want a look that reads
//! as native to GNOME 50. This module is pure: it composites Adwaita-styled shapes and system-font
//! text straight into a [`Framebuffer`] with plain straight-alpha source-over blending. It knows
//! nothing about Wayland, D-Bus, or the current color scheme; the caller decides light versus dark
//! and hands the matching [`Palette`] to each helper. Color-scheme detection is not this crate's job.

use crate::Framebuffer;

/// A named-color set matching the real libadwaita defaults. Colors are straight (non-premultiplied)
/// `RGBA8`; overlay colors carry a fractional alpha and are meant to be composited over a filled
/// background (a hovered or pressed row, a separator line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Window/dialog body background.
    pub window_bg: [u8; 4],
    /// Popover (menu) background.
    pub popover_bg: [u8; 4],
    /// Content view background (lists, entries).
    pub view_bg: [u8; 4],
    /// Primary text/foreground.
    pub fg: [u8; 4],
    /// Dimmed text (secondary labels, shortcuts).
    pub dim_fg: [u8; 4],
    /// Accent fill (checked toggles, focused controls).
    pub accent_bg: [u8; 4],
    /// Text drawn on top of `accent_bg`.
    pub accent_fg: [u8; 4],
    /// Hover state overlay, composited over the surface beneath.
    pub hover: [u8; 4],
    /// Pressed/active state overlay, composited over the surface beneath.
    pub active: [u8; 4],
    /// Selected-row fill (opaque accent).
    pub selected_row: [u8; 4],
    /// Text drawn on a selected row.
    pub selected_fg: [u8; 4],
    /// Hairline border around popovers and dialogs.
    pub border: [u8; 4],
    /// Separator line color.
    pub separator: [u8; 4],
    /// Keyboard focus ring color.
    pub focus_ring: [u8; 4],
}

impl Palette {
    /// The default Adwaita light theme colors.
    pub fn light() -> Self {
        Self {
            window_bg: [0xfa, 0xfa, 0xfb, 255],
            popover_bg: [0xff, 0xff, 0xff, 255],
            view_bg: [0xff, 0xff, 0xff, 255],
            fg: [0, 0, 0, 230],       // rgba(0,0,0,0.9)
            dim_fg: [0, 0, 0, 140],   // rgba(0,0,0,0.55)
            accent_bg: [0x35, 0x84, 0xe4, 255],
            accent_fg: [0xff, 0xff, 0xff, 255],
            hover: [0, 0, 0, 13],     // rgba(0,0,0,0.05)
            active: [0, 0, 0, 26],    // rgba(0,0,0,0.10)
            selected_row: [0x35, 0x84, 0xe4, 255],
            selected_fg: [0xff, 0xff, 0xff, 255],
            border: [0, 0, 0, 38],    // rgba(0,0,0,0.15)
            separator: [0, 0, 0, 26], // rgba(0,0,0,0.10)
            focus_ring: [0x35, 0x84, 0xe4, 255],
        }
    }

    /// The default Adwaita dark theme colors.
    pub fn dark() -> Self {
        Self {
            window_bg: [0x24, 0x24, 0x24, 255],
            popover_bg: [0x38, 0x38, 0x38, 255],
            view_bg: [0x1e, 0x1e, 0x1e, 255],
            fg: [255, 255, 255, 230],     // rgba(255,255,255,0.9)
            dim_fg: [255, 255, 255, 140], // rgba(255,255,255,0.55)
            accent_bg: [0x35, 0x84, 0xe4, 255],
            accent_fg: [0xff, 0xff, 0xff, 255],
            hover: [255, 255, 255, 15],   // rgba(255,255,255,0.06)
            active: [255, 255, 255, 31],  // rgba(255,255,255,0.12)
            selected_row: [0x35, 0x84, 0xe4, 255],
            selected_fg: [0xff, 0xff, 0xff, 255],
            border: [255, 255, 255, 38],    // rgba(255,255,255,0.15)
            separator: [255, 255, 255, 26], // rgba(255,255,255,0.10)
            focus_ring: [0x78, 0xae, 0xed, 255],
        }
    }
}

/// Corner radius of a popover (menu) surface, in pixels.
pub const POPOVER_RADIUS: i32 = 12;
/// Corner radius of a window or dialog surface, in pixels.
pub const WINDOW_RADIUS: i32 = 12;
/// Thickness of a separator line, in pixels.
pub const SEPARATOR_THICKNESS: i32 = 1;
/// Thickness of the keyboard focus ring, in pixels.
pub const FOCUS_RING_THICKNESS: i32 = 2;

/// Straight-alpha source-over blend of `rgba` (scaled by `coverage` in `0.0..=1.0`) onto the pixel
/// at (`x`, `y`). Clipped to the framebuffer, so out-of-bounds coordinates are simply dropped. When
/// the destination is opaque this reduces to `out = src * a + dst * (1 - a)`; when it is transparent
/// the source shows through with its own alpha, so filled corners stay see-through.
fn blend_pixel(fb: &mut Framebuffer, x: i32, y: i32, rgba: [u8; 4], coverage: f32) {
    if x < 0 || y < 0 || x >= fb.width as i32 || y >= fb.height as i32 {
        return;
    }
    let sa = (rgba[3] as f32 / 255.0) * coverage.clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    let offset = ((y as u32 * fb.width + x as u32) * 4) as usize;
    let dst = &mut fb.rgba[offset..offset + 4];
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a > 0.0 {
        for c in 0..3 {
            let s = rgba[c] as f32;
            let d = dst[c] as f32;
            let out = (s * sa + d * da * (1.0 - sa)) / out_a;
            dst[c] = out.round().clamp(0.0, 255.0) as u8;
        }
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// sRGB channel (0..=255) to linear light (0.0..=1.0).
fn srgb_to_linear(c: u8) -> f32 {
    let s = c as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light (0.0..=1.0) back to an sRGB channel (0..=255).
fn linear_to_srgb(l: f32) -> u8 {
    let s = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Glyph-coverage blend in linear light: each channel is linearized, mixed by coverage, and
/// re-encoded. Assumes an opaque destination (every surface here composes over an opaque base).
fn blend_glyph_linear(fb: &mut Framebuffer, x: i32, y: i32, rgba: [u8; 4], coverage: f32) {
    if x < 0 || y < 0 || x >= fb.width as i32 || y >= fb.height as i32 {
        return;
    }
    let cov = (coverage * rgba[3] as f32 / 255.0).clamp(0.0, 1.0);
    if cov <= 0.0 {
        return;
    }
    let offset = ((y as u32 * fb.width + x as u32) * 4) as usize;
    let dst = &mut fb.rgba[offset..offset + 4];
    for c in 0..3 {
        let s = srgb_to_linear(rgba[c]);
        let d = srgb_to_linear(dst[c]);
        dst[c] = linear_to_srgb(s * cov + d * (1.0 - cov));
    }
    dst[3] = 255;
}

/// Fill an axis-aligned rectangle, alpha-blended and clipped to the framebuffer.
pub fn fill_rect(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, rgba: [u8; 4]) {
    for yy in y..y + h.max(0) {
        for xx in x..x + w.max(0) {
            blend_pixel(fb, xx, yy, rgba, 1.0);
        }
    }
}

/// Coverage in `0.0..=1.0` of the pixel centered at (`px`, `py`) by a rounded rectangle. Straight
/// edges return full coverage; the four corner arcs get an antialiased one-pixel feather from a
/// distance test against the corner circle center. Degenerate (non-positive) rectangles cover
/// nothing.
fn rounded_coverage(px: f32, py: f32, x: i32, y: i32, w: i32, h: i32, radius: f32) -> f32 {
    if w <= 0 || h <= 0 {
        return 0.0;
    }
    let left = x as f32;
    let top = y as f32;
    let right = (x + w) as f32;
    let bottom = (y + h) as f32;
    if px < left || px >= right || py < top || py >= bottom {
        return 0.0;
    }
    let r = radius.max(0.0).min(w as f32 / 2.0).min(h as f32 / 2.0);
    if r <= 0.0 {
        return 1.0;
    }
    // Snap the corner circle center to the nearest rounded corner; pixels in the straight middle
    // bands keep their own coordinate, so the distance is zero and coverage is full.
    let cx = if px < left + r {
        left + r
    } else if px > right - r {
        right - r
    } else {
        px
    };
    let cy = if py < top + r {
        top + r
    } else if py > bottom - r {
        bottom - r
    } else {
        py
    };
    let dx = px - cx;
    let dy = py - cy;
    let dist = (dx * dx + dy * dy).sqrt();
    (r - dist + 0.5).clamp(0.0, 1.0)
}

/// Fill a rounded rectangle with antialiased corners, alpha-blended and clipped to the framebuffer.
pub fn fill_rounded_rect(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, radius: i32, rgba: [u8; 4]) {
    if w <= 0 || h <= 0 {
        return;
    }
    let radius = radius as f32;
    for yy in y..y + h {
        for xx in x..x + w {
            let cov = rounded_coverage(xx as f32 + 0.5, yy as f32 + 0.5, x, y, w, h, radius);
            if cov > 0.0 {
                blend_pixel(fb, xx, yy, rgba, cov);
            }
        }
    }
}

/// Stroke the outline of a rounded rectangle. The ring is the difference between the outer shape and
/// a shape inset by `thickness`, so it stays crisp and antialiased on both edges.
#[allow(clippy::too_many_arguments)]
pub fn stroke_rounded_rect(
    fb: &mut Framebuffer,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    thickness: i32,
    rgba: [u8; 4],
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let t = thickness.max(1);
    let outer_r = radius as f32;
    let inner_r = (radius - t).max(0) as f32;
    for yy in y..y + h {
        for xx in x..x + w {
            let px = xx as f32 + 0.5;
            let py = yy as f32 + 0.5;
            let outer = rounded_coverage(px, py, x, y, w, h, outer_r);
            let inner = rounded_coverage(px, py, x + t, y + t, w - 2 * t, h - 2 * t, inner_r);
            let cov = (outer - inner).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_pixel(fb, xx, yy, rgba, cov);
            }
        }
    }
}

/// Draw a two-pixel accent focus ring, inset one pixel from the given bounds so it sits just inside
/// the control rather than on its edge.
pub fn draw_focus_ring(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, radius: i32, palette: &Palette) {
    let inset = 1;
    stroke_rounded_rect(
        fb,
        x + inset,
        y + inset,
        w - 2 * inset,
        h - 2 * inset,
        (radius - inset).max(0),
        FOCUS_RING_THICKNESS,
        palette.focus_ring,
    );
}

/// Draw a one-pixel horizontal separator in the palette's separator color.
pub fn draw_separator(fb: &mut Framebuffer, x: i32, y: i32, w: i32, palette: &Palette) {
    fill_rect(fb, x, y, w, SEPARATOR_THICKNESS, palette.separator);
}

/// A system UI font (Adwaita Sans / Cantarell / DejaVu Sans) wrapping a [`fontdue::Font`], used to
/// rasterize menu and dialog labels in the native GNOME look.
pub struct UiFont {
    font: fontdue::Font,
}

impl UiFont {
    /// Parse a font from raw `TrueType`/`OpenType` bytes. Returns `None` if the data is not a font.
    pub fn from_bytes(bytes: &[u8]) -> Option<UiFont> {
        fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .ok()
            .map(|font| UiFont { font })
    }

    /// Load the first available system UI font, trying Adwaita Sans, then Cantarell, then DejaVu Sans
    /// at their common install paths. Returns `None` when none of them are present.
    pub fn load_system() -> Option<UiFont> {
        const PATHS: &[&str] = &[
            "/usr/share/fonts/adwaita-sans-fonts/AdwaitaSans-Regular.ttf",
            "/usr/share/fonts/adwaita-sans/AdwaitaSans-Regular.ttf",
            "/usr/share/fonts/cantarell/Cantarell-Regular.otf",
            "/usr/share/fonts/abattis-cantarell/Cantarell-Regular.otf",
            "/usr/share/fonts/truetype/cantarell/Cantarell-Regular.ttf",
            "/usr/share/fonts/gnome-shell/Cantarell-Regular.otf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        ];
        for path in PATHS {
            if let Ok(bytes) = std::fs::read(path) {
                if let Some(font) = Self::from_bytes(&bytes) {
                    return Some(font);
                }
            }
        }
        None
    }

    /// Resolve a font-face name (as a skin's `pledit.txt` writes it, e.g. `Arial`) through
    /// fontconfig's `fc-match`, which substitutes a metric-compatible installed face for missing
    /// Windows ones (Arial usually lands on Liberation Sans). Returns `None` when `fc-match` is
    /// unavailable or the matched file cannot be parsed as a font.
    pub fn load_named(name: &str) -> Option<UiFont> {
        let out = std::process::Command::new("fc-match")
            .args(["--format=%{file}", name])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let path = String::from_utf8(out.stdout).ok()?;
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        Self::from_bytes(&std::fs::read(path).ok()?)
    }

    /// Total advance width of `text` at `px` pixels-per-em, summed over each glyph's metrics.
    pub fn text_width(&self, text: &str, px: f32) -> f32 {
        text.chars()
            .map(|ch| self.font.metrics(ch, px).advance_width)
            .sum()
    }

    /// Advance width of a single character at `px` pixels-per-em.
    pub fn advance(&self, ch: char, px: f32) -> f32 {
        self.font.metrics(ch, px).advance_width
    }

    /// Baseline-to-top ascent at `px` pixels-per-em. Falls back to 0.8em when the font exposes no
    /// horizontal line metrics.
    pub fn ascent(&self, px: f32) -> f32 {
        self.font
            .horizontal_line_metrics(px)
            .map_or(px * 0.8, |m| m.ascent)
    }

    /// Distance between baselines for a single line at `px` pixels-per-em. Falls back to a plain
    /// 1.2x multiple when the font exposes no horizontal line metrics.
    pub fn line_height(&self, px: f32) -> f32 {
        self.font
            .horizontal_line_metrics(px)
            .map_or(px * 1.2, |m| m.new_line_size)
    }

    /// Rasterize `text` left-to-right starting at pen position `x`, with the glyph baseline at
    /// `baseline_y`. Each glyph's coverage bitmap is alpha-composited in `rgba`, positioned by its
    /// bearings, and the pen advances by `advance_width`. Clipped to the framebuffer. ASCII and
    /// Latin-1 are enough for the labels this draws.
    pub fn draw_text(&self, fb: &mut Framebuffer, x: i32, baseline_y: i32, text: &str, px: f32, rgba: [u8; 4]) {
        let mut pen = x as f32;
        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, px);
            // fontdue's bitmap starts at the glyph's top-left corner: xmin is the left bearing, and
            // ymin is the offset of the bitmap bottom above the baseline, so the top row sits at
            // baseline_y - ymin - height.
            let gx = pen.round() as i32 + metrics.xmin;
            let gy = baseline_y - metrics.ymin - metrics.height as i32;
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let coverage = bitmap[row * metrics.width + col] as f32 / 255.0;
                    if coverage > 0.0 {
                        blend_pixel(fb, gx + col as i32, gy + row as i32, rgba, coverage);
                    }
                }
            }
            pen += metrics.advance_width;
        }
    }

    /// Like [`draw_text`](Self::draw_text), but compositing the glyph coverage in linear light
    /// instead of sRGB space. Blending in sRGB darkens every partially-covered edge pixel, which
    /// reads as a muddy fringe around small light-on-dark text (the classic green-on-black
    /// playlist); linear blending keeps those edges at their true brightness. Menu/dialog text
    /// keeps the sRGB blend, matching how GTK renders its dark-on-light labels.
    pub fn draw_text_linear(&self, fb: &mut Framebuffer, x: i32, baseline_y: i32, text: &str, px: f32, rgba: [u8; 4]) {
        let mut pen = x as f32;
        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, px);
            let gx = pen.round() as i32 + metrics.xmin;
            let gy = baseline_y - metrics.ymin - metrics.height as i32;
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let coverage = bitmap[row * metrics.width + col] as f32 / 255.0;
                    if coverage > 0.0 {
                        blend_glyph_linear(fb, gx + col as i32, gy + row as i32, rgba, coverage);
                    }
                }
            }
            pen += metrics.advance_width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(fb: &Framebuffer, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * fb.width + x) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const ACCENT: [u8; 4] = [0x35, 0x84, 0xe4, 255];

    #[test]
    fn srgb_linear_round_trips_every_channel_value() {
        for v in 0..=255u8 {
            assert_eq!(linear_to_srgb(srgb_to_linear(v)), v);
        }
    }

    #[test]
    fn linear_glyph_blend_keeps_half_coverage_at_true_half_brightness() {
        let mut fb = Framebuffer::new(1, 1);
        fill_rect(&mut fb, 0, 0, 1, 1, [0, 0, 0, 255]);
        blend_glyph_linear(&mut fb, 0, 0, WHITE, 0.5);
        // Half linear light re-encodes near sRGB 188; the old sRGB-space blend gave 128,
        // which is only ~21% of white's light and read as a dark fringe.
        assert_eq!(px(&fb, 0, 0)[0], 188);
    }

    #[test]
    fn linear_text_is_never_darker_than_srgb_text_on_black() {
        let Some(font) = UiFont::load_system() else {
            return;
        };
        let mut srgb = Framebuffer::new(80, 16);
        let mut linear = Framebuffer::new(80, 16);
        for fb in [&mut srgb, &mut linear] {
            fill_rect(fb, 0, 0, 80, 16, [0, 0, 0, 255]);
        }
        font.draw_text(&mut srgb, 2, 12, "legible", 10.0, WHITE);
        font.draw_text_linear(&mut linear, 2, 12, "legible", 10.0, WHITE);
        let sum = |fb: &Framebuffer| fb.rgba.iter().map(|&v| v as u64).sum::<u64>();
        assert!(
            sum(&linear) > sum(&srgb),
            "linear blending brightens antialiased edges on a dark background"
        );
        for (l, s) in linear.rgba.iter().zip(srgb.rgba.iter()) {
            assert!(l >= s, "no pixel gets darker");
        }
    }

    #[test]
    fn load_named_mirrors_what_fontconfig_matches() {
        let matched = std::process::Command::new("fc-match")
            .args(["--format=%{file}", "Arial"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty());
        let font = UiFont::load_named("Arial");
        match matched {
            // fc-match produced a file: load_named succeeds exactly when fontdue can parse it.
            Some(path) => assert_eq!(
                font.is_some(),
                std::fs::read(&path)
                    .ok()
                    .and_then(|b| UiFont::from_bytes(&b))
                    .is_some()
            ),
            None => assert!(font.is_none(), "no fontconfig means no named font"),
        }
    }

    #[test]
    fn palettes_share_the_accent_but_differ_in_background() {
        let light = Palette::light();
        let dark = Palette::dark();
        assert_eq!(light.accent_bg, ACCENT, "light accent is #3584e4");
        assert_eq!(dark.accent_bg, ACCENT, "dark accent is #3584e4");
        assert_ne!(
            light.window_bg, dark.window_bg,
            "light and dark window backgrounds differ"
        );
        assert_eq!(light.window_bg, [0xfa, 0xfa, 0xfb, 255]);
        assert_eq!(dark.window_bg, [0x24, 0x24, 0x24, 255]);
        assert_eq!(dark.focus_ring, [0x78, 0xae, 0xed, 255], "dark ring is lighter");
    }

    #[test]
    fn fill_rect_full_alpha_sets_exact_color_and_half_alpha_blends() {
        let mut fb = Framebuffer::new(10, 10);
        // Full alpha writes the exact color.
        fill_rect(&mut fb, 0, 0, 10, 10, [10, 20, 30, 255]);
        assert_eq!(px(&fb, 5, 5), [10, 20, 30, 255], "opaque fill is exact");

        // Half alpha over an opaque background lands halfway between the two.
        fill_rect(&mut fb, 0, 0, 10, 10, WHITE);
        fill_rect(&mut fb, 0, 0, 10, 10, [0, 0, 0, 128]);
        let mid = px(&fb, 5, 5);
        assert!(
            (120..=135).contains(&(mid[0] as i32)),
            "half-alpha black over white is roughly mid-gray, got {}",
            mid[0]
        );
        assert_eq!(mid[3], 255, "compositing over opaque stays opaque");
    }

    #[test]
    fn rounded_rect_fills_the_center_but_leaves_corners_lighter() {
        let mut fb = Framebuffer::new(40, 40);
        fill_rect(&mut fb, 0, 0, 40, 40, WHITE);
        fill_rounded_rect(&mut fb, 0, 0, 40, 40, 12, ACCENT);
        assert_eq!(px(&fb, 20, 20), ACCENT, "center is fully filled");
        // The middle of an edge is still full coverage.
        assert_eq!(px(&fb, 20, 0), ACCENT, "edge midpoint is full");
        // The extreme corner sits outside the radius, so the white background shows through.
        assert_eq!(px(&fb, 0, 0), WHITE, "corner stays background");
        assert_eq!(px(&fb, 39, 39), WHITE, "opposite corner stays background");
    }

    #[test]
    fn stroke_rounded_rect_draws_the_edge_and_not_the_middle() {
        let mut fb = Framebuffer::new(40, 40);
        let red = [255, 0, 0, 255];
        stroke_rounded_rect(&mut fb, 0, 0, 40, 40, 12, 2, red);
        // The left edge midpoint is on the ring.
        assert_eq!(px(&fb, 0, 20), red, "left edge is stroked");
        assert_eq!(px(&fb, 1, 20), red, "stroke is two pixels wide");
        // Two pixels in from the edge is already past the 2px ring.
        assert_eq!(px(&fb, 3, 20)[3], 0, "inside the ring is untouched");
        // The center is hollow.
        assert_eq!(px(&fb, 20, 20)[3], 0, "center is not stroked");
    }

    #[test]
    fn focus_ring_is_inset_accent_and_hollow() {
        let mut fb = Framebuffer::new(40, 40);
        let palette = Palette::light();
        draw_focus_ring(&mut fb, 0, 0, 40, 40, 12, &palette);
        // Inset by one pixel: the outermost column is untouched, the ring sits just inside.
        assert_eq!(px(&fb, 0, 20)[3], 0, "ring is inset one pixel");
        assert_eq!(px(&fb, 2, 20), palette.focus_ring, "ring drawn in accent");
        assert_eq!(px(&fb, 20, 20)[3], 0, "ring interior is hollow");
    }

    #[test]
    fn separator_touches_only_its_own_row_and_span() {
        let mut fb = Framebuffer::new(20, 6);
        let palette = Palette::light();
        draw_separator(&mut fb, 2, 3, 10, &palette);
        assert!(px(&fb, 5, 3)[3] > 0, "separator pixel is painted");
        assert_eq!(px(&fb, 5, 2)[3], 0, "nothing above the separator");
        assert_eq!(px(&fb, 5, 4)[3], 0, "nothing below the separator");
        assert_eq!(px(&fb, 1, 3)[3], 0, "nothing before the span");
        assert_eq!(px(&fb, 12, 3)[3], 0, "nothing at the end of the span");
    }

    #[test]
    fn drawing_out_of_bounds_clips_without_panicking() {
        let mut fb = Framebuffer::new(10, 10);
        // A rect straddling the top-left corner only paints its in-bounds region.
        fill_rect(&mut fb, -5, -5, 8, 8, [255, 0, 0, 255]);
        assert_eq!(px(&fb, 0, 0), [255, 0, 0, 255], "in-bounds corner painted");
        assert_eq!(px(&fb, 2, 2), [255, 0, 0, 255], "in-bounds interior painted");
        assert_eq!(px(&fb, 3, 3)[3], 0, "beyond the rect stays untouched");
        // Rounded and stroked shapes partly off-screen must not panic either.
        fill_rounded_rect(&mut fb, 8, 8, 6, 6, 3, [0, 255, 0, 255]);
        stroke_rounded_rect(&mut fb, -2, -2, 5, 5, 2, 1, [0, 0, 255, 255]);
        draw_focus_ring(&mut fb, -3, -3, 6, 6, 3, &Palette::dark());
    }

    #[test]
    fn from_bytes_rejects_non_font_data() {
        assert!(
            UiFont::from_bytes(b"not a font").is_none(),
            "garbage bytes are not a font"
        );
    }

    #[test]
    fn system_font_measures_and_rasterizes_when_present() {
        // Skip silently on hosts with none of the expected fonts, so this passes anywhere.
        let Some(font) = UiFont::load_system() else {
            return;
        };
        assert!(font.text_width("Hello", 14.0) > 0.0, "text has positive width");
        assert!(font.line_height(14.0) > 0.0, "line height is positive");

        let mut fb = Framebuffer::new(64, 24);
        fill_rect(&mut fb, 0, 0, 64, 24, WHITE);
        font.draw_text(&mut fb, 2, 18, "Hg", 14.0, [0, 0, 0, 255]);
        let touched = (0..(fb.width * fb.height) as usize).any(|i| {
            let o = i * 4;
            fb.rgba[o] != 255 || fb.rgba[o + 1] != 255 || fb.rgba[o + 2] != 255
        });
        assert!(touched, "draw_text painted at least one non-background pixel");
    }
}
