//! The output scale, read at startup before the app makes a surface.
//!
//! A surface's own `preferred_scale` only arrives a few frames after it maps,
//! and an app that sizes a card or rasterises an atlas in `on_app_ready` would
//! otherwise do it at a guess. The output is bound on a queue of its own, as
//! [`crate::backdrop`] does, so the roundtrip that reads it dispatches nothing
//! of the application's early.

use wayland_client::{
    globals::GlobalList,
    protocol::wl_output::{self, WlOutput},
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};

/// What the output said about itself during the roundtrip.
#[derive(Default)]
struct Probe {
    scale: Option<i32>,
    transform: Option<wl_output::Transform>,
    mode: Option<(i32, i32)>,
    logical_size: Option<(i32, i32)>,
}

impl Dispatch<WlOutput, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Geometry {
                transform: WEnum::Value(transform),
                ..
            } => state.transform = Some(transform),
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => state.mode = Some((width, height)),
            wl_output::Event::Scale { factor } => state.scale = Some(factor),
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_output_v1::Event::LogicalSize { width, height } = event {
            state.logical_size = Some((width, height));
        }
    }
}

wayland_client::delegate_noop!(Probe: ignore ZxdgOutputManagerV1);

impl Probe {
    /// The scale in 120ths. `wl_output` only carries an integer — 2 on a 1.5x
    /// output — so the fraction comes from the current mode against the
    /// `xdg_output` logical size, which is the mode divided by the fractional
    /// scale. Without a logical size, the integer one.
    fn scale_120(&self) -> Option<u32> {
        let fractional = self
            .mode
            .zip(self.logical_size)
            .and_then(|((w, h), (logical_w, _))| {
                // The logical size is transformed; the mode is not.
                let physical_w = match self.transform {
                    Some(
                        wl_output::Transform::_90
                        | wl_output::Transform::_270
                        | wl_output::Transform::Flipped90
                        | wl_output::Transform::Flipped270,
                    ) => h,
                    _ => w,
                };
                (logical_w > 0 && physical_w > 0)
                    .then(|| (f64::from(physical_w) / f64::from(logical_w) * 120.0).round() as u32)
            });
        fractional.or_else(|| self.scale.map(|factor| factor.max(1) as u32 * 120))
    }
}

/// The scale of the first output the compositor advertised, in 120ths.
///
/// Which output a surface will map on is unknown until it does; the first one
/// advertised is the compositor's primary, the best guess there is. Every
/// object made here is gone again before this returns.
pub(crate) fn probe(conn: &Connection, globals: &GlobalList) -> Option<u32> {
    let mut queue = conn.new_event_queue::<Probe>();
    let qh = queue.handle();
    let output = globals.bind::<WlOutput, _, _>(&qh, 1..=4, ()).ok()?;
    let manager = globals
        .bind::<ZxdgOutputManagerV1, _, _>(&qh, 1..=3, ())
        .ok();
    let xdg_output = manager
        .as_ref()
        .map(|manager| manager.get_xdg_output(&output, &qh, ()));

    let mut probe = Probe::default();
    let read = queue.roundtrip(&mut probe).is_ok();

    if let Some(xdg_output) = xdg_output {
        xdg_output.destroy();
    }
    if let Some(manager) = manager {
        manager.destroy();
    }
    if output.version() >= 3 {
        output.release();
    }
    read.then(|| probe.scale_120()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(scale: i32, mode: (i32, i32), logical_size: Option<(i32, i32)>) -> Probe {
        Probe {
            scale: Some(scale),
            transform: Some(wl_output::Transform::Normal),
            mode: Some(mode),
            logical_size,
        }
    }

    /// The logical size is rounded to whole points; the scale must still come
    /// out as the 120ths the compositor's `preferred_scale` will send.
    #[test]
    fn fractional_scale_from_logical_size() {
        assert_eq!(
            probe(2, (2560, 1440), Some((1707, 960))).scale_120(),
            Some(180)
        );
        assert_eq!(
            probe(1, (1920, 1080), Some((1920, 1080))).scale_120(),
            Some(120)
        );
        assert_eq!(
            probe(2, (3840, 2160), Some((1920, 1080))).scale_120(),
            Some(240)
        );
    }

    #[test]
    fn rotated_output_compares_like_with_like() {
        let mut rotated = probe(2, (2560, 1440), Some((960, 1707)));
        rotated.transform = Some(wl_output::Transform::_90);
        assert_eq!(rotated.scale_120(), Some(180));
    }

    #[test]
    fn integer_scale_without_xdg_output() {
        assert_eq!(probe(2, (2560, 1440), None).scale_120(), Some(240));
    }
}
