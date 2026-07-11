//! The decoded skin: sheets turned into images, ready for the renderer.

use crate::bmp::{self, Image};
use crate::container::SkinArchive;

/// Decoded skin sheets. Each is `None` when the skin omits it (the renderer then falls
/// back to the bundled default). Sheets are added here as later phases render them.
#[derive(Default)]
pub struct Skin {
    pub main: Option<Image>,
    pub titlebar: Option<Image>,
    pub cbuttons: Option<Image>,
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
    }
}
