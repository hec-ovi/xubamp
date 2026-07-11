//! xubamp binary entry point.
//!
//! Composes a skin's main window and shows it in a native Wayland window. The skin is
//! resolved in order: a `.wsz`/`.zip` path argument, then `$XUBAMP_SKIN`, then a local
//! `skins/` test skin if one is checked out, and finally the built-in default skin
//! (original, clean-room; see `xubamp_skin::default_skin`). Audio, real skin
//! interactivity, and the rest land in later phases; see `docs/ARCHITECTURE.md`.

use std::path::Path;

use xubamp_skin::bmp::Image;
use xubamp_skin::container::SkinArchive;
use xubamp_skin::{default_skin, Skin};

/// A local skin used only during development. It lives under `skins/`, which is gitignored
/// (third-party art, never committed or shipped), so a released binary never finds it and
/// falls through to the built-in default. This is the "use the XMMS skin for now" hook.
const DEV_SKIN: &str = "skins/XMMS_standard_skin.wsz";

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

/// Resolve which skin to show, in priority order: CLI path, `$XUBAMP_SKIN`, a local dev
/// skin if checked out, else the built-in default.
fn resolve_skin() -> Skin {
    if let Some(path) = std::env::args().nth(1) {
        return load_skin(&path);
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
    let skin = resolve_skin();
    let fb = xubamp_render::compose_main_window(&skin);

    // Debug affordance / seed for the later headless render-diff harness: dump the raw
    // RGBA the window would display, then exit without opening a window.
    if let Ok(path) = std::env::var("XUBAMP_DUMP_RGBA") {
        std::fs::write(&path, &fb.rgba).expect("write rgba dump");
        println!("dumped {}x{} rgba to {path}", fb.width, fb.height);
        return;
    }

    if let Err(e) = xubamp_wl::run(fb) {
        eprintln!("xubamp: {e}");
        std::process::exit(1);
    }
}
