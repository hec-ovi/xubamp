//! Native Wayland window: one undecorated `xdg_toplevel` backed by a `wl_shm` software
//! buffer that receives a rendered `Framebuffer`. Target is GNOME 50 / Mutter, no toolkit.
//!
//! The Wayland plumbing (registry, shm slot pool, xdg window) is handled by
//! smithay-client-toolkit; we still own every pixel by blitting our own `Framebuffer`
//! into the shm buffer. This layer needs a live compositor, so it is verified by running
//! on Ubuntu 26.04 rather than by unit tests.

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
    delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};
use xubamp_render::vis::{VisMode, FFT_N};
use xubamp_render::{compose_main_window, hit, marquee, Framebuffer};
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

/// Redraw cadence while the visualizer is settling to baseline (paused/stopped), where there is no
/// frame-callback loop. While actually playing the visualizer renders off the compositor's frame
/// callbacks at the display's refresh rate instead (see [`App::draw`]/[`App::on_frame`]), which is
/// far smoother than a fixed timer.
const VIS_TICK: Duration = Duration::from_millis(33);

/// While the frame-callback loop drives the visualizer, the timer only needs to poll the clock and
/// re-arm the loop if it ever stalls; a slow cadence keeps that cheap.
const FRAME_FALLBACK: Duration = Duration::from_millis(250);

/// Fills a caller-owned buffer with the most recent output samples (mono, oldest first) for the
/// visualizer to read each frame.
type SampleSource = Box<dyn FnMut(&mut [f32])>;

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
    on_command: impl FnMut(hit::Command) + 'static,
    playback_source: impl FnMut() -> hit::Playback + 'static,
    sample_source: impl FnMut(&mut [f32]) + 'static,
) -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let xdg_shell = XdgShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let state = hit::UiState {
        title,
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
        shm,
        pool,
        window,
        skin,
        state,
        fb,
        on_command: Box::new(on_command),
        playback_source: Box::new(playback_source),
        sample_source: Box::new(sample_source),
        vis_samples: vec![0.0; FFT_N],
        last_marquee: Instant::now(),
        qh: qh.clone(),
        frame_pending: false,
        playing: false,
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
    Ok(())
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
    /// The pointer, once the seat reports the capability. `None` on a seat with no mouse.
    pointer: Option<wl_pointer::WlPointer>,
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
    /// Set once the window has had its first `configure`, so the timer never attaches a buffer
    /// before the surface is mapped.
    configured: bool,
    exit: bool,
}

impl App {
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
            (self.on_command)(command);
        }
        if let Some(action) = outcome.window {
            match action {
                hit::TitleButton::Close => self.exit = true,
                hit::TitleButton::Minimize => self.window.set_minimized(),
                // Windowshade mode and the main menu are later phases; the button still shows its
                // pressed feedback, but the action is a no-op for now.
                hit::TitleButton::Shade => eprintln!("xubamp: windowshade mode not implemented yet"),
                hit::TitleButton::Options => eprintln!("xubamp: main menu not implemented yet"),
            }
        }
    }

    /// Whether the visualizer should be animating from live audio right now: configured, a palette
    /// present, a mode other than Off, and audio playing. While this holds the visualizer renders
    /// off frame callbacks; otherwise the timer settles it to baseline.
    fn animating(&self) -> bool {
        self.configured
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
        let mut changed = hit::on_tick(&mut self.state, pb);
        // The marquee steps on its OWN 100 ms clock, not once per redraw: the frame-callback loop
        // redraws at the display rate, and stepping the title every frame would scroll it far too
        // fast. Only skins with text.bmp render a marquee.
        if self.skin.text.is_some() && marquee::is_scrolling(&self.state.title) {
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

    /// Step the visualizer from the latest output samples (or silence when not playing, so it
    /// settles), returning whether its drawing changed. No-op when the skin ships no palette or the
    /// mode is Off.
    fn step_vis(&mut self) -> bool {
        if self.skin.viscolor.is_none() || self.state.vis.mode == VisMode::Off {
            return false;
        }
        if self.playing {
            (self.sample_source)(&mut self.vis_samples);
        } else {
            self.vis_samples.iter_mut().for_each(|s| *s = 0.0);
        }
        self.state.vis.advance(&self.vis_samples)
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
        if !self.configured {
            // Nothing to draw into yet; retry soon so scrolling begins right after the first
            // configure instead of waiting out a full second.
            return MARQUEE_TICK;
        }
        let changed = self.step_clock_and_marquee();
        if self.animating() {
            // The frame-callback loop renders the visualizer. Kick it off (or restart it if it
            // stalled) with a redraw, which re-arms the callback; otherwise just poll again soon.
            if changed || !self.frame_pending {
                self.redraw();
            }
            FRAME_FALLBACK
        } else {
            // Paused/stopped/vis-off: no frame callbacks, so the timer settles the visualizer.
            let vis_changed = self.step_vis();
            if changed || vis_changed {
                self.redraw();
            }
            if vis_changed {
                VIS_TICK
            } else if self.skin.text.is_some() && marquee::is_scrolling(&self.state.title) {
                MARQUEE_TICK
            } else {
                Duration::from_secs(1)
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
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.on_frame();
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
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        _: WindowConfigure,
        _: u32,
    ) {
        self.configured = true;
        self.draw();
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
            let pointer = self
                .seat_state
                .get_pointer(qh, &seat)
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
                    Box::new(|app: &mut App, _kbd: &wl_keyboard::WlKeyboard, event: KeyEvent| {
                        app.on_key(&event, true);
                    }),
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
                pointer.release();
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
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            // Ignore events for surfaces that are not our window.
            if event.surface != *self.window.wl_surface() {
                continue;
            }
            let (x, y) = (event.position.0 as i32, event.position.1 as i32);
            match event.kind {
                PointerEventKind::Press {
                    button, serial, ..
                } if button == BTN_LEFT => {
                    let outcome = hit::on_press(&mut self.state, x, y);
                    // A title-bar press arms a window drag, but does NOT start it yet: the compositor
                    // move is deferred until the pointer moves past a threshold, so a click (or a
                    // near-miss on a small title-bar button) does not jump the window.
                    if outcome.start_move {
                        self.armed_move = Some((x, y, serial));
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
        if surface == self.window.wl_surface() {
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
        if surface == self.window.wl_surface() {
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
        // Plain shortcuts only (Shift/Caps merely change letter case). A Ctrl/Alt/Super chord is
        // left for the compositor or a later binding, so e.g. Ctrl+X never triggers Play.
        let m = self.modifiers;
        if m.ctrl || m.alt || m.logo {
            return;
        }
        let Some(key) = decode_key(event) else {
            return;
        };
        let outcome = hit::on_key(&mut self.state, key, is_repeat);
        self.apply(outcome);
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
delegate_xdg_shell!(App);
delegate_xdg_window!(App);
delegate_registry!(App);
