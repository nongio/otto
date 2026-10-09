//! Wires the edge swipe into the udev backend's libinput event stream.
//!
//! [`Otto::filter_edge_swipe`] sees every libinput event before the normal
//! dispatch. Events from touchpads refresh the touch origins; two-finger
//! scrolls go through the [`EdgeSwipe`] state machine and are delivered,
//! held, replayed or swallowed according to its verdict.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};

use smithay::{
    backend::{
        input::{Axis, Event, InputEvent, PointerAxisEvent as _},
        libinput::{LibinputInputBackend, PointerScrollAxis},
    },
    reexports::input::{Device as LibinputDevice, DeviceCapability},
};

use super::{
    evdev::EvdevFds, Context, EdgeSwipe, Scroll, Touch, TouchOrigins, Verdict, MAX_COMPENSATION_MM,
};
use crate::{renderer::active::RendererApi, udev::UdevData, Otto};

/// libinput scroll units per millimetre of finger travel.
///
/// libinput normalises touchpad deltas to a 1000 dpi device, so a unit is
/// about 1/39 mm. Scroll deltas may carry a mild speed factor on top, which
/// is why the origin correction built from this is capped.
const LIBINPUT_UNITS_PER_MM: f64 = 1000.0 / 25.4;

type LibinputEvent = InputEvent<LibinputInputBackend>;

/// What the dispatcher does with an event after the edge swipe saw it.
#[derive(Debug)]
pub enum Filtered {
    /// Dispatch it as usual.
    Deliver(LibinputEvent),
    /// The edge swipe used it; clients never see it.
    Consumed,
    /// Dispatch these, in order: held events released with the current one.
    Replay(Vec<LibinputEvent>),
}

/// Edge swipe state owned by the udev backend.
#[derive(Debug, Default)]
pub struct EdgeSwipeInput {
    fds: EvdevFds,
    machine: EdgeSwipe,
    origins: TouchOrigins,
    /// Node of the touchpad the origins belong to.
    pad: Option<PathBuf>,
    /// Scratch buffer for slot samples.
    touches: Vec<Touch>,
    /// Scroll events kept back while the machine decides.
    held: Vec<LibinputEvent>,
    /// Horizontal finger travel of the current scroll, in libinput units,
    /// positive to the right.
    travel: f64,
}

impl EdgeSwipeInput {
    /// Creates the state around the registry libinput's interface fills.
    pub fn new(fds: EvdevFds) -> Self {
        Self {
            fds,
            ..Self::default()
        }
    }

    /// Samples the pad at `path` and folds the touches into the origins.
    ///
    /// Returns whether the touches started at the right edge.
    fn observe(&mut self, path: &Path) -> bool {
        if self.pad.as_deref() != Some(path) {
            self.origins.clear();
            self.pad = Some(path.to_path_buf());
        }
        let Some(zone) = self.fds.sample(path, &mut self.touches) else {
            self.origins.clear();
            return false;
        };
        let travel_mm =
            (self.travel / LIBINPUT_UNITS_PER_MM).clamp(-MAX_COMPENSATION_MM, MAX_COMPENSATION_MM);
        self.origins
            .observe(&self.touches, zone.mm_to_units(travel_mm), &zone);
        zone.is_edge_start(self.origins.xs())
    }

    /// Drops the current scroll and returns the held events to deliver.
    ///
    /// The flag says whether the canvas was being driven.
    fn abandon(&mut self) -> (bool, Vec<LibinputEvent>) {
        let claimed = self.machine.reset();
        (claimed, std::mem::take(&mut self.held))
    }
}

/// The libinput device behind the events the edge swipe cares about.
fn source_device(event: &LibinputEvent) -> Option<LibinputDevice> {
    Some(match event {
        InputEvent::PointerMotion { event } => event.device(),
        InputEvent::PointerButton { event } => event.device(),
        InputEvent::PointerAxis { event } => event.device(),
        InputEvent::GestureSwipeBegin { event } => event.device(),
        InputEvent::GestureSwipeUpdate { event } => event.device(),
        InputEvent::GestureSwipeEnd { event } => event.device(),
        InputEvent::GesturePinchBegin { event } => event.device(),
        InputEvent::GesturePinchUpdate { event } => event.device(),
        InputEvent::GesturePinchEnd { event } => event.device(),
        InputEvent::GestureHoldBegin { event } => event.device(),
        InputEvent::GestureHoldEnd { event } => event.device(),
        InputEvent::DeviceRemoved { device } => device.clone(),
        _ => return None,
    })
}

/// Device node of a libinput device.
fn node_path(device: &LibinputDevice) -> PathBuf {
    Path::new("/dev/input").join(device.sysname())
}

impl<A: RendererApi> Otto<UdevData<A>> {
    /// Runs `event` through the edge swipe before it is dispatched.
    pub fn filter_edge_swipe(&mut self, event: LibinputEvent) -> Filtered {
        let Some(device) = source_device(&event) else {
            return Filtered::Deliver(event);
        };
        // Only touchpads report gestures.
        if !device.has_capability(DeviceCapability::Gesture) {
            return Filtered::Deliver(event);
        }
        let path = node_path(&device);

        let axis = match &event {
            InputEvent::PointerAxis {
                event: axis @ PointerScrollAxis::Finger(_),
            } => axis,
            InputEvent::DeviceRemoved { .. } => {
                let state = &mut self.backend_data.edge_swipe;
                if state.pad.as_deref() == Some(path.as_path()) {
                    state.origins.clear();
                    state.pad = None;
                }
                return self.release_with(event);
            }
            InputEvent::GestureSwipeBegin { .. } | InputEvent::GesturePinchBegin { .. } => {
                self.backend_data.edge_swipe.observe(&path);
                return self.release_with(event);
            }
            _ => {
                let state = &mut self.backend_data.edge_swipe;
                state.observe(&path);
                if state.machine.is_deciding() {
                    return self.release_with(event);
                }
                return Filtered::Deliver(event);
            }
        };

        let horizontal = axis.amount(Axis::Horizontal);
        let vertical = axis.amount(Axis::Vertical);
        let stop = (horizontal.is_some() || vertical.is_some())
            && horizontal.unwrap_or(0.0) == 0.0
            && vertical.unwrap_or(0.0) == 0.0;
        // libinput inverts the values under natural scrolling; undo that to
        // get the direction the fingers moved.
        let sign = if device.config_scroll_natural_scroll_enabled() {
            -1.0
        } else {
            1.0
        };
        let (dx, dy) = (
            horizontal.unwrap_or(0.0) * sign,
            vertical.unwrap_or(0.0) * sign,
        );
        #[expect(clippy::cast_precision_loss, reason = "microsecond timestamps")]
        let time_s = axis.time().micros() as f64 / 1_000_000.0;

        let idle = self.backend_data.edge_swipe.machine.is_idle();
        let state = &mut self.backend_data.edge_swipe;
        if idle {
            state.travel = 0.0;
        }
        state.travel += dx;
        let from_edge = state.observe(&path);
        // The machine reads the context only when a scroll begins. A shown
        // canvas treats a rightward scroll that did not start at the edge as
        // a dismiss, over the canvas too: its items scroll vertically, and a
        // scroll that turns out not to be horizontal is replayed to them.
        let ctx = if idle && !self.canvas_suspended() {
            Context {
                available: self.canvas_available(),
                shown: self.canvas_is_shown(),
                from_edge,
            }
        } else {
            Context::default()
        };

        // Scroll deltas are device pixels, like pointer motion; dividing by
        // the output scale gives logical points that track the fingers 1:1.
        let scale = self.pointer_output_scale();
        let scroll = Scroll {
            dx: dx / scale,
            dy: dy / scale,
            time_s,
            stop,
        };
        let state = &mut self.backend_data.edge_swipe;
        match state.machine.on_scroll(scroll, ctx) {
            Verdict::Pass => Filtered::Deliver(event),
            Verdict::Hold => {
                state.held.push(event);
                Filtered::Consumed
            }
            Verdict::Release => {
                let mut events = std::mem::take(&mut state.held);
                events.push(event);
                Filtered::Replay(events)
            }
            Verdict::Begin { delta } => {
                state.held.clear();
                tracing::debug!(delta, "edge swipe claimed");
                self.canvas_gesture_begin();
                self.canvas_gesture_update(delta);
                Filtered::Consumed
            }
            Verdict::Update { delta } => {
                self.canvas_gesture_update(delta);
                Filtered::Consumed
            }
            Verdict::End { velocity } => {
                tracing::debug!(velocity, "edge swipe released");
                self.canvas_gesture_end(velocity, false);
                Filtered::Consumed
            }
        }
    }

    /// Abandons the current scroll ahead of `event`.
    ///
    /// Held events go out before `event`, and a canvas being driven is
    /// settled as a cancelled gesture.
    fn release_with(&mut self, event: LibinputEvent) -> Filtered {
        let (claimed, mut events) = self.backend_data.edge_swipe.abandon();
        if claimed {
            self.canvas_gesture_end(0.0, true);
        }
        if events.is_empty() {
            return Filtered::Deliver(event);
        }
        events.push(event);
        Filtered::Replay(events)
    }
}
