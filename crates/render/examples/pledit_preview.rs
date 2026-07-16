//! Scratch preview: compose the playlist with a real skin and dump raw RGBA for inspection.
//! Usage: cargo run -p xubamp-render --example pledit_preview -- <skin.wsz> <out.raw>
//! The output is width*height*4 raw RGBA preceded by a text header line "W H\n".

use std::io::Write as _;

use xubamp_render::adwaita::UiFont;
use xubamp_render::pledit::{self, PlState, Row};
use xubamp_skin::container::SkinArchive;
use xubamp_skin::sprites;
use xubamp_skin::Skin;

fn main() {
    let mut args = std::env::args().skip(1);
    let wsz = args.next().expect("skin path");
    let out = args.next().expect("output path");
    let bytes = std::fs::read(&wsz).expect("read skin");
    let archive = SkinArchive::from_bytes(&bytes).expect("parse skin");
    let skin = Skin::from_archive(&archive);

    let name = skin
        .pledit_colors
        .as_ref()
        .map_or("Arial", |c| c.font.as_str());
    let font = UiFont::load_named(name).or_else(UiFont::load_system);

    let rows = vec![
        ("1. DJ Mike Llama - Llama Whippin' Intro", "0:05"),
        ("2. Aphex Twin - Windowlicker (Acid Edit)", "6:07"),
        ("3. Boards of Canada - Roygbiv", "2:31"),
        ("4. UPPERCASE TITLE STAYS UPPERCASE", "3:33"),
        ("5. lowercase title stays lowercase", "1:11"),
    ]
    .into_iter()
    .map(|(t, d)| Row {
        title: t.to_owned(),
        duration: d.to_owned(),
        ..Default::default()
    })
    .collect();
    let state = PlState {
        rows,
        current: Some(1),
        selected: vec![2],
        ..Default::default()
    };

    let fb = pledit::compose(
        &skin,
        &state,
        font.as_ref(),
        sprites::PLEDIT_W,
        sprites::PLEDIT_H,
    );
    let mut f = std::fs::File::create(&out).expect("create output");
    writeln!(f, "{} {}", fb.width, fb.height).unwrap();
    f.write_all(&fb.rgba).unwrap();
    eprintln!("wrote {}x{} to {}", fb.width, fb.height, out);
}
