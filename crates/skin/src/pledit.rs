//! `pledit.txt`: an INI-style file colouring the playlist editor. The `[Text]` section
//! carries hex colours (`Normal`, `Current`, `NormalBG`, `SelectedBG`, `MBFG`, `MBBG`)
//! and a `Font` name. There is no bitmap font for playlist rows; they use a system font,
//! so `Font` names a typeface we resolve through fontconfig later.
//!
//! Unknown or malformed keys keep the classic Winamp defaults.

use crate::color::Rgb;

/// Parsed playlist colours and font. Fields default to the classic Winamp look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlEdit {
    pub normal: Rgb,
    pub current: Rgb,
    pub normal_bg: Rgb,
    pub selected_bg: Rgb,
    pub mb_fg: Rgb,
    pub mb_bg: Rgb,
    pub font: String,
}

impl Default for PlEdit {
    fn default() -> Self {
        PlEdit {
            normal: Rgb::new(0x00, 0xFF, 0x00),
            current: Rgb::WHITE,
            normal_bg: Rgb::BLACK,
            selected_bg: Rgb::new(0x00, 0x00, 0xC6),
            mb_fg: Rgb::new(0x00, 0xFF, 0x00),
            mb_bg: Rgb::BLACK,
            font: "Arial".to_string(),
        }
    }
}

impl PlEdit {
    pub fn parse(text: &str) -> PlEdit {
        let mut p = PlEdit::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('[') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            match key.as_str() {
                "normal" => set(&mut p.normal, val),
                "current" => set(&mut p.current, val),
                "normalbg" => set(&mut p.normal_bg, val),
                "selectedbg" => set(&mut p.selected_bg, val),
                "mbfg" => set(&mut p.mb_fg, val),
                "mbbg" => set(&mut p.mb_bg, val),
                "font" if !val.is_empty() => p.font = val.to_string(),
                _ => {}
            }
        }
        p
    }
}

fn set(slot: &mut Rgb, val: &str) {
    if let Some(c) = Rgb::from_hex(val) {
        *slot = c;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_section() {
        let txt = "\
[Text]
Normal=#40FF20
Current=#FFFFFF
NormalBG=#101010
SelectedBG=0000C6
Font=Tahoma
";
        let p = PlEdit::parse(txt);
        assert_eq!(p.normal, Rgb::new(0x40, 0xFF, 0x20));
        assert_eq!(p.current, Rgb::WHITE);
        assert_eq!(p.normal_bg, Rgb::new(0x10, 0x10, 0x10));
        assert_eq!(p.selected_bg, Rgb::new(0x00, 0x00, 0xC6)); // no leading '#'
        assert_eq!(p.font, "Tahoma");
    }

    #[test]
    fn keeps_defaults_for_missing_and_bad_keys() {
        let p = PlEdit::parse("[Text]\nNormal=notacolor\nBogus=1\n");
        let d = PlEdit::default();
        assert_eq!(p.normal, d.normal); // malformed value ignored
        assert_eq!(p.selected_bg, d.selected_bg); // key absent
        assert_eq!(p.font, d.font);
    }

    #[test]
    fn empty_file_is_all_defaults() {
        assert_eq!(PlEdit::parse(""), PlEdit::default());
    }
}
