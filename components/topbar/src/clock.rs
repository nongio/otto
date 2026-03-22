use chrono::Local;

use crate::config::CLOCK_FORMAT;

/// Minimal clock state — just the current formatted time string.
pub struct Clock {
    pub text: String,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            text: Self::formatted_now(),
        }
    }

    /// Update the stored text. Returns `true` if the string changed.
    pub fn tick(&mut self) -> bool {
        let new = Self::formatted_now();
        if new != self.text {
            self.text = new;
            true
        } else {
            false
        }
    }

    fn formatted_now() -> String {
        Local::now().format(CLOCK_FORMAT).to_string()
    }
}
