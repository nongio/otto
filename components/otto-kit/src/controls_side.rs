//! Which end of the titlebar the window controls sit at.
//!
//! Travels the same way corner rounding does — see [`crate::corners`]: the
//! compositor reads the configuration, publishes the answer in the
//! environment, and everything that draws a titlebar reads it back from here.

use std::sync::atomic::{AtomicU8, Ordering};

/// The variable the compositor publishes.
pub const ENV: &str = "OTTO_WINDOW_CONTROLS_SIDE";

/// Which end of the bar the traffic lights are drawn at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ControlsSide {
    /// Leading edge, closest to the screen's left. Otto's default.
    #[default]
    Left,
    /// Trailing edge. The dots also swap order, so close stays outermost.
    Right,
}

impl ControlsSide {
    /// The configuration token, and what goes on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            ControlsSide::Left => "left",
            ControlsSide::Right => "right",
        }
    }

    /// Parse a configuration token, case-insensitively. Anything else is
    /// `None`, and the caller keeps the default rather than guessing.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "left" => Some(ControlsSide::Left),
            "right" => Some(ControlsSide::Right),
            _ => None,
        }
    }
}

/// 0 = not resolved yet, 1 = left, 2 = right.
static SIDE: AtomicU8 = AtomicU8::new(0);

/// The side the desktop puts its window controls on.
pub fn side() -> ControlsSide {
    match SIDE.load(Ordering::Relaxed) {
        0 => {
            let value = std::env::var(ENV)
                .ok()
                .and_then(|text| ControlsSide::parse(&text))
                .unwrap_or_default();
            store(value);
            value
        }
        2 => ControlsSide::Right,
        _ => ControlsSide::Left,
    }
}

fn store(value: ControlsSide) {
    SIDE.store(
        match value {
            ControlsSide::Left => 1,
            ControlsSide::Right => 2,
        },
        Ordering::Relaxed,
    );
}

/// Publish `value` to this process and everything it starts, and return the
/// assignment for the session's activation environments. See
/// [`crate::corners::export`].
pub fn export(value: ControlsSide) -> String {
    store(value);
    std::env::set_var(ENV, value.as_str());
    format!("{ENV}={}", value.as_str())
}
