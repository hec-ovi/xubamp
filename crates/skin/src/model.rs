//! The decoded skin: sheets turned into images, ready for the renderer.

use crate::bmp::{self, Image};
use crate::container::SkinArchive;
use crate::viscolor::VisColor;

/// Decoded skin sheets. Each is `None` when the skin omits it (the renderer then falls
/// back to the bundled default). Sheets are added here as later phases render them.
#[derive(Default)]
pub struct Skin {
    pub main: Option<Image>,
    pub titlebar: Option<Image>,
    pub cbuttons: Option<Image>,
    /// The digit sheet for the time display. Loaded from `nums_ex.bmp` when the skin has it,
    /// else the older `numbers.bmp`; the ten digit cells sit at identical coordinates in both.
    pub numbers: Option<Image>,
    /// The bitmap font sheet (`text.bmp`) for the song-title marquee. `None` when the skin
    /// omits it, in which case the marquee simply does not draw.
    pub text: Option<Image>,
    /// The volume slider sheet (`volume.bmp`): a column of level-indicator frames plus the
    /// thumb. `None` skips drawing the volume slider.
    pub volume: Option<Image>,
    /// The balance slider sheet (`balance.bmp`), same layout as `volume`. `None` skips the
    /// balance slider.
    pub balance: Option<Image>,
    /// The position/seek bar sheet (`posbar.bmp`): the 248x10 groove background plus the two
    /// thumb states. `None` skips drawing the seek bar (the main background groove shows through).
    pub posbar: Option<Image>,
    /// The visualization palette (`viscolor.txt`): the 24 fixed colours for the spectrum and
    /// oscilloscope. `None` when the skin omits it; the visualizer then draws nothing (it needs a
    /// palette), though a caller could substitute [`VisColor::default`].
    pub viscolor: Option<VisColor>,
}

impl Skin {
    /// Decode the main-window sheets from a loaded archive. An undecodable sheet becomes
    /// `None` rather than failing the whole load.
    pub fn from_archive(archive: &SkinArchive) -> Skin {
        let sheet = |name: &str| archive.get(name).and_then(|b| bmp::decode(b).ok());
        Skin {
            main: sheet("main.bmp"),
            titlebar: sheet("titlebar.bmp"),
            cbuttons: sheet("cbuttons.bmp"),
            // Prefer the extended digit sheet (it carries full blank and minus cells); fall
            // back to the classic one. Digits are at the same cells in both, so the renderer
            // draws either the same way.
            numbers: sheet("nums_ex.bmp").or_else(|| sheet("numbers.bmp")),
            text: sheet("text.bmp"),
            volume: sheet("volume.bmp"),
            balance: sheet("balance.bmp"),
            posbar: sheet("posbar.bmp"),
            viscolor: archive
                .get("viscolor.txt")
                .map(|b| VisColor::parse(&String::from_utf8_lossy(b))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{solid_bmp_24, wsz_stored};

    #[test]
    fn decodes_present_sheets_and_leaves_missing_none() {
        // Real path: BMP bytes -> .wsz -> archive -> decoded Skin.
        let main = solid_bmp_24(275, 116, 10, 20, 30);
        let wsz = wsz_stored(&[("MAIN.BMP", &main)]);
        let archive = SkinArchive::from_bytes(&wsz).unwrap();

        let skin = Skin::from_archive(&archive);
        let img = skin.main.expect("main sheet decoded");
        assert_eq!((img.width, img.height), (275, 116));
        assert_eq!(&img.rgba[0..4], &[10, 20, 30, 255]); // top-left pixel
        assert!(skin.cbuttons.is_none());
        assert!(skin.titlebar.is_none());
        assert!(skin.numbers.is_none());
        assert!(skin.text.is_none());
        assert!(skin.volume.is_none());
        assert!(skin.balance.is_none());
        assert!(skin.posbar.is_none());
        assert!(skin.viscolor.is_none());
    }

    #[test]
    fn parses_viscolor_txt_when_present() {
        // A minimal viscolor.txt: role 0 (background) and role 2 (spectrum top) set, rest default.
        let vis = b"0,0,0 // bg\n1,1,1\n255,0,0 // top\n";
        let wsz = wsz_stored(&[("VISCOLOR.TXT", vis)]);
        let archive = SkinArchive::from_bytes(&wsz).unwrap();

        let skin = Skin::from_archive(&archive);
        let vc = skin.viscolor.expect("viscolor parsed");
        assert_eq!(vc.background(), crate::color::Rgb::new(0, 0, 0));
        assert_eq!(vc.analyzer()[0], crate::color::Rgb::new(255, 0, 0), "spectrum top from the file");
    }

    #[test]
    fn decodes_the_posbar_sheet_when_present() {
        // A classic posbar.bmp is 307x10 (a 248px groove plus the two 29px thumb cells); here the
        // exact size does not matter, only that it lands in `skin.posbar`.
        let posbar = solid_bmp_24(307, 10, 7, 8, 9);
        let wsz = wsz_stored(&[("POSBAR.BMP", &posbar)]);
        let archive = SkinArchive::from_bytes(&wsz).unwrap();

        let skin = Skin::from_archive(&archive);
        let p = skin.posbar.expect("posbar sheet decoded");
        assert_eq!((p.width, p.height), (307, 10));
        assert_eq!(&p.rgba[0..4], &[7, 8, 9, 255]);
    }

    #[test]
    fn decodes_the_slider_sheets_when_present() {
        // Classic volume.bmp/balance.bmp are 68x433 (a 420px frame column plus the thumb row);
        // here the exact size does not matter, only that each lands in its own field.
        let volume = solid_bmp_24(68, 433, 1, 2, 3);
        let balance = solid_bmp_24(47, 433, 4, 5, 6);
        let wsz = wsz_stored(&[("VOLUME.BMP", &volume), ("BALANCE.BMP", &balance)]);
        let archive = SkinArchive::from_bytes(&wsz).unwrap();

        let skin = Skin::from_archive(&archive);
        let v = skin.volume.expect("volume sheet decoded");
        assert_eq!((v.width, v.height), (68, 433));
        assert_eq!(&v.rgba[0..4], &[1, 2, 3, 255]);
        let b = skin.balance.expect("balance sheet decoded");
        assert_eq!((b.width, b.height), (47, 433));
        assert_eq!(&b.rgba[0..4], &[4, 5, 6, 255]);
    }

    #[test]
    fn decodes_the_text_font_sheet_when_present() {
        // A classic text.bmp is 155x18 (31 cells wide, 3 rows of 6px). Its presence is what
        // switches the marquee on, so decoding it into `skin.text` is the gate.
        let text = solid_bmp_24(155, 18, 4, 5, 6);
        let wsz = wsz_stored(&[("TEXT.BMP", &text)]);
        let archive = SkinArchive::from_bytes(&wsz).unwrap();

        let skin = Skin::from_archive(&archive);
        let img = skin.text.expect("text sheet decoded");
        assert_eq!((img.width, img.height), (155, 18));
        assert_eq!(&img.rgba[0..4], &[4, 5, 6, 255]);
    }

    #[test]
    fn prefers_nums_ex_over_numbers() {
        // Both digit sheets present: nums_ex.bmp wins, its top-left pixel proves which loaded.
        let nums_ex = solid_bmp_24(108, 13, 1, 2, 3);
        let numbers = solid_bmp_24(99, 13, 9, 8, 7);
        let wsz = wsz_stored(&[("NUMS_EX.BMP", &nums_ex), ("NUMBERS.BMP", &numbers)]);
        let archive = SkinArchive::from_bytes(&wsz).unwrap();

        let skin = Skin::from_archive(&archive);
        let img = skin.numbers.expect("a digit sheet decoded");
        assert_eq!((img.width, img.height), (108, 13), "the 108-wide nums_ex sheet");
        assert_eq!(&img.rgba[0..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn falls_back_to_numbers_when_nums_ex_absent() {
        let numbers = solid_bmp_24(99, 13, 9, 8, 7);
        let wsz = wsz_stored(&[("NUMBERS.BMP", &numbers)]);
        let archive = SkinArchive::from_bytes(&wsz).unwrap();

        let skin = Skin::from_archive(&archive);
        let img = skin.numbers.expect("numbers sheet decoded");
        assert_eq!((img.width, img.height), (99, 13), "the 99-wide classic sheet");
        assert_eq!(&img.rgba[0..4], &[9, 8, 7, 255]);
    }
}
