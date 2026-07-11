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

/// Split CLI arguments into an optional skin path and an optional media path, by extension.
/// The first of each kind wins; anything unrecognized is ignored. Kept pure and iterator-based
/// so it is unit-testable without a real argv.
fn classify<I: IntoIterator<Item = String>>(args: I) -> (Option<String>, Option<String>) {
    let mut skin = None;
    let mut media = None;
    for arg in args {
        if has_ext(&arg, "wsz") || has_ext(&arg, "zip") {
            skin.get_or_insert(arg);
        } else if AUDIO_EXTS.iter().any(|e| has_ext(&arg, e)) {
            media.get_or_insert(arg);
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

fn main() {
    let (skin_arg, media_arg) = classify(std::env::args().skip(1));
    let skin = resolve_skin(skin_arg.as_deref());

    // The marquee shows the track's file name (tag-based titles arrive with the playlist).
    let title = media_arg.as_deref().map(track_title).unwrap_or_default();

    // Debug affordance / seed for the later headless render-diff harness: dump the raw RGBA the
    // window would display, then exit without opening a window. `XUBAMP_TITLE` overrides the
    // marquee text so the title strip can be diffed without a real media file.
    if let Ok(path) = std::env::var("XUBAMP_DUMP_RGBA") {
        let state = xubamp_render::hit::UiState {
            title: std::env::var("XUBAMP_TITLE").unwrap_or_else(|_| title.clone()),
            ..Default::default()
        };
        let fb = xubamp_render::compose_main_window(&skin, &state);
        std::fs::write(&path, &fb.rgba).expect("write rgba dump");
        println!("dumped {}x{} rgba to {path}", fb.width, fb.height);
        return;
    }

    // Start playback if built with audio and given a track. The engine runs on its own threads,
    // so the window loop below is unaffected; keeping `_engine` in scope until after `run`
    // returns means dropping it (window closed) stops playback and joins its threads cleanly.
    #[cfg(feature = "audio")]
    let _engine = media_arg.as_deref().and_then(|path| {
        match xubamp_audio::engine::AudioEngine::play(Path::new(path)) {
            Ok(engine) => {
                eprintln!("xubamp: playing {path}");
                Some(engine)
            }
            Err(e) => {
                eprintln!("xubamp: cannot play {path}: {e}");
                None
            }
        }
    });
    #[cfg(not(feature = "audio"))]
    if media_arg.is_some() {
        eprintln!("xubamp: built without audio; rebuild with `--features audio` to play files");
    }

    // Bridge transport commands from the window to the engine. Play resumes, Pause and Stop
    // halt (Stop reset-to-start needs decoder seeking, which comes with the seek bar). Prev,
    // Next and Eject wait for a playlist. Without the audio feature, commands are just logged.
    #[cfg(feature = "audio")]
    let on_command = {
        let handle = _engine.as_ref().map(|engine| engine.handle());
        move |command: xubamp_render::hit::Transport| {
            use xubamp_render::hit::Transport;
            match command {
                Transport::Play => {
                    if let Some(h) = &handle {
                        h.set_active(true);
                    }
                }
                Transport::Pause | Transport::Stop => {
                    if let Some(h) = &handle {
                        h.set_active(false);
                    }
                }
                other => eprintln!("xubamp: {other:?} not wired yet (needs a playlist)"),
            }
        }
    };
    #[cfg(not(feature = "audio"))]
    let on_command = |command: xubamp_render::hit::Transport| {
        eprintln!("xubamp: transport command {command:?}");
    };

    // Feed the window's once-a-second clock. With audio, report the engine's elapsed seconds
    // (or `None` when nothing is playing, blanking the display); without it, always blank.
    #[cfg(feature = "audio")]
    let time_source = {
        let handle = _engine.as_ref().map(|engine| engine.handle());
        move || handle.as_ref().map(|h| h.elapsed_secs())
    };
    #[cfg(not(feature = "audio"))]
    let time_source = || None::<u32>;

    if let Err(e) = xubamp_wl::run(skin, title, on_command, time_source) {
        eprintln!("xubamp: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, track_title};

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
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
        assert_eq!(media.as_deref(), Some("song.mp3"));
    }

    #[test]
    fn order_independent_and_case_insensitive() {
        let (skin, media) = classify(s(&["track.MP3", "Base.WSZ"]));
        assert_eq!(skin.as_deref(), Some("Base.WSZ"));
        assert_eq!(media.as_deref(), Some("track.MP3"));
    }

    #[test]
    fn first_of_each_kind_wins_and_unknown_ignored() {
        let (skin, media) = classify(s(&["notes.txt", "a.mp3", "b.wav", "one.wsz", "two.wsz"]));
        assert_eq!(skin.as_deref(), Some("one.wsz"));
        assert_eq!(media.as_deref(), Some("a.mp3"));
    }

    #[test]
    fn no_recognized_args_yields_none() {
        let (skin, media) = classify(s(&["readme.md"]));
        assert!(skin.is_none());
        assert!(media.is_none());
    }
}
