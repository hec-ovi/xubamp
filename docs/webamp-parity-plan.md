# Webamp-parity work plan

Goal: make xubamp usable and faithful to Winamp 2.9x / Webamp (no video/Milkdrop): load
music, save and load playlists, every UI control working with feedback, no freeze, and the
non-skin UI styled native to GNOME 50 (Adwaita). Reference behavior is captbaritone/webamp;
the durable behavior spec lives in `.research/winamp-webamp-behavior/FINDINGS.md`.

Scope decisions (confirmed with the user): Adwaita-styled custom rendering (no GTK toolkit);
EQ already works, only its reset control is missing; add FLAC + Ogg Vorbis decode; commit and
push to main per green sub-unit. Out of scope: Milkdrop/AVS video viz, network/URL streaming.

## Root causes found (investigation)

- FREEZE / "view does not update": `close_popup_menu` tears down the popup subsurface without
  committing the parent surface, so on Mutter the menu stays latched on screen and the main
  view looks frozen until the next input/clock tick. Menu items that only call a sink
  (File.../Load Skin/Add/Save-EQ) or dismiss never repaint the main surface. Also
  `frame_pending` can latch true forever if a frame callback is dropped (minimize/occlude),
  permanently killing the visualizer loop.
- "File load not working": the wiring is intact in the audio build; the lingering popup + the
  missing post-load main redraw made it look dead. The dev container also did not export
  `DBUS_SESSION_BUS_ADDRESS`, though the bus socket is mounted and the host runs the portal.
- "Base skin controls not working": hit-testing is skin-independent and works. But
  `default_skin()` bakes a static image and leaves every overlay sheet `None`, and
  `compose_main_window` gates all dynamic feedback (pressed buttons, thumbs, digits, marquee,
  viz) behind `if let Some(overlay)`. So on the base skin nothing on screen ever moves.
- "EQ reset (the 0db thing)": authentic Winamp has three left-edge labels `+12db/0db/-12db`
  that flatten all 10 bands to that level (0db = flat), leaving the preamp. No slider
  double-click reset exists in real Winamp. Currently unimplemented in xubamp.
- Playlist editor: only the ADD cluster works. REM/SEL/MISC/LIST are inert baked art; no time
  display, no scrollbar thumb, no remove key, no save/load.

## Execution order (commit + push per green unit)

0. Freeze fix: commit parent surface in `close_popup_menu`; redraw main after sink-only /
   dismissed menu outcomes; flip local repeat/shuffle + redraw on menu toggles; reset
   `frame_pending` when `animating()` goes false / on unminimize.
1. Load music robustly: post-load main+playlist redraw and marquee refresh; parent the portal
   chooser; dev-docker.sh exports `DBUS_SESSION_BUS_ADDRESS` so it is verifiable in-container.
2. FLAC + Ogg Vorbis: Symphonia `flac`+`ogg`+`vorbis` features; classify `.flac/.ogg/.oga`.
3. Playlist save/load: pure `.m3u`/`.m3u8`/`.pls` reader+writer; wire LIST New/Save/Load with
   the portal save/open dialogs.
4. Playlist editor completeness: REM/SEL/MISC/LIST fly-out menus + actions; selected/total
   time display (`M:SS/M:SS`); scrollbar thumb draw+drag; Del removes selected, Ctrl+A all.
5. Base skin feedback: draw dynamic states (pressed buttons, thumbs, clock digits, marquee,
   viz) on the base skin so its controls are visibly live.
6. EQ reset labels: render+hit `+12db/0db/-12db`, flatten the 10 bands; ±5-unit snap-to-center.
7. Double-size mode (Ctrl+D / menu / clutterbar D): 2x main window with scaled hit-testing.
8. Native GNOME/Adwaita theming for the non-skin UI (menus, playlist fly-outs, Jump,
   Preferences): Adwaita light+dark palette, system UI font, rounded corners, hover/focus.
9. Preferences wiring: `Runtime::with_preferences`; seed model from settings; sink applies and
   persists (morph rate, library roots, skin path, double-size).
10. Remaining menu items: ToggleMainWindow, Back/Forward ten tracks; decide URL (out of scope).
11. Final adversarial review + live in-container verification; docs + memory refresh.

Every unit ships tests (pure logic unit-tested; Wayland glue verified live in the container).
