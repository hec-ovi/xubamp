# xubamp

A from-scratch, native-Wayland reimplementation of the classic Winamp 2.9x player, built for one target only: Ubuntu 26.04.

It plays music. The main window renders a `.wsz` skin pixel for pixel, with working transport, a seek bar, volume and balance, the spectrum/oscilloscope visualizer, a resizable playlist window, and the classic hotkeys. MP3 and WAV decode through Symphonia and play over PipeWire. The 10-band equalizer is the main piece still to come. Build order and design notes live in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md); the running log is in [docs/PROGRESS.md](docs/PROGRESS.md).

## Why

Winamp 2.9x did a lot with almost nothing: a 275x116 bitmap UI, a software blitter, and instant startup on 2003 hardware. Reproducing that on a modern machine should come out smaller and faster, not heavier. xubamp aims for a tiny binary, low memory, and the classic skin, hotkey, playlist, and EQ behavior, running natively on Wayland with no widget toolkit and no XWayland.

## What works

- Loads classic Winamp 2.x skins (`.wsz`) and renders the main window pixel for pixel.
- Playback of MP3 and WAV through PipeWire, with a Bluetooth-safe gapless seek.
- Transport (play, pause, stop, previous, next), a dragged seek bar, volume and balance sliders (the value shows in the marquee while you drag), a running-time display, and the scrolling song-title marquee.
- Spectrum and oscilloscope visualizer; click it to cycle the mode.
- Shuffle and repeat, with Previous/Next that retrace the real play order, so Previous still works under shuffle.
- A separate playlist window: click to select (Ctrl and Shift extend), double-click to play, mouse-wheel scroll, resize by the bottom-right grip, and it remembers its size across close and reopen.
- A separate "Jump to file" dialog on `J`: type to filter the tracks, Enter or a double-click plays the pick, and it leaves the playlist untouched.
- Classic hotkeys: `z x c v b` for the transport, arrow keys for volume and seek, `J` to jump.

Still to come: the 10-band equalizer window and DSP, windowshade (collapsed) mode, FLAC and Ogg Vorbis, and `.m3u`/`.pls` playlist files. No album art, no media library, no modern chrome, on purpose.

Out of scope: Windows and macOS, X11, KDE and other compositors, older Ubuntu. This is tuned for Ubuntu 26.04 (GNOME 50, Mutter, Wayland, PipeWire) and nothing else. Targeting one platform is what keeps it small.

## Wayland notes

Wayland does not let a client position its own windows or read another window's position, so a couple of things Winamp assumed work differently:

- The playlist and jump-to-file windows are separate top-level windows, placed by the compositor. They cannot magnetically snap or dock to the main window the way Winamp's did, because the protocol forbids a client from setting a window's position. Reopening a window restores its size but not its place. (Compositing the docked panes as subsurfaces of one window is a possible future route; it is a larger change.)
- Always on top is a manual GNOME action (Super plus right-click, or a shortcut you bind), not an in-app toggle. Mutter does not expose window stacking to applications.

## Layout

A small Cargo workspace, one job per crate:

- `crates/skin` decodes `.wsz` skins (BMP sprites, config text) and holds the sprite geometry. No I/O beyond unzip.
- `crates/render` composes each window's framebuffer from the skin and the UI state and owns the hit-testing. Pure and heavily unit-tested.
- `crates/audio` decodes with Symphonia and plays through PipeWire, feeding a lock-free ring the realtime callback drains (no allocation or locking on the audio thread).
- `crates/wl` is the native Wayland layer (smithay-client-toolkit): windows, input, and the software blit into shared-memory buffers.
- `crates/xubamp` is the binary that wires them together.

## Build

Needs a Rust toolchain (pinned to 1.96.0).

    cargo build --release
    cargo test

The UI and skin crates build and test on the host with no system libraries. Playback and in-window keyboard shortcuts sit behind the `audio` and `keyboard` features, which need PipeWire and libxkbcommon; a dev container keeps those off the host and runs the app against the host's Wayland and PipeWire sockets:

    scripts/dev-docker.sh run skins/your-skin.wsz ~/Music/song.mp3

Bring your own `.wsz` skin and audio files.

## License

GPL-2.0-or-later; see [LICENSE](LICENSE). This is an independent, clean-room implementation written from public format documentation. It contains no Winamp code and ships no Winamp skins, names, or artwork.
