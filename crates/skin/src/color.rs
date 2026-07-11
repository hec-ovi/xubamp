//! A plain 8-bit RGB colour, shared by the skin config parsers.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const BLACK: Rgb = Rgb::new(0, 0, 0);
    pub const WHITE: Rgb = Rgb::new(255, 255, 255);

    /// Parse `#RRGGBB` or `RRGGBB` (case-insensitive). Returns `None` on malformed hex.
    pub fn from_hex(s: &str) -> Option<Rgb> {
        let h = s.trim().strip_prefix('#').unwrap_or(s.trim());
        if h.len() != 6 {
            return None;
        }
        Some(Rgb::new(
            u8::from_str_radix(&h[0..2], 16).ok()?,
            u8::from_str_radix(&h[2..4], 16).ok()?,
            u8::from_str_radix(&h[4..6], 16).ok()?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_with_and_without_hash() {
        assert_eq!(Rgb::from_hex("#00FF00"), Some(Rgb::new(0, 255, 0)));
        assert_eq!(Rgb::from_hex("0000c6"), Some(Rgb::new(0, 0, 0xC6)));
        assert_eq!(Rgb::from_hex("  #FFFFFF "), Some(Rgb::WHITE));
    }

    #[test]
    fn rejects_bad_hex() {
        assert_eq!(Rgb::from_hex("#fff"), None); // too short
        assert_eq!(Rgb::from_hex("nothex!"), None);
        assert_eq!(Rgb::from_hex(""), None);
    }
}
