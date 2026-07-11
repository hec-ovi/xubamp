//! Input mapping and UI state: turn a pointer position into the interactive region under it,
//! and turn press/release events into state changes and commands. All pure (no platform
//! types), so the interaction policy is unit-testable without a compositor. The `wl` crate
//! owns the event loop and calls these; it does the side effects (redraw, window move, emit
//! command) that the outcomes describe.

use xubamp_skin::sprites;

/// The six classic transport buttons, in the order they appear on the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Prev,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
}

/// Transport identity for each entry of [`sprites::CBUTTONS`] (and `CBUTTONS_PRESSED`), in the
/// same order, so the compositor can pick the pressed sprite for the held button.
pub const TRANSPORT_ORDER: [Transport; 6] = [
    Transport::Prev,
    Transport::Play,
    Transport::Pause,
    Transport::Stop,
    Transport::Next,
    Transport::Eject,
];

/// An interactive region of the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// The title-bar strip. Pressing here starts an interactive window move (classic drag).
    TitleBar,
    /// One of the six transport buttons.
    Transport(Transport),
    /// Not over any interactive element (the window body, for now).
    None,
}

/// Height of the draggable title-bar band, taken from the title-bar sprite so there is one
/// source of truth for the geometry.
pub const TITLEBAR_H: i32 = sprites::TITLEBAR_ACTIVE.src.h;

/// Does window-local pixel (`x`, `y`) fall inside the on-window rectangle of button `b`? The
/// button's screen rectangle is its destination plus the source sprite's width and height.
fn in_button(b: &sprites::Placement, x: i32, y: i32) -> bool {
    x >= b.dst_x && x < b.dst_x + b.src.w && y >= b.dst_y && y < b.dst_y + b.src.h
}

/// Which region of the main window is at window-local pixel (`x`, `y`)? Points outside the
/// window map to [`Region::None`]. Transport buttons win over the body; the title-bar band is
/// the top strip. (Buttons live well below the band, so the two never overlap.)
pub fn hit_test(x: i32, y: i32) -> Region {
    if x < 0 || y < 0 || x >= sprites::MAIN_W || y >= sprites::MAIN_H {
        return Region::None;
    }
    for (placement, id) in sprites::CBUTTONS.iter().zip(TRANSPORT_ORDER) {
        if in_button(placement, x, y) {
            return Region::Transport(id);
        }
    }
    if y < TITLEBAR_H {
        return Region::TitleBar;
    }
    Region::None
}

/// Mutable UI state that drives composition. Grows as controls are added (position, volume,
/// title, playback state). For now: which transport button, if any, is held down.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiState {
    /// The transport button currently pressed (drawn depressed), or `None`.
    pub pressed: Option<Transport>,
}

/// What the platform layer should do after a left-button press. Returned by [`on_press`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressOutcome {
    /// The press was on the title bar: start an interactive window move.
    StartMove,
    /// UI state changed (a button went down): recompose and redraw.
    Redraw,
    /// Nothing to do.
    Ignore,
}

/// What the platform layer should do after a left-button release. Returned by [`on_release`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseOutcome {
    /// The transport command to run, if a button was pressed and released over itself.
    pub command: Option<Transport>,
    /// Whether UI state changed and the window should be recomposed and redrawn.
    pub redraw: bool,
}

/// Handle a left-button press at window-local (`x`, `y`), updating `state`.
pub fn on_press(state: &mut UiState, x: i32, y: i32) -> PressOutcome {
    match hit_test(x, y) {
        Region::TitleBar => PressOutcome::StartMove,
        Region::Transport(b) => {
            state.pressed = Some(b);
            PressOutcome::Redraw
        }
        Region::None => PressOutcome::Ignore,
    }
}

/// Handle a left-button release at window-local (`x`, `y`), updating `state`. A transport
/// command fires only when the release lands on the same button that was pressed (releasing
/// off the button cancels), matching classic button behavior.
pub fn on_release(state: &mut UiState, x: i32, y: i32) -> ReleaseOutcome {
    match state.pressed.take() {
        Some(b) => {
            let command = (hit_test(x, y) == Region::Transport(b)).then_some(b);
            ReleaseOutcome {
                command,
                redraw: true,
            }
        }
        None => ReleaseOutcome {
            command: None,
            redraw: false,
        },
    }
}

/// Handle the pointer leaving the window: cancel any in-progress press so a button never
/// stays stuck down. Returns whether a redraw is needed.
pub fn on_leave(state: &mut UiState) -> bool {
    state.pressed.take().is_some()
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

    #[test]
    fn transport_buttons_hit_at_their_centers() {
        // Centers of the six buttons, derived from CBUTTONS destinations + sizes.
        let expect = [
            (Transport::Prev, 16 + 11, 88 + 9),
            (Transport::Play, 39 + 11, 88 + 9),
            (Transport::Pause, 62 + 11, 88 + 9),
            (Transport::Stop, 85 + 11, 88 + 9),
            (Transport::Next, 108 + 11, 88 + 9),
            (Transport::Eject, 136 + 11, 89 + 8),
        ];
        for (id, x, y) in expect {
            assert_eq!(hit_test(x, y), Region::Transport(id), "{id:?} at ({x},{y})");
        }
    }

    #[test]
    fn just_outside_a_button_is_not_a_hit() {
        // One pixel left of Play (Play starts at x=39) is still Previous' right edge or gap.
        assert_eq!(hit_test(38, 97), Region::Transport(Transport::Prev));
        // Below the button row (buttons end at y=88+18=106) is the body.
        assert_eq!(hit_test(50, 106), Region::None);
        // The gap between Next (ends x=130) and Eject (starts x=136) is not a button.
        assert_eq!(hit_test(132, 97), Region::None);
    }

    #[test]
    fn press_on_a_button_arms_it_and_asks_for_redraw() {
        let mut s = UiState::default();
        let out = on_press(&mut s, 39 + 11, 88 + 9); // play center
        assert_eq!(out, PressOutcome::Redraw);
        assert_eq!(s.pressed, Some(Transport::Play));
    }

    #[test]
    fn press_on_title_bar_starts_a_move_and_does_not_arm() {
        let mut s = UiState::default();
        let out = on_press(&mut s, 100, 5);
        assert_eq!(out, PressOutcome::StartMove);
        assert_eq!(s.pressed, None);
    }

    #[test]
    fn release_over_the_same_button_fires_the_command() {
        let mut s = UiState {
            pressed: Some(Transport::Play),
        };
        let out = on_release(&mut s, 39 + 11, 88 + 9);
        assert_eq!(out.command, Some(Transport::Play));
        assert!(out.redraw);
        assert_eq!(s.pressed, None, "button released");
    }

    #[test]
    fn release_off_the_button_cancels_the_command() {
        let mut s = UiState {
            pressed: Some(Transport::Play),
        };
        let out = on_release(&mut s, 200, 40); // released over the body
        assert_eq!(out.command, None, "dragged off = cancel");
        assert!(out.redraw, "still redraw to un-press");
        assert_eq!(s.pressed, None);
    }

    #[test]
    fn release_with_nothing_pressed_is_a_no_op() {
        let mut s = UiState::default();
        let out = on_release(&mut s, 39 + 11, 88 + 9);
        assert_eq!(out.command, None);
        assert!(!out.redraw);
    }

    #[test]
    fn leave_clears_a_pressed_button() {
        let mut s = UiState {
            pressed: Some(Transport::Stop),
        };
        assert!(on_leave(&mut s), "needs redraw to un-press");
        assert_eq!(s.pressed, None);
        assert!(!on_leave(&mut s), "nothing pressed now");
    }
}
