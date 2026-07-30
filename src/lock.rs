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

/// How long an output has to present the blank before the lock is given up on.
///
/// The alternative to giving up is a session that stays hidden with no locker
/// able to authenticate — the client is waiting on `locked`, which is waiting
/// on a frame that is not coming — and no way out but a VT switch, which a
/// tablet or a closed lid may not offer. Generous, because the cost of
/// tripping it on a slow first frame is a lock that did not happen.
const LOCK_CONFIRM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
        locker: SessionLocker,
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

impl<BackendData: Backend> Otto<BackendData> {
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

        // A lock that arrives while the previous one's shade is still going up
        // takes the screen back over; the deadline it left behind would let the
        // plane path resume mid-lock.
        self.lock_shade_until = None;

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

        // Nothing would ever confirm the blank, so the promise could not be
        // kept. Returning drops the locker, which sends `finished`: the
        // request is refused and the session is untouched.
        if pending.is_empty() {
            warn!("no output can present the blank; refusing to lock");
            return;
        }

        // Every output's blank starts just above its screen and falls into
        // place. An output with no locker surface keeps the bare blank for the
        // whole lock; one that gets a surface has the panel fall with it,
        // since the surface hangs off this layer.
        let geometries: Vec<(String, f32, f32)> = self
            .workspaces
            .outputs()
            .filter_map(|output| {
                let geometry = self.workspaces.output_geometry(&output)?;
                let scale = output.current_scale().fractional_scale() as f32;
                Some((
                    output.name(),
                    geometry.size.w as f32 * scale,
                    geometry.size.h as f32 * scale,
                ))
            })
            .collect();

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

        self.lock_state = LockState::Locking {
            locker,
            pending,
            since: std::time::Instant::now(),
            // The blank covers the screen at the end of its travel; the
            // rebound after that happens in the slack above and uncovers
            // nothing, so there is no need to wait for the spring to settle.
            landed: std::time::Instant::now() + std::time::Duration::from_secs_f32(SLIDE),
        };

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

    /// Give up on a lock that cannot be confirmed: put the session back and
    /// tell the client the request failed.
    ///
    /// Dropping the [`SessionLocker`] is what sends `finished`, so taking the
    /// state apart is all this has to do.
    fn abandon_lock(&mut self) {
        for ows in self.workspaces.output_workspaces.values() {
            ows.lock_plane.set_hidden(true);
        }
        self.lock_state = LockState::Unlocked;
        self.lock_locker_seen = false;
        self.lock_last_spawn = None;
        self.lock_shade_until = None;
        self.restore_session_focus();
        self.request_lock_redraw();
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

        let (confirmed, stalled) = match &mut self.lock_state {
            LockState::Locking {
                pending,
                since,
                landed,
                ..
            } => {
                // A frame presented while the blank is still falling has the
                // desktop under it, so it does not blank anything yet. The
                // slide keeps producing damage, so more frames are coming.
                if std::time::Instant::now() < *landed {
                    return;
                }
                pending.remove(&output.name());
                let stalled = !pending.is_empty() && since.elapsed() >= LOCK_CONFIRM_TIMEOUT;
                if stalled {
                    warn!(
                        outputs = ?pending,
                        "outputs never presented the blank; abandoning the lock"
                    );
                }
                (pending.is_empty(), stalled)
            }
            _ => (false, false),
        };

        // An output that cannot present cannot be part of a promise that the
        // session is hidden — and a lock that is never confirmed is one no
        // locker can ever unlock, because the client is waiting for `locked`
        // before it authenticates.
        if stalled {
            self.abandon_lock();
            return;
        }

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
        self.lock_locker_seen = false;
        self.lock_last_spawn = None;
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

        self.lock_surfaces.insert(
            name,
            LockSurfaceEntry {
                surface,
                layer,
                shade,
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
