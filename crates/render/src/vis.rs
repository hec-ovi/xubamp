//! The main-window visualizer: a spectrum analyzer, an oscilloscope, or off, cycled by clicking
//! the region. It reads the recent mono output samples (tapped from the RT), and for the spectrum
//! runs a small hand-rolled radix-2 FFT (no external crate). Pure: samples plus the skin's
//! `viscolor` palette in, pixels out; the per-frame decay state lives in [`VisState`].
//!
//! The analyzer pipeline is a faithful port of the XMMS/Audacious classic-skins visualizer (GPL,
//! the reference the user pointed at for original-Winamp behaviour): a 512-sample `1-0.85cos`
//! window into a 512-point FFT, 256 normalized magnitude bins integrated into 19 (thick) or 75
//! (thin) logarithmic bands with the `bands/12` height fudge, a 40 dB range mapped onto the
//! 16-row region, the five classic analyzer/peak falloff speeds, and the three coloring styles
//! (normal by absolute row, fire from each bar's tip, line = the whole bar in its height's
//! color). The oscilloscope mirrors the same source: 0..=16 rows, banded colors, dot/line/solid.

use std::f32::consts::TAU;

use xubamp_skin::color::Rgb;
use xubamp_skin::sprites;
use xubamp_skin::viscolor::VisColor;

use crate::Framebuffer;

/// Drawn columns (the region is 76 wide; the 76th stays background).
pub const VIS_COLS: usize = 75;
/// FFT size over the recent samples: the classic analyzer works on 512-sample windows.
pub const FFT_N: usize = 512;
/// Usable magnitude bins (frequencies 1..=N/2).
const BINS: usize = FFT_N / 2;
/// Bar values run 0..=16 over the 16-row region (a value of 16 tops out the display).
const BAR_MAX: i32 = sprites::VIS_H;
/// The analyzer's dB range: the bottom of a bar is -40 dB, the top 0 dB.
const DB_RANGE: f32 = 40.0;
/// Oscilloscope centre row for a zero sample (`8 + round(sample*16)`).
const OSC_CENTER: i32 = 8;

/// The falloff sliders run 1 (slowest) to [`SPEED_MAX`] (fastest): the five classic speeds.
pub const SPEED_MAX: u8 = 5;

/// The tick the classic falloff speeds were tuned for: the original advanced its visualizer at
/// roughly 30fps, and [`AFALLOFF`]/[`PFALLOFF`] are per-tick amounts on that clock.
const REFERENCE_PERIOD_MS: f32 = 33.0;

/// Bar drop per frame (in 0..=16 units) for each falloff speed: XMMS/Audacious
/// `vis_afalloff_speeds`.
const AFALLOFF: [f32; 5] = [0.34, 0.5, 1.0, 1.3, 1.6];
/// Per-frame multiplier of the accelerating peak-drop speed: `vis_pfalloff_speeds`.
const PFALLOFF: [f32; 5] = [1.2, 1.3, 1.4, 1.5, 1.6];

fn afalloff(speed: u8) -> f32 {
    AFALLOFF[(speed.clamp(1, SPEED_MAX) - 1) as usize]
}

fn pfalloff(speed: u8) -> f32 {
    PFALLOFF[(speed.clamp(1, SPEED_MAX) - 1) as usize]
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
    /// How many analyzer bands: the classic 19 wide bars (3px + 1px gap) or 75 thin 1px bars.
    fn bands(self) -> usize {
        match self {
            BandWidth::Thick => 19,
            BandWidth::Thin => VIS_COLS,
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
    /// Redraw rate (1..=SPEED_MAX); not used by drawing, carried here so the window layer can pace
    /// the visualizer from one place.
    pub refresh_rate: u8,
    /// Per-band bar values, 0..=16 (19 used in Thick mode, all 75 in Thin).
    data: [f32; VIS_COLS],
    peaks: [f32; VIS_COLS],
    peak_speed: [f32; VIS_COLS],
    /// Oscilloscope row per column, 0..=16.
    scope: [u8; VIS_COLS],
}

/// The classic refresh-rate scale: speed 1..=10 maps onto periods stepping up to the original's
/// ~70fps ceiling (the fps number the Preferences slider shows).
///
/// ```
/// use xubamp_render::vis::{refresh_fps, refresh_period_ms};
/// assert_eq!(refresh_period_ms(10), 14); // ~70 fps at the top
/// assert_eq!(refresh_period_ms(1), 100); // 10 fps at the bottom
/// assert_eq!(refresh_fps(10), 71);
/// assert_eq!(refresh_fps(0), 10, "out-of-range clamps");
/// ```
pub fn refresh_period_ms(rate: u8) -> u64 {
    const PERIOD_MS: [u64; 10] = [100, 71, 59, 43, 33, 29, 25, 20, 17, 14];
    PERIOD_MS[(rate.clamp(1, 10) - 1) as usize]
}

/// The frame rate the Preferences slider displays for a refresh speed.
pub fn refresh_fps(rate: u8) -> u32 {
    (1000 / refresh_period_ms(rate)) as u32
}

impl Default for VisState {
    fn default() -> Self {
        VisState {
            mode: VisMode::default(),
            show_peaks: true,
            analyzer_style: AnalyzerStyle::default(),
            band_width: BandWidth::default(),
            osc_style: OscStyle::default(),
            // The middle of the five classic falloff speeds; user adjustable.
            bar_falloff: 3,
            peak_falloff: 3,
            refresh_rate: 8,
            data: [0.0; VIS_COLS],
            peaks: [0.0; VIS_COLS],
            peak_speed: [0.0; VIS_COLS],
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
    pub fn advance(&mut self, samples: &[f32], period_ms: f32) -> bool {
        // The classic falloff constants are per tick of the original's ~30fps vis timer. Our
        // timer runs anywhere from 10 to ~70fps, so scale each step by the actual frame period
        // or fast refresh rates would fall (and compound the peak speed) several times too fast,
        // which reads as glitching.
        let dt = (period_ms / REFERENCE_PERIOD_MS).clamp(0.1, 4.0);
        match self.mode {
            VisMode::Off => false,
            VisMode::Bars => {
                let bands = self.band_width.bands();
                let mut graph = [0.0f32; VIS_COLS];
                make_log_graph(samples, &mut graph[..bands]);
                let fall = afalloff(self.bar_falloff) * dt;
                let mult = pfalloff(self.peak_falloff).powf(dt);
                let mut changed = false;
                for (i, &target) in graph.iter().enumerate().take(bands) {
                    let (old_bar, old_peak) = (self.data[i], self.peaks[i]);
                    if target > self.data[i] {
                        // Rise instantly to the new magnitude.
                        self.data[i] = target;
                        if self.data[i] > self.peaks[i] {
                            self.peaks[i] = self.data[i];
                            self.peak_speed[i] = 0.01 * dt;
                        } else {
                            self.fall_peak(i, mult);
                        }
                    } else {
                        if self.data[i] > 0.0 {
                            self.data[i] = (self.data[i] - fall).max(0.0);
                        }
                        self.fall_peak(i, mult);
                    }
                    if self.data[i] != old_bar || self.peaks[i] != old_peak {
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

    /// One frame of the classic peak physics: the dot falls at an exponentially accelerating
    /// speed but never below the live bar under it.
    fn fall_peak(&mut self, i: usize, mult: f32) {
        if self.peaks[i] > 0.0 {
            self.peaks[i] -= self.peak_speed[i];
            self.peak_speed[i] *= mult;
            if self.peaks[i] < self.data[i] {
                self.peaks[i] = self.data[i];
            }
            if self.peaks[i] < 0.0 {
                self.peaks[i] = 0.0;
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
            let thick = state.band_width == BandWidth::Thick;
            for x in 0..VIS_COLS {
                if thick && (x & 3) == 3 {
                    continue; // the 1px gap between the classic wide bars
                }
                let band = if thick { x >> 2 } else { x };
                let bar = (state.data[band] as i32).clamp(0, BAR_MAX);
                if bar > 0 {
                    let top = h - bar; // screen row of the bar's tip (0 tops out)
                    match state.analyzer_style {
                        // Plain vertical gradient: each row's absolute colour.
                        AnalyzerStyle::Normal => {
                            for row in top..h {
                                put(fb, x0 + x as i32, y0 + row, grad[row.clamp(0, 15) as usize]);
                            }
                        }
                        // Flame: hottest at the bar's own tip, cooling toward its base.
                        AnalyzerStyle::Fire => {
                            for (from_tip, row) in (top..h).enumerate() {
                                put(fb, x0 + x as i32, y0 + row, grad[from_tip.min(15)]);
                            }
                        }
                        // Line: the whole bar in one colour, picked by its height (taller =
                        // hotter), the classic "vertical lines" style.
                        AnalyzerStyle::Line => {
                            let color = grad[top.clamp(0, 15) as usize];
                            for row in top..h {
                                put(fb, x0 + x as i32, y0 + row, color);
                            }
                        }
                    }
                }
                if state.show_peaks {
                    let pk = (state.peaks[band] as i32).clamp(0, BAR_MAX);
                    if pk > 0 {
                        put(fb, x0 + x as i32, y0 + (h - pk), peak);
                    }
                }
            }
        }
        VisMode::Oscilloscope => {
            let osc = vc.oscilloscope(); // 5 colours, centre-out
            let color = |row: i32| osc[SCOPE_ROW_COLOR[row.clamp(0, 15) as usize]];
            match state.osc_style {
                // Isolated dots: one pixel per column at the sample row.
                OscStyle::Dots => {
                    for x in 0..VIS_COLS {
                        let row = (state.scope[x] as i32).clamp(0, 15);
                        put(fb, x0 + x as i32, y0 + row, color(row));
                    }
                }
                // Connected line: span each column toward the NEXT one so the trace is
                // continuous, with the classic one-row trim so segments do not double up.
                OscStyle::Lines => {
                    for x in 0..VIS_COLS - 1 {
                        let mut row = (state.scope[x] as i32).clamp(0, 15);
                        let mut row2 = (state.scope[x + 1] as i32).clamp(0, 15);
                        if row < row2 {
                            row2 -= 1;
                        } else if row > row2 {
                            let tip = row;
                            row = row2 + 1;
                            row2 = tip;
                        }
                        for r in row..=row2 {
                            put(fb, x0 + x as i32, y0 + r, color(r));
                        }
                    }
                    let last = (state.scope[VIS_COLS - 1] as i32).clamp(0, 15);
                    put(fb, x0 + (VIS_COLS - 1) as i32, y0 + last, color(last));
                }
                // Solid: fill between the centre line and the sample, a filled waveform.
                OscStyle::Solid => {
                    for x in 0..VIS_COLS {
                        let sample = (state.scope[x] as i32).clamp(0, 15);
                        let (lo, hi) = if sample < OSC_CENTER {
                            (sample, OSC_CENTER)
                        } else {
                            (OSC_CENTER, sample)
                        };
                        for r in lo..=hi {
                            put(fb, x0 + x as i32, y0 + r, color(r));
                        }
                    }
                }
            }
        }
    }
}

/// Which of the five oscilloscope palette entries colours each of the 16 rows, centre-out (the
/// XMMS/Audacious `vis_scope_colors` table mapped onto the palette; its row-5 entry is a stray
/// analyzer index upstream, replaced by the symmetric value).
const SCOPE_ROW_COLOR: [usize; 16] = [4, 4, 3, 3, 2, 2, 1, 1, 0, 1, 1, 2, 2, 3, 3, 4];

/// The classic log-frequency analyzer graph: window the newest [`FFT_N`] samples, take the
/// magnitude spectrum, integrate the 256 bins into `out.len()` logarithmic bands (fractional
/// edges included, with the `bands/12` height fudge so every band count peaks alike), and map
/// each band's dB level onto the 0..=16 bar scale (-40 dB empty, 0 dB full).
fn make_log_graph(samples: &[f32], out: &mut [f32]) {
    let mut re = [0.0f32; FFT_N];
    let mut im = [0.0f32; FFT_N];
    let n = samples.len().min(FFT_N);
    let off = samples.len() - n;
    for (i, r) in re.iter_mut().enumerate().take(n) {
        // The classic `1 - 0.85cos` window.
        let w = 1.0 - 0.85 * (TAU * i as f32 / FFT_N as f32).cos();
        *r = samples[off + i] * w;
    }
    fft(&mut re, &mut im);

    // Normalized magnitudes for frequencies 1..=N/2; all but the last are doubled.
    let mut freq = [0.0f32; BINS];
    for (k, f) in freq.iter_mut().enumerate() {
        let bin = k + 1;
        let mag = (re[bin] * re[bin] + im[bin] * im[bin]).sqrt() / FFT_N as f32;
        *f = if bin < BINS { 2.0 * mag } else { mag };
    }

    let bands = out.len();
    let xscale = |i: usize| 256.0f32.powf(i as f32 / bands as f32) - 0.5;
    for (band, o) in out.iter_mut().enumerate() {
        let lo = xscale(band);
        let hi = xscale(band + 1);
        let a = lo.ceil() as i32;
        let b = hi.floor() as i32;
        let mut n = 0.0f32;
        if b < a {
            n += freq[b.clamp(0, BINS as i32 - 1) as usize] * (hi - lo);
        } else {
            if a > 0 {
                n += freq[(a - 1) as usize] * (a as f32 - lo);
            }
            for bin in a..b {
                n += freq[bin as usize];
            }
            if (b as usize) < BINS {
                n += freq[b as usize] * (hi - b as f32);
            }
        }
        // The same overall height no matter how many bands there are.
        n *= bands as f32 / 12.0;
        let db = 20.0 * n.max(1e-9).log10();
        let val = (1.0 + db / DB_RANGE) * BAR_MAX as f32;
        *o = (val as i32).clamp(0, BAR_MAX) as f32;
    }
}

/// Map the waveform to [`VIS_COLS`] oscilloscope rows. The whole sample window spreads across
/// the columns (the window advances by roughly its own width each audio quantum, so consecutive
/// frames read as a continuous scroll); each sample scales by the classic `8 + round(16 *
/// sample)`, clamped to the 0..=16 value range (the draw clamps rows to 0..=15).
fn oscilloscope(samples: &[f32], out: &mut [u8; VIS_COLS]) {
    let n = samples.len();
    for (x, o) in out.iter_mut().enumerate() {
        let s = if n == 0 {
            0.0
        } else {
            samples[(x * n / VIS_COLS).min(n - 1)]
        };
        // `saturating_add` guards the +OSC_CENTER against overflow if a decoder ever hands us a
        // huge/non-finite sample (`as i32` saturates such a value to i32::MAX).
        let val = ((s * 16.0).round() as i32).saturating_add(OSC_CENTER).clamp(0, 16);
        *o = val as u8;
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
    fn log_graph_of_silence_is_flat_zero() {
        let mut bands = [9.9f32; 19];
        make_log_graph(&[0.0f32; FFT_N], &mut bands);
        assert!(bands.iter().all(|&b| b == 0.0), "silence yields empty bands");
    }

    #[test]
    fn log_graph_of_a_tone_lights_a_band_on_both_widths() {
        // A loud mid tone lights at least one band substantially, and not every band.
        let k = 30usize;
        let samples: Vec<f32> =
            (0..FFT_N).map(|i| 0.8 * (TAU * k as f32 * i as f32 / FFT_N as f32).cos()).collect();
        for bands in [19usize, 75] {
            let mut graph = vec![0.0f32; bands];
            make_log_graph(&samples, &mut graph);
            let max = graph.iter().cloned().fold(0.0f32, f32::max);
            assert!(max > 8.0, "the tone drives a tall band (got {max} of 16, {bands} bands)");
            let lit = graph.iter().filter(|&&b| b > max * 0.5).count();
            assert!(lit < bands / 2, "energy is localized, not spread across every band");
        }
    }

    #[test]
    fn oscilloscope_maps_centre_and_extremes() {
        let mut scope = [0u8; VIS_COLS];
        // Silence sits on the centre row.
        oscilloscope(&[0.0f32; FFT_N], &mut scope);
        assert!(scope.iter().all(|&y| y as i32 == OSC_CENTER), "silence is the centre line");
        // A strong positive sample clamps to the classic 0..=16 value range's bottom.
        let mut up = [0u8; VIS_COLS];
        oscilloscope(&[1.0f32; FFT_N], &mut up);
        assert!(up.iter().all(|&y| y == 16), "full positive clamps to value 16");
        // A strong negative sample clamps to the top.
        let mut down = [0u8; VIS_COLS];
        oscilloscope(&[-1.0f32; FFT_N], &mut down);
        assert!(down.iter().all(|&y| y == 0), "full negative clamps to the top row");
    }

    #[test]
    fn falloff_is_frame_rate_independent() {
        // The same wall-clock time must fall the same distance whether it elapses as one 33ms
        // reference tick or as many fast 14ms (~70fps) frames; otherwise fast refresh rates
        // fall several times too fast (the reported "glitch" at high falloff + high fps).
        let k = 30usize;
        let loud: Vec<f32> =
            (0..FFT_N).map(|i| 0.9 * (TAU * k as f32 * i as f32 / FFT_N as f32).cos()).collect();

        let mut slow = VisState::default();
        slow.bar_falloff = 5;
        slow.advance(&loud, 33.0);
        let mut fast = slow.clone();

        // 56ms of silence: one long 56ms tick vs four fast 14ms (~70fps) frames.
        slow.advance(&[0.0f32; FFT_N], 56.0);
        for _ in 0..4 {
            fast.advance(&[0.0f32; FFT_N], 14.0);
        }
        let col = (0..VIS_COLS)
            .max_by(|&a, &b| slow.data[a].total_cmp(&slow.data[b]))
            .unwrap();
        assert!(
            (slow.data[col] - fast.data[col]).abs() < 0.01,
            "same elapsed time falls the same distance (slow {}, fast {})",
            slow.data[col],
            fast.data[col]
        );
    }

    #[test]
    fn advance_bars_rise_instantly_then_fall_gradually() {
        let mut s = VisState::default();
        let k = 60usize;
        let loud: Vec<f32> =
            (0..FFT_N).map(|i| 0.9 * (TAU * k as f32 * i as f32 / FFT_N as f32).cos()).collect();
        assert!(s.advance(&loud, 33.0), "a tone animates the bars");
        let peak_col = (0..VIS_COLS).max_by(|&a, &b| s.data[a].total_cmp(&s.data[b])).unwrap();
        let high = s.data[peak_col];
        assert!(high > 5.0, "bar rose to the tone");
        // Now feed silence: the bar falls by at most one falloff step per frame (gradual release).
        s.advance(&[0.0f32; FFT_N], 33.0);
        let after = s.data[peak_col];
        assert!(after < high, "bar falls toward silence");
        let fall = afalloff(s.bar_falloff);
        assert!(high - after <= fall + 0.001, "it falls gradually, not instantly");
    }

    #[test]
    fn advance_peaks_hang_then_fall_with_acceleration() {
        let mut s = VisState::default();
        // Seed a bar high, then let it collapse while the peak lingers.
        let k = 60usize;
        let loud: Vec<f32> =
            (0..FFT_N).map(|i| 0.9 * (TAU * k as f32 * i as f32 / FFT_N as f32).cos()).collect();
        s.advance(&loud, 33.0);
        let col = (0..VIS_COLS).max_by(|&a, &b| s.data[a].total_cmp(&s.data[b])).unwrap();
        let seeded = s.peaks[col];
        assert!(seeded > 5.0, "peak seeded to the tone");
        // First silent frame: the peak descends (gravity is nonzero and the sign is down).
        s.advance(&[0.0f32; FFT_N], 33.0);
        let p1 = s.peaks[col];
        let drop1 = seeded - p1;
        assert!(drop1 > 0.0, "peak falls, not rises");
        // Second silent frame: it descends further, and by MORE than the first (accelerating).
        s.advance(&[0.0f32; FFT_N], 33.0);
        let drop2 = p1 - s.peaks[col];
        assert!(drop2 > drop1, "peak fall accelerates each frame");
        // Throughout, the peak stays at or above the (faster-)falling bar.
        assert!(s.peaks[col] >= s.data[col] - 0.001, "peak hangs above the bar");
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
        s.data[0] = BAR_MAX as f32;
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
    fn thick_mode_draws_nineteen_wide_bars_from_their_bands() {
        // Thick mode has 19 real bands; band b paints columns 4b..4b+3 with a gap at 4b+3.
        let vc = test_palette();
        let mut s = VisState::default();
        s.data[1] = BAR_MAX as f32; // band 1 -> columns 4..=6 lit, column 7 gap
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &vc, &s);
        let base = sprites::VIS_Y + sprites::VIS_H - 1;
        assert_eq!(px(&fb, sprites::VIS_X + 4, base), [17, 117, 200, 255], "band 1 first column");
        assert_eq!(px(&fb, sprites::VIS_X + 6, base), [17, 117, 200, 255], "band 1 last column");
        assert_eq!(px(&fb, sprites::VIS_X + 7, base), [0, 100, 200, 255], "gap column");
        assert_eq!(px(&fb, sprites::VIS_X, base), [0, 100, 200, 255], "band 0 untouched");
    }

    #[test]
    fn line_style_paints_the_whole_bar_in_its_height_color() {
        let vc = test_palette();
        let mut s = VisState {
            analyzer_style: AnalyzerStyle::Line,
            ..Default::default()
        };
        s.data[0] = 4.0; // a short bar: rows 12..=15, all in the color of row 12 (role 2+12)
        let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
        draw(&mut fb, &vc, &s);
        let expect = [14, 114, 200, 255];
        for row in 12..16 {
            assert_eq!(px(&fb, sprites::VIS_X, sprites::VIS_Y + row), expect, "row {row} single color");
        }
    }

    #[test]
    fn draw_bars_render_the_falling_peak_dot() {
        let vc = test_palette();
        // A short bar (3px) with a high peak (12): the peak dot floats above the bar.
        let mut s = VisState::default();
        s.data[0] = 3.0;
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
        s.data[0] = 3.0;
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

        assert!(s.advance(&loud, 33.0), "a tone still advances the spectrum");
        assert!(
            s.data.iter().any(|&bar| bar > 5.0),
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
        s.data[0] = BAR_MAX as f32; // would draw a bar if the mode were Bars
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
        base.data[0] = 12.0; // a tall bar in the first (drawn) column
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
        assert_eq!(painted(&line), 12, "Line fills the whole bar");
        let line_colors: std::collections::HashSet<[u8; 4]> =
            line.iter().copied().filter(|&p| p != BG).collect();
        assert_eq!(line_colors.len(), 1, "Line uses one colour for the whole bar");
        let normal_colors: std::collections::HashSet<[u8; 4]> =
            normal.iter().copied().filter(|&p| p != BG).collect();
        assert!(normal_colors.len() > 1, "Normal is a gradient");
        assert_ne!(normal, fire, "Fire and Normal colour the bar differently");
    }

    #[test]
    fn band_width_thin_and_thick_differ() {
        let vc = test_palette();
        let mut base = VisState {
            show_peaks: false,
            ..Default::default()
        };
        for b in base.data.iter_mut() {
            *b = 12.0;
        }
        let bottom_px = |band: BandWidth, x: i32| {
            let mut s = base.clone();
            s.band_width = band;
            let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
            draw(&mut fb, &vc, &s);
            px(&fb, sprites::VIS_X + x, sprites::VIS_Y + sprites::VIS_H - 1)
        };
        // Column 3 is the classic gap under Thick (19 wide bars) but drawn under Thin (75
        // 1px bands, no gaps).
        assert_eq!(bottom_px(BandWidth::Thick, 3), BG, "Thick leaves column 3 a gap");
        assert_ne!(bottom_px(BandWidth::Thin, 3), BG, "Thin draws every column");
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
            for b in s.data.iter_mut() {
                *b = 12.0;
            }
            s.advance(&[0.0f32; FFT_N], 33.0); // silence: bars fall one step
            s.data[0]
        };
        assert!(
            after_one_silent_frame(9) < after_one_silent_frame(2),
            "a higher falloff speed drops the bar further in one frame"
        );
    }
}
