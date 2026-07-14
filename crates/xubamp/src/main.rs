//! xubamp binary entry point.
//!
//! Composes a skin's main window and shows it in a native Wayland window, and (when built with
//! the `audio` feature) plays a media-file argument through PipeWire alongside it. Arguments
//! are classified by extension: a `.wsz`/`.zip` path is a skin, an audio file is a track.
//! The skin is otherwise resolved in order: `$XUBAMP_SKIN`, the saved skin, a local `skins/` test
//! skin if one is checked out, then the built-in default (original, clean-room;
//! `xubamp_skin::default_skin`).
//! Transport controls, a time display, and the rest land in later phases; see
//! `docs/ARCHITECTURE.md`.

use std::path::{Path, PathBuf};

use xubamp_skin::bmp::Image;
use xubamp_skin::container::SkinArchive;
use xubamp_skin::{default_skin, Skin};

/// The playlist-to-engine player. Only with the audio feature (it owns the PipeWire-backed engine).
#[cfg(feature = "audio")]
mod player;
mod portal_actions;

/// A local skin used only during development. It lives under `skins/`, which is gitignored
/// (third-party art, never committed or shipped), so a released binary never finds it and
/// falls through to the built-in default. This is the "use the XMMS skin for now" hook.
const DEV_SKIN: &str = "skins/XMMS_standard_skin.wsz";

fn has_ext(name: &str, ext: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The marquee title for a media path: its file name without the extension. This is Winamp's
/// fallback when there are no tags to read (tag-based titles come with the playlist). A path
/// with no file name yields an empty title, which draws no marquee. A leading-dot name (e.g.
/// `.mp3`) is extension-less to Rust, so the whole name is the stem.
fn track_title(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Split CLI arguments into an optional skin path and the media playlist, by extension. The first
/// skin wins; every media file is kept, in order, so `xubamp a.mp3 b.mp3` (or a shell glob) builds a
/// playlist. Anything unrecognized is ignored. Pure and iterator-based, so it is unit-testable
/// without a real argv.
fn classify<I: IntoIterator<Item = String>>(args: I) -> (Option<String>, Vec<String>) {
    let mut skin = None;
    let mut media = Vec::new();
    for arg in args {
        if has_ext(&arg, "wsz") || has_ext(&arg, "zip") {
            skin.get_or_insert(arg);
        } else if xubamp_library::is_audio_path(Path::new(&arg)) {
            media.push(arg);
        }
    }
    (skin, media)
}

fn load_skin(path: &Path) -> Skin {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("xubamp: cannot read {}: {e}", path.display());
        std::process::exit(1);
    });
    let archive = SkinArchive::from_bytes(&bytes).unwrap_or_else(|e| {
        eprintln!(
            "xubamp: {} is not a readable skin archive: {e:?}",
            path.display()
        );
        std::process::exit(1);
    });
    let skin = Skin::from_archive(&archive);
    let dim = |img: &Option<Image>| {
        img.as_ref()
            .map(|i| format!("{}x{}", i.width, i.height))
            .unwrap_or_else(|| "missing".into())
    };
    eprintln!(
        "xubamp: loaded {}: {} members, main={} titlebar={} cbuttons={}",
        path.display(),
        archive.len(),
        dim(&skin.main),
        dim(&skin.titlebar),
        dim(&skin.cbuttons),
    );
    skin
}

/// Select an archive in priority order. A missing saved archive is deliberately skipped: stale
/// persisted state must not prevent the application from starting. Explicit CLI/environment paths
/// remain authoritative and report their load error instead of silently changing skins.
fn preferred_skin_path(
    cli: Option<&str>,
    environment: Option<std::ffi::OsString>,
    saved: Option<&Path>,
    saved_exists: bool,
    dev_exists: bool,
) -> Option<PathBuf> {
    cli.map(PathBuf::from)
        .or_else(|| environment.map(PathBuf::from))
        .or_else(|| saved.filter(|_| saved_exists).map(Path::to_path_buf))
        .or_else(|| dev_exists.then(|| PathBuf::from(DEV_SKIN)))
}

/// Resolve which skin to show, in priority order: CLI path, `$XUBAMP_SKIN`, saved skin, a local dev
/// skin if checked out, else the built-in default.
fn resolve_skin(cli: Option<&str>, saved: Option<&Path>) -> Skin {
    let saved_exists = saved.is_some_and(Path::exists);
    if let Some(path) = saved.filter(|_| !saved_exists) {
        eprintln!(
            "xubamp: saved skin {} no longer exists; falling back",
            path.display()
        );
    }
    let dev_exists = Path::new(DEV_SKIN).exists();
    let selected = preferred_skin_path(
        cli,
        std::env::var_os("XUBAMP_SKIN"),
        saved,
        saved_exists,
        dev_exists,
    );
    if let Some(path) = selected {
        if path == Path::new(DEV_SKIN) {
            eprintln!("xubamp: using local dev skin {DEV_SKIN}");
        }
        return load_skin(&path);
    }
    eprintln!("xubamp: no skin given, using the built-in default skin");
    default_skin()
}

/// Load the settings file selected by the XDG base-directory rules. A malformed field only emits a
/// warning and falls back independently; an unreadable file falls back to all defaults.
fn load_settings(path: Option<&Path>) -> xubamp_config::Settings {
    let Some(path) = path else {
        eprintln!("xubamp: HOME and XDG_CONFIG_HOME are unset; settings will not be persisted");
        return xubamp_config::Settings::default();
    };
    match xubamp_config::load(path) {
        Ok(report) => {
            for warning in report.warnings {
                let key = if warning.key.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", warning.key)
                };
                eprintln!(
                    "xubamp: settings warning at line {}{key}: {}",
                    warning.line, warning.message
                );
            }
            report.settings
        }
        Err(error) => {
            eprintln!(
                "xubamp: cannot load settings from {}: {error}; using defaults",
                path.display()
            );
            xubamp_config::Settings::default()
        }
    }
}

/// Synchronize the two mutable playback modes and atomically save them after a user toggle. Other
/// settings stay untouched. `None` is valid on systems without a resolvable config directory.
#[cfg(any(feature = "audio", test))]
fn persist_playback_modes(
    path: Option<&Path>,
    settings: &mut xubamp_config::Settings,
    shuffle: bool,
    repeat: bool,
) -> std::io::Result<()> {
    settings.playback.shuffle = shuffle;
    settings.playback.repeat = repeat;
    match path {
        Some(path) => xubamp_config::save(path, settings),
        None => Ok(()),
    }
}

/// Copy the final, user-visible window and equalizer state out of the Wayland event loop before the
/// settings file is saved. Runtime-only hover/drag state never crosses this boundary.
fn apply_ui_session(settings: &mut xubamp_config::Settings, session: xubamp_wl::SessionState) {
    let panes = session.panes;
    settings.windows.main.open = true;
    settings.windows.main.shaded = panes.main_shaded;
    settings.windows.equalizer.open = panes.equalizer_open;
    settings.windows.equalizer.shaded = session.equalizer_shaded;
    settings.windows.equalizer.x = panes.equalizer_position.0;
    settings.windows.equalizer.y = panes.equalizer_position.1;
    settings.windows.playlist.open = panes.playlist_open;
    settings.windows.playlist.shaded = panes.playlist_shaded;
    settings.windows.playlist.x = panes.playlist_position.0;
    settings.windows.playlist.y = panes.playlist_position.1;
    settings.windows.playlist.width = panes.playlist_size.0;
    settings.windows.playlist.height = panes.playlist_size.1;
    settings.equalizer.enabled = session.equalizer_enabled;
    settings.equalizer.preamp_db = session.equalizer_preamp_db;
    settings.equalizer.bands_db = session.equalizer_bands_db;
}

fn classic_equalizer_presets() -> Vec<xubamp_render::equalizer::Preset> {
    xubamp_dsp::presets::builtins()
        .into_iter()
        .map(|preset| {
            let settings = preset.settings(true);
            xubamp_render::equalizer::Preset {
                name: preset.name,
                preamp_db: settings.preamp_db,
                bands_db: settings.bands_db,
            }
        })
        .collect()
}

fn dispatch_portal_request(launcher: &portal_actions::Launcher, request: xubamp_wl::MenuRequest) {
    match launcher.launch(request.clone()) {
        portal_actions::LaunchResult::Started => {}
        portal_actions::LaunchResult::Busy => {
            eprintln!("xubamp: a file chooser is already open")
        }
        portal_actions::LaunchResult::Unsupported => {
            eprintln!("xubamp: menu action pending integration: {request:?}")
        }
    }
}

/// Eject always invokes the replace-and-play chooser. Play does so only for an empty playlist;
/// Pause stays inert there instead of becoming another chooser shortcut.
fn transport_opens_media(t: xubamp_render::hit::Transport, playlist_empty: bool) -> bool {
    matches!(t, xubamp_render::hit::Transport::Eject)
        || (playlist_empty && matches!(t, xubamp_render::hit::Transport::Play))
}

/// A primitive engine operation. Transport commands (Play/Pause/Stop) map to a short sequence of
/// these; keeping the mapping pure (independent of the live engine) lets the play/pause/stop policy
/// be unit-tested on the host without PipeWire. Only compiled with `audio` (where it is used) or
/// under `test` (where it is exercised).
#[cfg(any(feature = "audio", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineOp {
    /// Rewind the decoder to frame 0.
    SeekToStart,
    /// Activate (`true`) or deactivate (`false`) the output stream.
    SetActive(bool),
}

/// Playback state relevant to the classic transport buttons. `Paused` and `Stopped` are both
/// inactive at the audio backend, but Play must resume the former in place and restart the latter.
#[cfg(any(feature = "audio", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportState {
    Playing,
    Paused,
    Stopped,
    Finished,
}

/// The engine operations a transport button maps to. Play restarts unless playback is paused, in
/// which case it resumes in place. Pause toggles while playing/paused and acts like Play from a
/// stopped or naturally-finished state. Stop halts and rewinds. This mirrors Webamp's media entry
/// point: `play()` seeks to zero for every state except Paused, while the Pause action dispatches
/// Play whenever it is not currently playing.
#[cfg(any(feature = "audio", test))]
fn transport_ops(t: xubamp_render::hit::Transport, state: TransportState) -> Vec<EngineOp> {
    use xubamp_render::hit::Transport;
    use EngineOp::{SeekToStart, SetActive};
    match t {
        Transport::Play if state == TransportState::Paused => vec![SetActive(true)],
        Transport::Play => vec![SeekToStart, SetActive(true)],
        Transport::Pause if state == TransportState::Playing => vec![SetActive(false)],
        Transport::Pause if state == TransportState::Paused => vec![SetActive(true)],
        Transport::Pause => vec![SeekToStart, SetActive(true)],
        Transport::Stop => vec![SetActive(false), SeekToStart],
        Transport::Prev | Transport::Next | Transport::Eject => Vec::new(),
    }
}

fn main() {
    let (skin_arg, media_args) = classify(std::env::args().skip(1));
    let settings_path = xubamp_config::default_path();
    let settings = load_settings(settings_path.as_deref());
    let skin = resolve_skin(skin_arg.as_deref(), settings.skin_path.as_deref());

    // The marquee shows the first track's file name (tag-based titles arrive later); it updates per
    // track as the playlist advances.
    let title = media_args
        .first()
        .map(|p| track_title(p))
        .unwrap_or_default();
    let equalizer_state = xubamp_render::equalizer::EqState {
        enabled: settings.equalizer.enabled,
        preamp_db: settings.equalizer.preamp_db,
        bands_db: settings.equalizer.bands_db,
        shade: settings.windows.equalizer.shaded,
        ..Default::default()
    };
    let pane_layout = xubamp_wl::PaneLayout {
        main_shaded: settings.windows.main.shaded,
        equalizer_open: settings.windows.equalizer.open,
        equalizer_position: (settings.windows.equalizer.x, settings.windows.equalizer.y),
        playlist_open: settings.windows.playlist.open,
        playlist_shaded: settings.windows.playlist.shaded,
        playlist_position: (settings.windows.playlist.x, settings.windows.playlist.y),
        playlist_size: (
            settings.windows.playlist.width,
            settings.windows.playlist.height,
        ),
    };
    let equalizer_presets = classic_equalizer_presets();

    // Debug affordance / seed for the later headless render-diff harness: dump the raw RGBA the
    // window would display, then exit without opening a window. `XUBAMP_TITLE` overrides the
    // marquee text, `XUBAMP_VOLUME` (0-100) / `XUBAMP_BALANCE` (-100..100) the slider positions,
    // and `XUBAMP_POSITION` (0.0-1.0) the seek-bar thumb, so those strips can be diffed without a
    // real media file or live input.
    if let Ok(path) = std::env::var("XUBAMP_DUMP_RGBA") {
        let mut state = xubamp_render::hit::UiState {
            title: std::env::var("XUBAMP_TITLE").unwrap_or_else(|_| title.clone()),
            ..Default::default()
        };
        if let Some(v) = std::env::var("XUBAMP_VOLUME")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            state.volume = v;
        }
        if let Some(b) = std::env::var("XUBAMP_BALANCE")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            state.balance = b;
        }
        if let Some(p) = std::env::var("XUBAMP_POSITION")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            state.position = Some(p);
        }
        let fb = xubamp_render::compose_main_window(&skin, &state);
        std::fs::write(&path, &fb.rgba).expect("write rgba dump");
        println!("dumped {}x{} rgba to {path}", fb.width, fb.height);
        return;
    }

    // Play the media playlist (with the audio feature). The Player owns the current AudioEngine and
    // switches tracks on prev/next and auto-advance; it lives on this (the UI) thread and is shared
    // with the window's callbacks through Rc<RefCell>, which the calloop event loop borrows one at a
    // time (all on this thread, so no locking).
    #[cfg(feature = "audio")]
    {
        use std::cell::RefCell;
        use std::rc::Rc;

        let (portal_launcher, mut portal_receiver) =
            portal_actions::bridge(settings.library.recurse);
        let tracks: Vec<std::path::PathBuf> =
            media_args.iter().map(std::path::PathBuf::from).collect();
        let equalizer = xubamp_audio::EqSettings {
            enabled: settings.equalizer.enabled,
            preamp_db: settings.equalizer.preamp_db,
            bands_db: settings.equalizer.bands_db,
        };
        let player = Rc::new(RefCell::new(player::Player::with_settings(
            tracks,
            settings.playback.shuffle,
            settings.playback.repeat,
            equalizer,
        )));
        let settings = Rc::new(RefCell::new(settings));
        player.borrow_mut().start(); // begin the first track

        let on_command = {
            let player = Rc::clone(&player);
            let settings = Rc::clone(&settings);
            let settings_path = settings_path.clone();
            let portal_launcher = portal_launcher.clone();
            move |command: xubamp_render::hit::Command| {
                use xubamp_render::hit::Command;
                let mut player = player.borrow_mut();
                match command {
                    Command::Transport(t) => {
                        let opens_media = transport_opens_media(t, player.is_empty());
                        if opens_media {
                            drop(player);
                            dispatch_portal_request(
                                &portal_launcher,
                                xubamp_wl::MenuRequest::OpenMedia,
                            );
                        } else {
                            player.transport(t);
                        }
                    }
                    Command::Volume(v) => player.set_volume(v),
                    Command::Balance(b) => player.set_balance(b),
                    Command::Seek(fraction) => player.seek_fraction(fraction),
                    Command::Restart => player.restart(),
                    Command::ToggleMode(mode) => {
                        player.toggle_mode(mode);
                        let result = persist_playback_modes(
                            settings_path.as_deref(),
                            &mut settings.borrow_mut(),
                            player.shuffle(),
                            player.repeat(),
                        );
                        if let Err(error) = result {
                            eprintln!("xubamp: cannot save playback settings: {error}");
                        }
                    }
                    Command::PlayIndex(i) => player.play_index(i),
                }
            }
        };
        let on_equalizer = {
            let player = Rc::clone(&player);
            let settings = Rc::clone(&settings);
            move |command: xubamp_render::equalizer::Command| {
                use xubamp_render::equalizer::Command;
                let mut player = player.borrow_mut();
                match command {
                    Command::Volume(volume) => player.set_volume(volume),
                    Command::Balance(balance) => player.set_balance(balance),
                    Command::Enabled(enabled) => {
                        let mut equalizer = player.equalizer_settings();
                        equalizer.enabled = enabled;
                        player.set_equalizer_settings(equalizer);
                        settings.borrow_mut().equalizer.enabled = enabled;
                    }
                    Command::Preamp(preamp_db) => {
                        let mut equalizer = player.equalizer_settings();
                        equalizer.preamp_db = preamp_db;
                        player.set_equalizer_settings(equalizer);
                        settings.borrow_mut().equalizer.preamp_db = preamp_db;
                    }
                    Command::Band { index, db } => {
                        let mut equalizer = player.equalizer_settings();
                        if let Some(band) = equalizer.bands_db.get_mut(index) {
                            *band = db;
                            player.set_equalizer_settings(equalizer);
                            settings.borrow_mut().equalizer.bands_db[index] = db;
                        }
                    }
                    Command::Preset {
                        preamp_db,
                        bands_db,
                    } => {
                        let equalizer = xubamp_audio::EqSettings {
                            enabled: player.equalizer_settings().enabled,
                            preamp_db,
                            bands_db,
                        };
                        player.set_equalizer_settings(equalizer);
                        let mut settings = settings.borrow_mut();
                        settings.equalizer.preamp_db = preamp_db;
                        settings.equalizer.bands_db = bands_db;
                    }
                }
            }
        };
        let on_menu = {
            let portal_launcher = portal_launcher.clone();
            move |request: xubamp_wl::MenuRequest| {
                dispatch_portal_request(&portal_launcher, request);
            }
        };
        let playback_source = {
            let player = Rc::clone(&player);
            move || {
                let mut player = player.borrow_mut();
                player.poll(); // auto-advance to the next track when the current one ends
                player.playback()
            }
        };
        let sample_source = {
            let player = Rc::clone(&player);
            move |out: &mut [f32]| player.borrow().read_scope(out)
        };
        let playlist_source = {
            let player = Rc::clone(&player);
            move || player.borrow().playlist_view()
        };
        let external_source = {
            let player = Rc::clone(&player);
            move || {
                let (completions, pending) = portal_receiver.poll();
                let mut events = Vec::new();
                for completion in completions {
                    match completion {
                        portal_actions::Completion::AddPaths { paths, warnings } => {
                            for warning in warnings {
                                eprintln!("xubamp: directory scan warning: {warning}");
                            }
                            let count = player.borrow_mut().append_paths(paths).len();
                            eprintln!("xubamp: added {count} audio file(s) to the playlist");
                        }
                        portal_actions::Completion::OpenPaths(paths) => {
                            let mut player = player.borrow_mut();
                            let count = player.replace_paths(paths);
                            if count > 0 {
                                player.start();
                                eprintln!("xubamp: opened {count} audio file(s)");
                            }
                        }
                        portal_actions::Completion::EqualizerPreset(preset) => {
                            events.push(xubamp_wl::ExternalEvent::EqualizerPreset(preset))
                        }
                        portal_actions::Completion::Saved(path) => {
                            eprintln!("xubamp: saved equalizer preset to {}", path.display());
                        }
                        portal_actions::Completion::Error(error) => {
                            eprintln!("xubamp: {error}");
                        }
                    }
                }
                xubamp_wl::ExternalPoll { events, pending }
            }
        };

        let result = xubamp_wl::run(
            skin,
            title,
            equalizer_state,
            pane_layout,
            xubamp_wl::Runtime::new(
                on_command,
                on_equalizer,
                on_menu,
                equalizer_presets,
                playback_source,
                sample_source,
                playlist_source,
            )
            .with_external_source(external_source),
        );
        if let Ok(session) = result.as_ref() {
            apply_ui_session(&mut settings.borrow_mut(), *session);
        }
        // Equalizer sliders update live audio continuously, but the final state and pane layout
        // reach disk once when the event loop exits rather than once per pointer-motion event.
        if let Some(path) = settings_path.as_deref() {
            if let Err(error) = xubamp_config::save(path, &settings.borrow()) {
                eprintln!("xubamp: cannot save settings on exit: {error}");
            }
        }
        if let Err(e) = result {
            eprintln!("xubamp: {e}");
            std::process::exit(1);
        }
    }

    // Without the audio feature: no playback, commands are logged, the clock stays blank.
    #[cfg(not(feature = "audio"))]
    {
        if !media_args.is_empty() {
            eprintln!("xubamp: built without audio; rebuild with `--features audio` to play files");
        }
        let (portal_launcher, mut portal_receiver) =
            portal_actions::bridge(settings.library.recurse);
        let on_command = {
            let portal_launcher = portal_launcher.clone();
            move |command: xubamp_render::hit::Command| {
                use xubamp_render::hit::Command;
                if matches!(command, Command::Transport(t) if transport_opens_media(t, true)) {
                    dispatch_portal_request(&portal_launcher, xubamp_wl::MenuRequest::OpenMedia);
                } else {
                    eprintln!("xubamp: command {command:?}");
                }
            }
        };
        let on_equalizer = |command: xubamp_render::equalizer::Command| {
            eprintln!("xubamp: equalizer command {command:?}")
        };
        let on_menu = move |request: xubamp_wl::MenuRequest| {
            dispatch_portal_request(&portal_launcher, request);
        };
        let playback_source = xubamp_render::hit::Playback::default;
        let sample_source = |out: &mut [f32]| out.iter_mut().for_each(|s| *s = 0.0);
        let playlist_source = || (Vec::new(), None);
        let external_source = move || {
            let (completions, pending) = portal_receiver.poll();
            let mut events = Vec::new();
            for completion in completions {
                match completion {
                    portal_actions::Completion::EqualizerPreset(preset) => {
                        events.push(xubamp_wl::ExternalEvent::EqualizerPreset(preset));
                    }
                    portal_actions::Completion::AddPaths { paths, warnings } => {
                        for warning in warnings {
                            eprintln!("xubamp: directory scan warning: {warning}");
                        }
                        eprintln!(
                            "xubamp: built without audio; {} selected file(s) were not added",
                            paths.len()
                        );
                    }
                    portal_actions::Completion::OpenPaths(paths) => {
                        eprintln!(
                            "xubamp: built without audio; {} opened file(s) were not played",
                            paths.len()
                        );
                    }
                    portal_actions::Completion::Saved(path) => {
                        eprintln!("xubamp: saved equalizer preset to {}", path.display());
                    }
                    portal_actions::Completion::Error(error) => eprintln!("xubamp: {error}"),
                }
            }
            xubamp_wl::ExternalPoll { events, pending }
        };
        match xubamp_wl::run(
            skin,
            title,
            equalizer_state,
            pane_layout,
            xubamp_wl::Runtime::new(
                on_command,
                on_equalizer,
                on_menu,
                equalizer_presets,
                playback_source,
                sample_source,
                playlist_source,
            )
            .with_external_source(external_source),
        ) {
            Ok(session) => {
                let mut settings = settings;
                apply_ui_session(&mut settings, session);
                if let Some(path) = settings_path.as_deref() {
                    if let Err(error) = xubamp_config::save(path, &settings) {
                        eprintln!("xubamp: cannot save settings on exit: {error}");
                    }
                }
            }
            Err(error) => {
                eprintln!("xubamp: {error}");
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_ui_session, classic_equalizer_presets, classify, persist_playback_modes,
        preferred_skin_path, track_title, transport_opens_media, transport_ops, EngineOp,
        TransportState, DEV_SKIN,
    };
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use xubamp_render::hit::Transport;

    #[test]
    fn only_eject_and_empty_play_open_the_media_picker() {
        assert!(transport_opens_media(Transport::Eject, false));
        assert!(transport_opens_media(Transport::Eject, true));
        assert!(transport_opens_media(Transport::Play, true));
        assert!(!transport_opens_media(Transport::Play, false));
        assert!(!transport_opens_media(Transport::Pause, true));
        assert!(!transport_opens_media(Transport::Stop, true));
        assert!(!transport_opens_media(Transport::Next, true));
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn temp_settings_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("xubamp-main-test-{}-{nonce}", std::process::id()))
            .join("settings.conf")
    }

    #[test]
    fn skin_priority_is_cli_then_environment_then_saved_then_development() {
        let saved = Path::new("/saved/theme.wsz");
        let env = Some(OsString::from("/environment/theme.wsz"));
        assert_eq!(
            preferred_skin_path(Some("/cli/theme.wsz"), env.clone(), Some(saved), true, true),
            Some(PathBuf::from("/cli/theme.wsz"))
        );
        assert_eq!(
            preferred_skin_path(None, env, Some(saved), true, true),
            Some(PathBuf::from("/environment/theme.wsz"))
        );
        assert_eq!(
            preferred_skin_path(None, None, Some(saved), true, true),
            Some(saved.to_path_buf())
        );
        assert_eq!(
            preferred_skin_path(None, None, Some(saved), false, true),
            Some(PathBuf::from(DEV_SKIN)),
            "a deleted saved skin falls through to the development/base choices"
        );
        assert_eq!(preferred_skin_path(None, None, None, false, false), None);
    }

    #[test]
    fn classic_equalizer_menu_uses_the_canonical_dsp_presets() {
        let presets = classic_equalizer_presets();
        assert_eq!(presets.len(), 17);
        assert_eq!(presets.first().unwrap().name, "Classical");
        assert_eq!(presets.last().unwrap().name, "Techno");
        assert!(presets.iter().all(|preset| {
            preset.preamp_db.is_finite()
                && preset.bands_db.iter().all(|db| (-12.0..=12.0).contains(db))
        }));
    }

    #[test]
    fn mode_persistence_changes_only_modes_and_round_trips_atomically() {
        let path = temp_settings_path();
        let mut settings = xubamp_config::Settings::default();
        settings.equalizer.preamp_db = 3.5;
        settings.skin_path = Some(PathBuf::from("/skins/kept.wsz"));

        persist_playback_modes(Some(&path), &mut settings, true, true).unwrap();

        let loaded = xubamp_config::load(&path).unwrap().settings;
        assert!(loaded.playback.shuffle);
        assert!(loaded.playback.repeat);
        assert_eq!(loaded.equalizer.preamp_db, 3.5);
        assert_eq!(loaded.skin_path, Some(PathBuf::from("/skins/kept.wsz")));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn mode_persistence_without_a_config_directory_still_updates_memory() {
        let mut settings = xubamp_config::Settings::default();
        persist_playback_modes(None, &mut settings, true, false).unwrap();
        assert!(settings.playback.shuffle);
        assert!(!settings.playback.repeat);
    }

    #[test]
    fn final_ui_session_updates_panes_and_equalizer_without_losing_other_settings() {
        let mut settings = xubamp_config::Settings::default();
        settings.playback.shuffle = true;
        settings.skin_path = Some(PathBuf::from("/skins/kept.wsz"));
        let session = xubamp_wl::SessionState {
            panes: xubamp_wl::PaneLayout {
                main_shaded: true,
                equalizer_open: true,
                equalizer_position: (-12, 14),
                playlist_open: true,
                playlist_shaded: true,
                playlist_position: (263, 14),
                playlist_size: (550, 232),
            },
            equalizer_enabled: false,
            equalizer_shaded: true,
            equalizer_preamp_db: 3.5,
            equalizer_bands_db: [-12.0, -9.0, -6.0, -3.0, 0.0, 3.0, 6.0, 9.0, 12.0, 1.5],
        };

        apply_ui_session(&mut settings, session);

        assert!(settings.windows.main.shaded);
        assert!(settings.windows.equalizer.open);
        assert!(settings.windows.equalizer.shaded);
        assert_eq!(
            (settings.windows.equalizer.x, settings.windows.equalizer.y),
            (-12, 14)
        );
        assert!(settings.windows.playlist.open);
        assert!(settings.windows.playlist.shaded);
        assert_eq!(
            (
                settings.windows.playlist.x,
                settings.windows.playlist.y,
                settings.windows.playlist.width,
                settings.windows.playlist.height,
            ),
            (263, 14, 550, 232)
        );
        assert!(!settings.equalizer.enabled);
        assert_eq!(settings.equalizer.preamp_db, 3.5);
        assert_eq!(settings.equalizer.bands_db, session.equalizer_bands_db);
        assert!(
            settings.playback.shuffle,
            "unrelated playback state is kept"
        );
        assert_eq!(settings.skin_path, Some(PathBuf::from("/skins/kept.wsz")));
    }

    #[test]
    fn play_resumes_only_when_paused_and_otherwise_restarts() {
        // Already playing: restart from the beginning, like the X hotkey.
        assert_eq!(
            transport_ops(Transport::Play, TransportState::Playing),
            vec![EngineOp::SeekToStart, EngineOp::SetActive(true)],
            "playing: Play restarts",
        );
        // Paused: resume in place.
        assert_eq!(
            transport_ops(Transport::Play, TransportState::Paused),
            vec![EngineOp::SetActive(true)],
            "paused: Play resumes in place",
        );
        for state in [TransportState::Stopped, TransportState::Finished] {
            assert_eq!(
                transport_ops(Transport::Play, state),
                vec![EngineOp::SeekToStart, EngineOp::SetActive(true)],
                "{state:?}: Play starts from the top",
            );
        }
    }

    #[test]
    fn pause_toggles_from_the_live_playing_state() {
        assert_eq!(
            transport_ops(Transport::Pause, TransportState::Playing),
            vec![EngineOp::SetActive(false)],
            "playing -> pause",
        );
        assert_eq!(
            transport_ops(Transport::Pause, TransportState::Paused),
            vec![EngineOp::SetActive(true)],
            "paused -> resume",
        );
        for state in [TransportState::Stopped, TransportState::Finished] {
            assert_eq!(
                transport_ops(Transport::Pause, state),
                vec![EngineOp::SeekToStart, EngineOp::SetActive(true)],
                "{state:?}: Pause acts like Play",
            );
        }
    }

    #[test]
    fn stop_halts_and_rewinds() {
        // Order matters: deactivate first, then rewind, so the clock shows 00:00 stopped.
        assert_eq!(
            transport_ops(Transport::Stop, TransportState::Playing),
            vec![EngineOp::SetActive(false), EngineOp::SeekToStart],
        );
    }

    #[test]
    fn skip_commands_map_to_nothing_until_a_playlist() {
        for t in [Transport::Prev, Transport::Next, Transport::Eject] {
            assert!(
                transport_ops(t, TransportState::Playing).is_empty(),
                "{t:?} maps to nothing yet"
            );
        }
    }

    #[test]
    fn track_title_is_the_file_stem() {
        assert_eq!(
            track_title("/music/Aphex Twin - Xtal.mp3"),
            "Aphex Twin - Xtal"
        );
        assert_eq!(track_title("song.flac"), "song");
        assert_eq!(track_title("no_extension"), "no_extension");
        assert_eq!(track_title(""), "");
        // A leading-dot name is extension-less to Rust, so the whole name is the stem.
        assert_eq!(track_title(".mp3"), ".mp3");
    }

    #[test]
    fn classifies_skin_and_media_by_extension() {
        let (skin, media) = classify(s(&["My Skin.wsz", "song.mp3"]));
        assert_eq!(skin.as_deref(), Some("My Skin.wsz"));
        assert_eq!(media, ["song.mp3"]);
    }

    #[test]
    fn order_independent_and_case_insensitive() {
        let (skin, media) = classify(s(&["track.MP3", "Base.WSZ"]));
        assert_eq!(skin.as_deref(), Some("Base.WSZ"));
        assert_eq!(media, ["track.MP3"]);
    }

    #[test]
    fn first_skin_wins_all_media_kept_in_order_unknown_ignored() {
        let (skin, media) = classify(s(&["notes.txt", "a.mp3", "b.wav", "one.wsz", "two.wsz"]));
        assert_eq!(skin.as_deref(), Some("one.wsz"), "the first skin wins");
        assert_eq!(
            media,
            ["a.mp3", "b.wav"],
            "every media file is kept as a playlist, in order"
        );
    }

    #[test]
    fn no_recognized_args_yields_none() {
        let (skin, media) = classify(s(&[
            "readme.md",
            "not-enabled.flac",
            "movie.mp4",
            "playlist.m3u",
        ]));
        assert!(skin.is_none());
        assert!(media.is_empty());
    }
}
