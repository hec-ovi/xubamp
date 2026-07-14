//! Classic 10-band equalizer window composition and input policy.
//!
//! The module is deliberately platform-free. It consumes the optional `EQMAIN.BMP` / `EQ_EX.BMP`
//! sheets decoded by `xubamp-skin`, produces an opaque framebuffer, and turns pointer events into a
//! small command/action vocabulary. The Wayland layer owns actual window resizing and native preset
//! menus; the audio/player layer owns applying the emitted dB values.

use xubamp_skin::bmp::Image;
use xubamp_skin::sprites::{self, Placement, Rect};
use xubamp_skin::{font, Skin};

use crate::{blit, blit_placement, Framebuffer};

/// Classic equalizer limits. Keeping these local avoids making the pure renderer depend on the DSP
/// crate while preserving the same public units at the integration boundary.
pub const MIN_DB: f32 = -12.0;
pub const MAX_DB: f32 = 12.0;

/// The three clickable dB labels on the graph's left edge (`+12db`/`0db`/`-12db`). Clicking one
/// flattens all ten bands to that level, leaving the preamp untouched, matching classic Winamp
/// (`eqmain_dbs.m` loops the ten bands only). Coordinates are the EQ-window-local click rects.
const DB_LABEL_X: i32 = 45;
const DB_LABEL_W: i32 = 22;
const DB_LABEL_H: i32 = 8;
const DB_LABELS: [(i32, i8); 3] = [(36, 12), (64, 0), (95, -12)];

/// Winamp snaps a band or preamp drag to exactly 0 dB when it lands within 5 of 100 slider units of
/// center, i.e. within 5% of the 24 dB span (1.2 dB). Keeps a flat setting easy to hit by hand.
fn snap_to_center(db: f32) -> f32 {
    const SNAP_DB: f32 = (MAX_DB - MIN_DB) * 0.05;
    if db.abs() <= SNAP_DB {
        0.0
    } else {
        db
    }
}

const FALLBACK_BODY: [u8; 3] = [17, 31, 41];
const FALLBACK_TITLE: [u8; 3] = [18, 83, 103];
const FALLBACK_FACE: [u8; 3] = [37, 65, 78];
const FALLBACK_LIGHT: [u8; 3] = [78, 128, 146];
const FALLBACK_DARK: [u8; 3] = [6, 15, 21];
const FALLBACK_PANEL: [u8; 3] = [2, 13, 18];
const FALLBACK_CYAN: [u8; 3] = [42, 208, 236];
const FALLBACK_DISABLED: [u8; 3] = [77, 88, 92];

/// Buttons which can be armed while held. AUTO is intentionally absent: automatic per-track
/// presets are unsupported, so its visible control is inert and never acquires a pressed state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    On,
    Presets,
    Shade,
    Close,
}

/// Equalizer sliders. The compact window also mirrors main-window volume and balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slider {
    Preamp,
    Band(usize),
    Volume,
    Balance,
}

/// The region beneath a window-local point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    TitleBar,
    Button(Button),
    /// Visible but unsupported AUTO control.
    Auto,
    Slider(Slider),
    /// One of the three `+12db`/`0db`/`-12db` graph labels (carrying that dB level). Clicking it
    /// flattens all ten bands to the level; `0db` is the "reset to flat".
    DbLabel(i8),
    Body,
    None,
}

/// Commands for state owned outside the renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Enabled(bool),
    Preamp(f32),
    Band {
        index: usize,
        db: f32,
    },
    /// Apply a complete menu/file preset atomically at the player boundary.
    Preset {
        preamp_db: f32,
        bands_db: [f32; 10],
    },
    Volume(u8),
    Balance(i8),
}

/// One caller-supplied preset shown by the platform menu. The DSP crate remains the canonical
/// source of built-in and EQF values; this representation only carries them through the
/// renderer/window boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub preamp_db: f32,
    pub bands_db: [f32; 10],
}

impl Preset {
    pub fn sanitized(mut self) -> Self {
        self.preamp_db = sanitize_db(self.preamp_db);
        for db in &mut self.bands_db {
            *db = sanitize_db(*db);
        }
        self
    }
}

/// Non-audio actions which the platform layer carries out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Resize the equalizer to 14px (`true`) or restore it to 116px (`false`).
    SetShade(bool),
    Close,
    /// Open the native presets menu. Load/save dialogs are intentionally outside this module.
    Presets,
}

/// Result of one pointer event.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Outcome {
    pub start_move: bool,
    pub command: Option<Command>,
    pub action: Option<Action>,
    pub redraw: bool,
}

/// Persistent visual and interaction state for the equalizer pane.
#[derive(Debug, Clone, PartialEq)]
pub struct EqState {
    pub enabled: bool,
    pub preamp_db: f32,
    pub bands_db: [f32; 10],
    /// Mirrored main-player values used only by the compact EQ strip.
    pub volume: u8,
    pub balance: i8,
    pub shade: bool,
    pub pressed_button: Option<Button>,
    pub dragging: Option<Slider>,
}

impl Default for EqState {
    fn default() -> Self {
        Self {
            enabled: true,
            preamp_db: 0.0,
            bands_db: [0.0; 10],
            volume: 100,
            balance: 0,
            shade: false,
            pressed_button: None,
            dragging: None,
        }
    }
}

impl EqState {
    /// Clamp data arriving from configuration or a preset before handing it to rendering/audio.
    pub fn sanitize(&mut self) {
        self.preamp_db = sanitize_db(self.preamp_db);
        for db in &mut self.bands_db {
            *db = sanitize_db(*db);
        }
        self.volume = self.volume.min(100);
        self.balance = self.balance.clamp(-100, 100);
    }
}

fn sanitize_db(db: f32) -> f32 {
    if db.is_finite() {
        db.clamp(MIN_DB, MAX_DB)
    } else {
        0.0
    }
}

/// Select the value-dependent EQMAIN background frame, -12 dB -> 0 and +12 dB -> 27.
pub fn slider_frame(db: f32) -> i32 {
    let normalized = (sanitize_db(db) - MIN_DB) / (MAX_DB - MIN_DB);
    (normalized * (sprites::EQ_SLIDER_FRAMES - 1) as f32).round() as i32
}

/// Source cell for a value-dependent 14x63 slider background in EQMAIN's 14-by-2 grid.
pub fn slider_frame_rect(db: f32) -> Rect {
    let frame = slider_frame(db);
    Rect::new(
        sprites::EQ_SLIDER_GRID.x
            + (frame % sprites::EQ_SLIDER_COLUMNS) * sprites::EQ_SLIDER_X_STRIDE,
        sprites::EQ_SLIDER_GRID.y
            + (frame / sprites::EQ_SLIDER_COLUMNS) * sprites::EQ_SLIDER_Y_STRIDE,
        sprites::EQ_SLIDER_W,
        sprites::EQ_SLIDER_H,
    )
}

/// Vertical thumb offset: +12 dB is flush-top and -12 dB flush-bottom.
pub fn slider_thumb_offset(db: f32) -> i32 {
    let top_to_bottom = (MAX_DB - sanitize_db(db)) / (MAX_DB - MIN_DB);
    (top_to_bottom * sprites::EQ_SLIDER_THUMB_TRAVEL as f32).round() as i32
}

/// Inverse of [`slider_thumb_offset`], centering the thumb on the pointer and clamping beyond the
/// track so an implicit pointer grab can drag cleanly to either rail.
pub fn db_from_y(y: i32) -> f32 {
    let offset = (y - sprites::EQ_SLIDER_Y - sprites::EQ_SLIDER_THUMB.h / 2)
        .clamp(0, sprites::EQ_SLIDER_THUMB_TRAVEL);
    MAX_DB - offset as f32 / sprites::EQ_SLIDER_THUMB_TRAVEL as f32 * (MAX_DB - MIN_DB)
}

fn shade_volume_offset(volume: u8) -> i32 {
    let travel = sprites::EQ_SHADE_VOLUME_W - sprites::EQ_SHADE_THUMB_W;
    (volume.min(100) as f32 / 100.0 * travel as f32).round() as i32
}

fn shade_balance_offset(balance: i8) -> i32 {
    let travel = sprites::EQ_SHADE_BALANCE_W - sprites::EQ_SHADE_THUMB_W;
    let normalized = (balance.clamp(-100, 100) as i32 + 100) as f32 / 200.0;
    (normalized * travel as f32).round() as i32
}

fn shade_volume_from_x(x: i32) -> u8 {
    let travel = sprites::EQ_SHADE_VOLUME_W - sprites::EQ_SHADE_THUMB_W;
    let offset = (x - sprites::EQ_SHADE_VOLUME_X - sprites::EQ_SHADE_THUMB_W / 2).clamp(0, travel);
    (offset as f32 / travel as f32 * 100.0).round() as u8
}

fn shade_balance_from_x(x: i32) -> i8 {
    let travel = sprites::EQ_SHADE_BALANCE_W - sprites::EQ_SHADE_THUMB_W;
    let offset = (x - sprites::EQ_SHADE_BALANCE_X - sprites::EQ_SHADE_THUMB_W / 2).clamp(0, travel);
    ((offset as f32 / travel as f32 * 200.0).round() as i32 - 100) as i8
}

fn in_rect(x: i32, y: i32, rx: i32, ry: i32, rw: i32, rh: i32) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

fn in_placement(x: i32, y: i32, p: Placement) -> bool {
    in_rect(x, y, p.dst_x, p.dst_y, p.src.w, p.src.h)
}

/// Hit-test either the full 275x116 equalizer or its 275x14 compact strip.
pub fn region_at(state: &EqState, x: i32, y: i32) -> Region {
    let height = if state.shade {
        sprites::EQ_SHADE_H
    } else {
        sprites::EQ_H
    };
    if x < 0 || y < 0 || x >= sprites::EQ_W || y >= height {
        return Region::None;
    }

    if in_rect(
        x,
        y,
        sprites::EQ_SHADE_BUTTON_X,
        sprites::EQ_TITLE_BUTTON_Y,
        sprites::EQ_TITLE_BUTTON_W,
        sprites::EQ_TITLE_BUTTON_W,
    ) {
        return Region::Button(Button::Shade);
    }
    if in_rect(
        x,
        y,
        sprites::EQ_CLOSE_BUTTON_X,
        sprites::EQ_TITLE_BUTTON_Y,
        sprites::EQ_TITLE_BUTTON_W,
        sprites::EQ_TITLE_BUTTON_W,
    ) {
        return Region::Button(Button::Close);
    }

    if state.shade {
        if in_rect(
            x,
            y,
            sprites::EQ_SHADE_VOLUME_X,
            sprites::EQ_SHADE_VOLUME_Y,
            sprites::EQ_SHADE_VOLUME_W,
            sprites::EQ_SHADE_THUMB_H,
        ) {
            return Region::Slider(Slider::Volume);
        }
        if in_rect(
            x,
            y,
            sprites::EQ_SHADE_BALANCE_X,
            sprites::EQ_SHADE_BALANCE_Y,
            sprites::EQ_SHADE_BALANCE_W,
            sprites::EQ_SHADE_THUMB_H,
        ) {
            return Region::Slider(Slider::Balance);
        }
        return Region::TitleBar;
    }

    if in_placement(x, y, sprites::EQMAIN_ON) {
        return Region::Button(Button::On);
    }
    if in_placement(x, y, sprites::EQ_AUTO) {
        return Region::Auto;
    }
    if in_placement(x, y, sprites::EQ_PRESETS) {
        return Region::Button(Button::Presets);
    }
    for (label_y, level) in DB_LABELS {
        if in_rect(x, y, DB_LABEL_X, label_y, DB_LABEL_W, DB_LABEL_H) {
            return Region::DbLabel(level);
        }
    }
    if in_rect(
        x,
        y,
        sprites::EQ_PREAMP_X,
        sprites::EQ_SLIDER_Y,
        sprites::EQ_SLIDER_W,
        sprites::EQ_SLIDER_H,
    ) {
        return Region::Slider(Slider::Preamp);
    }
    for (index, &band_x) in sprites::EQ_BAND_X.iter().enumerate() {
        if in_rect(
            x,
            y,
            band_x,
            sprites::EQ_SLIDER_Y,
            sprites::EQ_SLIDER_W,
            sprites::EQ_SLIDER_H,
        ) {
            return Region::Slider(Slider::Band(index));
        }
    }
    if y < sprites::EQ_SHADE_H {
        Region::TitleBar
    } else {
        Region::Body
    }
}

/// Arm buttons or start a live slider drag. Like the main-window gain controls, equalizer sliders
/// commit values as they move; release only returns the thumb to its ordinary sprite.
pub fn on_press(state: &mut EqState, x: i32, y: i32) -> Outcome {
    match region_at(state, x, y) {
        Region::TitleBar => Outcome {
            start_move: true,
            ..Default::default()
        },
        Region::Button(button) => {
            state.pressed_button = Some(button);
            Outcome {
                redraw: true,
                ..Default::default()
            }
        }
        Region::Slider(slider) => {
            state.dragging = Some(slider);
            let command = update_slider(state, slider, x, y);
            Outcome {
                command,
                redraw: true,
                ..Default::default()
            }
        }
        // A dB label flattens the ten bands to its level, leaving the preamp. Emitted as a single
        // atomic Preset so the player boundary applies it in one step, like a menu preset.
        Region::DbLabel(level) => {
            let db = sanitize_db(level as f32);
            state.bands_db = [db; 10];
            Outcome {
                command: Some(Command::Preset {
                    preamp_db: state.preamp_db,
                    bands_db: state.bands_db,
                }),
                redraw: true,
                ..Default::default()
            }
        }
        // AUTO is deliberately inert. It neither arms nor starts a window move.
        Region::Auto | Region::Body | Region::None => Outcome::default(),
    }
}

/// Follow an in-progress slider beyond the window edge, clamping to its rail.
pub fn on_motion(state: &mut EqState, x: i32, y: i32) -> Outcome {
    let Some(slider) = state.dragging else {
        return Outcome::default();
    };
    let before = slider_value(state, slider);
    let command = update_slider(state, slider, x, y);
    let changed = before != slider_value(state, slider);
    Outcome {
        command: changed.then_some(command).flatten(),
        redraw: changed,
        ..Default::default()
    }
}

/// Complete a live slider drag, or fire an armed button only when released over the same region.
/// Releasing a button elsewhere cancels it, matching the other classic panes.
pub fn on_release(state: &mut EqState, x: i32, y: i32) -> Outcome {
    if state.dragging.take().is_some() {
        return Outcome {
            redraw: true,
            ..Default::default()
        };
    }

    let Some(button) = state.pressed_button.take() else {
        return Outcome::default();
    };
    let fired = region_at(state, x, y) == Region::Button(button);
    if !fired {
        return Outcome {
            redraw: true,
            ..Default::default()
        };
    }

    let mut outcome = Outcome {
        redraw: true,
        ..Default::default()
    };
    match button {
        Button::On => {
            state.enabled = !state.enabled;
            outcome.command = Some(Command::Enabled(state.enabled));
        }
        Button::Presets => outcome.action = Some(Action::Presets),
        Button::Shade => {
            state.shade = !state.shade;
            outcome.action = Some(Action::SetShade(state.shade));
        }
        Button::Close => outcome.action = Some(Action::Close),
    }
    outcome
}

/// Cancel an armed button when the pointer leaves. Slider drags retain the implicit grab and are
/// completed by their eventual release, just like main-window sliders.
pub fn on_leave(state: &mut EqState) -> bool {
    state.pressed_button.take().is_some()
}

fn slider_value(state: &EqState, slider: Slider) -> Option<f32> {
    match slider {
        Slider::Preamp => Some(state.preamp_db),
        Slider::Band(index) => state.bands_db.get(index).copied(),
        Slider::Volume => Some(state.volume as f32),
        Slider::Balance => Some(state.balance as f32),
    }
}

fn update_slider(state: &mut EqState, slider: Slider, x: i32, y: i32) -> Option<Command> {
    match slider {
        Slider::Preamp => {
            let db = snap_to_center(db_from_y(y));
            state.preamp_db = db;
            Some(Command::Preamp(db))
        }
        Slider::Band(index) => {
            let db = snap_to_center(db_from_y(y));
            let value = state.bands_db.get_mut(index)?;
            *value = db;
            Some(Command::Band { index, db })
        }
        Slider::Volume => {
            let volume = shade_volume_from_x(x);
            state.volume = volume;
            Some(Command::Volume(volume))
        }
        Slider::Balance => {
            let balance = shade_balance_from_x(x);
            state.balance = balance;
            Some(Command::Balance(balance))
        }
    }
}

/// Compose the current expanded or shaded equalizer framebuffer.
pub fn compose(skin: &Skin, state: &EqState) -> Framebuffer {
    if state.shade {
        compose_shade(skin, state)
    } else {
        compose_expanded(skin, state)
    }
}

fn compose_expanded(skin: &Skin, state: &EqState) -> Framebuffer {
    let mut fb = Framebuffer::new(sprites::EQ_W as u32, sprites::EQ_H as u32);
    draw_fallback_expanded(&mut fb, state);

    let Some(sheet) = &skin.eqmain else {
        return fb;
    };
    blit_placement(&mut fb, sheet, sprites::EQ_BACKGROUND);
    blit_placement(&mut fb, sheet, sprites::EQ_TITLE_ACTIVE);

    let on = match (state.enabled, state.pressed_button == Some(Button::On)) {
        (false, false) => sprites::EQMAIN_ON,
        (false, true) => sprites::EQMAIN_ON_PRESSED,
        (true, false) => sprites::EQMAIN_ON_SELECTED,
        (true, true) => sprites::EQMAIN_ON_SELECTED_PRESSED,
    };
    blit_placement(&mut fb, sheet, on);
    blit_placement(&mut fb, sheet, sprites::EQ_AUTO);
    let presets = if state.pressed_button == Some(Button::Presets) {
        sprites::EQ_PRESETS_PRESSED
    } else {
        sprites::EQ_PRESETS
    };
    blit_placement(&mut fb, sheet, presets);
    blit_placement(&mut fb, sheet, sprites::EQ_GRAPH);

    draw_skin_slider(
        &mut fb,
        sheet,
        sprites::EQ_PREAMP_X,
        state.preamp_db,
        state.dragging == Some(Slider::Preamp),
    );
    for (index, &x) in sprites::EQ_BAND_X.iter().enumerate() {
        draw_skin_slider(
            &mut fb,
            sheet,
            x,
            state.bands_db[index],
            state.dragging == Some(Slider::Band(index)),
        );
    }

    draw_graph(&mut fb, Some(sheet), state);
    draw_expanded_title_press(&mut fb, skin, state);
    // AUTO has real art in classic sheets, but its per-track mode is not implemented. Muting the
    // supplied pixels avoids presenting an apparently functional control.
    dim_rect(
        &mut fb,
        sprites::EQ_AUTO.dst_x,
        sprites::EQ_AUTO.dst_y,
        sprites::EQ_AUTO.src.w,
        sprites::EQ_AUTO.src.h,
    );
    fb
}

fn compose_shade(skin: &Skin, state: &EqState) -> Framebuffer {
    let mut fb = Framebuffer::new(sprites::EQ_W as u32, sprites::EQ_SHADE_H as u32);
    draw_fallback_shade(&mut fb, state);

    let Some(sheet) = &skin.eq_ex else {
        return fb;
    };
    blit_placement(&mut fb, sheet, sprites::EQ_EX_SHADE_ACTIVE);

    let volume_segment = usize::from(state.volume > 33) + usize::from(state.volume > 66);
    blit(
        &mut fb,
        sheet,
        sprites::EQ_EX_VOLUME_THUMBS[volume_segment],
        sprites::EQ_SHADE_VOLUME_X + shade_volume_offset(state.volume),
        sprites::EQ_SHADE_VOLUME_Y,
    );
    let balance_segment = if state.balance < -33 {
        0
    } else if state.balance > 33 {
        2
    } else {
        1
    };
    blit(
        &mut fb,
        sheet,
        sprites::EQ_EX_BALANCE_THUMBS[balance_segment],
        sprites::EQ_SHADE_BALANCE_X + shade_balance_offset(state.balance),
        sprites::EQ_SHADE_BALANCE_Y,
    );

    if matches!(state.pressed_button, Some(Button::Shade | Button::Close)) {
        blit(
            &mut fb,
            sheet,
            sprites::EQ_EX_CLOSE,
            sprites::EQ_CLOSE_BUTTON_X,
            sprites::EQ_TITLE_BUTTON_Y,
        );
    }
    match state.pressed_button {
        Some(Button::Shade) => blit(
            &mut fb,
            sheet,
            sprites::EQ_EX_RESTORE_PRESSED,
            sprites::EQ_SHADE_BUTTON_X,
            sprites::EQ_TITLE_BUTTON_Y,
        ),
        Some(Button::Close) => blit(
            &mut fb,
            sheet,
            sprites::EQ_EX_CLOSE_PRESSED,
            sprites::EQ_CLOSE_BUTTON_X,
            sprites::EQ_TITLE_BUTTON_Y,
        ),
        _ => {}
    }
    fb
}

fn draw_skin_slider(fb: &mut Framebuffer, sheet: &Image, x: i32, db: f32, pressed: bool) {
    blit(fb, sheet, slider_frame_rect(db), x, sprites::EQ_SLIDER_Y);
    let thumb = if pressed {
        sprites::EQ_SLIDER_THUMB_PRESSED
    } else {
        sprites::EQ_SLIDER_THUMB
    };
    blit(
        fb,
        sheet,
        thumb,
        x + sprites::EQ_SLIDER_THUMB_DX,
        sprites::EQ_SLIDER_Y + slider_thumb_offset(db),
    );
}

fn draw_expanded_title_press(fb: &mut Framebuffer, skin: &Skin, state: &EqState) {
    if matches!(state.pressed_button, Some(Button::Shade | Button::Close)) {
        if let Some(main) = &skin.eqmain {
            blit(
                fb,
                main,
                sprites::EQ_CLOSE,
                sprites::EQ_CLOSE_BUTTON_X,
                sprites::EQ_TITLE_BUTTON_Y,
            );
        }
    }
    match state.pressed_button {
        Some(Button::Shade) => {
            if let Some(extension) = &skin.eq_ex {
                blit(
                    fb,
                    extension,
                    sprites::EQ_EX_SHADE_PRESSED,
                    sprites::EQ_SHADE_BUTTON_X,
                    sprites::EQ_TITLE_BUTTON_Y,
                );
            } else if let Some(main) = &skin.eqmain {
                blit(
                    fb,
                    main,
                    sprites::EQ_SHADE_PRESSED_FALLBACK,
                    sprites::EQ_SHADE_BUTTON_X,
                    sprites::EQ_TITLE_BUTTON_Y,
                );
            }
        }
        Some(Button::Close) => {
            if let Some(main) = &skin.eqmain {
                blit(
                    fb,
                    main,
                    sprites::EQ_CLOSE_PRESSED,
                    sprites::EQ_CLOSE_BUTTON_X,
                    sprites::EQ_TITLE_BUTTON_Y,
                );
            }
        }
        _ => {}
    }
}

/// Draw the skin's horizontal preamp marker and a connected band response curve over the graph.
/// Webamp maps the preamp marker and band curve in opposite vertical directions; preserve that
/// established visual convention here.
fn draw_graph(fb: &mut Framebuffer, sheet: Option<&Image>, state: &EqState) {
    let graph_x = sprites::EQ_GRAPH.dst_x;
    let graph_y = sprites::EQ_GRAPH.dst_y;
    let graph_h = sprites::EQ_GRAPH.src.h;

    let preamp_normalized = (sanitize_db(state.preamp_db) - MIN_DB) / (MAX_DB - MIN_DB);
    let preamp_y = (preamp_normalized * (graph_h - 1) as f32).round() as i32;
    if let Some(sheet) = sheet {
        blit(
            fb,
            sheet,
            sprites::EQ_PREAMP_LINE,
            graph_x,
            graph_y + preamp_y,
        );
    } else {
        fill_rect(fb, graph_x, graph_y + preamp_y, 113, 1, [26, 91, 108]);
    }

    let ys = state.bands_db.map(|db| {
        let normalized = (MAX_DB - sanitize_db(db)) / (MAX_DB - MIN_DB);
        (normalized * (graph_h - 1) as f32).round() as i32
    });
    let mut previous_y = ys[0];
    for x in 0..=108 {
        let segment = (x / 12).min(8) as usize;
        let fraction = (x - segment as i32 * 12) as f32 / 12.0;
        let target = ys[segment] as f32 + (ys[segment + 1] - ys[segment]) as f32 * fraction;
        let y = target.round().clamp(0.0, (graph_h - 1) as f32) as i32;
        let top = previous_y.min(y);
        let bottom = previous_y.max(y);
        for py in top..=bottom {
            let color = sheet
                .and_then(|image| {
                    pixel(
                        image,
                        sprites::EQ_GRAPH_COLORS.x,
                        sprites::EQ_GRAPH_COLORS.y + py,
                    )
                })
                .map(|rgba| [rgba[0], rgba[1], rgba[2]])
                .unwrap_or(FALLBACK_CYAN);
            put_rgb(fb, graph_x + 2 + x, graph_y + py, color);
        }
        previous_y = y;
    }
}

fn draw_fallback_expanded(fb: &mut Framebuffer, state: &EqState) {
    fill_rect(fb, 0, 0, sprites::EQ_W, sprites::EQ_H, FALLBACK_BODY);
    bevel(
        fb,
        0,
        0,
        sprites::EQ_W,
        sprites::EQ_H,
        FALLBACK_LIGHT,
        FALLBACK_DARK,
    );
    fill_rect(fb, 2, 2, sprites::EQ_W - 4, 11, FALLBACK_TITLE);
    let label = "EQUALIZER";
    let tx = (sprites::EQ_W - font::text_width(label) as i32) / 2;
    draw_text(fb, tx, 4, label, FALLBACK_CYAN);
    fallback_title_button(
        fb,
        sprites::EQ_SHADE_BUTTON_X,
        "-",
        state.pressed_button == Some(Button::Shade),
    );
    fallback_title_button(
        fb,
        sprites::EQ_CLOSE_BUTTON_X,
        "X",
        state.pressed_button == Some(Button::Close),
    );

    fallback_button(
        fb,
        sprites::EQMAIN_ON.dst_x,
        sprites::EQMAIN_ON.dst_y,
        sprites::EQMAIN_ON.src.w,
        sprites::EQMAIN_ON.src.h,
        "ON",
        state.enabled,
        state.pressed_button == Some(Button::On),
        false,
    );
    fallback_button(
        fb,
        sprites::EQ_AUTO.dst_x,
        sprites::EQ_AUTO.dst_y,
        sprites::EQ_AUTO.src.w,
        sprites::EQ_AUTO.src.h,
        "AUTO",
        false,
        false,
        true,
    );
    fallback_button(
        fb,
        sprites::EQ_PRESETS.dst_x,
        sprites::EQ_PRESETS.dst_y,
        sprites::EQ_PRESETS.src.w,
        sprites::EQ_PRESETS.src.h,
        "PRESETS",
        false,
        state.pressed_button == Some(Button::Presets),
        false,
    );

    fill_rect(
        fb,
        sprites::EQ_GRAPH.dst_x,
        sprites::EQ_GRAPH.dst_y,
        sprites::EQ_GRAPH.src.w,
        sprites::EQ_GRAPH.src.h,
        FALLBACK_PANEL,
    );
    bevel(
        fb,
        sprites::EQ_GRAPH.dst_x,
        sprites::EQ_GRAPH.dst_y,
        sprites::EQ_GRAPH.src.w,
        sprites::EQ_GRAPH.src.h,
        FALLBACK_DARK,
        FALLBACK_LIGHT,
    );
    draw_graph(fb, None, state);

    draw_text(fb, 45, 37, "+12", FALLBACK_CYAN);
    draw_text(fb, 50, 65, "0", FALLBACK_CYAN);
    draw_text(fb, 45, 96, "-12", FALLBACK_CYAN);
    draw_fallback_slider(
        fb,
        sprites::EQ_PREAMP_X,
        state.preamp_db,
        state.dragging == Some(Slider::Preamp),
    );
    for (index, &x) in sprites::EQ_BAND_X.iter().enumerate() {
        draw_fallback_slider(
            fb,
            x,
            state.bands_db[index],
            state.dragging == Some(Slider::Band(index)),
        );
    }
}

fn draw_fallback_shade(fb: &mut Framebuffer, state: &EqState) {
    fill_rect(fb, 0, 0, sprites::EQ_W, sprites::EQ_SHADE_H, FALLBACK_TITLE);
    bevel(
        fb,
        0,
        0,
        sprites::EQ_W,
        sprites::EQ_SHADE_H,
        FALLBACK_LIGHT,
        FALLBACK_DARK,
    );
    draw_text(fb, 5, 4, "EQ", FALLBACK_CYAN);
    draw_shade_track(
        fb,
        sprites::EQ_SHADE_VOLUME_X,
        sprites::EQ_SHADE_VOLUME_Y,
        sprites::EQ_SHADE_VOLUME_W,
        shade_volume_offset(state.volume),
        state.dragging == Some(Slider::Volume),
    );
    draw_shade_track(
        fb,
        sprites::EQ_SHADE_BALANCE_X,
        sprites::EQ_SHADE_BALANCE_Y,
        sprites::EQ_SHADE_BALANCE_W,
        shade_balance_offset(state.balance),
        state.dragging == Some(Slider::Balance),
    );
    fallback_title_button(
        fb,
        sprites::EQ_SHADE_BUTTON_X,
        "V",
        state.pressed_button == Some(Button::Shade),
    );
    fallback_title_button(
        fb,
        sprites::EQ_CLOSE_BUTTON_X,
        "X",
        state.pressed_button == Some(Button::Close),
    );
}

fn draw_fallback_slider(fb: &mut Framebuffer, x: i32, db: f32, pressed: bool) {
    fill_rect(
        fb,
        x,
        sprites::EQ_SLIDER_Y,
        sprites::EQ_SLIDER_W,
        sprites::EQ_SLIDER_H,
        FALLBACK_FACE,
    );
    let center = x + sprites::EQ_SLIDER_W / 2;
    fill_rect(
        fb,
        center,
        sprites::EQ_SLIDER_Y + 3,
        1,
        sprites::EQ_SLIDER_H - 6,
        FALLBACK_DARK,
    );
    let thumb_y = sprites::EQ_SLIDER_Y + slider_thumb_offset(db);
    let color = if pressed {
        FALLBACK_CYAN
    } else {
        FALLBACK_LIGHT
    };
    fill_rect(
        fb,
        x + sprites::EQ_SLIDER_THUMB_DX,
        thumb_y,
        sprites::EQ_SLIDER_THUMB.w,
        sprites::EQ_SLIDER_THUMB.h,
        color,
    );
    bevel(
        fb,
        x + sprites::EQ_SLIDER_THUMB_DX,
        thumb_y,
        sprites::EQ_SLIDER_THUMB.w,
        sprites::EQ_SLIDER_THUMB.h,
        FALLBACK_LIGHT,
        FALLBACK_DARK,
    );
}

fn draw_shade_track(fb: &mut Framebuffer, x: i32, y: i32, width: i32, offset: i32, pressed: bool) {
    fill_rect(fb, x, y + 2, width, 3, FALLBACK_PANEL);
    let color = if pressed {
        FALLBACK_CYAN
    } else {
        FALLBACK_LIGHT
    };
    fill_rect(
        fb,
        x + offset,
        y,
        sprites::EQ_SHADE_THUMB_W,
        sprites::EQ_SHADE_THUMB_H,
        color,
    );
}

#[allow(clippy::too_many_arguments)]
fn fallback_button(
    fb: &mut Framebuffer,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    label: &str,
    selected: bool,
    pressed: bool,
    disabled: bool,
) {
    let face = if disabled {
        FALLBACK_DISABLED
    } else if selected {
        [23, 98, 112]
    } else {
        FALLBACK_FACE
    };
    fill_rect(fb, x, y, width, height, face);
    let (tl, br) = if pressed {
        (FALLBACK_DARK, FALLBACK_LIGHT)
    } else {
        (FALLBACK_LIGHT, FALLBACK_DARK)
    };
    bevel(fb, x, y, width, height, tl, br);
    let color = if disabled {
        [38, 44, 46]
    } else {
        FALLBACK_CYAN
    };
    let text_x = x + (width - font::text_width(label) as i32) / 2;
    draw_text(fb, text_x, y + 3, label, color);
}

fn fallback_title_button(fb: &mut Framebuffer, x: i32, label: &str, pressed: bool) {
    fallback_button(
        fb,
        x,
        sprites::EQ_TITLE_BUTTON_Y,
        sprites::EQ_TITLE_BUTTON_W,
        sprites::EQ_TITLE_BUTTON_W,
        label,
        false,
        pressed,
        false,
    );
}

fn draw_text(fb: &mut Framebuffer, x: i32, y: i32, text: &str, color: [u8; 3]) {
    font::draw_text(&mut fb.rgba, fb.width, fb.height, x, y, text, color);
}

fn fill_rect(fb: &mut Framebuffer, x: i32, y: i32, width: i32, height: i32, color: [u8; 3]) {
    for py in y..y + height {
        for px in x..x + width {
            put_rgb(fb, px, py, color);
        }
    }
}

fn bevel(
    fb: &mut Framebuffer,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    top_left: [u8; 3],
    bottom_right: [u8; 3],
) {
    fill_rect(fb, x, y, width, 1, top_left);
    fill_rect(fb, x, y, 1, height, top_left);
    fill_rect(fb, x, y + height - 1, width, 1, bottom_right);
    fill_rect(fb, x + width - 1, y, 1, height, bottom_right);
}

fn put_rgb(fb: &mut Framebuffer, x: i32, y: i32, color: [u8; 3]) {
    if x < 0 || y < 0 || x as u32 >= fb.width || y as u32 >= fb.height {
        return;
    }
    let offset = ((y as u32 * fb.width + x as u32) * 4) as usize;
    fb.rgba[offset..offset + 3].copy_from_slice(&color);
    fb.rgba[offset + 3] = 255;
}

fn pixel(image: &Image, x: i32, y: i32) -> Option<[u8; 4]> {
    if x < 0 || y < 0 || x as u32 >= image.width || y as u32 >= image.height {
        return None;
    }
    let offset = ((y as u32 * image.width + x as u32) * 4) as usize;
    let bytes = image.rgba.get(offset..offset + 4)?;
    Some([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn dim_rect(fb: &mut Framebuffer, x: i32, y: i32, width: i32, height: i32) {
    for py in y.max(0)..(y + height).min(fb.height as i32) {
        for px in x.max(0)..(x + width).min(fb.width as i32) {
            let offset = ((py as u32 * fb.width + px as u32) * 4) as usize;
            let luminance = (u16::from(fb.rgba[offset])
                + u16::from(fb.rgba[offset + 1])
                + u16::from(fb.rgba[offset + 2]))
                / 3;
            let dim = (luminance / 2) as u8;
            fb.rgba[offset] = dim;
            fb.rgba[offset + 1] = dim;
            fb.rgba[offset + 2] = dim;
            fb.rgba[offset + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, color: [u8; 4]) -> Image {
        Image {
            width,
            height,
            rgba: color
                .iter()
                .copied()
                .cycle()
                .take(width as usize * height as usize * 4)
                .collect(),
        }
    }

    fn set_pixel(image: &mut Image, x: i32, y: i32, color: [u8; 4]) {
        let offset = ((y as u32 * image.width + x as u32) * 4) as usize;
        image.rgba[offset..offset + 4].copy_from_slice(&color);
    }

    fn framebuffer_pixel(fb: &Framebuffer, x: i32, y: i32) -> [u8; 4] {
        let offset = ((y as u32 * fb.width + x as u32) * 4) as usize;
        [
            fb.rgba[offset],
            fb.rgba[offset + 1],
            fb.rgba[offset + 2],
            fb.rgba[offset + 3],
        ]
    }

    #[test]
    fn slider_math_spans_both_rails_and_clamps_bad_values() {
        assert_eq!(slider_frame(MIN_DB), 0);
        assert_eq!(slider_frame(MAX_DB), 27);
        assert_eq!(slider_thumb_offset(MAX_DB), 0);
        assert_eq!(slider_thumb_offset(MIN_DB), 51);
        assert_eq!(slider_frame(f32::NAN), slider_frame(0.0));
        assert_eq!(slider_frame(99.0), 27);

        let top_center = sprites::EQ_SLIDER_Y + sprites::EQ_SLIDER_THUMB.h / 2;
        let bottom_center = top_center + sprites::EQ_SLIDER_THUMB_TRAVEL;
        assert_eq!(db_from_y(top_center), MAX_DB);
        assert_eq!(db_from_y(bottom_center), MIN_DB);
        assert_eq!(db_from_y(-1_000), MAX_DB);
        assert_eq!(db_from_y(1_000), MIN_DB);
    }

    #[test]
    fn external_presets_sanitize_every_curve_value() {
        let preset = Preset {
            name: "bad input".into(),
            preamp_db: f32::NAN,
            bands_db: [
                -99.0,
                -12.0,
                -6.0,
                0.0,
                6.0,
                12.0,
                99.0,
                f32::INFINITY,
                f32::NEG_INFINITY,
                1.5,
            ],
        }
        .sanitized();
        assert_eq!(preset.preamp_db, 0.0);
        assert_eq!(preset.bands_db[0], MIN_DB);
        assert_eq!(preset.bands_db[6], MAX_DB);
        assert_eq!(preset.bands_db[7], 0.0);
        assert_eq!(preset.bands_db[8], 0.0);
        assert_eq!(preset.bands_db[9], 1.5);
    }

    #[test]
    fn slider_frame_cells_follow_the_two_row_grid() {
        assert_eq!(slider_frame_rect(MIN_DB), Rect::new(13, 164, 14, 63));
        assert_eq!(slider_frame_rect(MAX_DB), Rect::new(208, 229, 14, 63));
        let middle = slider_frame_rect(0.0);
        assert_eq!(middle, Rect::new(13, 229, 14, 63));
    }

    #[test]
    fn hit_testing_uses_exact_expanded_geometry_and_auto_is_distinct() {
        let state = EqState::default();
        assert_eq!(region_at(&state, 14, 18), Region::Button(Button::On));
        assert_eq!(region_at(&state, 40, 18), Region::Auto);
        assert_eq!(region_at(&state, 217, 18), Region::Button(Button::Presets));
        assert_eq!(region_at(&state, 21, 38), Region::Slider(Slider::Preamp));
        assert_eq!(region_at(&state, 78, 38), Region::Slider(Slider::Band(0)));
        assert_eq!(region_at(&state, 253, 3), Region::TitleBar);
        assert_eq!(region_at(&state, 254, 3), Region::Button(Button::Shade));
        assert_eq!(region_at(&state, 264, 3), Region::Button(Button::Close));
        assert_eq!(region_at(&state, 275, 0), Region::None);
        assert_eq!(region_at(&state, 0, 116), Region::None);
    }

    #[test]
    fn auto_is_visible_but_completely_inert() {
        let mut state = EqState::default();
        let press = on_press(&mut state, 45, 20);
        assert_eq!(press, Outcome::default());
        assert_eq!(state.pressed_button, None);
        assert_eq!(on_release(&mut state, 45, 20), Outcome::default());
    }

    #[test]
    fn buttons_fire_on_matching_release_and_cancel_elsewhere() {
        let mut state = EqState::default();
        assert!(on_press(&mut state, 15, 19).redraw);
        let cancelled = on_release(&mut state, 100, 50);
        assert_eq!(cancelled.command, None);
        assert!(state.enabled);

        on_press(&mut state, 15, 19);
        let fired = on_release(&mut state, 15, 19);
        assert_eq!(fired.command, Some(Command::Enabled(false)));
        assert!(!state.enabled);

        on_press(&mut state, 220, 19);
        assert_eq!(
            on_release(&mut state, 220, 19).action,
            Some(Action::Presets)
        );
        on_press(&mut state, 265, 4);
        assert_eq!(on_release(&mut state, 265, 4).action, Some(Action::Close));
    }

    #[test]
    fn shade_button_toggles_state_and_changes_hit_map() {
        let mut state = EqState::default();
        on_press(&mut state, 255, 4);
        assert_eq!(
            on_release(&mut state, 255, 4).action,
            Some(Action::SetShade(true))
        );
        assert!(state.shade);
        assert_eq!(region_at(&state, 100, 13), Region::TitleBar);
        assert_eq!(region_at(&state, 100, 14), Region::None);

        on_press(&mut state, 255, 4);
        assert_eq!(
            on_release(&mut state, 255, 4).action,
            Some(Action::SetShade(false))
        );
        assert!(!state.shade);
    }

    #[test]
    fn db_labels_flatten_all_bands_and_leave_the_preamp() {
        for (y, level) in DB_LABELS {
            let mut state = EqState {
                preamp_db: 4.0,
                bands_db: [1.0; 10],
                ..EqState::default()
            };
            assert_eq!(region_at(&state, DB_LABEL_X + 1, y + 1), Region::DbLabel(level));
            let outcome = on_press(&mut state, DB_LABEL_X + 1, y + 1);
            assert_eq!(state.bands_db, [level as f32; 10], "all ten bands flattened");
            assert_eq!(state.preamp_db, 4.0, "the preamp is left untouched");
            assert_eq!(
                outcome.command,
                Some(Command::Preset {
                    preamp_db: 4.0,
                    bands_db: [level as f32; 10],
                })
            );
        }
    }

    #[test]
    fn snap_to_center_pulls_small_values_to_exactly_flat() {
        assert_eq!(snap_to_center(0.0), 0.0);
        assert_eq!(snap_to_center(0.5), 0.0);
        assert_eq!(snap_to_center(-1.0), 0.0);
        assert_eq!(snap_to_center(1.2), 0.0, "the 5%-of-span threshold snaps");
        assert!((snap_to_center(3.0) - 3.0).abs() < 1e-6, "past the threshold is kept");
        assert!((snap_to_center(-6.0) + 6.0).abs() < 1e-6);
    }

    #[test]
    fn equalizer_sliders_commit_live_and_release_only_ends_drag() {
        let mut state = EqState::default();
        let top = on_press(&mut state, 80, sprites::EQ_SLIDER_Y);
        assert_eq!(state.dragging, Some(Slider::Band(0)));
        assert_eq!(
            top.command,
            Some(Command::Band {
                index: 0,
                db: MAX_DB
            })
        );
        assert_eq!(state.bands_db[0], MAX_DB);

        let bottom = on_motion(&mut state, 80, 10_000);
        assert_eq!(
            bottom.command,
            Some(Command::Band {
                index: 0,
                db: MIN_DB
            })
        );
        assert_eq!(state.bands_db[0], MIN_DB);
        let unchanged = on_motion(&mut state, 80, 10_000);
        assert_eq!(unchanged, Outcome::default());

        let release = on_release(&mut state, 80, 10_000);
        assert_eq!(release.command, None);
        assert!(release.redraw);
        assert_eq!(state.dragging, None);
    }

    #[test]
    fn compact_sliders_mirror_volume_and_balance_with_clamped_drags() {
        let mut state = EqState {
            shade: true,
            ..Default::default()
        };
        let volume = on_press(&mut state, sprites::EQ_SHADE_VOLUME_X, 5);
        assert_eq!(volume.command, Some(Command::Volume(0)));
        assert_eq!(
            on_motion(&mut state, 10_000, 5).command,
            Some(Command::Volume(100))
        );
        on_release(&mut state, 10_000, 5);

        let balance = on_press(&mut state, sprites::EQ_SHADE_BALANCE_X, 5);
        assert_eq!(balance.command, Some(Command::Balance(-100)));
        assert_eq!(
            on_motion(&mut state, 10_000, 5).command,
            Some(Command::Balance(100))
        );
    }

    #[test]
    fn leave_cancels_buttons_but_preserves_an_implicit_slider_grab() {
        let mut state = EqState::default();
        on_press(&mut state, 15, 19);
        assert!(on_leave(&mut state));
        assert_eq!(state.pressed_button, None);

        on_press(&mut state, 22, 40);
        assert!(!on_leave(&mut state));
        assert_eq!(state.dragging, Some(Slider::Preamp));
    }

    #[test]
    fn fallback_frames_are_fully_opaque_in_both_modes() {
        let skin = Skin::default();
        let mut state = EqState::default();
        let expanded = compose(&skin, &state);
        assert_eq!((expanded.width, expanded.height), (275, 116));
        assert!(expanded.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));

        state.shade = true;
        let shade = compose(&skin, &state);
        assert_eq!((shade.width, shade.height), (275, 14));
        assert!(shade.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn expanded_composition_reads_eqmain_sprite_cells() {
        let mut eqmain = solid(275, 315, [1, 2, 3, 255]);
        // ON-selected sprite source is magenta; normal slider thumb is green.
        for y in 119..131 {
            for x in 69..95 {
                set_pixel(&mut eqmain, x, y, [255, 0, 255, 255]);
            }
        }
        for y in 164..175 {
            for x in 0..11 {
                set_pixel(&mut eqmain, x, y, [0, 255, 0, 255]);
            }
        }
        let skin = Skin {
            eqmain: Some(eqmain),
            ..Default::default()
        };
        let fb = compose(&skin, &EqState::default());
        assert_eq!(framebuffer_pixel(&fb, 15, 19), [255, 0, 255, 255]);
        let thumb_y = sprites::EQ_SLIDER_Y + slider_thumb_offset(0.0);
        assert_eq!(
            framebuffer_pixel(&fb, sprites::EQ_PREAMP_X + 2, thumb_y + 1),
            [0, 255, 0, 255]
        );
    }

    #[test]
    fn compact_composition_reads_eq_ex_and_uses_segmented_thumbs() {
        let mut extension = solid(275, 56, [4, 5, 6, 255]);
        for y in 30..37 {
            for x in 7..10 {
                set_pixel(&mut extension, x, y, [200, 10, 20, 255]);
            }
        }
        let skin = Skin {
            eq_ex: Some(extension),
            ..Default::default()
        };
        let state = EqState {
            shade: true,
            volume: 100,
            ..Default::default()
        };
        let fb = compose(&skin, &state);
        assert_eq!((fb.width, fb.height), (275, 14));
        assert_eq!(
            framebuffer_pixel(
                &fb,
                sprites::EQ_SHADE_VOLUME_X + sprites::EQ_SHADE_VOLUME_W - sprites::EQ_SHADE_THUMB_W,
                sprites::EQ_SHADE_VOLUME_Y,
            ),
            [200, 10, 20, 255]
        );
    }

    #[test]
    fn custom_auto_art_is_dimmed_to_signal_that_it_is_disabled() {
        let mut eqmain = solid(275, 315, [20, 40, 60, 255]);
        for y in sprites::EQ_AUTO.src.y..sprites::EQ_AUTO.src.y + sprites::EQ_AUTO.src.h {
            for x in sprites::EQ_AUTO.src.x..sprites::EQ_AUTO.src.x + sprites::EQ_AUTO.src.w {
                set_pixel(&mut eqmain, x, y, [240, 120, 60, 255]);
            }
        }
        let skin = Skin {
            eqmain: Some(eqmain),
            ..Default::default()
        };
        let fb = compose(&skin, &EqState::default());
        let shown = framebuffer_pixel(&fb, sprites::EQ_AUTO.dst_x + 2, sprites::EQ_AUTO.dst_y + 2);
        assert_eq!(shown[0], shown[1], "disabled art is grayscale");
        assert_eq!(shown[1], shown[2]);
        assert!(shown[0] < 100, "disabled art is visibly darkened");
        assert_eq!(shown[3], 255);
    }

    #[test]
    fn undersized_custom_sheets_leave_the_opaque_fallback_visible() {
        let skin = Skin {
            eqmain: Some(solid(1, 1, [255, 0, 0, 255])),
            eq_ex: Some(solid(1, 1, [0, 255, 0, 255])),
            ..Default::default()
        };
        let mut state = EqState::default();
        let expanded = compose(&skin, &state);
        assert!(expanded.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
        state.shade = true;
        let shade = compose(&skin, &state);
        assert!(shade.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn sanitize_clamps_external_state_and_replaces_non_finite_values() {
        let mut state = EqState {
            preamp_db: f32::NAN,
            bands_db: [99.0; 10],
            volume: 255,
            balance: -120,
            ..Default::default()
        };
        state.sanitize();
        assert_eq!(state.preamp_db, 0.0);
        assert_eq!(state.bands_db, [MAX_DB; 10]);
        assert_eq!(state.volume, 100);
        assert_eq!(state.balance, -100);
    }
}
