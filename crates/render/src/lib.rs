//! Software sprite compositor.
//!
//! The whole classic UI is bitmap sprites blitted into one CPU framebuffer, which the
//! `wl` crate then hands to the compositor as a `wl_shm` buffer. This crate is pure: a
//! `Framebuffer`, a clipping `blit`, and window-composition functions. No platform code,
//! no allocation per blit beyond the single framebuffer.

use xubamp_skin::bmp::Image;
use xubamp_skin::sprites::{self, Placement, Rect};
use xubamp_skin::{font, textfont, Skin};

pub mod adwaita;
pub mod equalizer;
pub mod hit;
pub mod jump;
pub mod marquee;
pub mod menu;
pub mod pledit;
pub mod posbar;
pub mod preferences;
pub mod shade;
pub mod slider;
pub mod vis;

/// A top-down `RGBA8888` framebuffer, 4 bytes per pixel.
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, top-down.
    pub rgba: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; width as usize * height as usize * 4],
        }
    }

    /// The raw pixel bytes, for upload into a `wl_shm` buffer.
    pub fn as_bytes(&self) -> &[u8] {
        &self.rgba
    }
}

/// Copy `rect` from `src` into `fb` at (`dst_x`, `dst_y`), opaque, clipped to both the
/// source image and the destination framebuffer. Regions outside either are skipped, so
/// off-screen or oversized placements never panic.
pub fn blit(fb: &mut Framebuffer, src: &Image, rect: Rect, dst_x: i32, dst_y: i32) {
    for row in 0..rect.h {
        let sy = rect.y + row;
        let dy = dst_y + row;
        if sy < 0 || dy < 0 || sy as u32 >= src.height || dy as u32 >= fb.height {
            continue;
        }
        for col in 0..rect.w {
            let sx = rect.x + col;
            let dx = dst_x + col;
            if sx < 0 || dx < 0 || sx as u32 >= src.width || dx as u32 >= fb.width {
                continue;
            }
            let s_off = ((sy as u32 * src.width + sx as u32) * 4) as usize;
            let d_off = ((dy as u32 * fb.width + dx as u32) * 4) as usize;
            fb.rgba[d_off..d_off + 4].copy_from_slice(&src.rgba[s_off..s_off + 4]);
        }
    }
}

pub(crate) fn blit_placement(fb: &mut Framebuffer, sheet: &Image, p: Placement) {
    blit(fb, sheet, p.src, p.dst_x, p.dst_y);
}

/// Split whole seconds into the four MM:SS digit values (tens then units of minutes, then of
/// seconds) for the time display. Minutes saturate at 99 so the two-digit field never
/// overflows; the classic display has no room to show more.
pub fn mmss_digits(secs: u32) -> [u8; 4] {
    let mins = (secs / 60).min(99);
    let s = secs % 60;
    [
        (mins / 10) as u8,
        (mins % 10) as u8,
        (s / 10) as u8,
        (s % 10) as u8,
    ]
}

/// Compose the main window (275x116): the MAIN background, the active title bar, then the
/// six transport buttons, drawing the pressed sprite for whichever button `state` reports as
/// held. Missing sheets are simply skipped (their pixels stay whatever the lower layer left),
/// which is the default-skin fallback point.
pub fn compose_main_window(skin: &Skin, state: &hit::UiState) -> Framebuffer {
    // Collapsed (windowshade) mode is just the title strip, composed separately.
    if state.shade {
        return shade::compose(skin, state);
    }
    let mut fb = Framebuffer::new(sprites::MAIN_W as u32, sprites::MAIN_H as u32);
    if let Some(main) = &skin.main {
        blit_placement(&mut fb, main, sprites::MAIN_BG);
    }
    if let Some(titlebar) = &skin.titlebar {
        blit_placement(&mut fb, titlebar, sprites::TITLEBAR_ACTIVE);
        // A held title-bar button shows its pressed sprite over the strip (its up graphic is
        // already baked into the strip).
        if let Some(b) = state.pressed_title {
            let idx = hit::TITLE_BUTTON_ORDER
                .iter()
                .position(|&t| t == b)
                .unwrap();
            blit_placement(&mut fb, titlebar, sprites::TITLE_BUTTONS_PRESSED[idx]);
        }
    }
    if let Some(cbuttons) = &skin.cbuttons {
        for ((normal, pressed), id) in sprites::CBUTTONS
            .iter()
            .zip(sprites::CBUTTONS_PRESSED.iter())
            .zip(hit::TRANSPORT_ORDER)
        {
            let placement = if state.pressed == Some(id) {
                *pressed
            } else {
                *normal
            };
            blit_placement(&mut fb, cbuttons, placement);
        }
    }
    // Time display: four digits from the number sheet, but only while the selected value is
    // available. Remaining mode derives duration - elapsed and adds the skin's classic minus
    // indicator. With nothing loaded (or no duration for a countdown), the display stays blank.
    if let (Some(numbers), Some(secs)) = (&skin.numbers, state.displayed_time()) {
        if state.time_display == hit::TimeDisplay::Remaining {
            draw_time_minus(&mut fb, numbers);
        }
        for (&(dx, dy), &d) in sprites::TIME_DIGITS.iter().zip(mmss_digits(secs).iter()) {
            blit(&mut fb, numbers, sprites::DIGITS[d as usize], dx, dy);
        }
    }
    // Song-title marquee: drawn from the skin's text.bmp font over the display panel. While a
    // volume/balance slider is being dragged it instead shows that value ("Volume: 78%",
    // "Balance: Center"/"Balance: 12% Left"), matching classic Winamp (verified against Webamp's
    // marqueeUtils). Skins without text.bmp (the built-in default) show no marquee here.
    if let Some(text) = &skin.text {
        match state.dragging {
            Some(hit::Slider::Volume) => {
                marquee::draw(&mut fb, text, &format!("Volume: {}%", state.volume), 0);
            }
            Some(hit::Slider::Balance) => {
                marquee::draw(&mut fb, text, &balance_readout(state.balance), 0);
            }
            _ => marquee::draw(
                &mut fb,
                text,
                &state.title,
                if state.scroll_title {
                    state.marquee_offset
                } else {
                    0
                },
            ),
        }
    }
    // Volume and balance sliders: each drawn from its own sheet at the current value, with the
    // thumb shown pressed while that slider is being dragged. Skins without the sheet skip it.
    if let Some(volume) = &skin.volume {
        let held = state.dragging == Some(hit::Slider::Volume);
        slider::draw_volume(&mut fb, volume, state.volume, held);
    }
    // Balance slider. Skins without balance.bmp (this dev skin is one) fall back to the volume
    // sheet, which shares the slider layout, so the pan control is still visible and draggable (its
    // art then matches the volume slider rather than showing a centre-out bar).
    if let Some(balance) = skin.balance.as_ref().or(skin.volume.as_ref()) {
        let held = state.dragging == Some(hit::Slider::Balance);
        slider::draw_balance(&mut fb, balance, state.balance, held);
    }
    // Position (seek) bar: the groove and a thumb at the current playback position (0 when nothing
    // is loaded), drawn pressed while the user scrubs. Skins without posbar.bmp show the main
    // background groove instead.
    if let Some(posbar) = &skin.posbar {
        let held = state.dragging == Some(hit::Slider::Position);
        posbar::draw(&mut fb, posbar, state.position.unwrap_or(0.0), held);
    }
    // EQ and PL toggle buttons (from shufrep.bmp): lit while their window is open, pressed while
    // held. Skins without shufrep.bmp show nothing here.
    if let Some(shufrep) = &skin.shufrep {
        let held = |t| state.pressed_toggle == Some(t);
        let eq = match (state.eq_open, held(hit::WindowToggle::Equalizer)) {
            (false, false) => sprites::EQ_OFF,
            (false, true) => sprites::EQ_OFF_PRESSED,
            (true, false) => sprites::EQ_ON,
            (true, true) => sprites::EQ_ON_PRESSED,
        };
        let pl = match (state.pl_open, held(hit::WindowToggle::Playlist)) {
            (false, false) => sprites::PL_OFF,
            (false, true) => sprites::PL_OFF_PRESSED,
            (true, false) => sprites::PL_ON,
            (true, true) => sprites::PL_ON_PRESSED,
        };
        blit_placement(&mut fb, shufrep, eq);
        blit_placement(&mut fb, shufrep, pl);
        // Shuffle + repeat mode buttons: lit while the mode is on, pressed while held.
        let held_mode = |m| state.pressed_mode == Some(m);
        let shuffle = match (state.shuffle_on, held_mode(hit::ModeButton::Shuffle)) {
            (false, false) => sprites::SHUFFLE_OFF,
            (false, true) => sprites::SHUFFLE_OFF_PRESSED,
            (true, false) => sprites::SHUFFLE_ON,
            (true, true) => sprites::SHUFFLE_ON_PRESSED,
        };
        let repeat = match (state.repeat_on, held_mode(hit::ModeButton::Repeat)) {
            (false, false) => sprites::REPEAT_OFF,
            (false, true) => sprites::REPEAT_OFF_PRESSED,
            (true, false) => sprites::REPEAT_ON,
            (true, true) => sprites::REPEAT_ON_PRESSED,
        };
        blit_placement(&mut fb, shufrep, shuffle);
        blit_placement(&mut fb, shufrep, repeat);
    }
    // kbps (bitrate) and kHz (sample rate) readouts, in the small text.bmp font, blank when nothing
    // is loaded. They share the marquee's font sheet.
    if let Some(text) = &skin.text {
        if let Some(kbps) = state.kbps {
            draw_small_number(
                &mut fb,
                text,
                kbps,
                sprites::KBPS_X,
                sprites::KBPS_Y,
                sprites::KBPS_DIGITS,
            );
        }
        if let Some(khz) = state.khz {
            draw_small_number(
                &mut fb,
                text,
                khz,
                sprites::KHZ_X,
                sprites::KHZ_Y,
                sprites::KHZ_DIGITS,
            );
        }
    }
    // Mono/stereo indicator: both words are drawn; the one matching the channel count is lit, the
    // other dim. Nothing loaded (0 channels) dims both.
    if let Some(monoster) = &skin.monoster {
        let (mono, stereo) = if state.channels == 1 {
            (sprites::MONO_LIT, sprites::STEREO_UNLIT)
        } else if state.channels >= 2 {
            (sprites::MONO_UNLIT, sprites::STEREO_LIT)
        } else {
            (sprites::MONO_UNLIT, sprites::STEREO_UNLIT)
        };
        blit_placement(&mut fb, monoster, mono);
        blit_placement(&mut fb, monoster, stereo);
    }
    // The visualizer: spectrum bars, oscilloscope, or off, over the recessed panel, coloured from
    // viscolor.txt. The built-in default skin now ships the classic palette so its visualizer
    // animates too; skins with no palette at all still show nothing here.
    if let Some(viscolor) = &skin.viscolor {
        vis::draw(&mut fb, viscolor, &state.vis);
    }
    // Procedural feedback for skins that bake a static window and ship no overlay sheets (the
    // built-in default). Each branch fires only when its sheet is absent, so real skins are
    // untouched: their sheets are Some and this is skipped entirely.
    draw_base_fallbacks(&mut fb, skin, state);
    fb
}

/// The built-in default skin bakes the whole window into one `main` image and ships no overlay
/// sheets, so `compose_main_window` would otherwise draw no pressed buttons, no moving slider
/// thumbs, and no clock: it reads as "the controls stopped working". This draws that dynamic state
/// procedurally in the base skin's cyan palette. Every element is guarded on its sheet being None,
/// so a real skin (sheets present) never reaches here.
fn draw_base_fallbacks(fb: &mut Framebuffer, skin: &Skin, state: &hit::UiState) {
    const CYAN: [u8; 3] = [0, 216, 240];
    // A held transport button: darken its footprint so the press is visible.
    if skin.cbuttons.is_none() {
        if let Some(id) = state.pressed {
            if let Some((placement, _)) = sprites::CBUTTONS
                .iter()
                .zip(hit::TRANSPORT_ORDER)
                .find(|(_, t)| *t == id)
            {
                darken_rect(
                    fb,
                    placement.dst_x,
                    placement.dst_y,
                    placement.src.w,
                    placement.src.h,
                );
            }
        }
    }
    // The clock, in the built-in 5x7 font at the classic digit slots.
    if skin.numbers.is_none() {
        if let Some(secs) = state.displayed_time() {
            for (&(dx, dy), &digit) in sprites::TIME_DIGITS.iter().zip(mmss_digits(secs).iter()) {
                font::draw_text(
                    &mut fb.rgba,
                    fb.width,
                    fb.height,
                    dx,
                    dy,
                    &digit.to_string(),
                    CYAN,
                );
            }
        }
    }
    // Slider thumbs, drawn as a bright bar at the value so a drag is visible.
    if skin.volume.is_none() {
        let frac = state.volume as f32 / 100.0;
        fallback_thumb(fb, sprites::VOLUME_X, sprites::VOLUME_Y, sprites::VOLUME_W, frac);
    }
    if skin.balance.is_none() && skin.volume.is_none() {
        let frac = (state.balance as f32 + 100.0) / 200.0;
        fallback_thumb(fb, sprites::BALANCE_X, sprites::BALANCE_Y, sprites::BALANCE_W, frac);
    }
    if skin.posbar.is_none() {
        let frac = state.position.unwrap_or(0.0).clamp(0.0, 1.0);
        fallback_thumb(fb, sprites::POSBAR_X, sprites::POSBAR_Y, sprites::POSBAR_W, frac);
    }
}

/// Draw a 4px bright thumb bar at `frac` (0..=1) across a slider track, for the base skin.
fn fallback_thumb(fb: &mut Framebuffer, track_x: i32, track_y: i32, track_w: i32, frac: f32) {
    const THUMB_W: i32 = 4;
    const THUMB_H: i32 = 11;
    const LIGHT: [u8; 3] = [210, 236, 244];
    let x = track_x + ((frac.clamp(0.0, 1.0)) * (track_w - THUMB_W) as f32).round() as i32;
    for yy in track_y + 1..track_y + 1 + THUMB_H {
        for xx in x..x + THUMB_W {
            put_rgb(fb, xx, yy, LIGHT);
        }
    }
}

/// Multiply an axis-aligned rectangle toward black to show a pressed control on the base skin.
fn darken_rect(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32) {
    for yy in y.max(0)..(y + h).min(fb.height as i32) {
        for xx in x.max(0)..(x + w).min(fb.width as i32) {
            let o = ((yy as u32 * fb.width + xx as u32) * 4) as usize;
            for c in 0..3 {
                fb.rgba[o + c] = (fb.rgba[o + c] as u32 * 9 / 16) as u8;
            }
        }
    }
}

/// Set one opaque RGB pixel, clipped to the framebuffer.
fn put_rgb(fb: &mut Framebuffer, x: i32, y: i32, color: [u8; 3]) {
    if x < 0 || y < 0 || x >= fb.width as i32 || y >= fb.height as i32 {
        return;
    }
    let o = ((y as u32 * fb.width + x as u32) * 4) as usize;
    fb.rgba[o..o + 3].copy_from_slice(&color);
    fb.rgba[o + 3] = 255;
}

/// Draw the remaining-time sign from either classic number-sheet layout. `NUMBERS.BMP` stores the
/// visible 5x1 line inside its digit atlas; `NUMS_EX.BMP` appends a complete 9x13 sign cell. The
/// decoded [`Skin`] deliberately exposes one number image for both, so the sheet width identifies
/// the extended form.
fn draw_time_minus(fb: &mut Framebuffer, numbers: &Image) {
    const STANDARD_MINUS: Rect = Rect::new(20, 6, 5, 1);
    const EXTENDED_MINUS: Rect = Rect::new(99, 0, 9, 13);
    const MINUS_X: i32 = 38;
    const TIME_Y: i32 = 26;

    if numbers.width >= 108 && numbers.height >= 13 {
        blit(fb, numbers, EXTENDED_MINUS, MINUS_X, TIME_Y);
    } else {
        blit(fb, numbers, STANDARD_MINUS, MINUS_X, TIME_Y + 6);
    }
}

/// Draw `value` as small `text.bmp` digits, left-aligned at (`x`, `y`) and clipped to `max_digits`
/// (matching the classic fixed-width field). Non-digit chars are skipped.
fn draw_small_number(
    fb: &mut Framebuffer,
    text: &Image,
    value: u32,
    x: i32,
    y: i32,
    max_digits: usize,
) {
    for (i, ch) in value.to_string().chars().take(max_digits).enumerate() {
        if let Some(cell) = textfont::cell(ch) {
            blit(fb, text, cell, x + i as i32 * textfont::ADVANCE, y);
        }
    }
}

/// The classic Winamp balance readout shown in the marquee while dragging the balance slider:
/// "Balance: Center" at centre, else "Balance: NN% Left"/"Right" by magnitude (verified against
/// Webamp's `getBalanceText`).
fn balance_readout(balance: i8) -> String {
    if balance == 0 {
        "Balance: Center".to_string()
    } else {
        let dir = if balance > 0 { "Right" } else { "Left" };
        format!("Balance: {}% {}", (balance as i32).abs(), dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Image {
        Image {
            width: w,
            height: h,
            rgba: rgba
                .iter()
                .copied()
                .cycle()
                .take(w as usize * h as usize * 4)
                .collect(),
        }
    }

    fn px(fb: &Framebuffer, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * fb.width + x) * 4) as usize;
        [fb.rgba[o], fb.rgba[o + 1], fb.rgba[o + 2], fb.rgba[o + 3]]
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];

    #[test]
    fn compose_fills_from_main_background() {
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!((fb.width, fb.height), (275, 116));
        assert_eq!(px(&fb, 0, 0), RED);
        assert_eq!(px(&fb, 274, 115), RED);
    }

    #[test]
    fn compose_collapses_to_the_title_strip_in_shade_mode() {
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            titlebar: Some(solid(344, 87, GREEN)),
            ..Default::default()
        };
        // Shade mode dispatches to the compact strip (275x14); expanded is the full window.
        let shaded = compose_main_window(
            &skin,
            &hit::UiState {
                shade: true,
                ..Default::default()
            },
        );
        assert_eq!(
            (shaded.width, shaded.height),
            (275, 14),
            "shade collapses to the strip"
        );
        let full = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(
            (full.width, full.height),
            (275, 116),
            "expanded is the full window"
        );
    }

    #[test]
    fn transport_buttons_land_on_their_rects() {
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            cbuttons: Some(solid(136, 36, GREEN)),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &hit::UiState::default());
        // Play button occupies dst x 39..62, y 88..106.
        assert_eq!(px(&fb, 39, 88), GREEN, "play top-left");
        assert_eq!(px(&fb, 61, 105), GREEN, "play bottom-right");
        // Away from any button the main background shows through.
        assert_eq!(px(&fb, 200, 40), RED);
        assert_eq!(px(&fb, 0, 0), RED);
    }

    #[test]
    fn pressed_button_draws_from_the_bottom_row() {
        // A cbuttons sheet split top/bottom: normal row (y 0..18) BLUE, pressed row WHITE.
        let mut sheet = solid(136, 36, [0, 0, 255, 255]); // BLUE top
        for y in 18..36 {
            for x in 0..136 {
                let o = ((y * 136 + x) * 4) as usize;
                sheet.rgba[o..o + 4].copy_from_slice(&[255, 255, 255, 255]); // WHITE bottom
            }
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            cbuttons: Some(sheet),
            ..Default::default()
        };
        let state = hit::UiState {
            pressed: Some(hit::Transport::Play),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &state);
        // Play (dst 39,88) is pressed -> sampled from the WHITE bottom row.
        assert_eq!(
            px(&fb, 39 + 11, 88 + 9),
            [255, 255, 255, 255],
            "play pressed"
        );
        // Stop (dst 85,88) is not pressed -> still the BLUE normal row.
        assert_eq!(px(&fb, 85 + 11, 88 + 9), [0, 0, 255, 255], "stop normal");
    }

    #[test]
    fn pressed_title_button_draws_its_down_sprite() {
        // A title-bar sheet all BLUE, with the close DOWN sprite (18,9,9,9) painted WHITE.
        let mut sheet = solid(344, 87, [0, 0, 255, 255]);
        for y in 9..18u32 {
            for x in 18..27u32 {
                let o = ((y * 344 + x) * 4) as usize;
                sheet.rgba[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            titlebar: Some(sheet),
            ..Default::default()
        };
        // Idle: the close area shows the (blue) strip, no pressed sprite.
        let idle = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(
            px(&idle, 264 + 4, 3 + 4),
            [0, 0, 255, 255],
            "close area is the strip when idle"
        );
        // Held: the WHITE down sprite is drawn at the close destination (264,3,9,9).
        let state = hit::UiState {
            pressed_title: Some(hit::TitleButton::Close),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &state);
        assert_eq!(
            px(&fb, 264 + 4, 3 + 4),
            [255, 255, 255, 255],
            "held close shows its pressed sprite"
        );
    }

    #[test]
    fn mmss_digits_split_and_clamp() {
        assert_eq!(mmss_digits(0), [0, 0, 0, 0]);
        assert_eq!(mmss_digits(65), [0, 1, 0, 5]); // 01:05
        assert_eq!(mmss_digits(3599), [5, 9, 5, 9]); // 59:59
        assert_eq!(mmss_digits(6000), [9, 9, 0, 0]); // 100:00 clamps to 99:00
        assert_eq!(mmss_digits(600_000), [9, 9, 0, 0]); // far past the cap, still 99:xx
    }

    #[test]
    fn time_display_draws_the_elapsed_digits() {
        // A number sheet where digit d's 9px cell is a d-distinct red, so we can read back
        // which digit landed where. (Digit 0 is (10,0,0), digit 5 is (110,0,0), etc.)
        let mut numbers = solid(99, 13, [0, 0, 0, 255]);
        for d in 0..10u32 {
            let color = [(10 + d * 20) as u8, 0, 0, 255];
            for y in 0..13u32 {
                for x in d * 9..d * 9 + 9 {
                    let o = ((y * 99 + x) * 4) as usize;
                    numbers.rgba[o..o + 4].copy_from_slice(&color);
                }
            }
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            numbers: Some(numbers),
            ..Default::default()
        };
        let state = hit::UiState {
            elapsed: Some(65), // 01:05 -> digits [0, 1, 0, 5]
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &state);
        for (&(dx, dy), &d) in xubamp_skin::sprites::TIME_DIGITS
            .iter()
            .zip([0u32, 1, 0, 5].iter())
        {
            let want = [(10 + d * 20) as u8, 0, 0, 255];
            let (cx, cy) = (dx as u32 + 4, dy as u32 + 6); // sample a pixel inside the cell
            assert_eq!(px(&fb, cx, cy), want, "digit {d} at ({dx},{dy})");
        }

        // With no elapsed time the slots stay blank: the main background shows through.
        let blank = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(
            px(&blank, 48 + 4, 26 + 6),
            RED,
            "blank display draws no digit"
        );
    }

    #[test]
    fn remaining_time_draws_duration_minus_elapsed_and_the_standard_minus_sign() {
        // Standard NUMBERS.BMP stores the visible minus as a 5x1 crop at (20,6). Give every digit
        // a readable colour, then make that source line green so its exact destination is visible.
        let mut numbers = solid(99, 13, [0, 0, 0, 255]);
        for d in 0..10u32 {
            let color = [(10 + d * 20) as u8, 0, 0, 255];
            for y in 0..13u32 {
                for x in d * 9..d * 9 + 9 {
                    let o = ((y * numbers.width + x) * 4) as usize;
                    numbers.rgba[o..o + 4].copy_from_slice(&color);
                }
            }
        }
        for x in 20..25u32 {
            let o = ((6 * numbers.width + x) * 4) as usize;
            numbers.rgba[o..o + 4].copy_from_slice(&GREEN);
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            numbers: Some(numbers),
            ..Default::default()
        };
        let state = hit::UiState {
            time_display: hit::TimeDisplay::Remaining,
            elapsed: Some(135),
            duration: Some(200), // 200 - 135 = 65 -> 01:05
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &state);
        for (&(dx, dy), &d) in sprites::TIME_DIGITS.iter().zip([0u32, 1, 0, 5].iter()) {
            let want = [(10 + d * 20) as u8, 0, 0, 255];
            assert_eq!(
                px(&fb, dx as u32 + 4, dy as u32 + 5),
                want,
                "remaining digit {d}"
            );
        }
        assert_eq!(px(&fb, 38, 32), GREEN, "minus starts at x=38, y=32");
        assert_eq!(px(&fb, 42, 32), GREEN, "standard minus is five pixels wide");
        assert_eq!(
            px(&fb, 43, 32),
            RED,
            "pixel after the standard minus is untouched"
        );
        assert_eq!(px(&fb, 38, 31), RED, "standard minus is one pixel tall");

        let unknown = compose_main_window(
            &skin,
            &hit::UiState {
                duration: None,
                ..state
            },
        );
        assert_eq!(px(&unknown, 38, 32), RED, "unknown countdown has no sign");
        assert_eq!(
            px(&unknown, 48 + 4, 26 + 6),
            RED,
            "unknown countdown has no digits"
        );
    }

    #[test]
    fn remaining_time_uses_the_extended_minus_cell_and_saturates_at_zero() {
        // NUMS_EX.BMP appends a complete 9x13 minus cell at x=99.
        let mut numbers = solid(108, 13, [0, 0, 0, 255]);
        for y in 0..13u32 {
            for x in 99..108u32 {
                let o = ((y * numbers.width + x) * 4) as usize;
                numbers.rgba[o..o + 4].copy_from_slice(&GREEN);
            }
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            numbers: Some(numbers),
            ..Default::default()
        };
        let state = hit::UiState {
            time_display: hit::TimeDisplay::Remaining,
            elapsed: Some(101),
            duration: Some(100),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &state);
        assert_eq!(px(&fb, 38 + 4, 26 + 6), GREEN, "full extended sign cell");
        for &(dx, dy) in &sprites::TIME_DIGITS {
            assert_eq!(
                px(&fb, dx as u32 + 4, dy as u32 + 6),
                [0, 0, 0, 255],
                "elapsed beyond duration renders saturated 00:00"
            );
        }
    }

    #[test]
    fn kbps_and_khz_draw_small_text_font_digits() {
        // A text sheet where digit d's 5x6 cell (at x=d*5, y=6) is a d-distinct red.
        let mut text = solid(50, 12, [0, 0, 0, 255]);
        for d in 0..10u32 {
            let color = [(10 + d * 20) as u8, 0, 0, 255];
            for y in 6..12u32 {
                for x in d * 5..d * 5 + 5 {
                    let o = ((y * 50 + x) * 4) as usize;
                    text.rgba[o..o + 4].copy_from_slice(&color);
                }
            }
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            text: Some(text),
            ..Default::default()
        };
        let state = hit::UiState {
            kbps: Some(192),
            khz: Some(44),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &state);
        let color = |d: u32| [(10 + d * 20) as u8, 0, 0, 255];
        use xubamp_skin::sprites::{KBPS_X, KBPS_Y, KHZ_X, KHZ_Y};
        // kbps "192": digits at x=111,116,121 (y=43), sampled a couple pixels into each cell.
        assert_eq!(
            px(&fb, KBPS_X as u32 + 2, KBPS_Y as u32 + 2),
            color(1),
            "kbps hundreds"
        );
        assert_eq!(
            px(&fb, KBPS_X as u32 + 7, KBPS_Y as u32 + 2),
            color(9),
            "kbps tens"
        );
        assert_eq!(
            px(&fb, KBPS_X as u32 + 12, KBPS_Y as u32 + 2),
            color(2),
            "kbps units"
        );
        // khz "44": digits at x=156,161 (y=43).
        assert_eq!(
            px(&fb, KHZ_X as u32 + 2, KHZ_Y as u32 + 2),
            color(4),
            "khz tens"
        );
        assert_eq!(
            px(&fb, KHZ_X as u32 + 7, KHZ_Y as u32 + 2),
            color(4),
            "khz units"
        );
        // Nothing loaded: the readouts stay blank.
        let blank = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(
            px(&blank, KBPS_X as u32 + 2, KBPS_Y as u32 + 2),
            RED,
            "no kbps without a track"
        );
    }

    #[test]
    fn mono_stereo_lights_the_channel_word() {
        const BLUE: [u8; 4] = [0, 0, 255, 255];
        use xubamp_skin::sprites::{MONO_LIT, STEREO_LIT};
        // monoster: lit row (y=0) GREEN, unlit row (y=12) BLUE.
        let mut monoster = solid(56, 24, BLUE);
        for y in 0..12u32 {
            for x in 0..56u32 {
                let o = ((y * 56 + x) * 4) as usize;
                monoster.rgba[o..o + 4].copy_from_slice(&GREEN);
            }
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            monoster: Some(monoster),
            ..Default::default()
        };
        let (mono_x, mono_y) = (MONO_LIT.dst_x as u32 + 3, MONO_LIT.dst_y as u32 + 3);
        let (stereo_x, stereo_y) = (STEREO_LIT.dst_x as u32 + 3, STEREO_LIT.dst_y as u32 + 3);

        // Stereo (2 channels): stereo lit (green), mono dim (blue).
        let st = compose_main_window(
            &skin,
            &hit::UiState {
                channels: 2,
                ..Default::default()
            },
        );
        assert_eq!(
            px(&st, stereo_x, stereo_y),
            GREEN,
            "stereo lit for 2 channels"
        );
        assert_eq!(px(&st, mono_x, mono_y), BLUE, "mono dim for 2 channels");
        // Mono (1 channel): mono lit, stereo dim.
        let mo = compose_main_window(
            &skin,
            &hit::UiState {
                channels: 1,
                ..Default::default()
            },
        );
        assert_eq!(px(&mo, mono_x, mono_y), GREEN, "mono lit for 1 channel");
        assert_eq!(
            px(&mo, stereo_x, stereo_y),
            BLUE,
            "stereo dim for 1 channel"
        );
        // Nothing loaded (0 channels): both dim.
        let none = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(px(&none, mono_x, mono_y), BLUE, "mono dim with no track");
        assert_eq!(
            px(&none, stereo_x, stereo_y),
            BLUE,
            "stereo dim with no track"
        );
    }

    #[test]
    fn marquee_draws_over_the_panel_only_with_a_title_and_a_text_sheet() {
        // A text sheet whose glyph cells are all GREEN, over a RED main background.
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            text: Some(solid(155, 18, GREEN)),
            ..Default::default()
        };
        let (mx, my) = (
            xubamp_skin::sprites::MARQUEE_X as u32,
            xubamp_skin::sprites::MARQUEE_Y as u32,
        );

        // With a title, the first glyph cell paints the marquee origin green.
        let playing = hit::UiState {
            title: "HELLO".to_string(),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &playing);
        assert_eq!(
            px(&fb, mx, my),
            GREEN,
            "title glyph drawn at the marquee origin"
        );
        // The glyph row is confined to CELL_H pixels: the rows just above and below stay the
        // red background, so a mis-sized cell (drawing above or below the strip) would be caught.
        assert_eq!(
            px(&fb, mx, my - 1),
            RED,
            "nothing drawn above the glyph row"
        );
        assert_eq!(
            px(&fb, mx, my + xubamp_skin::textfont::CELL_H as u32),
            RED,
            "nothing drawn below the glyph row",
        );

        // With no title the strip is untouched: the red background shows through.
        let idle = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(
            px(&idle, mx, my),
            RED,
            "empty title leaves the panel background"
        );

        // A skin without text.bmp never draws a marquee, even with a title set.
        let no_font = Skin {
            main: Some(solid(275, 116, RED)),
            ..Default::default()
        };
        let fb = compose_main_window(&no_font, &playing);
        assert_eq!(px(&fb, mx, my), RED, "no text sheet, no marquee");
    }

    #[test]
    fn disabled_title_scrolling_draws_a_long_title_from_offset_zero() {
        // Give A and B distinct source-cell colours. At a one-glyph offset, normal scrolling puts
        // B at the strip origin; disabling the setting must put A there even if state still carries
        // an old offset from the preceding animated frame.
        const BLUE: [u8; 4] = [0, 0, 255, 255];
        let mut text = solid(155, 18, [0, 0, 0, 255]);
        for y in 0..6u32 {
            for x in 0..5u32 {
                let a = ((y * text.width + x) * 4) as usize;
                text.rgba[a..a + 4].copy_from_slice(&GREEN);
                let b = ((y * text.width + 5 + x) * 4) as usize;
                text.rgba[b..b + 4].copy_from_slice(&BLUE);
            }
        }
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            text: Some(text),
            ..Default::default()
        };
        let state = hit::UiState {
            title: format!("AB{}", "A".repeat(40)),
            marquee_offset: 5,
            ..Default::default()
        };
        let (mx, my) = (sprites::MARQUEE_X as u32, sprites::MARQUEE_Y as u32);

        let moving = compose_main_window(&skin, &state);
        assert_eq!(
            px(&moving, mx, my),
            BLUE,
            "saved offset normally shifts B into view"
        );

        let static_title = compose_main_window(
            &skin,
            &hit::UiState {
                scroll_title: false,
                ..state
            },
        );
        assert_eq!(
            px(&static_title, mx, my),
            GREEN,
            "static title ignores stale offset and begins with A"
        );
    }

    #[test]
    fn balance_readout_matches_winamp() {
        assert_eq!(balance_readout(0), "Balance: Center");
        assert_eq!(balance_readout(-12), "Balance: 12% Left");
        assert_eq!(balance_readout(34), "Balance: 34% Right");
        assert_eq!(balance_readout(-100), "Balance: 100% Left");
        assert_eq!(balance_readout(100), "Balance: 100% Right");
    }

    #[test]
    fn dragging_a_slider_shows_its_readout_in_the_marquee() {
        // A GREEN text sheet over a RED background. With an empty title the marquee is normally
        // blank, but while a volume/balance slider is dragged it paints the readout there.
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            text: Some(solid(155, 18, GREEN)),
            ..Default::default()
        };
        let (mx, my) = (
            xubamp_skin::sprites::MARQUEE_X as u32,
            xubamp_skin::sprites::MARQUEE_Y as u32,
        );

        // No title, not dragging: the marquee stays the background.
        let idle = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(px(&idle, mx, my), RED, "idle marquee is blank");

        // Dragging volume paints the "Volume: 100%" readout ('V' cell) at the origin.
        let vol = hit::UiState {
            dragging: Some(hit::Slider::Volume),
            volume: 100,
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &vol);
        assert_eq!(
            px(&fb, mx, my),
            GREEN,
            "volume readout drawn while dragging"
        );

        // Dragging balance paints its readout too.
        let bal = hit::UiState {
            dragging: Some(hit::Slider::Balance),
            balance: -50,
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &bal);
        assert_eq!(
            px(&fb, mx, my),
            GREEN,
            "balance readout drawn while dragging"
        );

        // Disabling song-title scrolling must not suppress these temporary classic readouts.
        let static_vol = hit::UiState {
            scroll_title: false,
            marquee_offset: 45,
            dragging: Some(hit::Slider::Volume),
            volume: 73,
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &static_vol);
        assert_eq!(
            px(&fb, mx, my),
            GREEN,
            "volume readout remains visible while title scrolling is disabled"
        );
    }

    #[test]
    fn sliders_draw_over_the_panel_only_when_their_sheets_are_present() {
        use xubamp_skin::sprites;
        // GREEN volume + balance sheets over a RED main; the background column and thumb are all
        // GREEN, so any slider pixel reads GREEN and the untouched background stays RED.
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            volume: Some(solid(68, 433, GREEN)),
            balance: Some(solid(47, 433, GREEN)),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(
            px(&fb, sprites::VOLUME_X as u32, sprites::VOLUME_Y as u32),
            GREEN,
            "volume drawn"
        );
        assert_eq!(
            px(&fb, sprites::BALANCE_X as u32, sprites::BALANCE_Y as u32),
            GREEN,
            "balance drawn"
        );
        // Between the two sliders the main background shows through.
        assert_eq!(
            px(
                &fb,
                (sprites::VOLUME_X + sprites::VOLUME_W) as u32,
                sprites::VOLUME_Y as u32
            ),
            RED
        );

        // A skin without the slider sheets draws neither.
        let bare = Skin {
            main: Some(solid(275, 116, RED)),
            ..Default::default()
        };
        let fb = compose_main_window(&bare, &hit::UiState::default());
        assert_eq!(
            px(&fb, sprites::VOLUME_X as u32, sprites::VOLUME_Y as u32),
            RED,
            "no volume sheet"
        );
        assert_eq!(
            px(&fb, sprites::BALANCE_X as u32, sprites::BALANCE_Y as u32),
            RED,
            "no balance sheet"
        );
    }

    #[test]
    fn posbar_draws_over_the_panel_only_when_its_sheet_is_present() {
        use xubamp_skin::sprites;
        // A GREEN posbar sheet over a RED main. The whole sheet is GREEN, so any seek-bar pixel
        // (groove or thumb) reads GREEN and the untouched background stays RED.
        let skin = Skin {
            main: Some(solid(275, 116, RED)),
            posbar: Some(solid(307, 10, GREEN)),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &hit::UiState::default());
        assert_eq!(
            px(&fb, sprites::POSBAR_X as u32, sprites::POSBAR_Y as u32),
            GREEN,
            "posbar drawn"
        );
        // Just below the 10px-tall bar the main background shows through.
        assert_eq!(
            px(
                &fb,
                sprites::POSBAR_X as u32,
                (sprites::POSBAR_Y + sprites::POSBAR_H) as u32
            ),
            RED,
            "nothing drawn below the bar",
        );

        // A skin without posbar.bmp draws no seek bar.
        let bare = Skin {
            main: Some(solid(275, 116, RED)),
            ..Default::default()
        };
        let fb = compose_main_window(&bare, &hit::UiState::default());
        assert_eq!(
            px(&fb, sprites::POSBAR_X as u32, sprites::POSBAR_Y as u32),
            RED,
            "no posbar sheet"
        );
    }

    #[test]
    fn base_skin_draws_procedural_feedback_for_its_missing_sheets() {
        use xubamp_skin::sprites;
        let skin = xubamp_skin::default_skin();
        let idle = compose_main_window(&skin, &hit::UiState::default());

        // A held transport button darkens its footprint, so the press is visible on the base skin.
        let pressed = hit::UiState {
            pressed: Some(hit::TRANSPORT_ORDER[0]),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &pressed);
        let (bx, by) = (
            sprites::CBUTTONS[0].dst_x as u32 + 2,
            sprites::CBUTTONS[0].dst_y as u32 + 2,
        );
        assert_ne!(
            px(&fb, bx, by),
            px(&idle, bx, by),
            "a held button darkens on the base skin"
        );

        // The volume thumb moves with the value, so quiet and loud compose to different frames.
        let quiet = compose_main_window(&skin, &hit::UiState { volume: 0, ..Default::default() });
        let loud = compose_main_window(&skin, &hit::UiState { volume: 100, ..Default::default() });
        assert_ne!(quiet.rgba, loud.rgba, "the volume thumb tracks the value");

        // The clock draws digits once a time is known.
        let playing = hit::UiState {
            elapsed: Some(65),
            ..Default::default()
        };
        let fb = compose_main_window(&skin, &playing);
        let (cx, cy) = (
            sprites::TIME_DIGITS[0].0 as u32,
            sprites::TIME_DIGITS[0].1 as u32,
        );
        let differs = (0..7).any(|dy| (0..6).any(|dx| px(&fb, cx + dx, cy + dy) != px(&idle, cx + dx, cy + dy)));
        assert!(differs, "the clock draws digits on the base skin");
    }

    #[test]
    fn blit_clips_at_the_edge_without_panicking() {
        let mut fb = Framebuffer::new(10, 10);
        let src = solid(5, 5, GREEN);
        // Drawn at (8,8): only the 2x2 top-left corner of src fits.
        blit(&mut fb, &src, Rect::new(0, 0, 5, 5), 8, 8);
        assert_eq!(px(&fb, 9, 9), GREEN); // inside
        assert_eq!(px(&fb, 7, 7), [0, 0, 0, 0]); // untouched
    }
}
