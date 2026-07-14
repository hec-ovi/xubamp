//! The main-window visualizer: a spectrum analyzer, an oscilloscope, or off, cycled by clicking
//! the region. It reads the recent mono output samples (tapped from the RT), and for the spectrum
//! runs a small hand-rolled radix-2 FFT (no external crate). Pure: samples plus the skin's
//! `viscolor` palette in, pixels out; the per-frame decay state lives in [`VisState`]. Geometry
//! and behaviour follow classic Winamp (cross-checked against Webamp): 75 columns in a 76x16
//! region, "wide" 3px bars with 1px gaps over a fixed vertical gradient that bars reveal as they
//! grow, and falling peak dots.

use std::f32::consts::TAU;

use xubamp_skin::color::Rgb;
use xubamp_skin::sprites;
use xubamp_skin::viscolor::VisColor;

use crate::Framebuffer;

/// Drawn columns (the region is 76 wide; the 76th stays background).
pub const VIS_COLS: usize = 75;
/// FFT size over the recent samples, matching classic Winamp / Webamp (1024-point).
pub const FFT_N: usize = 1024;

/// Region height in pixels (also the max bar value: a full bar fills all 16 rows).
const BAR_MAX: f32 = sprites::VIS_H as f32;
/// Magnitude at or below this many dBFS reads as an empty bar; 0 dBFS fills the region.
const FLOOR_DB: f32 = -66.0;
/// Oscilloscope centre row for a zero sample (Winamp's `round(sample*16) + 7`).
const OSC_CENTER: i32 = 7;

/// The falloff/refresh sliders run 1 (slowest) to [`SPEED_MAX`] (fastest), Winamp-style.
pub const SPEED_MAX: u8 = 10;

/// Pixels per frame the bar top falls at falloff `speed` (1..=SPEED_MAX). Level 5 reproduces the
/// old fixed 1.5 px/frame; higher drops faster.
fn bar_fall_px(speed: u8) -> f32 {
    0.3 * speed.clamp(1, SPEED_MAX) as f32
}

/// Pixels per frame^2 the peak dot accelerates at peak-falloff `speed`. Level 5 reproduces the old
/// fixed 0.03; higher drops the peak faster.
fn peak_gravity(speed: u8) -> f32 {
    0.006 * speed.clamp(1, SPEED_MAX) as f32
}

/// Which visualization is shown. Clicking the region cycles Bars -> Oscilloscope -> Off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisMode {
    #[default]
    Bars,
    Oscilloscope,
    Off,
}

impl VisMode {
    /// The next mode in the classic click cycle.
    pub fn next(self) -> VisMode {
        match self {
            VisMode::Bars => VisMode::Oscilloscope,
            VisMode::Oscilloscope => VisMode::Off,
            VisMode::Off => VisMode::Bars,
        }
    }
}

/// Spectrum-analyzer coloring style (classic Winamp): the plain vertical gradient, a flame that is
/// hottest at each bar's tip, or just the top edge line of each bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnalyzerStyle {
    #[default]
    Normal,
    Fire,
    Line,
}

/// Analyzer band width: `Thick` is the classic wide 3px bars (4px pitch); `Thin` is narrow 1px
/// bars (2px pitch), so more, slimmer bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BandWidth {
    #[default]
    Thick,
    Thin,
}

impl BandWidth {
    /// Column pitch: the bar occupies `pitch - 1` columns and the last column of each group is the
    /// gap.
    fn pitch(self) -> usize {
        match self {
            BandWidth::Thick => 4,
            BandWidth::Thin => 2,
        }
    }
}

/// Oscilloscope drawing style (classic Winamp): isolated dots, a connected line, or a filled area
/// down to the centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OscStyle {
    Dots,
    #[default]
    Lines,
    Solid,
}

/// Per-frame visualizer state: the mode, the spectrum bar heights with their falling peaks, and
/// the last oscilloscope column samples. Keep one across frames and step it with [`VisState::advance`].
#[derive(Debug, Clone, PartialEq)]
pub struct VisState {
    pub mode: VisMode,
    /// Whether the falling peak dots are visible in spectrum mode.
    pub show_peaks: bool,
    /// Spectrum coloring style.
    pub analyzer_style: AnalyzerStyle,
    /// Spectrum band width (Thick wide bars vs Thin narrow bars).
    pub band_width: BandWidth,
    /// Oscilloscope drawing style.
    pub osc_style: OscStyle,
    /// Bar-drop speed (1..=SPEED_MAX); higher falls faster.
    pub bar_falloff: u8,
    /// Peak-dot drop speed (1..=SPEED_MAX); higher falls faster.
    pub peak_falloff: u8,
    bars: [f32; VIS_COLS],
    peaks: [f32; VIS_COLS],
    peak_vel: [f32; VIS_COLS],
    scope: [u8; VIS_COLS],
}

impl Default for VisState {
    fn default() -> Self {
        VisState {
            mode: VisMode::default(),
            show_peaks: true,
            analyzer_style: AnalyzerStyle::default(),
            band_width: BandWidth::default(),
            osc_style: OscStyle::default(),
            // Snappier than the old fixed feel out of the box; both are user adjustable.
            bar_falloff: 7,
            peak_falloff: 6,
            bars: [0.0; VIS_COLS],
            peaks: [0.0; VIS_COLS],
            peak_vel: [0.0; VIS_COLS],
            scope: [OSC_CENTER as u8; VIS_COLS], // a flat centre line
        }
    }
}

impl VisState {
    /// Cycle to the next visualization mode (on a click in the region).
    pub fn cycle(&mut self) {
        self.mode = self.mode.next();
    }

    /// Advance one frame from the most recent mono `samples` (oldest first; ideally >= [`FFT_N`]
    /// long, shorter is zero-padded). For the spectrum, bars rise instantly and fall gradually
    /// with falling peak dots; feeding silence lets it settle to baseline (do that while paused or
    /// stopped). For the oscilloscope the columns follow the waveform. Returns whether the drawn
    /// output changed this frame, so the caller redraws exactly the frames that move (including the
    /// final settle-to-baseline frame) and can slow its timer once nothing changes.
    pub fn advance(&mut self, samples: &[f32]) -> bool {
        match self.mode {
            VisMode::Off => false,
            VisMode::Bars => {
                let mut target = [0.0f32; VIS_COLS];
                spectrum(samples, &mut target);
                group_wide(&mut target, self.band_width);
                let bar_fall = bar_fall_px(self.bar_falloff);
                let gravity = peak_gravity(self.peak_falloff);
                let mut changed = false;
                for (x, &t) in target.iter().enumerate() {
                    let (old_bar, old_peak) = (self.bars[x], self.peaks[x]);
                    // Rise instantly to the new magnitude; fall gradually toward it otherwise.
                    if t >= self.bars[x] {
                        self.bars[x] = t;
                    } else {
                        self.bars[x] = (self.bars[x] - bar_fall).max(t);
                    }
                    // Peak: reset to the bar when it tops the peak, else accelerate down toward 0.
                    if self.bars[x] >= self.peaks[x] {
                        self.peaks[x] = self.bars[x];
                        self.peak_vel[x] = 0.0;
                    } else {
                        self.peak_vel[x] += gravity;
                        self.peaks[x] = (self.peaks[x] - self.peak_vel[x]).max(0.0);
                    }
                    if self.bars[x] != old_bar || self.peaks[x] != old_peak {
                        changed = true;
                    }
                }
                changed
            }
            VisMode::Oscilloscope => {
                let mut next = [0u8; VIS_COLS];
                oscilloscope(samples, &mut next);
                let changed = next != self.scope;
                self.scope = next;
                changed
            }
        }
    }
}

/// Draw the visualizer into `fb` at the main-window region using the skin's `viscolor` palette.
pub fn draw(fb: &mut Framebuffer, vc: &VisColor, state: &VisState) {
    let (x0, y0, h) = (sprites::VIS_X, sprites::VIS_Y, sprites::VIS_H);
    // Background across the whole 76-wide region.
    fill(fb, x0, y0, sprites::VIS_W, h, vc.background());
    match state.mode {
        VisMode::Off => {}
        VisMode::Bars => {
            let grad = vc.analyzer(); // 16 colours: [0] = top (hottest) .. [15] = bottom
            let peak = vc.peak();
            let pitch = state.band_width.pitch();
            for x in 0..VIS_COLS {
                if x % pitch == pitch - 1 {
                    continue; // the gap column between bars
                }
                let bh = round_clamp(state.bars[x], h);
                if bh > 0 {
                    let top = h - bh;
                    match state.analyzer_style {
                        // Plain vertical gradient: the row's absolute colour, hottest at the top.
                        AnalyzerStyle::Normal => {
                            for row in top..h {
                                put(fb, x0 + x as i32, y0 + row, grad[row as usize]);
                            }
                        }
                        // Flame: hottest at the bar's own tip, cooling toward its base.
                        AnalyzerStyle::Fire => {
                            for row in top..h {
                                let from_tip = (row - top).clamp(0, 15) as usize;
                                put(fb, x0 + x as i32, y0 + row, grad[from_tip]);
                            }
                        }
                        // Just the top edge line of each bar (an envelope outline).
                        AnalyzerStyle::Line => {
                            put(fb, x0 + x as i32, y0 + top, grad[top as usize]);
                        }
                    }
                }
                if state.show_peaks {
                    let pk = round_clamp(state.peaks[x], h);
                    if pk > 0 {
                        put(fb, x0 + x as i32, y0 + (h - pk), peak);
                    }
                }
            }
        }
        VisMode::Oscilloscope => {
            let osc = vc.oscilloscope(); // 5 colours, centre-out
            match state.osc_style {
                // Isolated dots: one pixel per column at the sample row.
                OscStyle::Dots => {
                    for x in 0..VIS_COLS {
                        let y = state.scope[x] as i32;
                        put(fb, x0 + x as i32, y0 + y, osc[osc_color_index(y)]);
                    }
                }
                // Connected line: fill the vertical span between consecutive columns so the trace
                // is continuous rather than a scatter of dots.
                OscStyle::Lines => {
                    let mut prev = state.scope[0] as i32;
                    for x in 0..VIS_COLS {
                        let y = state.scope[x] as i32;
                        let (lo, hi) = if y < prev { (y, prev) } else { (prev, y) };
                        for row in lo..=hi {
                            put(fb, x0 + x as i32, y0 + row, osc[osc_color_index(row)]);
                        }
                        prev = y;
                    }
                }
                // Solid: fill from the centre line out to the sample, a filled waveform.
                OscStyle::Solid => {
                    for x in 0..VIS_COLS {
                        let y = state.scope[x] as i32;
                        let (lo, hi) = if y < OSC_CENTER {
                            (y, OSC_CENTER)
                        } else {
                            (OSC_CENTER, y)
                        };
                        for row in lo..=hi {
                            put(fb, x0 + x as i32, y0 + row, osc[osc_color_index(row)]);
                        }
                    }
                }
            }
        }
    }
}

/// A magnitude spectrum of `samples` mapped to [`VIS_COLS`] bar heights in `0..=VIS_H`. Applies a
/// Hann window, a radix-2 FFT, per-band peak magnitude on a log-frequency axis, and a dB amplitude
/// scale so bass does not swamp the display.
fn spectrum(samples: &[f32], out: &mut [f32; VIS_COLS]) {
    let mut re = [0.0f32; FFT_N];
    let mut im = [0.0f32; FFT_N];
    let n = samples.len().min(FFT_N);
    let off = samples.len() - n;
    for (i, r) in re.iter_mut().enumerate().take(n) {
        // Hann window over the FFT length reduces spectral leakage.
        let w = 0.5 - 0.5 * (TAU * i as f32 / (FFT_N as f32 - 1.0)).cos();
        *r = samples[off + i] * w;
    }
    fft(&mut re, &mut im);

    let bins = FFT_N / 2; // usable bins (1..bins); bin b is frequency b*rate/FFT_N
    for (x, o) in out.iter_mut().enumerate() {
        let b0 = log_bin(x, bins);
        let b1 = log_bin(x + 1, bins).max(b0 + 1).min(bins);
        // Loudest bin in this column's frequency band.
        let mut m = 0.0f32;
        for b in b0..b1 {
            let mag = (re[b] * re[b] + im[b] * im[b]).sqrt();
            if mag > m {
                m = mag;
            }
        }
        // Normalise the bin magnitude to ~amplitude, take dBFS, map [FLOOR_DB, 0] -> [0, VIS_H].
        let norm = m * 2.0 / FFT_N as f32;
        let db = 20.0 * norm.max(1e-9).log10();
        *o = ((db - FLOOR_DB) / -FLOOR_DB * BAR_MAX).clamp(0.0, BAR_MAX);
    }
}

/// The FFT bin for column `x` on a log-frequency axis spanning bins `1..bins` across the columns.
fn log_bin(x: usize, bins: usize) -> usize {
    let frac = x as f32 / VIS_COLS as f32; // 0 at column 0, 1 at column VIS_COLS
    let bin = (bins as f32 - 1.0).powf(frac); // 1 .. bins-1
    (bin.round() as usize).clamp(1, bins - 1)
}

/// Combine each group of `pitch` columns to the group's loudest value, so a narrow tone lights its
/// whole bar rather than being averaged away. Thick groups four columns (3px bars); Thin groups two
/// (1px bars).
fn group_wide(v: &mut [f32; VIS_COLS], band: BandWidth) {
    let pitch = band.pitch();
    let mut x = 0;
    while x < VIS_COLS {
        let end = (x + pitch).min(VIS_COLS);
        let peak = v[x..end].iter().cloned().fold(0.0f32, f32::max);
        for c in &mut v[x..end] {
            *c = peak;
        }
        x += pitch;
    }
}

/// Map the waveform to [`VIS_COLS`] oscilloscope column rows (0..VIS_H). Spreads the WHOLE sample
/// window across the columns (stride `n / VIS_COLS`), not just its first ~518 samples: the window
/// advances by roughly its own width each audio quantum, so mapping the whole window makes
/// consecutive frames contiguous (a continuous scroll) instead of disjoint snapshots with a ~10ms
/// gap between them, which reads as a choppy, low-fps scope. A sample scales to a row with
/// `round(sample*16) + centre`.
fn oscilloscope(samples: &[f32], out: &mut [u8; VIS_COLS]) {
    let n = samples.len();
    for (x, o) in out.iter_mut().enumerate() {
        let s = if n == 0 { 0.0 } else { samples[(x * n / VIS_COLS).min(n - 1)] };
        // `saturating_add` guards the +OSC_CENTER against overflow if a decoder ever hands us a
        // huge/non-finite sample (`as i32` saturates such a value to i32::MAX). Clamped to range.
        let y = ((s * 16.0).round() as i32).saturating_add(OSC_CENTER).clamp(0, sprites::VIS_H - 1);
        *o = y as u8;
    }
}

/// The oscilloscope colour index (0..5, into the 5-colour palette) for row `y`: brightest on the
/// centre line, dimming toward the edges (centre-out, matching classic Winamp).
fn osc_color_index(y: i32) -> usize {
    match y {
        6 | 7 => 0,
        4 | 5 | 8 | 9 => 1,
        2 | 3 | 10 | 11 => 2,
        0 | 1 | 12 | 13 => 3,
        _ => 4,
    }
}

/// In-place iterative radix-2 Cooley-Tukey FFT (decimation-in-time): a bit-reversal permutation
/// then log2(n) butterfly stages. `re`/`im` are the complex signal, length a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // Butterfly stages, twiddle by recurrence.
    let mut len = 2;
    while len <= n {
        let ang = -TAU / len as f32;
        let (wlr, wli) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let a = i + k;
                let b = a + len / 2;
                let tr = wr * re[b] - wi * im[b];
                let ti = wr * im[b] + wi * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nwr = wr * wlr - wi * wli;
                wi = wr * wli + wi * wlr;
                wr = nwr;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Round a bar/peak value to an integer pixel height clamped to `0..=max`.
fn round_clamp(v: f32, max: i32) -> i32 {
    (v.round() as i32).clamp(0, max)
}

/// Set one opaque pixel, bounds-checked against the framebuffer.
fn put(fb: &mut Framebuffer, x: i32, y: i32, c: Rgb) {
    if x < 0 || y < 0 || x as u32 >= fb.width || y as u32 >= fb.height {
        return;
    }
    let o = ((y as u32 * fb.width + x as u32) * 4) as usize;
    fb.rgba[o] = c.r;
    fb.rgba[o + 1] = c.g;
    fb.rgba[o + 2] = c.b;
    fb.rgba[o + 3] = 255;
}

/// Fill a rectangle with an opaque colour, bounds-checked.
fn fill(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
    for row in 0..h {
        for col in 0..w {
            put(fb, x + col, y + row, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(fb: &Framebuffer, x: i32, y: i32) -> [u8; 4] {
        let o = ((y as u32 * fb.width + x as u32) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    #[test]
    fn mode_cycles_bars_oscilloscope_off() {
        assert_eq!(VisMode::Bars.next(), VisMode::Oscilloscope);
        assert_eq!(VisMode::Oscilloscope.next(), VisMode::Off);
        assert_eq!(VisMode::Off.next(), VisMode::Bars);
        let mut s = VisState::default();
        assert_eq!(s.mode, VisMode::Bars);
        assert!(s.show_peaks, "classic peak dots are visible by default");
        s.cycle();
        assert_eq!(s.mode, VisMode::Oscilloscope);
        s.cycle();
        s.cycle();
        assert_eq!(s.mode, VisMode::Bars, "wraps");
    }

    #[test]
    fn fft_of_a_sine_peaks_at_its_bin() {
        // A pure cosine at bin k should concentrate magnitude in bin k.
        let k = 20usize;
        let mut re = [0.0f32; FFT_N];
        let mut im = [0.0f32; FFT_N];
        for (i, r) in re.iter_mut().enumerate() {
            *r = (TAU * k as f32 * i as f32 / FFT_N as f32).cos();
        }
        fft(&mut re, &mut im);
        let mag = |b: usize| (re[b] * re[b] + im[b] * im[b]).sqrt();
        let peak = mag(k);
        assert!(peak > 100.0, "strong response at the tone's bin (got {peak})");
        // Neighbouring and far bins are tiny by comparison.
        assert!(mag(k + 5) < peak * 0.05, "energy is concentrated, not smeared");
        assert!(mag(3) < peak * 0.05);
    }

    #[test]
    fn spectrum_of_silence_is_flat_zero() {
        let mut bars = [9.9f32; VIS_COLS];
        spectrum(&[0.0f32; FFT_N], &mut bars);
        assert!(bars.iter().all(|&b| b == 0.0), "silence yields empty bars");
    }

    #[test]
    fn spectrum_of_a_tone_lights_a_bar() {
        // A loud mid tone lights at least one bar substantially, and not every bar (it is not noise).
        let k = 60usize;
        let samples: Vec<f32> =
            (0..FFT_N).map(|i| 0.8 * (TAU * k as f32 * i as f32 / FFT_N as f32).cos()).collect();
        let mut bars = [0.0f32; VIS_COLS];
        spectrum(&samples, &mut bars);
        let max = bars.iter().cloned().fold(0.0f32, f32::max);
        assert!(max > 6.0, "the tone drives a tall bar (got {max})");
        let lit = bars.iter().filter(|&&b| b > max * 0.5).count();
        assert!(lit < VIS_COLS / 2, "energy is localized, not spread across every bar");
    }

    #[test]
    fn oscilloscope_maps_centre_and_extremes() {
        let mut scope = [0u8; VIS_COLS];
        // Silence sits on the centre row.
        oscilloscope(&[0.0f32; FFT_N], &mut scope);
        assert!(scope.iter().all(|&y| y as i32 == OSC_CENTER), "silence is the centre line");
        // A strong positive sample pushes below-centre rows (larger row index), clamped in range.
        let mut up = [0u8; VIS_COLS];
        oscilloscope(&[1.0f32; FFT_N], &mut up);
        assert!(up.iter().all(|&y| y == (sprites::VIS_H - 1) as u8), "full positive clamps to the bottom row");
        // A strong negative sample (every real waveform has troughs) clamps to the top row 0.
        let mut down = [0u8; VIS_COLS];
        oscilloscope(&[-1.0f32; FFT_N], &mut down);
        assert!(down.iter().all(|&y| y == 0), "full negative clamps to the top row");
    }

    #[test]
    fn advance_bars_rise_instantly_then_fall_gradually() {
        let mut s = VisState::default();
        let k = 60usize;
        let loud: Vec<f32> =
            (0..FFT_N).map(|i| 0.9 * (TAU * k as f32 * i as f32 / FFT_N as f32).cos()).collect();
        assert!(s.advance(&loud), "a tone animates the bars");
        let peak_col = (0..VIS_COLS).max_by(|&a, &b| s.bars[a].total_cmp(&s.bars[b])).unwrap();
        let high = s.bars[peak_col];
        assert!(high > 5.0, "bar rose to the tone");
        // Now feed silence: the bar falls by at most one falloff step per frame (gradual release).
        s.advance(&[0.0f32; FFT_N]);
        let after = s.bars[peak_col];
        assert!(after < high, "bar falls toward silence");
        let fall = bar_fall_px(s.bar_falloff);
        assert!(high - after <= fall + 0.001, "it falls gradually, not instantly");
    }

    #[test]
    fn advance_peaks_hang_then_fall_with_acceleration() {
        let mut s = VisState::default();
        // Seed a bar high, then let it collapse while the peak lingers.
        let k = 60usize;
        let loud: Vec<f32> =
            (0..FFT_N).map(|i| 0.9 * (TAU * k as f32 * i as f32 / FFT_N as f32).cos()).collect();
        s.advance(&loud);
        let col = (0..VIS_COLS).max_by(|&a, &b| s.bars[a].total_cmp(&s.bars[b])).unwrap();
        let seeded = s.peaks[col];
        assert!(seeded > 5.0, "peak seeded to the tone");
        // First silent frame: the peak descends (gravity is nonzero and the sign is down).
        s.advance(&[0.0f32; FFT_N]);
        let p1 = s.peaks[col];
        let drop1 = seeded - p1;
        assert!(drop1 > 0.0, "peak falls, not rises");
        // Second silent frame: it descends further, and by MORE than the first (accelerating).
        s.advance(&[0.0f32; FFT_N]);
        let drop2 = p1 - s.peaks[col];
        assert!(drop2 > drop1, "peak fall accelerates each frame");
        // Throughout, the peak stays at or above the (faster-)falling bar.
        assert!(s.peaks[col] >= s.bars[col] - 0.001, "peak hangs above the bar");
    }

    /// A viscolor palette with a distinct colour per role so a draw can be read back.
    fn test_palette() -> VisColor {
        let mut txt = String::new();
        for i in 0..24u8 {
            // role i -> colour (i, 100+i, 200) so each index is unique and identifiable.
            txt.push_str(&format!("{},{},200\n", i, 100 + i as u16));
        }
        VisColor::parse(&txt)
    }

    #[test]
    fn draw_bars_fill_the_gradient_bottom_up_with_gaps() {
        let vc = test_palette();
        // Default mode is Bars; force a full-height bar at column 0 (its wide group), rest at 0.
        let mut s = VisState::default();
        s.bars[0] = BAR_MAX;
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &vc, &s);
        // The bottom pixel of column 0 is the bottom gradient colour (role 17).
        let bottom = px(&fb, sprites::VIS_X, sprites::VIS_Y + sprites::VIS_H - 1);
        assert_eq!(bottom, [17, 117, 200, 255], "bottom row is the base gradient colour");
        // The top pixel is the hottest colour (role 2).
        let top = px(&fb, sprites::VIS_X, sprites::VIS_Y);
        assert_eq!(top, [2, 102, 200, 255], "top row is the hottest gradient colour");
        // Column 3 is a gap: it shows the background (role 0), not a bar.
        let gap = px(&fb, sprites::VIS_X + 3, sprites::VIS_Y + sprites::VIS_H - 1);
        assert_eq!(gap, [0, 100, 200, 255], "the 4th column of each group is a background gap");
    }

    #[test]
    fn group_wide_spreads_each_group_max() {
        // The wide bandwidth combines each group of 4 columns to the group MAX (so a narrow tone
        // lights its whole bar), not the average and not per-column.
        let mut v = [0.0f32; VIS_COLS];
        v[1] = 10.0; // one tall value inside the first group (columns 0..4)
        group_wide(&mut v, BandWidth::Thick);
        assert_eq!(&v[0..4], &[10.0, 10.0, 10.0, 10.0], "the group takes the max, not the average");
        assert_eq!(v[4], 0.0, "the next group is untouched");
    }

    #[test]
    fn draw_bars_render_the_falling_peak_dot() {
        let vc = test_palette();
        // A short bar (3px) with a high peak (12): the peak dot floats above the bar.
        let mut s = VisState::default();
        s.bars[0] = 3.0;
        s.peaks[0] = 12.0;
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &vc, &s);
        // Peak dot at row VIS_H - 12 = 4, in the peak colour (role 23).
        let prow = sprites::VIS_H - 12;
        assert_eq!(px(&fb, sprites::VIS_X, sprites::VIS_Y + prow), [23, 123, 200, 255], "peak dot is role 23");
        // The bar base is still the bottom gradient colour (role 17).
        assert_eq!(
            px(&fb, sprites::VIS_X, sprites::VIS_Y + sprites::VIS_H - 1),
            [17, 117, 200, 255],
            "bar base is role 17",
        );
    }

    #[test]
    fn draw_bars_hide_only_the_falling_peak_dot() {
        let vc = test_palette();
        let mut s = VisState {
            show_peaks: false,
            ..Default::default()
        };
        s.bars[0] = 3.0;
        s.peaks[0] = 12.0;
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &vc, &s);

        let peak_row = sprites::VIS_H - 12;
        assert_eq!(
            px(&fb, sprites::VIS_X, sprites::VIS_Y + peak_row),
            [0, 100, 200, 255],
            "hidden peak location remains background",
        );
        assert_eq!(
            px(&fb, sprites::VIS_X, sprites::VIS_Y + sprites::VIS_H - 1),
            [17, 117, 200, 255],
            "the spectrum bar remains visible",
        );
    }

    #[test]
    fn hidden_peaks_do_not_disable_bar_animation() {
        let mut s = VisState {
            show_peaks: false,
            ..Default::default()
        };
        let k = 60usize;
        let loud: Vec<f32> =
            (0..FFT_N).map(|i| 0.9 * (TAU * k as f32 * i as f32 / FFT_N as f32).cos()).collect();

        assert!(s.advance(&loud), "a tone still advances the spectrum");
        assert!(
            s.bars.iter().any(|&bar| bar > 5.0),
            "hidden peaks do not suppress bar motion"
        );
    }

    #[test]
    fn draw_oscilloscope_colours_the_centre_line() {
        let vc = test_palette();
        // Oscilloscope mode with the default flat centre line at OSC_CENTER.
        let s = VisState { mode: VisMode::Oscilloscope, ..Default::default() };
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &vc, &s);
        // The centre row uses the brightest oscilloscope colour (role 18).
        let centre = px(&fb, sprites::VIS_X, sprites::VIS_Y + OSC_CENTER);
        assert_eq!(centre, [18, 118, 200, 255], "centre line is oscilloscope colour 0 (role 18)");
    }

    #[test]
    fn draw_off_is_just_background() {
        let vc = test_palette();
        let mut s = VisState { mode: VisMode::Off, ..Default::default() };
        s.bars[0] = BAR_MAX; // would draw a bar if the mode were Bars
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &vc, &s);
        // Everything in the region is the background colour (role 0).
        for x in 0..sprites::VIS_W {
            assert_eq!(
                px(&fb, sprites::VIS_X + x, sprites::VIS_Y + 8),
                [0, 100, 200, 255],
                "off mode draws only the background",
            );
        }
    }

    const BG: [u8; 4] = [0, 100, 200, 255]; // test_palette role 0

    #[test]
    fn analyzer_styles_paint_distinguishable_pixels() {
        let vc = test_palette();
        let mut base = VisState {
            show_peaks: false,
            ..Default::default()
        };
        base.bars[0] = 12.0; // a tall bar in the first (drawn) column
        let render = |style: AnalyzerStyle| {
            let mut s = base.clone();
            s.analyzer_style = style;
            let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
            draw(&mut fb, &vc, &s);
            (0..sprites::VIS_H)
                .map(|row| px(&fb, sprites::VIS_X, sprites::VIS_Y + row))
                .collect::<Vec<_>>()
        };
        let normal = render(AnalyzerStyle::Normal);
        let fire = render(AnalyzerStyle::Fire);
        let line = render(AnalyzerStyle::Line);
        let painted = |v: &[[u8; 4]]| v.iter().filter(|&&p| p != BG).count();
        assert_eq!(painted(&line), 1, "Line draws only the top edge of the bar");
        assert!(painted(&normal) > 1, "Normal fills the whole bar");
        assert_ne!(normal, fire, "Fire and Normal colour the bar differently");
    }

    #[test]
    fn band_width_thin_and_thick_differ() {
        let vc = test_palette();
        let mut base = VisState {
            show_peaks: false,
            ..Default::default()
        };
        for b in base.bars.iter_mut() {
            *b = 12.0;
        }
        let bottom_px = |band: BandWidth, x: i32| {
            let mut s = base.clone();
            s.band_width = band;
            let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
            draw(&mut fb, &vc, &s);
            px(&fb, sprites::VIS_X + x, sprites::VIS_Y + sprites::VIS_H - 1)
        };
        // Column 1 is a bar under Thick (pitch 4) but a gap under Thin (pitch 2).
        assert_ne!(bottom_px(BandWidth::Thick, 1), BG, "Thick draws column 1");
        assert_eq!(bottom_px(BandWidth::Thin, 1), BG, "Thin leaves column 1 a gap");
    }

    #[test]
    fn oscilloscope_styles_differ() {
        let vc = test_palette();
        let mut base = VisState {
            mode: VisMode::Oscilloscope,
            ..Default::default()
        };
        for (x, v) in base.scope.iter_mut().enumerate() {
            *v = (x % sprites::VIS_H as usize) as u8; // a ramp so the styles diverge
        }
        let painted = |style: OscStyle| {
            let mut s = base.clone();
            s.osc_style = style;
            let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
            draw(&mut fb, &vc, &s);
            let mut n = 0;
            for x in 0..VIS_COLS {
                for row in 0..sprites::VIS_H {
                    if px(&fb, sprites::VIS_X + x as i32, sprites::VIS_Y + row) != BG {
                        n += 1;
                    }
                }
            }
            n
        };
        let dots = painted(OscStyle::Dots);
        let lines = painted(OscStyle::Lines);
        let solid = painted(OscStyle::Solid);
        assert!(dots < lines, "dots paint fewer pixels than a connected line");
        assert!(dots < solid, "dots paint fewer pixels than a solid fill");
        assert_ne!(lines, solid, "a connected line and a centre fill differ");
    }

    #[test]
    fn faster_falloff_drops_the_bar_more_per_frame() {
        let after_one_silent_frame = |falloff: u8| {
            let mut s = VisState {
                bar_falloff: falloff,
                ..Default::default()
            };
            for b in s.bars.iter_mut() {
                *b = 12.0;
            }
            s.advance(&[0.0f32; FFT_N]); // silence: bars fall one step
            s.bars[0]
        };
        assert!(
            after_one_silent_frame(9) < after_one_silent_frame(2),
            "a higher falloff speed drops the bar further in one frame"
        );
    }
}
