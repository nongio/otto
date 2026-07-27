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
//! Otto performs no authentication. The locker (`otto-lock` by default) runs as
//! the session user and talks to PAM itself, exactly as the greeter talks to
//! greetd. See `specs/lock-screen.md`.

use std::collections::{HashMap, HashSet};

use layers::prelude::*;
use layers::types::Size;
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::output::Output;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{IsAlive, SERIAL_COUNTER};
use smithay::wayland::session_lock::{LockSurface, SessionLocker};
use tracing::{debug, info, warn};

use crate::state::{Backend, Otto};

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
        locker: SessionLocker,
        /// Outputs that have yet to present the blank.
        pending: HashSet<String>,
    },
    /// The client has been told the session is locked.
    Locked,
}

impl LockState {
    /// Whether the session is locked or on its way there. Input gating uses
    /// this rather than [`LockState::Locked`]: the window between the request
    /// and the confirmation is exactly when the desktop must stop reacting.
    pub fn is_active(&self) -> bool {
        !matches!(self, LockState::Unlocked)
    }
}

/// A locker's surface for one output, and the scene layer it draws into.
pub struct LockSurfaceEntry {
    pub surface: LockSurface,
    /// Container layer, a child of that output's `lock_plane`.
    pub layer: Layer,
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

impl<BackendData: Backend> Otto<BackendData> {
    /// Whether the session is locked, or locking.
    pub fn is_session_locked(&self) -> bool {
        self.lock_state.is_active()
    }

    /// Accept a lock request: raise the blank on every output and wait for it
    /// to reach the screen.
    pub fn begin_lock(&mut self, locker: SessionLocker) {
        if self.lock_state.is_active() {
            // Dropping the locker sends `finished`. One lock at a time — a
            // second one could otherwise unlock a session it did not lock.
            warn!("session lock requested while already locked; refusing");
            return;
        }

        info!("Locking session");

        // Every output goes opaque now, before the client has drawn anything.
        // An output with no locker surface stays this way for the whole lock.
        let pending: HashSet<String> = self
            .workspaces
            .output_workspaces
            .iter()
            .map(|(name, ows)| {
                ows.lock_plane.set_hidden(false);
                name.clone()
            })
            .collect();

        // Drop any interactive grab (move, resize, drag) and take keyboard
        // focus off the session. Focus moves to a lock surface as soon as one
        // is mapped; until then nothing has it, so nothing receives keys.
        self.cancel_session_interaction();

        self.lock_state = LockState::Locking { locker, pending };
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

        let confirmed = match &mut self.lock_state {
            LockState::Locking { pending, .. } => {
                pending.remove(&output.name());
                pending.is_empty()
            }
            _ => false,
        };

        if confirmed {
            // Take the locker out of the state to consume it; `lock()` is what
            // sends the `locked` event the client is waiting on.
            if let LockState::Locking { locker, .. } =
                std::mem::replace(&mut self.lock_state, LockState::Locked)
            {
                info!("Session locked");
                locker.lock();
            }
        }
    }

    /// The locker has authenticated the user and asked for the session back.
    pub fn finish_unlock(&mut self) {
        if !self.lock_state.is_active() {
            return;
        }
        info!("Unlocking session");

        for entry in self.lock_surfaces.values() {
            self.surface_layers.remove(&entry.surface.wl_surface().id());
            entry.layer.remove();
        }
        self.lock_surfaces.clear();

        for ows in self.workspaces.output_workspaces.values() {
            ows.lock_plane.set_hidden(true);
        }

        self.lock_state = LockState::Unlocked;
        self.lock_locker_seen = false;
        self.lock_last_spawn = None;
        self.restore_session_focus();
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
                entry.layer.remove();
            }
        }

        self.respawn_locker_if_gone();
    }

    /// Bring the locker back if it died without unlocking.
    ///
    /// The session stays hidden either way — that is the protocol's guarantee
    /// and it is not negotiable here. But leaving the user with a black screen
    /// and no field to type into means the only way back in is a VT switch,
    /// which a tablet or a closed-lid laptop may not have. So the lock stands
    /// and a new locker is started into it, rate-limited so a locker that
    /// crashes on startup cannot spin.
    fn respawn_locker_if_gone(&mut self) {
        const RESPAWN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

        if !self.lock_locker_seen || !self.lock_surfaces.is_empty() {
            return;
        }
        if self
            .lock_last_spawn
            .is_some_and(|at| at.elapsed() < RESPAWN_INTERVAL)
        {
            return;
        }

        let (cmd, args) = locker_command();
        warn!(locker = %cmd, "locker gone while locked; restarting it");
        self.lock_last_spawn = Some(std::time::Instant::now());
        self.launch_program(cmd, args);
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

        let layer = self.layers_engine.new_layer();
        layer.set_key(format!("lock_surface_{name}"));
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        layer.set_size(Size::points(width_px, height_px), None);
        layer.set_pointer_events(true);
        let _ = ows.lock_plane.add_sublayer(&layer);

        let surface_id = surface.wl_surface().id();
        self.surface_layers.insert(surface_id, layer.clone());

        // Configure carries logical points; the client scales by the output's
        // fractional scale, which it learns from wp_fractional_scale.
        surface.with_pending_state(|state| {
            state.size = Some((geometry.size.w as u32, geometry.size.h as u32).into());
        });
        surface.send_configure();

        debug!(output = %name, w = geometry.size.w, h = geometry.size.h, "lock surface configured");

        self.lock_surfaces.insert(
            name,
            LockSurfaceEntry {
                surface,
                layer,
                output,
            },
        );
        self.lock_locker_seen = true;
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
        entry.layer.set_size(
            Size::points(
                geometry.size.w as f32 * scale,
                geometry.size.h as f32 * scale,
            ),
            None,
        );
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
        let time = self.clock.now().as_millis();
        pointer.unset_grab(self, serial, time);
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
