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
- `./packaging/install-icons.sh` puts the icon + desktop entry under `~/.local/share` so
  GNOME shows the app icon; `uninstall-icons.sh` reverses it.
- User is authoring the real default skin; the built-in one is only the safe fallback.

## In progress

- Phase 3: audio engine (producer thread, lock-free ring, native PipeWire output,
  WAV then MP3 decode). Second "strong" phase; gets a written plan before code.

## Next

- Phase 4: transport and a real skin (buttons wired, time display, marquee, sliders,
  in-window hotkeys, drag).

## Working rules

- Commit and push per green sub-unit: `cargo build` + `cargo test` + `cargo clippy`
  all pass first. Small and frequent.
- Each phase ends on a runnable, testable artifact; that push is a safe point to
  compact context.
- No CI and no GitHub Actions. Tests run locally via `cargo test`.
