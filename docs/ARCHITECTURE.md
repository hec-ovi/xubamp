# xubamp architecture and plan

This is the design record and the build order. It is the source of truth for what
xubamp is, what it deliberately is not, and the sequence in which it gets built.

## Target, fixed

One platform, tuned hard, nothing else:

- Ubuntu 26.04 LTS, GNOME 50 on Mutter. GNOME 50 is Wayland only (the X11 session is
  gone), so "GNOME 50" and "native Wayland" mean the same target here.
- PipeWire for audio (the only sound server on 26.04).
- x86-64 and aarch64.

No Windows, no macOS, no X11, no XWayland, no KDE or wlroots, no older Ubuntu. Every
fallback branch those would require is code we do not write. Single-target is the main
lever for staying small and fast.

## Principles

- Lean by construction. No widget toolkit (GTK/Qt/SDL). The UI is classic Winamp: a
  set of bitmap sprites blitted into a software framebuffer. That maps directly onto a
  Wayland `wl_shm` buffer and needs no GPU.
- Isolated crates. Each concern is its own workspace crate with its own tests. The
  pure parts (skin decode, DSP, playlist, config) have no I/O and no platform code, so
  they are tested headless and fast.
- Tests first, per piece. Every behavioral change ships tests that exercise the real
  entry point through to its effect (decode a real byte stream, round-trip a real
  playlist file, toggle real transport state), not mocks of internal functions.
- Performance is a budget, not an afterthought. See below.

## Performance and resource budget

Targets we hold the build to, and the rules that keep us there:

- Cold start to a drawn window: well under 100 ms.
- Idle RAM (RSS): aim under 25 MB including shared libraries.
- Release binary: small; full LTO, one codegen unit, `panic = "abort"`, stripped.
- The audio realtime callback does no allocation, no locking, no file I/O, no syscalls
  beyond the buffer copy. All decode, resample, and EQ happen on a producer thread and
  reach the callback through a lock-free single-producer/single-consumer ring.
- The renderer keeps one offscreen buffer per window, sized once, and repaints only
  damaged regions. No per-frame allocation on the UI path.
- Dependencies are justified one at a time. Prefer std and small, audited crates. Skin
  parsing, config, playlist, and DSP carry zero third-party dependencies.

Benchmarks (criterion) guard the hot paths (blit, resample, EQ, FFT) as they land.

## Crate map

A Cargo workspace, one job per crate. Crates appear as their phase is built.

- `skin` (present): `.wsz` container (zip), BMP decoder (all bit depths), config
  parsers (region.txt, pledit.txt, viscolor.txt), and the static sprite-coordinate
  tables. Pure; no I/O beyond turning bytes into pixels/structs.
- `render`: sprite compositor. Blit into an RGBA buffer, 9-slice tiling for the
  resizable playlist and the general window, the 5x6 bitmap-font text engine, the digit
  display, the scrolling marquee, the sliders (including the 28-frame volume/balance
  animation), per-pixel hit testing.
- `wl`: the native Wayland client. One undecorated `xdg_toplevel`, `wl_subsurface`
  children for the docked panes, `wl_shm` buffers, input regions and opaque regions,
  interactive move, fractional-scale handling, keyboard and pointer input.
- `audio`: decode + output engine. Producer thread, lock-free ring, PipeWire output,
  decoder dispatch, resample to a fixed internal format, next-track prefetch.
- `dsp`: 10-band peaking-biquad equalizer plus preamp, and EQF/.q1 preset I/O. Pure.
- `vis`: spectrum FFT and oscilloscope, fed from a post-EQ sample tap.
- `playlist`: the playlist model and `.m3u`/`.pls` read and write. Pure.
- `config`: preferences and session persistence. Pure over an injected path.
- `mpris`: MPRIS service (media keys, desktop media widget) and the GlobalShortcuts
  portal session, over D-Bus.
- `xubamp` (present): the binary. State machine, event loop, and the glue that wires
  the crates together.

## Data flow

Open a file, it enters the `playlist` model. The `audio` producer thread demuxes and
decodes a chunk, applies preamp and the `dsp` EQ, writes interleaved f32 to the PCM
ring (and a tagged copy to the `vis` ring), and the realtime callback copies the PCM
ring into the PipeWire buffer. In parallel the UI thread reads transport and EQ state,
reads the `vis` ring at the current playback position for the spectrum and scope, and
blits sprites into the `wl_shm` buffer, then damages and commits. Pointer and keyboard
events hit-test in 1x surface-local coordinates, mutate state, and signal the producer
thread on seek.

## Wayland strategy

Each classic behavior, and how it is done natively on GNOME 50 / Mutter:

- Borderless window: Mutter is client-side-decoration only, so an undecorated
  `xdg_toplevel` that simply draws no titlebar is already borderless. No libdecor.
- Non-rectangular shape (region.txt): draw alpha = 0 outside the skin polygon in the
  RGBA buffer; the compositor composites the transparent cutouts. Set `set_opaque_region`
  to the solid body as a hint and `set_input_region` from a rectangle decomposition so
  transparent corners click through, with a per-pixel alpha check for exact edges.
- Dragging a titlebar-less window: `xdg_toplevel.move(seat, serial)` from the pointer
  button-press handler over the title strip.
- Multi-window docking: model the docked main + EQ + playlist cluster as one
  `xdg_toplevel` with `wl_subsurface` children positioned by the app. Wayland does not
  let a client set the absolute position of separate top-levels, so the app owns the
  relative geometry instead and snaps the panes itself. Subsurfaces allow negative
  offsets and are not clipped to the parent, so this reproduces the rigid snap-as-one
  feel. Limitation accepted by design: the cluster is one window to the compositor
  (one task entry, one move grab); a pane cannot be torn off into an independent
  free-floating OS window, because no GNOME protocol allows a client to place it.
- Always on top: not client-controllable on Mutter (no protocol; window stacking is not
  exposed to apps, by design). Handled as a documented manual action: GNOME's built-in
  Super plus right-click "Always on Top", which works on our borderless window, or a
  user-set shortcut via `org.gnome.desktop.wm.keybindings always-on-top`.
- Global media keys: MPRIS. gnome-settings-daemon intercepts the hardware play/pause/
  next/prev keys and dispatches to the active MPRIS player, natively on Wayland.
- Other global shortcuts: the `org.freedesktop.portal.GlobalShortcuts` portal, present
  since GNOME 48 and so available on GNOME 50, behind a one-time user approval.
- In-window hotkeys (the z/x/c/v/b row, numpad, Alt and Ctrl toggles): plain
  `wl_keyboard` plus libxkbcommon while focused, no portal, no restriction.
- HiDPI: `wp_fractional_scale_v1` plus `wp_viewport`, rendering the fixed 1x skin and
  integer-upscaling with nearest-neighbor so pixels stay crisp at 125/150/200 percent.

## Playlist behavior

The classic Winamp add/save flow, in full:

- Add single songs (one or many files).
- Add a folder, recursing into subfolders, filtered to supported audio extensions.
- Save and load `.m3u` (extended, with `#EXTINF`) and `.pls` (INI style).
- Remove, clear, sort (title/filename), reverse, randomize, and manual reorder.
- Double-click a row to play; remember and restore the playlist across runs.

## Phased plan

Each phase produces a runnable, testable artifact. Tests run locally (a `cargo test`
target); no CI is added.

- Phase 0 (done): workspace scaffold and the isolated BMP decoder with tests.
- Phase 1 (done): `.wsz` container reader (case-insensitive, default-skin fallback) plus
  the region/pledit/viscolor config parsers. Test: build real archives, assert parsed
  structs.
- Phase 2: a native Wayland window showing a static render of MAIN, CBUTTONS, and
  TITLEBAR from a bundled default skin. Test: headless offscreen render diffed against
  a reference image.
- Phase 3: audio. Producer thread, lock-free ring, PipeWire output, WAV then MP3 decode.
  Test: play to a file/null sink, assert PCM checksum and duration; assert no allocation
  on the callback path.
- Phase 4: transport and a real skin. Buttons wired, time display, marquee, sliders,
  in-window hotkeys, drag. Test: synthetic input into the real loop asserts state
  transitions and that a click on the play sprite starts playback.
- Phase 5: playlist. The full add/save/load behavior above, the resizable playlist
  window. Test: round-trip real `.m3u` and `.pls`; folder-add over a temp tree; resize
  with no sprite tearing.
- Phase 6: EQ and visualizer. Biquad bank, EQF presets, spectrum and scope. Test: EQF
  byte round-trip; a known sine attenuated/boosted by the expected dB per band.
- Phase 7: MPRIS, GlobalShortcuts portal, region shaping, fractional scale, windowshade
  and doublesize, cursors. Test: MPRIS PlayPause over D-Bus toggles playback; point-in-
  polygon hit tests; a scaled render diff.
- Phase 8: packaging. `debian/` with debhelper 13, `.desktop` with audio MIME types, a
  shared-mime-info entry for `.wsz`, AppStream metainfo, man page; a `.deb` and a
  Launchpad PPA. Test: install the built `.deb` in a clean container, launch, load a
  skin, assert it renders.

## Decisions

- Language: Rust. The native-Wayland-in-Rust stack is production proven, memory safe,
  and has mature crates for the fiddly D-Bus/portal/decoder work. Runner-up was C for
  raw minimalism.
- License: GPL-2.0-or-later.
- Always on top: manual GNOME action, no bundled shell extension.
- Windowing: raw Wayland with a `wl_shm` software framebuffer, not SDL or a toolkit.

## Clean-room note

Implemented from public format documentation only. No code is copied from Winamp, from
the 2024 Winamp source release, or from GPL players such as Audacious, qmmp, or XMMS.
The Webamp project (MIT) is used only as a machine-readable reference for the fixed skin
coordinate data (facts about the format), never as a code source. xubamp ships no
Winamp skins, names, or artwork.

## References

Load-bearing external facts behind the Wayland strategy:

- No client window stacking / always on top on Mutter, by design: GNOME Discourse,
  https://discourse.gnome.org/t/any-way-to-set-window-always-on-top-programmatically/31579
- Mutter does not implement wlr-layer-shell: https://gitlab.gnome.org/GNOME/mutter/-/issues/973
  and the support matrix at https://wayland.app/protocols/wlr-layer-shell-unstable-v1
- Clients cannot set absolute top-level positions (rationale):
  https://canonical.com/mir/docs/2.26/explanation/window-positions-under-wayland/
- Subsurfaces allow negative offsets and are not clipped to the parent:
  https://wayland-book.com/surfaces-in-depth/subsurfaces.html
- GlobalShortcuts portal added in GNOME 48: https://release.gnome.org/48/developers/index.html
- Media keys via MPRIS and gnome-settings-daemon:
  https://work.lisk.in/2020/05/06/linux-media-control.html
- Existing skinned players fall back to XWayland on GNOME (what we avoid):
  https://audacious-media-player.org/problems
