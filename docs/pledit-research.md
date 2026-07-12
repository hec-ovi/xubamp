# PLEDIT research (multi-window + geometry + interactions) vs Webamp

## Multi-window + PL/EQ sprites

I have everything verified. Here is the report.

---

# Multi-window plan for xubamp (SCTK 0.19.2) + PL/EQ/shuffle/repeat sprites

## Part 1 — SCTK 0.19.2 facts I verified (docs.rs + v0.19.2 source)

- `XdgShell::create_window(surface: impl Into<Surface>, decorations: WindowDecorations, qh: &QueueHandle<State>) -> Window`. It takes `&self`, so **`XdgShell` must stay alive to make a second window**. Your `run()` currently drops `xdg_shell` after building the first window — you must move it into `App`.
- `Window` is `pub struct Window(Arc<WindowInner>)`, `Clone`, and `impl PartialEq { Arc::ptr_eq }`. So you can compare `&Window` by `==` directly for handler routing (identity, not value).
- **There is no `close()`/`hide()`/`unmap()` on `Window`.** Closing a toplevel = drop the last `Window` clone. `WindowInner`'s `Drop` calls `toplevel_decoration.destroy()` then `xdg_toplevel.destroy()`, and the contained `XdgShellSurface` destroys the `xdg_surface` (and its `wl_surface`) on its own drop. So `self.playlist = None;` fully tears the window down.
- Pointer, keyboard, and seat are **seat-level and shared** — there is no per-window pointer/keyboard. You route by surface. `PointerEvent.surface` and `KeyboardHandler::enter/leave` carry the `wl_surface`; `press_key`/`release_key` do **not**, so you must remember which surface has keyboard focus (the prompt's instinct is correct).
- `WlSurface` and `Window` both implement `PartialEq`; either works for routing. Surface comparison is what pointer/keyboard give you; `Window ==` is what `WindowHandler` gives you.
- `SlotPool` grows its shm file on demand when `create_buffer` needs more space, so a per-window pool sized `w*h*4` is fine even if the playlist later becomes resizable.

## Part 2 — Restructuring `crates/wl/src/lib.rs` (no big rewrite)

Extract the per-window Wayland resources into one struct and give it the `draw`/frame bookkeeping. Everything shared (seat, skin, callbacks, pointer, timer) stays flat on `App`.

```rust
struct WindowCtx {
    window: Window,
    pool: SlotPool,
    fb: Framebuffer,
    configured: bool,
    frame_pending: bool,
    armed_move: Option<(i32, i32, u32)>, // title-bar drag is per-window
}

impl WindowCtx {
    /// Old App::draw, made window-agnostic. `animating` is passed in
    /// (main computes it via App::animating(); playlist passes false).
    fn draw(&mut self, qh: &QueueHandle<App>, animating: bool) {
        let (w, h) = (self.fb.width, self.fb.height);
        let stride = w as i32 * 4;
        let (buffer, canvas) = self.pool
            .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
            .expect("create wl_shm buffer");
        for (dst, src) in canvas.chunks_exact_mut(4).zip(self.fb.rgba.chunks_exact(4)) {
            dst[0] = src[2]; dst[1] = src[1]; dst[2] = src[0]; dst[3] = src[3];
        }
        let surface = self.window.wl_surface().clone();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        if animating && !self.frame_pending {
            surface.frame(qh, surface.clone());
            self.frame_pending = true;
        }
        buffer.attach_to(&surface).expect("attach buffer");
        surface.commit();
    }
}
```

`App` changes (keep the shared fields you already have; the delta is):

```rust
struct App {
    // ... registry/output/seat state, shm, compositor unchanged ...
    xdg_shell: XdgShell,               // NEW: keep alive to spawn window #2
    skin: Skin,
    on_command, playback_source, sample_source, vis_samples, last_marquee,
    compositor, shm, pointer, seat, modifiers, playing, loop_handle, qh, exit, // as before

    // main window (was the flat window/pool/fb/state/configured/frame_pending/armed_move)
    main: WindowCtx,
    main_state: hit::UiState,

    // second window: None == closed. Drop == close.
    playlist: Option<WindowCtx>,
    playlist_state: pledit::UiState,   // survives close/reopen; the Window does not carry it

    // keyboard focus can't come from press_key, so remember the target from enter/leave
    #[cfg(feature = "keyboard")]
    kbd_focus: Option<Target>,
}

#[derive(Clone, Copy, PartialEq)]
enum Target { Main, Playlist }
```

Add a small router used by every surface-carrying handler:

```rust
impl App {
    fn target_of(&self, surface: &wl_surface::WlSurface) -> Option<Target> {
        if surface == self.main.window.wl_surface() {
            Some(Target::Main)
        } else if self.playlist.as_ref()
            .is_some_and(|p| surface == p.window.wl_surface()) {
            Some(Target::Playlist)
        } else { None }
    }
    fn ctx(&mut self, t: Target) -> &mut WindowCtx {
        match t { Target::Main => &mut self.main,
                  Target::Playlist => self.playlist.as_mut().unwrap() }
    }
}
```

`App::redraw`/`apply`/`animating`/`step_*` stay, but `redraw` becomes per-target: `compose_main_window` for `Main`, a new `pledit::compose` for `Playlist`, then `self.ctx(t).draw(&qh, animating_for(t))`.

## Part 3 — Event routing

**Pointer** (`pointer_frame`): replace the `if event.surface != *self.window.wl_surface() { continue }` guard with `let Some(t) = self.target_of(&event.surface) else { continue };`, then dispatch to the matching state + hit module. Main uses `hit::on_press/on_motion/on_release/on_leave` against `main_state`; playlist uses `pledit::…` against `playlist_state`. `armed_move` and `window.move_(seat, serial)` operate on `self.ctx(t)` / `self.ctx(t).window`. The implicit pointer grab keeps delivering motion to the surface where the press landed, so cross-window drags of a slider/scrollbar stay correct.

**Keyboard**: in `enter`, set `self.kbd_focus = self.target_of(surface)`; in `leave`, `if self.target_of(surface) == self.kbd_focus { self.kbd_focus = None }`. `press_key` (and the repeat closure) route with `match self.kbd_focus { Some(Target::Main) => …main shortcuts…, Some(Target::Playlist) => …pledit shortcuts…, None => {} }`. This is the crux: because `press_key` carries no surface, `kbd_focus` is the only source of truth.

**Frame callback** (`CompositorHandler::frame`): it already receives the surface (currently `_`). Route it: `if let Some(t) = self.target_of(surface) { self.on_frame(t) }`, and have `on_frame` clear `self.ctx(t).frame_pending`. In practice only `Main` ever requests frames (the visualizer), but routing keeps main's `frame_pending` from being cleared by an unrelated callback.

## Part 4 — `WindowHandler` for two windows

`configure` and `request_close` hand you `&Window`; compare with `==` (Arc identity):

```rust
fn configure(&mut self, _, qh, window: &Window, _cfg, _serial) {
    if *window == self.main.window {
        self.main.configured = true;
        self.main.draw(qh, self.animating());
    } else if let Some(p) = self.playlist.as_mut().filter(|p| *window == p.window) {
        p.configured = true;
        p.draw(qh, false);
    }
}
fn request_close(&mut self, _, _, window: &Window) {
    if *window == self.main.window {
        self.exit = true;                 // closing main quits the app
    } else {
        self.close_playlist();            // closing PL just hides it, main keeps running
    }
}
```

## Part 5 — Toggling the second window

**Destroy + recreate via `Option` is the clean SCTK way** — there is no `hide()`, and the playlist's real state (scroll offset, selection, entries) lives in `playlist_state`/the shared model, not in the `Window`, so recreating loses nothing. (The alternative, keeping the `Window` and unmapping by committing a null buffer, is more code: remapping forces you back through the `configure` handshake before you may attach again. Not worth it here.)

```rust
impl App {
    fn open_playlist(&mut self) {
        if self.playlist.is_some() { return; }
        let state = pledit::UiState::default(); // or reuse self.playlist_state
        let fb = pledit::compose(&self.skin, &self.playlist_state);
        let (w, h) = (fb.width, fb.height);
        let surface = self.compositor.create_surface(&self.qh);
        let window = self.xdg_shell.create_window(
            surface, WindowDecorations::RequestClient, &self.qh);
        window.set_title("xubamp – playlist");
        window.set_app_id("xubamp");
        window.set_parent(Some(&self.main.window)); // stacks with main; optional
        window.set_min_size(Some((w, h)));
        window.set_max_size(Some((w, h)));           // drop later to allow resize
        window.commit();                             // configured=false until configure()
        let pool = SlotPool::new(w as usize * h as usize * 4, &self.shm)
            .expect("playlist pool");
        self.playlist = Some(WindowCtx { window, pool, fb,
            configured: false, frame_pending: false, armed_move: None });
        self.main_state.pl_lit = true;               // light the PL button
        self.redraw_main();
    }
    fn close_playlist(&mut self) {
        self.playlist = None;                        // Drop -> xdg_toplevel.destroy()
        if self.kbd_focus == Some(Target::Playlist) { self.kbd_focus = None; }
        self.main_state.pl_lit = false;
        self.redraw_main();
    }
    fn toggle_playlist(&mut self) {
        if self.playlist.is_some() { self.close_playlist() } else { self.open_playlist() }
    }
}
```

Wire `toggle_playlist()` to (a) a click on the main-window PL button (add a `hit::Outcome` variant, e.g. `window: Some(TitleButton::…)` won't fit — add a new outcome field like `toggle: Option<Window::Playlist>` or a `Command`-adjacent enum), and (b) a hotkey. Winamp's key is **`Alt+E`** for EQ and the PL toggle historically has no default letter, but Webamp/most clones bind the on-window button; keep it button-driven for PL. Do the toggle **after** you finish reading the borrowed event in `pointer_frame` (compute the outcome, then mutate `self.playlist`) so you never drop the `WindowCtx` while holding a `&mut` into it.

## Part 6 — Timer + frame-callback loop with two windows

- The single calloop redraw `Timer` stays shared; its closure gets `&mut App` and can touch both windows. `App::tick()` keeps driving the **main** window's clock/marquee/visualizer exactly as now. Add: if `playlist_state` shows now-playing/time that changed this tick, `self.redraw_playlist()`.
- The **frame-callback visualizer loop stays main-only.** The playlist is static; redraw it on demand (interaction, track change, scroll), not off frame callbacks. So `WindowCtx::draw` requests a frame only when `animating` is true, which it never is for the playlist.
- Keep `frame_pending` per-`WindowCtx` and clear it in the routed `frame()` handler. `configured` is per-window too, so the timer/first-draw guard (`if !self.configured`) becomes `if !self.main.configured` for the vis path and per-ctx for playlist draws.

No changes needed to the `delegate_*!` macros or `EventLoop<App>` — one `App`, one loop, N windows.

---

## Part 7 — PL / EQ / shuffle / repeat sprites (verified against Webamp `skinSprites.ts` + `css/main-window.css`, cross-checked with the classic layout)

**All four buttons come from `SHUFREP.BMP`** (one sheet). Add it to `crates/skin/src/model.rs` as `shufrep: sheet("shufrep.bmp")`. Rows y=0..60 hold shuffle+repeat (4 states × 15px), rows y=61..84 hold EQ+PL (2 states × 12px).

Semantics: **base** = window/mode off and not pressed; **selected** = window open / mode enabled (the "lit" state — for PL/EQ this is the lower row, y=73); **depressed** = mouse held (the right-hand columns).

| Button | src rect (x, y, w, h) in shufrep.bmp | state | dest (x, y) on main window |
|---|---|---|---|
| **EQ** off | 0, 61, 23, 12 | closed, up | **219, 58** |
| EQ off pressed | 46, 61, 23, 12 | closed, held | 219, 58 |
| EQ lit | 0, 73, 23, 12 | open, up | 219, 58 |
| EQ lit pressed | 46, 73, 23, 12 | open, held | 219, 58 |
| **PL** off | 23, 61, 23, 12 | closed, up | **242, 58** |
| PL off pressed | 69, 61, 23, 12 | closed, held | 242, 58 |
| PL lit | 23, 73, 23, 12 | open, up | 242, 58 |
| PL lit pressed | 69, 73, 23, 12 | open, held | 242, 58 |
| **Shuffle** off | 28, 0, 47, 15 | off, up | **164, 89** |
| Shuffle off pressed | 28, 15, 47, 15 | off, held | 164, 89 |
| Shuffle on | 28, 30, 47, 15 | on, up | 164, 89 |
| Shuffle on pressed | 28, 45, 47, 15 | on, held | 164, 89 |
| **Repeat** off | 0, 0, 28, 15 | off, up | **210, 89** |
| Repeat off pressed | 0, 15, 28, 15 | off, held | 210, 89 |
| Repeat on | 0, 30, 28, 15 | on, up | 210, 89 |
| Repeat on pressed | 0, 45, 28, 15 | on, held | 210, 89 |

Your guessed dests (EQ x=219 / PL x=242, y=58) are exactly right; shuffle/repeat sit at y=89 (164 and 210). Constants in your `sprites.rs` style:

```rust
// --- SHUFREP.BMP: EQ + PL toggles (23x12) at y=58; shuffle/repeat (…x15) at y=89 ---
// EQ toggle, dest (219,58). Lit == equalizer window open.
pub const EQ_OFF:         Placement = Placement::new(Rect::new(0, 61, 23, 12), 219, 58);
pub const EQ_OFF_PRESSED: Placement = Placement::new(Rect::new(46, 61, 23, 12), 219, 58);
pub const EQ_ON:          Placement = Placement::new(Rect::new(0, 73, 23, 12), 219, 58);
pub const EQ_ON_PRESSED:  Placement = Placement::new(Rect::new(46, 73, 23, 12), 219, 58);
// PL toggle, dest (242,58). Lit == playlist window open.
pub const PL_OFF:         Placement = Placement::new(Rect::new(23, 61, 23, 12), 242, 58);
pub const PL_OFF_PRESSED: Placement = Placement::new(Rect::new(69, 61, 23, 12), 242, 58);
pub const PL_ON:          Placement = Placement::new(Rect::new(23, 73, 23, 12), 242, 58);
pub const PL_ON_PRESSED:  Placement = Placement::new(Rect::new(69, 73, 23, 12), 242, 58);
// Shuffle, dest (164,89). "on" == shuffle enabled.
pub const SHUFFLE_OFF:         Placement = Placement::new(Rect::new(28, 0, 47, 15), 164, 89);
pub const SHUFFLE_OFF_PRESSED: Placement = Placement::new(Rect::new(28, 15, 47, 15), 164, 89);
pub const SHUFFLE_ON:          Placement = Placement::new(Rect::new(28, 30, 47, 15), 164, 89);
pub const SHUFFLE_ON_PRESSED:  Placement = Placement::new(Rect::new(28, 45, 47, 15), 164, 89);
// Repeat, dest (210,89). "on" == repeat enabled.
pub const REPEAT_OFF:         Placement = Placement::new(Rect::new(0, 0, 28, 15), 210, 89);
pub const REPEAT_OFF_PRESSED: Placement = Placement::new(Rect::new(0, 15, 28, 15), 210, 89);
pub const REPEAT_ON:          Placement = Placement::new(Rect::new(0, 30, 28, 15), 210, 89);
pub const REPEAT_ON_PRESSED:  Placement = Placement::new(Rect::new(0, 45, 28, 15), 210, 89);
```

Note for this phase: only the **PL** button toggles the new window (its lit state binds to `self.playlist.is_some()`). EQ is drawn but inert until the EQ window phase; shuffle/repeat drive `Command`s into the engine's playlist mode, not window toggles.

Relevant files: `/home/hec/workspace/xubamp/crates/wl/src/lib.rs` (all restructuring), `/home/hec/workspace/xubamp/crates/skin/src/model.rs` (add `shufrep` sheet), `/home/hec/workspace/xubamp/crates/skin/src/sprites.rs` (add the constants above), and a new `crates/render/src/pledit.rs` + `pledit` hit module mirroring `hit.rs`/`compose_main_window`.

## PLEDIT geometry

Verified everything against the Webamp source. Here is the complete, pixel-exact reference.

---

# Winamp 2.x PLEDIT (playlist editor) — verified geometry

Sources (all `github.com/captbaritone/webamp`, `packages/webamp/`):
- `js/skinSprites.ts` — sprite rects
- `js/constants.ts` — `WINDOW_WIDTH`, `WINDOW_HEIGHT`, `TRACK_HEIGHT`, `CHARACTER_WIDTH`, `WINDOW_RESIZE_SEGMENT_*`
- `js/selectors.ts` — pixel-size + visible-track math
- `js/skinParserUtils.ts` (`getPlaylistStyle`) — pledit.txt color keys
- `css/playlist-window.css` + `js/components/PlaylistWindow/*.tsx` — layout
- Skinning spec: winampskins.neocities.org/config, wiki.winamp.com "Editing the Configuration Files"

## 1. pledit.bmp sprite map (source rects, x/y/w/h)

Two title-bar rows exist: **row y=0 = focused/active** window, **row y=21 = unfocused**. Everything is `_SELECTED` = the focused/active variant.

**Title bar (20px tall):**
| Sprite | x | y | w | h |
|---|---|---|---|---|
| PLAYLIST_TOP_LEFT_SELECTED (focused) | 0 | 0 | 25 | 20 |
| PLAYLIST_TITLE_BAR_SELECTED | 26 | 0 | 100 | 20 |
| PLAYLIST_TOP_TILE_SELECTED (fill) | 127 | 0 | 25 | 20 |
| PLAYLIST_TOP_RIGHT_CORNER_SELECTED | 153 | 0 | 25 | 20 |
| PLAYLIST_TOP_LEFT_CORNER (unfocused) | 0 | 21 | 25 | 20 |
| PLAYLIST_TITLE_BAR | 26 | 21 | 100 | 20 |
| PLAYLIST_TOP_TILE (fill) | 127 | 21 | 25 | 20 |
| PLAYLIST_TOP_RIGHT_CORNER | 153 | 21 | 25 | 20 |

**Side edges (repeat-y, 29px source tile):**
| Sprite | x | y | w | h |
|---|---|---|---|---|
| PLAYLIST_LEFT_TILE | 0 | 42 | 12 | 29 |
| PLAYLIST_RIGHT_TILE | 31 | 42 | 20 | 29 |

**Bottom bar (38px tall):**
| Sprite | x | y | w | h |
|---|---|---|---|---|
| PLAYLIST_BOTTOM_LEFT_CORNER | 0 | 72 | 125 | 38 |
| PLAYLIST_BOTTOM_RIGHT_CORNER | 126 | 72 | 150 | 38 |
| PLAYLIST_BOTTOM_TILE (fill) | 179 | 0 | 25 | 38 |
| PLAYLIST_VISUALIZER_BACKGROUND | 205 | 0 | 75 | 38 |

**Scrollbar thumb (the ONLY scrollbar sprite — no arrow-button sprite exists):**
| Sprite | x | y | w | h |
|---|---|---|---|---|
| PLAYLIST_SCROLL_HANDLE (normal) | 52 | 53 | 8 | 18 |
| PLAYLIST_SCROLL_HANDLE_SELECTED (pressed) | 61 | 53 | 8 | 18 |

**Title-bar buttons (9×9):** PLAYLIST_CLOSE_SELECTED 52,42; PLAYLIST_COLLAPSE_SELECTED (shade) 62,42; PLAYLIST_EXPAND_SELECTED 150,42.

**Windowshade (rolled-up) strip, 14px tall** — only if you implement shade mode: PLAYLIST_SHADE_BACKGROUND_LEFT 72,42,25,14; PLAYLIST_SHADE_BACKGROUND 72,57,25,14 (fill); PLAYLIST_SHADE_BACKGROUND_RIGHT 99,57,50,14; ..._RIGHT_SELECTED 99,42,50,14.

**Bottom-bar menu button glyphs (all 22×18, normal at left col, `_SELECTED`=pressed at +23px x):** ADD_URL 0,111 / ADD_DIR 0,130 / ADD_FILE 0,149; REMOVE_ALL 54,111 / CROP 54,130 / REMOVE_SELECTED 54,149 / REMOVE_MISC 54,168; INVERT_SELECTION 104,111 / SELECT_ZERO 104,130 / SELECT_ALL 104,149; SORT_LIST 154,111 / FILE_INFO 154,130 / MISC_OPTIONS 154,149; NEW_LIST 204,111 / SAVE_LIST 204,130 / LOAD_LIST 204,149. Menu divider bars (3px wide): ADD_MENU_BAR 48,111,3,54 / REMOVE_MENU_BAR 100,111,3,72 / SELECT_MENU_BAR 150,111,3,54 / MISC_MENU_BAR 200,111,3,54 / LIST_BAR 250,111,3,54.

## 2. Default/collapsed size + tile layout

Constants: `WINDOW_WIDTH=275`, `WINDOW_HEIGHT=116`, `WINDOW_RESIZE_SEGMENT_WIDTH=25`, `WINDOW_RESIZE_SEGMENT_HEIGHT=29`.

**Pixel size from size-units [w,h]** (`getWindowPixelSize`):
```
pixelW = 275 + w*25      pixelH = 116 + h*29
```
Default `[0,0]` = **275×116** (same width as the main window — confirmed). Width only grows in 25px steps, height in 29px steps.

Layout at 275×116 (window-relative coords):
- **Title bar** y 0..20: `[25 left corner][top-tile fill][100 title, centered][top-tile fill][25 right corner]`. Title is horizontally centered; the two fills use PLAYLIST_TOP_TILE repeated (odd remainder clipped).
- **Middle** y 20..78 (58px): left edge = PLAYLIST_LEFT_TILE, **12px** wide (x 0..12), repeat-y; right edge = PLAYLIST_RIGHT_TILE, **20px** wide (x 255..275), repeat-y.
- **Bottom bar** y 78..116 (38px): at default width the two corners meet exactly — left corner **125px** (x 0..125) + right corner **150px** (x 125..275) = 275, no fill needed. When wider, PLAYLIST_BOTTOM_TILE (25px) repeats between them; same for PLAYLIST_TOP_TILE up top.

## 3. Track-list content rect

- **x = 12** (right of left edge) to **x = width−20** (left of right edge). At 275 → x 12..255, **width 243px**.
- Tracks start at **y = 23** (20 title + 3px top pad; confirmed by Webamp's drop-target math `(top − 23)/TRACK_HEIGHT`). Bottom pad 3px.
- **Row height `TRACK_HEIGHT = 13`.**
- **Visible rows** (`getNumberOfVisibleTracks`): `floor((58 + 29*h) / 13)`. At default → `floor(58/13) = 4` rows. (58 = 116−20−38, the middle-band height.)
- Title text left-aligned starting x=12; duration column right-aligned with **3px right padding** against x=255.

Correction on your assumption: the track rows do **not** use the text.bmp bitmap font. Classic Winamp renders playlist rows with the real Windows font named by `Font=` in pledit.txt (Webamp draws it at 9px, 0.5px letter-spacing, in a 13px row). The ~6px text.bmp cell font is used only for the running-time readout (see §5), not the list.

## 4. Scrollbar

- Thumb = PLAYLIST_SCROLL_HANDLE, **8px wide × 18px tall** (`HANDLE_HEIGHT=18`); pressed = PLAYLIST_SCROLL_HANDLE_SELECTED.
- Lives in the 20px right-edge band, `marginLeft: 5` → thumb x ≈ **260** (255 + 5), spanning ~260..268.
- Vertical travel (`PlaylistScrollBar.tsx`): `sliderHeight = pixelH − 58` (= 58 at default, i.e. exactly the middle band from y=20 to y=78). `thumbTop = (scrollPos/100) * (sliderHeight − 18)`; travel range = 40px at default. Disabled/hidden when all tracks fit.
- There is **no arrow-button sprite** in classic skins. Webamp's `#playlist-scroll-up/down-button` are invisible 8×5px click zones at right:7 / top:2 and top:8; ignore for pixel-exact rendering.

Also: the **resize grip is bottom-RIGHT**, not bottom-left. `#playlist-resize-target` = right:0 bottom:0, 20×20; the grip graphic is baked into PLAYLIST_BOTTOM_RIGHT_CORNER.

## 5. Bottom-bar time display + mini-visualizer

- **Running-time display**: positioned top:10 / left:7 relative to the bottom-right 150px block (i.e. ≈ window (132, 88) at default). Rendered with the **text.bmp bitmap font** (`CHARACTER_WIDTH=5`, ~6px cell). This is the total playlist time.
- **Mini-visualizer**: PLAYLIST_VISUALIZER_BACKGROUND (75×38) exists; `#playlist-visualizer` is 75px wide, full bottom-bar height, positioned right:150 from the window. It is **only rendered when playlist width-units > 2** (i.e. width ≥ 350). At the default 275 width it is **not shown** — the bottom-right corner shows only the time. The mini-vis draws with the **viscolor.txt** palette (same as the main analyzer), NOT any pledit key.
- `.mini-time` (top:23/left:66) is the shade-mode mini time readout, unrelated to the normal bottom bar.

## 6. pledit.txt colors (`[Text]` section)

Webamp `getPlaylistStyle` normalizes keys (lowercased, forced `#`, truncated to 7 chars): **normal, current, normalbg, selectedbg, mbFG, mbBG**, plus **Font**.

| Key | Maps to |
|---|---|
| **Normal** | unselected track-row text color |
| **Current** | text color of the currently-playing track |
| **NormalBG** | list background fill (whole track area) |
| **SelectedBG** | background rectangle behind a selected row |
| **MbFG** | minibrowser status-bar text (NOT the playlist) |
| **MbBG** | minibrowser status-bar background |
| **Font** | font face for playlist rows (and minibrowser) |

There are **no mini-vis color keys in pledit.txt** — the playlist mini-visualizer takes its colors from **viscolor.txt**. Base-skin example values: `Normal=#00FF00 Current=#FFFFFF NormalBG=#000000 SelectedBG=#0000C6 Font=Arial` (all skin-defined; a selected + current row shows Current text over SelectedBG).

Sources: [Webamp repo](https://github.com/captbaritone/webamp), [Winamp skin config tutorial](https://winampskins.neocities.org/config), [Editing the Configuration Files (Winamp wiki)](http://wiki.winamp.com/wiki/Editing_the_Configuration_Files).

## PLEDIT interactions

Research complete. Everything below is verified against the Webamp source at `packages/webamp/js` (repo `captbaritone/webamp`, `master`). All file paths cited are within that tree.

---

# Winamp 2.x PLEDIT — behavior spec (verified vs Webamp)

## Data model (how Webamp stores the list)
- `state.playlist.trackOrder: number[]` — ordered array of track **IDs**. `state.tracks[id]` holds each track's metadata (`metaData.artist/title`, `defaultName`, `url`, `duration`).
- `state.playlist.selectedTracks: number[]` — selected track **IDs** (not indices).
- `state.playlist.lastSelectedIndex: number | null` — anchor **index** into `trackOrder` for shift-range.
- `state.display.playlistScrollPosition: number` — scroll as a **percentage 0..100** (not a pixel/row offset). Everything else is derived.
- Constant: `TRACK_HEIGHT = 13` px per row; `WINDOW_RESIZE_SEGMENT_HEIGHT = 29` (`js/constants.ts`).

---

## 1. Selection
Sources: `js/components/PlaylistWindow/TrackCell.tsx`, `js/reducers/playlist.ts`, `js/actionCreators/playlist.ts`.

Selection happens on **`onMouseDown`** (not click), so a drag can begin from the freshly selected row. The handler branches:
- `e.shiftKey` → dispatch `SHIFT_CLICKED_TRACK {index}`. Reducer: `start=min(index,lastSelectedIndex)`, `end=max(...)`, `selectedTracks = trackOrder.slice(start, end+1)`. Anchor is `lastSelectedIndex`.
- `e.metaKey || e.ctrlKey` → `CTRL_CLICKED_TRACK {index}`. Reducer toggles that ID in/out of `selectedTracks`; sets `lastSelectedIndex = index` even when deselecting.
- plain click, **and only if the row is not already selected** → `CLICKED_TRACK {index}`. Reducer: `selectedTracks = [trackOrder[index]]`, `lastSelectedIndex = index`. (Plain-clicking a row that's already in a multi-selection does NOT collapse the selection — this lets you grab and drag the whole group.)
- In all three branches it then calls `handleMoveClick(e)` to arm drag-reorder.

Clearing: the `TrackList` container `onClick` dispatches `selectZero` (click empty area below rows → clear). Each `TrackCell`'s `onClick` calls `e.stopPropagation()` so clicking a row doesn't bubble up and clear.

Drawing: `TrackCell.tsx` sets `style.backgroundColor = selected ? skinPlaylistStyle.selectedbg : undefined` and adds CSS class `selected`. `selectedbg` = the `SelectedBG` color from the skin's `PLEDIT.TXT`.

## 2. Double-click to play
`TrackCell.tsx`: `onDoubleClick={() => playTrackNow(id)}`. `playTrackNow` is in `js/actionCreators/media.ts` and dispatches `PLAY_TRACK` with that `id` → the engine loads and plays it immediately. (Touch: a second tap within 250ms synthesizes the double-click.)

## 3. Scrolling
Sources: `js/selectors.ts`, `js/actionCreators/playlist.ts`, `js/components/PlaylistWindow/PlaylistScrollBar.tsx`.

Visible-window math (all derived from the 0..100 percentage):
```
numberOfVisibleTracks = floor((BASE_WINDOW_HEIGHT + 29 * playlistSizeY) / 13)   // 13 = TRACK_HEIGHT
overflow              = max(0, trackCount - numberOfVisibleTracks)
offset (top row idx)  = percentToIndex(scrollPosition/100, overflow+1)
                      = round((scrollPosition/100) * overflow)
visibleTrackIds       = trackOrder.slice(offset, offset + numberOfVisibleTracks)
```
`getVisibleTrackIds` returns exactly that slice; `TrackList` renders each with absolute index `offset + i`.

Three ways to move it, all writing `SET_PLAYLIST_SCROLL_POSITION` (a percentage):
- **Thumb drag** (`PlaylistScrollBar.tsx`): a `VerticalSlider` (handle 18px tall, track height `playlistHeight - 58`). `onChange(val 0..1)` → `setPlaylistScrollPosition(val * 100)`; stored value fed back as `position/100`. Disabled when `allTracksAreVisible` (overflow == 0).
- **Mouse wheel** (`scrollPlaylistByDelta`): `totalPixelHeight = trackOrder.length * 13`; `percentDelta = (e.deltaY / totalPixelHeight) * 100`; `position = clamp(pos + percentDelta, 0, 100)`. `stopPropagation` only when there's overflow. (Speed is proportional to full list length, so long lists scroll slower per notch.)
- **N-track nudge** (`scrollNTracks(n)`, used by `scrollUpFourTracks`/`scrollDownFourTracks`): `position = clamp((currentOffset + n)/overflow, 0, 1) * 100`.

## 4. Row text
Sources: `js/components/PlaylistWindow/TrackList.tsx`, `TrackTitle.tsx`, `js/reducers/tracks.ts`, `js/utils.ts`.

The row is two columns. Left `.playlist-track-titles`, right `.playlist-track-durations`.
- **Title** (`TrackTitle.tsx`): `` `${paddedTrackNumber}. ${getTrackDisplayName(id)}` ``. `paddedTrackNumber = paddedTrackNumForIndex(i)` is the 1-based number, space-padded so the dots line up. Display name priority (`tracks.ts` → `trackName`): `"Artist - Title"` → `Title` only → `defaultName` → filename parsed from the URL → `"???"`.
- **Duration** (right column): `getTimeStr(track.duration)` → `"M:SS"`, leading minute-zero truncated (`"3:07"`, `"12:34"`; `null` → `""`).
- **Current/playing indicator**: `current = (getCurrentTrackId === id)`. `TrackCell` sets `style.color = current ? skinPlaylistStyle.current : undefined` and CSS class `current`. There is **no arrow/marker glyph** — the playing row is simply recolored with the skin's `Current` color from `PLEDIT.TXT`.

## 5. Bottom button clusters
Each cluster is a `PlaylistMenu` that pops a small sub-menu (matching Winamp's fly-out buttons). Actions:
- **ADD** (`AddMenu.tsx`) — `add-url` → `addFilesFromUrl(nextIndex)`; `add-dir` → `addDirAtIndex(nextIndex)`; `add-file` → `addFilesAtIndex(nextIndex)`. `nextIndex` = current track count (append at end).
- **REM** (`RemoveMenu.tsx`) — `remove-misc` → `alert("Not supported in Webamp")`; `remove-all` → `removeAllTracks`; `crop` → `cropPlaylist` (keep only selected, drop rest); `remove-selected` → `removeSelectedTracks`.
- **SEL** (`SelectionMenu.tsx`) — `select-all` → `selectAll`; `select-zero` → `selectZero`; `invert-selection` → `invertSelection` (reducer: `trackOrder.filter(id => !selectedTracks.includes(id))`).
- **MISC** (`MiscMenu.tsx`) — `sort-list` → opens `SortContextMenu` (`Sort list by title` → `sortListByTitle`; `Reverse list` → `reverseList`; `Randomize list` → `randomizeList` (shuffle)); `file-info` → `alert("Not supported")`; `misc-options` → `MiscOptionsContextMenu`.
- **LIST** (`ListMenu.tsx`) — `new-list` → `removeAllTracks`; `save-list` → `saveFilesToList` (writes .m3u); `load-list` → `addFilesFromList` (loads .m3u).

## 6. Jump to File (J key)
**Webamp does not implement it.** There is no `jumpToFile`/`JumpToFile` symbol, no `Hotkey`/`Keyboard`/search component (the `js/components` root has `App.tsx` and no keyboard/search module), and no `J`-key handler in `packages/webamp/js`; deepwiki likewise has no record. So there is nothing to copy here — implement it from native Winamp behavior instead: `J` opens a Jump-to-File dialog listing every track, a text box does incremental **substring filtering** on the display name (case-insensitive), and **Enter** jumps to + plays the highlighted match (there's also a "Jump" vs just-select distinction in Winamp, but Enter = play). This is the one item with no Webamp reference.

---

# Prioritized implementation order

**ESSENTIAL (first playable PLEDIT — the "select / double-click-play / scroll / list contents" core):**
1. **Render list contents** — 13px rows; `visibleCount = floor(area/13)`; left `"N. Name"` + right `"M:SS"`; recolor the current/playing row with the skin `Current` color; draw `selectedbg` behind selected rows. (`TrackList`, `TrackTitle`, `tracks.ts`, `getTimeStr`)
2. **Single-click select** on mousedown → replace selection with that one row; click empty area → clear. (`CLICKED_TRACK` / `selectZero`)
3. **Double-click → play that track.** (`playTrackNow` → your engine's load+play)
4. **Scroll** — keep a 0..100 position; `offset = round(pos/100 * overflow)`, `visible = slice(offset, offset+visibleCount)`; wheel updates via `deltaY/(len*13)*100`; thumb drag maps 0..1→0..100; disable/hide thumb when `overflow==0`.

**Fast-follow (still core-ish, cheap once selection exists):**
5. **ctrl-click toggle** and **shift-click range** multi-select (`CTRL_CLICKED_TRACK`, `SHIFT_CLICKED_TRACK` with `lastSelectedIndex` anchor). Needed before REM/crop/SEL are useful.

**LATER (menus & file I/O):**
6. Button clusters ADD/REM/SEL/MISC — mostly thin dispatchers over the selection/track state you already have (`removeSelectedTracks`, `cropPlaylist`, `selectAll/zero/invert`, `sort/reverse/randomize`).
7. Drag-to-reorder rows (`handleMoveClick` + `TRACK_HEIGHT` displacement math).
8. LIST + `.m3u` save/load, ADD dir/url dialogs.

**LAST (no Webamp reference; build from Winamp spec):**
9. **Jump to File (J)** — search box, incremental substring filter, Enter = jump+play.

Key Webamp files to keep open while implementing: `js/components/PlaylistWindow/{TrackList,TrackCell,TrackTitle,PlaylistScrollBar}.tsx`, `js/reducers/playlist.ts`, `js/actionCreators/{playlist,media}.ts`, `js/selectors.ts`, `js/utils.ts`, `js/constants.ts`, plus the five `*Menu.tsx` for the button clusters.