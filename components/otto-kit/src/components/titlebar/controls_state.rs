use super::{WindowControl, WindowControls};

/// Pointer state of a titlebar's traffic lights.
///
/// A client that draws its own decoration also owns the bookkeeping that makes
/// the glyphs appear: which control the pointer is over, and which one is being
/// held. This keeps that in one place — feed it the control under the pointer,
/// and repaint when it says something changed.
///
/// Every mutating method returns whether the drawn state changed, so a handler
/// can request a frame only when the pixels would differ.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowControlsState {
    hovered: bool,
    pressed: Option<WindowControl>,
}

impl WindowControlsState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pointer is over the group, so the glyphs are revealed.
    pub fn hovered(&self) -> bool {
        self.hovered
    }

    /// The control being held down, drawn a shade darker.
    pub fn pressed(&self) -> Option<WindowControl> {
        self.pressed
    }

    /// The pointer moved to a point over `control` (or, with `None`, off the
    /// group).
    pub fn on_motion(&mut self, control: Option<WindowControl>) -> bool {
        self.set_hovered(control.is_some())
    }

    /// The pointer left the surface entirely. Without this the glyphs stay
    /// drawn on a window the pointer has already moved away from.
    pub fn on_leave(&mut self) -> bool {
        let changed = self.hovered || self.pressed.is_some();
        self.hovered = false;
        self.pressed = None;
        changed
    }

    /// A press landed on `control`, which arms it. Returns whether it hit one
    /// at all, so a press on the bare bar can fall through to a window move.
    pub fn on_press(&mut self, control: Option<WindowControl>) -> bool {
        self.hovered = control.is_some() || self.hovered;
        self.pressed = control;
        control.is_some()
    }

    /// A release landed on `control`. The control fires only when the press
    /// and the release land on the same one, the rule a button follows
    /// anywhere else; returns the one that fired, if any.
    pub fn on_release(&mut self, control: Option<WindowControl>) -> Option<WindowControl> {
        let armed = self.pressed.take();
        self.hovered = control.is_some();
        match (armed, control) {
            (Some(armed), Some(released)) if armed == released => Some(armed),
            _ => None,
        }
    }

    /// Hand the state to a control group about to be drawn.
    pub fn apply(&self, controls: WindowControls) -> WindowControls {
        controls
            .with_hovered(self.hovered)
            .with_pressed(self.pressed)
    }

    fn set_hovered(&mut self, hovered: bool) -> bool {
        let changed = self.hovered != hovered;
        self.hovered = hovered;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_reports_only_real_changes() {
        let mut state = WindowControlsState::new();
        assert!(state.on_motion(Some(WindowControl::Close)));
        assert!(state.hovered());
        // Moving between dots keeps the group hovered, so nothing repaints.
        assert!(!state.on_motion(Some(WindowControl::Zoom)));
        assert!(state.on_motion(None));
        assert!(!state.hovered());
    }

    #[test]
    fn a_control_fires_only_where_it_was_pressed() {
        let mut state = WindowControlsState::new();
        state.on_press(Some(WindowControl::Close));
        assert_eq!(state.pressed(), Some(WindowControl::Close));
        assert_eq!(state.on_release(Some(WindowControl::Zoom)), None);
        assert_eq!(state.pressed(), None);

        state.on_press(Some(WindowControl::Close));
        assert_eq!(
            state.on_release(Some(WindowControl::Close)),
            Some(WindowControl::Close)
        );
    }

    #[test]
    fn a_press_dragged_off_the_bar_fires_nothing() {
        let mut state = WindowControlsState::new();
        state.on_press(Some(WindowControl::Minimize));
        assert_eq!(state.on_release(None), None);
    }

    #[test]
    fn leaving_clears_everything() {
        let mut state = WindowControlsState::new();
        state.on_press(Some(WindowControl::Close));
        assert!(state.on_leave());
        assert!(!state.hovered());
        assert_eq!(state.pressed(), None);
        assert!(!state.on_leave());
    }
}
