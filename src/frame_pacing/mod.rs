//! When a surface draws, and when its clients are told to paint.
//!
//! A draw pass is expensive even when it ends up flipping nothing: every
//! element is rebuilt, the scene ticked, the planes reconsidered. So passes
//! run only for a reason, and each surface tracks whether one is already on
//! its way ([`FramePacer::is_scheduled`]) so a reason never schedules two.
//!
//! Reasons arrive as events — a client commit or input bumps the redraw
//! generation, lay-rs reports pending transactions — or hold for a while
//! (`continuous_frames`: a fullscreen scanout window, a screencast, a drag
//! icon, an animated cursor). The decisions are pure so the invariants can
//! be tested: content at 30 fps costs 30 passes a second, an idle desktop
//! none, and a request that lands while a frame is in flight is never lost.
//!
//! [`FramePacer`] holds one surface's pacing state and is the only thing the
//! udev backend asks when to draw. It knows no clock, DRM device or event
//! loop: the backend hands it times in microseconds and turns its answers
//! into calloop timers. [`sim`] drives the same pacer from a virtual display
//! (an [`edid`] mode with a perfect VBlank), so frame pacing can be measured
//! deterministically, without a GPU (`cargo bench --bench frame_pacing`).
//! See `specs/frame-pacing.md` and `docs/developer/render_loop.md`.

pub mod edid;
pub mod sim;

/// The share of a period the compositor's own CPU plus GPU need may take for
/// it to count as keeping up: below it, the VBlank-cadence tick sends the
/// callbacks of VBlanks at which nothing flipped (see
/// [`FramePacer::keeping_up`]).
pub const KEEP_UP_FRACTION: f32 = 0.6;

/// The share of a period a full-rate surface must have gone without a frame
/// callback for a commit to be answered at once (see [`callback_due_on_commit`]).
pub const ON_COMMIT_FRACTION: f32 = 0.95;

/// The CPU estimate a surface starts from, before it has drawn.
pub const INITIAL_CPU_ESTIMATE_US: f32 = 2_000.0;

/// Whether a commit applied `since_sent_us` after the surface's last
/// VBlank-paced callback gets its next callback right away: a full-rate
/// surface whose last send is nearly a whole period old. Just under a
/// period, as the VBlank handler runs a little after the VBlank itself and a
/// commit applied in that sliver has waited one full period.
pub fn callback_due_on_commit(full_rate: bool, since_sent_us: f32, period_us: f32) -> bool {
    full_rate && since_sent_us >= period_us * ON_COMMIT_FRACTION
}

/// When the pass a decision asks for runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Draw {
    /// Right away, in this loop turn.
    Now,
    /// After this many microseconds.
    In(f32),
}

/// Why the next pass may be wanted, as the VBlank handler sees it. The
/// generation the last pass started from is the pacer's own.
#[derive(Debug, Clone, Copy, Default)]
pub struct VblankReasons {
    /// The scene tick run at this VBlank reported damage.
    pub scene_has_damage: bool,
    /// The last pass found a consumer that needs every VBlank.
    pub continuous_frames: bool,
    /// Current redraw-request generation (input, commits).
    pub redraw_gen: u64,
    /// lay-rs has transactions or animations pending.
    pub animations_pending: bool,
    /// A promoted (direct-scanout) window committed a new buffer.
    pub scanout_commit_pending: bool,
}

/// One output's pacing state: what its passes cost, how long its period is,
/// whether a pass is on its way and which requests the last one saw.
///
/// Times are microseconds. The pacer never reads a clock: the caller says
/// how long ago the last VBlank was, and turns [`Draw`] into a timer.
#[derive(Debug, Clone)]
pub struct FramePacer {
    /// Mean CPU time of a pass (EMA, α = 0.1).
    avg_cpu_us: f32,
    /// GPU time of a pass's composite, from submission to the fence
    /// ([`gpu_estimate_us`]); zero until one was measured.
    avg_gpu_us: f32,
    /// The refresh period; zero until the output has a mode.
    period_us: f32,
    /// A draw pass is on its way: a deadline timer is armed, or a frame is
    /// queued and its VBlank decides the next one. While set, nothing else
    /// schedules a pass — the pending one sees every change before it runs.
    scheduled: bool,
    /// The redraw-request generation the last pass started from. A newer one
    /// means input or a commit arrived since, and a pass is owed.
    seen_redraw_gen: u64,
}

impl Default for FramePacer {
    fn default() -> Self {
        Self::new()
    }
}

impl FramePacer {
    pub fn new() -> Self {
        Self {
            avg_cpu_us: INITIAL_CPU_ESTIMATE_US,
            avg_gpu_us: 0.0,
            period_us: 0.0,
            scheduled: false,
            seen_redraw_gen: 0,
        }
    }

    /// Takes the period of a mode refreshing at `refresh_mhz` (millihertz,
    /// as DRM and Smithay report it). A zero rate leaves the period unknown.
    pub fn set_refresh_mhz(&mut self, refresh_mhz: i32) {
        self.period_us = if refresh_mhz > 0 {
            1_000_000_000f32 / refresh_mhz as f32
        } else {
            0.0
        };
    }

    pub fn period_us(&self) -> f32 {
        self.period_us
    }

    pub fn avg_cpu_us(&self) -> f32 {
        self.avg_cpu_us
    }

    pub fn avg_gpu_us(&self) -> f32 {
        self.avg_gpu_us
    }

    /// Whether a pass is on its way.
    pub fn is_scheduled(&self) -> bool {
        self.scheduled
    }

    /// The generation the last pass started from.
    pub fn seen_redraw_gen(&self) -> u64 {
        self.seen_redraw_gen
    }

    /// The deadline delay of a pass aimed from a VBlank ([`draw_delay_us`]);
    /// the floor while the period is unknown.
    pub fn draw_delay_us(&self) -> f32 {
        if self.period_us <= 0.0 {
            return MIN_DRAW_DELAY_US;
        }
        draw_delay_us(self.period_us, self.avg_cpu_us, self.avg_gpu_us)
    }

    /// Whether the compositor composes comfortably within a period, so
    /// clients may be paced at VBlanks where nothing flipped. The compose
    /// interval itself is no signal: at 60 fps it reads as "behind" because
    /// the client is at 60, and that would hold it there.
    pub fn keeping_up(&self) -> bool {
        self.avg_cpu_us + self.avg_gpu_us < self.period_us * KEEP_UP_FRACTION
    }

    /// The flipped frame's VBlank was acknowledged: decides whether the next
    /// pass is owed, and when it starts. `None` leaves the surface idle until
    /// an event wakes it ([`Self::should_wake`]).
    pub fn on_vblank(&mut self, reasons: &VblankReasons) -> Option<Draw> {
        self.scheduled = false;
        let wants = wants_frame_after_vblank(&VblankInputs {
            scene_has_damage: reasons.scene_has_damage,
            continuous_frames: reasons.continuous_frames,
            redraw_gen: reasons.redraw_gen,
            seen_redraw_gen: self.seen_redraw_gen,
            animations_pending: reasons.animations_pending,
            scanout_commit_pending: reasons.scanout_commit_pending,
        });
        if !wants {
            return None;
        }
        self.scheduled = true;
        Some(Draw::In(self.draw_delay_us()))
    }

    /// Whether the event loop should wake this idle surface now.
    pub fn should_wake(&self, redraw_gen: u64, animations_pending: bool) -> bool {
        wake_idle_surface(
            self.scheduled,
            redraw_gen,
            self.seen_redraw_gen,
            animations_pending,
        )
    }

    /// Places the pass a wake-up asks for. `since_vblank_us` is the time
    /// since the surface's last VBlank, `None` before its first: then, or
    /// before the period is known, the pass runs right away. A deferred pass
    /// marks the surface scheduled; one run now marks itself as it ends.
    pub fn wake(&mut self, since_vblank_us: Option<f32>) -> Draw {
        let wait = since_vblank_us
            .filter(|_| self.period_us > 0.0)
            .map(|since| {
                let budget = draw_budget_us(self.avg_cpu_us, self.avg_gpu_us);
                wake_delay_us(since, self.period_us, self.draw_delay_us(), budget)
            });
        match wait {
            Some(wait) if wait > 0.0 => {
                self.scheduled = true;
                Draw::In(wait)
            }
            _ => Draw::Now,
        }
    }

    /// A pass starts and sees every request up to `redraw_gen`.
    pub fn begin_pass(&mut self, redraw_gen: u64) {
        self.seen_redraw_gen = redraw_gen;
    }

    /// A pass took `cpu_us`. Returns the delay of a follow-up pass when one
    /// is owed without a VBlank to ask for it ([`after_pass`]): nothing was
    /// drawn but an animation is in flight and `may_reschedule` allows it.
    pub fn end_pass(
        &mut self,
        cpu_us: f32,
        rendered: bool,
        may_reschedule: bool,
        animations_pending: bool,
    ) -> Option<f32> {
        self.avg_cpu_us = self.avg_cpu_us * 0.9 + cpu_us * 0.1;
        let (scheduled, reschedule) = after_pass(rendered, may_reschedule, animations_pending);
        self.scheduled = scheduled;
        reschedule.then(|| self.draw_delay_us())
    }

    /// A pass's composite took `gpu_us` on the GPU.
    pub fn record_gpu(&mut self, gpu_us: f32) {
        self.avg_gpu_us = gpu_estimate_us(self.avg_gpu_us, gpu_us);
    }

    /// Claims the surface for a pass scheduled from outside the pacer's own
    /// decisions (an output lagging behind the damage generation). Returns
    /// false, changing nothing, when a pass is already on its way.
    pub fn claim(&mut self) -> bool {
        !std::mem::replace(&mut self.scheduled, true)
    }
}

/// What the VBlank handler knows once the flipped frame is acknowledged.
struct VblankInputs {
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
fn wants_frame_after_vblank(i: &VblankInputs) -> bool {
    i.scene_has_damage
        || i.continuous_frames
        || i.seen_redraw_gen != i.redraw_gen
        || i.animations_pending
        || i.scanout_commit_pending
}

/// Whether the event loop kicks a pass for a surface right now. A surface
/// with a pass on its way is left alone: that pass runs after whatever just
/// happened and sees it, and a second one would double-render.
fn wake_idle_surface(
    frame_scheduled: bool,
    redraw_gen: u64,
    seen_redraw_gen: u64,
    animations_pending: bool,
) -> bool {
    !frame_scheduled && (animations_pending || seen_redraw_gen != redraw_gen)
}

/// The earliest a draw pass starts after a VBlank, in microseconds. Also the
/// floor that keeps a surface from spinning when nothing fits.
const MIN_DRAW_DELAY_US: f32 = 1_000.0;

/// Microseconds after a VBlank at which the draw pass starts.
///
/// The pass is scheduled as late as possible, so clients get most of the
/// period to paint and input is picked up close to the flip, but it must
/// still be *done* — CPU pass and the GPU work it submits — before the
/// VBlank it aims at, or the flip slips. The budget is that time doubled
/// (at least +1 ms) as the margin for variance.
///
/// A budget that fits one period aims at the next VBlank. One that does not
/// (a 120 Hz panel with a few ms of composite, a GPU shared with busy
/// clients) aims at the first VBlank it can make — every second one, say —
/// and still starts as late as that allows. Drawing at the floor instead
/// would flip whenever the GPU happens to finish, which lands on no
/// cadence at all and leaves clients, whose callbacks follow the draw,
/// almost nothing of the period to paint in: measured, that is 48 fps with
/// a 31 ms p95 against a steady 60.
fn draw_delay_us(frame_period_us: f32, avg_cpu_us: f32, avg_gpu_us: f32) -> f32 {
    let budget = draw_budget_us(avg_cpu_us, avg_gpu_us);
    let periods = (budget / frame_period_us).ceil().max(1.0);
    (periods * frame_period_us - budget).max(MIN_DRAW_DELAY_US)
}

/// The time a pass is given before the VBlank it aims at: its expected CPU
/// and GPU time, doubled (at least +1 ms) as the margin for variance.
fn draw_budget_us(avg_cpu_us: f32, avg_gpu_us: f32) -> f32 {
    let need = avg_cpu_us + avg_gpu_us;
    need + need.max(MIN_DRAW_DELAY_US)
}

/// How long an idle surface waits before the pass a wake-up asks for,
/// `since_vblank_us` after its last VBlank.
///
/// VBlanks keep their rhythm while the surface idles, so the pass is placed
/// where the deadline would have put it: at `draw_delay_us` into the
/// current period when that is still ahead, right away when the deadline
/// has passed but the budget still fits before the VBlank, and otherwise
/// at the next period's deadline. Drawing the moment a commit arrives
/// instead sends the client its frame callback while the composite is
/// still on the GPU; the two then slow each other, the client's next
/// commit lands just after the VBlank, and the output settles at half
/// rate — measured as a flat 60 on a 120 Hz panel, with the client's
/// frame costing half as much again.
fn wake_delay_us(
    since_vblank_us: f32,
    frame_period_us: f32,
    draw_delay_us: f32,
    budget_us: f32,
) -> f32 {
    let phase = since_vblank_us.rem_euclid(frame_period_us);
    if phase <= draw_delay_us {
        draw_delay_us - phase
    } else if phase + budget_us <= frame_period_us {
        0.0
    } else {
        frame_period_us - phase + draw_delay_us
    }
}

/// The running estimate of a pass's GPU time after a new measurement.
///
/// Rises fast and falls slowly: a pass that took longer than expected is
/// evidence the next may too, and missing a VBlank costs a whole frame,
/// while a short one is only slowly trusted. The CPU estimate uses a plain
/// mean, its variance being far lower.
fn gpu_estimate_us(avg_us: f32, sample_us: f32) -> f32 {
    if sample_us > avg_us {
        avg_us + (sample_us - avg_us) * 0.5
    } else {
        avg_us * 0.9 + sample_us * 0.1
    }
}

/// What follows a pass: `(frame_scheduled, reschedule_timer)`.
///
/// A queued frame's VBlank decides what comes next. A pass that drew
/// nothing continues on a timer only for an animation still in flight —
/// anything else that needs a frame arrives as an event and wakes the
/// surface through the loop.
fn after_pass(rendered: bool, may_reschedule: bool, animations_pending: bool) -> (bool, bool) {
    let reschedule = may_reschedule && !rendered && animations_pending;
    (rendered || reschedule, reschedule)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One output driven through the real pacer, with the VBlank-scheduled
    /// pass modelled as "runs before the next VBlank". `sim` measures timing;
    /// this checks only which passes happen.
    struct Sim {
        pacer: FramePacer,
        redraw_gen: u64,
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
                pacer: FramePacer::new(),
                redraw_gen: 0,
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
            if self
                .pacer
                .should_wake(self.redraw_gen, self.animations_pending)
            {
                // No VBlank seen: the wake-up draws at once.
                assert_eq!(self.pacer.wake(None), Draw::Now);
                self.pass();
            }
        }

        /// The draw pass: sees everything requested so far, flips when there
        /// is something to show.
        fn pass(&mut self) {
            self.pass_pending = false;
            self.passes += 1;
            self.pacer.begin_pass(self.redraw_gen);
            let rendered = self.scene_damage || self.scanout_commit;
            self.scene_damage = false;
            self.scanout_commit = false;
            let reschedule = self
                .pacer
                .end_pass(1_000.0, rendered, true, self.animations_pending);
            if rendered {
                self.in_flight = true;
                self.flips += 1;
            } else if reschedule.is_some() {
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
            let draw = self.pacer.on_vblank(&VblankReasons {
                scene_has_damage: self.scene_damage,
                continuous_frames: self.continuous_frames,
                redraw_gen: self.redraw_gen,
                animations_pending: self.animations_pending,
                scanout_commit_pending: self.scanout_commit,
            });
            if draw.is_some() {
                // The deadline timer fires before the next VBlank; anything
                // that happens in between is seen by this pass either way.
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
        assert!(!sim.pacer.is_scheduled());
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
        assert!(!sim.pacer.is_scheduled());
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

    /// The draw starts late enough to keep latency low at 60 Hz, fits a
    /// light frame into one 120 Hz period, and aims a heavy one at every
    /// second VBlank — late, not at the floor.
    #[test]
    fn draw_deadline_counts_the_gpu() {
        let at_60 = 1_000_000.0 / 60.0;
        let at_120 = 1_000_000.0 / 120.0;
        let close = |a: f32, b: f32| (a - b).abs() < 1e-2;
        // Without a GPU measurement the deadline is the CPU-only one.
        assert!(close(draw_delay_us(at_60, 1_500.0, 0.0), at_60 - 3_000.0));
        // 1.5 ms of CPU and 3 ms of GPU: still 7.7 ms of slack at 60 Hz …
        assert!(close(
            draw_delay_us(at_60, 1_500.0, 3_000.0),
            at_60 - 9_000.0
        ));
        // … and at 120 Hz a 9 ms budget does not fit one period: aim at the
        // second VBlank, 7.7 ms in. The CPU-only deadline would have
        // started 5.3 ms in and slipped every frame.
        assert!(close(
            draw_delay_us(at_120, 1_500.0, 3_000.0),
            2.0 * at_120 - 9_000.0
        ));
        assert!(draw_delay_us(at_120, 1_500.0, 0.0) > 5_000.0);
        // A light 120 Hz frame (0.3 ms CPU, 2 ms GPU) fits one period with
        // its margin and starts 3.7 ms in.
        assert!(close(
            draw_delay_us(at_120, 300.0, 2_000.0),
            at_120 - 4_600.0
        ));
        // Tiny frames still get the 1 ms margin and never dip under the floor.
        assert!(close(draw_delay_us(at_120, 100.0, 100.0), at_120 - 1_200.0));
        // A budget that is exactly whole periods would start at the VBlank
        // itself; the floor keeps it 1 ms in. Heavier budgets just take more
        // periods and stay at or above the floor.
        assert_eq!(
            draw_delay_us(at_120, at_120 / 4.0, at_120 / 4.0),
            MIN_DRAW_DELAY_US
        );
        assert!(draw_delay_us(at_120, 8_000.0, 8_000.0) >= MIN_DRAW_DELAY_US);
        assert!(draw_delay_us(at_120, 8_000.0, 8_000.0) < at_120);
    }

    /// A wake-up lands on the period's deadline, goes right away only when
    /// the budget still fits before the VBlank, and otherwise waits for the
    /// next deadline.
    #[test]
    fn wake_up_keeps_the_vblank_rhythm() {
        let period = 1_000_000.0 / 120.0;
        let delay = 3_000.0;
        let budget = 4_000.0;
        // 1 ms into the period: wait the remaining 2 ms to the deadline.
        assert!(close_to(
            wake_delay_us(1_000.0, period, delay, budget),
            2_000.0
        ));
        // 4 ms in: the deadline passed, 4 ms of budget still fits — go now.
        assert_eq!(wake_delay_us(4_000.0, period, delay, budget), 0.0);
        // 6 ms in: too late for this VBlank, aim at the next deadline.
        assert!(close_to(
            wake_delay_us(6_000.0, period, delay, budget),
            period - 6_000.0 + delay
        ));
        // A long idle keeps the rhythm: 100 periods plus 1 ms is 1 ms in
        // (within f32 precision at that magnitude).
        let after_idle = wake_delay_us(100.0 * period + 1_000.0, period, delay, budget);
        assert!((after_idle - 2_000.0).abs() < 1.0, "{after_idle}");
    }

    /// A long pass raises the GPU estimate at once; a short one lowers it slowly.
    #[test]
    fn gpu_estimate_rises_fast_and_falls_slowly() {
        assert_eq!(gpu_estimate_us(2_000.0, 6_000.0), 4_000.0);
        assert!(close_to(gpu_estimate_us(6_000.0, 2_000.0), 5_600.0));
        assert!(close_to(gpu_estimate_us(0.0, 0.0), 0.0));
    }

    /// A wake-up before the first VBlank, or before the mode is known, draws
    /// at once; after one it keeps the rhythm and claims the surface.
    #[test]
    fn a_wake_up_is_placed_on_the_rhythm_once_there_is_one() {
        let mut pacer = FramePacer::new();
        assert_eq!(pacer.wake(Some(1_000.0)), Draw::Now, "no period yet");
        pacer.set_refresh_mhz(60_000);
        assert_eq!(pacer.wake(None), Draw::Now, "no VBlank yet");
        assert!(!pacer.is_scheduled());
        let Draw::In(wait) = pacer.wake(Some(1_000.0)) else {
            panic!("1 ms into the period the deadline is still ahead");
        };
        assert!(close_to(wait, pacer.draw_delay_us() - 1_000.0));
        assert!(pacer.is_scheduled());
        assert!(!pacer.claim(), "already claimed by the wake-up");
    }

    #[test]
    fn an_unknown_period_never_spins() {
        let mut pacer = FramePacer::new();
        assert_eq!(
            pacer.end_pass(1_000.0, false, true, true),
            Some(MIN_DRAW_DELAY_US)
        );
        pacer.set_refresh_mhz(0);
        assert_eq!(pacer.draw_delay_us(), MIN_DRAW_DELAY_US);
    }

    /// The tick paces clients at VBlanks with no flip only while the
    /// compositor's own need is under 60 % of the period.
    #[test]
    fn keeping_up_is_the_compositors_need_not_its_rate() {
        let mut pacer = FramePacer::new();
        pacer.set_refresh_mhz(120_000);
        for _ in 0..100 {
            pacer.end_pass(1_000.0, true, true, false);
        }
        // The GPU estimate moves halfway to a longer sample.
        pacer.record_gpu(4_000.0);
        assert!(pacer.keeping_up(), "1 + 2 ms of 8.3");
        pacer.record_gpu(10_000.0);
        assert!(!pacer.keeping_up(), "1 + 6 ms of 8.3");
    }

    #[test]
    fn a_commit_is_answered_once_most_of_a_period_has_passed() {
        let period = 1_000_000.0 / 120.0;
        assert!(!callback_due_on_commit(true, 7_000.0, period));
        assert!(callback_due_on_commit(true, 8_000.0, period));
        assert!(!callback_due_on_commit(false, 20_000.0, period));
    }

    fn close_to(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-2
    }
}
