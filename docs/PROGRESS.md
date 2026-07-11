# xubamp progress

Resume point after any context compaction: read this file, then
[ARCHITECTURE.md](ARCHITECTURE.md), then `git log --oneline`. Everything durable is
in the repo and git history, nothing important lives only in chat.

## Done

- Phase 0: Cargo workspace scaffold, lean release profile, GPL-2.0 license, README,
  architecture and plan doc.
- Phase 0: `crates/skin` BMP decoder (1/4/8/24/32-bit, top-down and bottom-up) with 5
  unit tests, clippy clean.
- Phase 1: `crates/skin` `.wsz` container reader (hand-rolled ZIP over miniz_oxide,
  case-insensitive lookup, default-skin fallback contract) plus the config parsers
  (viscolor.txt, pledit.txt, region.txt) and the shared `Rgb` type. 19 tests total,
  skin-crate dependency surface is 2 crates (miniz_oxide, adler2).

- Phase 2: `render` crate (Framebuffer, clipping blit, compose_main_window) plus the
  `wl` crate (native Wayland window via smithay-client-toolkit + wl_shm). Verified on the
  real Ubuntu 26.04 GNOME 50 session, and validated against real skins (SpyAMP, XMMS, and
  the RLE8-compressed base 2.91) by dumping the composed frame. The binary loads a skin
  from a path argument. 26 tests.

- Built-in default skin and app packaging: an original clean-room 275x116 default skin
  (`skin::default_skin`) drawn in code in a cyan/blue classic layout, plus a compact 5x7
  bitmap font (`skin::font`) for the pixels we author ourselves. Ships no third-party skin
  art (every classic `.wsz` is copyrighted). The binary resolves its skin in order: CLI
  path, `$XUBAMP_SKIN`, a local `skins/` dev skin if checked out, else the built-in
  default. App icon packaged from `icons/` (sizes 32-256 generated from the 1024 master)
  with a validated `packaging/xubamp.desktop` (app_id `xubamp`) and install/uninstall
  scripts. 31 tests.

## Running it

- `cargo run -p xubamp` shows the default (or your local `skins/` dev skin if present).
- `cargo run -p xubamp -- path/to/skin.wsz` loads a specific skin.
- `scripts/dev-docker.sh run ~/Music/song.mp3` plays a track with the window (audio needs the
  dev container; the plain host build has no PipeWire deps and just shows the window).
- `./packaging/install-icons.sh` puts the icon + desktop entry under `~/.local/share` so
  GNOME shows the app icon; `uninstall-icons.sh` reverses it.
- User is authoring the real default skin; the built-in one is only the safe fallback.

## In progress

- Phase 4: interactivity. Sub-units:
  - (a) done: pointer input and interactive title-bar drag. The `wl` crate now binds `wl_seat`
    and `wl_pointer` (SeatHandler + PointerHandler; still `default-features = false`, so no
    calloop and no xkbcommon keyboard) and a left-button press on the title-bar band hands an
    interactive move to the compositor via `xdg_toplevel.move` (`Window::move_`). Wayland has no
    client-set absolute position, so a compositor grab is the way to do the classic drag. Hit
    mapping is a pure `render::hit::hit_test(x, y) -> Region` (unit-tested: title-bar band is the
    top 14px from the title-bar sprite, body and out-of-bounds are not draggable); the platform
    glue is verified by running on the real GNOME 50 session.
  - (b) done: transport buttons are interactive. `render::hit` maps the six button rects to a
    `Transport` id and holds the input policy as pure functions (`on_press` arms a button or starts a
    title-bar move, `on_release` fires the command only when released over the same button, `on_leave`
    un-presses), all unit-tested. `compose_main_window(skin, &UiState)` draws the pressed sprite (the
    bottom row of cbuttons.bmp, coordinates from Webamp's sprite map) for the held button, and the `wl`
    crate recomposes and redraws the 275x116 frame on each pointer event. `run(skin, on_command)` emits
    a `Transport` command to the caller on a completed click; the binary logs it for now. Wiring those
    commands to the engine (play/pause/stop) is (c).
  - (c) done: transport commands drive the engine. `AudioEngine::handle()` returns a cloneable
    `EngineHandle` (a clone of the PipeWire control channel, so it can outlive borrows of the engine and
    coexist with the engine's own shutdown control). The binary bridges the window's `Transport` commands
    to it: Play resumes, Pause and Stop deactivate the stream (the realtime callback stops pulling frames
    and the position clock holds; Stop reset-to-start waits for decoder seeking with the seek bar). Prev,
    Next and Eject wait for a playlist. Pausing is a stream deactivation, so no decoder changes were
    needed. Verified in the dev container against a silent null sink: an ignored engine test asserts the
    position clock holds while paused and advances again on resume.
  - (d) done: running time display. The main window shows elapsed MM:SS and updates once a second.
    `render::hit::UiState` gained an `elapsed` seconds field with a pure `on_tick` that sets it and
    reports whether the shown value moved, so a held (paused) clock costs no redraw;
    `compose_main_window` draws the four digits from the skin's number sheet (`nums_ex.bmp` preferred,
    else `numbers.bmp`) at the classic destinations, and `mmss_digits` splits seconds into digit values
    (minutes clamp at 99). The `wl` crate moved off the blocking Wayland dispatch to a calloop event
    loop (SCTK's `calloop` feature, pure Rust, no new system deps) with a re-arming ~1s `Timer` that
    polls a `time_source` closure and recomposes only on a change. The binary feeds it
    `EngineHandle::elapsed_secs()` (a clone of the position clock plus the stream rate). Digit
    sprites/destinations, the tick policy, and the seconds split are unit-tested; the elapsed clock is
    checked against a null sink in the ignored engine test; verified live on GNOME. End-of-track freeze
    (the clock currently counts silence past a track's end, needing an end-of-stream signal), the pause
    blink, and the remaining-time toggle come next.

- Phase 3: audio engine. Written plan first (see ARCHITECTURE.md). Sub-units:
  - (a) done: Symphonia decode (WAV + MP3), channel map to stereo. Pure Rust.
  - (b) done: lock-free SPSC ring (`audio::ring`: `SharedState`, `new_ring`, `push_block`,
    `fill_output`) with round-trip/wrap/underrun/flush tests and a counting-allocator proof
    that the realtime path never allocates. Pure Rust (rtrb).
  - (c) done: PipeWire output (`audio::output`: `run_loop`, `RtData`, the RT `process`
    callback, `param_changed` rate readback, control channel) plus `command::Control` and
    `examples/tone.rs`. Behind the `output` cargo feature (default off) so the pipewire FFI
    only builds in the dev container; the pure decode/ring/channels still build on the host.
    Verified live against the real PipeWire daemon from the container: the ignored
    `tests/live_playback.rs` connects, negotiates 48 kHz, and the RT callback consumes frames;
    a null-sink capture of the tone example shows a clean 440 Hz sine (format/stride/channel
    map correct end to end). Built against pipewire 0.10.0 / libspa 0.10.0.
  - (d) done (minimal): `audio::engine::AudioEngine::play(path)` spawns the output loop + a
    decode/producer thread and starts playback; `Drop` quits the loop and joins both threads
    cleanly (no hang in any close scenario). Hooked into the binary: `xubamp song.mp3` plays
    the track while the window shows. Arguments are classified by extension (`.wsz` -> skin,
    audio -> track). Audio is behind the binary's `audio` feature (off by default) so host UI
    dev stays PipeWire-free; the dev container runs `--features audio`. Streams at the file's
    native rate (PipeWire converts to the device), so no resampler is needed yet. Verified with
    a real MP3 (audible) and an ignored engine test (generated WAV -> null sink -> asserts the
    clock advances). Transport (pause/resume/stop/seek), a time display and playlists come with
    the interactivity phase; (e) a fixed-rate + own-resampler design is optional and deferred.

- Dev build for the PipeWire crates runs in Docker so the host stays clean: `Dockerfile.dev`
  (Ubuntu 26.04, rust pinned to 1.96.0, clippy+rustfmt, and the PipeWire client runtime bits
  `pipewire-bin` + `libspa-0.2-modules` so a program in the container can connect to the host
  daemon over the mounted socket) + `scripts/dev-docker.sh {image|build|test|run|shell}`. The
  audio `output` feature builds there with `--features output`; pure crates
  (skin/render/wl/audio-so-far) still build and test natively on the host.

## Next

- Phase 4 (continued): end-of-track auto-stop so the clock freezes at the true end (the engine
  needs an end-of-stream signal), the marquee song title, seek/volume/balance sliders, in-window
  hotkeys (needs keyboard input, i.e. re-enabling xkbcommon), and the spectrum/oscilloscope.
  Polish: pause-blink and the click-to-toggle remaining-time display. Plus a real skin.

## Working rules

- Commit and push per green sub-unit: `cargo build` + `cargo test` + `cargo clippy`
  all pass first. Small and frequent.
- Each phase ends on a runnable, testable artifact; that push is a safe point to
  compact context.
- No CI and no GitHub Actions. Tests run locally via `cargo test`.
