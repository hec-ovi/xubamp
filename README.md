# xubamp

A from-scratch, native-Wayland reimplementation of the classic Winamp 2.9x player, built for one target only: Ubuntu 26.04.

It plays music. The main window renders a `.wsz` skin pixel for pixel, with working transport, seek bar, volume and balance, the spectrum/oscilloscope visualizer, the 10-band equalizer, a resizable playlist window, and the classic hotkeys. MP3, WAV, FLAC, and Ogg Vorbis decode through Symphonia and play over PipeWire. Build order and design notes live in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md); the running log is in [docs/PROGRESS.md](docs/PROGRESS.md).

## Why

Winamp 2.9x did a lot with almost nothing: a 275x116 bitmap UI, a software blitter, and instant startup on 2003 hardware. Reproducing that on a modern machine should come out smaller and faster, not heavier. xubamp aims for a tiny binary, low memory, and the classic skin, hotkey, playlist, and EQ behavior, running natively on Wayland with no widget toolkit and no XWayland.

## What works

- Loads classic Winamp 2.x skins (`.wsz`) and renders the main, equalizer, and playlist windows pixel for pixel; skins switch at runtime from the menu or Preferences.
- Playback of MP3, WAV, FLAC, and Ogg Vorbis through PipeWire, with a Bluetooth-safe gapless seek.
- Transport (play, pause, stop, previous, next, eject), a dragged seek bar, volume and balance sliders (the value shows in the marquee while you drag), elapsed/remaining clock with the classic paused blink, the play/pause/stop indicator, kbps/kHz/mono/stereo readouts, and a scrolling marquee showing the tagged "N. Artist - Title (M:SS)" (ID3v2, Vorbis comments, RIFF INFO; file name when untagged).
- Spectrum and oscilloscope visualizer with the classic options: analyzer styles (normal/fire/line), thick or thin bands, peaks and falloff speeds, oscilloscope styles, refresh rate. Click the panel to cycle the mode.
- The 10-band equalizer with preamp, the 17 classic presets, EQF load/save, +12/0/-12 db flatten labels, and its own windowshade strip.
- Shuffle and repeat, with Previous/Next that retrace the real play order, so Previous still works under shuffle.
- The playlist editor: click to select (Ctrl and Shift extend), double-click to play, per-track durations and the selected/total readout, a live current-track clock, working ADD/REM/SEL/MISC/LIST clusters (including `.m3u`/`.pls` save and load and an Audio Library scan), a draggable scrollbar, resize grip, and a windowshade strip.
- Windowshade mode on all three panes; the main strip shows the title, mini clock, mini seek bar, and mini transport.
- Double-size mode (Ctrl+D, the menu, or the clutterbar D) doubles the main window and equalizer.
- The clutterbar: O pops the menu, I the file info box, D double size, V the visualization menu.
- A file info box (playlist MISC, Alt+3, or clutterbar I) showing the stream facts and an editable ID3v1 tag form for MP3s.
- Native GNOME-styled (Adwaita, light and dark) menus, Preferences, Jump-to-file, and file info dialogs; everything else is skin-rendered.
- Preferences pages: Shuffle morph rate, Options (read titles on load/play, sort on load, manual advance, title conversions), Visualization, Display (time mode, double size, title scroll, clutterbar, playlist numbers, snap distance), Audio Library roots, and Skins.
- Classic hotkeys: `z x c v b` transport, `r s` repeat/shuffle, `l` open files, arrows for volume and seek, `j` jump, `Ctrl+T` time mode, `Ctrl+D` double size, `Ctrl+P` preferences, `Alt+3` file info, Del/Ctrl+A in the playlist.
- Every window drags from any free surface, not just the 14px title strip.

No album art, no media library view, no modern chrome, on purpose. Add URL (network streaming) is not implemented; its menu item is disabled.

Out of scope: Windows and macOS, X11, KDE and other compositors, older Ubuntu. This is tuned for Ubuntu 26.04 (GNOME 50, Mutter, Wayland, PipeWire) and nothing else. Targeting one platform is what keeps it small.

## Wayland notes

Wayland does not let a client position its own windows or read another window's position, so a couple of things Winamp assumed work differently:

- The equalizer and playlist panes are child surfaces of the main window, so they dock, edge-snap (threshold configurable in Preferences), and travel with it like the classic cluster. The dialogs (Jump, Preferences, file info) are ordinary top-level windows placed by the compositor.
- Always on top is a manual GNOME action (Super plus right-click, or a shortcut you bind), not an in-app toggle. Mutter does not expose window stacking to applications, which is why the clutterbar's A button only shows a notice.

## Layout

A small Cargo workspace, one job per crate:

- `crates/skin` decodes `.wsz` skins (BMP sprites, config text) and holds the sprite geometry. No I/O beyond unzip.
- `crates/render` composes each window's framebuffer from the skin and the UI state and owns the hit-testing. Pure and heavily unit-tested.
- `crates/audio` decodes with Symphonia and plays through PipeWire, feeding a lock-free ring the realtime callback drains (no allocation or locking on the audio thread).
- `crates/wl` is the native Wayland layer (smithay-client-toolkit): windows, input, and the software blit into shared-memory buffers.
- `crates/dsp` is the 10-band equalizer filter and the classic preset table.
- `crates/config` parses and writes the settings file (`~/.config/xubamp/settings.conf`).
- `crates/library` classifies audio paths and scans directories.
- `crates/portal` talks to the XDG desktop portals (file choosers, color scheme).
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
