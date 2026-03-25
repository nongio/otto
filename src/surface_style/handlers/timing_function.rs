use std::sync::Mutex;

use wayland_backend::server::ClientId;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle};

use super::super::protocol::gen::otto_timing_function_v1::{self, OttoTimingFunctionV1};
use crate::{state::Backend, Otto};
use layers::prelude::{Easing, Spring, TimingFunction};

/// Interior-mutable state for timing function parameters.
/// Uses `Mutex` to satisfy wayland-server's `Send + Sync` requirement.
/// Never contended — Wayland dispatch is single-threaded.
pub struct ScTimingFunctionData {
    inner: Mutex<ScTimingFunctionInner>,
}

pub struct ScTimingFunctionInner {
    pub timing: TimingFunction,
    pub spring_uses_duration: bool,
    pub spring_bounce: Option<f32>,
    pub spring_initial_velocity: f32,
}

impl Default for ScTimingFunctionData {
    fn default() -> Self {
        Self::new()
    }
}

impl ScTimingFunctionData {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ScTimingFunctionInner {
                timing: TimingFunction::linear(0.0),
                spring_uses_duration: false,
                spring_bounce: None,
                spring_initial_velocity: 0.0,
            }),
        }
    }

    /// Read a snapshot of the current state.
    pub fn read(&self) -> ScTimingFunctionInner {
        let guard = self.inner.lock().unwrap();
        ScTimingFunctionInner {
            timing: guard.timing,
            spring_uses_duration: guard.spring_uses_duration,
            spring_bounce: guard.spring_bounce,
            spring_initial_velocity: guard.spring_initial_velocity,
        }
    }
}

impl<BackendData: Backend> Dispatch<OttoTimingFunctionV1, ScTimingFunctionData>
    for Otto<BackendData>
{
    fn request(
        _state: &mut Self,
        _client: &Client,
        _timing_function: &OttoTimingFunctionV1,
        request: otto_timing_function_v1::Request,
        data: &ScTimingFunctionData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            otto_timing_function_v1::Request::SetPreset { preset } => {
                let timing = match preset.into_result() {
                    Ok(otto_timing_function_v1::Preset::Linear) => TimingFunction::linear(0.0),
                    Ok(otto_timing_function_v1::Preset::EaseIn) => TimingFunction::ease_in(0.0),
                    Ok(otto_timing_function_v1::Preset::EaseOut) => TimingFunction::ease_out(0.0),
                    Ok(otto_timing_function_v1::Preset::EaseInOut) => {
                        TimingFunction::ease_in_out(0.0)
                    }
                    Ok(otto_timing_function_v1::Preset::EaseInQuad) => {
                        TimingFunction::ease_in_quad(0.0)
                    }
                    Ok(otto_timing_function_v1::Preset::EaseOutQuad) => {
                        TimingFunction::ease_out_quad(0.0)
                    }
                    Ok(otto_timing_function_v1::Preset::EaseInOutQuad) => {
                        TimingFunction::ease_in_out_quad(0.0)
                    }
                    Ok(otto_timing_function_v1::Preset::EaseInCubic) => {
                        TimingFunction::ease_in_cubic(0.0)
                    }
                    Ok(otto_timing_function_v1::Preset::EaseOutCubic) => {
                        TimingFunction::ease_out_cubic(0.0)
                    }
                    Ok(otto_timing_function_v1::Preset::EaseInOutCubic) => {
                        TimingFunction::ease_in_out_cubic(0.0)
                    }
                    Err(_) => {
                        tracing::warn!("Unknown timing function preset: {:?}", preset);
                        TimingFunction::linear(0.0)
                    }
                };
                data.inner.lock().unwrap().timing = timing;
            }

            otto_timing_function_v1::Request::SetBezier { c1x, c1y, c2x, c2y } => {
                let easing = Easing {
                    x1: c1x as f32,
                    y1: c1y as f32,
                    x2: c2x as f32,
                    y2: c2y as f32,
                };
                data.inner.lock().unwrap().timing = TimingFunction::Easing(easing, 0.0);
            }

            otto_timing_function_v1::Request::SetSpring {
                bounce,
                initial_velocity,
            } => {
                tracing::debug!(
                    "Setting duration-based spring parameters: bounce={}, initial_velocity={}",
                    bounce,
                    initial_velocity
                );
                let mut inner = data.inner.lock().unwrap();
                inner.timing =
                    TimingFunction::Spring(Spring::with_duration_and_bounce(0.0, bounce as f32));
                inner.spring_bounce = Some(bounce as f32);
                inner.spring_initial_velocity = initial_velocity as f32;
                inner.spring_uses_duration = true;
            }

            otto_timing_function_v1::Request::SetSpringStiffnessDamping {
                stiffness,
                damping,
                initial_velocity,
            } => {
                let mut spring = Spring::new(1.0, stiffness as f32, damping as f32);
                spring.initial_velocity = initial_velocity as f32;
                tracing::debug!(
                    "Creating physics-based spring (ignores duration): stiffness={}, damping={}, initial_velocity={}",
                    stiffness,
                    damping,
                    initial_velocity
                );
                let mut inner = data.inner.lock().unwrap();
                inner.timing = TimingFunction::Spring(spring);
                inner.spring_uses_duration = false;
            }

            otto_timing_function_v1::Request::Destroy => {
                // Cleanup handled in destroyed()
            }
        }
    }

    fn destroyed(
        _state: &mut Self,
        _client: ClientId,
        _timing_function: &OttoTimingFunctionV1,
        _data: &ScTimingFunctionData,
    ) {
        // No cleanup needed - timing function data will be dropped
    }
}
