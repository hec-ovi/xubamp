//! `viscolor.txt`: exactly 24 lines of `R,G,B` (with optional `// comment`) that colour
//! the visualization. The role of each line is fixed by index:
//!
//! - 0: background
//! - 1: the grid dots
//! - 2..18: 16-colour spectrum-analyzer gradient (quiet to loud)
//! - 18..23: 5-colour oscilloscope
//! - 23: analyzer peak dots
//!
//! Missing or malformed lines fall back to black, so a short file still yields a full
//! 24-entry palette.

use crate::color::Rgb;

/// The classic default visualization palette (Winamp's built-in / Webamp `baseSkin`), used as the
/// base so a missing or partial `viscolor.txt` keeps sensible colours rather than going black. A
/// well-formed skin overwrites all 24; a skin with no `viscolor.txt` still renders a real spectrum.
pub const DEFAULT: [Rgb; 24] = [
    Rgb::new(0, 0, 0),       // 0  background
    Rgb::new(24, 33, 41),    // 1  grid dots
    Rgb::new(239, 49, 16),   // 2  spectrum top (hottest)
    Rgb::new(206, 41, 16),   // 3
    Rgb::new(214, 90, 0),    // 4
    Rgb::new(214, 102, 0),   // 5
    Rgb::new(214, 115, 0),   // 6
    Rgb::new(198, 123, 8),   // 7
    Rgb::new(222, 165, 24),  // 8
    Rgb::new(214, 181, 33),  // 9
    Rgb::new(189, 222, 41),  // 10
    Rgb::new(148, 222, 33),  // 11
    Rgb::new(41, 206, 16),   // 12
    Rgb::new(50, 190, 16),   // 13
    Rgb::new(57, 181, 16),   // 14
    Rgb::new(49, 156, 8),    // 15
    Rgb::new(41, 148, 0),    // 16
    Rgb::new(24, 132, 8),    // 17 spectrum bottom
    Rgb::new(255, 255, 255), // 18 oscilloscope centre
    Rgb::new(214, 214, 222), // 19
    Rgb::new(181, 189, 189), // 20
    Rgb::new(160, 170, 175), // 21
    Rgb::new(148, 156, 165), // 22 oscilloscope edge
    Rgb::new(150, 150, 150), // 23 analyzer peak dot
];

/// The parsed visualization palette: 24 colours indexed by their fixed role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisColor {
    pub colors: [Rgb; 24],
}

impl Default for VisColor {
    /// The classic default palette, for a skin that ships no `viscolor.txt`.
    fn default() -> Self {
        VisColor { colors: DEFAULT }
    }
}

impl VisColor {
    /// Parse `viscolor.txt`. Each successfully parsed line overwrites the next role index, starting
    /// from [`DEFAULT`], so a partial file keeps defaults for the indices it omits.
    pub fn parse(text: &str) -> VisColor {
        let mut colors = DEFAULT;
        let mut i = 0;
        for line in text.lines() {
            if i >= 24 {
                break;
            }
            let code = line.split("//").next().unwrap_or("").trim();
            if code.is_empty() {
                continue;
            }
            if let Some(rgb) = parse_triplet(code) {
                colors[i] = rgb;
                i += 1;
            }
        }
        VisColor { colors }
    }

    pub fn background(&self) -> Rgb {
        self.colors[0]
    }
    pub fn dots(&self) -> Rgb {
        self.colors[1]
    }
    /// The 16-colour spectrum-analyzer gradient, quiet to loud.
    pub fn analyzer(&self) -> &[Rgb] {
        &self.colors[2..18]
    }
    /// The 5-colour oscilloscope palette.
    pub fn oscilloscope(&self) -> &[Rgb] {
        &self.colors[18..23]
    }
    pub fn peak(&self) -> Rgb {
        self.colors[23]
    }
}

fn parse_triplet(s: &str) -> Option<Rgb> {
    let mut it = s.split(',').map(str::trim);
    let r = it.next()?.parse::<u8>().ok()?;
    let g = it.next()?.parse::<u8>().ok()?;
    let b = it.next()?.parse::<u8>().ok()?;
    Some(Rgb::new(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_24_roles_in_order() {
        let mut txt = String::new();
        for i in 0..24u8 {
            txt.push_str(&format!("{i},{i},{i} // role {i}\n"));
        }
        let vc = VisColor::parse(&txt);
        assert_eq!(vc.background(), Rgb::new(0, 0, 0));
        assert_eq!(vc.dots(), Rgb::new(1, 1, 1));
        assert_eq!(vc.analyzer().len(), 16);
        assert_eq!(vc.analyzer()[0], Rgb::new(2, 2, 2));
        assert_eq!(vc.analyzer()[15], Rgb::new(17, 17, 17));
        assert_eq!(vc.oscilloscope().len(), 5);
        assert_eq!(vc.oscilloscope()[0], Rgb::new(18, 18, 18));
        assert_eq!(vc.peak(), Rgb::new(23, 23, 23));
    }

    #[test]
    fn short_file_keeps_defaults_for_omitted_roles() {
        let vc = VisColor::parse("10,20,30\n40,50,60\n");
        // The two parsed lines override roles 0 and 1...
        assert_eq!(vc.colors[0], Rgb::new(10, 20, 30));
        assert_eq!(vc.colors[1], Rgb::new(40, 50, 60));
        // ...and the rest keep the classic defaults rather than going black.
        assert_eq!(vc.colors[2], DEFAULT[2], "spectrum top keeps its default");
        assert_eq!(vc.peak(), DEFAULT[23], "peak dot keeps its default");
    }

    #[test]
    fn default_palette_is_the_classic_one() {
        let vc = VisColor::default();
        assert_eq!(vc.background(), Rgb::new(0, 0, 0));
        assert_eq!(vc.analyzer()[0], Rgb::new(239, 49, 16), "hottest bar colour");
        assert_eq!(vc.oscilloscope()[0], Rgb::new(255, 255, 255), "oscilloscope centre");
        assert_eq!(vc.peak(), Rgb::new(150, 150, 150));
    }

    #[test]
    fn ignores_blank_and_comment_only_lines() {
        let vc = VisColor::parse("\n// header comment\n5,6,7\n");
        assert_eq!(vc.colors[0], Rgb::new(5, 6, 7));
    }
}
