//! Which text field has the keyboard, and its caret.

use super::*;

impl Browser {
    /// Whether a caret is on screen and blinking. The cheap half of
    /// [`Self::focused_input`], for the idle clock — a field that has been
    /// blurred draws no caret and is no reason to keep waking the window.
    pub(super) fn has_focused_input(&self) -> bool {
        self.rename
            .as_ref()
            .map(|session| &session.input)
            .or(self.path_entry.as_ref())
            .or(self.save_name.as_ref())
            .or(self.search.as_ref())
            .is_some_and(|input| input.state.focused())
    }

    /// The field holding the keyboard, if any.
    ///
    /// The order is [`Self::on_key_event`]'s own precedence, so the caret that
    /// blinks is always the caret the keys are going to: a rename takes the
    /// keyboard from everything, then the path entry, then the picker's name
    /// field, and the filter strip's query last.
    pub(super) fn focused_input(&mut self) -> Option<&mut TextInput> {
        // The palette takes the keyboard whole while it is up, so its field is
        // the one blinking whatever else is open behind it.
        if let Some(palette) = self.palette.as_mut() {
            return Some(palette.input_mut());
        }
        if let Some(session) = self.rename.as_mut() {
            return Some(&mut session.input);
        }
        if let Some(input) = self.path_entry.as_mut() {
            return Some(input);
        }
        if let Some(input) = self.save_name.as_mut() {
            return Some(input);
        }
        self.search.as_mut()
    }

    /// Advance the focused field's caret blink, and say whether the caret
    /// changed phase.
    ///
    /// Only the phase change is worth a repaint. The clock ticks at 125 Hz
    /// with everything else; the caret turns over about twice a second, and
    /// repainting the window on every tick to draw the same caret would be a
    /// hundred wasted frames for each one that shows something new.
    pub(super) fn tick_caret(&mut self, delta: f32) -> bool {
        match self.focused_input() {
            Some(input) => {
                let was = input.caret_visible();
                input.tick(delta);
                was != input.caret_visible()
            }
            None => false,
        }
    }

    /// Seconds since the caret was last advanced — real time, not a count of
    /// passes through the loop. Capped so a stall does not land as one long
    /// step, and one idle tick's worth on the first call.
    pub(super) fn caret_elapsed(&mut self) -> f32 {
        let now = std::time::Instant::now();
        match self.caret_clock.replace(now) {
            Some(last) => now.duration_since(last).as_secs_f32().min(0.25),
            None => IDLE_TICK.as_secs_f32(),
        }
    }
}
