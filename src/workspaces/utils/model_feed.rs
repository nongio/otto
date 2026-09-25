//! Tracks how far a view that follows the workspace model off-thread has got.
//!
//! The dock and the app switcher don't redraw when the model changes: the
//! compositor posts each change to a channel, and a task picks up the newest
//! one on a timer and re-renders from it. The scene therefore changes some
//! time after the model did, from outside the compositor loop. Each change
//! posted gets a sequence number, and the task records the one it last
//! rendered, so the gap between the two says whether a redraw is still owed.

// Rust guideline compliant 2026-02-21

use std::sync::atomic::{AtomicU64, Ordering};

/// Sequence numbers of the model changes posted to a view and rendered by it.
#[derive(Debug, Default)]
pub struct ModelFeed {
    posted: AtomicU64,
    rendered: AtomicU64,
}

impl ModelFeed {
    /// The sequence number the next posted change will carry.
    pub fn next(&self) -> u64 {
        self.posted.load(Ordering::Acquire) + 1
    }

    /// Records that the change numbered `seq` reached the view's channel.
    ///
    /// A change the channel refused is not recorded: the view will never
    /// see it, so it owes no redraw for it.
    pub fn posted(&self, seq: u64) {
        self.posted.fetch_max(seq, Ordering::AcqRel);
    }

    /// Records that the view has rendered the change numbered `seq`.
    pub fn rendered(&self, seq: u64) {
        self.rendered.fetch_max(seq, Ordering::AcqRel);
    }

    /// Whether a posted change has not been rendered yet.
    pub fn is_behind(&self) -> bool {
        self.rendered.load(Ordering::Acquire) < self.posted.load(Ordering::Acquire)
    }
}
