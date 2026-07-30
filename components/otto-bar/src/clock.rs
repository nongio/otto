use chrono::Local;

use crate::config::clock_format;

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
        Local::now().format(clock_format()).to_string()
    }

    /// Time until the clock text next changes: the next second boundary when
    /// the format shows seconds, otherwise the next minute boundary.
    pub fn until_next_change() -> std::time::Duration {
        use chrono::Timelike;
        let now = Local::now();
        let fmt = clock_format();
        let has_seconds = ["%S", "%T", "%X", "%r", "%s"]
            .iter()
            .any(|s| fmt.contains(s));
        // nanosecond() can exceed 1e9 during a leap second — clamp via modulo.
        let to_next_second = 1_000_000_000 - (now.nanosecond() % 1_000_000_000) as u64;
        let ns = if has_seconds {
            to_next_second
        } else {
            (59 - now.second().min(59)) as u64 * 1_000_000_000 + to_next_second
        };
        // +25ms so the wake lands safely past the boundary.
        std::time::Duration::from_nanos(ns) + std::time::Duration::from_millis(25)
    }
}
