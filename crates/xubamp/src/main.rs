//! xubamp binary entry point.
//!
//! Composes a skin's main window and shows it in a native Wayland window, and (when built with
//! the `audio` feature) plays a media-file argument through PipeWire alongside it. Arguments
//! are classified by extension: a `.wsz`/`.zip` path is a skin, an audio file is a track.
//! The skin is otherwise resolved in order: `$XUBAMP_SKIN`, a local `skins/` test skin if one
//! is checked out, then the built-in default (original, clean-room; `xubamp_skin::default_skin`).
//! Transport controls, a time display, and the rest land in later phases; see
//! `docs/ARCHITECTURE.md`.

use std::path::Path;

use xubamp_skin::bmp::Image;
use xubamp_skin::container::SkinArchive;
use xubamp_skin::{default_skin, Skin};

/// The playlist-to-engine player. Only with the audio feature (it owns the PipeWire-backed engine).
#[cfg(feature = "audio")]
mod player;

/// A local skin used only during development. It lives under `skins/`, which is gitignored
/// (third-party art, never committed or shipped), so a released binary never finds it and
/// falls through to the built-in default. This is the "use the XMMS skin for now" hook.
const DEV_SKIN: &str = "skins/XMMS_standard_skin.wsz";

/// Extensions we treat as playable media on the command line.
const AUDIO_EXTS: &[&str] = &["mp3", "wav", "flac", "m4a", "ogg", "oga", "aac"];

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
        } else if AUDIO_EXTS.iter().any(|e| has_ext(&arg, e)) {
            media.push(arg);
        }
    }
    (skin, media)
}

fn load_skin(path: &str) -> Skin {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("xubamp: cannot read {path}: {e}");
        std::process::exit(1);
    });
    let archive = SkinArchive::from_bytes(&bytes).unwrap_or_else(|e| {
        eprintln!("xubamp: {path} is not a readable skin archive: {e:?}");
        std::process::exit(1);
    });
    let skin = Skin::from_archive(&archive);
    let dim = |img: &Option<Image>| {
        img.as_ref()
            .map(|i| format!("{}x{}", i.width, i.height))
            .unwrap_or_else(|| "missing".into())
    };
    eprintln!(
        "xubamp: loaded {path}: {} members, main={} titlebar={} cbuttons={}",
        archive.len(),
        dim(&skin.main),
        dim(&skin.titlebar),
        dim(&skin.cbuttons),
    );
    skin
}

/// Resolve which skin to show, in priority order: CLI path, `$XUBAMP_SKIN`, a local dev skin
/// if checked out, else the built-in default.
fn resolve_skin(cli: Option<&str>) -> Skin {
    if let Some(path) = cli {
        return load_skin(path);
    }
    if let Ok(path) = std::env::var("XUBAMP_SKIN") {
        return load_skin(&path);
    }
    if Path::new(DEV_SKIN).exists() {
        eprintln!("xubamp: using local dev skin {DEV_SKIN}");
        return load_skin(DEV_SKIN);
    }
    eprintln!("xubamp: no skin given, using the built-in default skin");
    default_skin()
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

/// The engine operations a transport button maps to, given whether audio is currently `playing` and
/// whether the track has `finished`. Classic Winamp semantics for the Play button: a no-op while
/// already playing; a restart from the top once the track has finished; otherwise (paused, or
/// stopped which already rewound to 0) a plain resume in place. Pause toggles pause/resume; Stop
/// halts and rewinds. Prev/Next/Eject need a playlist, so they map to nothing yet. (The `x` hotkey's
/// unconditional restart is a separate `Command::Restart`, not routed through here.)
#[cfg(any(feature = "audio", test))]
fn transport_ops(t: xubamp_render::hit::Transport, playing: bool, finished: bool) -> Vec<EngineOp> {
    use xubamp_render::hit::Transport;
    use EngineOp::{SeekToStart, SetActive};
    match t {
        Transport::Play if playing => Vec::new(),
        Transport::Play if finished => vec![SeekToStart, SetActive(true)],
        Transport::Play => vec![SetActive(true)],
        Transport::Pause => vec![SetActive(!playing)],
        Transport::Stop => vec![SetActive(false), SeekToStart],
        Transport::Prev | Transport::Next | Transport::Eject => Vec::new(),
    }
}

fn main() {
    let (skin_arg, media_args) = classify(std::env::args().skip(1));
    let skin = resolve_skin(skin_arg.as_deref());

    // The marquee shows the first track's file name (tag-based titles arrive later); it updates per
    // track as the playlist advances.
    let title = media_args.first().map(|p| track_title(p)).unwrap_or_default();

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
        if let Some(v) = std::env::var("XUBAMP_VOLUME").ok().and_then(|s| s.parse().ok()) {
            state.volume = v;
        }
        if let Some(b) = std::env::var("XUBAMP_BALANCE").ok().and_then(|s| s.parse().ok()) {
            state.balance = b;
        }
        if let Some(p) = std::env::var("XUBAMP_POSITION").ok().and_then(|s| s.parse().ok()) {
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

        let tracks: Vec<std::path::PathBuf> =
            media_args.iter().map(std::path::PathBuf::from).collect();
        let player = Rc::new(RefCell::new(player::Player::new(tracks)));
        player.borrow_mut().start(); // begin the first track

        let on_command = {
            let player = Rc::clone(&player);
            move |command: xubamp_render::hit::Command| {
                use xubamp_render::hit::Command;
                let mut player = player.borrow_mut();
                match command {
                    Command::Transport(t) => player.transport(t),
                    Command::Volume(v) => player.set_volume(v),
                    Command::Balance(b) => player.set_balance(b),
                    Command::Seek(fraction) => player.seek_fraction(fraction),
                    Command::Restart => player.restart(),
                    Command::ToggleMode(mode) => player.toggle_mode(mode),
                }
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

        if let Err(e) =
            xubamp_wl::run(skin, title, on_command, playback_source, sample_source, playlist_source)
        {
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
        let on_command =
            |command: xubamp_render::hit::Command| eprintln!("xubamp: command {command:?}");
        let playback_source = xubamp_render::hit::Playback::default;
        let sample_source = |out: &mut [f32]| out.iter_mut().for_each(|s| *s = 0.0);
        let playlist_source = || (Vec::new(), None);
        if let Err(e) =
            xubamp_wl::run(skin, title, on_command, playback_source, sample_source, playlist_source)
        {
            eprintln!("xubamp: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, track_title, transport_ops, EngineOp};
    use xubamp_render::hit::Transport;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn play_button_resumes_when_paused_restarts_when_finished_and_is_a_noop_while_playing() {
        // Already playing: no-op.
        assert!(transport_ops(Transport::Play, true, false).is_empty(), "playing: Play does nothing");
        // Paused or stopped (not playing, not finished): resume in place. Stop already rewound to 0,
        // so this doubles as "play from the top" after a Stop, while Pause resumes where it paused.
        assert_eq!(
            transport_ops(Transport::Play, false, false),
            vec![EngineOp::SetActive(true)],
            "paused/stopped: Play resumes in place",
        );
        // Finished: restart from the top (resume in place would find nothing left to play).
        assert_eq!(
            transport_ops(Transport::Play, false, true),
            vec![EngineOp::SeekToStart, EngineOp::SetActive(true)],
            "finished: Play restarts from the top",
        );
    }

    #[test]
    fn pause_toggles_from_the_live_playing_state() {
        assert_eq!(
            transport_ops(Transport::Pause, true, false),
            vec![EngineOp::SetActive(false)],
            "playing -> pause",
        );
        assert_eq!(
            transport_ops(Transport::Pause, false, false),
            vec![EngineOp::SetActive(true)],
            "paused -> resume",
        );
    }

    #[test]
    fn stop_halts_and_rewinds() {
        // Order matters: deactivate first, then rewind, so the clock shows 00:00 stopped.
        assert_eq!(
            transport_ops(Transport::Stop, true, false),
            vec![EngineOp::SetActive(false), EngineOp::SeekToStart],
        );
    }

    #[test]
    fn skip_commands_map_to_nothing_until_a_playlist() {
        for t in [Transport::Prev, Transport::Next, Transport::Eject] {
            assert!(transport_ops(t, true, false).is_empty(), "{t:?} maps to nothing yet");
        }
    }

    #[test]
    fn track_title_is_the_file_stem() {
        assert_eq!(track_title("/music/Aphex Twin - Xtal.mp3"), "Aphex Twin - Xtal");
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
        assert_eq!(media, ["a.mp3", "b.wav"], "every media file is kept as a playlist, in order");
    }

    #[test]
    fn no_recognized_args_yields_none() {
        let (skin, media) = classify(s(&["readme.md"]));
        assert!(skin.is_none());
        assert!(media.is_empty());
    }
}
