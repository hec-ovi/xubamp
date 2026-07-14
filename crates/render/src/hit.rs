//! Input mapping and UI state: turn a pointer position into the interactive region under it,
//! and turn press/motion/release events into state changes and commands. All pure (no platform
//! types), so the interaction policy is unit-testable without a compositor. The `wl` crate
//! owns the event loop and calls these; it does the side effects (redraw, window move, emit
//! command) that the outcomes describe.

use xubamp_skin::sprites;

use crate::vis::VisState;
use crate::{posbar, shade, slider};

/// The six classic transport buttons, in the order they appear on the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Prev,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
}

/// The four title-bar buttons, left to right. A click carries out a window action (handled by the
/// platform layer, not the audio engine); the up graphics are part of the title-bar strip and only
/// the pressed sprite is drawn while one is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleButton {
    Options,
    Minimize,
    Shade,
    Close,
}

/// The two main-window toggle buttons that open/close a secondary window: the equalizer and the
/// playlist editor. They light while their window is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowToggle {
    Equalizer,
    Playlist,
}

/// The two main-window mode buttons: shuffle and repeat. They light while their mode is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeButton {
    Shuffle,
    Repeat,
}

/// Which value the main clock shows. Remaining time is derived from the latest playback duration
/// and elapsed time; it is not stored separately, so seeking and clock ticks cannot make the two
/// representations drift apart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TimeDisplay {
    #[default]
    Elapsed,
    Remaining,
}

impl TimeDisplay {
    fn toggled(self) -> Self {
        match self {
            Self::Elapsed => Self::Remaining,
            Self::Remaining => Self::Elapsed,
        }
    }
}

/// Title-button identity for each entry of [`sprites::TITLE_BUTTONS_PRESSED`], in the same order.
pub const TITLE_BUTTON_ORDER: [TitleButton; 4] = [
    TitleButton::Options,
    TitleButton::Minimize,
    TitleButton::Shade,
    TitleButton::Close,
];

/// The three draggable sliders on the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slider {
    Volume,
    Balance,
    /// The position (seek) bar. It differs from the other two: dragging it only previews (moves
    /// the thumb and the time display); the seek commits once, on release.
    Position,
}

/// A command the window emits to the caller (the binary bridges these to the audio engine). A
/// transport button fires one on a completed click; the volume/balance sliders fire one whenever
/// their value moves (press and drag); the seek bar fires one `Seek` on release. `Eq` is not
/// derived because `Seek` carries an `f32` fraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Transport(Transport),
    /// New volume level, 0..=100.
    Volume(u8),
    /// New balance, -100..=100 (negative pans left, positive right).
    Balance(i8),
    /// Seek to `fraction` (0..=1) of the track, emitted once when the seek-bar drag is released.
    Seek(f32),
    /// Force the current track to restart from the top: the `x` hotkey, which (unlike the Play
    /// button) restarts even while already playing.
    Restart,
    /// Toggle the shuffle or repeat playback mode (the main-window mode buttons).
    ToggleMode(ModeButton),
    /// Play the playlist track at this index (a double-click on a playlist row, or the J
    /// jump-to-file confirming a match).
    PlayIndex(usize),
}

/// A decoded key the main window responds to, produced by the platform layer from its keysym so
/// this crate needs no windowing types. Letter shortcuts arrive as their produced character folded
/// to lowercase (so Shift is transparent and the binding follows the key's printed label rather
/// than a physical scancode); the arrow keys arrive as named variants, which are layout-independent.
/// The platform layer only forwards a press when no Ctrl/Alt/Super modifier is held, so those
/// combinations never reach here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPress {
    /// A character-producing key, already lowercased (e.g. `'x'`).
    Char(char),
    Up,
    Down,
    Left,
    Right,
}

/// Transport identity for each entry of [`sprites::CBUTTONS`] (and `CBUTTONS_PRESSED`), in the
/// same order, so the compositor can pick the pressed sprite for the held button.
pub const TRANSPORT_ORDER: [Transport; 6] = [
    Transport::Prev,
    Transport::Play,
    Transport::Pause,
    Transport::Stop,
    Transport::Next,
    Transport::Eject,
];

/// An interactive region of the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// The title-bar strip. Pressing here starts an interactive window move (classic drag).
    TitleBar,
    /// One of the four title-bar buttons (they take priority over the drag band).
    TitleButton(TitleButton),
    /// One of the six transport buttons.
    Transport(Transport),
    /// The volume slider.
    Volume,
    /// The balance slider.
    Balance,
    /// The position (seek) bar.
    Position,
    /// The MM:SS clock. Clicking it switches between elapsed and remaining time.
    Time,
    /// The visualizer panel. Clicking it cycles the visualization mode.
    Vis,
    /// The EQ or PL toggle button (opens/closes the equalizer or playlist window).
    Toggle(WindowToggle),
    /// The shuffle or repeat mode button.
    Mode(ModeButton),
    /// Not over any interactive element (the window body).
    None,
}

/// Height of the draggable title-bar band, taken from the title-bar sprite so there is one
/// source of truth for the geometry.
pub const TITLEBAR_H: i32 = sprites::TITLEBAR_ACTIVE.src.h;

/// Expanded clock click target. This includes the leading sign cell as well as all four digits,
/// matching the classic 59x13 time element without stealing pixels from adjacent controls.
pub const TIME_X: i32 = 39;
pub const TIME_Y: i32 = 26;
pub const TIME_W: i32 = 59;
pub const TIME_H: i32 = 13;

/// Compact clock click target. The visible glyphs run from the sign at x=128 through the final
/// digit at x=156; the parent target starts one pixel earlier, as in the classic shade layout.
pub const SHADE_TIME_X: i32 = 127;
pub const SHADE_TIME_Y: i32 = sprites::SHADE_TIME_Y;
pub const SHADE_TIME_W: i32 = 30;
pub const SHADE_TIME_H: i32 = 6;

/// How far the pointer must move from a title-bar press before it becomes a window drag, in window
/// pixels. Below this, a press-and-release is a click, not a move, so a near-miss on one of the
/// small title-bar buttons does not jump the whole window (the band surrounds those buttons).
pub const MOVE_THRESHOLD_PX: i32 = 4;

/// Has the pointer moved far enough from the title-bar press point (offset `dx`, `dy`) to begin a
/// window drag? Squared distance, so there is no float math and no directional bias.
pub fn exceeds_move_threshold(dx: i32, dy: i32) -> bool {
    dx * dx + dy * dy > MOVE_THRESHOLD_PX * MOVE_THRESHOLD_PX
}

/// Does window-local pixel (`x`, `y`) fall inside the on-window rectangle of button `b`? The
/// button's screen rectangle is its destination plus the source sprite's width and height.
fn in_button(b: &sprites::Placement, x: i32, y: i32) -> bool {
    x >= b.dst_x && x < b.dst_x + b.src.w && y >= b.dst_y && y < b.dst_y + b.src.h
}

/// Is (`x`, `y`) inside the axis-aligned rectangle at (`rx`, `ry`) of size `rw`x`rh`?
fn in_rect(x: i32, y: i32, rx: i32, ry: i32, rw: i32, rh: i32) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

/// Which region of the main window is at window-local pixel (`x`, `y`)? Points outside the
/// window map to [`Region::None`]. Transport buttons and sliders win over the body; the
/// title-bar band is the top strip. The interactive elements occupy disjoint rows, so their
/// order here does not matter.
pub fn hit_test(x: i32, y: i32) -> Region {
    if x < 0 || y < 0 || x >= sprites::MAIN_W || y >= sprites::MAIN_H {
        return Region::None;
    }
    // Title-bar buttons win over the drag band (a click on a button never starts a move).
    for (placement, id) in sprites::TITLE_BUTTONS_PRESSED
        .iter()
        .zip(TITLE_BUTTON_ORDER)
    {
        if in_button(placement, x, y) {
            return Region::TitleButton(id);
        }
    }
    for (placement, id) in sprites::CBUTTONS.iter().zip(TRANSPORT_ORDER) {
        if in_button(placement, x, y) {
            return Region::Transport(id);
        }
    }
    if in_rect(
        x,
        y,
        sprites::VOLUME_X,
        sprites::VOLUME_Y,
        sprites::VOLUME_W,
        sprites::SLIDER_BG_H,
    ) {
        return Region::Volume;
    }
    if in_rect(
        x,
        y,
        sprites::BALANCE_X,
        sprites::BALANCE_Y,
        sprites::BALANCE_W,
        sprites::SLIDER_BG_H,
    ) {
        return Region::Balance;
    }
    if in_rect(
        x,
        y,
        sprites::POSBAR_X,
        sprites::POSBAR_Y,
        sprites::POSBAR_W,
        sprites::POSBAR_H,
    ) {
        return Region::Position;
    }
    if in_rect(x, y, TIME_X, TIME_Y, TIME_W, TIME_H) {
        return Region::Time;
    }
    if in_rect(
        x,
        y,
        sprites::VIS_X,
        sprites::VIS_Y,
        sprites::VIS_W,
        sprites::VIS_H,
    ) {
        return Region::Vis;
    }
    if in_button(&sprites::EQ_OFF, x, y) {
        return Region::Toggle(WindowToggle::Equalizer);
    }
    if in_button(&sprites::PL_OFF, x, y) {
        return Region::Toggle(WindowToggle::Playlist);
    }
    if in_button(&sprites::SHUFFLE_OFF, x, y) {
        return Region::Mode(ModeButton::Shuffle);
    }
    if in_button(&sprites::REPEAT_OFF, x, y) {
        return Region::Mode(ModeButton::Repeat);
    }
    if y < TITLEBAR_H {
        return Region::TitleBar;
    }
    Region::None
}

/// Which region of the collapsed (windowshade) main window is at window-local (`x`, `y`)? The strip
/// carries the title buttons (same places as expanded), the mini transport click targets and the
/// mini seek bar (their art baked into the strip), and is otherwise the drag band. Points below the
/// [`sprites::MAIN_SHADE_H`] strip, or outside it, map to [`Region::None`].
pub fn hit_test_shade(x: i32, y: i32) -> Region {
    if x < 0 || y < 0 || x >= sprites::MAIN_W || y >= sprites::MAIN_SHADE_H {
        return Region::None;
    }
    // Title buttons keep their expanded destinations, so reuse those rectangles.
    for (placement, id) in sprites::TITLE_BUTTONS_PRESSED
        .iter()
        .zip(TITLE_BUTTON_ORDER)
    {
        if in_button(placement, x, y) {
            return Region::TitleButton(id);
        }
    }
    // The mini transport glyphs are click targets baked into the strip.
    for (&(rx, ry, rw, rh), id) in sprites::SHADE_TRANSPORT.iter().zip(TRANSPORT_ORDER) {
        if in_rect(x, y, rx, ry, rw, rh) {
            return Region::Transport(id);
        }
    }
    if in_rect(x, y, SHADE_TIME_X, SHADE_TIME_Y, SHADE_TIME_W, SHADE_TIME_H) {
        return Region::Time;
    }
    if in_rect(
        x,
        y,
        sprites::SHADE_POSBAR_X,
        sprites::SHADE_POSBAR_Y,
        sprites::SHADE_POSBAR_W,
        sprites::SHADE_POSBAR_H,
    ) {
        return Region::Position;
    }
    // The rest of the strip is the drag band (a double-click there also toggles shade, in `wl`).
    Region::TitleBar
}

/// The interactive region at (`x`, `y`), dispatched by window mode: the compact shade strip when
/// collapsed, else the full main window.
fn region_at(shade: bool, x: i32, y: i32) -> Region {
    if shade {
        hit_test_shade(x, y)
    } else {
        hit_test(x, y)
    }
}

/// The 0..=1 seek fraction for a pointer x, using the mini geometry while collapsed and the full
/// position bar when expanded, so a scrub maps correctly in either mode.
fn seek_fraction(shade: bool, x: i32) -> f32 {
    if shade {
        shade::seek_from_x(x)
    } else {
        posbar::position_from_x(x)
    }
}

/// The thumb pixel offset for a `fraction` in the current mode, used to skip a redraw when a drag
/// would not move the thumb.
fn seek_offset(shade: bool, fraction: f32) -> i32 {
    if shade {
        shade::seek_thumb_offset(fraction)
    } else {
        posbar::position_thumb_offset(fraction)
    }
}

/// Mutable UI state that drives composition: which button is held, the clock, the marquee, and
/// the slider values and in-progress drag. `Eq` is not derived because `position` is an `f32`.
#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    /// The transport button currently pressed (drawn depressed), or `None`.
    pub pressed: Option<Transport>,
    /// The title-bar button currently pressed (drawn depressed), or `None`.
    pub pressed_title: Option<TitleButton>,
    /// Whether the equalizer / playlist windows are open (their toggle buttons light while open).
    pub eq_open: bool,
    pub pl_open: bool,
    /// The EQ/PL toggle button currently held (drawn pressed), or `None`.
    pub pressed_toggle: Option<WindowToggle>,
    /// Whether shuffle / repeat modes are on (their buttons light while on).
    pub shuffle_on: bool,
    pub repeat_on: bool,
    /// The shuffle/repeat mode button currently held (drawn pressed), or `None`.
    pub pressed_mode: Option<ModeButton>,
    /// Whether the MM:SS clock shows elapsed time or a remaining-time countdown.
    pub time_display: TimeDisplay,
    /// Elapsed play time, in whole seconds, or `None` when nothing is loaded or stopped. The
    /// platform timer refreshes it once a second via [`on_tick`]. Composition combines this with
    /// [`Self::duration`] according to [`Self::time_display`].
    pub elapsed: Option<u32>,
    /// The song title shown in the marquee. Empty draws nothing. When it overruns the marquee
    /// width it scrolls, paced by the platform timer through [`crate::marquee::advance`].
    pub title: String,
    /// Horizontal scroll offset of the marquee, in pixels, wrapped over the looping title.
    /// Only meaningful while the title scrolls; held at 0 for a title that fits.
    pub marquee_offset: u32,
    /// Volume level, 0..=100. Defaults to full so a fresh window matches the engine's unity gain.
    pub volume: u8,
    /// Stereo balance, -100..=100 (negative pans left). Defaults to centered.
    pub balance: i8,
    /// The slider currently being dragged (its thumb drawn pressed), or `None`.
    pub dragging: Option<Slider>,
    /// Playback position as a 0..=1 fraction for the seek-bar thumb, or `None` when nothing is
    /// loaded or the length is unknown. Set from the clock each tick, except while the seek bar is
    /// being dragged, when it follows the cursor (a preview) until release commits the seek.
    pub position: Option<f32>,
    /// A committed seek target (0..=1 fraction) whose landing the engine's clock has not yet caught
    /// up to. While set, the display holds here and keyboard seeks accumulate from it, so a held
    /// arrow ramps smoothly instead of fighting the lagging clock. Cleared once the clock arrives.
    pub seek_target: Option<f32>,
    /// Total track length in whole seconds, or `None` when unknown. Kept so a seek-bar drag can
    /// preview the target time in the MM:SS display.
    pub duration: Option<u32>,
    /// The visualizer: its mode plus the per-frame spectrum/oscilloscope decay state. Stepped by
    /// the platform layer each frame from the audio scope tap; clicking the panel cycles the mode.
    pub vis: VisState,
    /// Bitrate (kbps) and sample rate (kHz) for the small readouts, `None` when nothing is loaded.
    pub kbps: Option<u32>,
    pub khz: Option<u32>,
    /// Channel count: 2 lights "stereo", 1 lights "mono", 0 (nothing loaded) dims both.
    pub channels: u8,
    /// Whether the main window is collapsed to its title strip (windowshade mode). Composition draws
    /// the compact strip and hit-testing uses the shade layout; the `wl` layer resizes the toplevel.
    pub shade: bool,
}

impl Default for UiState {
    fn default() -> Self {
        // Volume defaults to full (not 0) so a freshly opened window plays at unity, matching the
        // audio engine's default gain, and shows the volume thumb flush-right; balance centered.
        Self {
            pressed: None,
            pressed_title: None,
            eq_open: false,
            pl_open: false,
            pressed_toggle: None,
            shuffle_on: false,
            repeat_on: false,
            pressed_mode: None,
            time_display: TimeDisplay::Elapsed,
            elapsed: None,
            title: String::new(),
            marquee_offset: 0,
            volume: 100,
            balance: 0,
            dragging: None,
            position: None,
            seek_target: None,
            duration: None,
            vis: VisState::default(),
            kbps: None,
            khz: None,
            channels: 0,
            shade: false,
        }
    }
}

impl UiState {
    /// Whole seconds to draw in the clock, or `None` when the selected representation is not
    /// available. Remaining time needs both values and saturates at zero when an imprecise or stale
    /// playback clock briefly reports elapsed time beyond the track duration.
    pub fn displayed_time(&self) -> Option<u32> {
        match self.time_display {
            TimeDisplay::Elapsed => self.elapsed,
            TimeDisplay::Remaining => Some(self.duration?.saturating_sub(self.elapsed?)),
        }
    }
}

/// A snapshot of the playback clock for the display, polled once per redraw tick. Carries the
/// elapsed seconds (the MM:SS display), the 0..=1 position (the seek-bar thumb), and the total
/// duration in seconds (so a seek-bar drag can preview the target time). All `None` when nothing
/// is playing or the length is unknown. `Eq` is not derived because `position` is an `f32`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playback {
    pub elapsed: Option<u32>,
    pub position: Option<f32>,
    pub duration: Option<u32>,
    /// Whether audio is actively playing (not paused/stopped). Gates the visualizer animation: the
    /// platform layer feeds live samples while playing and silence otherwise, so it settles.
    pub playing: bool,
    /// Whether playback is STOPPED (as opposed to merely paused). Stop clears the visualizer to its
    /// baseline (a reset), while a pause freezes it on its last frame.
    pub stopped: bool,
    /// Bitrate in kbps, sample rate in kHz, and channel count, for the small indicators. `None`/0
    /// when nothing is loaded. Constant per track, but polled with the clock for simplicity.
    pub kbps: Option<u32>,
    pub khz: Option<u32>,
    pub channels: u8,
    /// The current track's marquee title, so switching tracks (a playlist) updates the marquee.
    /// Empty when nothing is loaded. (Not `Copy` because of this; the string is short.)
    pub title: String,
    /// Whether shuffle / repeat modes are on, so their main-window buttons light.
    pub shuffle: bool,
    pub repeat: bool,
}

/// What the platform layer should do after handling a pointer event. Every field defaults to
/// "nothing", so a handler sets only what applies. `Eq` is not derived because a `Seek` command
/// carries an `f32`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Outcome {
    /// Start an interactive window move (a title-bar press): hand the drag to the compositor.
    pub start_move: bool,
    /// A command to emit to the caller, if any.
    pub command: Option<Command>,
    /// A window action requested by a title-bar button (close, minimize, ...), for the platform
    /// layer to carry out. Distinct from `command`, which drives the audio engine.
    pub window: Option<TitleButton>,
    /// A request to toggle (open/close) the equalizer or playlist window, for the platform layer.
    pub toggle: Option<WindowToggle>,
    /// Whether UI state changed and the window should be recomposed and redrawn.
    pub redraw: bool,
}

/// While dragging the seek bar, show the drag target without seeking: move the thumb to `fraction`
/// and, when the duration is known, preview the target time in the MM:SS display. The real seek is
/// deferred to release, so this only touches display state.
fn preview_seek(state: &mut UiState, fraction: f32) {
    state.position = Some(fraction);
    if let Some(dur) = state.duration {
        state.elapsed = Some((fraction * dur as f32).round() as u32);
    }
}

/// Handle a left-button press at window-local (`x`, `y`), updating `state`. A title-bar press
/// asks for a move; a transport press arms the button; a volume/balance press begins a drag and
/// jumps the value to the click, emitting it immediately; a seek-bar press begins a drag and
/// previews the target (thumb + time) but emits nothing yet (the seek commits on release).
pub fn on_press(state: &mut UiState, x: i32, y: i32) -> Outcome {
    match region_at(state.shade, x, y) {
        Region::TitleBar => Outcome {
            start_move: true,
            ..Default::default()
        },
        Region::TitleButton(b) => {
            state.pressed_title = Some(b);
            Outcome {
                redraw: true,
                ..Default::default()
            }
        }
        Region::Transport(b) => {
            state.pressed = Some(b);
            Outcome {
                redraw: true,
                ..Default::default()
            }
        }
        Region::Volume => {
            state.dragging = Some(Slider::Volume);
            state.volume = slider::volume_from_x(x);
            Outcome {
                command: Some(Command::Volume(state.volume)),
                redraw: true,
                ..Default::default()
            }
        }
        Region::Balance => {
            state.dragging = Some(Slider::Balance);
            state.balance = slider::balance_from_x(x);
            Outcome {
                command: Some(Command::Balance(state.balance)),
                redraw: true,
                ..Default::default()
            }
        }
        Region::Position => {
            // Inert when the track length is unknown (an unseekable stream): without a duration a
            // click can't map to a seek target, so the bar doesn't respond, matching classic
            // Winamp. `duration` is set from the playback clock each tick.
            if state.duration.is_none() {
                return Outcome::default();
            }
            state.dragging = Some(Slider::Position);
            preview_seek(state, seek_fraction(state.shade, x));
            Outcome {
                redraw: true,
                ..Default::default()
            }
        }
        Region::Time => {
            state.time_display = state.time_display.toggled();
            Outcome {
                redraw: true,
                ..Default::default()
            }
        }
        Region::Vis => {
            // Clicking the panel cycles Bars -> Oscilloscope -> Off. Purely a display change, so no
            // command is emitted; the platform layer keeps stepping the (new) mode each frame.
            state.vis.cycle();
            Outcome {
                redraw: true,
                ..Default::default()
            }
        }
        Region::Toggle(t) => {
            // Arm the toggle button (drawn pressed); it fires on release over the same button.
            state.pressed_toggle = Some(t);
            Outcome {
                redraw: true,
                ..Default::default()
            }
        }
        Region::Mode(m) => {
            state.pressed_mode = Some(m);
            Outcome {
                redraw: true,
                ..Default::default()
            }
        }
        Region::None => Outcome::default(),
    }
}

/// Handle pointer motion at window-local (`x`, `y`). Only meaningful while a slider is being
/// dragged: it tracks the cursor, emitting the new value (and redrawing) when it changes.
/// Wayland keeps delivering motion during the implicit button grab even past the window edge,
/// and [`slider::volume_from_x`]/[`slider::balance_from_x`] clamp to the track, so dragging off
/// the side pins to an extreme rather than jumping.
pub fn on_motion(state: &mut UiState, x: i32, _y: i32) -> Outcome {
    match state.dragging {
        Some(Slider::Volume) => {
            let v = slider::volume_from_x(x);
            if v == state.volume {
                return Outcome::default();
            }
            state.volume = v;
            Outcome {
                command: Some(Command::Volume(v)),
                redraw: true,
                ..Default::default()
            }
        }
        Some(Slider::Balance) => {
            let b = slider::balance_from_x(x);
            if b == state.balance {
                return Outcome::default();
            }
            state.balance = b;
            Outcome {
                command: Some(Command::Balance(b)),
                redraw: true,
                ..Default::default()
            }
        }
        Some(Slider::Position) => {
            let f = seek_fraction(state.shade, x);
            // Preview only; the seek fires on release. Skip the redraw when the thumb would not
            // move (same pixel offset) so a jittery cursor does not recompose needlessly.
            let shade = state.shade;
            if seek_offset(shade, f) == state.position.map_or(-1, |p| seek_offset(shade, p)) {
                return Outcome::default();
            }
            preview_seek(state, f);
            Outcome {
                redraw: true,
                ..Default::default()
            }
        }
        None => Outcome::default(),
    }
}

/// Handle a left-button release at window-local (`x`, `y`), updating `state`. Ending a
/// volume/balance drag just swaps the thumb back to its normal sprite (the value was committed
/// live during the drag). Ending a seek-bar drag commits the seek now, once, to the previewed
/// position (so we issue one seek per drag, not one per pixel). Otherwise a transport command
/// fires only when the release lands on the same button that was pressed (releasing off the
/// button cancels), matching classic button behavior.
pub fn on_release(state: &mut UiState, x: i32, y: i32) -> Outcome {
    let shade = state.shade;
    if let Some(slider) = state.dragging.take() {
        let command = match slider {
            // Volume and balance already emitted their value live; only the seek bar defers. Hold
            // the released position as the seek target so the clock does not snap the thumb back to
            // the pre-seek spot before the engine lands there.
            Slider::Position => {
                state.seek_target = state.position;
                state.position.map(Command::Seek)
            }
            Slider::Volume | Slider::Balance => None,
        };
        return Outcome {
            command,
            redraw: true,
            ..Default::default()
        };
    }
    if let Some(b) = state.pressed_title.take() {
        // A title-bar button carries out its window action only if released over the same button.
        let fired = region_at(shade, x, y) == Region::TitleButton(b);
        return Outcome {
            window: fired.then_some(b),
            redraw: true,
            ..Default::default()
        };
    }
    if let Some(t) = state.pressed_toggle.take() {
        // The EQ/PL toggle fires only if released over the same button.
        let fired = region_at(shade, x, y) == Region::Toggle(t);
        return Outcome {
            toggle: fired.then_some(t),
            redraw: true,
            ..Default::default()
        };
    }
    if let Some(m) = state.pressed_mode.take() {
        // The shuffle/repeat mode toggle fires only if released over the same button.
        let fired = region_at(shade, x, y) == Region::Mode(m);
        return Outcome {
            command: fired.then_some(Command::ToggleMode(m)),
            redraw: true,
            ..Default::default()
        };
    }
    match state.pressed.take() {
        Some(b) => {
            let fired = region_at(shade, x, y) == Region::Transport(b);
            Outcome {
                command: fired.then_some(Command::Transport(b)),
                redraw: true,
                ..Default::default()
            }
        }
        None => Outcome::default(),
    }
}

/// Handle the pointer leaving the window: cancel any in-progress button press so a button never
/// stays stuck down. A slider drag is left alone: Wayland's implicit grab keeps sending motion
/// and the release past the edge, so the drag should continue rather than abort here. Returns
/// whether a redraw is needed.
pub fn on_leave(state: &mut UiState) -> bool {
    // Cancel any armed button so none stays stuck down.
    let transport = state.pressed.take().is_some();
    let title = state.pressed_title.take().is_some();
    let toggle = state.pressed_toggle.take().is_some();
    let mode = state.pressed_mode.take().is_some();
    transport || title || toggle || mode
}

/// Volume change per Up/Down key, in 0..=100 units. Webamp steps by 1 and real Winamp 2.x by a
/// small internal increment (~1-2%); we use 2 so a single tap is perceptible while OS key-repeat
/// still ramps it smoothly when the key is held.
const VOLUME_STEP: i32 = 2;

/// Seek distance per Left/Right key, in seconds. Classic Winamp and Webamp both seek 5 seconds.
const SEEK_STEP_SECS: f32 = 5.0;

/// How close (in seconds) the engine's clock must get to a committed seek target before the display
/// stops holding at the target and resumes following the clock. Smaller than one seek step so a
/// single tap still holds until the seek lands, and large enough to absorb seek/clock granularity.
const SEEK_SETTLE_SECS: f32 = 2.0;

/// Map a decoded key press to its effect on `state`, returning the [`Outcome`] for the platform
/// layer to carry out (emit a command, redraw). `is_repeat` is true when the key is auto-repeating
/// while held: the seek and volume keys ramp on repeat, but the transport keys fire once per
/// physical press (so holding `b` does not machine-gun through a playlist). Keys with no binding,
/// and transport keys on auto-repeat, return the empty outcome. This is the keyboard twin of
/// [`on_press`]: same command vocabulary, no pointer geometry.
pub fn on_key(state: &mut UiState, key: KeyPress, is_repeat: bool) -> Outcome {
    match key {
        // Transport: one action per press, never on auto-repeat. `x` restarts from the top (a
        // distinct command from the Play button, which does nothing while already playing).
        KeyPress::Char('z') => transport_key(is_repeat, Transport::Prev),
        KeyPress::Char('x') => restart_key(is_repeat),
        KeyPress::Char('c') => transport_key(is_repeat, Transport::Pause),
        KeyPress::Char('v') => transport_key(is_repeat, Transport::Stop),
        KeyPress::Char('b') => transport_key(is_repeat, Transport::Next),
        // Volume and seek ramp while the key is held.
        KeyPress::Up => volume_key(state, VOLUME_STEP),
        KeyPress::Down => volume_key(state, -VOLUME_STEP),
        KeyPress::Right => seek_key(state, SEEK_STEP_SECS),
        KeyPress::Left => seek_key(state, -SEEK_STEP_SECS),
        // Any other character key is unbound (for now: r/s/l and the rest arrive with the playlist
        // and equalizer).
        KeyPress::Char(_) => Outcome::default(),
    }
}

/// A transport shortcut: emit the command once, but swallow auto-repeats so a held key does not
/// re-fire (holding Next must not skip repeatedly). No redraw: unlike a mouse click there is no
/// button to draw depressed for a keystroke.
fn transport_key(is_repeat: bool, t: Transport) -> Outcome {
    if is_repeat {
        return Outcome::default();
    }
    Outcome {
        command: Some(Command::Transport(t)),
        ..Default::default()
    }
}

/// The restart hotkey (`x`): force the track from the top, once per press (auto-repeat swallowed).
/// Unlike the Play button it restarts even while already playing.
fn restart_key(is_repeat: bool) -> Outcome {
    if is_repeat {
        return Outcome::default();
    }
    Outcome {
        command: Some(Command::Restart),
        ..Default::default()
    }
}

/// Nudge the volume by `step` (clamped 0..=100), emitting the new value and redrawing the slider.
/// A no-op (no command, no redraw) once the value is already at the rail, so holding the key at
/// full or zero does not spam identical commands.
fn volume_key(state: &mut UiState, step: i32) -> Outcome {
    let v = (state.volume as i32 + step).clamp(0, 100) as u8;
    if v == state.volume {
        return Outcome::default();
    }
    state.volume = v;
    Outcome {
        command: Some(Command::Volume(v)),
        redraw: true,
        ..Default::default()
    }
}

/// Seek by `delta_secs` relative to the current position, emitting an absolute [`Command::Seek`]
/// fraction. Inert when the length or position is unknown (an unseekable stream, matching the
/// position bar) or while the bar is being dragged with the pointer (the drag owns the thumb).
/// The thumb and time are moved optimistically so the display responds at once even while paused
/// (the clock tick would otherwise lag up to a second); the engine's clock reconfirms them when
/// the seek lands.
fn seek_key(state: &mut UiState, delta_secs: f32) -> Outcome {
    let Some(dur) = state.duration else {
        return Outcome::default();
    };
    // Accumulate from a pending seek target if we have one (so a held key ramps steadily) rather
    // than the clock-driven position, which lags the seek and would make repeats flip-flop.
    let Some(base) = state.seek_target.or(state.position) else {
        return Outcome::default();
    };
    if dur == 0 || state.dragging == Some(Slider::Position) {
        return Outcome::default();
    }
    let target = (base * dur as f32 + delta_secs).clamp(0.0, dur as f32);
    let fraction = target / dur as f32;
    state.seek_target = Some(fraction);
    state.position = Some(fraction);
    state.elapsed = Some(target.round() as u32);
    Outcome {
        command: Some(Command::Seek(fraction)),
        redraw: true,
        ..Default::default()
    }
}

/// Refresh the display from the latest playback snapshot (elapsed seconds and seek-bar position,
/// both `None` when nothing is playing), updating `state`. Returns whether anything shown changed
/// and a redraw is needed, so the timer recomposes only when the display actually moves (not while
/// paused, where the clock holds and this returns `false`). While the user is dragging the seek
/// bar, the drag owns the thumb and the time preview, so the clock does not overwrite them; the
/// duration is still refreshed (it is constant per track and feeds the drag preview).
pub fn on_tick(state: &mut UiState, pb: Playback) -> bool {
    let mut changed = state.duration != pb.duration;
    state.duration = pb.duration;
    // The marquee title changes when the playlist moves to another track: adopt it and restart the
    // scroll from the left so the new title reads from its start.
    if state.title != pb.title {
        state.title = pb.title;
        state.marquee_offset = 0;
        changed = true;
    }
    // The kbps/kHz/channel indicators are constant per track; copy them and redraw on any change
    // (which is really just once, when a track loads or clears).
    if state.kbps != pb.kbps || state.khz != pb.khz || state.channels != pb.channels {
        state.kbps = pb.kbps;
        state.khz = pb.khz;
        state.channels = pb.channels;
        changed = true;
    }
    // Shuffle/repeat mode lights.
    if state.shuffle_on != pb.shuffle || state.repeat_on != pb.repeat {
        state.shuffle_on = pb.shuffle;
        state.repeat_on = pb.repeat;
        changed = true;
    }
    // The seek bar and time: a pointer drag owns them outright; after a committed seek (keyboard or
    // drag-release) hold the target until the engine's clock reaches it, so the thumb does not snap
    // back to the lagging pre-seek position (which made held-arrow seeks jitter back and forth).
    if state.dragging == Some(Slider::Position) {
        // The drag preview owns the thumb and time; nothing to sync from the clock here.
    } else if let Some(target) = state.seek_target {
        let reached = match (pb.position, state.duration) {
            (Some(p), Some(d)) if d > 0 => ((p - target) * d as f32).abs() < SEEK_SETTLE_SECS,
            // No usable clock (unseekable, ended, or unknown length): stop holding.
            _ => true,
        };
        if reached {
            state.seek_target = None;
            if state.elapsed != pb.elapsed {
                state.elapsed = pb.elapsed;
                changed = true;
            }
            if state.position != pb.position {
                state.position = pb.position;
                changed = true;
            }
        }
    } else {
        if state.elapsed != pb.elapsed {
            state.elapsed = pb.elapsed;
            changed = true;
        }
        if state.position != pb.position {
            state.position = pb.position;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_threshold_ignores_small_jitters_but_starts_on_a_real_drag() {
        // A click with no (or tiny) motion stays a click, so a near-miss on a title button does not
        // move the window.
        assert!(!exceeds_move_threshold(0, 0), "no motion is a click");
        assert!(!exceeds_move_threshold(3, 0), "a 3px jitter stays a click");
        assert!(
            !exceeds_move_threshold(-2, 2),
            "diagonal within the threshold"
        );
        assert!(
            !exceeds_move_threshold(0, 4),
            "exactly the threshold is not yet a drag"
        );
        // A deliberate drag past the threshold starts the move, in any direction.
        assert!(exceeds_move_threshold(5, 0), "5px horizontal is a drag");
        assert!(exceeds_move_threshold(0, -5), "upward drag counts");
        assert!(
            exceeds_move_threshold(4, 4),
            "diagonal beyond the threshold"
        );
    }

    #[test]
    fn title_bar_band_is_the_top_strip() {
        assert_eq!(TITLEBAR_H, 14);
        assert_eq!(hit_test(0, 0), Region::TitleBar, "top-left corner");
        assert_eq!(
            hit_test(274, 13),
            Region::TitleBar,
            "bottom-right of the band"
        );
        assert_eq!(hit_test(137, 7), Region::TitleBar, "middle of the band");
    }

    #[test]
    fn below_the_band_is_not_draggable() {
        assert_eq!(
            hit_test(0, 14),
            Region::None,
            "first row under the title bar"
        );
        assert_eq!(
            hit_test(137, 45),
            Region::None,
            "window body above the sliders"
        );
        assert_eq!(
            hit_test(274, 115),
            Region::None,
            "bottom-right of the window"
        );
    }

    #[test]
    fn points_outside_the_window_are_none() {
        assert_eq!(hit_test(-1, 5), Region::None);
        assert_eq!(hit_test(5, -1), Region::None);
        assert_eq!(hit_test(sprites::MAIN_W, 5), Region::None);
        assert_eq!(hit_test(5, sprites::MAIN_H), Region::None);
    }

    #[test]
    fn transport_buttons_hit_at_their_centers() {
        // Centers of the six buttons, derived from CBUTTONS destinations + sizes.
        let expect = [
            (Transport::Prev, 16 + 11, 88 + 9),
            (Transport::Play, 39 + 11, 88 + 9),
            (Transport::Pause, 62 + 11, 88 + 9),
            (Transport::Stop, 85 + 11, 88 + 9),
            (Transport::Next, 108 + 11, 88 + 9),
            (Transport::Eject, 136 + 11, 89 + 8),
        ];
        for (id, x, y) in expect {
            assert_eq!(hit_test(x, y), Region::Transport(id), "{id:?} at ({x},{y})");
        }
    }

    #[test]
    fn just_outside_a_button_is_not_a_hit() {
        // One pixel left of Play (Play starts at x=39) is still Previous' right edge or gap.
        assert_eq!(hit_test(38, 97), Region::Transport(Transport::Prev));
        // Below the button row (buttons end at y=88+18=106) is the body.
        assert_eq!(hit_test(50, 106), Region::None);
        // The gap between Next (ends x=130) and Eject (starts x=136) is not a button.
        assert_eq!(hit_test(132, 97), Region::None);
    }

    #[test]
    fn sliders_have_their_own_regions() {
        // Inside each slider's background rectangle.
        assert_eq!(
            hit_test(sprites::VOLUME_X, sprites::VOLUME_Y),
            Region::Volume,
            "volume top-left"
        );
        assert_eq!(
            hit_test(
                sprites::VOLUME_X + sprites::VOLUME_W - 1,
                sprites::VOLUME_Y + sprites::SLIDER_BG_H - 1
            ),
            Region::Volume,
            "volume bottom-right",
        );
        assert_eq!(
            hit_test(sprites::BALANCE_X, sprites::BALANCE_Y),
            Region::Balance,
            "balance top-left"
        );
        // The gap between the two sliders belongs to neither.
        assert_eq!(
            hit_test(sprites::VOLUME_X + sprites::VOLUME_W, sprites::VOLUME_Y),
            Region::None
        );
        // One row below the sliders is the body.
        assert_eq!(
            hit_test(sprites::VOLUME_X, sprites::VOLUME_Y + sprites::SLIDER_BG_H),
            Region::None
        );
    }

    #[test]
    fn default_volume_is_full_and_balance_centered() {
        let s = UiState::default();
        assert_eq!(s.volume, 100, "fresh window is at unity gain, not silent");
        assert_eq!(s.balance, 0, "centered");
        assert_eq!(s.dragging, None);
        assert_eq!(s.time_display, TimeDisplay::Elapsed);
        assert!(!s.shade, "the window starts expanded, not collapsed");
    }

    #[test]
    fn displayed_time_derives_a_saturating_countdown_without_hiding_elapsed_time() {
        let elapsed_only = UiState {
            elapsed: Some(70),
            ..Default::default()
        };
        assert_eq!(
            elapsed_only.displayed_time(),
            Some(70),
            "elapsed mode does not require a known duration"
        );

        let remaining = UiState {
            time_display: TimeDisplay::Remaining,
            elapsed: Some(70),
            duration: Some(200),
            ..Default::default()
        };
        assert_eq!(remaining.displayed_time(), Some(130));
        assert_eq!(
            UiState {
                elapsed: Some(201),
                ..remaining.clone()
            }
            .displayed_time(),
            Some(0),
            "a clock that briefly passes the duration clamps to zero"
        );
        assert_eq!(
            UiState {
                duration: None,
                ..remaining.clone()
            }
            .displayed_time(),
            None,
            "remaining time is unknown without a duration"
        );
        assert_eq!(
            UiState {
                elapsed: None,
                ..remaining
            }
            .displayed_time(),
            None,
            "stopped or unloaded playback stays blank"
        );
    }

    #[test]
    fn expanded_clock_has_an_exact_hit_region_and_toggles_on_press() {
        assert_eq!(hit_test(TIME_X, TIME_Y), Region::Time, "top-left");
        assert_eq!(
            hit_test(TIME_X + TIME_W - 1, TIME_Y + TIME_H - 1),
            Region::Time,
            "bottom-right"
        );
        assert_eq!(
            hit_test(TIME_X - 1, TIME_Y),
            Region::None,
            "left edge is out"
        );
        assert_eq!(
            hit_test(TIME_X + TIME_W, TIME_Y),
            Region::None,
            "right edge is out"
        );
        assert_eq!(
            hit_test(TIME_X, TIME_Y - 1),
            Region::None,
            "top edge is out"
        );
        assert_eq!(
            hit_test(TIME_X, TIME_Y + TIME_H),
            Region::None,
            "bottom edge is out"
        );

        let mut state = UiState::default();
        let out = on_press(&mut state, TIME_X + TIME_W / 2, TIME_Y + TIME_H / 2);
        assert_eq!(state.time_display, TimeDisplay::Remaining);
        assert_eq!(
            out,
            Outcome {
                redraw: true,
                ..Default::default()
            },
            "clock changes only render-owned state"
        );
        let out = on_press(&mut state, TIME_X + 1, TIME_Y + 1);
        assert_eq!(
            state.time_display,
            TimeDisplay::Elapsed,
            "second click toggles back"
        );
        assert!(out.redraw);
    }

    #[test]
    fn shade_hit_test_only_yields_strip_regions() {
        // Below the 14px strip, nothing is interactive.
        assert_eq!(
            hit_test_shade(10, 14),
            Region::None,
            "first row under the strip"
        );
        assert_eq!(hit_test_shade(10, 40), Region::None, "well below the strip");
        assert_eq!(hit_test_shade(-1, 5), Region::None, "left of the window");
        // The title buttons keep their expanded destinations.
        assert_eq!(
            hit_test_shade(254 + 4, 3 + 4),
            Region::TitleButton(TitleButton::Shade),
            "restore"
        );
        assert_eq!(
            hit_test_shade(264 + 4, 3 + 4),
            Region::TitleButton(TitleButton::Close)
        );
        assert_eq!(
            hit_test_shade(6 + 4, 3 + 4),
            Region::TitleButton(TitleButton::Options)
        );
        // The mini transport glyphs and the mini seek bar are their own regions.
        let (px, py, pw, _) = sprites::SHADE_TRANSPORT[1]; // play
        assert_eq!(
            hit_test_shade(px + pw / 2, py + 4),
            Region::Transport(Transport::Play)
        );
        let (ex, ey, ew, _) = sprites::SHADE_TRANSPORT[5]; // eject
        assert_eq!(
            hit_test_shade(ex + ew / 2, ey + 4),
            Region::Transport(Transport::Eject)
        );
        assert_eq!(
            hit_test_shade(sprites::SHADE_POSBAR_X + 2, sprites::SHADE_POSBAR_Y + 2),
            Region::Position,
            "mini seek bar",
        );
        assert_eq!(
            hit_test_shade(SHADE_TIME_X, SHADE_TIME_Y),
            Region::Time,
            "mini clock top-left",
        );
        assert_eq!(
            hit_test_shade(
                SHADE_TIME_X + SHADE_TIME_W - 1,
                SHADE_TIME_Y + SHADE_TIME_H - 1,
            ),
            Region::Time,
            "mini clock bottom-right",
        );
        assert_eq!(
            hit_test_shade(SHADE_TIME_X + SHADE_TIME_W, SHADE_TIME_Y),
            Region::TitleBar,
            "first pixel to the right remains draggable",
        );
        // Bare strip between the controls is the drag band.
        assert_eq!(
            hit_test_shade(90, 7),
            Region::TitleBar,
            "empty strip is draggable"
        );
    }

    #[test]
    fn shade_clock_toggles_the_same_typed_mode() {
        let mut state = UiState {
            shade: true,
            ..Default::default()
        };
        let out = on_press(
            &mut state,
            SHADE_TIME_X + SHADE_TIME_W / 2,
            SHADE_TIME_Y + SHADE_TIME_H / 2,
        );
        assert_eq!(state.time_display, TimeDisplay::Remaining);
        assert!(out.redraw && !out.start_move && out.command.is_none());
    }

    #[test]
    fn shade_transport_press_and_release_fires_the_command() {
        // While collapsed, a click on a baked-in transport glyph still arms and fires on release.
        let mut s = UiState {
            shade: true,
            ..Default::default()
        };
        let (px, py, pw, _) = sprites::SHADE_TRANSPORT[1]; // play
        let out = on_press(&mut s, px + pw / 2, py + 4);
        assert_eq!(s.pressed, Some(Transport::Play), "shade play arms");
        assert!(out.redraw);
        let out = on_release(&mut s, px + pw / 2, py + 4);
        assert_eq!(
            out.command,
            Some(Command::Transport(Transport::Play)),
            "shade play fires on release"
        );
    }

    #[test]
    fn shade_button_press_release_requests_the_window_action() {
        // Pressing and releasing the shade (restore) button in collapsed mode asks the platform to
        // toggle back, exactly as the button does when expanded.
        let mut s = UiState {
            shade: true,
            ..Default::default()
        };
        let (bx, by) = (254 + 4, 3 + 4);
        let out = on_press(&mut s, bx, by);
        assert_eq!(
            s.pressed_title,
            Some(TitleButton::Shade),
            "restore button arms"
        );
        assert!(out.redraw && !out.start_move);
        let out = on_release(&mut s, bx, by);
        assert_eq!(
            out.window,
            Some(TitleButton::Shade),
            "restore fires on release over it"
        );
    }

    #[test]
    fn shade_seek_press_uses_the_mini_geometry_not_the_full_posbar() {
        // A press on the mini seek bar while collapsed starts a Position drag mapped through the
        // mini geometry (far right pins to 1.0), previewing without seeking.
        let mut s = UiState {
            shade: true,
            duration: Some(100),
            ..Default::default()
        };
        let x = sprites::SHADE_POSBAR_X + sprites::SHADE_POSBAR_W - 1; // the right end of the track
        let out = on_press(&mut s, x, sprites::SHADE_POSBAR_Y + 3);
        assert_eq!(s.dragging, Some(Slider::Position), "mini seek drag begins");
        assert_eq!(s.position, Some(1.0), "pinned to the mini track end");
        assert_eq!(
            s.elapsed,
            Some(100),
            "time previews the end of a 100s track"
        );
        assert_eq!(out.command, None, "the seek still commits on release");
        // The same window x is NOT on the full-window seek bar, so this only works via the mini map.
        assert_eq!(
            hit_test(x, sprites::SHADE_POSBAR_Y + 3),
            Region::TitleBar,
            "expanded: that x is the title band"
        );
    }

    #[test]
    fn on_press_dispatches_by_shade_mode_at_the_same_point() {
        // A point on the expanded volume slider row is unreachable when collapsed (off the 14px
        // strip): the same coordinates arm a volume drag expanded, but do nothing collapsed.
        let (vx, vy) = (sprites::VOLUME_X + 2, sprites::VOLUME_Y + 2);
        let mut expanded = UiState::default();
        let out = on_press(&mut expanded, vx, vy);
        assert_eq!(
            expanded.dragging,
            Some(Slider::Volume),
            "expanded: volume drag"
        );
        assert!(out.redraw);
        let mut collapsed = UiState {
            shade: true,
            ..Default::default()
        };
        let out = on_press(&mut collapsed, vx, vy);
        assert_eq!(
            collapsed.dragging, None,
            "collapsed: that row is off the strip"
        );
        assert_eq!(out, Outcome::default());
    }

    #[test]
    fn press_on_a_button_arms_it_and_asks_for_redraw() {
        let mut s = UiState::default();
        let out = on_press(&mut s, 39 + 11, 88 + 9); // play center
        assert!(out.redraw && out.command.is_none() && !out.start_move);
        assert_eq!(s.pressed, Some(Transport::Play));
    }

    #[test]
    fn press_on_title_bar_starts_a_move_and_does_not_arm() {
        let mut s = UiState::default();
        let out = on_press(&mut s, 100, 5);
        assert!(out.start_move);
        assert_eq!(s.pressed, None);
        assert_eq!(s.dragging, None);
    }

    #[test]
    fn title_buttons_win_over_the_drag_band_and_fire_on_release() {
        // Close (264,3) and minimize (244,3) are their own regions, not the drag band.
        let (cx, cy) = (264 + 4, 3 + 4);
        assert_eq!(hit_test(cx, cy), Region::TitleButton(TitleButton::Close));
        assert_eq!(
            hit_test(244 + 4, 3 + 4),
            Region::TitleButton(TitleButton::Minimize)
        );
        // Bare title-bar area away from the buttons is still the drag band (no move suppressed).
        assert_eq!(hit_test(137, 5), Region::TitleBar);

        // Press arms the button (drawn pressed), starts no move, and fires no action yet.
        let mut s = UiState::default();
        let out = on_press(&mut s, cx, cy);
        assert_eq!(s.pressed_title, Some(TitleButton::Close));
        assert!(
            out.redraw && !out.start_move && out.window.is_none(),
            "arm only"
        );
        // Release over the same button carries out the window action.
        let out = on_release(&mut s, cx, cy);
        assert_eq!(
            out.window,
            Some(TitleButton::Close),
            "close fires on release over it"
        );
        assert_eq!(s.pressed_title, None);
    }

    #[test]
    fn eq_and_pl_toggle_buttons_arm_and_fire_on_release() {
        // From shufrep dests: EQ at (219,58), PL at (242,58), each 23x12.
        assert_eq!(
            hit_test(219 + 4, 58 + 4),
            Region::Toggle(WindowToggle::Equalizer)
        );
        assert_eq!(
            hit_test(242 + 4, 58 + 4),
            Region::Toggle(WindowToggle::Playlist)
        );
        // Press arms (drawn pressed), release over the same fires the toggle.
        let mut s = UiState::default();
        let out = on_press(&mut s, 242 + 4, 58 + 4);
        assert_eq!(s.pressed_toggle, Some(WindowToggle::Playlist));
        assert!(
            out.redraw && out.toggle.is_none(),
            "arm only, no toggle yet"
        );
        let out = on_release(&mut s, 242 + 4, 58 + 4);
        assert_eq!(
            out.toggle,
            Some(WindowToggle::Playlist),
            "PL toggles on release over it"
        );
        assert_eq!(s.pressed_toggle, None);
        // Released off the button cancels (dragged away).
        let mut s2 = UiState {
            pressed_toggle: Some(WindowToggle::Equalizer),
            ..Default::default()
        };
        let out = on_release(&mut s2, 137, 45);
        assert_eq!(out.toggle, None, "off-button = no toggle");
        assert!(out.redraw);
    }

    #[test]
    fn shuffle_and_repeat_buttons_arm_and_fire_toggle_mode() {
        // From shufrep dests: shuffle at (164,89) 47x15, repeat at (210,89) 28x15.
        assert_eq!(
            hit_test(164 + 10, 89 + 5),
            Region::Mode(ModeButton::Shuffle)
        );
        assert_eq!(hit_test(210 + 10, 89 + 5), Region::Mode(ModeButton::Repeat));
        let mut s = UiState::default();
        let out = on_press(&mut s, 164 + 10, 89 + 5);
        assert_eq!(s.pressed_mode, Some(ModeButton::Shuffle));
        assert!(out.redraw && out.command.is_none(), "arm only");
        let out = on_release(&mut s, 164 + 10, 89 + 5);
        assert_eq!(
            out.command,
            Some(Command::ToggleMode(ModeButton::Shuffle)),
            "fires on release"
        );
        assert_eq!(s.pressed_mode, None);
        // Released off the button cancels.
        let mut s2 = UiState {
            pressed_mode: Some(ModeButton::Repeat),
            ..Default::default()
        };
        let out = on_release(&mut s2, 137, 45);
        assert_eq!(out.command, None, "off-button = no toggle");
    }

    #[test]
    fn title_button_released_off_the_button_cancels() {
        let mut s = UiState {
            pressed_title: Some(TitleButton::Close),
            ..Default::default()
        };
        let out = on_release(&mut s, 137, 45); // released over the window body
        assert_eq!(out.window, None, "dragged off the button = no action");
        assert!(out.redraw, "still redraw to un-press");
        assert_eq!(s.pressed_title, None);
    }

    #[test]
    fn leave_clears_a_pressed_title_button() {
        let mut s = UiState {
            pressed_title: Some(TitleButton::Minimize),
            ..Default::default()
        };
        assert!(
            on_leave(&mut s),
            "needs redraw to un-press the title button"
        );
        assert_eq!(s.pressed_title, None);
    }

    #[test]
    fn press_on_volume_begins_a_drag_sets_the_value_and_emits() {
        let mut s = UiState::default();
        // Press near the far-left of the volume track: value pins low, drag begins.
        let out = on_press(&mut s, sprites::VOLUME_X, sprites::VOLUME_Y + 2);
        assert_eq!(s.dragging, Some(Slider::Volume));
        assert_eq!(s.volume, slider::volume_from_x(sprites::VOLUME_X));
        assert_eq!(out.command, Some(Command::Volume(s.volume)));
        assert!(out.redraw);
    }

    #[test]
    fn press_on_balance_center_sets_zero_and_drags() {
        let mut s = UiState::default();
        let x = sprites::BALANCE_X + sprites::BALANCE_W / 2;
        let out = on_press(&mut s, x, sprites::BALANCE_Y + 2);
        assert_eq!(s.dragging, Some(Slider::Balance));
        assert_eq!(s.balance, 0, "clicking the middle centers the balance");
        assert_eq!(out.command, Some(Command::Balance(0)));
    }

    #[test]
    fn motion_while_dragging_volume_tracks_and_emits_only_on_change() {
        let mut s = UiState {
            dragging: Some(Slider::Volume),
            volume: 0,
            ..Default::default()
        };
        // Move to the far right: volume jumps to 100.
        let out = on_motion(&mut s, sprites::VOLUME_X + 1000, sprites::VOLUME_Y);
        assert_eq!(s.volume, 100);
        assert_eq!(out.command, Some(Command::Volume(100)));
        assert!(out.redraw);
        // A second motion to the same place changes nothing: no command, no redraw.
        let out = on_motion(&mut s, sprites::VOLUME_X + 1000, sprites::VOLUME_Y);
        assert_eq!(out, Outcome::default());
    }

    #[test]
    fn motion_while_dragging_balance_tracks_and_emits_only_on_change() {
        let mut s = UiState {
            dragging: Some(Slider::Balance),
            balance: 0,
            ..Default::default()
        };
        // Drag off the left edge: balance pins to -100 and emits a Balance command (not Volume,
        // and mapped through balance_from_x, so a copy-paste of the volume arm would fail here).
        let out = on_motion(&mut s, sprites::BALANCE_X - 1000, sprites::BALANCE_Y);
        assert_eq!(s.balance, -100);
        assert_eq!(out.command, Some(Command::Balance(-100)));
        assert!(out.redraw);
        // Staying at the same value emits nothing (the de-dup path).
        let out = on_motion(&mut s, sprites::BALANCE_X - 1000, sprites::BALANCE_Y);
        assert_eq!(out, Outcome::default());
        // Drag off the right edge: balance pins to +100.
        let out = on_motion(&mut s, sprites::BALANCE_X + 1000, sprites::BALANCE_Y);
        assert_eq!(s.balance, 100);
        assert_eq!(out.command, Some(Command::Balance(100)));
    }

    #[test]
    fn motion_without_a_drag_is_inert() {
        let mut s = UiState::default();
        let out = on_motion(&mut s, 137, 60);
        assert_eq!(out, Outcome::default());
        assert_eq!(s.volume, 100, "hover does not touch the value");
    }

    #[test]
    fn release_ends_a_slider_drag_and_redraws() {
        let mut s = UiState {
            dragging: Some(Slider::Volume),
            volume: 42,
            ..Default::default()
        };
        let out = on_release(&mut s, 500, 500); // released anywhere
        assert_eq!(s.dragging, None, "drag ended");
        assert_eq!(s.volume, 42, "value held from the drag");
        assert_eq!(
            out.command, None,
            "no new command on release; value already emitted"
        );
        assert!(out.redraw, "redraw to restore the normal thumb sprite");
    }

    #[test]
    fn release_over_the_same_button_fires_the_command() {
        let mut s = UiState {
            pressed: Some(Transport::Play),
            ..Default::default()
        };
        let out = on_release(&mut s, 39 + 11, 88 + 9);
        assert_eq!(out.command, Some(Command::Transport(Transport::Play)));
        assert!(out.redraw);
        assert_eq!(s.pressed, None, "button released");
    }

    #[test]
    fn release_off_the_button_cancels_the_command() {
        let mut s = UiState {
            pressed: Some(Transport::Play),
            ..Default::default()
        };
        let out = on_release(&mut s, 137, 45); // released over the body
        assert_eq!(out.command, None, "dragged off = cancel");
        assert!(out.redraw, "still redraw to un-press");
        assert_eq!(s.pressed, None);
    }

    #[test]
    fn release_with_nothing_pressed_is_a_no_op() {
        let mut s = UiState::default();
        let out = on_release(&mut s, 39 + 11, 88 + 9);
        assert_eq!(out, Outcome::default());
    }

    #[test]
    fn leave_clears_a_pressed_button_but_not_a_drag() {
        let mut s = UiState {
            pressed: Some(Transport::Stop),
            ..Default::default()
        };
        assert!(on_leave(&mut s), "needs redraw to un-press");
        assert_eq!(s.pressed, None);
        assert!(!on_leave(&mut s), "nothing pressed now");

        // A drag survives a leave (the implicit grab keeps delivering motion past the edge).
        let mut d = UiState {
            dragging: Some(Slider::Volume),
            ..Default::default()
        };
        assert!(!on_leave(&mut d), "leaving mid-drag needs no redraw");
        assert_eq!(d.dragging, Some(Slider::Volume), "drag continues");
    }

    /// A clock snapshot carrying only an elapsed value (no position/duration), for the tick tests
    /// that predate the seek bar.
    fn elapsed(secs: Option<u32>) -> Playback {
        Playback {
            elapsed: secs,
            ..Default::default()
        }
    }

    #[test]
    fn tick_redraws_only_when_the_shown_time_changes() {
        let mut s = UiState::default();
        assert!(
            on_tick(&mut s, elapsed(Some(0))),
            "blank -> 00:00 is a change"
        );
        assert_eq!(s.elapsed, Some(0));
        assert!(
            !on_tick(&mut s, elapsed(Some(0))),
            "same second (e.g. paused): no redraw"
        );
        assert!(on_tick(&mut s, elapsed(Some(1))), "next second: redraw");
        assert!(
            on_tick(&mut s, elapsed(None)),
            "stop blanks the display: redraw"
        );
        assert_eq!(s.elapsed, None);
        assert!(!on_tick(&mut s, elapsed(None)), "still blank: no redraw");
    }

    #[test]
    fn posbar_has_its_own_region() {
        assert_eq!(
            hit_test(sprites::POSBAR_X, sprites::POSBAR_Y),
            Region::Position,
            "posbar top-left"
        );
        assert_eq!(
            hit_test(
                sprites::POSBAR_X + sprites::POSBAR_W - 1,
                sprites::POSBAR_Y + sprites::POSBAR_H - 1
            ),
            Region::Position,
            "posbar bottom-right",
        );
        // One row below the bar is the body.
        assert_eq!(
            hit_test(sprites::POSBAR_X, sprites::POSBAR_Y + sprites::POSBAR_H),
            Region::None
        );
    }

    #[test]
    fn vis_region_click_cycles_the_mode() {
        use crate::vis::VisMode;
        // The panel is its own region.
        assert_eq!(
            hit_test(sprites::VIS_X, sprites::VIS_Y),
            Region::Vis,
            "vis top-left"
        );
        assert_eq!(
            hit_test(
                sprites::VIS_X + sprites::VIS_W - 1,
                sprites::VIS_Y + sprites::VIS_H - 1
            ),
            Region::Vis,
            "vis bottom-right",
        );
        // Clicking it cycles the mode and redraws, emitting no command (a display-only change).
        let mut s = UiState::default();
        assert_eq!(s.vis.mode, VisMode::Bars);
        let out = on_press(&mut s, sprites::VIS_X + 10, sprites::VIS_Y + 8);
        assert_eq!(
            s.vis.mode,
            VisMode::Oscilloscope,
            "one click advances the mode"
        );
        assert!(out.redraw && out.command.is_none() && !out.start_move);
        on_press(&mut s, sprites::VIS_X + 10, sprites::VIS_Y + 8);
        on_press(&mut s, sprites::VIS_X + 10, sprites::VIS_Y + 8);
        assert_eq!(s.vis.mode, VisMode::Bars, "three clicks wrap back to bars");
    }

    #[test]
    fn press_on_posbar_begins_a_drag_previews_and_does_not_seek_yet() {
        let mut s = UiState {
            duration: Some(200), // 3:20 track, so the preview time is meaningful
            ..Default::default()
        };
        // Press at the far-right edge of the track (still inside the window): the thumb pins to
        // the end and the time previews the track length, but NO command fires (seek on release).
        let out = on_press(
            &mut s,
            sprites::POSBAR_X + sprites::POSBAR_W - 1,
            sprites::POSBAR_Y + 5,
        );
        assert_eq!(s.dragging, Some(Slider::Position));
        assert_eq!(s.position, Some(1.0), "thumb pinned to the end");
        assert_eq!(
            s.elapsed,
            Some(200),
            "time preview at the end of a 200s track"
        );
        assert_eq!(out.command, None, "no seek on press; it commits on release");
        assert!(out.redraw);
    }

    #[test]
    fn posbar_is_inert_when_the_track_length_is_unknown() {
        // No duration (an unseekable / headerless stream): a press starts no drag, previews
        // nothing, and emits nothing, so the bar cannot phantom-scrub and then snap back.
        let mut s = UiState {
            duration: None,
            ..Default::default()
        };
        let out = on_press(&mut s, sprites::POSBAR_X + 50, sprites::POSBAR_Y + 5);
        assert_eq!(s.dragging, None, "no drag begins without a known length");
        assert_eq!(s.position, None, "the thumb is not moved");
        assert_eq!(out, Outcome::default(), "no redraw and no command");
    }

    #[test]
    fn dragging_the_posbar_tracks_but_only_release_seeks() {
        let mut s = UiState {
            dragging: Some(Slider::Position),
            duration: Some(100),
            position: Some(0.0),
            ..Default::default()
        };
        // Motion to mid-track moves the thumb and previews ~50s, but emits nothing.
        let out = on_motion(
            &mut s,
            sprites::POSBAR_X + posbar::POSBAR_TRAVEL / 2 + 14,
            sprites::POSBAR_Y,
        );
        assert!(
            (s.position.unwrap() - 0.5).abs() < 0.02,
            "thumb near mid (got {:?})",
            s.position
        );
        assert_eq!(
            s.elapsed,
            Some((s.position.unwrap() * 100.0).round() as u32),
            "time previews the target"
        );
        assert_eq!(out.command, None, "still no seek during the drag");
        assert!(out.redraw);

        // Release commits exactly one Seek to the previewed fraction, and ends the drag.
        let previewed = s.position.unwrap();
        let out = on_release(&mut s, 0, 0);
        assert_eq!(s.dragging, None, "drag ended");
        assert_eq!(
            out.command,
            Some(Command::Seek(previewed)),
            "seek commits on release"
        );
        assert!(out.redraw);
    }

    #[test]
    fn releasing_volume_or_balance_still_emits_no_command() {
        // The seek bar's release-to-commit must not leak into the other sliders, which committed
        // live during their drag.
        for slider in [Slider::Volume, Slider::Balance] {
            let mut s = UiState {
                dragging: Some(slider),
                ..Default::default()
            };
            let out = on_release(&mut s, 500, 500);
            assert_eq!(out.command, None, "{slider:?} release emits nothing");
            assert!(
                out.redraw,
                "{slider:?} release still restores the normal thumb"
            );
        }
    }

    #[test]
    fn tick_updates_the_posbar_position_but_yields_to_a_drag() {
        let mut s = UiState::default();
        // Normal tick: the clock sets elapsed, position, and duration.
        assert!(on_tick(
            &mut s,
            Playback {
                elapsed: Some(30),
                position: Some(0.25),
                duration: Some(120),
                playing: true,
                ..Default::default()
            }
        ));
        assert_eq!(
            (s.elapsed, s.position, s.duration),
            (Some(30), Some(0.25), Some(120))
        );

        // During a seek-bar drag the clock must not fight the preview: elapsed and position hold.
        // The (normally constant) duration is still refreshed though, so feed a CHANGED duration
        // to prove it updates mid-drag and forces a redraw while the preview is left untouched.
        s.dragging = Some(Slider::Position);
        s.elapsed = Some(90);
        s.position = Some(0.75);
        let changed = on_tick(
            &mut s,
            Playback {
                elapsed: Some(31),
                position: Some(0.26),
                duration: Some(130),
                playing: true,
                ..Default::default()
            },
        );
        assert!(changed, "a changed duration still redraws mid-drag");
        assert_eq!(s.duration, Some(130), "duration refreshes during the drag");
        assert_eq!(
            (s.elapsed, s.position),
            (Some(90), Some(0.75)),
            "preview held against the clock"
        );
        // With the duration unchanged, a drag-phase tick changes nothing at all.
        let changed = on_tick(
            &mut s,
            Playback {
                elapsed: Some(32),
                position: Some(0.27),
                duration: Some(130),
                playing: true,
                ..Default::default()
            },
        );
        assert!(
            !changed,
            "no redraw: the drag owns the display and duration was unchanged"
        );
        assert_eq!(
            (s.elapsed, s.position),
            (Some(90), Some(0.75)),
            "preview still held"
        );
    }

    #[test]
    fn transport_keys_emit_the_command_once_and_swallow_repeat() {
        // The transport letters (x is the restart key, tested separately), matched on their char.
        let keys = [
            ('z', Transport::Prev),
            ('c', Transport::Pause),
            ('v', Transport::Stop),
            ('b', Transport::Next),
        ];
        for (ch, t) in keys {
            let mut s = UiState::default();
            let out = on_key(&mut s, KeyPress::Char(ch), false);
            assert_eq!(out.command, Some(Command::Transport(t)), "{ch} -> {t:?}");
            assert!(
                !out.redraw,
                "a transport keystroke draws no depressed button"
            );
            // Held: the auto-repeat must not re-fire the transport action.
            let repeat = on_key(&mut s, KeyPress::Char(ch), true);
            assert_eq!(
                repeat,
                Outcome::default(),
                "{ch} held emits nothing on repeat"
            );
        }
    }

    #[test]
    fn x_key_restarts_and_is_distinct_from_the_play_button() {
        // `x` emits Restart (force from the top), not Transport(Play): it is the restart hotkey,
        // distinct from the Play button. Fires once per press, swallowed on auto-repeat.
        let mut s = UiState::default();
        let out = on_key(&mut s, KeyPress::Char('x'), false);
        assert_eq!(out.command, Some(Command::Restart), "x restarts");
        assert!(!out.redraw);
        assert_eq!(
            on_key(&mut s, KeyPress::Char('x'), true),
            Outcome::default(),
            "x held: no repeat"
        );
    }

    #[test]
    fn unbound_keys_do_nothing() {
        let mut s = UiState::default();
        for key in [
            KeyPress::Char('q'),
            KeyPress::Char('1'),
            KeyPress::Char(' '),
        ] {
            assert_eq!(
                on_key(&mut s, key, false),
                Outcome::default(),
                "{key:?} is unbound"
            );
        }
    }

    #[test]
    fn volume_keys_step_clamp_and_dedup() {
        // Fresh volume is 100 (full): Up is already at the rail, so it is a no-op.
        let mut s = UiState::default();
        assert_eq!(
            on_key(&mut s, KeyPress::Up, false),
            Outcome::default(),
            "already at 100"
        );
        assert_eq!(s.volume, 100);
        // Down steps by VOLUME_STEP and emits + redraws.
        let out = on_key(&mut s, KeyPress::Down, false);
        assert_eq!(s.volume, 98);
        assert_eq!(out.command, Some(Command::Volume(98)));
        assert!(out.redraw);
        // Repeats ramp (the auto-repeat is allowed through for volume).
        let out = on_key(&mut s, KeyPress::Down, true);
        assert_eq!(s.volume, 96, "held Down keeps ramping");
        assert_eq!(out.command, Some(Command::Volume(96)));
        // Near the bottom, the step clamps to 0 rather than underflowing.
        s.volume = 1;
        let out = on_key(&mut s, KeyPress::Down, false);
        assert_eq!(s.volume, 0);
        assert_eq!(out.command, Some(Command::Volume(0)));
        // At 0, Down is a no-op (no spam of identical commands while held at the rail).
        assert_eq!(
            on_key(&mut s, KeyPress::Down, true),
            Outcome::default(),
            "pinned at 0"
        );
        // And Up clamps to 100 from the top.
        s.volume = 99;
        let out = on_key(&mut s, KeyPress::Up, false);
        assert_eq!(s.volume, 100);
        assert_eq!(out.command, Some(Command::Volume(100)));
    }

    #[test]
    fn seek_keys_move_relative_and_clamp_to_the_track() {
        // Pull the fraction out of a Seek command (f32, so compared with a tolerance).
        fn seek_frac(out: &Outcome) -> f32 {
            match out.command {
                Some(Command::Seek(f)) => f,
                other => panic!("expected a Seek command, got {other:?}"),
            }
        }
        // A 100s track at the halfway point: Right adds 5s, Left subtracts 5s.
        let mut s = UiState {
            duration: Some(100),
            position: Some(0.5),
            ..Default::default()
        };
        let out = on_key(&mut s, KeyPress::Right, false);
        assert!((seek_frac(&out) - 0.55).abs() < 1e-6, "50s + 5s = 55%");
        assert_eq!(s.elapsed, Some(55), "time preview updated at once");
        assert!(
            (s.position.unwrap() - 0.55).abs() < 1e-6,
            "thumb moved optimistically"
        );
        assert!(out.redraw);

        // Reset the pending target too, to test an independent seek from the halfway point (a held
        // key instead accumulates, which its own test covers).
        s.seek_target = None;
        s.position = Some(0.5);
        let out = on_key(&mut s, KeyPress::Left, false);
        assert!((seek_frac(&out) - 0.45).abs() < 1e-6, "50s - 5s = 45%");

        // Near the ends, the target clamps into [0, duration] (0.0 and 1.0 are exact).
        s.seek_target = None;
        s.position = Some(0.02); // 2s in
        let out = on_key(&mut s, KeyPress::Left, false);
        assert_eq!(
            out.command,
            Some(Command::Seek(0.0)),
            "clamps to the start, not negative"
        );
        assert_eq!(s.elapsed, Some(0));
        s.seek_target = None;
        s.position = Some(0.99); // 99s in
        let out = on_key(&mut s, KeyPress::Right, true); // repeat also seeks
        assert_eq!(out.command, Some(Command::Seek(1.0)), "clamps to the end");
        assert_eq!(s.elapsed, Some(100));
    }

    #[test]
    fn seek_keys_are_inert_without_a_length_or_during_a_pointer_drag() {
        // No duration/position (an unseekable or not-yet-playing stream): nothing happens.
        let mut s = UiState::default();
        assert_eq!(
            on_key(&mut s, KeyPress::Right, false),
            Outcome::default(),
            "no length"
        );
        assert_eq!(
            on_key(&mut s, KeyPress::Left, false),
            Outcome::default(),
            "no length"
        );

        // A known length but the pointer is mid-drag on the bar: the drag owns the thumb, so the
        // key must not fight it.
        let mut d = UiState {
            duration: Some(100),
            position: Some(0.5),
            dragging: Some(Slider::Position),
            ..Default::default()
        };
        assert_eq!(
            on_key(&mut d, KeyPress::Right, false),
            Outcome::default(),
            "yields to the drag"
        );
        assert_eq!(d.position, Some(0.5), "preview untouched");
    }

    #[test]
    fn held_seek_accumulates_against_the_target_not_the_lagging_clock() {
        let mut s = UiState {
            duration: Some(200),
            position: Some(0.5),
            ..Default::default()
        };
        // First Right: 100s -> 105s, and it records the target.
        let o1 = on_key(&mut s, KeyPress::Right, false);
        assert!(matches!(o1.command, Some(Command::Seek(f)) if (f - 105.0 / 200.0).abs() < 1e-4));
        assert_eq!(s.seek_target, Some(105.0 / 200.0));
        // A clock tick arrives still reporting the OLD position (the async seek has not landed): the
        // display must hold at the target, not snap back to 100s.
        on_tick(
            &mut s,
            Playback {
                position: Some(0.5),
                elapsed: Some(100),
                duration: Some(200),
                ..Default::default()
            },
        );
        assert_eq!(
            s.position,
            Some(105.0 / 200.0),
            "held at the target while the clock lags"
        );
        assert_eq!(s.seek_target, Some(105.0 / 200.0));
        // Second Right (auto-repeat) accumulates from the target -> 110s, not from the lagging clock.
        let o2 = on_key(&mut s, KeyPress::Right, true);
        assert!(matches!(o2.command, Some(Command::Seek(f)) if (f - 110.0 / 200.0).abs() < 1e-4));
    }

    #[test]
    fn the_display_resumes_the_clock_once_the_seek_lands() {
        let mut s = UiState {
            duration: Some(200),
            position: Some(0.5),
            ..Default::default()
        };
        on_key(&mut s, KeyPress::Right, false); // target 105s
                                                // The engine lands near 105s: the hold clears and the clock takes over.
        on_tick(
            &mut s,
            Playback {
                position: Some(105.0 / 200.0),
                elapsed: Some(105),
                duration: Some(200),
                ..Default::default()
            },
        );
        assert_eq!(
            s.seek_target, None,
            "cleared once the clock reached the target"
        );
        assert_eq!(s.position, Some(105.0 / 200.0));
        // A normal tick now advances freely.
        on_tick(
            &mut s,
            Playback {
                position: Some(106.0 / 200.0),
                elapsed: Some(106),
                duration: Some(200),
                ..Default::default()
            },
        );
        assert_eq!(s.position, Some(106.0 / 200.0));
    }

    #[test]
    fn a_backward_seek_holds_until_the_clock_falls_back() {
        let mut s = UiState {
            duration: Some(200),
            position: Some(0.5),
            ..Default::default()
        };
        on_key(&mut s, KeyPress::Left, false); // 100s -> 95s
        assert_eq!(s.seek_target, Some(95.0 / 200.0));
        // Clock still at 100s: must NOT snap forward to 100.
        on_tick(
            &mut s,
            Playback {
                position: Some(0.5),
                elapsed: Some(100),
                duration: Some(200),
                ..Default::default()
            },
        );
        assert_eq!(
            s.position,
            Some(95.0 / 200.0),
            "backward seek holds at the target"
        );
        // Engine falls back to 95s: resume the clock.
        on_tick(
            &mut s,
            Playback {
                position: Some(95.0 / 200.0),
                elapsed: Some(95),
                duration: Some(200),
                ..Default::default()
            },
        );
        assert_eq!(s.seek_target, None);
    }
}
