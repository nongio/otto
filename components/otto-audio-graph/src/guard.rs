//! Not fighting another program over a link.
//!
//! The service puts back a saved link another program removed. When that
//! program keeps removing it, the two would trade the link back and forth
//! forever; the guard gives up on a link removed again after being restored
//! three times within two seconds.

// Rust guideline compliant 2026-02-21

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::time::{Duration, Instant};

/// How many restores within [`WINDOW`] are allowed before a further removal
/// means another program wants the link gone.
const LIMIT: usize = 3;

/// The span the restores are counted over. Long enough to catch a program
/// that removes the link as soon as it appears, short enough that unplugging
/// and plugging a device back now and then never trips it.
const WINDOW: Duration = Duration::from_secs(2);

/// What to do about a link that was removed from outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Put it back.
    Restore,
    /// Leave it removed. `first` is true the one time the guard gives up, so
    /// it is reported once.
    GiveUp { first: bool },
}

/// Counts restores per link and says when to stop.
#[derive(Debug, Clone)]
pub struct LoopGuard<K> {
    restores: HashMap<K, VecDeque<Instant>>,
    given_up: HashSet<K>,
}

impl<K> Default for LoopGuard<K> {
    fn default() -> Self {
        Self {
            restores: HashMap::new(),
            given_up: HashSet::new(),
        }
    }
}

impl<K: Hash + Eq + Clone> LoopGuard<K> {
    /// A guard that has seen nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `key` was removed by someone else at `now`: restore it, or give up.
    /// A restore is counted when this says [`Verdict::Restore`].
    pub fn removed(&mut self, key: &K, now: Instant) -> Verdict {
        if self.given_up.contains(key) {
            return Verdict::GiveUp { first: false };
        }
        let times = self.restores.entry(key.clone()).or_default();
        while times
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > WINDOW)
        {
            times.pop_front();
        }
        if times.len() >= LIMIT {
            self.restores.remove(key);
            self.given_up.insert(key.clone());
            return Verdict::GiveUp { first: true };
        }
        times.push_back(now);
        Verdict::Restore
    }

    /// Whether the guard has given up on `key`.
    #[must_use]
    pub fn has_given_up(&self, key: &K) -> bool {
        self.given_up.contains(key)
    }

    /// Forget everything, as when the saved routing changes.
    pub fn reset(&mut self) {
        self.restores.clear();
        self.given_up.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gives_up_on_the_fourth_removal_within_the_window() {
        let mut guard = LoopGuard::new();
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        assert_eq!(guard.removed(&1, at(0)), Verdict::Restore);
        assert_eq!(guard.removed(&1, at(300)), Verdict::Restore);
        assert_eq!(guard.removed(&1, at(600)), Verdict::Restore);
        assert_eq!(guard.removed(&1, at(900)), Verdict::GiveUp { first: true });
        assert_eq!(
            guard.removed(&1, at(5000)),
            Verdict::GiveUp { first: false }
        );
        assert!(guard.has_given_up(&1));
    }

    #[test]
    fn removals_spread_out_are_always_restored() {
        let mut guard = LoopGuard::new();
        let start = Instant::now();
        for i in 0..10 {
            let now = start + Duration::from_millis(1500 * i);
            assert_eq!(guard.removed(&1, now), Verdict::Restore);
        }
    }

    #[test]
    fn links_are_counted_apart() {
        let mut guard = LoopGuard::new();
        let now = Instant::now();
        for _ in 0..3 {
            guard.removed(&1, now);
        }
        assert_eq!(guard.removed(&2, now), Verdict::Restore);
    }

    #[test]
    fn reset_forgets_a_link_given_up_on() {
        let mut guard = LoopGuard::new();
        let now = Instant::now();
        for _ in 0..4 {
            guard.removed(&1, now);
        }
        guard.reset();
        assert_eq!(guard.removed(&1, now), Verdict::Restore);
    }
}
