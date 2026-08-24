//! How the deck moves. A platter does not snap to speed and a tonearm does not
//! drop instantly, so play and pause are ramps, not switches.

use std::time::Instant;

/// 33⅓ revolutions per minute, in radians per second.
pub const TURN: f32 = 33.333 / 60.0 * std::f32::consts::TAU;

/// Time constants: spinning up is quicker than coasting down.
const SPIN_UP: f32 = 0.75;
const SPIN_DOWN: f32 = 1.9;
/// The arm lifts fast and lowers gently.
const LIFT_UP: f32 = 0.16;
const LIFT_DOWN: f32 = 0.34;

pub struct Motion {
    pub angle: f32,
    pub velocity: f32,
    pub playing: bool,
    /// 0 = stylus in the groove, 1 = arm raised clear of the record.
    pub lift: f32,
    pub hovering_play: bool,
    started: Instant,
    last: Instant,
}

impl Motion {
    pub fn new(playing: bool) -> Self {
        let now = Instant::now();
        Self {
            angle: 0.0,
            velocity: if playing { TURN } else { 0.0 },
            playing,
            lift: if playing { 0.0 } else { 1.0 },
            hovering_play: false,
            started: now,
            last: now,
        }
    }

    pub fn toggle(&mut self) {
        self.playing = !self.playing;
    }

    /// Advance to now. Called once per event-loop tick.
    pub fn step(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;

        let (target, tau) = if self.playing {
            (TURN, SPIN_UP)
        } else {
            (0.0, SPIN_DOWN)
        };
        self.velocity += (target - self.velocity) * (1.0 - (-dt / tau).exp());

        // Wow and flutter: a fraction of a percent, but it is the difference
        // between a record and a CSS rotation.
        let t = self.started.elapsed().as_secs_f32();
        let flutter = 1.0 + 0.0018 * (t * 3.9).sin() + 0.0009 * (t * 11.3).sin();
        self.angle = (self.angle + self.velocity * flutter * dt) % std::f32::consts::TAU;

        let (lift_target, lift_tau) = if self.playing {
            (0.0, LIFT_DOWN)
        } else {
            (1.0, LIFT_UP)
        };
        self.lift += (lift_target - self.lift) * (1.0 - (-dt / lift_tau).exp());
    }

    /// True while the platter is still turning, so the window keeps repainting
    /// through the spin-down instead of freezing mid-coast.
    pub fn is_animating(&self) -> bool {
        self.playing || self.velocity.abs() > 0.001 || self.lift < 0.999
    }
}
