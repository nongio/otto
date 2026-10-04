//! Logging out: closing every window the way its close button would, then
//! ending the session once they are gone.
//!
//! Asking rather than disconnecting is what gives an application the chance
//! to save. An editor holding unsaved work answers the close with a "save
//! changes?" dialog, a new window that was not there when the logout began:
//! that, or windows still open when the grace period runs out, means an
//! application wants the person, and the logout stands down and leaves the
//! session as it is. Logging out again once the prompt is answered finishes
//! the job.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::backend::ObjectId;
use tracing::{info, warn};

use crate::shell::WindowElement;

use super::{Backend, Otto};

/// How often the windows are counted while a logout waits for them.
const POLL: Duration = Duration::from_millis(250);
/// How long applications get to close before the logout gives up.
const GRACE: Duration = Duration::from_secs(10);

/// What a logout in progress decides on each count.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Every window has gone: end the session.
    Done,
    /// A window opened that was not there before: an application is asking
    /// something, most likely whether to save.
    Asked,
    /// The grace period is over and windows are still open.
    TimedOut,
    /// Windows are still closing.
    Waiting,
}

fn verdict<T: Eq + std::hash::Hash>(
    before: &HashSet<T>,
    now: &HashSet<T>,
    elapsed: Duration,
) -> Verdict {
    if now.is_empty() {
        Verdict::Done
    } else if now.iter().any(|id| !before.contains(id)) {
        Verdict::Asked
    } else if elapsed >= GRACE {
        Verdict::TimedOut
    } else {
        Verdict::Waiting
    }
}

/// Ask one window to close, as its close button does.
fn request_close(window: &WindowElement) {
    match window.underlying_surface() {
        smithay::desktop::WindowSurface::Wayland(toplevel) => toplevel.send_close(),
        #[cfg(feature = "xwayland")]
        smithay::desktop::WindowSurface::X11(surface) => {
            let _ = surface.close();
        }
    }
}

// ObjectId as key — see window_throttle.rs.
#[allow(clippy::mutable_key_type)]
impl<BackendData: Backend> Otto<BackendData> {
    fn window_ids(&self) -> HashSet<ObjectId> {
        self.workspaces.windows_map.keys().cloned().collect()
    }

    /// Log out: ask every window to close, and end the session when they all
    /// have. A second request while one is under way is ignored.
    pub fn begin_logout(&mut self) {
        if self.logout_pending {
            return;
        }
        let before = self.window_ids();
        if before.is_empty() {
            info!("Logging out: no windows open.");
            self.running.store(false, Ordering::SeqCst);
            return;
        }

        info!(
            windows = before.len(),
            "Logging out: asking windows to close."
        );
        for window in self.workspaces.windows_map.values() {
            request_close(window);
        }

        let started = Instant::now();
        let scheduled = self
            .handle
            .insert_source(Timer::from_duration(POLL), move |_, _, otto| {
                match verdict(&before, &otto.window_ids(), started.elapsed()) {
                    Verdict::Waiting => return TimeoutAction::ToDuration(POLL),
                    Verdict::Done => {
                        info!("Logging out: every window closed.");
                        otto.running.store(false, Ordering::SeqCst);
                    }
                    Verdict::Asked => {
                        info!("Logout cancelled: an application opened a window to ask something.");
                    }
                    Verdict::TimedOut => {
                        info!("Logout cancelled: windows still open after the grace period.");
                    }
                }
                otto.logout_pending = false;
                TimeoutAction::Drop
            });
        match scheduled {
            Ok(_) => self.logout_pending = true,
            Err(_) => warn!("Could not schedule the logout; the session stays."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[u32]) -> HashSet<u32> {
        ids.iter().copied().collect()
    }

    #[test]
    fn an_empty_session_is_done() {
        assert_eq!(
            verdict(&set(&[1]), &set(&[]), Duration::ZERO),
            Verdict::Done
        );
    }

    #[test]
    fn windows_still_closing_are_waited_for_until_the_grace_runs_out() {
        let before = set(&[1, 2]);
        assert_eq!(
            verdict(&before, &set(&[2]), Duration::from_secs(1)),
            Verdict::Waiting
        );
        assert_eq!(verdict(&before, &set(&[2]), GRACE), Verdict::TimedOut);
    }

    #[test]
    fn a_new_window_is_an_application_asking() {
        // An editor's "save changes?" dialog, opened in answer to the close.
        assert_eq!(
            verdict(&set(&[1]), &set(&[1, 3]), Duration::from_secs(1)),
            Verdict::Asked
        );
    }
}
