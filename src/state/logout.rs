//! Logging out: closing every window the way its close button would, then
//! ending the session once they are gone.
//!
//! Asking rather than disconnecting is what gives an application the chance
//! to save. An editor holding unsaved work answers the close with a "save
//! changes?" dialog, a new window that was not there when the logout began.
//! The logout waits for as long as such a window is open: the person is
//! deciding, and saving can take a file chooser and a while. When the last
//! prompt goes, the application either closes (saved, or discarded) and the
//! logout carries on, or it stays (cancelled), and once it has had a moment
//! to close and has not, the logout stands down and leaves the session as it
//! is. Windows that neither close nor ask, within the grace period, stand it
//! down too.

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
/// How long applications get to close before the logout gives up, while
/// none of them is asking anything.
const GRACE: Duration = Duration::from_secs(10);
/// How long an application gets to close after its last prompt went away.
/// Longer than that and the prompt was cancelled.
const AFTER_PROMPT: Duration = Duration::from_secs(3);

/// What a logout in progress decides on each count.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Every window has gone: end the session.
    Done,
    /// A window opened that was not there before: an application is asking
    /// something, most likely whether to save. Wait for the answer.
    Asking,
    /// The last prompt went away and its application stayed open: the
    /// person cancelled.
    Cancelled,
    /// The grace period is over and windows are still open.
    TimedOut,
    /// Windows are still closing.
    Waiting,
}

/// `since_prompt` is how long ago a prompt was last seen open, if one ever
/// was during this logout.
fn verdict<T: Eq + std::hash::Hash>(
    before: &HashSet<T>,
    now: &HashSet<T>,
    elapsed: Duration,
    since_prompt: Option<Duration>,
) -> Verdict {
    if now.is_empty() {
        Verdict::Done
    } else if now.iter().any(|id| !before.contains(id)) {
        Verdict::Asking
    } else if let Some(since) = since_prompt {
        if since >= AFTER_PROMPT {
            Verdict::Cancelled
        } else {
            Verdict::Waiting
        }
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
        let mut last_prompt: Option<Instant> = None;
        let scheduled = self
            .handle
            .insert_source(Timer::from_duration(POLL), move |_, _, otto| {
                let since_prompt = last_prompt.map(|at| at.elapsed());
                match verdict(&before, &otto.window_ids(), started.elapsed(), since_prompt) {
                    Verdict::Waiting => return TimeoutAction::ToDuration(POLL),
                    Verdict::Asking => {
                        if last_prompt.is_none() {
                            info!("Logging out: waiting for an application's question to be answered.");
                        }
                        last_prompt = Some(Instant::now());
                        return TimeoutAction::ToDuration(POLL);
                    }
                    Verdict::Done => {
                        info!("Logging out: every window closed.");
                        otto.running.store(false, Ordering::SeqCst);
                    }
                    Verdict::Cancelled => {
                        info!("Logout cancelled: an application stayed open after its question.");
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

    const SOON: Duration = Duration::from_secs(1);

    #[test]
    fn an_empty_session_is_done() {
        assert_eq!(
            verdict(&set(&[1]), &set(&[]), Duration::ZERO, None),
            Verdict::Done
        );
    }

    #[test]
    fn windows_still_closing_are_waited_for_until_the_grace_runs_out() {
        let before = set(&[1, 2]);
        assert_eq!(verdict(&before, &set(&[2]), SOON, None), Verdict::Waiting);
        assert_eq!(verdict(&before, &set(&[2]), GRACE, None), Verdict::TimedOut);
    }

    #[test]
    fn an_open_prompt_is_waited_for_however_long_it_takes() {
        // An editor's "save changes?" dialog, opened in answer to the close.
        let before = set(&[1]);
        let asking = set(&[1, 3]);
        assert_eq!(verdict(&before, &asking, SOON, None), Verdict::Asking);
        assert_eq!(
            verdict(&before, &asking, GRACE * 6, Some(Duration::ZERO)),
            Verdict::Asking
        );
    }

    #[test]
    fn after_the_prompt_the_application_closes_or_the_logout_stands_down() {
        let before = set(&[1]);
        // Saved: the editor is closing, given a moment, then gone.
        assert_eq!(
            verdict(&before, &set(&[1]), GRACE * 2, Some(SOON)),
            Verdict::Waiting
        );
        assert_eq!(
            verdict(&before, &set(&[]), GRACE * 2, Some(SOON)),
            Verdict::Done
        );
        // Cancelled: the editor is still there once the moment has passed,
        // however long ago the logout began.
        assert_eq!(
            verdict(&before, &set(&[1]), GRACE * 2, Some(AFTER_PROMPT)),
            Verdict::Cancelled
        );
    }
}
