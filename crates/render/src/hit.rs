//! Pure hit-testing: map a pointer position in the main window to the interactive region
//! under it. No platform types, so it is unit-testable without a compositor. The `wl` crate
//! calls this on pointer events. Regions are added here as controls are wired in later phases.

use xubamp_skin::sprites;

/// An interactive region of the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// The title-bar strip. Pressing here starts an interactive window move (classic drag).
    TitleBar,
    /// Not over any interactive element (the window body, for now).
    None,
}

/// Height of the draggable title-bar band, taken from the title-bar sprite so there is one
/// source of truth for the geometry.
pub const TITLEBAR_H: i32 = sprites::TITLEBAR_ACTIVE.src.h;

/// Which region of the main window is at window-local pixel (`x`, `y`)? Points outside the
/// window map to [`Region::None`].
pub fn hit_test(x: i32, y: i32) -> Region {
    if x < 0 || y < 0 || x >= sprites::MAIN_W || y >= sprites::MAIN_H {
        return Region::None;
    }
    if y < TITLEBAR_H {
        return Region::TitleBar;
    }
    Region::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_bar_band_is_the_top_strip() {
        assert_eq!(TITLEBAR_H, 14);
        assert_eq!(hit_test(0, 0), Region::TitleBar, "top-left corner");
        assert_eq!(hit_test(274, 13), Region::TitleBar, "bottom-right of the band");
        assert_eq!(hit_test(137, 7), Region::TitleBar, "middle of the band");
    }

    #[test]
    fn below_the_band_is_not_draggable() {
        assert_eq!(hit_test(0, 14), Region::None, "first row under the title bar");
        assert_eq!(hit_test(137, 60), Region::None, "window body");
        assert_eq!(hit_test(274, 115), Region::None, "bottom-right of the window");
    }

    #[test]
    fn points_outside_the_window_are_none() {
        assert_eq!(hit_test(-1, 5), Region::None);
        assert_eq!(hit_test(5, -1), Region::None);
        assert_eq!(hit_test(sprites::MAIN_W, 5), Region::None);
        assert_eq!(hit_test(5, sprites::MAIN_H), Region::None);
    }
}
