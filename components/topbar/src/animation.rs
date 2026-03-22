/// Smooth spring animation for a floating-point value with configurable bounce.
pub struct AnimatedValue {
    current: f32,
    target: f32,
    velocity: f32,
    /// Snap to target on the first update instead of animating.
    first: bool,
}

/// Spring parameters: stiffness controls speed, damping controls bounce.
/// Lower damping ratio = more bounce (underdamped < 1.0).
const STIFFNESS: f32 = 280.0;
const DAMPING: f32 = 18.0;

impl AnimatedValue {
    pub fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            velocity: 0.0,
            first: true,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Advance the animation by `dt` seconds.
    /// Returns `true` if the value is still moving.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.first {
            self.current = self.target;
            self.first = false;
            return false;
        }

        // Spring physics: F = -k * displacement - c * velocity
        let displacement = self.current - self.target;
        let spring_force = -STIFFNESS * displacement;
        let damping_force = -DAMPING * self.velocity;
        let acceleration = spring_force + damping_force;

        self.velocity += acceleration * dt;
        self.current += self.velocity * dt;

        if displacement.abs() < 0.3 && self.velocity.abs() < 0.5 {
            self.current = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }

    pub fn value(&self) -> f32 {
        self.current
    }
}
