//! When a client paints the panel: the frame-callback bound and the minute
//! clock the greeter, the lock screen and otto-authorize all pace by.

use std::time::{Duration, Instant};

/// How long a painted frame is given to reach the screen before painting
/// again. Frames are paced by the compositor's frame callbacks — that is what
/// keeps an animating panel from painting faster than anyone can see — and
/// this is the bound on trusting it to send them: a login or lock screen is
/// not something anyone can close and reopen.
pub const FRAME_TIMEOUT: Duration = Duration::from_millis(100);

/// Whether the last painted frame is still on its way to the screen, in which
/// case there is nothing to gain by painting another one yet.
///
/// `painted_at` is when it was painted, and `pending` whether any of the
/// client's surfaces is still waiting on its frame callback. Only until
/// [`FRAME_TIMEOUT`]: a compositor that stops answering with frame callbacks
/// must not be able to freeze the panel.
pub fn frame_in_flight(painted_at: Option<Instant>, pending: impl FnOnce() -> bool) -> bool {
    painted_at.is_some_and(|at| at.elapsed() < FRAME_TIMEOUT) && pending()
}

/// The minute the panel's clock was last drawn showing.
///
/// The clock draws from a closure the engine records once and replays, so a
/// screen left up overnight keeps the time it appeared at unless the client
/// notices the minute turn and asks for it again.
#[derive(Debug, Default)]
pub struct Clock {
    shown: Option<i64>,
}

impl Clock {
    fn minute() -> i64 {
        chrono::Local::now().timestamp() / 60
    }

    /// Whether the minute has turned since the clock was last drawn.
    pub fn stale(&self) -> bool {
        self.shown != Some(Self::minute())
    }

    /// Note that the clock is being drawn now. Returns whether the minute had
    /// turned, and so whether the panel's clock needs refreshing.
    pub fn catch_up(&mut self) -> bool {
        let minute = Self::minute();
        let turned = self.shown != Some(minute);
        self.shown = Some(minute);
        turned
    }

    /// How long until the minute turns: all a clock needs the loop awake for.
    pub fn until_next_minute() -> Duration {
        Duration::from_secs(60 - (chrono::Local::now().timestamp() % 60).unsigned_abs())
    }
}
