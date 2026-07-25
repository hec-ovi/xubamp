//! PipeWire realtime output: one persistent Output stream, the RT `process` callback that
//! drains the SPSC ring into the mapped buffer, and the loop-thread control channel.
//!
//! Built only with the `output` feature, which pulls in the system libpipewire FFI. The
//! whole file runs on a single dedicated loop thread: the PipeWire smart pointers are `!Send`
//! and must be constructed there. `run_loop` blocks in `mainloop.run()` until a
//! [`Control::Quit`] arrives over the control channel.
//!
//! RT-safety: the `process` closure runs on PipeWire's realtime data thread. It only
//! dequeues a buffer, copies from the ring via [`fill_output`] (no alloc, no lock, no
//! syscall), advances one atomic, and stamps the chunk. With `panic = "abort"` a panic there
//! aborts the process, so it clamps and uses fallible casts instead of unwrapping.

use std::io::Cursor;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;
use pw::spa::sys as spa_sys;
use rtrb::Consumer;

use crate::command::Control;
use crate::ring::{apply_gain, fill_output, SharedState, CHANNELS};

/// Bytes per interleaved stereo f32 frame (8): the negotiated stride of the output buffer.
const STRIDE: usize = std::mem::size_of::<f32>() * CHANNELS;

/// How many frames the realtime callback writes this cycle: what the graph asked for
/// (`buffer.requested()`), clamped to what the mapped buffer holds.
///
/// The two are far apart. PipeWire sizes the mapped buffer for a worst case (12288 frames on
/// Ubuntu 26.04) while a cycle wants one quantum, ~470 frames for a 22.05 kHz stream in a 48 kHz
/// graph. Filling the buffer end to end drains everything the producer has queued into a single
/// cycle and silence-pads the shortfall, so the stream emits a burst of audio and then a gap, over
/// and over. That stayed inaudible at 44.1 and 48 kHz only because the producer's half-second of
/// buffered audio is longer there than the mapped buffer; at 22.05 kHz half a second is 11025
/// frames against a 12288-frame buffer, and every cycle ended in ~57 ms of silence.
///
/// `requested` is 0 on PipeWire older than 0.3.49 (and on the first cycles of some sinks), where
/// the buffer size is the only figure available.
fn frames_this_cycle(requested: usize, capacity: usize) -> usize {
    if requested > 0 {
        requested.min(capacity)
    } else {
        capacity
    }
}

/// User data owned by the stream listener for the life of the loop.
///
/// `consumer` is touched only by the realtime `process` callback. `shared` is all atomics,
/// so both the loop-thread `param_changed` and the RT `process` may reach it safely.
pub struct RtData {
    /// Realtime read side of the ring. `rtrb::Consumer` is `Send`, moved in on construction.
    pub consumer: Consumer<f32>,
    /// Atomics shared with the producer and app threads.
    pub shared: Arc<SharedState>,
}

pub use pw::channel::{Receiver as ControlReceiver, Sender as ControlSender};

/// App/producer -> loop-thread control channel. The receiver is attached to the loop inside
/// [`run_loop`]; the sender stays with the caller (the engine, or the tone example).
pub fn control_channel() -> (ControlSender<Control>, ControlReceiver<Control>) {
    pw::channel::channel()
}

/// Run the PipeWire main loop on the calling thread until [`Control::Quit`].
///
/// Connects one persistent Output stream offering exactly one format: F32LE stereo at
/// `request_rate`, the track's own rate. Nothing downstream resamples, so the graph either accepts
/// that rate (its adapter converts to the device) or the stream fails to connect; the rate it
/// actually agreed to is read back in `param_changed` and published to `shared.stream_rate`, where
/// the live playback test checks it against the request. Spawn this on its own thread
/// (`std::thread::spawn(move || run_loop(rx, rt, 48000))`).
pub fn run_loop(
    rx: ControlReceiver<Control>,
    rt: RtData,
    request_rate: u32,
) -> Result<(), pw::Error> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    // Ask for ~11ms quanta (512 frames at the requested rate). PipeWire otherwise batches this
    // stream at whatever quantum the graph settles on (often 2048-8192 frames), and the
    // visualizer's sample tap then only refreshes a few times a second, which reads as a
    // slideshow no matter how fast the UI repaints. A latency REQUEST, so the graph may still
    // choose differently; playback correctness never depends on it.
    let latency = format!("512/{request_rate}");
    let stream = pw::stream::StreamRc::new(
        core,
        "xubamp",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::AUDIO_CHANNELS => "2",
            *pw::keys::NODE_LATENCY => latency.as_str(),
        },
    )?;

    // Marshal pause/resume/quit onto this loop thread. `attach` returns a `#[must_use]` guard
    // whose drop detaches the receiver, so keep it alive until `run()` returns. The Active handler
    // also publishes the play/pause state for the visualizer and a play indicator.
    let shared = Arc::clone(&rt.shared);
    let _control = rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        let stream = stream.clone();
        let shared = Arc::clone(&shared);
        move |c| match c {
            Control::Active(active) => {
                let _ = stream.set_active(active);
                shared.playing.store(active, Ordering::Relaxed);
            }
            Control::Quit => mainloop.quit(),
        }
    });

    // Listener owns `rt`. Both callbacks receive `&mut RtData`; they never run concurrently on
    // the same field (`param_changed` on the loop thread only touches the shared atomics,
    // `process` on the RT thread owns the consumer), so `&mut` aliasing is not a data race.
    let _listener = stream
        .add_local_listener_with_user_data(rt)
        .param_changed(|_stream, ud, id, param| {
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else { return };
            let mut info = spa::param::audio::AudioInfoRaw::new();
            if info.parse(param).is_ok() {
                ud.shared.stream_rate.store(info.rate(), Ordering::Release);
            }
        })
        .process(|stream, ud| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            // How many frames the graph wants this cycle, which is NOT the size of the mapped
            // buffer. See [`frames_this_cycle`].
            let requested = buffer.requested() as usize;
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];

            let mut frames = 0usize;
            if let Some(raw) = data.data() {
                frames = frames_this_cycle(requested, raw.len() / STRIDE);
                let usable = &mut raw[..frames * STRIDE];
                // Aligned (MAP_BUFFERS) and length is a multiple of 4, so this succeeds; the
                // fallible form keeps the RT thread panic-free if that ever fails to hold.
                // `fill_output` advances `frames_consumed` by the real frames it copies (before
                // padding), so trailing silence after a track's last frame never moves the clock
                // and the producer draining the ring sees the final count immediately.
                match bytemuck::try_cast_slice_mut::<u8, f32>(usable) {
                    Ok(out) => {
                        fill_output(&mut ud.consumer, out, &ud.shared);
                        // Scale by the current volume/balance gains. Unity (full volume, centered)
                        // short-circuits, so the common case adds nothing to the RT path.
                        let (gl, gr) = ud.shared.gains();
                        apply_gain(out, gl, gr);
                        // Tap the post-gain output for the visualizer (wait-free, no alloc).
                        ud.shared.push_scope(out);
                    }
                    Err(_) => usable.fill(0),
                }
            }

            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = STRIDE as _;
            *chunk.size_mut() = (frames * STRIDE) as _;
        })
        .register()?;

    // Offer exactly one EnumFormat: F32LE interleaved, `request_rate`, stereo FL/FR.
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(request_rate);
    info.set_channels(CHANNELS as u32);
    let mut position = [0u32; spa::param::audio::MAX_CHANNELS];
    position[0] = spa_sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa_sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);

    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(pw::spa::pod::Object {
            type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: spa::param::ParamType::EnumFormat.as_raw(),
            properties: info.into(),
        }),
    )
    .expect("serialize format pod")
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).expect("valid format pod")];

    stream.connect(
        spa::utils::Direction::Output,
        None, // let the session manager route to the default sink
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS // so data.data() returns the mapped bytes
            | pw::stream::StreamFlags::RT_PROCESS, // process runs on the realtime data thread
        &mut params,
    )?;
    // The stream connects active, so playback is on until the app pauses it.
    shared.playing.store(true, Ordering::Relaxed);

    mainloop.run(); // blocks until Control::Quit -> mainloop.quit(); keeps listeners alive
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::frames_this_cycle;

    #[test]
    fn a_cycle_writes_what_the_graph_asked_for_not_the_whole_buffer() {
        // A 22.05 kHz stream in a 48 kHz graph: ~470 frames per cycle out of a 12288-frame buffer.
        assert_eq!(frames_this_cycle(471, 12288), 471);
        // Never past the end of the mapping, whatever the graph claims to want.
        assert_eq!(frames_this_cycle(20_000, 12288), 12288);
        // PipeWire older than 0.3.49 reports nothing; the buffer size is all there is to go on.
        assert_eq!(frames_this_cycle(0, 12288), 12288);
    }
}
