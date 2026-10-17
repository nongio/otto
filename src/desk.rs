//! The desk process: `otto-files --desk`, run and supervised by the compositor.
//!
//! Whether it runs is the `desk.enabled` setting (`specs/desk.md`,
//! *Lifecycle*). The compositor spawns it with the session's environment,
//! watches it through a pidfd on the event loop, and restarts it with a
//! back-off if it dies while the setting is on. Switching the setting off
//! terminates it.

// Rust guideline compliant 2026-02-21

use std::os::fd::{FromRawFd, OwnedFd};
use std::process::Child;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::{
    generic::Generic,
    timer::{TimeoutAction, Timer},
    Interest, Mode, PostAction, RegistrationToken,
};
use tracing::{info, warn};

use crate::config::Config;
use crate::state::{Backend, Otto};

/// The program that draws the desk, looked up on `PATH` like the other
/// session helpers (`otto-bar`, `otto-islands`).
const DESK_PROGRAM: &str = "otto-files";
/// The argument that makes `otto-files` run as the desk rather than a browser.
const DESK_ARG: &str = "--desk";

/// The wait before the first restart after a crash.
const FIRST_RESTART_DELAY: Duration = Duration::from_secs(1);
/// The longest wait between restarts. A desk that crashes on every start is
/// retried once a minute rather than given up on, so a fix that lands (a
/// config file repaired, a package upgraded) is picked up without a toggle.
const MAX_RESTART_DELAY: Duration = Duration::from_secs(60);
/// How long the desk has to stay up for its next crash to count as the first
/// one again. Long enough that a crash during startup never qualifies.
const STABLE_UPTIME: Duration = Duration::from_secs(30);

/// The compositor's hold on the desk process.
#[derive(Debug, Default)]
pub struct Desk {
    running: Option<Running>,
    /// A desk sent `SIGTERM` that has not exited yet. A new desk waits for it,
    /// since a second desk in the session exits at once.
    stopping: Option<Running>,
    /// The pending restart, while the desk waits out its back-off.
    restart: Option<RegistrationToken>,
    /// Crashes in a row, each one within [`STABLE_UPTIME`] of its start.
    failures: u32,
}

#[derive(Debug)]
struct Running {
    child: Child,
    started: Instant,
    /// The event source that fires when the process exits.
    watch: Option<RegistrationToken>,
}

/// How long to wait before restarting after the `failures`-th crash in a row.
///
/// Doubles from [`FIRST_RESTART_DELAY`] and stops at [`MAX_RESTART_DELAY`].
pub fn restart_delay(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    (FIRST_RESTART_DELAY * (1u32 << doublings)).min(MAX_RESTART_DELAY)
}

/// The crash count after a crash that came `uptime` after the start.
///
/// A desk that stayed up for [`STABLE_UPTIME`] starts counting again, so an
/// occasional crash is always retried quickly.
pub fn failures_after_crash(failures: u32, uptime: Duration) -> u32 {
    if uptime >= STABLE_UPTIME {
        1
    } else {
        failures.saturating_add(1)
    }
}

/// A pidfd for `child`, readable once the process has exited.
pub(crate) fn pidfd(child: &Child) -> std::io::Result<OwnedFd> {
    let pid = libc::pid_t::try_from(child.id())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: `pidfd_open` takes a pid and a flags word and returns a new file
    // descriptor or -1. The pid is our own unreaped child, so it cannot have
    // been recycled for another process.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd =
        i32::try_from(fd).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
    // SAFETY: the syscall succeeded, so `fd` is a fresh descriptor that
    // nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Whether the desk should be running in this session.
fn desk_wanted() -> bool {
    Config::with(|c| c.desk.enabled) && !crate::login::is_login_mode()
}

impl<B: Backend + 'static> Otto<B> {
    /// Start or stop the desk to match `desk.enabled`.
    ///
    /// Called at startup and whenever the setting changes. Switching it on
    /// forgets any earlier crashes, so the desk starts at once.
    pub fn apply_desk_setting(&mut self) {
        if !desk_wanted() {
            self.stop_desk();
            return;
        }
        if self.desk.running.is_some() {
            return;
        }
        if let Some(token) = self.desk.restart.take() {
            self.handle.remove(token);
        }
        self.desk.failures = 0;
        self.start_desk();
    }

    /// Spawn the desk, unless one is running or still on its way out; the
    /// latter starts it once it has exited.
    fn start_desk(&mut self) {
        if self.desk.running.is_some() || self.desk.stopping.is_some() {
            return;
        }
        let args = [DESK_ARG.to_string()];
        let child = match self.program_command(DESK_PROGRAM, &args).spawn() {
            Ok(child) => child,
            // Not installed: retrying cannot help until the setting is
            // switched on again.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                warn!(program = DESK_PROGRAM, "the desk is not installed");
                return;
            }
            Err(err) => {
                warn!(program = DESK_PROGRAM, %err, "could not start the desk");
                self.desk.failures = self.desk.failures.saturating_add(1);
                self.schedule_desk_restart();
                return;
            }
        };
        info!(pid = child.id(), "desk started");

        let pid = child.id();
        let watch = match pidfd(&child) {
            Ok(fd) => self
                .handle
                .insert_source(
                    Generic::new(fd, Interest::READ, Mode::Level),
                    move |_, _, state| Ok(state.desk_exited(pid)),
                )
                .map_err(|err| warn!(err = %err.error, "could not watch the desk"))
                .ok(),
            // Without a watch a crash goes unnoticed until the setting is
            // switched off, which still reaps the process.
            Err(err) => {
                warn!(%err, "could not watch the desk; it will not be restarted");
                None
            }
        };

        self.desk.running = Some(Running {
            child,
            started: Instant::now(),
            watch,
        });
    }

    /// The desk process `pid` has exited: reap it and, if that was not asked
    /// for, schedule a restart.
    fn desk_exited(&mut self, pid: u32) -> PostAction {
        if self
            .desk
            .stopping
            .as_ref()
            .is_some_and(|s| s.child.id() == pid)
        {
            return self.stopped_desk_exited();
        }
        let Some(running) = self.desk.running.as_mut().filter(|r| r.child.id() == pid) else {
            return PostAction::Remove;
        };
        let status = match running.child.try_wait() {
            Ok(Some(status)) => status,
            Ok(None) => return PostAction::Continue,
            Err(err) => {
                warn!(%err, "could not reap the desk");
                return PostAction::Continue;
            }
        };
        let uptime = running.started.elapsed();
        self.desk.running = None;

        // A clean exit is the desk's own decision, such as finding another
        // desk already running in this session. Starting it again would only
        // repeat that.
        if status.success() {
            info!("desk exited");
            return PostAction::Remove;
        }

        self.desk.failures = failures_after_crash(self.desk.failures, uptime);
        warn!(
            %status,
            uptime_ms = uptime.as_millis() as u64,
            failures = self.desk.failures,
            "desk exited unexpectedly; restarting it"
        );
        self.schedule_desk_restart();
        PostAction::Remove
    }

    /// A desk that was asked to stop has exited: reap it, and start its
    /// successor if the setting was switched back on meanwhile.
    fn stopped_desk_exited(&mut self) -> PostAction {
        let Some(stopping) = self.desk.stopping.as_mut() else {
            return PostAction::Remove;
        };
        match stopping.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => return PostAction::Continue,
            Err(err) => warn!(%err, "could not reap the stopped desk"),
        }
        self.desk.stopping = None;
        if desk_wanted() && self.desk.restart.is_none() {
            self.start_desk();
        }
        PostAction::Remove
    }

    /// Start the desk again once the back-off for the current crash count
    /// has passed.
    fn schedule_desk_restart(&mut self) {
        if let Some(token) = self.desk.restart.take() {
            self.handle.remove(token);
        }
        let delay = restart_delay(self.desk.failures);
        let timer = Timer::from_duration(delay);
        match self.handle.insert_source(timer, |_, _, state| {
            state.desk.restart = None;
            if desk_wanted() {
                state.start_desk();
            }
            TimeoutAction::Drop
        }) {
            Ok(token) => self.desk.restart = Some(token),
            Err(err) => warn!(err = %err.error, "could not schedule a desk restart"),
        }
    }

    /// Terminate the desk and cancel any pending restart.
    fn stop_desk(&mut self) {
        if let Some(token) = self.desk.restart.take() {
            self.handle.remove(token);
        }
        self.desk.failures = 0;
        let Some(running) = self.desk.running.take() else {
            return;
        };
        let pid = running.child.id();
        info!(pid, "stopping the desk");
        if let Ok(pid) = libc::pid_t::try_from(pid) {
            // SAFETY: `kill` has no memory-safety preconditions. The pid is
            // our own child and has not been reaped, so it still names it.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        // The watch stays registered and reaps it; without one a thread does.
        if running.watch.is_some() {
            self.desk.stopping = Some(running);
        } else {
            crate::input::actions::reap_in_background(DESK_PROGRAM, running.child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_back_off_from_a_second_to_a_minute() {
        let delays: Vec<u64> = (1..=8).map(|n| restart_delay(n).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(restart_delay(u32::MAX), MAX_RESTART_DELAY);
        // No crash yet still waits the first delay rather than none at all.
        assert_eq!(restart_delay(0), FIRST_RESTART_DELAY);
    }

    #[test]
    fn a_crash_after_a_stable_run_counts_as_the_first() {
        assert_eq!(failures_after_crash(0, Duration::from_millis(200)), 1);
        assert_eq!(failures_after_crash(5, Duration::from_millis(200)), 6);
        assert_eq!(failures_after_crash(5, STABLE_UPTIME), 1);
        assert_eq!(failures_after_crash(u32::MAX, Duration::ZERO), u32::MAX);
    }
}
