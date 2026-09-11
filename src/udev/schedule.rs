//! When a surface draws.
//!
//! A draw pass is expensive even when it ends up flipping nothing: every
//! element is rebuilt, the scene ticked, the planes reconsidered. So passes
//! run only for a reason, and each surface tracks whether one is already on
//! its way (`SurfaceData::frame_scheduled`) so a reason never schedules two.
//!
//! Reasons arrive as events — a client commit or input bumps the redraw
//! generation, lay-rs reports pending transactions — or hold for a while
//! (`continuous_frames`: a fullscreen scanout window, a screencast, a drag
//! icon, an animated cursor). The decisions are pure so the invariants can
//! be tested: content at 30 fps costs 30 passes a second, an idle desktop
//! none, and a request that lands while a frame is in flight is never lost.

/// What the VBlank handler knows once the flipped frame is acknowledged.
pub(super) struct VblankInputs {
    /// The scene tick run at this VBlank reported damage.
    pub scene_has_damage: bool,
    /// The last pass found a consumer that needs every VBlank.
    pub continuous_frames: bool,
    /// Current redraw-request generation (input, commits).
    pub redraw_gen: u64,
    /// The generation the surface's last pass started from.
    pub seen_redraw_gen: u64,
    /// lay-rs has transactions or animations pending.
    pub animations_pending: bool,
    /// A promoted (direct-scanout) window committed a new buffer.
    pub scanout_commit_pending: bool,
}

/// Whether the VBlank schedules the next pass. Without a reason the surface
/// goes idle until an event wakes it through [`wake_idle_surface`].
pub(super) fn wants_frame_after_vblank(i: &VblankInputs) -> bool {
    i.scene_has_damage
        || i.continuous_frames
        || i.seen_redraw_gen != i.redraw_gen
        || i.animations_pending
        || i.scanout_commit_pending
}

/// Whether the event loop kicks a pass for a surface right now. A surface
/// with a pass on its way is left alone: that pass runs after whatever just
/// happened and sees it, and a second one would double-render.
pub(super) fn wake_idle_surface(
    frame_scheduled: bool,
    redraw_gen: u64,
    seen_redraw_gen: u64,
    animations_pending: bool,
) -> bool {
    !frame_scheduled && (animations_pending || seen_redraw_gen != redraw_gen)
}

/// What follows a pass: `(frame_scheduled, reschedule_timer)`.
///
/// A queued frame's VBlank decides what comes next. A pass that drew
/// nothing continues on a timer only for an animation still in flight —
/// anything else that needs a frame arrives as an event and wakes the
/// surface through the loop.
pub(super) fn after_pass(
    rendered: bool,
    may_reschedule: bool,
    animations_pending: bool,
) -> (bool, bool) {
    let reschedule = may_reschedule && !rendered && animations_pending;
    (rendered || reschedule, reschedule)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One output driven through the real decision points, with the
    /// VBlank-scheduled pass modelled as "runs before the next VBlank".
    struct Sim {
        redraw_gen: u64,
        seen_redraw_gen: u64,
        frame_scheduled: bool,
        /// A pass is scheduled by a timer and runs before the next VBlank.
        pass_pending: bool,
        /// A frame was queued; the next VBlank acknowledges it.
        in_flight: bool,
        continuous_frames: bool,
        animations_pending: bool,
        scene_damage: bool,
        scanout_commit: bool,
        passes: u32,
        flips: u32,
    }

    impl Sim {
        fn new() -> Self {
            Sim {
                redraw_gen: 0,
                seen_redraw_gen: 0,
                frame_scheduled: false,
                pass_pending: false,
                in_flight: false,
                continuous_frames: false,
                animations_pending: false,
                scene_damage: false,
                scanout_commit: false,
                passes: 0,
                flips: 0,
            }
        }

        /// Input or a client commit: what `request_redraw` does, followed by
        /// the loop turn that consumes events.
        fn request(&mut self) {
            self.redraw_gen += 1;
            self.loop_turn();
        }

        fn loop_turn(&mut self) {
            if wake_idle_surface(
                self.frame_scheduled,
                self.redraw_gen,
                self.seen_redraw_gen,
                self.animations_pending,
            ) {
                self.pass();
            }
        }

        /// The draw pass: sees everything requested so far, flips when there
        /// is something to show.
        fn pass(&mut self) {
            self.pass_pending = false;
            self.passes += 1;
            self.seen_redraw_gen = self.redraw_gen;
            let rendered = self.scene_damage || self.scanout_commit;
            self.scene_damage = false;
            self.scanout_commit = false;
            let (scheduled, reschedule) = after_pass(rendered, true, self.animations_pending);
            self.frame_scheduled = scheduled;
            if rendered {
                self.in_flight = true;
                self.flips += 1;
            } else if reschedule {
                self.pass_pending = true;
            }
        }

        fn vblank(&mut self) {
            if self.pass_pending {
                self.pass();
            }
            if !self.in_flight {
                return;
            }
            self.in_flight = false;
            self.frame_scheduled = false;
            let wants = wants_frame_after_vblank(&VblankInputs {
                scene_has_damage: self.scene_damage,
                continuous_frames: self.continuous_frames,
                redraw_gen: self.redraw_gen,
                seen_redraw_gen: self.seen_redraw_gen,
                animations_pending: self.animations_pending,
                scanout_commit_pending: self.scanout_commit,
            });
            if wants {
                // The deadline timer fires before the next VBlank; anything
                // that happens in between is seen by this pass either way.
                self.frame_scheduled = true;
                self.pass();
            }
        }
    }

    #[test]
    fn thirty_fps_content_costs_thirty_passes_on_a_120hz_output() {
        // A promoted video window commits every fourth refresh. Before, each
        // flip was followed by three empty passes: 120 passes a second for
        // 30 frames of content.
        let mut sim = Sim::new();
        for vblank in 0..1200 {
            if vblank % 4 == 0 {
                sim.scanout_commit = true;
                sim.request();
            }
            sim.vblank();
        }
        assert_eq!(sim.flips, 300);
        assert_eq!(sim.passes, 300, "every pass should flip");
    }

    #[test]
    fn an_idle_desktop_runs_no_passes() {
        let mut sim = Sim::new();
        sim.scene_damage = true;
        sim.request();
        for _ in 0..600 {
            sim.vblank();
        }
        assert_eq!(sim.flips, 1);
        assert_eq!(sim.passes, 1);
        assert!(!sim.frame_scheduled);
    }

    #[test]
    fn a_request_during_an_in_flight_frame_is_not_lost() {
        // The cursor moves while the previous frame waits for its VBlank:
        // the loop must not kick (a frame is scheduled), and the VBlank must
        // still schedule the pass that draws the new cursor position.
        let mut sim = Sim::new();
        sim.scene_damage = true;
        sim.request();
        assert!(sim.in_flight);
        let passes_before = sim.passes;
        sim.request();
        assert_eq!(
            sim.passes, passes_before,
            "no kick while a frame is in flight"
        );
        sim.vblank();
        assert_eq!(sim.passes, passes_before + 1, "the VBlank owes the pass");
    }

    #[test]
    fn a_request_while_idle_wakes_exactly_once() {
        let mut sim = Sim::new();
        sim.request();
        assert_eq!(sim.passes, 1);
        sim.loop_turn();
        sim.loop_turn();
        assert_eq!(sim.passes, 1, "a seen request does not kick again");
    }

    #[test]
    fn an_animation_keeps_passes_coming_without_flips() {
        // lay-rs reports pending transactions but the scene has not produced
        // damage yet (the first tick of a transition): the surface must keep
        // ticking on its own timer until the animation drains.
        let mut sim = Sim::new();
        sim.animations_pending = true;
        sim.loop_turn();
        for _ in 0..10 {
            sim.vblank();
        }
        assert_eq!(sim.passes, 11);
        sim.animations_pending = false;
        sim.vblank();
        sim.vblank();
        assert_eq!(
            sim.passes, 12,
            "one last pass sees the animation drained, then idle"
        );
        assert!(!sim.frame_scheduled);
    }

    #[test]
    fn a_continuous_consumer_holds_the_refresh_rate() {
        // A fullscreen scanout window's commits leave no scene damage and set
        // no flag, so the surface has to draw every VBlank while one exists.
        let mut sim = Sim::new();
        sim.continuous_frames = true;
        sim.scene_damage = true;
        sim.request();
        for _ in 0..120 {
            sim.scene_damage = true; // each pass finds a new buffer
            sim.vblank();
        }
        assert_eq!(sim.passes, 121);
    }

    #[test]
    fn one_request_wakes_every_idle_output() {
        // The generation is compared, never consumed: output A's pass seeing
        // it must not hide it from output B.
        let redraw_gen = 7;
        let a_seen = 7; // A already drew for it
        let b_seen = 6;
        assert!(!wake_idle_surface(false, redraw_gen, a_seen, false));
        assert!(wake_idle_surface(false, redraw_gen, b_seen, false));
        assert!(wants_frame_after_vblank(&VblankInputs {
            scene_has_damage: false,
            continuous_frames: false,
            redraw_gen,
            seen_redraw_gen: b_seen,
            animations_pending: false,
            scanout_commit_pending: false,
        }));
    }

    #[test]
    fn a_failed_pass_does_not_spin() {
        // DeviceInactive (suspend) asks to retry; without an animation the
        // retry waits for the resume path's explicit render instead.
        assert_eq!(after_pass(false, true, false), (false, false));
        assert_eq!(after_pass(false, true, true), (true, true));
        assert_eq!(after_pass(true, false, false), (true, false));
    }
}
