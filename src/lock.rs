//! Session locking (`ext-session-lock-v1`).
//!
//! Locking hides the running session behind an opaque surface on every output
//! and routes all input to the locking client, which authenticates the user and
//! then asks for the session back. The session itself keeps running: windows,
//! workspaces and focus are exactly as they were.
//!
//! What makes this protocol worth implementing rather than reusing an overlay
//! layer surface is the failure mode. "Locked" is compositor state, not the
//! client's presence — a locker that crashes leaves the screen blank and the
//! session unreachable, where a layer surface would simply vanish and expose
//! the desktop. Every path in here that can lose the client therefore leaves
//! the lock standing; see [`Otto::lock_surfaces_pruned`].
//!
//! Locking is also the compositor's before it is the client's. The blank goes
//! up the moment a lock is asked for — shortcut, idle timer, lid, logind, a
//! suspend — and the locker is started into it: a locker that is slow to start,
//! fails to start, or dies is replaced, and the screen stays blank throughout.
//! A suspend waits for the blank to reach the screen, on a logind `delay`
//! inhibitor, so the machine never wakes to the desktop.
//!
//! Otto performs no authentication. The locker (`otto-lock` by default) runs as
//! the session user and talks to PAM itself, exactly as the greeter talks to
//! greetd. See `specs/lock-screen.md`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use layers::prelude::*;
use layers::types::Size;
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::{IsAlive, SERIAL_COUNTER};
use smithay::wayland::session_lock::{LockSurface, SessionLocker};
use tracing::{debug, error, info, warn};

use crate::state::{Backend, ClientState, Otto, OttoComponent};

/// How long an output has to present the blank before it is no longer waited
/// for.
///
/// Waiting for ever would leave a session that stays hidden with no locker
/// able to authenticate — the client is waiting on `locked`, which is waiting
/// on a frame that is not coming — and no way out but a VT switch, which a
/// tablet or a closed lid may not offer. Giving up on the lock instead would
/// uncover every other output because one is stuck. So the stalled output
/// keeps its blank and is dropped from the outputs `locked` waits for.
const LOCK_CONFIRM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How often a standing lock is looked after: a dead locker replaced, a
/// stalled output given up on, a held suspend let go.
const LOCK_WATCHDOG_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// How soon a locker that is gone may be started again, so one that crashes on
/// startup cannot spin.
const RESPAWN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// The longest a suspend is held for the blank. logind's own ceiling
/// (`InhibitDelayMaxSec`) defaults to 5 s, after which it suspends anyway.
const SLEEP_HOLD_MAX: std::time::Duration = std::time::Duration::from_secs(4);

/// How long the blank takes to come down from the top of the screen, and how
/// much it bounces when it lands.
///
/// The session is visible underneath while it falls, which is what a shade
/// coming down is — but nothing of the session is reachable: input is cut off
/// the moment the lock is requested, and the client is not told the session is
/// hidden until the blank has landed and been presented.
const SLIDE: f32 = 0.45;
const SLIDE_BOUNCE: f32 = 0.3;

/// How long the blank takes to go back up on unlock.
///
/// No spring on the way out: a bounce here would drop the shade back over a
/// session the user has already been given back. It accelerates away instead,
/// and the session is interactive from the moment the unlock is accepted —
/// the shade rising is a curtain, not a modal.
const SLIDE_OUT: f32 = 0.4;

/// Extra height the blank carries above the screen, as a fraction of it.
///
/// A spring overshoots and rebounds, and a shade that rebounded past its
/// resting place would lift a strip of itself off the top of the screen and
/// show the desktop through the gap — after landing, which is exactly when the
/// session is supposed to be hidden. The blank is therefore taller than the
/// output it covers, and rests with the excess off-screen above, so the whole
/// rebound happens in slack rather than in view.
const SLIDE_OVERSHOOT: f32 = 0.25;

/// Where the session sits between unlocked and locked.
pub enum LockState {
    /// Ordinary operation.
    Unlocked,
    /// A lock has been requested and the blank is up, but the client has not
    /// been told yet. `locked` is only sent once a frame carrying the blank has
    /// been presented on every output — until then the desktop may still be on
    /// screen, and a client that believed otherwise would be showing a lock
    /// screen over a visible session.
    ///
    /// Dropping the [`SessionLocker`] instead of calling `lock()` on it sends
    /// `finished`, which is how a refused lock is reported.
    Locking {
        /// The locker's request, once one has made it. The blank goes up when
        /// the lock is asked for and the locker is started after, so this is
        /// `None` until the locker connects and asks. A locker that asked and
        /// then died is replaced here by the next one Otto starts.
        locker: Option<SessionLocker>,
        /// Outputs that have yet to present the blank.
        pending: HashSet<String>,
        /// When the lock was requested, so a confirmation that never comes
        /// can be given up on. See [`LOCK_CONFIRM_TIMEOUT`].
        since: std::time::Instant,
        /// When the blank finishes falling. No output counts as blanked before
        /// this: a frame presented mid-slide still has the desktop under it,
        /// and `locked` is a promise that it does not.
        landed: std::time::Instant,
    },
    /// The client has been told the session is locked.
    Locked {
        /// The lock object of the locker that was told. Once it is dead — the
        /// locker crashed or was killed — another locker Otto started may take
        /// the lock over; while it lives, nobody else can.
        owner: ExtSessionLockV1,
    },
}

impl LockState {
    /// Whether the session is locked or on its way there. Input gating uses
    /// this rather than [`LockState::Locked`]: the window between the request
    /// and the confirmation is exactly when the desktop must stop reacting.
    pub fn is_active(&self) -> bool {
        !matches!(self, LockState::Unlocked)
    }

    /// Whether the locker holding, or asking for, the lock is still there.
    fn owner_alive(&self) -> bool {
        match self {
            LockState::Locking {
                locker: Some(locker),
                ..
            } => locker.ext_session_lock().is_alive(),
            LockState::Locked { owner } => owner.is_alive(),
            _ => false,
        }
    }

    /// Whether the blank has landed and been presented on every output that
    /// is waited for — the session is hidden, whether or not a locker has
    /// been told yet.
    pub fn blank_presented(&self) -> bool {
        match self {
            LockState::Locked { .. } => true,
            LockState::Locking {
                pending, landed, ..
            } => pending.is_empty() && std::time::Instant::now() >= *landed,
            LockState::Unlocked => false,
        }
    }
}

/// A locker's surface for one output, and the scene layer it draws into.
pub struct LockSurfaceEntry {
    pub surface: LockSurface,
    /// The client's layer, registered in `surface_layers` — so the commit path
    /// owns its size and position, and nothing here may hold state in it.
    pub layer: Layer,
    /// What holds [`LockSurfaceEntry::layer`], a child of that output's
    /// `lock_plane`, carrying the offset past the blank's slack. Removing this
    /// removes both.
    pub shade: Layer,
    pub output: Output,
}

/// The locker to launch, as `(command, args)`.
///
/// `$OTTO_LOCKER_COMMAND` overrides the configured command and is parsed as a
/// whitespace-separated argv, so an uninstalled build can be tested with
/// `OTTO_LOCKER_COMMAND=target/release/otto-lock`.
pub fn locker_command() -> (String, Vec<String>) {
    if let Ok(override_cmd) = std::env::var("OTTO_LOCKER_COMMAND") {
        let mut argv = override_cmd.split_whitespace().map(str::to_string);
        if let Some(cmd) = argv.next() {
            return (cmd, argv.collect());
        }
    }
    crate::config::Config::with(|c| (c.lock.locker_command.clone(), c.lock.locker_args.clone()))
}

/// Let `fd` survive into the child's `exec`. Called between fork and exec.
fn inherit(fd: i32) -> std::io::Result<()> {
    // SAFETY: fcntl on a descriptor this process holds open; async-signal-safe.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// Lock the session: blank every output now, and start the locker into
    /// the blank.
    ///
    /// Every way of locking comes through here — the shortcut, the idle
    /// timer, the lid, the power button, `loginctl lock-session`, a suspend.
    /// The blank does not wait for the locker: the desktop stops being
    /// reachable and starts going out of sight the moment the lock is asked
    /// for, and a locker that is slow to start, fails to start or dies only
    /// means a blank screen for a little longer — the watchdog keeps starting
    /// one (see [`Otto::respawn_locker_if_gone`]). Nothing here ever unlocks.
    pub fn lock_session(&mut self) {
        if !self.lock_state.is_active() {
            info!("Locking session");
            self.lock_locker_missing_reported = false;
            self.raise_blank();
        }
        if !self.locker_running() {
            self.spawn_locker();
        }
    }

    /// Start the configured locker, which asks for the lock itself.
    ///
    /// This is the only client that can lock: it is connected on a
    /// socketpair (`WAYLAND_SOCKET`) and marked as Otto's locker, and
    /// `ext_session_lock_manager_v1` is offered to no one else. Whatever
    /// holds the lock collects the password, so a program the user did not
    /// configure must not be able to put one up.
    fn spawn_locker(&mut self) {
        // Recorded whether or not the start works: a locker that cannot be
        // started is retried at the respawn interval, not on every tick.
        self.lock_last_spawn = Some(std::time::Instant::now());
        let (cmd, args) = locker_command();
        info!(locker = %cmd, "Starting the locker");
        let failure = match self.start_locker(Path::new(&cmd), &args) {
            Ok(()) => {
                self.lock_locker_missing_reported = false;
                return;
            }
            Err(err) => format!("Failed to start the locker {cmd}: {err}"),
        };
        // The screen stays blank and the watchdog keeps trying every
        // `RESPAWN_INTERVAL` — a locker installed meanwhile is picked up —
        // but the failure is an error once per lock, not once per retry.
        if self.lock_locker_missing_reported {
            debug!(locker = %cmd, "{failure}");
        } else {
            error!(
                locker = %cmd,
                "{failure}; the screen stays blank until a locker can be started"
            );
            self.lock_locker_missing_reported = true;
        }
    }

    fn start_locker(&mut self, program: &Path, args: &[String]) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;

        let (client, theirs) = self.connect_locker_client()?;
        let socket_fd = theirs.as_raw_fd();
        let mut command = self.program_command(&program.to_string_lossy(), args);
        // One connection, the one handed over. `WAYLAND_DISPLAY` would let a
        // locker that ignores `WAYLAND_SOCKET` connect as an ordinary client,
        // which cannot lock; better that it fails to connect at all.
        command
            .env_remove("WAYLAND_DISPLAY")
            .env("WAYLAND_SOCKET", socket_fd.to_string());
        // SAFETY: only fcntl between fork and exec.
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(move || inherit(socket_fd));
        }
        match command.spawn() {
            Ok(child) => {
                drop(theirs);
                crate::input::actions::reap_in_background(&program.to_string_lossy(), child);
                Ok(())
            }
            Err(err) => {
                self.lock_locker_client = None;
                self.display_handle.backend_handle().kill_client(
                    client.id(),
                    smithay::reexports::wayland_server::backend::DisconnectReason::ConnectionClosed,
                );
                Err(err)
            }
        }
    }

    /// Connect a client as Otto's locker, returning its end of the socket.
    ///
    /// [`Otto::spawn_locker`] hands that end to the locker it starts; tests
    /// connect a locker of their own the same way. Either way it is the
    /// locker from now on: while it is connected, no other is started.
    pub fn connect_locker_client(
        &mut self,
    ) -> std::io::Result<(Client, std::os::unix::net::UnixStream)> {
        let (client, theirs) = self.connect_component_client(OttoComponent::Locker)?;
        self.lock_locker_client = Some(client.clone());
        Ok((client, theirs))
    }

    /// Connect a client on a socketpair as one of Otto's own components,
    /// returning its end of the socket.
    pub fn connect_component_client(
        &mut self,
        component: OttoComponent,
    ) -> std::io::Result<(Client, std::os::unix::net::UnixStream)> {
        let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
        let client = self
            .display_handle
            .insert_client(
                ours,
                Arc::new(ClientState {
                    component: Some(component),
                    ..ClientState::default()
                }),
            )
            .map_err(std::io::Error::other)?;
        Ok((client, theirs))
    }

    /// Whether a locker is there to take, or holding, the lock: the one Otto
    /// last started is still connected, or the lock's owner is.
    fn locker_running(&self) -> bool {
        let started_alive = self.lock_locker_client.as_ref().is_some_and(|client| {
            self.display_handle
                .backend_handle()
                .get_client_data(client.id())
                .is_ok()
        });
        started_alive || self.lock_state.owner_alive()
    }

    /// Answer logind: lock when it asks, and hold a suspend until the lock
    /// is on screen.
    ///
    /// logind emits `Lock` on this session (`loginctl lock-session`, an idle
    /// daemon, a suspend hook) and whoever runs the session answers it. Otto
    /// answers by locking, which starts its locker. `Unlock` is not honoured —
    /// unlocking is the locker's, after the password.
    ///
    /// For suspend, Otto holds a `delay` inhibitor on `sleep`. When logind
    /// announces a suspend (`PrepareForSleep(true)`), the session is locked
    /// if the configuration says so (see [`lock_before_sleep`]) and the
    /// inhibitor let go once the blank has been presented — or after
    /// [`SLEEP_HOLD_MAX`], whichever is first — so the machine never goes to
    /// sleep, and wakes, showing the desktop. It is taken again on resume.
    ///
    /// Only on a real session (the udev backend): a nested Otto shares its
    /// host's logind session, and locking the host is the host's job.
    pub fn watch_logind(handle: &smithay::reexports::calloop::LoopHandle<'static, Self>) {
        use smithay::reexports::calloop::channel::{channel, Event as ChannelEvent};

        let (tx, rx) = channel::<LogindEvent>();
        if handle
            .insert_source(rx, |event, _, state| {
                let ChannelEvent::Msg(event) = event else {
                    return;
                };
                match event {
                    LogindEvent::Lock => {
                        if !state.is_session_locked() {
                            info!("logind asked for the session to be locked");
                            state.lock_session();
                        }
                    }
                    LogindEvent::PrepareForSleep(true) => state.prepare_for_sleep(),
                    LogindEvent::PrepareForSleep(false) => {
                        info!("resumed from sleep");
                        state.sleep_pending_since = None;
                    }
                    LogindEvent::SleepInhibitor(fd) => {
                        debug!("holding logind's sleep delay inhibitor");
                        state.sleep_inhibitor = Some(fd);
                    }
                }
            })
            .is_err()
        {
            warn!("could not listen to logind; `loginctl lock-session` will not lock");
            return;
        }

        let lock_tx = tx.clone();
        let spawned = std::thread::Builder::new()
            .name("logind-lock".into())
            .spawn(move || {
                if let Err(err) = listen_for_logind_lock(&lock_tx) {
                    warn!(%err, "not listening for logind's Lock; `loginctl lock-session` will not lock");
                }
            });
        if let Err(err) = spawned {
            warn!(%err, "could not start the logind Lock listener");
        }

        let spawned = std::thread::Builder::new()
            .name("logind-sleep".into())
            .spawn(move || {
                if let Err(err) = listen_for_sleep(&tx) {
                    warn!(%err, "not holding suspend for the lock; the machine may sleep before the screen is blank");
                }
            });
        if let Err(err) = spawned {
            warn!(%err, "could not start the logind sleep listener");
        }
    }

    /// logind is about to suspend: lock first if configured to, and let the
    /// suspend go once the blank is on screen.
    fn prepare_for_sleep(&mut self) {
        if lock_before_sleep() && !self.is_session_locked() {
            info!("suspending; locking first");
            self.lock_session();
        }
        if self.lock_state.is_active() && !self.lock_state.blank_presented() {
            // Held until the blank is presented — see `confirm_lock_if_blanked`
            // — or the watchdog gives up waiting.
            self.sleep_pending_since = Some(std::time::Instant::now());
            self.request_lock_redraw();
            return;
        }
        self.release_sleep_inhibitor();
    }

    /// Let a held suspend go ahead.
    fn release_sleep_inhibitor(&mut self) {
        self.sleep_pending_since = None;
        if self.sleep_inhibitor.take().is_some() {
            debug!("released logind's sleep delay inhibitor");
        }
    }

    /// Note that the user did something. Auto-lock measures from the last such
    /// moment, so every input path has to call this — a session that only ever
    /// sees mouse motion is not idle.
    pub fn note_input_activity(&mut self) {
        self.lock_last_activity = std::time::Instant::now();
    }

    /// Start the timer that locks the session after
    /// [`crate::config::LockConfig::auto_lock_timeout`] seconds without input.
    ///
    /// Nothing is scheduled when the setting is off, and each tick is scheduled
    /// from the time left rather than at a fixed rate, so an idle session wakes
    /// once per timeout instead of once a second.
    ///
    /// The returned token is the timer's event source. It is what makes the
    /// setting live: the interval is captured when the timer is armed, so
    /// following a changed timeout means dropping this source and arming a new
    /// one — see [`Otto::rearm_auto_lock_timer`].
    pub fn start_auto_lock_timer(
        handle: &smithay::reexports::calloop::LoopHandle<'static, Self>,
    ) -> Option<smithay::reexports::calloop::RegistrationToken> {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

        let timeout = crate::config::Config::with(|c| c.lock.auto_lock_timeout);
        if timeout == 0 {
            return None;
        }
        // In login mode the greeter *is* the screen: there is no session behind
        // it to hide, and the locker would authenticate a user who has not
        // logged in yet.
        if crate::login::is_login_mode() {
            return None;
        }
        let timeout = std::time::Duration::from_secs(timeout);

        match handle.insert_source(Timer::from_duration(timeout), move |_, _, data| {
            // A client asking not to be considered idle (a video playing)
            // both holds the lock off and restarts the clock, so the
            // countdown runs from when it stops, not from the last time
            // the user touched anything.
            if data.idle_inhibited() {
                debug!("Auto-lock held off by an idle inhibitor");
                data.note_input_activity();
                return TimeoutAction::ToDuration(timeout);
            }
            let idle_for = data.lock_last_activity.elapsed();
            let left = timeout.saturating_sub(idle_for);
            if left.is_zero() {
                data.auto_lock_now(timeout);
                return TimeoutAction::ToDuration(timeout);
            }
            TimeoutAction::ToDuration(left)
        }) {
            Ok(token) => {
                info!(idle_secs = timeout.as_secs(), "Auto-lock armed");
                Some(token)
            }
            Err(_) => {
                warn!("failed to schedule the auto-lock timer; auto-locking is off");
                None
            }
        }
    }

    /// Re-arm the auto-lock timer against the current configuration.
    ///
    /// Called when `lock.auto_lock_timeout` changes. A running timer keeps the
    /// interval it was armed with and would go on locking at the old one, so
    /// the old source is removed before a new one is armed — including when the
    /// new value is 0, which arms nothing and leaves auto-locking off. The
    /// idle clock is restarted too: the countdown a user has just changed
    /// should run from now rather than expire the moment they save it.
    pub fn rearm_auto_lock_timer(&mut self) {
        if let Some(token) = self.auto_lock_timer.take() {
            self.handle.remove(token);
        }
        self.note_input_activity();
        self.auto_lock_timer = Self::start_auto_lock_timer(&self.handle);
    }

    /// Whether a client is asking for the session not to be considered idle
    /// (`idle-inhibit-unstable-v1`) — a video playing, a presentation.
    ///
    /// An inhibitor only counts while its surface is on screen. The protocol
    /// leaves that to the compositor precisely because clients forget to drop
    /// them: a dead or minimized window holding one would block auto-lock for
    /// the rest of the session.
    ///
    /// On screen means a window that is not minimized, on the workspace an
    /// output is showing, or a mapped layer surface. A surface that is
    /// neither — never given a role, or a window on a workspace nobody is
    /// looking at — does not count: anything could otherwise keep the session
    /// from locking without showing anything.
    pub fn idle_inhibited(&self) -> bool {
        self.idle_inhibitors
            .iter()
            .any(|surface| surface.alive() && self.inhibitor_on_screen(surface))
    }

    fn inhibitor_on_screen(&self, surface: &WlSurface) -> bool {
        // Inhibitors are usually taken on a subsurface (the video area); what
        // is on screen or not is the surface at the root of its tree.
        let mut root = surface.clone();
        while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
            root = parent;
        }
        if let Some(window) = self.workspaces.get_window_for_surface(&root.id()) {
            return !window.is_minimised()
                && self.workspaces.output_workspaces.values().any(|ows| {
                    ows.current_space()
                        .elements()
                        .any(|element| element.id() == window.id())
                });
        }
        self.workspaces.outputs().any(|output| {
            smithay::desktop::layer_map_for_output(output)
                .layer_for_surface(&root, smithay::desktop::WindowSurfaceType::TOPLEVEL)
                .is_some()
        })
    }

    /// Lock the session because it has been idle for `timeout`.
    fn auto_lock_now(&mut self, timeout: std::time::Duration) {
        if self.is_session_locked() {
            return;
        }
        info!(idle_secs = timeout.as_secs(), "Auto-locking idle session");
        // Same path as the `lock` action.
        self.lock_session();
        // The idle clock restarts, so the countdown after an unlock runs from
        // the unlock rather than expiring at once.
        self.note_input_activity();
    }

    /// Whether the session is locked, or locking.
    pub fn is_session_locked(&self) -> bool {
        self.lock_state.is_active()
    }

    /// Whether the blank is anywhere on screen — locked, locking, or on its
    /// way back up after an unlock.
    ///
    /// The renderer needs this rather than [`Otto::is_session_locked`]: the
    /// KMS plane decomposition has no plane for the lock, so a frame that
    /// carries the blank has to be composited whole, and a window promoted to
    /// its own plane would scan out straight through it. Both stay off until
    /// the shade is gone, not until the session is nominally unlocked.
    pub fn lock_blank_on_screen(&self) -> bool {
        self.lock_state.is_active()
            || self
                .lock_shade_until
                .is_some_and(|until| std::time::Instant::now() < until)
    }

    /// Accept a lock request from Otto's locker.
    ///
    /// If the session is not locked yet, the blank goes up now and `locked`
    /// is sent once it is on screen. If it is — the blank raised ahead of the
    /// locker, or a lock whose locker has died — the request takes the
    /// standing lock over. What is refused is a second lock while the locker
    /// holding the first is alive: it could otherwise unlock a session it did
    /// not lock.
    pub fn begin_lock(&mut self, locker: SessionLocker) {
        if self.lock_state.is_active() {
            if self.lock_state.owner_alive() {
                // Dropping the locker sends `finished`.
                warn!("session lock requested while another locker holds it; refusing");
                return;
            }
            if matches!(self.lock_state, LockState::Locked { .. }) {
                warn!("the locker holding the lock is gone; a new one takes it over");
            }
        } else {
            info!("Locking session");
            self.raise_blank();
        }

        self.lock_locker_client = locker.ext_session_lock().client();
        match &mut self.lock_state {
            LockState::Locking { locker: slot, .. } => {
                // A request from a locker that died is dropped here, and its
                // `finished` goes nowhere.
                *slot = Some(locker);
            }
            LockState::Locked { owner } => {
                // The blank has stood the whole time the lock was defunct, so
                // there is nothing to wait for: the session is hidden now.
                *owner = locker.ext_session_lock().clone();
                info!("Session locked");
                locker.lock();
            }
            LockState::Unlocked => {}
        }
        self.confirm_lock_if_blanked();
        self.request_lock_redraw();
    }

    /// Raise the blank on every output and cut the session off, with no
    /// locker yet: the start of every lock.
    fn raise_blank(&mut self) {
        // A lock that arrives while the previous one's shade is still going up
        // takes the screen back over; the deadline it left behind would let the
        // plane path resume mid-lock.
        self.lock_shade_until = None;

        // An output plugged in from now until the unlock gets its blank as it
        // is added, before its first frame.
        self.workspaces.blank_new_outputs = true;

        // What `locked` is a promise about is screens someone can see. A
        // virtual output composites only when something is consuming it, so a
        // PipeWire output with no stream attached never presents a frame — and
        // waiting for one would mean the promise is never made, the locker
        // never authenticates, and the session cannot be unlocked at all,
        // short of a VT switch. Nothing of the session is visible on one
        // either way: its blank goes up with all the others below, and a
        // stream that starts later finds it there.
        let pending: HashSet<String> = self
            .workspaces
            .outputs()
            .filter(|output| !crate::virtual_output::is_virtual_output(output))
            .map(|output| output.name())
            .filter(|name| self.workspaces.output_workspaces.contains_key(name))
            .collect();

        // No screen anyone can see — the lid closed on the only panel, or
        // nothing but virtual outputs. Nothing of the session is visible, so
        // there is nothing to wait for; the blanks still go up below, for
        // when a screen comes back.
        let nothing_to_wait_for = pending.is_empty();

        // Every output's blank starts just above its screen and falls into
        // place. An output with no locker surface keeps the bare blank for the
        // whole lock; one that gets a surface has the panel fall with it,
        // since the surface hangs off this layer.
        let geometries: Vec<(String, f32, f32)> = self
            .workspaces
            .outputs()
            .filter_map(|output| {
                let geometry = self.workspaces.output_geometry(output)?;
                let scale = output.current_scale().fractional_scale() as f32;
                Some((
                    output.name(),
                    geometry.size.w as f32 * scale,
                    geometry.size.h as f32 * scale,
                ))
            })
            .collect();

        // An output with a workspace but no geometry is one set aside —
        // the laptop panel while the lid is shut. It comes back as it was, so
        // its blank has to be up already when it does.
        for (name, ows) in self.workspaces.output_workspaces.iter() {
            if geometries.iter().any(|(shown, _, _)| shown == name) {
                continue;
            }
            ows.lock_plane.set_position(Point { x: 0.0, y: 0.0 }, None);
            ows.lock_plane.set_hidden(false);
        }

        for (name, width_px, height_px) in geometries {
            let Some(ows) = self.workspaces.output_workspaces.get(&name) else {
                continue;
            };
            let margin = height_px * SLIDE_OVERSHOOT;
            ows.lock_plane
                .set_size(Size::points(width_px, height_px + margin), None);
            // Fully above the screen, then down to rest with only the slack
            // off-screen.
            ows.lock_plane.set_position(
                Point {
                    x: 0.0,
                    y: -(height_px + margin),
                },
                None,
            );
            ows.lock_plane.set_hidden(false);
            ows.lock_plane.set_position(
                Point { x: 0.0, y: -margin },
                Some(Transition::spring(SLIDE, SLIDE_BOUNCE)),
            );
        }

        // The shade is falling; the sound goes with it. Nothing depends on it
        // being heard — a theme without a `desktop-screen-lock` event locks
        // silently.
        if let Some(sound_player) = &self.sound_player {
            sound_player.play_lock_sound();
        }

        // Drop any interactive grab (move, resize, drag) and take keyboard
        // focus off the session. Focus moves to a lock surface as soon as one
        // is mapped; until then nothing has it, so nothing receives keys.
        self.cancel_session_interaction();

        let now = std::time::Instant::now();
        self.lock_state = LockState::Locking {
            locker: None,
            pending,
            since: now,
            // The blank covers the screen at the end of its travel; the
            // rebound after that happens in the slack above and uncovers
            // nothing, so there is no need to wait for the spring to settle.
            landed: if nothing_to_wait_for {
                now
            } else {
                now + std::time::Duration::from_secs_f32(SLIDE)
            },
        };
        self.arm_lock_watchdog();

        // The blank has to reach the screen for the lock to be confirmed at
        // all, and nothing else will ask for that frame. It happens to work
        // when a keypress triggered the lock — input requests a redraw of its
        // own — but an idle timer or a lid switch has no keypress behind it.
        self.request_lock_redraw();
    }

    /// Ask the backend to draw a frame after the lock state has changed.
    ///
    /// Raising the blank and taking it down are scene changes like any other,
    /// and like any other they are only visible once something draws them.
    /// Nothing else is asking while locked — the session is hidden and its
    /// clients throttled — so without this the screen keeps whatever frame it
    /// last presented until some unrelated redraw happens along. Moving the
    /// pointer is one, which is why a missing request looks like "it appears
    /// when I touch the mouse" rather than like nothing working at all.
    fn request_lock_redraw(&mut self) {
        self.backend_data.invalidate_scene_prefetch();
        self.backend_data.request_redraw();
        self.schedule_event_loop_dispatch();
    }

    /// Tell the locker the session is locked, once there is a locker to tell
    /// and the blank is on every screen; and let a held suspend go once the
    /// blank is, locker or not.
    fn confirm_lock_if_blanked(&mut self) {
        let ready = matches!(
            &self.lock_state,
            LockState::Locking {
                locker: Some(_),
                ..
            }
        ) && self.lock_state.blank_presented();
        if ready {
            if let LockState::Locking {
                locker: Some(locker),
                ..
            } = std::mem::replace(&mut self.lock_state, LockState::Unlocked)
            {
                // `lock()` is what sends the `locked` event the client is
                // waiting on; it consumes the locker, so the owner is kept.
                self.lock_state = LockState::Locked {
                    owner: locker.ext_session_lock().clone(),
                };
                info!("Session locked");
                locker.lock();
            }
        }
        if self.sleep_pending_since.is_some() && self.lock_state.blank_presented() {
            info!("the screen is blank; letting the suspend go ahead");
            self.release_sleep_inhibitor();
        }
    }

    /// Start the timer that looks after the lock while it stands. It goes
    /// with the lock, in [`Otto::finish_unlock`].
    fn arm_lock_watchdog(&mut self) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

        if self.lock_watchdog.is_some() {
            return;
        }
        let token = self.handle.insert_source(
            Timer::from_duration(LOCK_WATCHDOG_INTERVAL),
            |_, _, state| {
                if !state.lock_state.is_active() {
                    state.lock_watchdog = None;
                    return TimeoutAction::Drop;
                }
                state.lock_watchdog_tick();
                TimeoutAction::ToDuration(LOCK_WATCHDOG_INTERVAL)
            },
        );
        match token {
            Ok(token) => self.lock_watchdog = Some(token),
            Err(_) => {
                warn!("could not schedule the lock watchdog; a dead locker will not be replaced")
            }
        }
    }

    fn lock_watchdog_tick(&mut self) {
        self.give_up_on_stalled_outputs();
        self.confirm_lock_if_blanked();
        if self
            .sleep_pending_since
            .is_some_and(|since| since.elapsed() >= SLEEP_HOLD_MAX)
        {
            warn!("the blank did not reach the screen in time; letting the suspend go ahead");
            self.release_sleep_inhibitor();
        }
        self.lock_surfaces_pruned();
    }

    /// Stop waiting for outputs that have not presented the blank within
    /// [`LOCK_CONFIRM_TIMEOUT`]. Their blank stays up; the lock goes on
    /// without them rather than being given up, which would uncover every
    /// other screen because one is stuck.
    fn give_up_on_stalled_outputs(&mut self) {
        if let LockState::Locking { pending, since, .. } = &mut self.lock_state {
            if !pending.is_empty() && since.elapsed() >= LOCK_CONFIRM_TIMEOUT {
                warn!(
                    outputs = ?pending,
                    "outputs never presented the blank; locking without waiting for them"
                );
                pending.clear();
            }
        }
    }

    /// A frame has been presented on `output`. Sends frame callbacks to that
    /// output's lock surface and, while locking, counts the output as blanked.
    pub fn lock_frame_presented(&mut self, output: &Output) {
        if !self.lock_state.is_active() {
            return;
        }

        let time = self.clock.now();
        if let Some(entry) = self.lock_surfaces.get(&output.name()) {
            // Lock surfaces animate (the greeter's Touch ID mark does), and no
            // other path sends them frame callbacks — session clients get
            // theirs from `post_repaint`, which knows nothing about locking.
            send_frames_surface_tree(entry.surface.wl_surface(), output, time, None, |_, _| {
                Some(output.clone())
            });
        }

        self.refresh_lock_focus();

        // An output that arrived after the lock began — hotplugged — gets
        // its blank as it is added (`Workspaces::blank_new_outputs`), before
        // its first frame. This is the backstop for one that somehow did
        // not: it goes up now, and the frame just presented is the only one
        // that can have shown the session.
        if let Some(ows) = self.workspaces.output_workspaces.get(&output.name()) {
            if ows.lock_plane.hidden() {
                warn!(output = %output.name(), "output without its blank while locked; raising it");
                ows.lock_plane.set_position(Point { x: 0.0, y: 0.0 }, None);
                ows.lock_plane.set_hidden(false);
                self.request_lock_redraw();
            }
        }

        if let LockState::Locking {
            pending, landed, ..
        } = &mut self.lock_state
        {
            // A frame presented while the blank is still falling has the
            // desktop under it, so it does not blank anything yet. The slide
            // keeps producing damage, so more frames are coming.
            if std::time::Instant::now() >= *landed {
                pending.remove(&output.name());
            }
        }
        // An output that never presents is not waited for past the timeout;
        // see `give_up_on_stalled_outputs`.
        self.give_up_on_stalled_outputs();
        self.confirm_lock_if_blanked();
    }

    /// The locker has authenticated the user and asked for the session back:
    /// the blank goes back up the way it came down, and the session is under it
    /// again immediately.
    pub fn finish_unlock(&mut self) {
        if !self.lock_state.is_active() {
            return;
        }
        info!("Unlocking session");

        // A little past the slide, so the last frame of it is still composited
        // whole rather than handed back to the planes with the shade mid-air.
        self.lock_shade_until = Some(
            std::time::Instant::now()
                + std::time::Duration::from_secs_f32(SLIDE_OUT)
                + std::time::Duration::from_millis(50),
        );

        // The locker asks for the session back and exits, so the panel is
        // already a dead client by the time the shade starts moving. Its
        // wl_surface goes with it — nothing here may touch it again — but the
        // scene layers and the textures behind them are ours, so the panel
        // rides the shade up rather than vanishing at the first frame. The
        // layers are handed to the animation, which removes them once the
        // shade is off-screen; keeping them past that would leave the next
        // lock stacking a second panel on top of this one.
        let mut retiring: HashMap<String, Layer> = HashMap::new();
        for (name, entry) in self.lock_surfaces.drain() {
            self.surface_layers.remove(&entry.surface.wl_surface().id());
            retiring.insert(name, entry.shade);
        }

        for (name, ows) in self.workspaces.output_workspaces.iter() {
            let plane = ows.lock_plane.clone();
            let shade = retiring.remove(name);
            // Back to where the slide started: fully above the screen, slack
            // and all. A plane with no laid-out size has nowhere to go, so it
            // is taken down at once rather than waiting on an animation that
            // will not run.
            let height_px = plane.render_size().y;
            if height_px <= 0.0 {
                plane.set_hidden(true);
                if let Some(shade) = shade {
                    shade.remove();
                }
                continue;
            }
            plane
                .set_position(
                    Point {
                        x: 0.0,
                        y: -height_px,
                    },
                    Some(Transition::ease_in_quad(SLIDE_OUT)),
                )
                .on_finish(
                    move |l: &Layer, _| {
                        l.set_hidden(true);
                        if let Some(shade) = &shade {
                            shade.remove();
                        }
                    },
                    true,
                );
        }

        // Outputs that had a lock surface but no workspace entry (an output
        // taken away mid-lock) have no plane to ride up on.
        for shade in retiring.into_values() {
            shade.remove();
        }

        self.lock_state = LockState::Unlocked;
        self.workspaces.blank_new_outputs = false;
        self.lock_locker_client = None;
        self.lock_last_spawn = None;
        self.lock_locker_missing_reported = false;
        if let Some(token) = self.lock_watchdog.take() {
            self.handle.remove(token);
        }
        self.restore_session_focus();
        self.request_lock_redraw();
    }

    /// Drop lock surfaces whose client has gone. The lock itself survives: the
    /// output falls back to the blank, and the session stays hidden.
    pub fn lock_surfaces_pruned(&mut self) {
        if !self.lock_state.is_active() {
            return;
        }
        let dead: Vec<String> = self
            .lock_surfaces
            .iter()
            .filter(|(_, entry)| !entry.surface.alive())
            .map(|(name, _)| name.clone())
            .collect();
        for name in dead {
            if let Some(entry) = self.lock_surfaces.remove(&name) {
                debug!(output = %name, "lock surface gone; falling back to the blank");
                self.surface_layers.remove(&entry.surface.wl_surface().id());
                entry.shade.remove();
            }
        }

        self.respawn_locker_if_gone();
    }

    /// Bring the locker back if it is gone without unlocking — died after
    /// locking, died before its first surface, or never started at all.
    ///
    /// The session stays hidden either way — that is the protocol's guarantee
    /// and it is not negotiable here. But leaving the user with a black screen
    /// and no field to type into means the only way back in is a VT switch,
    /// which a tablet or a closed-lid laptop may not have. So the lock stands
    /// and a new locker is started into it, which takes the lock over (see
    /// [`Otto::begin_lock`]), rate-limited so a locker that crashes on startup
    /// cannot spin.
    ///
    /// Only for a lock Otto started a locker for. A test that connects a
    /// locker of its own gets no second one behind its back.
    fn respawn_locker_if_gone(&mut self) {
        let Some(last_spawn) = self.lock_last_spawn else {
            return;
        };
        if !self.lock_state.is_active()
            || self.locker_running()
            || self
                .lock_surfaces
                .values()
                .any(|entry| entry.surface.alive())
            || last_spawn.elapsed() < RESPAWN_INTERVAL
        {
            return;
        }

        if self.lock_locker_missing_reported {
            debug!("still no locker while locked; trying again");
        } else {
            warn!("no locker while locked; starting one");
        }
        self.spawn_locker();
    }

    /// Register a lock surface for `output` and configure it to the output's
    /// size. The layer hangs from that output's `lock_plane`, so it is drawn
    /// above everything and covered by the blank until it commits a buffer.
    pub fn add_lock_surface(&mut self, surface: LockSurface, output: Output) {
        let name = output.name();
        let Some(geometry) = self.workspaces.output_geometry(&output) else {
            warn!(output = %name, "lock surface for an output with no geometry");
            return;
        };

        let Some(ows) = self.workspaces.output_workspaces.get(&name) else {
            warn!(output = %name, "lock surface for an unknown output");
            return;
        };

        let scale = output.current_scale().fractional_scale() as f32;
        let width_px = geometry.size.w as f32 * scale;
        let height_px = geometry.size.h as f32 * scale;

        // A layer of our own between the blank and the client's, holding the
        // offset that puts the panel on the screen rather than up in the slack
        // the blank carries above it. It cannot be the client's own layer:
        // that one is registered in `surface_layers`, so every commit runs it
        // through `configure_surface_layer`, which sets the position from the
        // surface's geometry — and would drop this offset on the floor, taking
        // the panel a quarter-screen up and showing the blank below it.
        let shade = self.layers_engine.new_layer();
        shade.set_key(format!("lock_shade_{name}"));
        shade.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        shade.set_size(Size::points(width_px, height_px), None);
        shade.set_position(
            Point {
                x: 0.0,
                y: height_px * SLIDE_OVERSHOOT,
            },
            None,
        );
        let _ = ows.lock_plane.add_sublayer(&shade);

        let layer = self.layers_engine.new_layer();
        layer.set_key(format!("lock_surface_{name}"));
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        layer.set_size(Size::points(width_px, height_px), None);
        layer.set_pointer_events(true);
        let _ = shade.add_sublayer(&layer);

        let surface_id = surface.wl_surface().id();
        self.surface_layers.insert(surface_id, layer.clone());

        // Configure carries logical points; the client scales by the output's
        // fractional scale, which it learns from wp_fractional_scale.
        surface.with_pending_state(|state| {
            state.size = Some((geometry.size.w as u32, geometry.size.h as u32).into());
        });
        surface.send_configure();

        debug!(output = %name, w = geometry.size.w, h = geometry.size.h, "lock surface configured");

        // A surface a dead locker left on this output, not yet pruned, goes:
        // replacing the entry would leave its layers in the scene.
        if let Some(old) = self.lock_surfaces.insert(
            name,
            LockSurfaceEntry {
                surface,
                layer,
                shade,
                output,
            },
        ) {
            self.surface_layers.remove(&old.surface.wl_surface().id());
            old.shade.remove();
        }
    }

    /// Whether `surface_id` belongs to a lock surface (not a subsurface of one).
    pub fn lock_surface_output(&self, surface_id: &ObjectId) -> Option<String> {
        self.lock_surfaces
            .iter()
            .find(|(_, entry)| entry.surface.wl_surface().id() == *surface_id)
            .map(|(name, _)| name.clone())
    }

    /// Mirror a committed lock surface into the scene.
    pub fn update_lock_surface(&mut self, output_name: &str) {
        let Some((wl_surface, layer, scale)) = self.lock_surfaces.get(output_name).map(|entry| {
            (
                entry.surface.wl_surface().clone(),
                entry.layer.clone(),
                entry.output.current_scale().fractional_scale(),
            )
        }) else {
            return;
        };

        self.sync_surface_tree_layers(&wl_surface, scale, "lock_surface");
        layer.set_hidden(false);
        debug!(
            output = %output_name,
            size = ?layer.render_size(),
            children = layer.children().len(),
            "lock surface committed"
        );
    }

    /// Resize the lock surface on `output` after a mode or scale change.
    pub fn reconfigure_lock_surface(&mut self, output: &Output) {
        if !self.lock_state.is_active() {
            return;
        }
        let Some(geometry) = self.workspaces.output_geometry(output) else {
            return;
        };
        let Some(entry) = self.lock_surfaces.get(&output.name()) else {
            return;
        };
        let scale = output.current_scale().fractional_scale() as f32;
        let width_px = geometry.size.w as f32 * scale;
        let height_px = geometry.size.h as f32 * scale;
        let margin = height_px * SLIDE_OVERSHOOT;
        entry
            .shade
            .set_size(Size::points(width_px, height_px), None);
        entry.shade.set_position(Point { x: 0.0, y: margin }, None);
        entry
            .layer
            .set_size(Size::points(width_px, height_px), None);
        // The blank keeps its slack across a mode change, and stays at rest:
        // this is a resize, not a second arrival, and re-running the slide
        // would drop the screen back to the desktop mid-lock.
        if let Some(ows) = self.workspaces.output_workspaces.get(&output.name()) {
            ows.lock_plane
                .set_size(Size::points(width_px, height_px + margin), None);
            ows.lock_plane
                .set_position(Point { x: 0.0, y: -margin }, None);
        }
        entry.surface.with_pending_state(|state| {
            state.size = Some((geometry.size.w as u32, geometry.size.h as u32).into());
        });
        entry.surface.send_configure();
    }

    /// Point the keyboard at the lock surface of the output under the pointer.
    ///
    /// Called every frame while locked rather than from the motion handler:
    /// it is a pointer-location comparison, and it also covers the surface
    /// appearing, being replaced, or its output going away — none of which are
    /// motion events.
    pub fn refresh_lock_focus(&mut self) {
        if !self.lock_state.is_active() || self.lock_surfaces.is_empty() {
            return;
        }

        let pointer_pos = self.pointer.current_location();
        let wanted = self
            .workspaces
            .outputs()
            .find(|o| {
                self.workspaces
                    .output_geometry(o)
                    .is_some_and(|g| g.contains(pointer_pos.to_i32_round()))
            })
            .and_then(|o| self.lock_surfaces.get(&o.name()))
            .or_else(|| self.lock_surfaces.values().next())
            .map(|entry| entry.surface.wl_surface().clone());

        let Some(wanted) = wanted else { return };
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if matches!(
            keyboard.current_focus(),
            Some(crate::focus::KeyboardFocusTarget::LockSurface(current)) if current == wanted
        ) {
            return;
        }

        let serial = SERIAL_COUNTER.next_serial();
        keyboard.set_focus(
            self,
            Some(crate::focus::KeyboardFocusTarget::LockSurface(wanted)),
            serial,
        );
    }

    /// Take the session out of the user's hands: end any interactive grab and
    /// move keyboard focus off the session, remembering where it was so
    /// unlocking can put it back.
    fn cancel_session_interaction(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            self.lock_previous_focus = keyboard.current_focus();
            // A keyboard grab (a menu, an interactive move) would otherwise
            // keep delivering keys to the session while it is hidden.
            keyboard.unset_grab(self);
            keyboard.set_focus(self, None, serial);
        }
        // Same for the pointer: an X client holding a grab must lose it, or a
        // drag begun before the lock keeps receiving motion.
        let pointer = self.pointer.clone();
        pointer.unset_grab(self, serial, smithay::backend::input::InputTime::now());
    }

    /// Give focus back to whatever had it when the lock began, if it is still
    /// there.
    fn restore_session_focus(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();
        let previous = self.lock_previous_focus.take().filter(|f| f.alive());
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, previous, serial);
        }
    }

    /// The surface a locked screen sends input to on `output`, if the locker
    /// has mapped one there.
    pub fn lock_surface_for_output(&self, output: &Output) -> Option<WlSurface> {
        self.lock_surfaces
            .get(&output.name())
            .map(|entry| entry.surface.wl_surface().clone())
    }
}

/// Lock surfaces keyed by output name.
pub type LockSurfaces = HashMap<String, LockSurfaceEntry>;

/// What the logind listeners report to the event loop.
enum LogindEvent {
    /// `Lock` on this session.
    Lock,
    /// `PrepareForSleep`: `true` before a suspend, `false` after the resume.
    PrepareForSleep(bool),
    /// A fresh `delay` inhibitor on `sleep`, to hold until the next suspend
    /// has been locked for.
    SleepInhibitor(std::os::fd::OwnedFd),
}

/// Whether a suspend locks the session first.
///
/// `lock.on_suspend` (on by default) says the machine should wake to the
/// locker, whatever asked for the suspend — the power menu, the power button,
/// the lid or `systemctl suspend`. `on_lid_close =
/// "lock"` says the same, and locks on a suspend even with it off. Without
/// either, Otto still holds each suspend briefly, but only to finish a lock
/// that is already under way.
fn lock_before_sleep() -> bool {
    crate::config::Config::with(locks_before_sleep)
}

fn locks_before_sleep(c: &crate::config::Config) -> bool {
    c.lock.on_suspend || c.power_management.on_lid_close == crate::config::LidCloseAction::Lock
}

const LOGIND: &str = "org.freedesktop.login1";

fn logind_manager(bus: &zbus::blocking::Connection) -> zbus::Result<zbus::blocking::Proxy<'_>> {
    zbus::blocking::Proxy::new(
        bus,
        LOGIND,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
}

/// The logind session this compositor runs in.
///
/// The one this process belongs to, first: `$XDG_SESSION_ID` is inherited,
/// and names the wrong session — or one that no longer exists — when Otto is
/// started from outside its own (a login shell on another VT, a user unit).
/// Failing that, the user's display session, which is the graphical one
/// logind picked for them; and only then the environment.
fn own_session(
    bus: &zbus::blocking::Connection,
    manager: &zbus::blocking::Proxy<'_>,
) -> zbus::Result<zbus::zvariant::OwnedObjectPath> {
    use zbus::zvariant::OwnedObjectPath;

    let by_pid = manager.call::<_, _, OwnedObjectPath>("GetSessionByPID", &(std::process::id(),));
    let err = match by_pid {
        Ok(session) => return Ok(session),
        Err(err) => err,
    };
    debug!(%err, "this process is in no logind session; trying the user's display session");

    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    let display = manager
        .call::<_, _, OwnedObjectPath>("GetUser", &(uid,))
        .and_then(|user| {
            zbus::blocking::Proxy::new(bus, LOGIND, user, "org.freedesktop.login1.User")?
                .get_property::<(String, OwnedObjectPath)>("Display")
        });
    match display {
        Ok((id, session)) if !id.is_empty() => return Ok(session),
        Ok(_) => debug!("the user has no display session"),
        Err(err) => debug!(%err, "could not read the user's display session"),
    }

    match std::env::var("XDG_SESSION_ID") {
        Ok(id) if !id.is_empty() => manager.call("GetSession", &(id,)),
        _ => Err(err),
    }
}

/// Forward every `Lock` logind emits on this session to `tx`. Blocks for as
/// long as the system bus is there.
fn listen_for_logind_lock(
    tx: &smithay::reexports::calloop::channel::Sender<LogindEvent>,
) -> zbus::Result<()> {
    let bus = zbus::blocking::Connection::system()?;
    let manager = logind_manager(&bus)?;
    let session = own_session(&bus, &manager)?;
    info!(session = %session.as_str(), "Listening for logind's Lock");
    let session =
        zbus::blocking::Proxy::new(&bus, LOGIND, session, "org.freedesktop.login1.Session")?;
    for _ in session.receive_signal("Lock")? {
        if tx.send(LogindEvent::Lock).is_err() {
            break;
        }
    }
    Ok(())
}

/// Take a `delay` inhibitor on sleep: logind waits for it to be closed, up to
/// `InhibitDelayMaxSec`, before suspending.
fn take_sleep_inhibitor(manager: &zbus::blocking::Proxy<'_>) -> zbus::Result<std::os::fd::OwnedFd> {
    let fd: zbus::zvariant::OwnedFd =
        manager.call("Inhibit", &("sleep", "Otto", "lock before sleep", "delay"))?;
    Ok(fd.into())
}

/// Hold a sleep inhibitor and forward `PrepareForSleep` to `tx`, taking a new
/// inhibitor after every resume. Blocks for as long as the system bus is
/// there.
fn listen_for_sleep(
    tx: &smithay::reexports::calloop::channel::Sender<LogindEvent>,
) -> zbus::Result<()> {
    let bus = zbus::blocking::Connection::system()?;
    let manager = logind_manager(&bus)?;
    // Subscribed before the inhibitor is taken, so a suspend that starts in
    // between is not missed.
    let signals = manager.receive_signal("PrepareForSleep")?;
    let send_inhibitor = |tx: &smithay::reexports::calloop::channel::Sender<LogindEvent>| {
        match take_sleep_inhibitor(&manager) {
            Ok(fd) => tx.send(LogindEvent::SleepInhibitor(fd)).is_ok(),
            Err(err) => {
                warn!(%err, "could not take logind's sleep delay inhibitor");
                true
            }
        }
    };
    if !send_inhibitor(tx) {
        return Ok(());
    }
    info!("Holding suspend for the lock");
    for signal in signals {
        let start: bool = match signal.body().deserialize() {
            Ok(start) => start,
            Err(err) => {
                warn!(%err, "unreadable PrepareForSleep");
                continue;
            }
        };
        if tx.send(LogindEvent::PrepareForSleep(start)).is_err() {
            break;
        }
        if !start && !send_inhibitor(tx) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A suspend locks by default, and turning `lock.on_suspend` off leaves
    /// the lid's `lock` action still locking.
    #[test]
    fn a_suspend_locks_by_default() {
        use crate::config::{Config, LidCloseAction};
        let mut config = Config::default();
        assert_ne!(config.power_management.on_lid_close, LidCloseAction::Lock);
        assert!(locks_before_sleep(&config));
        config.lock.on_suspend = false;
        assert!(!locks_before_sleep(&config));
        config.power_management.on_lid_close = LidCloseAction::Lock;
        assert!(locks_before_sleep(&config));
    }
}
