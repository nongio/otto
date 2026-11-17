//! A deterministic frame-pacing bench: the real [`FramePacer`] driven by a
//! virtual display.
//!
//! The display scans out a mode read from an EDID ([`super::edid`]) with a
//! perfect VBlank every period, the way vkms or a panel would, and flips a
//! queued frame at the first VBlank after its GPU work is done. Around it
//! the simulator plays the parts of the udev backend that decide timing and
//! nothing else: one single-threaded event loop (a pass blocks it for its
//! CPU time), the deadline, wake and reschedule timers, the VBlank-cadence
//! callback ticks, GPU fences, callbacks on commit, and clients that paint
//! when called back (or on their own clock, like a video player).
//!
//! Costs are drawn from a seeded generator, so a scenario gives the same
//! numbers on every run and every machine: a change in them is a change in
//! the pacing code, not noise. What it does not model: GPU contention
//! between the clients and the compositor, plane scanout, several outputs,
//! VRR, and the scene work a VBlank does before arming the deadline.
//!
//! `cargo bench --bench frame_pacing` runs [`standard`] and prints a table.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use serde::{Deserialize, Serialize};

use super::edid::{self, DisplayMode};
use super::{callback_due_on_commit, Draw, FramePacer, VblankReasons};

/// The share of a pass's CPU time one that draws nothing costs: it still
/// rebuilds the elements and reconsiders the planes.
const EMPTY_PASS_COST: f64 = 0.25;

/// What one piece of work costs, in microseconds: `base`, plus up to
/// `jitter` drawn uniformly, plus `spike` every `spike_every`-th time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cost {
    pub base_us: f64,
    pub jitter_us: f64,
    pub spike_us: f64,
    pub spike_every: u64,
}

impl Cost {
    pub const ZERO: Cost = Cost::fixed(0.0);

    pub const fn fixed(us: f64) -> Self {
        Self {
            base_us: us,
            jitter_us: 0.0,
            spike_us: 0.0,
            spike_every: 0,
        }
    }

    /// `base_ms` plus up to `jitter_ms`.
    pub const fn ms(base_ms: f64, jitter_ms: f64) -> Self {
        Self {
            base_us: base_ms * 1000.0,
            jitter_us: jitter_ms * 1000.0,
            spike_us: 0.0,
            spike_every: 0,
        }
    }

    /// The same cost, `spike_ms` more every `every`-th time.
    pub const fn spiking(self, spike_ms: f64, every: u64) -> Self {
        Self {
            spike_us: spike_ms * 1000.0,
            spike_every: every,
            ..self
        }
    }

    fn sample(&self, rng: &mut Rng, n: u64) -> f64 {
        let spike = if self.spike_every > 0 && n % self.spike_every == self.spike_every - 1 {
            self.spike_us
        } else {
            0.0
        };
        self.base_us + rng.unit() * self.jitter_us + spike
    }
}

/// How a client decides to paint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClientPacing {
    /// On each frame callback, like a toolkit (GTK, Qt, otto-kit).
    Callbacks,
    /// On its own clock, like a video player at the content's rate.
    Fixed { fps: f64 },
}

/// A client window: how it is paced and what a frame costs it from the
/// callback (or its clock) to the commit being applied, its GPU work
/// included (the compositor applies a commit once the buffer is ready).
#[derive(Debug, Clone)]
pub struct Client {
    pub name: String,
    pub pacing: ClientPacing,
    pub frame: Cost,
}

impl Client {
    pub fn toolkit(name: &str, frame: Cost) -> Self {
        Self {
            name: name.to_string(),
            pacing: ClientPacing::Callbacks,
            frame,
        }
    }

    pub fn video(name: &str, fps: f64, frame: Cost) -> Self {
        Self {
            name: name.to_string(),
            pacing: ClientPacing::Fixed { fps },
            frame,
        }
    }
}

/// A lay-rs animation (a transition, exposé): the scene is damaged on every
/// pass while it runs.
#[derive(Debug, Clone, Copy)]
pub struct Animation {
    pub start_ms: f64,
    pub len_ms: f64,
}

/// One bench run.
#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    pub display: DisplayMode,
    pub duration_ms: f64,
    /// Nothing before this counts: the estimates are still settling.
    pub warmup_ms: f64,
    /// The CPU time of a pass that draws.
    pub compositor_cpu: Cost,
    /// The GPU time of its composite.
    pub compositor_gpu: Cost,
    pub clients: Vec<Client>,
    pub animations: Vec<Animation>,
    /// Pointer motion events per second (each moves the cursor), 0 for none.
    pub pointer_hz: f64,
    pub seed: u64,
}

impl Scenario {
    /// A scenario on a synthesized `width`×`height` panel at `hz`, with
    /// nothing on it and a light compositor.
    pub fn on_panel(name: &str, width: u32, height: u32, hz: f64) -> Self {
        let edid = edid::synthesize("Otto Virtual", width, height, hz);
        Self::on_display(name, edid::parse(&edid).expect("synthesized EDID parses"))
    }

    pub fn on_display(name: &str, display: DisplayMode) -> Self {
        Self {
            name: name.to_string(),
            display,
            duration_ms: 5_000.0,
            warmup_ms: 250.0,
            compositor_cpu: Cost::ms(1.0, 0.3),
            compositor_gpu: Cost::ms(1.0, 0.3),
            clients: Vec::new(),
            animations: Vec::new(),
            pointer_hz: 0.0,
            seed: 0x07_70_0F_7A,
        }
    }

    pub fn compositor(mut self, cpu: Cost, gpu: Cost) -> Self {
        self.compositor_cpu = cpu;
        self.compositor_gpu = gpu;
        self
    }

    pub fn client(mut self, client: Client) -> Self {
        self.clients.push(client);
        self
    }

    pub fn animation(mut self, start_ms: f64, len_ms: f64) -> Self {
        self.animations.push(Animation { start_ms, len_ms });
        self
    }

    pub fn pointer(mut self, hz: f64) -> Self {
        self.pointer_hz = hz;
        self
    }

    /// Runs it.
    pub fn run(&self) -> Report {
        Sim::new(self).run()
    }
}

/// The bench's scenarios, each a case `specs/frame-pacing.md` makes a
/// promise about.
pub fn standard() -> Vec<Scenario> {
    vec![
        // An idle desktop costs nothing once a short transition is over.
        Scenario::on_panel("idle-after-transition", 1920, 1080, 60.0).animation(100.0, 300.0),
        // A toolkit window with time to spare is shown on every VBlank.
        Scenario::on_panel("toolkit-60", 1920, 1080, 60.0)
            .compositor(Cost::ms(1.5, 0.3), Cost::ms(1.0, 0.3))
            .client(Client::toolkit("gtk", Cost::ms(4.0, 1.0))),
        // GTK at 120 Hz: about 7 ms of an 8.3 ms period for its frame.
        Scenario::on_panel("toolkit-heavy-120", 2560, 1440, 120.0)
            .compositor(Cost::ms(0.8, 0.2), Cost::ms(1.2, 0.3))
            .client(Client::toolkit("gtk", Cost::ms(6.6, 0.6))),
        // A fast client gets its frame on the next VBlank.
        Scenario::on_panel("toolkit-fast-144", 2560, 1440, 144.0)
            .compositor(Cost::ms(0.6, 0.1), Cost::ms(0.8, 0.2))
            .client(Client::toolkit("foot", Cost::ms(1.0, 0.3))),
        // Five windows the compositor cannot compose every period.
        Scenario::on_panel("five-windows-120", 2560, 1440, 120.0)
            .compositor(Cost::ms(2.5, 0.5), Cost::ms(5.0, 1.0))
            .client(Client::toolkit("a", Cost::ms(3.0, 1.0)))
            .client(Client::toolkit("b", Cost::ms(3.0, 1.0)))
            .client(Client::toolkit("c", Cost::ms(3.0, 1.0)))
            .client(Client::toolkit("d", Cost::ms(3.0, 1.0)))
            .client(Client::toolkit("e", Cost::ms(3.0, 1.0))),
        // 30 fps content on a 120 Hz panel: 30 passes a second, evenly spaced.
        Scenario::on_panel("video-30-on-120", 2560, 1440, 120.0)
            .compositor(Cost::ms(0.8, 0.2), Cost::ms(1.5, 0.3))
            .client(Client::video("mpv", 30.0, Cost::ms(2.0, 0.5))),
        // Film on a 60 Hz panel: 3:2 cadence is the content's, not ours.
        Scenario::on_panel("film-24-on-60", 1920, 1080, 60.0).client(Client::video(
            "mpv",
            23.976,
            Cost::ms(2.0, 0.5),
        )),
        // A GPU stall every half second must not knock the deadline off for
        // longer than it lasts.
        Scenario::on_panel("gpu-spikes-60", 1920, 1080, 60.0)
            .compositor(Cost::ms(1.5, 0.3), Cost::ms(2.0, 0.5).spiking(12.0, 30))
            .client(Client::toolkit("gtk", Cost::ms(5.0, 1.0))),
        // A 1000 Hz mouse over a static scene: one pass per refresh.
        Scenario::on_panel("pointer-1000hz-120", 2560, 1440, 120.0)
            .compositor(Cost::ms(0.5, 0.1), Cost::ms(0.5, 0.1))
            .pointer(1000.0),
        // Exposé on a 165 Hz panel.
        Scenario::on_panel("animation-165", 1920, 1080, 165.0)
            .compositor(Cost::ms(1.0, 0.3), Cost::ms(1.2, 0.3))
            .animation(500.0, 3000.0),
        // The same with 4 ms of compose: its doubled budget does not fit a
        // 6 ms period, so the deadline aims at every second VBlank — a
        // steady half rate, by design, rather than flips on no cadence.
        Scenario::on_panel("heavy-animation-165", 1920, 1080, 165.0)
            .compositor(Cost::ms(1.5, 0.4), Cost::ms(2.0, 0.5))
            .animation(500.0, 3000.0),
    ]
}

/// The bench's scenarios with every display replaced by `display`, for a
/// panel's own EDID (`--edid /sys/class/drm/card1-eDP-1/edid`).
pub fn standard_on(display: &DisplayMode) -> Vec<Scenario> {
    standard()
        .into_iter()
        .map(|mut s| {
            s.display = display.clone();
            s
        })
        .collect()
}

/// p50, p95, p99 and the largest of a set of samples, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Spread {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Spread {
    fn of_us(samples: &[f64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut s: Vec<f64> = samples.to_vec();
        s.sort_by(f64::total_cmp);
        let at = |q: f64| s[((s.len() - 1) as f64 * q).round() as usize] / 1000.0;
        Self {
            p50: at(0.5),
            p95: at(0.95),
            p99: at(0.99),
            max: at(1.0),
        }
    }
}

/// What one client saw.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientReport {
    pub name: String,
    /// New frames of this client on screen per second.
    pub fps: f64,
    /// Time between two of its frames reaching the screen.
    pub interval_ms: Spread,
    /// From its commit being applied to the VBlank that showed it.
    pub latency_ms: Spread,
    /// Frames it painted that a newer one replaced before any compose.
    pub superseded: u64,
    /// Cadence breaks: a frame interval more than half a period off the
    /// one before it.
    pub cadence_breaks: u64,
}

/// What a scenario measured, after its warm-up.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub scenario: String,
    /// `1920x1080@59.95`.
    pub display: String,
    pub seconds: f64,
    /// Frames flipped per second.
    pub flips_per_s: f64,
    /// Draw passes per second, and how many of those drew nothing.
    pub passes_per_s: f64,
    pub empty_passes_per_s: f64,
    /// Flips that landed on a later VBlank than the pass aimed at.
    pub missed_vblanks: u64,
    /// Time between flips.
    pub flip_interval_ms: Spread,
    pub clients: Vec<ClientReport>,
}

/// xorshift64*: small, fast and the same everywhere.
struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Event {
    /// A queued frame is on screen at this VBlank.
    Flip,
    /// The composite's fence signalled, `submitted_ns` after the pass began
    /// submitting.
    GpuDone {
        submitted_ns: u64,
    },
    /// A deadline, wake or reschedule timer: the pass runs.
    Pass,
    /// The VBlank-cadence callback tick armed by flip number `armed`.
    Tick {
        armed: u64,
    },
    /// A client's commit is applied.
    Commit {
        client: usize,
    },
    /// A fixed-rate client's clock: it starts a frame.
    ClientClock {
        client: usize,
    },
    Pointer,
    /// Nothing but a loop turn (an animation starting or ending).
    Wake,
}

struct ClientState {
    /// A frame callback is requested and not yet done.
    waiting_callback: bool,
    /// When the last VBlank-paced (or on-commit) callback went out.
    last_sent_us: Option<f64>,
    /// The applied commit no compose has taken yet.
    pending_commit_us: Option<f64>,
    frames: u64,
    presented_us: Vec<f64>,
    latency_us: Vec<f64>,
    superseded: u64,
}

struct Frame {
    /// `(client, commit time)` of every buffer in it.
    latched: Vec<(usize, f64)>,
    /// The VBlank index the pass expected to make.
    aimed_vblank: u64,
}

struct Sim<'a> {
    sc: &'a Scenario,
    rng: Rng,
    period_us: f64,
    /// The event loop is inside a pass until then.
    busy_until_us: f64,
    queue: BinaryHeap<Reverse<(u64, u64, Event)>>,
    seq: u64,
    pacer: FramePacer,
    redraw_gen: u64,
    last_vblank_us: Option<f64>,
    flips: u64,
    in_flight: Option<Frame>,
    cursor_damage: bool,
    passes: u64,
    gpu_runs: u64,
    clients: Vec<ClientState>,
    // Measured after the warm-up.
    counted_passes: u64,
    counted_empty: u64,
    missed: u64,
    flip_times_us: Vec<f64>,
}

impl<'a> Sim<'a> {
    fn new(sc: &'a Scenario) -> Self {
        let mut pacer = FramePacer::new();
        pacer.set_refresh_mhz(sc.display.refresh_mhz);
        Self {
            sc,
            rng: Rng(sc.seed | 1),
            period_us: sc.display.period_us(),
            busy_until_us: 0.0,
            queue: BinaryHeap::new(),
            seq: 0,
            pacer,
            redraw_gen: 0,
            last_vblank_us: None,
            flips: 0,
            in_flight: None,
            cursor_damage: false,
            passes: 0,
            gpu_runs: 0,
            clients: sc
                .clients
                .iter()
                .map(|_| ClientState {
                    waiting_callback: false,
                    last_sent_us: None,
                    pending_commit_us: None,
                    frames: 0,
                    presented_us: Vec::new(),
                    latency_us: Vec::new(),
                    superseded: 0,
                })
                .collect(),
            counted_passes: 0,
            counted_empty: 0,
            missed: 0,
            flip_times_us: Vec::new(),
        }
    }

    fn at(&mut self, t_us: f64, event: Event) {
        self.seq += 1;
        let ns = (t_us * 1000.0).round().max(0.0) as u64;
        self.queue.push(Reverse((ns, self.seq, event)));
    }

    fn counted(&self, t_us: f64) -> bool {
        t_us >= self.sc.warmup_ms * 1000.0
    }

    fn animating(&self, t_us: f64) -> bool {
        let ms = t_us / 1000.0;
        self.sc
            .animations
            .iter()
            .any(|a| ms >= a.start_ms && ms < a.start_ms + a.len_ms)
    }

    /// The index of the first VBlank at or after `t_us`.
    fn vblank_at_or_after(&self, t_us: f64) -> u64 {
        (t_us / self.period_us - 1e-9).ceil().max(0.0) as u64
    }

    fn run(mut self) -> Report {
        let end_us = self.sc.duration_ms * 1000.0;
        for (i, client) in self.sc.clients.iter().enumerate() {
            match client.pacing {
                ClientPacing::Callbacks => {
                    // The first frame is painted unasked; its callback comes
                    // with the first VBlank send.
                    self.start_frame(i, 0.0);
                }
                ClientPacing::Fixed { .. } => self.at(0.0, Event::ClientClock { client: i }),
            }
        }
        if self.sc.pointer_hz > 0.0 {
            self.at(0.0, Event::Pointer);
        }
        for a in &self.sc.animations {
            self.at(a.start_ms * 1000.0, Event::Wake);
            self.at((a.start_ms + a.len_ms) * 1000.0, Event::Wake);
        }

        while let Some(Reverse((ns, _, event))) = self.queue.pop() {
            let at_us = ns as f64 / 1000.0;
            if at_us > end_us {
                break;
            }
            // One thread: an event that comes due during a pass is handled
            // after it.
            let now = at_us.max(self.busy_until_us);
            match event {
                Event::Flip => self.flip(at_us, now),
                Event::GpuDone { submitted_ns } => {
                    self.pacer
                        .record_gpu((now - submitted_ns as f64 / 1000.0) as f32);
                }
                Event::Pass => self.pass(now),
                Event::Tick { armed } => {
                    if armed == self.flips && self.pacer.keeping_up() {
                        self.send_callbacks(now);
                    }
                }
                Event::Commit { client } => self.commit(client, now),
                Event::ClientClock { client } => {
                    if let ClientPacing::Fixed { fps } = self.sc.clients[client].pacing {
                        self.start_frame(client, now);
                        // The clock keeps its own time, not the loop's.
                        self.at(at_us + 1_000_000.0 / fps, Event::ClientClock { client });
                    }
                }
                Event::Pointer => {
                    self.cursor_damage = true;
                    self.redraw_gen += 1;
                    self.at(at_us + 1_000_000.0 / self.sc.pointer_hz, Event::Pointer);
                }
                Event::Wake => {}
            }
            self.loop_turn(self.busy_until_us.max(now));
        }
        self.report(end_us)
    }

    /// After each dispatch the loop wakes an idle surface that is owed a
    /// pass (`run_udev`'s loop, `render_on_wake`).
    fn loop_turn(&mut self, now: f64) {
        if !self.pacer.should_wake(self.redraw_gen, self.animating(now)) {
            return;
        }
        let since = self.last_vblank_us.map(|v| (now - v) as f32);
        match self.pacer.wake(since) {
            Draw::Now => self.pass(now),
            Draw::In(us) => self.at(now + us as f64, Event::Pass),
        }
    }

    /// `render_surface`: compose what is new, queue it, and say what follows.
    fn pass(&mut self, now: f64) {
        self.passes += 1;
        self.pacer.begin_pass(self.redraw_gen);
        let animating = self.animating(now);
        let has_commits = self.clients.iter().any(|c| c.pending_commit_us.is_some());
        let rendered = self.in_flight.is_none() && (has_commits || self.cursor_damage || animating);
        let full_cpu = self.sc.compositor_cpu.sample(&mut self.rng, self.passes);
        let cpu = if rendered {
            full_cpu
        } else {
            full_cpu * EMPTY_PASS_COST
        };
        let end = now + cpu;
        self.busy_until_us = end;
        if self.counted(now) {
            self.counted_passes += 1;
            if !rendered {
                self.counted_empty += 1;
            }
        }
        if rendered {
            let expected = (self.pacer.avg_cpu_us() + self.pacer.avg_gpu_us()) as f64;
            let aimed_vblank = self.vblank_at_or_after(now + expected);
            let latched = self
                .clients
                .iter_mut()
                .enumerate()
                .filter_map(|(i, c)| c.pending_commit_us.take().map(|t| (i, t)))
                .collect();
            self.cursor_damage = false;
            self.gpu_runs += 1;
            let gpu = self.sc.compositor_gpu.sample(&mut self.rng, self.gpu_runs);
            let done = end + gpu;
            self.at(
                done,
                Event::GpuDone {
                    submitted_ns: (end * 1000.0) as u64,
                },
            );
            let flip_vblank = self.vblank_at_or_after(done);
            self.at(flip_vblank as f64 * self.period_us, Event::Flip);
            self.in_flight = Some(Frame {
                latched,
                aimed_vblank,
            });
        }
        let animating_after = self.animating(end);
        if let Some(us) = self
            .pacer
            .end_pass(cpu as f32, rendered, true, animating_after)
        {
            self.at(end + us as f64, Event::Pass);
        }
    }

    /// `frame_finish`: the frame is on screen at `vblank_us`; the handler
    /// runs at `now`.
    fn flip(&mut self, vblank_us: f64, now: f64) {
        let Some(frame) = self.in_flight.take() else {
            return;
        };
        let index = (vblank_us / self.period_us).round() as u64;
        if self.counted(vblank_us) {
            self.flip_times_us.push(vblank_us);
            if index > frame.aimed_vblank {
                self.missed += 1;
            }
        }
        for (client, commit_us) in frame.latched {
            let c = &mut self.clients[client];
            c.presented_us.push(vblank_us);
            c.latency_us.push(vblank_us - commit_us);
        }
        self.flips += 1;
        self.last_vblank_us = Some(now);
        self.send_callbacks(now);
        for n in 1..=2 {
            self.at(
                now + n as f64 * self.period_us,
                Event::Tick { armed: self.flips },
            );
        }
        let reasons = VblankReasons {
            scene_has_damage: self.animating(now),
            continuous_frames: false,
            redraw_gen: self.redraw_gen,
            animations_pending: self.animating(now),
            scanout_commit_pending: false,
        };
        match self.pacer.on_vblank(&reasons) {
            Some(Draw::In(us)) => self.at(now + us as f64, Event::Pass),
            Some(Draw::Now) => self.pass(now),
            None => {}
        }
    }

    /// `send_frame_callbacks`: every callback-paced client is served.
    fn send_callbacks(&mut self, now: f64) {
        for i in 0..self.clients.len() {
            if self.sc.clients[i].pacing != ClientPacing::Callbacks {
                continue;
            }
            let c = &mut self.clients[i];
            c.last_sent_us = Some(now);
            if std::mem::take(&mut c.waiting_callback) {
                self.start_frame(i, now);
            }
        }
    }

    fn start_frame(&mut self, client: usize, now: f64) {
        let c = &mut self.clients[client];
        c.frames += 1;
        let cost = self.sc.clients[client]
            .frame
            .sample(&mut self.rng, c.frames);
        self.at(now + cost, Event::Commit { client });
    }

    /// A client's commit is applied: a redraw request, and for a toolkit a
    /// new frame request, answered at once if the surface has waited most of
    /// a period (`frame_callback_on_commit`).
    fn commit(&mut self, client: usize, now: f64) {
        let counted = self.counted(now);
        let c = &mut self.clients[client];
        if c.pending_commit_us.replace(now).is_some() && counted {
            c.superseded += 1;
        }
        self.redraw_gen += 1;
        if self.sc.clients[client].pacing != ClientPacing::Callbacks {
            return;
        }
        c.waiting_callback = true;
        let due = c.last_sent_us.is_some_and(|sent| {
            callback_due_on_commit(true, (now - sent) as f32, self.period_us as f32)
        });
        if due {
            c.last_sent_us = Some(now);
            c.waiting_callback = false;
            self.start_frame(client, now);
        }
    }

    fn report(self, end_us: f64) -> Report {
        let warmup_us = self.sc.warmup_ms * 1000.0;
        let seconds = (end_us - warmup_us) / 1_000_000.0;
        let period = self.period_us;
        let intervals =
            |times: &[f64]| -> Vec<f64> { times.windows(2).map(|w| w[1] - w[0]).collect() };
        let clients = self
            .sc
            .clients
            .iter()
            .zip(&self.clients)
            .map(|(spec, c)| {
                let shown: Vec<f64> = c
                    .presented_us
                    .iter()
                    .copied()
                    .filter(|&t| t >= warmup_us)
                    .collect();
                let latency: Vec<f64> = c
                    .presented_us
                    .iter()
                    .zip(&c.latency_us)
                    .filter(|(&t, _)| t >= warmup_us)
                    .map(|(_, &l)| l)
                    .collect();
                let iv = intervals(&shown);
                let cadence_breaks = iv
                    .windows(2)
                    .filter(|w| (w[1] - w[0]).abs() > period / 2.0)
                    .count() as u64;
                ClientReport {
                    name: spec.name.clone(),
                    fps: shown.len() as f64 / seconds,
                    interval_ms: Spread::of_us(&iv),
                    latency_ms: Spread::of_us(&latency),
                    superseded: c.superseded,
                    cadence_breaks,
                }
            })
            .collect();
        Report {
            scenario: self.sc.name.clone(),
            display: self.sc.display.label(),
            seconds,
            flips_per_s: self.flip_times_us.len() as f64 / seconds,
            passes_per_s: self.counted_passes as f64 / seconds,
            empty_passes_per_s: self.counted_empty as f64 / seconds,
            missed_vblanks: self.missed,
            flip_interval_ms: Spread::of_us(&intervals(&self.flip_times_us)),
            clients,
        }
    }
}

/// A Markdown table of `reports`, one row per client (or per scenario when it
/// has none).
pub fn markdown(reports: &[Report]) -> String {
    let mut out = String::from(
        "| scenario | display | flips/s | passes/s (empty) | missed | flip p99 ms | client | fps | interval p50/p99 ms | latency p50/p99 ms | superseded | cadence breaks |\n\
         |---|---|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|\n",
    );
    for r in reports {
        let head = format!(
            "| {} | {} | {:.1} | {:.1} ({:.1}) | {} | {:.2} |",
            r.scenario,
            r.display,
            r.flips_per_s,
            r.passes_per_s,
            r.empty_passes_per_s,
            r.missed_vblanks,
            r.flip_interval_ms.p99
        );
        if r.clients.is_empty() {
            out.push_str(&head);
            out.push_str(" – | | | | | |\n");
        }
        for c in &r.clients {
            out.push_str(&format!(
                "{head} {} | {:.1} | {:.2} / {:.2} | {:.2} / {:.2} | {} | {} |\n",
                c.name,
                c.fps,
                c.interval_ms.p50,
                c.interval_ms.p99,
                c.latency_ms.p50,
                c.latency_ms.p99,
                c.superseded,
                c.cadence_breaks
            ));
        }
    }
    out
}

/// How `head` is worse than `base`, scenario by scenario and client by
/// client: one line per regression, empty when none.
///
/// The runs are deterministic, so any difference is the code's; the
/// tolerances only absorb float rounding across toolchains. Worse is fewer
/// frames on screen, more missed VBlanks or cadence breaks, more superseded
/// client frames, a slower p99 latency, or more passes for the same content.
pub fn regressions(base: &[Report], head: &[Report]) -> Vec<String> {
    let mut out = Vec::new();
    for h in head {
        let Some(b) = base.iter().find(|b| b.scenario == h.scenario) else {
            continue;
        };
        let s = &h.scenario;
        if h.display != b.display {
            out.push(format!(
                "{s}: display changed, {} → {}",
                b.display, h.display
            ));
            continue;
        }
        if h.flips_per_s < b.flips_per_s - 0.5 {
            out.push(format!(
                "{s}: flips/s {:.1} → {:.1}",
                b.flips_per_s, h.flips_per_s
            ));
        }
        if h.passes_per_s > b.passes_per_s + 1.0 {
            out.push(format!(
                "{s}: passes/s {:.1} → {:.1}",
                b.passes_per_s, h.passes_per_s
            ));
        }
        if h.missed_vblanks > b.missed_vblanks {
            out.push(format!(
                "{s}: missed VBlanks {} → {}",
                b.missed_vblanks, h.missed_vblanks
            ));
        }
        for hc in &h.clients {
            let Some(bc) = b.clients.iter().find(|c| c.name == hc.name) else {
                continue;
            };
            let c = &hc.name;
            if hc.fps < bc.fps - 0.5 {
                out.push(format!("{s}/{c}: fps {:.1} → {:.1}", bc.fps, hc.fps));
            }
            if hc.cadence_breaks > bc.cadence_breaks {
                out.push(format!(
                    "{s}/{c}: cadence breaks {} → {}",
                    bc.cadence_breaks, hc.cadence_breaks
                ));
            }
            if hc.superseded > bc.superseded {
                out.push(format!(
                    "{s}/{c}: superseded frames {} → {}",
                    bc.superseded, hc.superseded
                ));
            }
            if hc.latency_ms.p99 > bc.latency_ms.p99 + 0.5 {
                out.push(format!(
                    "{s}/{c}: latency p99 {:.2} ms → {:.2} ms",
                    bc.latency_ms.p99, hc.latency_ms.p99
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str) -> Report {
        standard()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no scenario {name}"))
            .run()
    }

    #[test]
    fn a_worse_run_is_reported() {
        let base = vec![run("gpu-spikes-60")];
        assert!(regressions(&base, &base).is_empty());
        let mut head = base.clone();
        head[0].missed_vblanks += 1;
        head[0].clients[0].fps -= 2.0;
        let found = regressions(&base, &head);
        assert_eq!(found.len(), 2, "{found:?}");
        // Better is not a regression.
        assert!(regressions(&head, &base).is_empty());
    }

    #[test]
    fn runs_are_deterministic() {
        assert_eq!(run("toolkit-60"), run("toolkit-60"));
    }

    #[test]
    fn an_idle_desktop_stops_drawing() {
        let r = run("idle-after-transition");
        // The transition is 300 ms of 4.75 s: about 18 flips a second on
        // average, and nothing once it is over.
        assert!(r.passes_per_s < 20.0, "{r:?}");
        assert_eq!(r.missed_vblanks, 0);
    }

    #[test]
    fn a_toolkit_window_with_time_to_spare_is_shown_every_vblank() {
        let r = run("toolkit-60");
        let c = &r.clients[0];
        assert!(c.fps > 59.0, "{r:?}");
        assert_eq!(r.missed_vblanks, 0, "{r:?}");
        assert_eq!(c.cadence_breaks, 0, "{r:?}");
    }

    #[test]
    fn a_fast_client_is_composed_on_the_next_vblank() {
        let r = run("toolkit-fast-144");
        let c = &r.clients[0];
        assert!(c.fps > 142.0, "{r:?}");
        // Committed and composed within the period it was asked in.
        assert!(c.latency_ms.p99 < 1000.0 / 144.0, "{r:?}");
    }

    #[test]
    fn video_costs_one_pass_per_content_frame() {
        let r = run("video-30-on-120");
        assert!((r.passes_per_s - 30.0).abs() < 1.0, "{r:?}");
        assert!((r.clients[0].fps - 30.0).abs() < 0.5, "{r:?}");
        assert_eq!(r.clients[0].cadence_breaks, 0, "{r:?}");
    }

    #[test]
    fn a_fast_mouse_costs_one_pass_per_refresh() {
        let r = run("pointer-1000hz-120");
        assert!(r.passes_per_s <= 121.0, "{r:?}");
        assert!(r.flips_per_s > 118.0, "{r:?}");
    }

    #[test]
    fn an_animation_holds_the_refresh_rate() {
        let r = run("animation-165");
        // 3 s of the 4.75 measured.
        let expected = 165.0 * 3.0 / r.seconds;
        assert!(r.flips_per_s > expected * 0.98, "{r:?}");
        assert_eq!(r.missed_vblanks, 0, "{r:?}");
    }

    #[test]
    fn a_compose_over_half_the_period_settles_at_half_rate() {
        let r = run("heavy-animation-165");
        let two_periods = 2000.0 / 165.0;
        assert!((r.flip_interval_ms.p50 - two_periods).abs() < 0.05, "{r:?}");
        // Only the animation's first frame: nothing has drawn before it, so
        // there is no GPU estimate yet and the pass runs at once.
        assert!(r.missed_vblanks <= 1, "{r:?}");
    }

    #[test]
    fn a_panels_own_edid_drives_the_display() {
        let display = edid::parse(&edid::synthesize("eDP", 2880, 1800, 90.0)).unwrap();
        let reports: Vec<Report> = standard_on(&display).iter().map(Scenario::run).collect();
        assert!(reports.iter().all(|r| r.display == display.label()));
        let toolkit = reports.iter().find(|r| r.scenario == "toolkit-60").unwrap();
        assert!(toolkit.clients[0].fps > 88.0, "{toolkit:?}");
    }
}
