//! The session's polkit authentication agent: `otto-authorize
//! --polkit-agent`, started and supervised by the compositor.
//!
//! polkit asks the agent registered for a session whenever a program in it
//! needs a password for something — `pkexec`, a system service's action.
//! With none registered every such request fails (pkexec falls back to a
//! terminal prompt, which a desktop program does not have). Otto's agent
//! shows the same password panel as the lock screen; see
//! `components/otto-authorize/src/polkit/`.
//!
//! It is connected as a Wayland client on a socketpair (`WAYLAND_SOCKET`)
//! marked [`OttoComponent::Authorize`], so its panel holds the keyboard
//! against any other overlay while it is up.
//!
//! One per session, started at startup (not in login mode) unless
//! `polkit_agent = false`, and started again with a back-off if it dies. A
//! clean exit is the agent's own decision — polkit already has an agent for
//! this session — and is not retried.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::Child;
use std::time::Instant;

use smithay::reexports::calloop::{
    generic::Generic,
    timer::{TimeoutAction, Timer},
    Interest, Mode, PostAction, RegistrationToken,
};
use tracing::{info, warn};

use crate::config::Config;
use crate::desk::{failures_after_crash, restart_delay};
use crate::state::{Backend, Otto, OttoComponent};

/// The agent's program.
pub const AGENT: &str = "otto-authorize";

/// The argument that makes otto-authorize the polkit agent.
pub const AGENT_ARG: &str = "--polkit-agent";

/// The agent's program to run: in a `dev` build the one beside the running
/// compositor when there is one, so an uninstalled build runs its own;
/// otherwise found through `$PATH`.
fn agent_program() -> String {
    if cfg!(feature = "dev") {
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(AGENT)))
            .filter(|path| path.is_file());
        if let Some(path) = beside {
            return path.to_string_lossy().into_owned();
        }
    }
    AGENT.to_string()
}

/// The compositor's hold on the agent process.
#[derive(Debug, Default)]
pub struct PolkitAgent {
    running: Option<Running>,
    /// The pending restart, while the agent waits out its back-off.
    restart: Option<RegistrationToken>,
    /// Crashes in a row, each soon after its start.
    failures: u32,
}

#[derive(Debug)]
struct Running {
    child: Child,
    started: Instant,
}

/// Whether this session should have Otto's agent.
fn agent_wanted() -> bool {
    Config::with(|c| c.polkit_agent) && !crate::login::is_login_mode()
}

/// A pidfd for `child`, readable once the process has exited.
fn pidfd(child: &Child) -> std::io::Result<OwnedFd> {
    let pid = libc::pid_t::try_from(child.id())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: `pidfd_open` takes a pid and a flags word and returns a new
    // descriptor or -1. The pid is our own unreaped child.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd =
        i32::try_from(fd).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
    // SAFETY: a fresh descriptor nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl<B: Backend + 'static> Otto<B> {
    /// Start the agent, unless it is running or not wanted.
    pub fn start_polkit_agent(&mut self) {
        if !agent_wanted() || self.polkit_agent.running.is_some() {
            return;
        }
        let helper = agent_program();
        let (client, theirs) = match self.connect_component_client(OttoComponent::Authorize) {
            Ok(pair) => pair,
            Err(err) => {
                warn!(%err, "cannot connect the polkit agent");
                return;
            }
        };
        let socket_fd = theirs.as_raw_fd();
        let mut command = self.program_command(&helper, &[AGENT_ARG.to_string()]);
        command
            // One connection, the one handed over.
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env("WAYLAND_SOCKET", socket_fd.to_string());
        // SAFETY: only fcntl between fork and exec.
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(move || crate::lock::inherit(socket_fd));
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                warn!(%err, "cannot start the polkit agent");
                self.display_handle.backend_handle().kill_client(
                    client.id(),
                    smithay::reexports::wayland_server::backend::DisconnectReason::ConnectionClosed,
                );
                self.polkit_agent.failures = self.polkit_agent.failures.saturating_add(1);
                self.schedule_polkit_agent_restart();
                return;
            }
        };
        // The child has its copy.
        drop(theirs);
        let pid = child.id();
        info!(pid, "polkit agent started");

        match pidfd(&child) {
            Ok(fd) => {
                if let Err(err) = self.handle.insert_source(
                    Generic::new(fd, Interest::READ, Mode::Level),
                    move |_, _, state| Ok(state.polkit_agent_exited(pid)),
                ) {
                    warn!(err = %err.error, "cannot watch the polkit agent; it will not be restarted");
                }
            }
            Err(err) => warn!(%err, "cannot watch the polkit agent; it will not be restarted"),
        }
        self.polkit_agent.running = Some(Running {
            child,
            started: Instant::now(),
        });
    }

    /// The agent `pid` has exited: reap it and, unless it chose to, start it
    /// again after a back-off.
    fn polkit_agent_exited(&mut self, pid: u32) -> PostAction {
        let Some(running) = self
            .polkit_agent
            .running
            .as_mut()
            .filter(|r| r.child.id() == pid)
        else {
            return PostAction::Remove;
        };
        let status = match running.child.try_wait() {
            Ok(Some(status)) => status,
            Ok(None) => return PostAction::Continue,
            Err(err) => {
                warn!(%err, "cannot reap the polkit agent");
                return PostAction::Continue;
            }
        };
        let uptime = running.started.elapsed();
        self.polkit_agent.running = None;

        if status.success() {
            info!("polkit agent exited; polkit has another agent for this session");
            return PostAction::Remove;
        }
        self.polkit_agent.failures = failures_after_crash(self.polkit_agent.failures, uptime);
        warn!(
            %status,
            failures = self.polkit_agent.failures,
            "polkit agent exited unexpectedly; restarting it"
        );
        self.schedule_polkit_agent_restart();
        PostAction::Remove
    }

    fn schedule_polkit_agent_restart(&mut self) {
        if let Some(token) = self.polkit_agent.restart.take() {
            self.handle.remove(token);
        }
        let delay = restart_delay(self.polkit_agent.failures);
        match self
            .handle
            .insert_source(Timer::from_duration(delay), |_, _, state| {
                state.polkit_agent.restart = None;
                state.start_polkit_agent();
                TimeoutAction::Drop
            }) {
            Ok(token) => self.polkit_agent.restart = Some(token),
            Err(err) => warn!(err = %err.error, "cannot schedule a polkit agent restart"),
        }
    }
}
