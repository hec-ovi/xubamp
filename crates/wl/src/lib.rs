//! Native Wayland window: one undecorated `xdg_toplevel` backed by a `wl_shm` software
//! buffer that receives a rendered `Framebuffer`. Target is GNOME 50 / Mutter, no toolkit.
//!
//! The Wayland plumbing (registry, shm slot pool, xdg window) is handled by
//! smithay-client-toolkit; we still own every pixel by blitting our own `Framebuffer`
//! into the shm buffer. This layer needs a live compositor, so it is verified by running
//! on Ubuntu 26.04 rather than by unit tests.

mod panes;

use std::error::Error;
use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::calloop::{
    timer::{TimeoutAction, Timer},
    EventLoop,
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_pointer, delegate_registry, delegate_seat,
    delegate_shm, delegate_subcompositor, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{
            CursorIcon, PointerData, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec,
            ThemedPointer, BTN_LEFT, BTN_RIGHT,
        },
        Capability, SeatHandler, SeatState,
    },
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell, XdgSurface,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
    subcompositor::SubcompositorState,
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_subsurface, wl_surface},
    Connection, QueueHandle,
};
use xubamp_render::vis::{VisMode, FFT_N};
use xubamp_render::{
    compose_main_window, equalizer, hit, jump, marquee, menu, pledit, Framebuffer,
};
use xubamp_skin::Skin;

// Keyboard shortcuts are gated behind the `keyboard` feature so the host build stays free of the
// libxkbcommon build dependency (see Cargo.toml). These imports exist only when it is enabled.
#[cfg(feature = "keyboard")]
use smithay_client_toolkit::delegate_keyboard;
#[cfg(feature = "keyboard")]
use smithay_client_toolkit::reexports::calloop::LoopHandle;
#[cfg(feature = "keyboard")]
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers};
#[cfg(feature = "keyboard")]
use wayland_client::protocol::wl_keyboard;

/// How often the marquee steps while a title is scrolling. Between scrolls the redraw timer
/// falls back to a once-a-second cadence for the clock, so an idle window stays cheap.
const MARQUEE_TICK: Duration = Duration::from_millis(100);

/// While the frame-callback loop drives the visualizer at the display's refresh rate (see
/// [`App::draw`]/[`App::on_frame`]), the timer only needs to poll the clock and re-arm the loop if
/// it ever stalls; a slow cadence keeps that cheap. When paused the visualizer is frozen, so the
/// timer just keeps the clock and marquee moving.
const FRAME_FALLBACK: Duration = Duration::from_millis(250);

/// Redraw cadence while the visualizer settles to baseline after a Stop (~30 fps): a few frames of
/// smooth decay, then it goes quiet.
const VIS_SETTLE_TICK: Duration = Duration::from_millis(33);

/// Two playlist clicks on the same row within this window count as a double-click, which plays it.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Tracks scrolled per mouse-wheel notch over the playlist list.
const WHEEL_TRACKS_PER_NOTCH: f32 = 3.0;

/// Fills a caller-owned buffer with the most recent output samples (mono, oldest first) for the
/// visualizer to read each frame.
type SampleSource = Box<dyn FnMut(&mut [f32])>;

/// Returns the playlist rows and the index of the currently-playing track, polled each tick so the
/// playlist window follows track changes. The window layer keeps selection/scroll itself.
type PlaylistSource = Box<dyn FnMut() -> (Vec<pledit::Row>, Option<usize>)>;

/// Applies equalizer-specific controls to the player/configuration owner.
type EqualizerSink = Box<dyn FnMut(equalizer::Command)>;

/// A popup-menu request whose side effect belongs to the application layer. Saving an equalizer
/// preset carries the current live curve because the window layer owns slider interaction state.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuRequest {
    Action(menu::ClassicMenuAction),
    /// Main/playlist Eject and Play-on-empty use the file chooser's replace-and-play behavior.
    OpenMedia,
    SaveEqualizer(equalizer::Preset),
}

type MenuSink = Box<dyn FnMut(MenuRequest)>;

/// An event produced outside the Wayland thread and applied on the next event-loop tick.
#[derive(Clone, Debug, PartialEq)]
pub enum ExternalEvent {
    EqualizerPreset(equalizer::Preset),
}

/// Result of polling application work such as a desktop-portal dialog. `pending` keeps the poll
/// cadence responsive without waking the idle player continuously.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExternalPoll {
    pub events: Vec<ExternalEvent>,
    pub pending: bool,
}

type ExternalSource = Box<dyn FnMut() -> ExternalPoll>;

/// Event sinks and live data sources used by the Wayland event loop. Grouping these keeps the
/// window constructor stable as secondary panes gain controls.
pub struct Runtime {
    on_command: Box<dyn FnMut(hit::Command)>,
    on_equalizer: EqualizerSink,
    on_menu: MenuSink,
    equalizer_presets: Vec<equalizer::Preset>,
    playback_source: Box<dyn FnMut() -> hit::Playback>,
    sample_source: SampleSource,
    playlist_source: PlaylistSource,
    external_source: ExternalSource,
}

impl Runtime {
    pub fn new(
        on_command: impl FnMut(hit::Command) + 'static,
        on_equalizer: impl FnMut(equalizer::Command) + 'static,
        on_menu: impl FnMut(MenuRequest) + 'static,
        equalizer_presets: Vec<equalizer::Preset>,
        playback_source: impl FnMut() -> hit::Playback + 'static,
        sample_source: impl FnMut(&mut [f32]) + 'static,
        playlist_source: impl FnMut() -> (Vec<pledit::Row>, Option<usize>) + 'static,
    ) -> Self {
        Self {
            on_command: Box::new(on_command),
            on_equalizer: Box::new(on_equalizer),
            on_menu: Box::new(on_menu),
            equalizer_presets,
            playback_source: Box::new(playback_source),
            sample_source: Box::new(sample_source),
            playlist_source: Box::new(playlist_source),
            external_source: Box::new(ExternalPoll::default),
        }
    }

    /// Poll application-owned worker results on the Wayland thread. The source must never block.
    pub fn with_external_source(mut self, source: impl FnMut() -> ExternalPoll + 'static) -> Self {
        self.external_source = Box::new(source);
        self
    }
}

/// Persistable pane layout restored before the first mapped frame. Positions are relative to the
/// main surface; playlist size is its remembered expanded size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneLayout {
    pub main_shaded: bool,
    pub equalizer_open: bool,
    pub equalizer_position: (i32, i32),
    pub playlist_open: bool,
    pub playlist_shaded: bool,
    pub playlist_position: (i32, i32),
    pub playlist_size: (u32, u32),
}

/// User-visible UI state returned after the main window closes. Transient interaction fields such
/// as hovered buttons and active pointer drags are deliberately excluded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionState {
    pub panes: PaneLayout,
    pub equalizer_enabled: bool,
    pub equalizer_shaded: bool,
    pub equalizer_preamp_db: f32,
    pub equalizer_bands_db: [f32; 10],
}

impl Default for PaneLayout {
    fn default() -> Self {
        Self {
            main_shaded: false,
            equalizer_open: false,
            equalizer_position: (0, xubamp_skin::sprites::MAIN_H),
            playlist_open: false,
            playlist_shaded: false,
            playlist_position: (xubamp_skin::sprites::MAIN_W, 0),
            playlist_size: (
                xubamp_skin::sprites::PLEDIT_W as u32,
                xubamp_skin::sprites::PLEDIT_H as u32,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PaneDrag {
    /// Pointer position in parent-surface coordinates when the press began.
    press: panes::Point,
    /// Pane position when the press began.
    origin: panes::Point,
    moved: bool,
}

#[derive(Clone, Copy, Debug)]
struct PaneResize {
    /// Pointer position in parent-surface coordinates when the press began.
    press: panes::Point,
    width: i32,
    height: i32,
}

/// The playlist is a child surface of the main `xdg_toplevel`, not another toplevel. Wayland lets a
/// client position subsurfaces, so this preserves classic edge snapping and makes the panes travel
/// together when Mutter moves the main window. Content and selection live in `App::playlist_state`,
/// so close/reopen loses nothing.
struct PlaylistWin {
    subsurface: wl_subsurface::WlSubsurface,
    surface: wl_surface::WlSurface,
    pool: SlotPool,
    fb: Framebuffer,
    /// Position relative to the main surface. Subsurfaces may extend outside their parent and keep
    /// receiving pointer input there.
    position: panes::Point,
    /// Current pane size in px (at least the classic minimum).
    width: i32,
    height: i32,
    drag: Option<PaneDrag>,
    resize: Option<PaneResize>,
    /// The last bare title-bar click, for double-click windowshade toggling. Button clicks and moves
    /// do not participate.
    title_last_click: Option<Instant>,
    /// Whether the pointer is currently over the bottom-right resize grip, so the resize cursor is
    /// set only on the hover transition rather than on every motion event.
    grip_hover: bool,
}

/// Equalizer child-surface resources. The renderer owns all control state; this wrapper only owns
/// Wayland presentation and title-bar dragging.
struct EqualizerWin {
    subsurface: wl_subsurface::WlSubsurface,
    surface: wl_surface::WlSurface,
    pool: SlotPool,
    fb: Framebuffer,
    position: panes::Point,
    drag: Option<PaneDrag>,
    title_last_click: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopupOwner {
    Main,
    EqualizerPresets,
    PlaylistAdd,
}

struct PopupMenuWin {
    owner: PopupOwner,
    subsurface: wl_subsurface::WlSubsurface,
    surface: wl_surface::WlSurface,
    pool: SlotPool,
    fb: Framebuffer,
    model: menu::Menu<menu::ClassicMenuAction>,
    interaction: menu::MenuInteraction,
}

/// The "Jump to file" dialog window (classic `J`): a standalone toplevel that filters the track
/// list and plays the pick, without touching the playlist. Its content lives in `App::jump_state`.
struct JumpWin {
    window: Window,
    pool: SlotPool,
    fb: Framebuffer,
    configured: bool,
    width: i32,
    height: i32,
    /// A title-bar press deferred into a compositor move.
    armed_move: Option<(i32, i32, u32)>,
    /// The last result row clicked and when, to detect a double-click (which plays it).
    last_click: Option<(usize, Instant)>,
}

impl JumpWin {
    /// Upload `self.fb` to the window's shm buffer and commit (static, no frame callback).
    fn present(&mut self) {
        let (w, h) = (self.fb.width, self.fb.height);
        let stride = w as i32 * 4;
        let (buffer, canvas) = self
            .pool
            .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
            .expect("create wl_shm buffer");
        for (dst, src) in canvas.chunks_exact_mut(4).zip(self.fb.rgba.chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        buffer.attach_to(surface).expect("attach buffer");
        surface.commit();
    }
}

/// Resolve a requested playlist size into the actual buffer size and the remembered expanded size.
/// While shaded, only width updates; restoring recovers the pre-shade height.
fn playlist_configured_size(
    shaded: bool,
    expanded: (i32, i32),
    suggested: (Option<i32>, Option<i32>),
) -> ((i32, i32), (i32, i32)) {
    let mut expanded = (
        expanded.0.max(xubamp_skin::sprites::PLEDIT_W),
        expanded.1.max(xubamp_skin::sprites::PLEDIT_H),
    );
    if shaded {
        if let Some(w) = suggested.0 {
            expanded.0 = w.max(xubamp_skin::sprites::PLEDIT_W);
        }
        ((expanded.0, xubamp_skin::sprites::PLEDIT_SHADE_H), expanded)
    } else {
        if let Some(w) = suggested.0 {
            expanded.0 = w.max(xubamp_skin::sprites::PLEDIT_W);
        }
        if let Some(h) = suggested.1 {
            expanded.1 = h.max(xubamp_skin::sprites::PLEDIT_H);
        }
        (expanded, expanded)
    }
}

/// Open the main window for `skin` and run until the user closes it. `title` is the song title
/// shown in the marquee (empty for none). `on_command` is called on the event-loop thread for
/// every UI command: a transport button click, a volume/balance change as its slider is dragged,
/// or a seek when the position bar is released; the caller bridges it to the audio engine.
/// `playback_source` is polled each redraw tick for the clock snapshot (elapsed, seek-bar position,
/// duration, and whether audio is playing). `sample_source` fills a buffer with the most recent
/// output samples (mono, oldest first) for the visualizer, so this layer animates it without
/// knowing anything about audio.
pub fn run(
    skin: Skin,
    title: String,
    equalizer_state: equalizer::EqState,
    pane_layout: PaneLayout,
    runtime: Runtime,
) -> Result<SessionState, Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let subcompositor =
        SubcompositorState::bind(compositor.wl_compositor().clone(), &globals, &qh)?;
    let xdg_shell = XdgShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let state = hit::UiState {
        title,
        shade: pane_layout.main_shaded,
        ..Default::default()
    };
    let fb = compose_main_window(&skin, &state);
    let (w, h) = (fb.width, fb.height);

    let surface = compositor.create_surface(&qh);
    // RequestClient: no server-side decorations. We draw the whole window ourselves.
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestClient, &qh);
    window.set_title("xubamp");
    window.set_app_id("xubamp");
    // Classic main window is a fixed size.
    window.set_min_size(Some((w, h)));
    window.set_max_size(Some((w, h)));
    // Child panes extend outside the root surface. Keep the xdg window geometry pinned to the main
    // pane so their bounding box does not change Mutter's placement or resize calculations.
    window.set_window_geometry(0, 0, w, h);
    window.commit();

    let pool = SlotPool::new(w as usize * h as usize * 4, &shm)?;

    // Build the calloop event loop before the App so its LoopHandle can be handed both to the
    // redraw timer and (with the `keyboard` feature) to the keyboard, on which SCTK schedules key
    // repeat. The timer is what makes the clock tick; the blocking dispatch we replaced could only
    // wake on Wayland events, never on its own.
    let mut event_loop: EventLoop<App> =
        EventLoop::try_new().expect("failed to create the calloop event loop");
    let loop_handle = event_loop.handle();

    // WaylandSource feeds compositor events into the loop and flushes our requests back out; it
    // takes the connection (cheap Arc clone) and the queue by value.
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .expect("failed to insert the Wayland source");

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        compositor,
        subcompositor,
        #[cfg(feature = "keyboard")]
        xdg_shell,
        shm,
        pool,
        window,
        skin,
        state,
        fb,
        on_command: runtime.on_command,
        on_equalizer: runtime.on_equalizer,
        on_menu: runtime.on_menu,
        equalizer_presets: runtime.equalizer_presets,
        playback_source: runtime.playback_source,
        sample_source: runtime.sample_source,
        playlist: None,
        playlist_state: pledit::PlState {
            shade: pane_layout.playlist_shaded,
            ..Default::default()
        },
        pl_size: (
            i32::try_from(pane_layout.playlist_size.0).unwrap_or(i32::MAX),
            i32::try_from(pane_layout.playlist_size.1).unwrap_or(i32::MAX),
        ),
        pl_position: panes::Point {
            x: pane_layout.playlist_position.0,
            y: pane_layout.playlist_position.1,
        },
        jump_win: None,
        jump_state: jump::JumpState::default(),
        playlist_source: runtime.playlist_source,
        external_source: runtime.external_source,
        mod_ctrl: false,
        mod_shift: false,
        last_click: None,
        title_last_click: None,
        vis_samples: vec![0.0; FFT_N],
        last_marquee: Instant::now(),
        qh: qh.clone(),
        frame_pending: false,
        playing: false,
        stopped: false,
        equalizer: None,
        equalizer_state,
        equalizer_position: panes::Point {
            x: pane_layout.equalizer_position.0,
            y: pane_layout.equalizer_position.1,
        },
        pending_equalizer_open: pane_layout.equalizer_open,
        pending_playlist_open: pane_layout.playlist_open,
        popup_menu: None,
        pointer: None,
        seat: None,
        armed_move: None,
        #[cfg(feature = "keyboard")]
        keyboard: None,
        #[cfg(feature = "keyboard")]
        keyboard_focus: false,
        #[cfg(feature = "keyboard")]
        modifiers: Modifiers::default(),
        #[cfg(feature = "keyboard")]
        loop_handle: loop_handle.clone(),
        configured: false,
        exit: false,
    };

    // A self-re-arming redraw timer. Each tick steps the marquee and polls the clock, then
    // reschedules itself: fast while a title is scrolling, once a second otherwise, so an idle
    // window barely wakes.
    loop_handle
        .insert_source(
            Timer::from_duration(MARQUEE_TICK),
            |_deadline, _meta, app: &mut App| TimeoutAction::ToDuration(app.tick()),
        )
        .expect("failed to insert the redraw timer");

    // `None` blocks until a Wayland event or the timer fires; no busy loop.
    while !app.exit {
        event_loop.dispatch(None, &mut app)?;
    }
    Ok(app.session_state())
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    /// The decoded skin, kept so the window can be recomposed when UI state changes.
    skin: Skin,
    /// Current interaction state (which button is held, etc.), drives composition.
    state: hit::UiState,
    /// The composited frame for the current `state`, uploaded to the shm buffer by `draw`.
    fb: Framebuffer,
    /// Sink for UI commands (transport clicks, slider drags, seek), called on the event-loop
    /// thread.
    on_command: Box<dyn FnMut(hit::Command)>,
    /// Sink for DSP-specific equalizer changes.
    on_equalizer: EqualizerSink,
    /// Sink for popup actions whose side effects live above the window layer (file choosers,
    /// settings, skin selection).
    on_menu: MenuSink,
    /// Canonical DSP-provided preset values paired with the labels shown by the popup.
    equalizer_presets: Vec<equalizer::Preset>,
    /// Polled each redraw tick for the clock snapshot (elapsed, seek-bar position, duration,
    /// playing).
    playback_source: Box<dyn FnMut() -> hit::Playback>,
    /// Fills `vis_samples` with the most recent output samples for the visualizer, per tick.
    sample_source: SampleSource,
    /// Scratch buffer of recent samples the visualizer reads (reused each frame, no per-tick alloc).
    vis_samples: Vec<f32>,
    /// When the marquee last stepped, so it advances on its own ~100ms wall clock independent of
    /// how fast the visualizer drives the redraw timer.
    last_marquee: Instant,
    /// The compositor, kept so a cursor surface can be created when the pointer is set up.
    compositor: CompositorState,
    /// Positions the playlist and equalizer as child panes of the main toplevel.
    subcompositor: SubcompositorState,
    /// The xdg shell, kept alive so standalone dialogs can be created on demand.
    #[cfg(feature = "keyboard")]
    xdg_shell: XdgShell,
    /// The pointer, once the seat reports the capability. A themed pointer so we can set a proper
    /// arrow cursor on enter (without it the window inherits whatever cursor was last active).
    /// `None` on a seat with no mouse.
    pointer: Option<ThemedPointer<PointerData>>,
    /// The seat the pointer belongs to, kept so a title-bar press can start an interactive
    /// move: `xdg_toplevel.move` needs the seat plus the press serial.
    seat: Option<wl_seat::WlSeat>,
    /// A title-bar press that has not yet become a window drag: the press position and its serial.
    /// The compositor move is deferred until the pointer moves past a small threshold, so a click
    /// (or a near-miss on a title-bar button) does not jump the window. Cleared on release/leave or
    /// once the move starts.
    armed_move: Option<(i32, i32, u32)>,
    /// The keyboard, once the seat reports the capability. Created with repeat so held seek/volume
    /// keys auto-ramp; `None` on a seat with no keyboard.
    #[cfg(feature = "keyboard")]
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Whether our surface currently holds keyboard focus, so a shortcut fires only while focused.
    #[cfg(feature = "keyboard")]
    keyboard_focus: bool,
    /// The latest modifier state. Key press events do not carry it, so it is cached here from
    /// `update_modifiers` and read to decide whether a shortcut's modifiers are clear.
    #[cfg(feature = "keyboard")]
    modifiers: Modifiers,
    /// A handle to the event loop, so the keyboard can be created with SCTK's calloop-driven key
    /// repeat when the seat advertises the capability. The `'static` here pins the loop's lifetime.
    #[cfg(feature = "keyboard")]
    loop_handle: LoopHandle<'static, App>,
    /// A queue handle, kept so a redraw can request the next frame callback (the visualizer renders
    /// off the compositor's frame callbacks while playing, for display-rate smoothness).
    qh: QueueHandle<App>,
    /// Whether a frame callback has been requested and not yet delivered, so we request at most one
    /// in flight (a second request without a callback would stall the loop).
    frame_pending: bool,
    /// Whether audio is playing, from the last playback poll. Gates the frame-callback loop: the
    /// visualizer only animates from live audio while this holds.
    playing: bool,
    /// Whether playback is stopped (vs paused), from the last poll. Stop settles the visualizer to
    /// baseline; a pause freezes it on its last frame.
    stopped: bool,
    /// Equalizer pane and its state, which survive close/reopen.
    equalizer: Option<EqualizerWin>,
    equalizer_state: equalizer::EqState,
    equalizer_position: panes::Point,
    /// Restored only after the root's initial xdg configure, when child surfaces may be mapped.
    pending_equalizer_open: bool,
    pending_playlist_open: bool,
    popup_menu: Option<PopupMenuWin>,
    /// The secondary playlist pane (PLEDIT), or `None` when closed.
    playlist: Option<PlaylistWin>,
    /// The playlist window's content + selection/scroll state; survives close/reopen.
    playlist_state: pledit::PlState,
    /// The playlist pane's last expanded size, remembered across shade and close/reopen.
    pl_size: (i32, i32),
    /// Last child-surface position, remembered across close/reopen.
    pl_position: panes::Point,
    /// The "Jump to file" dialog window, or `None` when closed.
    jump_win: Option<JumpWin>,
    /// The jump dialog's search query, track list, and result selection.
    jump_state: jump::JumpState,
    /// Latest Ctrl/Shift state, mirrored from `update_modifiers`. Always present (unlike the
    /// keyboard-gated `modifiers`) so the pointer handler can read them for ctrl/shift-click
    /// selection; they simply stay false in a build without the keyboard feature.
    mod_ctrl: bool,
    mod_shift: bool,
    /// The last playlist row clicked and when, to detect a double-click (which plays the row).
    last_click: Option<(usize, Instant)>,
    /// When the main title bar was last clicked, to detect a double-click (which toggles windowshade,
    /// like the classic title-bar double-click). Cleared once a click becomes a window drag.
    title_last_click: Option<Instant>,
    /// Polled each tick for the current track rows + playing index, to keep the playlist in sync.
    playlist_source: PlaylistSource,
    /// Non-blocking application-owned worker poller. Results are applied only on this UI thread.
    external_source: ExternalSource,
    /// Set once the main window has had its first `configure`, so the timer never attaches a buffer
    /// before the surface is mapped.
    configured: bool,
    exit: bool,
}

impl App {
    fn session_state(&self) -> SessionState {
        let playlist_width = u32::try_from(self.pl_size.0)
            .unwrap_or(xubamp_skin::sprites::PLEDIT_W as u32)
            .max(xubamp_skin::sprites::PLEDIT_W as u32);
        let playlist_height = u32::try_from(self.pl_size.1)
            .unwrap_or(xubamp_skin::sprites::PLEDIT_H as u32)
            .max(xubamp_skin::sprites::PLEDIT_H as u32);
        SessionState {
            panes: PaneLayout {
                main_shaded: self.state.shade,
                equalizer_open: self.equalizer.is_some(),
                equalizer_position: (self.equalizer_position.x, self.equalizer_position.y),
                playlist_open: self.playlist.is_some(),
                playlist_shaded: self.playlist_state.shade,
                playlist_position: (self.pl_position.x, self.pl_position.y),
                playlist_size: (playlist_width, playlist_height),
            },
            equalizer_enabled: self.equalizer_state.enabled,
            equalizer_shaded: self.equalizer_state.shade,
            equalizer_preamp_db: self.equalizer_state.preamp_db,
            equalizer_bands_db: self.equalizer_state.bands_db,
        }
    }

    /// Recompose the frame from the current UI state and push it to the screen. Cheap (the
    /// window is 275x116), so we just rebuild the whole frame on any state change.
    fn redraw(&mut self) {
        self.fb = compose_main_window(&self.skin, &self.state);
        self.draw();
    }

    /// Carry out the redraw and command side effects of a pointer [`hit::Outcome`]. The
    /// `start_move` side effect is handled at the press site (it needs the event serial).
    fn apply(&mut self, outcome: hit::Outcome) {
        if outcome.redraw {
            self.redraw();
        }
        if let Some(command) = outcome.command {
            match command {
                hit::Command::Volume(volume) => {
                    self.equalizer_state.volume = volume;
                    self.redraw_equalizer();
                }
                hit::Command::Balance(balance) => {
                    self.equalizer_state.balance = balance;
                    self.redraw_equalizer();
                }
                _ => {}
            }
            (self.on_command)(command);
        }
        if let Some(action) = outcome.window {
            match action {
                hit::TitleButton::Close => self.exit = true,
                hit::TitleButton::Minimize => self.window.set_minimized(),
                hit::TitleButton::Shade => self.toggle_shade(),
                hit::TitleButton::Options => self.open_main_menu_at(panes::Point {
                    x: xubamp_skin::sprites::TITLE_BUTTONS_PRESSED[0].dst_x,
                    y: hit::TITLEBAR_H,
                }),
            }
        }
        if let Some(t) = outcome.toggle {
            match t {
                hit::WindowToggle::Playlist => self.toggle_playlist(),
                hit::WindowToggle::Equalizer => self.toggle_equalizer(),
            }
        }
    }

    /// Toggle windowshade (collapsed) mode: flip the flag, pin the toplevel to the new fixed size,
    /// and immediately attach a matching buffer. Size hints do not require the compositor to send a
    /// new configure, so waiting for one can leave the old-size surface visible indefinitely.
    fn toggle_shade(&mut self) {
        let old_h = if self.state.shade {
            xubamp_skin::sprites::MAIN_SHADE_H
        } else {
            xubamp_skin::sprites::MAIN_H
        };
        self.state.shade = !self.state.shade;
        let w = xubamp_skin::sprites::MAIN_W as u32;
        let h = if self.state.shade {
            xubamp_skin::sprites::MAIN_SHADE_H
        } else {
            xubamp_skin::sprites::MAIN_H
        };

        // Preserve the direct pane graph through the main height change, including a playlist
        // attached below an equalizer which itself is attached below the main pane. Positions are
        // updated even while a pane is closed so reopening stays docked.
        let visible_playlist_h = if self.playlist_state.shade {
            xubamp_skin::sprites::PLEDIT_SHADE_H
        } else {
            self.pl_size.1
        };
        let old_main = panes::Rect {
            x: 0,
            y: 0,
            width: xubamp_skin::sprites::MAIN_W,
            height: old_h,
        };
        let new_main = panes::Rect {
            height: h,
            ..old_main
        };
        let equalizer_h = if self.equalizer_state.shade {
            xubamp_skin::sprites::EQ_SHADE_H
        } else {
            xubamp_skin::sprites::EQ_H
        };
        let old_equalizer = panes::Rect::at(
            self.equalizer_position,
            xubamp_skin::sprites::EQ_W,
            equalizer_h,
        );
        self.equalizer_position =
            panes::preserve_resize_attachment(old_equalizer, old_main, new_main);
        let new_equalizer = panes::Rect::at(
            self.equalizer_position,
            xubamp_skin::sprites::EQ_W,
            equalizer_h,
        );
        let playlist_rect = panes::Rect::at(self.pl_position, self.pl_size.0, visible_playlist_h);
        let after_main = panes::preserve_resize_attachment(playlist_rect, old_main, new_main);
        let playlist_rect = panes::Rect::at(after_main, self.pl_size.0, visible_playlist_h);
        self.pl_position =
            panes::preserve_resize_attachment(playlist_rect, old_equalizer, new_equalizer);
        if let Some(equalizer) = &mut self.equalizer {
            equalizer.position = self.equalizer_position;
            equalizer
                .subsurface
                .set_position(self.equalizer_position.x, self.equalizer_position.y);
        }
        if let Some(playlist) = &mut self.playlist {
            playlist.position = self.pl_position;
            playlist
                .subsurface
                .set_position(self.pl_position.x, self.pl_position.y);
        }

        let h = h as u32;
        self.window.set_min_size(Some((w, h)));
        self.window.set_max_size(Some((w, h)));
        self.window.set_window_geometry(0, 0, w, h);
        self.window.commit();
        if self.configured {
            self.redraw();
        }
    }

    /// Whether the visualizer should be animating from live audio right now: configured, expanded (no
    /// visualizer shows in the collapsed strip), a palette present, a mode other than Off, and audio
    /// playing. While this holds the visualizer renders off frame callbacks; otherwise the timer
    /// settles it to baseline.
    fn animating(&self) -> bool {
        self.configured
            && !self.state.shade
            && self.playing
            && self.skin.viscolor.is_some()
            && self.state.vis.mode != VisMode::Off
    }

    /// Poll the playback clock (updating [`Self::playing`]) and step the marquee on its own ~100 ms
    /// wall clock. Returns whether anything the display shows (time, marquee) changed. Does NOT step
    /// the visualizer; callers do that when appropriate.
    fn step_clock_and_marquee(&mut self) -> bool {
        let pb = (self.playback_source)();
        self.playing = pb.playing;
        self.stopped = pb.stopped;
        let mut changed = hit::on_tick(&mut self.state, pb);
        // The marquee steps on its OWN 100 ms clock, not once per redraw: the frame-callback loop
        // redraws at the display rate, and stepping the title every frame would scroll it far too
        // fast. Only skins with text.bmp render a marquee, and the collapsed strip shows none.
        if !self.state.shade && self.skin.text.is_some() && marquee::is_scrolling(&self.state.title)
        {
            let elapsed = self.last_marquee.elapsed();
            if elapsed >= MARQUEE_TICK {
                changed |= marquee::advance(&mut self.state);
                self.last_marquee = if elapsed < MARQUEE_TICK * 2 {
                    self.last_marquee + MARQUEE_TICK
                } else {
                    Instant::now()
                };
            }
        }
        changed
    }

    /// Step the visualizer, returning whether its drawing changed. No-op (returns `false`) when the
    /// skin ships no palette or the mode is Off. While PLAYING it animates from live samples; while
    /// STOPPED it advances with silence so it decays to the baseline (a reset); while merely PAUSED
    /// it does nothing, freezing on its last frame.
    fn step_vis(&mut self) -> bool {
        // No visualizer shows in the collapsed strip, so there is nothing to step while shaded.
        if self.state.shade || self.skin.viscolor.is_none() || self.state.vis.mode == VisMode::Off {
            return false;
        }
        if self.playing {
            (self.sample_source)(&mut self.vis_samples);
            self.state.vis.advance(&self.vis_samples)
        } else if self.stopped {
            self.vis_samples.iter_mut().for_each(|s| *s = 0.0);
            self.state.vis.advance(&self.vis_samples)
        } else {
            false // paused: frozen
        }
    }

    /// A compositor frame callback: the display is ready for the next frame. Step the clock, marquee
    /// and visualizer and redraw. The redraw re-arms the next frame callback while still animating,
    /// so this self-sustains at the display's refresh rate; when playback stops it does not re-arm
    /// and the timer takes over the (settling) visualizer.
    fn on_frame(&mut self) {
        self.frame_pending = false;
        if !self.configured {
            return;
        }
        self.step_clock_and_marquee();
        self.step_vis();
        self.redraw();
    }

    /// Redraw-timer tick: keeps the clock and marquee moving, and either drives the settling
    /// visualizer directly (when not animating, since there are no frame callbacks then) or just
    /// re-arms the frame-callback loop (when animating). Returns the next timer delay.
    fn tick(&mut self) -> Duration {
        let external = (self.external_source)();
        let external_pending = external.pending;
        for event in external.events {
            self.apply_external_event(event);
        }
        if !self.configured {
            // Nothing to draw into yet; retry soon so scrolling begins right after the first
            // configure instead of waiting out a full second.
            return external_tick_delay(MARQUEE_TICK, external_pending);
        }
        let changed = self.step_clock_and_marquee();
        // Keep the playlist window (if open) in sync with the track list and playing track.
        if self.playlist.is_some() {
            let (rows, current) = (self.playlist_source)();
            if rows != self.playlist_state.rows || current != self.playlist_state.current {
                self.playlist_state.rows = rows;
                self.playlist_state.current = current;
                self.redraw_playlist();
            }
        }
        let delay = if self.animating() {
            // The frame-callback loop renders the visualizer. Kick it off (or restart it if it
            // stalled) with a redraw, which re-arms the callback; otherwise just poll again soon.
            if changed || !self.frame_pending {
                self.redraw();
            }
            FRAME_FALLBACK
        } else {
            // Not playing, so no frame callbacks. When STOPPED the visualizer settles to baseline
            // (step_vis advances with silence and reports the change); when merely PAUSED it stays
            // frozen. The clock and marquee keep moving regardless.
            let vis_changed = self.step_vis();
            if changed || vis_changed {
                self.redraw();
            }
            if vis_changed {
                VIS_SETTLE_TICK
            } else if !self.state.shade
                && self.skin.text.is_some()
                && marquee::is_scrolling(&self.state.title)
            {
                MARQUEE_TICK
            } else {
                Duration::from_secs(1)
            }
        };
        external_tick_delay(delay, external_pending)
    }

    fn apply_external_event(&mut self, event: ExternalEvent) {
        match event {
            ExternalEvent::EqualizerPreset(preset) => {
                let preset = preset.sanitized();
                self.equalizer_state.preamp_db = preset.preamp_db;
                self.equalizer_state.bands_db = preset.bands_db;
                (self.on_equalizer)(equalizer::Command::Preset {
                    preamp_db: preset.preamp_db,
                    bands_db: preset.bands_db,
                });
                self.redraw_equalizer();
            }
        }
    }

    fn draw(&mut self) {
        let (w, h) = (self.fb.width, self.fb.height);
        let stride = w as i32 * 4;
        let (buffer, canvas) = self
            .pool
            .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
            .expect("create wl_shm buffer");

        // Framebuffer is RGBA; wl_shm Argb8888 is BGRA in little-endian memory.
        for (dst, src) in canvas.chunks_exact_mut(4).zip(self.fb.rgba.chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }

        // Own the surface handle so setting `frame_pending` below does not clash with a borrow of
        // `self.window`.
        let surface = self.window.wl_surface().clone();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        // While the visualizer is animating, ask to be woken for the next frame so it renders at the
        // display's refresh rate. Guarded so exactly one callback is in flight; the callback and the
        // commit below are what make it fire.
        if self.animating() && !self.frame_pending {
            surface.frame(&self.qh, surface.clone());
            self.frame_pending = true;
        }
        buffer.attach_to(&surface).expect("attach buffer");
        surface.commit();
    }

    /// Whether `surface` is one of our panes or dialogs. Used only by
    /// the keyboard-focus handlers, so it is gated with them.
    #[cfg(feature = "keyboard")]
    fn is_our_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        surface == self.window.wl_surface()
            || self
                .popup_menu
                .as_ref()
                .is_some_and(|popup| surface == &popup.surface)
            || self
                .equalizer
                .as_ref()
                .is_some_and(|eq| surface == &eq.surface)
            || self
                .playlist
                .as_ref()
                .is_some_and(|pl| surface == &pl.surface)
            || self
                .jump_win
                .as_ref()
                .is_some_and(|j| surface == j.window.wl_surface())
    }

    fn open_equalizer(&mut self) {
        if self.equalizer.is_some() {
            return;
        }
        self.equalizer_state.sanitize();
        let fb = equalizer::compose(&self.skin, &self.equalizer_state);
        let (subsurface, surface) = self
            .subcompositor
            .create_subsurface(self.window.wl_surface().clone(), &self.qh);
        subsurface.set_position(self.equalizer_position.x, self.equalizer_position.y);
        subsurface.set_desync();
        self.window.wl_surface().commit();
        let pool = SlotPool::new(fb.width as usize * fb.height as usize * 4, &self.shm)
            .expect("equalizer pool");
        self.equalizer = Some(EqualizerWin {
            subsurface,
            surface,
            pool,
            fb,
            position: self.equalizer_position,
            drag: None,
            title_last_click: None,
        });
        self.state.eq_open = true;
        self.redraw();
        self.redraw_equalizer();
    }

    fn close_equalizer(&mut self) {
        if self
            .popup_menu
            .as_ref()
            .is_some_and(|popup| popup.owner == PopupOwner::EqualizerPresets)
        {
            self.close_popup_menu();
        }
        if let Some(equalizer) = self.equalizer.take() {
            equalizer.subsurface.destroy();
            equalizer.surface.destroy();
        }
        self.equalizer_state.pressed_button = None;
        self.equalizer_state.dragging = None;
        self.state.eq_open = false;
        self.redraw();
    }

    fn toggle_equalizer(&mut self) {
        if self.equalizer.is_some() {
            self.close_equalizer();
        } else {
            self.open_equalizer();
        }
    }

    fn redraw_equalizer(&mut self) {
        if self.equalizer.is_none() {
            return;
        }
        let fb = equalizer::compose(&self.skin, &self.equalizer_state);
        let equalizer = self.equalizer.as_mut().unwrap();
        equalizer.fb = fb;
        equalizer.present();
    }

    fn set_equalizer_shade(&mut self, shaded: bool) {
        // Renderer actions are emitted after `EqState` flips, so derive the previous height from the
        // requested destination rather than reading the already-updated flag.
        let old_h = if shaded {
            xubamp_skin::sprites::EQ_H
        } else {
            xubamp_skin::sprites::EQ_SHADE_H
        };
        self.equalizer_state.shade = shaded;
        let new_h = if shaded {
            xubamp_skin::sprites::EQ_SHADE_H
        } else {
            xubamp_skin::sprites::EQ_H
        };

        // Keep a playlist directly attached below the equalizer attached when the EQ changes
        // height. Side-by-side panes remain fixed because the equalizer width never changes.
        let old_eq = panes::Rect::at(self.equalizer_position, xubamp_skin::sprites::EQ_W, old_h);
        let new_eq = panes::Rect {
            height: new_h,
            ..old_eq
        };
        let playlist_h = if self.playlist_state.shade {
            xubamp_skin::sprites::PLEDIT_SHADE_H
        } else {
            self.pl_size.1
        };
        let playlist = panes::Rect::at(self.pl_position, self.pl_size.0, playlist_h);
        self.pl_position = panes::preserve_resize_attachment(playlist, old_eq, new_eq);
        if let Some(playlist) = &mut self.playlist {
            playlist.position = self.pl_position;
            playlist
                .subsurface
                .set_position(self.pl_position.x, self.pl_position.y);
        }
        if let Some(equalizer) = &mut self.equalizer {
            equalizer.drag = None;
            equalizer.title_last_click = None;
        }
        self.window.wl_surface().commit();
        self.redraw_equalizer();
    }

    fn apply_equalizer(&mut self, outcome: equalizer::Outcome) {
        if let Some(command) = outcome.command {
            match command {
                equalizer::Command::Volume(volume) => {
                    self.state.volume = volume;
                    self.redraw();
                }
                equalizer::Command::Balance(balance) => {
                    self.state.balance = balance;
                    self.redraw();
                }
                _ => {}
            }
            (self.on_equalizer)(command);
        }
        if let Some(action) = outcome.action {
            match action {
                equalizer::Action::SetShade(shaded) => self.set_equalizer_shade(shaded),
                equalizer::Action::Close => self.close_equalizer(),
                equalizer::Action::Presets => self.open_equalizer_presets_menu(),
            }
        }
        if outcome.redraw && self.equalizer.is_some() {
            self.redraw_equalizer();
        }
    }

    fn open_equalizer_presets_menu(&mut self) {
        let names = self
            .equalizer_presets
            .iter()
            .map(|preset| preset.name.clone());
        let Ok(model) = menu::equalizer_presets_menu(names) else {
            eprintln!(
                "xubamp: equalizer preset menu requires {} built-in presets",
                menu::CLASSIC_EQ_PRESET_COUNT
            );
            return;
        };
        let mut interaction = menu::MenuInteraction::default();
        interaction.open(&model);
        let fb = menu::compose(&model, &interaction);
        let button = xubamp_skin::sprites::EQ_PRESETS;
        let position = panes::Point {
            x: self.equalizer_position.x + button.dst_x + button.src.w - fb.width as i32,
            y: self.equalizer_position.y + button.dst_y + button.src.h,
        };
        self.open_popup_menu(
            PopupOwner::EqualizerPresets,
            model,
            interaction,
            fb,
            position,
        );
    }

    fn open_main_menu_at(&mut self, position: panes::Point) {
        let model = menu::main_menu(menu::MainMenuState {
            main_window_open: true,
            equalizer_open: self.equalizer.is_some(),
            playlist_open: self.playlist.is_some(),
            repeat: self.state.repeat_on,
            shuffle: self.state.shuffle_on,
            ..menu::MainMenuState::default()
        });
        let mut interaction = menu::MenuInteraction::default();
        interaction.open(&model);
        let fb = menu::compose(&model, &interaction);
        self.open_popup_menu(PopupOwner::Main, model, interaction, fb, position);
    }

    fn open_playlist_add_menu(&mut self) {
        if self.playlist_state.shade {
            return;
        }
        let model = menu::playlist_add_menu();
        let mut interaction = menu::MenuInteraction::default();
        interaction.open(&model);
        let fb = menu::compose(&model, &interaction);
        let Some(playlist) = self.playlist.as_ref() else {
            return;
        };
        let (_, add_y, _, _) = pledit::add_button_rect(playlist.height);
        let position = panes::Point {
            x: playlist.position.x + pledit::ADD_BUTTON_X,
            y: playlist.position.y + add_y - fb.height as i32,
        };
        self.open_popup_menu(PopupOwner::PlaylistAdd, model, interaction, fb, position);
        self.playlist_state.pressed_add = true;
        self.redraw_playlist();
    }

    fn open_popup_menu(
        &mut self,
        owner: PopupOwner,
        model: menu::Menu<menu::ClassicMenuAction>,
        interaction: menu::MenuInteraction,
        fb: Framebuffer,
        position: panes::Point,
    ) {
        self.close_popup_menu();
        let (subsurface, surface) = self
            .subcompositor
            .create_subsurface(self.window.wl_surface().clone(), &self.qh);
        subsurface.set_position(position.x, position.y);
        subsurface.set_desync();
        // Keep the popup above every persistent pane regardless of the order those siblings were
        // opened. Reordering is latched by the parent commit below.
        if let Some(equalizer) = &self.equalizer {
            equalizer.subsurface.place_below(&surface);
        }
        if let Some(playlist) = &self.playlist {
            playlist.subsurface.place_below(&surface);
        }
        let pool = SlotPool::new(fb.width as usize * fb.height as usize * 4, &self.shm)
            .expect("popup menu pool");
        self.popup_menu = Some(PopupMenuWin {
            owner,
            subsurface,
            surface,
            pool,
            fb,
            model,
            interaction,
        });
        self.window.wl_surface().commit();
        self.redraw_popup_menu();
    }

    fn close_popup_menu(&mut self) {
        let Some(popup) = self.popup_menu.take() else {
            return;
        };
        popup.subsurface.destroy();
        popup.surface.destroy();
        if popup.owner == PopupOwner::PlaylistAdd && self.playlist_state.pressed_add {
            self.playlist_state.pressed_add = false;
            self.redraw_playlist();
        }
    }

    fn redraw_popup_menu(&mut self) {
        let Some(popup) = &mut self.popup_menu else {
            return;
        };
        popup.fb = menu::compose(&popup.model, &popup.interaction);
        popup.present();
    }

    fn apply_popup_outcome(&mut self, outcome: menu::MenuOutcome<menu::ClassicMenuAction>) {
        match outcome {
            menu::MenuOutcome::Unchanged => {}
            menu::MenuOutcome::Redraw => self.redraw_popup_menu(),
            menu::MenuOutcome::Dismissed => self.close_popup_menu(),
            menu::MenuOutcome::Activated(action) => {
                self.close_popup_menu();
                self.activate_popup_action(action);
            }
        }
    }

    fn activate_popup_action(&mut self, action: menu::ClassicMenuAction) {
        match action {
            menu::ClassicMenuAction::OpenMedia => (self.on_menu)(MenuRequest::OpenMedia),
            menu::ClassicMenuAction::Play => {
                (self.on_command)(hit::Command::Transport(hit::Transport::Play));
            }
            menu::ClassicMenuAction::Previous => {
                (self.on_command)(hit::Command::Transport(hit::Transport::Prev));
            }
            menu::ClassicMenuAction::Pause => {
                (self.on_command)(hit::Command::Transport(hit::Transport::Pause));
            }
            menu::ClassicMenuAction::Stop => {
                (self.on_command)(hit::Command::Transport(hit::Transport::Stop));
            }
            menu::ClassicMenuAction::Next => {
                (self.on_command)(hit::Command::Transport(hit::Transport::Next));
            }
            menu::ClassicMenuAction::ToggleEqualizer => self.toggle_equalizer(),
            menu::ClassicMenuAction::TogglePlaylistEditor => self.toggle_playlist(),
            menu::ClassicMenuAction::ToggleRepeat => {
                (self.on_command)(hit::Command::ToggleMode(hit::ModeButton::Repeat));
            }
            menu::ClassicMenuAction::ToggleShuffle => {
                (self.on_command)(hit::Command::ToggleMode(hit::ModeButton::Shuffle));
            }
            menu::ClassicMenuAction::BackFiveSeconds => {
                let outcome = hit::on_key(&mut self.state, hit::KeyPress::Left, false);
                self.apply(outcome);
            }
            menu::ClassicMenuAction::ForwardFiveSeconds => {
                let outcome = hit::on_key(&mut self.state, hit::KeyPress::Right, false);
                self.apply(outcome);
            }
            menu::ClassicMenuAction::Exit => self.exit = true,
            menu::ClassicMenuAction::EqualizerLoadPreset(index) => {
                let Some(preset) = self.equalizer_presets.get(index).cloned() else {
                    return;
                };
                self.apply_external_event(ExternalEvent::EqualizerPreset(preset));
            }
            menu::ClassicMenuAction::EqualizerSaveAs => {
                (self.on_menu)(MenuRequest::SaveEqualizer(equalizer::Preset {
                    name: "Custom".to_owned(),
                    preamp_db: self.equalizer_state.preamp_db,
                    bands_db: self.equalizer_state.bands_db,
                }));
            }
            action => (self.on_menu)(MenuRequest::Action(action)),
        }
    }

    fn popup_menu_pointer(&mut self, conn: &Connection, kind: &PointerEventKind, x: i32, y: i32) {
        if matches!(kind, PointerEventKind::Enter { .. }) {
            if let Some(pointer) = &self.pointer {
                let _ = pointer.set_cursor(conn, CursorIcon::Default);
            }
        }
        let Some(popup) = &mut self.popup_menu else {
            return;
        };
        let outcome = match kind {
            PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                popup.interaction.pointer_move(&popup.model, x, y)
            }
            PointerEventKind::Press { button, .. } if *button == BTN_LEFT => {
                popup.interaction.pointer_press(&popup.model, x, y)
            }
            PointerEventKind::Release { button, .. } if *button == BTN_LEFT => {
                popup.interaction.pointer_release(&popup.model, x, y)
            }
            _ => menu::MenuOutcome::Unchanged,
        };
        self.apply_popup_outcome(outcome);
    }

    /// Open the playlist child pane if it is not already open, and light the PL button on the main
    /// window. No-op if already open.
    fn open_playlist(&mut self) {
        if self.playlist.is_some() {
            return;
        }
        // Reopen at the remembered position and expanded width; a shaded playlist keeps that width
        // but maps only its 14px strip.
        let ((w, h), _) =
            playlist_configured_size(self.playlist_state.shade, self.pl_size, (None, None));
        let fb = pledit::compose(&self.skin, &self.playlist_state, w, h);
        let (subsurface, surface) = self
            .subcompositor
            .create_subsurface(self.window.wl_surface().clone(), &self.qh);
        subsurface.set_position(self.pl_position.x, self.pl_position.y);
        // The playlist is static and can publish independently of the visualizer-driven parent.
        subsurface.set_desync();
        // A subsurface position is parent state and takes effect on the parent's next commit.
        self.window.wl_surface().commit();
        let pool = SlotPool::new(w as usize * h as usize * 4, &self.shm).expect("playlist pool");
        self.playlist = Some(PlaylistWin {
            subsurface,
            surface,
            pool,
            fb,
            position: self.pl_position,
            width: w,
            height: h,
            drag: None,
            resize: None,
            title_last_click: None,
            grip_hover: false,
        });
        self.state.pl_open = true;
        self.redraw(); // relight the PL button on the main window
        self.redraw_playlist();
    }

    /// Close the playlist pane and dim the PL button.
    fn close_playlist(&mut self) {
        if self
            .popup_menu
            .as_ref()
            .is_some_and(|popup| popup.owner == PopupOwner::PlaylistAdd)
        {
            self.close_popup_menu();
        }
        if let Some(playlist) = self.playlist.take() {
            playlist.subsurface.destroy();
            playlist.surface.destroy();
        }
        self.playlist_state.pressed_title = None;
        self.state.pl_open = false;
        self.redraw();
    }

    fn toggle_playlist(&mut self) {
        if self.playlist.is_some() {
            self.close_playlist();
        } else {
            self.open_playlist();
        }
    }

    /// Toggle the playlist's own windowshade without losing its expanded resize. The child surface
    /// is resized immediately, independent of compositor configure events.
    fn toggle_playlist_shade(&mut self) {
        if self.playlist.is_none() {
            return;
        }
        let old_h = self
            .playlist
            .as_ref()
            .map_or(self.pl_size.1, |playlist| playlist.height);
        self.playlist_state.shade = !self.playlist_state.shade;
        self.playlist_state.pressed_title = None;
        let ((w, h), expanded) =
            playlist_configured_size(self.playlist_state.shade, self.pl_size, (None, None));
        self.pl_size = expanded;

        let old_playlist = panes::Rect::at(self.pl_position, w, old_h);
        let new_playlist = panes::Rect::at(self.pl_position, w, h);
        let equalizer_h = if self.equalizer_state.shade {
            xubamp_skin::sprites::EQ_SHADE_H
        } else {
            xubamp_skin::sprites::EQ_H
        };
        let equalizer = panes::Rect::at(
            self.equalizer_position,
            xubamp_skin::sprites::EQ_W,
            equalizer_h,
        );
        self.equalizer_position =
            panes::preserve_resize_attachment(equalizer, old_playlist, new_playlist);
        if let Some(equalizer) = &mut self.equalizer {
            equalizer.position = self.equalizer_position;
            equalizer
                .subsurface
                .set_position(self.equalizer_position.x, self.equalizer_position.y);
        }

        if let Some(pl) = &mut self.playlist {
            pl.width = w;
            pl.height = h;
            pl.position = self.pl_position;
            pl.drag = None;
            pl.resize = None;
            pl.title_last_click = None;
            pl.grip_hover = false;
            pl.subsurface
                .set_position(self.pl_position.x, self.pl_position.y);
            self.window.wl_surface().commit();
        }
        self.redraw_playlist();
    }

    /// Recompose and present the playlist pane from `playlist_state`, if it is open.
    fn redraw_playlist(&mut self) {
        let Some((w, h)) = self.playlist.as_ref().map(|pl| (pl.width, pl.height)) else {
            return;
        };
        let fb = pledit::compose(&self.skin, &self.playlist_state, w, h);
        let pl = self.playlist.as_mut().unwrap();
        pl.fb = fb;
        pl.present();
    }

    fn equalizer_pointer(&mut self, conn: &Connection, kind: &PointerEventKind, x: i32, y: i32) {
        match kind {
            PointerEventKind::Enter { .. } => {
                if let Some(pointer) = &self.pointer {
                    let _ = pointer.set_cursor(conn, CursorIcon::Default);
                }
            }
            PointerEventKind::Press { button, .. } if *button == BTN_LEFT => {
                let outcome = equalizer::on_press(&mut self.equalizer_state, x, y);
                if outcome.start_move {
                    let now = Instant::now();
                    let double = self
                        .equalizer
                        .as_ref()
                        .and_then(|eq| eq.title_last_click)
                        .is_some_and(|at| now.duration_since(at) < DOUBLE_CLICK);
                    if double {
                        if let Some(eq) = &mut self.equalizer {
                            eq.title_last_click = None;
                            eq.drag = None;
                        }
                        self.set_equalizer_shade(!self.equalizer_state.shade);
                    } else if let Some(eq) = &mut self.equalizer {
                        eq.title_last_click = Some(now);
                        eq.drag = Some(PaneDrag {
                            press: panes::Point {
                                x: eq.position.x + x,
                                y: eq.position.y + y,
                            },
                            origin: eq.position,
                            moved: false,
                        });
                    }
                }
                self.apply_equalizer(outcome);
            }
            PointerEventKind::Motion { .. } => {
                let drag_snapshot = self
                    .equalizer
                    .as_ref()
                    .and_then(|eq| eq.drag.map(|drag| (drag, eq.position)));
                if let Some((mut drag, current)) = drag_snapshot {
                    let pointer = panes::Point {
                        x: current.x + x,
                        y: current.y + y,
                    };
                    let dx = pointer.x - drag.press.x;
                    let dy = pointer.y - drag.press.y;
                    if drag.moved || hit::exceeds_move_threshold(dx, dy) {
                        drag.moved = true;
                        let proposed = panes::Rect {
                            x: drag.origin.x + dx,
                            y: drag.origin.y + dy,
                            width: xubamp_skin::sprites::EQ_W,
                            height: if self.equalizer_state.shade {
                                xubamp_skin::sprites::EQ_SHADE_H
                            } else {
                                xubamp_skin::sprites::EQ_H
                            },
                        };
                        let main = panes::Rect {
                            x: 0,
                            y: 0,
                            width: xubamp_skin::sprites::MAIN_W,
                            height: if self.state.shade {
                                xubamp_skin::sprites::MAIN_SHADE_H
                            } else {
                                xubamp_skin::sprites::MAIN_H
                            },
                        };
                        let mut stationary = vec![main];
                        if let Some(playlist) = &self.playlist {
                            stationary.push(panes::Rect::at(
                                playlist.position,
                                playlist.width,
                                playlist.height,
                            ));
                        }
                        let position = panes::snap_to_many(proposed, &stationary);
                        self.equalizer_position = position;
                        if let Some(eq) = &mut self.equalizer {
                            eq.position = position;
                            eq.drag = Some(drag);
                            eq.title_last_click = None;
                            eq.subsurface.set_position(position.x, position.y);
                        }
                        self.window.wl_surface().commit();
                    }
                    return;
                }
                let outcome = equalizer::on_motion(&mut self.equalizer_state, x, y);
                self.apply_equalizer(outcome);
            }
            PointerEventKind::Release { button, .. } if *button == BTN_LEFT => {
                if let Some(eq) = &mut self.equalizer {
                    eq.drag = None;
                }
                let outcome = equalizer::on_release(&mut self.equalizer_state, x, y);
                self.apply_equalizer(outcome);
            }
            PointerEventKind::Leave { .. } => {
                let active_drag = self
                    .equalizer
                    .as_ref()
                    .is_some_and(|equalizer| equalizer.drag.is_some());
                if !active_drag {
                    if let Some(eq) = &mut self.equalizer {
                        eq.title_last_click = None;
                    }
                }
                if equalizer::on_leave(&mut self.equalizer_state) {
                    self.redraw_equalizer();
                }
            }
            _ => {}
        }
    }

    /// Pointer handling for the playlist window: title buttons, title-bar drag/double-click shade,
    /// expanded and shaded resize grips, list interaction, and cursor feedback.
    fn playlist_pointer(&mut self, conn: &Connection, kind: &PointerEventKind, x: i32, y: i32) {
        match kind {
            PointerEventKind::Enter { .. } => {
                if let Some(pointer) = &self.pointer {
                    let _ = pointer.set_cursor(conn, CursorIcon::Default);
                }
            }
            PointerEventKind::Press { button, .. } if *button == BTN_LEFT => {
                let Some((width, height)) = self.playlist.as_ref().map(|pl| (pl.width, pl.height))
                else {
                    return;
                };
                match pledit::region_at(&self.playlist_state, width, height, x, y) {
                    pledit::Region::TitleButton(button) => {
                        self.playlist_state.pressed_title = Some(button);
                        if let Some(pl) = &mut self.playlist {
                            pl.drag = None;
                            pl.resize = None;
                            pl.title_last_click = None;
                        }
                        self.redraw_playlist();
                    }
                    pledit::Region::Resize => {
                        // Child panes are client-positioned. Record the pointer in parent-surface
                        // coordinates so resizing remains stable while motion is reported relative
                        // to this child surface.
                        if let Some(pl) = &mut self.playlist {
                            pl.resize = Some(PaneResize {
                                press: panes::Point {
                                    x: pl.position.x + x,
                                    y: pl.position.y + y,
                                },
                                width: pl.width,
                                height: pl.height,
                            });
                            pl.drag = None;
                            pl.title_last_click = None;
                        }
                    }
                    pledit::Region::TitleBar => {
                        let now = Instant::now();
                        let double = self
                            .playlist
                            .as_ref()
                            .and_then(|pl| pl.title_last_click)
                            .is_some_and(|at| now.duration_since(at) < DOUBLE_CLICK);
                        if double {
                            if let Some(pl) = &mut self.playlist {
                                pl.title_last_click = None;
                                pl.drag = None;
                                pl.resize = None;
                            }
                            self.toggle_playlist_shade();
                        } else if let Some(pl) = &mut self.playlist {
                            pl.title_last_click = Some(now);
                            pl.drag = Some(PaneDrag {
                                press: panes::Point {
                                    x: pl.position.x + x,
                                    y: pl.position.y + y,
                                },
                                origin: pl.position,
                                moved: false,
                            });
                            pl.resize = None;
                        }
                    }
                    pledit::Region::AddMenu => {
                        self.playlist_state.pressed_add = true;
                        if let Some(pl) = &mut self.playlist {
                            pl.drag = None;
                            pl.resize = None;
                            pl.title_last_click = None;
                        }
                        self.redraw_playlist();
                    }
                    pledit::Region::Body => self.playlist_press_row(x, y),
                    pledit::Region::None => {}
                }
            }
            PointerEventKind::Axis { vertical, .. } => {
                // The 14px strip has no list to scroll.
                if self.playlist_state.shade {
                    return;
                }
                // Mouse wheel (or trackpad) over the list scrolls it. A mouse reports discrete
                // notches; a trackpad a continuous pixel delta. Positive scrolls toward the end.
                let Some(ph) = self.playlist.as_ref().map(|pl| pl.height) else {
                    return;
                };
                let tracks = if vertical.discrete != 0 {
                    vertical.discrete as f32 * WHEEL_TRACKS_PER_NOTCH
                } else {
                    vertical.absolute as f32 / xubamp_skin::sprites::PLEDIT_ROW_H as f32
                };
                if tracks != 0.0 {
                    self.playlist_state.scroll_by_tracks(tracks, ph);
                    self.redraw_playlist();
                }
            }
            PointerEventKind::Motion { .. } => {
                let drag_snapshot = self
                    .playlist
                    .as_ref()
                    .and_then(|pl| pl.drag.map(|drag| (drag, pl.position, pl.width, pl.height)));
                if let Some((mut drag, current, width, height)) = drag_snapshot {
                    let pointer = panes::Point {
                        x: current.x + x,
                        y: current.y + y,
                    };
                    let dx = pointer.x - drag.press.x;
                    let dy = pointer.y - drag.press.y;
                    if drag.moved || hit::exceeds_move_threshold(dx, dy) {
                        drag.moved = true;
                        let proposed = panes::Rect {
                            x: drag.origin.x + dx,
                            y: drag.origin.y + dy,
                            width,
                            height,
                        };
                        let main = panes::Rect {
                            x: 0,
                            y: 0,
                            width: xubamp_skin::sprites::MAIN_W,
                            height: if self.state.shade {
                                xubamp_skin::sprites::MAIN_SHADE_H
                            } else {
                                xubamp_skin::sprites::MAIN_H
                            },
                        };
                        let mut stationary = vec![main];
                        if let Some(equalizer) = &self.equalizer {
                            stationary.push(panes::Rect::at(
                                equalizer.position,
                                xubamp_skin::sprites::EQ_W,
                                if self.equalizer_state.shade {
                                    xubamp_skin::sprites::EQ_SHADE_H
                                } else {
                                    xubamp_skin::sprites::EQ_H
                                },
                            ));
                        }
                        let position = panes::snap_to_many(proposed, &stationary);
                        self.pl_position = position;
                        if let Some(pl) = &mut self.playlist {
                            pl.position = position;
                            pl.drag = Some(drag);
                            pl.title_last_click = None;
                            pl.subsurface.set_position(position.x, position.y);
                        }
                        // Subsurface positions are latched by a parent commit.
                        self.window.wl_surface().commit();
                    }
                    return;
                }

                let resize_snapshot = self.playlist.as_ref().and_then(|pl| {
                    pl.resize
                        .map(|resize| (resize, pl.position, pl.width, pl.height))
                });
                if let Some((resize, position, _, _)) = resize_snapshot {
                    let pointer = panes::Point {
                        x: position.x + x,
                        y: position.y + y,
                    };
                    let requested_w = resize.width + pointer.x - resize.press.x;
                    let requested_h = resize.height + pointer.y - resize.press.y;
                    let ((width, height), expanded) = playlist_configured_size(
                        self.playlist_state.shade,
                        self.pl_size,
                        (Some(requested_w), Some(requested_h)),
                    );
                    self.pl_size = expanded;
                    if let Some(pl) = &mut self.playlist {
                        pl.width = width;
                        pl.height = height;
                    }
                    self.redraw_playlist();
                    return;
                }

                let over_grip = self.playlist.as_ref().is_some_and(|pl| {
                    pledit::region_at(&self.playlist_state, pl.width, pl.height, x, y)
                        == pledit::Region::Resize
                });
                if self
                    .playlist
                    .as_ref()
                    .is_some_and(|pl| pl.grip_hover != over_grip)
                {
                    if let Some(pointer) = &self.pointer {
                        let icon = if over_grip {
                            if self.playlist_state.shade {
                                CursorIcon::EwResize
                            } else {
                                CursorIcon::SeResize
                            }
                        } else {
                            CursorIcon::Default
                        };
                        let _ = pointer.set_cursor(conn, icon);
                    }
                    if let Some(pl) = &mut self.playlist {
                        pl.grip_hover = over_grip;
                    }
                }
            }
            PointerEventKind::Release { button, .. } if *button == BTN_LEFT => {
                if let Some(pl) = &mut self.playlist {
                    pl.drag = None;
                    pl.resize = None;
                }
                if self.playlist_state.pressed_add {
                    self.playlist_state.pressed_add = false;
                    let fired = self.playlist.as_ref().is_some_and(|pl| {
                        pledit::region_at(&self.playlist_state, pl.width, pl.height, x, y)
                            == pledit::Region::AddMenu
                    });
                    if fired {
                        self.open_playlist_add_menu();
                    } else {
                        self.redraw_playlist();
                    }
                    return;
                }
                let Some(pressed) = self.playlist_state.pressed_title.take() else {
                    return;
                };
                let fired = self.playlist.as_ref().is_some_and(|pl| {
                    pledit::region_at(&self.playlist_state, pl.width, pl.height, x, y)
                        == pledit::Region::TitleButton(pressed)
                });
                if !fired {
                    self.redraw_playlist();
                    return;
                }
                match pressed {
                    pledit::TitleButton::Shade => self.toggle_playlist_shade(),
                    pledit::TitleButton::Close => self.close_playlist(),
                }
            }
            PointerEventKind::Leave { .. } => {
                let menu_keeps_add_pressed = self
                    .popup_menu
                    .as_ref()
                    .is_some_and(|popup| popup.owner == PopupOwner::PlaylistAdd);
                let add_cleared =
                    !menu_keeps_add_pressed && std::mem::take(&mut self.playlist_state.pressed_add);
                let redraw = self.playlist_state.pressed_title.take().is_some() || add_cleared;
                if let Some(pl) = &mut self.playlist {
                    pl.grip_hover = false;
                    // During the implicit button grab, moving the subsurface may cause an enter or
                    // leave transition. Keep the active drag/resize alive until button release.
                    if pl.drag.is_none() && pl.resize.is_none() {
                        pl.title_last_click = None;
                    }
                }
                if redraw {
                    self.redraw_playlist();
                }
            }
            _ => {}
        }
    }

    /// Handle a left-press in the playlist list body (below the title bar, not on the grip): select
    /// the clicked row honoring Ctrl/Shift, clear the selection on an empty-area click, and play the
    /// row on a double-click.
    fn playlist_press_row(&mut self, x: i32, y: i32) {
        let Some((pw, ph)) = self.playlist.as_ref().map(|pl| (pl.width, pl.height)) else {
            return;
        };
        // The right edge is the scrollbar column (thumb-drag is a later addition); ignore presses
        // there so they neither select nor clear.
        if x >= pw - xubamp_skin::sprites::PLEDIT_RIGHT_TILE.w {
            return;
        }
        match self.playlist_state.row_at(x, y, ph) {
            Some(i) => {
                if self.mod_shift {
                    self.playlist_state.shift_select(i);
                } else if self.mod_ctrl {
                    self.playlist_state.ctrl_select(i);
                } else {
                    self.playlist_state.click_select(i);
                }
                // A second click on the same row within the double-click window plays it.
                let now = Instant::now();
                let double = self
                    .last_click
                    .is_some_and(|(row, at)| row == i && now.duration_since(at) < DOUBLE_CLICK);
                if double {
                    (self.on_command)(hit::Command::PlayIndex(i));
                    self.last_click = None;
                } else {
                    self.last_click = Some((i, now));
                }
                self.redraw_playlist();
            }
            None => {
                // A click in the empty area below the last track clears the selection.
                self.last_click = None;
                if !self.playlist_state.selected.is_empty() {
                    self.playlist_state.clear_selection();
                    self.redraw_playlist();
                }
            }
        }
    }

    /// Open the jump-to-file dialog (a standalone window) unless already open, filtered over the
    /// current track list with a fresh empty query. Only reachable via the `J` key.
    #[cfg(feature = "keyboard")]
    fn open_jump(&mut self) {
        if self.jump_win.is_some() {
            return;
        }
        let (rows, _current) = (self.playlist_source)();
        self.jump_state = jump::JumpState {
            rows,
            query: String::new(),
            selected: 0,
            scroll: 0,
        };
        let (w, h) = (jump::JUMP_W, jump::JUMP_H);
        let fb = jump::compose(&self.jump_state, w, h);
        let surface = self.compositor.create_surface(&self.qh);
        let window =
            self.xdg_shell
                .create_window(surface, WindowDecorations::RequestClient, &self.qh);
        window.set_title("xubamp jump to file");
        window.set_app_id("xubamp");
        // Fixed size for now (the classic dialog is resizable; that can follow).
        window.set_min_size(Some((w as u32, h as u32)));
        window.set_max_size(Some((w as u32, h as u32)));
        window.commit();
        let pool = SlotPool::new(w as usize * h as usize * 4, &self.shm).expect("jump pool");
        self.jump_win = Some(JumpWin {
            window,
            pool,
            fb,
            configured: false,
            width: w,
            height: h,
            armed_move: None,
            last_click: None,
        });
    }

    fn close_jump(&mut self) {
        self.jump_win = None;
    }

    #[cfg(feature = "keyboard")]
    fn toggle_jump(&mut self) {
        if self.jump_win.is_some() {
            self.close_jump();
        } else {
            self.open_jump();
        }
    }

    /// Play the currently-highlighted match (if any) and close the dialog.
    fn jump_confirm(&mut self) {
        if let Some(i) = self.jump_state.selected_track() {
            (self.on_command)(hit::Command::PlayIndex(i));
        }
        self.close_jump();
    }

    /// Recompose and present the jump dialog from `jump_state`, if open and mapped.
    fn redraw_jump(&mut self) {
        let Some((w, h)) = self
            .jump_win
            .as_ref()
            .filter(|j| j.configured)
            .map(|j| (j.width, j.height))
        else {
            return;
        };
        let fb = jump::compose(&self.jump_state, w, h);
        let j = self.jump_win.as_mut().unwrap();
        j.fb = fb;
        j.present();
    }

    /// Pointer handling for the jump dialog: title-bar drag, result-row select + double-click-play,
    /// and the Jump/Close buttons.
    fn jump_pointer(&mut self, conn: &Connection, kind: &PointerEventKind, x: i32, y: i32) {
        match kind {
            PointerEventKind::Enter { .. } => {
                if let Some(pointer) = &self.pointer {
                    let _ = pointer.set_cursor(conn, CursorIcon::Default);
                }
            }
            PointerEventKind::Press { button, serial, .. } if *button == BTN_LEFT => {
                // Title-bar band: arm a compositor move.
                if y < jump::JUMP_TITLE_H {
                    if let Some(j) = &mut self.jump_win {
                        j.armed_move = Some((x, y, *serial));
                    }
                    return;
                }
                let Some((w, h)) = self.jump_win.as_ref().map(|j| (j.width, j.height)) else {
                    return;
                };
                // Bottom buttons.
                if let Some(btn) = self.jump_state.button_at(x, y, w, h) {
                    match btn {
                        jump::JumpButton::Jump => self.jump_confirm(),
                        jump::JumpButton::Close => self.close_jump(),
                    }
                    return;
                }
                // A result row: select it; a double-click on the same row plays it.
                if let Some(pos) = self.jump_state.row_at(x, y, h) {
                    self.jump_state.selected = pos;
                    let now = Instant::now();
                    let double = self
                        .jump_win
                        .as_ref()
                        .and_then(|j| j.last_click)
                        .is_some_and(|(p, at)| p == pos && now.duration_since(at) < DOUBLE_CLICK);
                    if double {
                        self.jump_confirm();
                    } else {
                        if let Some(j) = &mut self.jump_win {
                            j.last_click = Some((pos, now));
                        }
                        self.redraw_jump();
                    }
                }
            }
            PointerEventKind::Motion { .. } => {
                let start = self
                    .jump_win
                    .as_ref()
                    .and_then(|j| j.armed_move)
                    .filter(|&(px, py, _)| hit::exceeds_move_threshold(x - px, y - py));
                if let Some((_, _, serial)) = start {
                    if let (Some(seat), Some(j)) = (self.seat.clone(), self.jump_win.as_mut()) {
                        j.window.move_(&seat, serial);
                        j.armed_move = None;
                    }
                }
            }
            PointerEventKind::Release { button, .. } if *button == BTN_LEFT => {
                if let Some(j) = &mut self.jump_win {
                    j.armed_move = None;
                }
            }
            PointerEventKind::Leave { .. } => {
                if let Some(j) = &mut self.jump_win {
                    j.armed_move = None;
                }
            }
            _ => {}
        }
    }
}

fn external_tick_delay(base: Duration, pending: bool) -> Duration {
    if pending {
        base.min(MARQUEE_TICK)
    } else {
        base
    }
}

impl PlaylistWin {
    /// Upload `self.fb` to the child surface's shm buffer and commit. No frame callback: playlist is
    /// static (redrawn only on interaction / track change), so it does not drive an animation loop.
    fn present(&mut self) {
        let (w, h) = (self.fb.width, self.fb.height);
        let stride = w as i32 * 4;
        let (buffer, canvas) = self
            .pool
            .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
            .expect("create wl_shm buffer");
        for (dst, src) in canvas.chunks_exact_mut(4).zip(self.fb.rgba.chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        self.surface.damage_buffer(0, 0, w as i32, h as i32);
        buffer.attach_to(&self.surface).expect("attach buffer");
        self.surface.commit();
    }
}

impl EqualizerWin {
    fn present(&mut self) {
        let (w, h) = (self.fb.width, self.fb.height);
        let stride = w as i32 * 4;
        let (buffer, canvas) = self
            .pool
            .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
            .expect("create equalizer wl_shm buffer");
        for (dst, src) in canvas.chunks_exact_mut(4).zip(self.fb.rgba.chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        self.surface.damage_buffer(0, 0, w as i32, h as i32);
        buffer
            .attach_to(&self.surface)
            .expect("attach equalizer buffer");
        self.surface.commit();
    }
}

impl PopupMenuWin {
    fn present(&mut self) {
        let (w, h) = (self.fb.width, self.fb.height);
        let stride = w as i32 * 4;
        let (buffer, canvas) = self
            .pool
            .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
            .expect("create popup menu wl_shm buffer");
        for (dst, src) in canvas.chunks_exact_mut(4).zip(self.fb.rgba.chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        self.surface.damage_buffer(0, 0, w as i32, h as i32);
        buffer
            .attach_to(&self.surface)
            .expect("attach popup menu buffer");
        self.surface.commit();
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        // Only the main window drives the frame-callback (visualizer) loop; the playlist is static.
        if surface == self.window.wl_surface() {
            self.on_frame();
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        // Closing the main window quits; closing the standalone jump dialog drops only it. Child
        // panes close through their own rendered buttons and never receive xdg close requests.
        if *window == self.window {
            self.exit = true;
        } else if self.jump_win.as_ref().is_some_and(|j| *window == j.window) {
            self.close_jump();
        }
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        window: &Window,
        _configure: WindowConfigure,
        _: u32,
    ) {
        if *window == self.window {
            let first_configure = !self.configured;
            self.configured = true;
            // Initial configure maps the surface. Later configures reassert the framebuffer that
            // matches current shade state; toggles already attach it immediately.
            self.redraw();
            if first_configure {
                let open_equalizer = std::mem::take(&mut self.pending_equalizer_open);
                let open_playlist = std::mem::take(&mut self.pending_playlist_open);
                if open_equalizer {
                    self.open_equalizer();
                }
                if open_playlist {
                    self.open_playlist();
                }
            }
        } else if self.jump_win.as_ref().is_some_and(|j| *window == j.window) {
            if let Some(j) = &mut self.jump_win {
                j.configured = true;
            }
            self.redraw_jump();
        }
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            // A themed pointer, so we can set a normal arrow cursor on enter (without it the window
            // inherits whatever cursor was last active, often an I-beam). It needs its own cursor
            // surface, and uses the cursor-shape protocol when the compositor supports it (Mutter
            // does), else the system XCURSOR theme.
            let cursor_surface = self.compositor.create_surface(qh);
            let pointer = self
                .seat_state
                .get_pointer_with_theme(
                    qh,
                    &seat,
                    self.shm.wl_shm(),
                    cursor_surface,
                    ThemeSpec::System,
                )
                .expect("failed to create pointer");
            self.pointer = Some(pointer);
            // Clone so the keyboard branch below can still take `seat`; only one capability arrives
            // per call, but both branches reference `seat`, so the pointer branch must not move it.
            self.seat = Some(seat.clone());
        }
        #[cfg(feature = "keyboard")]
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            // Create the keyboard WITH repeat: SCTK arms a calloop timer on our loop and re-invokes
            // this callback for each auto-repeat, which we route to the same handler as a real press
            // (flagged `is_repeat` so only seek/volume ramp, never the transport keys).
            let loop_handle = self.loop_handle.clone();
            let keyboard = self
                .seat_state
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    loop_handle,
                    Box::new(
                        |app: &mut App, _kbd: &wl_keyboard::WlKeyboard, event: KeyEvent| {
                            app.on_key(&event, true);
                        },
                    ),
                )
                .expect("failed to create keyboard");
            self.keyboard = Some(keyboard);
            self.seat = Some(seat);
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.pointer().release();
            }
            self.seat = None;
        }
        #[cfg(feature = "keyboard")]
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
            // SCTK cancels any in-flight repeat timer on release; drop focus so nothing lingers.
            self.keyboard_focus = false;
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let (x, y) = (event.position.0 as i32, event.position.1 as i32);
            let on_main = event.surface == *self.window.wl_surface();
            let on_popup = !on_main
                && self
                    .popup_menu
                    .as_ref()
                    .is_some_and(|popup| event.surface == popup.surface);
            if on_popup {
                self.popup_menu_pointer(conn, &event.kind, x, y);
                continue;
            }
            if matches!(event.kind, PointerEventKind::Press { button, .. } if button == BTN_RIGHT) {
                let position = if on_main {
                    Some(panes::Point { x, y })
                } else if let Some(equalizer) = self
                    .equalizer
                    .as_ref()
                    .filter(|equalizer| event.surface == equalizer.surface)
                {
                    Some(panes::Point {
                        x: equalizer.position.x + x,
                        y: equalizer.position.y + y,
                    })
                } else {
                    self.playlist
                        .as_ref()
                        .filter(|playlist| event.surface == playlist.surface)
                        .map(|playlist| panes::Point {
                            x: playlist.position.x + x,
                            y: playlist.position.y + y,
                        })
                };
                if let Some(position) = position {
                    self.close_popup_menu();
                    self.open_main_menu_at(position);
                    continue;
                }
            }
            if self.popup_menu.is_some()
                && matches!(
                    &event.kind,
                    PointerEventKind::Press { button, .. } if *button == BTN_LEFT
                )
            {
                self.close_popup_menu();
            }
            let on_equalizer = !on_main
                && self
                    .equalizer
                    .as_ref()
                    .is_some_and(|equalizer| event.surface == equalizer.surface);
            if on_equalizer {
                self.equalizer_pointer(conn, &event.kind, x, y);
                continue;
            }
            let on_playlist = !on_main
                && self
                    .playlist
                    .as_ref()
                    .is_some_and(|pl| event.surface == pl.surface);
            if on_playlist {
                self.playlist_pointer(conn, &event.kind, x, y);
                continue;
            }
            let on_jump = !on_main
                && self
                    .jump_win
                    .as_ref()
                    .is_some_and(|j| event.surface == *j.window.wl_surface());
            if on_jump {
                self.jump_pointer(conn, &event.kind, x, y);
                continue;
            }
            if !on_main {
                continue; // an event for some other surface (e.g. the cursor surface)
            }
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    // Set a normal arrow cursor; without this the window shows whatever cursor was
                    // active on entry (often an I-beam), which makes the title bar feel un-draggable.
                    if let Some(pointer) = &self.pointer {
                        let _ = pointer.set_cursor(conn, CursorIcon::Default);
                    }
                }
                PointerEventKind::Press { button, serial, .. } if button == BTN_LEFT => {
                    let outcome = hit::on_press(&mut self.state, x, y);
                    // A title-bar press arms a window drag, but does NOT start it yet: the compositor
                    // move is deferred until the pointer moves past a threshold, so a click (or a
                    // near-miss on a small title-bar button) does not jump the window. A second quick
                    // title-bar click toggles windowshade instead (the classic double-click), same as
                    // the shade button.
                    if outcome.start_move {
                        let now = Instant::now();
                        let double = self
                            .title_last_click
                            .is_some_and(|at| now.duration_since(at) < DOUBLE_CLICK);
                        if double {
                            self.title_last_click = None;
                            self.toggle_shade();
                        } else {
                            self.title_last_click = Some(now);
                            self.armed_move = Some((x, y, serial));
                        }
                    }
                    self.apply(outcome);
                }
                PointerEventKind::Motion { .. } => {
                    // A moved-far-enough armed title-bar press becomes a compositor window drag:
                    // hand it off with the original press serial, then let the compositor move the
                    // window until release. Wayland has no client-set absolute position, so this is
                    // the classic title-bar drag.
                    if let Some((px, py, serial)) = self.armed_move {
                        if hit::exceeds_move_threshold(x - px, y - py) {
                            if let Some(seat) = &self.seat {
                                self.window.move_(seat, serial);
                            }
                            self.armed_move = None;
                            // A drag is not the first half of a double-click, so it must not arm a
                            // windowshade toggle on the next title-bar press.
                            self.title_last_click = None;
                        }
                    }
                    // Drives slider dragging; inert otherwise. Wayland keeps delivering motion
                    // during the implicit button grab, so a drag continues past the window edge.
                    let outcome = hit::on_motion(&mut self.state, x, y);
                    self.apply(outcome);
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    // A release without crossing the threshold was a click, not a drag.
                    self.armed_move = None;
                    let outcome = hit::on_release(&mut self.state, x, y);
                    self.apply(outcome);
                }
                PointerEventKind::Leave { .. } => {
                    self.armed_move = None;
                    // Cancel any in-progress button press so a button never stays stuck down.
                    let needs_redraw = hit::on_leave(&mut self.state);
                    if needs_redraw {
                        self.redraw();
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(feature = "keyboard")]
impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        // Focus on EITHER of our windows enables shortcuts (Winamp's hotkeys are global), and lets
        // the jump-to-file query keep receiving keys after the playlist window is what's focused.
        if self.is_our_surface(surface) {
            self.keyboard_focus = true;
        }
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        if self.is_our_surface(surface) {
            self.keyboard_focus = false;
        }
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.on_key(&event, false);
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _layout: u32,
    ) {
        self.modifiers = modifiers;
        // Mirror into the always-present bools the pointer handler reads for ctrl/shift-click.
        self.mod_ctrl = modifiers.ctrl;
        self.mod_shift = modifiers.shift;
    }
}

#[cfg(feature = "keyboard")]
impl App {
    /// Handle a key: gate on focus and modifiers, decode it to a [`hit::KeyPress`], and run it
    /// through the same [`hit::Outcome`] path as a pointer event. `is_repeat` is true for SCTK's
    /// synthesized auto-repeats, so [`hit::on_key`] ramps seek/volume while firing transport keys
    /// once.
    fn on_key(&mut self, event: &KeyEvent, is_repeat: bool) {
        // Events only reach a focused surface, but gate on our own flag so a trailing repeat right
        // after a focus loss can never fire.
        if !self.keyboard_focus {
            return;
        }
        if self.popup_menu.is_some() {
            self.popup_menu_key(event);
            return;
        }
        // The jump-to-file dialog captures every key (text, Backspace, arrows, Enter, Escape) while
        // it is open.
        if self.jump_win.is_some() {
            self.jump_key(event);
            return;
        }
        // Plain shortcuts only (Shift/Caps merely change letter case). A Ctrl/Alt/Super chord is
        // left for the compositor or a later binding, so e.g. Ctrl+X never triggers Play.
        let m = self.modifiers;
        if m.ctrl || m.alt || m.logo {
            return;
        }
        let Some(key) = decode_key(event) else {
            return;
        };
        // J opens (toggles) the jump-to-file dialog rather than being a main-window shortcut.
        if key == hit::KeyPress::Char('j') {
            self.toggle_jump();
            return;
        }
        let outcome = hit::on_key(&mut self.state, key, is_repeat);
        self.apply(outcome);
    }

    fn popup_menu_key(&mut self, event: &KeyEvent) {
        let key = match event.keysym {
            Keysym::Escape => menu::MenuKey::Escape,
            Keysym::Return | Keysym::KP_Enter => menu::MenuKey::Enter,
            Keysym::Up => menu::MenuKey::Up,
            Keysym::Down => menu::MenuKey::Down,
            Keysym::Left => menu::MenuKey::Left,
            Keysym::Right => menu::MenuKey::Right,
            Keysym::Home => menu::MenuKey::Home,
            Keysym::End => menu::MenuKey::End,
            _ => {
                let Some(character) = event.utf8.as_deref().and_then(|text| text.chars().next())
                else {
                    return;
                };
                if character.is_control() {
                    return;
                }
                menu::MenuKey::Character(character)
            }
        };
        let Some(popup) = &mut self.popup_menu else {
            return;
        };
        let outcome = popup.interaction.key(&popup.model, key);
        self.apply_popup_outcome(outcome);
    }

    /// Handle a key while the jump dialog is open: printable characters and Backspace edit the
    /// query, Up/Down move the highlighted match, Enter plays it and closes, Escape closes.
    fn jump_key(&mut self, event: &KeyEvent) {
        let h = self.jump_win.as_ref().map_or(jump::JUMP_H, |j| j.height);
        match event.keysym {
            Keysym::Escape => self.close_jump(),
            Keysym::Return | Keysym::KP_Enter => self.jump_confirm(),
            Keysym::Up => {
                self.jump_state.move_selection(-1, h);
                self.redraw_jump();
            }
            Keysym::Down => {
                self.jump_state.move_selection(1, h);
                self.redraw_jump();
            }
            Keysym::BackSpace => {
                let mut q = self.jump_state.query.clone();
                q.pop();
                self.jump_state.set_query(q, h);
                self.redraw_jump();
            }
            _ => {
                if let Some(c) = event.utf8.as_deref().and_then(|s| s.chars().next()) {
                    if !c.is_control() {
                        let mut q = self.jump_state.query.clone();
                        q.push(c);
                        self.jump_state.set_query(q, h);
                        self.redraw_jump();
                    }
                }
            }
        }
    }
}

/// Decode an SCTK key event into a main-window [`hit::KeyPress`], or `None` for unbound keys. The
/// arrow keys are matched on their layout-independent keysym; any other key is taken from its
/// produced text (`utf8`) folded to lowercase, so a letter shortcut follows the key's printed label
/// on the user's layout rather than a fixed physical position.
#[cfg(feature = "keyboard")]
fn decode_key(event: &KeyEvent) -> Option<hit::KeyPress> {
    use hit::KeyPress;
    match event.keysym {
        Keysym::Up => return Some(KeyPress::Up),
        Keysym::Down => return Some(KeyPress::Down),
        Keysym::Left => return Some(KeyPress::Left),
        Keysym::Right => return Some(KeyPress::Right),
        _ => {}
    }
    let ch = event.utf8.as_deref()?.chars().next()?;
    Some(KeyPress::Char(ch.to_ascii_lowercase()))
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(App);
delegate_output!(App);
delegate_seat!(App);
delegate_pointer!(App);
#[cfg(feature = "keyboard")]
delegate_keyboard!(App);
delegate_shm!(App);
delegate_subcompositor!(App);
delegate_xdg_shell!(App);
delegate_xdg_window!(App);
delegate_registry!(App);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_external_work_caps_the_idle_poll_delay() {
        assert_eq!(
            external_tick_delay(Duration::from_secs(1), true),
            MARQUEE_TICK
        );
        assert_eq!(external_tick_delay(VIS_SETTLE_TICK, true), VIS_SETTLE_TICK);
        assert_eq!(
            external_tick_delay(Duration::from_secs(1), false),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn playlist_shade_preserves_expanded_height_and_accepts_only_width() {
        let expanded = (350, 203);
        let (shown, remembered) = playlist_configured_size(true, expanded, (Some(425), Some(999)));
        assert_eq!(
            shown,
            (425, xubamp_skin::sprites::PLEDIT_SHADE_H),
            "mapped buffer is completely collapsed"
        );
        assert_eq!(
            remembered,
            (425, 203),
            "shade resize updates width without destroying expanded height"
        );

        let (restored, remembered) = playlist_configured_size(false, remembered, (None, None));
        assert_eq!(restored, (425, 203));
        assert_eq!(remembered, restored);
    }

    #[test]
    fn playlist_configure_clamps_each_expanded_dimension_to_the_classic_minimum() {
        let (shown, remembered) = playlist_configured_size(false, (400, 200), (Some(10), Some(20)));
        assert_eq!(
            shown,
            (
                xubamp_skin::sprites::PLEDIT_W,
                xubamp_skin::sprites::PLEDIT_H
            )
        );
        assert_eq!(remembered, shown);

        let (shade, remembered) = playlist_configured_size(true, shown, (Some(10), None));
        assert_eq!(
            shade,
            (
                xubamp_skin::sprites::PLEDIT_W,
                xubamp_skin::sprites::PLEDIT_SHADE_H
            )
        );
        assert_eq!(remembered.1, xubamp_skin::sprites::PLEDIT_H);
    }
}
