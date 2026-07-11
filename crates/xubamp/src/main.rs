//! xubamp binary entry point.
//!
//! Phase 2: composes a skin's main window and shows it in a native Wayland window. Pass a
//! `.wsz` (or `.zip`) skin path to load it, or none for a placeholder. Audio, real skin
//! interactivity, and the rest land in later phases; see `docs/ARCHITECTURE.md`.

use xubamp_skin::bmp::Image;
use xubamp_skin::container::SkinArchive;
use xubamp_skin::Skin;

fn solid_image(w: u32, h: u32, rgb: [u8; 3]) -> Image {
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for _ in 0..(w * h) {
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

/// A temporary placeholder skin until an original default skin is authored: a dark main
/// window with green transport-button rects, enough to see the compose pipeline working.
fn placeholder_skin() -> Skin {
    Skin {
        main: Some(solid_image(275, 116, [28, 28, 38])),
        cbuttons: Some(solid_image(136, 36, [60, 200, 90])),
        ..Default::default()
    }
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

fn main() {
    let skin = match std::env::args().nth(1) {
        Some(path) => load_skin(&path),
        None => placeholder_skin(),
    };
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
