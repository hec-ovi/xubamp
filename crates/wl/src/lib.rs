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

/// How often the marquee steps while a title is scrolling. Between scrolls the redraw timer
/// falls back to a once-a-second cadence for the clock, so an idle window stays cheap.
const MARQUEE_TICK: Duration = Duration::from_millis(100);

/// Redraw cadence while the visualizer is animating (~30 fps), fast enough for smooth bars but
/// far cheaper than the display refresh rate.
const VIS_TICK: Duration = Duration::from_millis(33);

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
        pointer: None,
        seat: None,
        configured: false,
        exit: false,
    };

    // Drive the Wayland queue and a periodic redraw timer from one calloop event loop. The
    // timer is what makes the clock tick; the blocking dispatch we replaced could only wake on
    // Wayland events, never on its own.
    let mut event_loop: EventLoop<App> =
        EventLoop::try_new().expect("failed to create the calloop event loop");
    let loop_handle = event_loop.handle();

    // WaylandSource feeds compositor events into the loop and flushes our requests back out; it
    // takes the connection (cheap Arc clone) and the queue by value.
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .expect("failed to insert the Wayland source");

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

    /// Redraw-timer tick: poll the playback clock, step the marquee and the visualizer, and
    /// recompose only if something moved. Returns how long to wait before the next tick: the fast
    /// [`VIS_TICK`] while the visualizer animates, [`MARQUEE_TICK`] while a title scrolls, else a
    /// second (just the clock). Does nothing before the first configure (nothing to draw into yet).
    fn tick(&mut self) -> Duration {
        if !self.configured {
            // Nothing to draw into yet; retry soon so scrolling begins right after the first
            // configure instead of waiting out a full second.
            return MARQUEE_TICK;
        }
        let pb = (self.playback_source)();
        let playing = pb.playing;
        let mut changed = hit::on_tick(&mut self.state, pb);

        // Step the marquee on its own ~100ms wall clock, NOT once per redraw tick: the visualizer
        // can drive the timer at 33ms, and stepping the title every tick would scroll it ~3x too
        // fast while playing. `is_scrolling` reports the scroll state independently of stepping.
        // Only skins with text.bmp render a marquee.
        let title_scrolls = self.skin.text.is_some() && marquee::is_scrolling(&self.state.title);
        if title_scrolls {
            let elapsed = self.last_marquee.elapsed();
            if elapsed >= MARQUEE_TICK {
                changed |= marquee::advance(&mut self.state);
                // Keep the phase accurate across fast ticks; resync (do not burst to catch up) if
                // the window sat idle a while between steps.
                self.last_marquee = if elapsed < MARQUEE_TICK * 2 {
                    self.last_marquee + MARQUEE_TICK
                } else {
                    Instant::now()
                };
            }
        }

        // Step the visualizer from the latest samples (or silence when not playing, so it settles),
        // but only when the skin ships a palette and the mode is not Off. `advance` reports whether
        // the drawing changed, so the settle-to-baseline frame is drawn and an idle vis stops.
        let vis_active = self.skin.viscolor.is_some() && self.state.vis.mode != VisMode::Off;
        let mut vis_changed = false;
        if vis_active {
            if playing {
                (self.sample_source)(&mut self.vis_samples);
            } else {
                self.vis_samples.iter_mut().for_each(|s| *s = 0.0);
            }
            vis_changed = self.state.vis.advance(&self.vis_samples);
            changed |= vis_changed;
        }
        if changed {
            self.redraw();
        }
        // Fast while the visualizer is live (playing) or still changing; otherwise the marquee or
        // clock cadence, so an idle window barely wakes.
        if (vis_active && playing) || vis_changed {
            VIS_TICK
        } else if title_scrolls {
            MARQUEE_TICK
        } else {
            Duration::from_secs(1)
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

        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        buffer.attach_to(surface).expect("attach buffer");
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
        self.draw();
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
                    // A title-bar press hands the drag to the compositor: it moves the window
                    // while the button is held, then ends the grab on release. Wayland has no
                    // client-set absolute position, so this is the classic title-bar drag.
                    if outcome.start_move {
                        if let Some(seat) = &self.seat {
                            self.window.move_(seat, serial);
                        }
                    }
                    self.apply(outcome);
                }
                PointerEventKind::Motion { .. } => {
                    // Drives slider dragging; inert otherwise. Wayland keeps delivering motion
                    // during the implicit button grab, so a drag continues past the window edge.
                    let outcome = hit::on_motion(&mut self.state, x, y);
                    self.apply(outcome);
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    let outcome = hit::on_release(&mut self.state, x, y);
                    self.apply(outcome);
                }
                PointerEventKind::Leave { .. } => {
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
delegate_shm!(App);
delegate_xdg_shell!(App);
delegate_xdg_window!(App);
delegate_registry!(App);
