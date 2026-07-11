//! Native Wayland window: one undecorated `xdg_toplevel` backed by a `wl_shm` software
//! buffer that receives a rendered `Framebuffer`. Target is GNOME 50 / Mutter, no toolkit.
//!
//! The Wayland plumbing (registry, shm slot pool, xdg window) is handled by
//! smithay-client-toolkit; we still own every pixel by blitting our own `Framebuffer`
//! into the shm buffer. This layer needs a live compositor, so it is verified by running
//! on Ubuntu 26.04 rather than by unit tests.

use std::error::Error;
use std::time::Duration;

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
use xubamp_render::{compose_main_window, hit, Framebuffer};
use xubamp_skin::Skin;

/// Open the main window for `skin` and run until the user closes it. `on_command` is called on
/// the event-loop thread whenever a transport button is clicked (pressed and released over
/// itself); the caller bridges it to the audio engine. `time_source` is polled once a second
/// for the elapsed play time (whole seconds, or `None` to blank the display), so the time
/// digits tick without this layer knowing anything about audio.
pub fn run(
    skin: Skin,
    on_command: impl FnMut(hit::Transport) + 'static,
    time_source: impl FnMut() -> Option<u32> + 'static,
) -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let xdg_shell = XdgShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let state = hit::UiState::default();
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
        time_source: Box::new(time_source),
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

    // A self-re-arming ~1s timer: poll the clock and recompose only if the shown time moved.
    loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_secs(1)),
            |_deadline, _meta, app: &mut App| {
                app.tick();
                TimeoutAction::ToDuration(Duration::from_secs(1))
            },
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
    /// Sink for transport commands, called when a button click completes.
    on_command: Box<dyn FnMut(hit::Transport)>,
    /// Polled once a second for the elapsed play time that drives the clock display.
    time_source: Box<dyn FnMut() -> Option<u32>>,
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

    /// Once-a-second timer tick: poll the playback clock and recompose only if the shown time
    /// changed. Does nothing before the first configure (nothing to draw into yet) or while the
    /// value is steady (paused / stopped), so idle playback costs nothing.
    fn tick(&mut self) {
        if !self.configured {
            return;
        }
        let elapsed = (self.time_source)();
        if hit::on_tick(&mut self.state, elapsed) {
            self.redraw();
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
                } if button == BTN_LEFT => match hit::on_press(&mut self.state, x, y) {
                    // Hand the drag to the compositor: it moves the window while the button
                    // is held, then ends the grab on release. This is the classic title-bar
                    // drag; Wayland has no client-set absolute position, so this is the way.
                    hit::PressOutcome::StartMove => {
                        if let Some(seat) = &self.seat {
                            self.window.move_(seat, serial);
                        }
                    }
                    hit::PressOutcome::Redraw => self.redraw(),
                    hit::PressOutcome::Ignore => {}
                },
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    let outcome = hit::on_release(&mut self.state, x, y);
                    if outcome.redraw {
                        self.redraw();
                    }
                    if let Some(command) = outcome.command {
                        (self.on_command)(command);
                    }
                }
                PointerEventKind::Leave { .. } => {
                    // Cancel any in-progress press so a button never stays stuck down.
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
