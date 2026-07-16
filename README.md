# xubamp

A from-scratch, native-Wayland reimplementation of the classic Winamp 2.9x player, built for one target: Ubuntu 26.04. No Wine, no XWayland, no widget toolkit. One 4.4 MB binary in a 1.5 MB deb that depends on libc, libgcc, PipeWire, and xkbcommon.

<!-- demo GIF here: main window + EQ + playlist with the visualizer running -->

## Install

Download the deb from [Releases](https://github.com/hec-ovi/xubamp/releases), then:

    sudo apt install ./xubamp_*_amd64.deb
    xubamp song.mp3

amd64 only for now. It starts on a built-in clean-room skin; point it at any classic `.wsz` on the command line (`xubamp skin.wsz *.mp3`) or switch at runtime from Preferences. No skins ship with it (see License).

## What works

- Classic `.wsz` skins rendered pixel for pixel: main window, equalizer, resizable playlist, windowshade modes on all three, double-size mode (Ctrl+D).
- MP3, WAV, FLAC, and Ogg Vorbis decoded with Symphonia, played through PipeWire, with gapless seek.
- The visualizer is a faithful port of the XMMS/Audacious classic analyzer: log bands, 40 dB range, normal/fire/line styles, thick or thin bands, peaks, five falloff speeds, 10 to 70 fps refresh.
- The 10-band equalizer with preamp, the 17 classic presets, and EQF load/save.
- Playlist editor with working ADD/REM/SEL/MISC/LIST clusters, `.m3u`/`.pls` save and load, right-click track menu, per-track times, mini clock and mini transport, a skinned scrollbar, and a resize grip. The playlist survives close and reopen.
- Jump-to-file (`j`) searches every tag the file carries, not just the shown title.
- Marquee shows the tagged "N. Artist - Title (M:SS)" (ID3v2, Vorbis comments, RIFF INFO; file name when untagged), next to kbps/kHz readouts and the elapsed/remaining clock with the classic paused blink.
- Shuffle and repeat retrace the real play order, so Previous works under shuffle.
- File info box with the stream facts and an editable ID3v1 tag form for MP3s.
- Classic hotkeys (`z x c v b` transport, `r s`, `l`, `j`, arrows for volume and seek, `Ctrl+T/D/P`, `Alt+3`) and mouse wheel for volume, balance, seek, and list scroll.
- Menus, Preferences, Jump, and file info are native GNOME dialogs (Adwaita, light and dark); everything else is skin-rendered.
- Every window drags from any free surface, not just the 14px title strip.

No album art, no library view, no URL streaming (its menu item is disabled), no modern chrome, on purpose.

Out of scope: X11, other compositors or distros, Windows, macOS. Tuned for Ubuntu 26.04 (GNOME 50, Mutter, Wayland, PipeWire) and nothing else; targeting one platform is what keeps it small.

## Why

Winamp 2.9x did a lot with almost nothing: a 275x116 bitmap UI, a software blitter, instant startup on 2003 hardware. Reproducing that today should come out smaller, not heavier. The existing routes either emulate (Wine) or carry a toolkit and X11 history (Audacious, QMMP); xubamp draws the skin bitmaps straight into Wayland shared memory.

## Wayland notes

Wayland does not let a client position its own windows, so two things work differently:

- The equalizer and playlist are child surfaces of the main window: they dock, edge-snap (threshold in Preferences), and travel with it like the classic cluster. Dialogs are ordinary top-level windows placed by the compositor.
- Always on top is a manual GNOME action (Super plus right-click), because Mutter gives applications no stacking control; the clutterbar's A button shows a notice saying so.

## Build

Needs Rust 1.96:

    cargo build --release
    cargo test

The UI and skin crates build and test on the host with no system libraries. Playback and in-window keyboard input sit behind the `audio` and `keyboard` features (PipeWire, libxkbcommon); a dev container keeps those off the host and runs the app against the host's Wayland and PipeWire sockets:

    scripts/dev-docker.sh run skins/your-skin.wsz ~/Music/song.mp3

## Layout

A small Cargo workspace, one job per crate: `skin` (wsz/BMP decode, sprite geometry), `render` (pure framebuffer composition and hit testing, heavily unit-tested), `audio` (Symphonia decode, PipeWire output, lock-free ring into the realtime thread), `wl` (smithay-client-toolkit windows, input, shm blit), `dsp` (equalizer filters and presets), `config` (settings file), `library` (audio path scanning), `portal` (XDG portals), `xubamp` (the binary). Design notes live in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), the running log in [docs/PROGRESS.md](docs/PROGRESS.md).

## License

GPL-2.0-or-later; see [LICENSE](LICENSE). This is an independent, clean-room implementation written from public format documentation. It contains no Winamp code and ships no Winamp skins, names, or artwork.
