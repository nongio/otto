//! Key repeat, timed on the client.
//!
//! Wayland leaves repeating a held key to the client: the compositor only says
//! how fast (`wl_keyboard.repeat_info`, the delay and rate chosen in Settings)
//! and reports the press and the release. The runner's loop is a plain `poll`,
//! so the held key is kept here and the loop wakes on its next deadline.

use std::time::{Duration, Instant};

use smithay_client_toolkit::seat::keyboard::{KeyEvent, RepeatInfo};

/// Used until the compositor says otherwise — the protocol's own suggestion
/// for a seat that never sends `repeat_info`.
const DEFAULT_DELAY: Duration = Duration::from_millis(600);
const DEFAULT_RATE: u32 = 25;

struct Held {
    event: KeyEvent,
    serial: u32,
    next: Instant,
}

pub(super) struct KeyRepeat {
    /// `None` when the compositor turned repeat off.
    timing: Option<(Duration, Duration)>,
    held: Option<Held>,
}

impl Default for KeyRepeat {
    fn default() -> Self {
        Self {
            timing: Some((DEFAULT_DELAY, interval(DEFAULT_RATE))),
            held: None,
        }
    }
}

fn interval(rate: u32) -> Duration {
    Duration::from_micros(1_000_000 / u64::from(rate.max(1)))
}

impl KeyRepeat {
    pub(super) fn set_info(&mut self, info: RepeatInfo) {
        self.timing = match info {
            RepeatInfo::Repeat { rate, delay } => Some((
                Duration::from_millis(u64::from(delay)),
                interval(rate.get()),
            )),
            RepeatInfo::Disable => None,
        };
        // A key already held picks the new timing up on its next repeat; with
        // repeat off it simply stops.
        if self.timing.is_none() {
            self.held = None;
        }
    }

    /// A key went down. It replaces whatever was repeating, as a second key
    /// does on any keyboard; modifiers never repeat.
    pub(super) fn press(&mut self, event: &KeyEvent, serial: u32, now: Instant) {
        let Some((delay, _)) = self.timing else {
            return;
        };
        if event.keysym.is_modifier_key() {
            return;
        }
        self.held = Some(Held {
            event: event.clone(),
            serial,
            next: now + delay,
        });
    }

    pub(super) fn release(&mut self, raw_code: u32) {
        if self
            .held
            .as_ref()
            .is_some_and(|h| h.event.raw_code == raw_code)
        {
            self.held = None;
        }
    }

    /// The keyboard left: the release will go to someone else.
    pub(super) fn cancel(&mut self) {
        self.held = None;
    }

    /// How long the loop may sleep before the next repeat is due.
    pub(super) fn timeout(&self, now: Instant) -> Option<Duration> {
        self.held
            .as_ref()
            .map(|h| h.next.saturating_duration_since(now))
    }

    /// The repeat due now, if any. At most one per call: a loop that fell
    /// behind resumes the cadence rather than delivering a burst.
    pub(super) fn due(&mut self, now: Instant) -> Option<(KeyEvent, u32)> {
        let (_, interval) = self.timing?;
        let held = self.held.as_mut()?;
        if now < held.next {
            return None;
        }
        held.next += interval;
        if held.next <= now {
            held.next = now + interval;
        }
        Some((held.event.clone(), held.serial))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay_client_toolkit::seat::keyboard::Keysym;
    use std::num::NonZeroU32;

    fn key(raw_code: u32, keysym: Keysym) -> KeyEvent {
        KeyEvent {
            time: 0,
            raw_code,
            keysym,
            utf8: None,
        }
    }

    fn repeat(rate: u32, delay: u32) -> KeyRepeat {
        let mut r = KeyRepeat::default();
        r.set_info(RepeatInfo::Repeat {
            rate: NonZeroU32::new(rate).unwrap(),
            delay,
        });
        r
    }

    #[test]
    fn repeats_after_the_delay_at_the_rate() {
        let mut r = repeat(20, 300);
        let t0 = Instant::now();
        r.press(&key(14, Keysym::BackSpace), 7, t0);

        assert!(r.due(t0 + Duration::from_millis(299)).is_none());
        let (event, serial) = r.due(t0 + Duration::from_millis(300)).unwrap();
        assert_eq!((event.raw_code, serial), (14, 7));
        assert!(r.due(t0 + Duration::from_millis(349)).is_none());
        assert!(r.due(t0 + Duration::from_millis(350)).is_some());
        assert_eq!(
            r.timeout(t0 + Duration::from_millis(360)),
            Some(Duration::from_millis(40))
        );
    }

    #[test]
    fn release_and_leave_stop_it() {
        let mut r = repeat(20, 300);
        let t0 = Instant::now();
        r.press(&key(14, Keysym::BackSpace), 1, t0);
        r.release(30);
        assert!(r.timeout(t0).is_some(), "another key's release is ignored");
        r.release(14);
        assert!(r.due(t0 + Duration::from_secs(1)).is_none());

        r.press(&key(14, Keysym::BackSpace), 1, t0);
        r.cancel();
        assert!(r.timeout(t0).is_none());
    }

    #[test]
    fn modifiers_and_disabled_repeat_never_start() {
        let mut r = repeat(20, 300);
        let t0 = Instant::now();
        r.press(&key(42, Keysym::Shift_L), 1, t0);
        assert!(r.timeout(t0).is_none());

        r.set_info(RepeatInfo::Disable);
        r.press(&key(14, Keysym::BackSpace), 1, t0);
        assert!(r.timeout(t0).is_none());
    }

    #[test]
    fn a_stalled_loop_does_not_burst() {
        let mut r = repeat(50, 100);
        let t0 = Instant::now();
        r.press(&key(14, Keysym::BackSpace), 1, t0);
        let late = t0 + Duration::from_secs(2);
        assert!(r.due(late).is_some());
        assert!(r.due(late).is_none());
    }
}
