# xubamp

A from-scratch, native-Wayland reimplementation of the classic Winamp 2.9x player, built for one target only: Ubuntu 26.04.

Status: early. This is the scaffold plus the first isolated piece of the skin engine (the BMP decoder). Nothing plays music yet. The build order and design live in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Why

Winamp 2.9x did a lot with almost nothing: a 275x116 bitmap UI, a software blitter, and instant startup on 2003 hardware. Reproducing that on a modern machine should come out smaller and faster, not heavier. xubamp aims for a tiny binary, low memory, and the classic skin, hotkey, playlist, and EQ behavior, running natively on Wayland with no widget toolkit and no XWayland.

## Scope

- Loads classic Winamp 2.x skins (`.wsz`) and renders them pixel for pixel.
- Classic hotkeys, 10-band EQ, spectrum and oscilloscope visualizer.
- Playlists the classic way: add single songs, add a folder (recursing into subfolders), save and load `.m3u` and `.pls`, remove, sort, reorder.
- MP3 and WAV first, then FLAC and Ogg Vorbis.
- No album art, no media library, no modern chrome. The same thing Winamp did, nothing more.

Out of scope on purpose: Windows and macOS, X11, KDE and other compositors, older Ubuntu. This is tuned for Ubuntu 26.04 (GNOME 50, Mutter, Wayland, PipeWire) and nothing else. Targeting one platform is what keeps it small.

## Wayland notes

Wayland does not allow some things Winamp assumed, so a few behaviors differ by design:

- The main, equalizer, and playlist windows are one top-level window with internal panes that snap together. Wayland does not let a client place separate windows, so they dock and move as a unit rather than as three free-floating windows.
- Always on top is a manual GNOME action (Super plus right-click, or a shortcut you bind), not an in-app toggle. Mutter does not expose window stacking to applications.
- Hardware media keys work through MPRIS; other global shortcuts go through the desktop portal.

## Layout

A small Cargo workspace, one job per crate:

- `crates/skin` decodes `.wsz` skins (BMP sprites, config text) with no I/O and no dependencies beyond unzip. Unit-tested in isolation.
- `crates/xubamp` is the binary; it grows window, audio, render, and DSP crates as the phases land.

## Build

Needs a Rust toolchain.

    cargo build --release
    cargo test

## License

GPL-2.0-or-later; see [LICENSE](LICENSE). This is an independent, clean-room implementation written from public format documentation. It contains no Winamp code and ships no Winamp skins, names, or artwork. Bring your own `.wsz` skins.
