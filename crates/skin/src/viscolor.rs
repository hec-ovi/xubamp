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

/// The parsed visualization palette: 24 colours indexed by their fixed role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisColor {
    pub colors: [Rgb; 24],
}

impl VisColor {
    pub fn parse(text: &str) -> VisColor {
        let mut colors = [Rgb::BLACK; 24];
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
    fn short_file_leaves_rest_black() {
        let vc = VisColor::parse("10,20,30\n40,50,60\n");
        assert_eq!(vc.colors[0], Rgb::new(10, 20, 30));
        assert_eq!(vc.colors[1], Rgb::new(40, 50, 60));
        assert_eq!(vc.colors[2], Rgb::BLACK);
        assert_eq!(vc.peak(), Rgb::BLACK);
    }

    #[test]
    fn ignores_blank_and_comment_only_lines() {
        let vc = VisColor::parse("\n// header comment\n5,6,7\n");
        assert_eq!(vc.colors[0], Rgb::new(5, 6, 7));
    }
}
