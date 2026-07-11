//! xubamp binary entry point.
//!
//! Phase 2: composes the main window from a placeholder skin and shows it in a native
//! Wayland window. Audio, real skin loading, and interactivity land in later phases; see
//! `docs/ARCHITECTURE.md`.

use xubamp_skin::bmp::Image;
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

fn main() {
    let skin = placeholder_skin();
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
