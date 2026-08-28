//! Server-side implementation of the wlr-virtual-pointer-unstable-v1 protocol.
//!
//! Lets clients (ydotool, wtype, wlrctl, and any bespoke automation driver —
//! e.g. an MCP wrapper) synthesize pointer motion / button / axis events that
//! appear to the rest of the compositor as if they came from a real
//! libinput-backed pointer. The events feed directly into the `PointerHandle`
//! of the default seat, so downstream focus/hit-testing/cursor rendering all
//! flow through the same path used by real input.
//!
//! Smithay does not ship a delegate for this protocol, so the
//! `GlobalDispatch`/`Dispatch` plumbing is hand-rolled here. Scope is the
//! minimum useful subset:
//!
//! - `motion` — relative displacement, applied to the current pointer
//!   location.
//! - `motion_absolute` — normalized absolute position mapped to the first
//!   available output.
//! - `button` — wl_pointer button press/release.
//! - `axis` — wl_pointer axis scroll.
//! - `frame` — flush a coalesced event sequence.
//! - `axis_source`, `axis_stop`, `axis_discrete` — accumulated on the pending
//!   `AxisFrame` and committed on `frame`.
//! - `destroy` — drop the pointer; freeing state.
//!
//! Pointer constraints, locked pointers, and relative-motion reporting are
//! not honored from synthesized events — those are real-pointer concerns.

use std::sync::Mutex;

use smithay::{
    backend::input::{Axis, ButtonState},
    input::{
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
        SeatHandler,
    },
    reexports::{
        wayland_protocols_wlr::virtual_pointer::v1::server::{
            zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
            zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
        },
        wayland_server::{
            backend::{ClientId, GlobalId},
            protocol::wl_pointer,
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
        },
    },
    utils::{Point, SERIAL_COUNTER},
};

use crate::state::Otto;

/// Module-level state for the virtual pointer manager global.
#[derive(Debug)]
pub struct VirtualPointerManagerState {
    #[allow(dead_code)]
    global: GlobalId,
}

impl VirtualPointerManagerState {
    pub fn new<BackendData>(display: &DisplayHandle) -> Self
    where
        BackendData: crate::state::Backend + 'static,
        Otto<BackendData>: GlobalDispatch<ZwlrVirtualPointerManagerV1, ()>,
        Otto<BackendData>: Dispatch<ZwlrVirtualPointerManagerV1, ()>,
        Otto<BackendData>: Dispatch<ZwlrVirtualPointerV1, VirtualPointerUserData>,
    {
        let global = display.create_global::<Otto<BackendData>, ZwlrVirtualPointerManagerV1, ()>(
            2, // protocol version (create_virtual_pointer_with_output added in v2)
            (),
        );
        Self { global }
    }
}

/// Per-pointer state. An AxisFrame accumulates axis / axis_source / axis_stop /
/// axis_discrete events until the client commits with `frame`.
#[derive(Debug, Default)]
pub struct VirtualPointerUserData {
    pending: Mutex<PendingFrame>,
    /// Output bound at creation (`create_virtual_pointer_with_output`).
    /// Absolute motion maps into this output's geometry; without it the
    /// first output is used. Lets a driver target a specific output (e.g.
    /// an interactive virtual output) instead of whichever enumerates first.
    output: Option<smithay::output::Output>,
}

#[derive(Debug, Default)]
struct PendingFrame {
    /// Accumulated relative displacement (logical pixels) since the last frame.
    motion_rel: Option<(f64, f64)>,
    /// Absolute position (logical pixels) replacing any pending relative motion.
    motion_abs: Option<(f64, f64)>,
    /// Button events to flush.
    buttons: Vec<(u32, u32, ButtonState)>,
    /// Axis accumulator built via `AxisFrame::new`.
    axis: Option<AxisFrame>,
}

impl<BackendData> GlobalDispatch<ZwlrVirtualPointerManagerV1, (), Otto<BackendData>>
    for VirtualPointerManagerState
where
    BackendData: crate::state::Backend + 'static,
    Otto<BackendData>: Dispatch<ZwlrVirtualPointerManagerV1, ()>,
    Otto<BackendData>: Dispatch<ZwlrVirtualPointerV1, VirtualPointerUserData>,
{
    fn bind(
        _state: &mut Otto<BackendData>,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrVirtualPointerManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        data_init.init(resource, ());
    }
}

impl<BackendData> Dispatch<ZwlrVirtualPointerManagerV1, (), Otto<BackendData>>
    for VirtualPointerManagerState
where
    BackendData: crate::state::Backend + 'static,
    Otto<BackendData>: Dispatch<ZwlrVirtualPointerV1, VirtualPointerUserData>,
{
    fn request(
        _state: &mut Otto<BackendData>,
        _client: &Client,
        _resource: &ZwlrVirtualPointerManagerV1,
        request: zwlr_virtual_pointer_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        match request {
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointer { seat: _, id } => {
                data_init.init(id, VirtualPointerUserData::default());
            }
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointerWithOutput {
                seat: _,
                output,
                id,
            } => {
                let output = output
                    .as_ref()
                    .and_then(smithay::output::Output::from_resource);
                data_init.init(
                    id,
                    VirtualPointerUserData {
                        output,
                        ..Default::default()
                    },
                );
            }
            zwlr_virtual_pointer_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl<BackendData> Dispatch<ZwlrVirtualPointerV1, VirtualPointerUserData, Otto<BackendData>>
    for VirtualPointerManagerState
where
    BackendData: crate::state::Backend + 'static,
    Otto<BackendData>: SeatHandler<PointerFocus = crate::focus::PointerFocusTarget<BackendData>>,
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        _resource: &ZwlrVirtualPointerV1,
        request: zwlr_virtual_pointer_v1::Request,
        data: &VirtualPointerUserData,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        let mut pending = data.pending.lock().unwrap();
        match request {
            zwlr_virtual_pointer_v1::Request::Motion { time: _, dx, dy } => {
                // dx / dy are fixed-point values; into `f64` preserves precision.
                let (ax, ay) = pending.motion_rel.unwrap_or((0.0, 0.0));
                pending.motion_rel = Some((ax + dx, ay + dy));
            }
            zwlr_virtual_pointer_v1::Request::MotionAbsolute {
                time: _,
                x,
                y,
                x_extent,
                y_extent,
            } => {
                // Map the normalized absolute position into logical-pixel
                // coordinates using the bound output's geometry (see
                // `VirtualPointerUserData::output`), falling back to the
                // first output. If we have no outputs (shouldn't happen in
                // practice), drop the event.
                if x_extent == 0 || y_extent == 0 {
                    return;
                }
                let Some(output) = data
                    .output
                    .clone()
                    .or_else(|| state.workspaces.outputs().next().cloned())
                else {
                    return;
                };
                let Some(geo) = state.workspaces.output_geometry(&output) else {
                    return;
                };
                let nx = x as f64 / x_extent as f64;
                let ny = y as f64 / y_extent as f64;
                let abs_x = geo.loc.x as f64 + nx * geo.size.w as f64;
                let abs_y = geo.loc.y as f64 + ny * geo.size.h as f64;
                pending.motion_rel = None;
                pending.motion_abs = Some((abs_x, abs_y));
            }
            zwlr_virtual_pointer_v1::Request::Button {
                time,
                button,
                state: btn_state,
            } => {
                let btn_state = match btn_state.into_result() {
                    Ok(wl_pointer::ButtonState::Pressed) => ButtonState::Pressed,
                    Ok(wl_pointer::ButtonState::Released) => ButtonState::Released,
                    _ => return,
                };
                pending.buttons.push((time, button, btn_state));
            }
            zwlr_virtual_pointer_v1::Request::Axis {
                time: _,
                axis,
                value,
            } => {
                let Ok(axis_kind) = axis.into_result() else {
                    return;
                };
                let smithay_axis = match axis_kind {
                    wl_pointer::Axis::VerticalScroll => Axis::Vertical,
                    wl_pointer::Axis::HorizontalScroll => Axis::Horizontal,
                    _ => return,
                };
                let frame = pending.axis.take().unwrap_or_else(|| {
                    AxisFrame::new(0).source(smithay::backend::input::AxisSource::Wheel)
                });
                pending.axis = Some(frame.value(smithay_axis, value));
            }
            zwlr_virtual_pointer_v1::Request::AxisSource { axis_source } => {
                let Ok(src) = axis_source.into_result() else {
                    return;
                };
                let smithay_src = match src {
                    wl_pointer::AxisSource::Wheel => smithay::backend::input::AxisSource::Wheel,
                    wl_pointer::AxisSource::Finger => smithay::backend::input::AxisSource::Finger,
                    wl_pointer::AxisSource::Continuous => {
                        smithay::backend::input::AxisSource::Continuous
                    }
                    wl_pointer::AxisSource::WheelTilt => {
                        smithay::backend::input::AxisSource::WheelTilt
                    }
                    _ => smithay::backend::input::AxisSource::Wheel,
                };
                let frame = pending.axis.take().unwrap_or_else(|| AxisFrame::new(0));
                pending.axis = Some(frame.source(smithay_src));
            }
            zwlr_virtual_pointer_v1::Request::AxisStop { time: _, axis } => {
                let Ok(axis_kind) = axis.into_result() else {
                    return;
                };
                let smithay_axis = match axis_kind {
                    wl_pointer::Axis::VerticalScroll => Axis::Vertical,
                    wl_pointer::Axis::HorizontalScroll => Axis::Horizontal,
                    _ => return,
                };
                let frame = pending.axis.take().unwrap_or_else(|| AxisFrame::new(0));
                pending.axis = Some(frame.stop(smithay_axis));
            }
            zwlr_virtual_pointer_v1::Request::AxisDiscrete {
                time: _,
                axis,
                value,
                discrete,
            } => {
                let Ok(axis_kind) = axis.into_result() else {
                    return;
                };
                let smithay_axis = match axis_kind {
                    wl_pointer::Axis::VerticalScroll => Axis::Vertical,
                    wl_pointer::Axis::HorizontalScroll => Axis::Horizontal,
                    _ => return,
                };
                let frame = pending.axis.take().unwrap_or_else(|| AxisFrame::new(0));
                pending.axis = Some(
                    frame
                        .value(smithay_axis, value)
                        .v120(smithay_axis, discrete * 120),
                );
            }
            zwlr_virtual_pointer_v1::Request::Frame => {
                // Flush the accumulated events in order:
                //   motion → buttons → axis
                // motion_abs takes precedence over motion_rel if both set.
                let motion_rel = pending.motion_rel.take();
                let motion_abs = pending.motion_abs.take();
                let buttons = std::mem::take(&mut pending.buttons);
                let axis = pending.axis.take();
                drop(pending);

                let pointer = state.pointer.clone();

                // Compute the new absolute location, clamped to screen
                // bounds like real input — unclamped synthesized motion
                // accumulates unbounded off-screen positions, which breaks
                // focused-output tracking and makes harness moves land on
                // the wrong output.
                let mut new_location = pointer.current_location();
                if let Some((ax, ay)) = motion_abs {
                    new_location = Point::from((ax, ay));
                } else if let Some((dx, dy)) = motion_rel {
                    new_location += Point::from((dx, dy));
                }
                let new_location = state.clamp_coords(new_location);

                if motion_rel.is_some() || motion_abs.is_some() {
                    let under = state.surface_under(new_location);
                    let serial = SERIAL_COUNTER.next_serial();
                    pointer.motion(
                        state,
                        under,
                        &MotionEvent {
                            location: new_location,
                            serial,
                            time: 0,
                        },
                    );
                    // Mirror the real-input path: track which output the
                    // pointer is on so focused-output consumers (expose,
                    // selector, window routing) see harness-driven moves.
                    let focused = state.workspaces.output_under(new_location).next().cloned();
                    state.workspaces.set_focused_output(focused.as_ref());

                    // Also mirror the lay-rs side of the real-input path:
                    // forward the position to the engine in physical pixels.
                    // Dock label hover, tooltips, and clicks on lay-rs UI all
                    // hit-test against the engine's pointer position — without
                    // this they keep resolving against wherever the physical
                    // mouse last was.
                    state.last_pointer_location = (new_location.x, new_location.y);
                    state.a11y.pointer.set(new_location.x, new_location.y);
                    let hit_output = focused.as_ref().or(data.output.as_ref());
                    let scale = hit_output
                        .map(|o| o.current_scale().fractional_scale())
                        .unwrap_or(1.0);
                    // lay-rs scene coordinates are OUTPUT-LOCAL (output subtrees
                    // overlap at (0,0)), so rebase the global position to the
                    // output's origin before hit-testing — exactly as the
                    // real-input path does. Without it, synthesized hover on any
                    // output placed past x=0 (an RDP virtual output, say) probes
                    // the scene far outside the subtree and never lands on the
                    // UI under the cursor.
                    let origin = hit_output.map(|o| o.current_location()).unwrap_or_default();
                    let phys = (new_location - origin.to_f64()).to_physical(scale);
                    state.cursor_physical_position = (phys.x, phys.y);
                    state
                        .layers_engine
                        .pointer_move(&(phys.x as f32, phys.y as f32).into(), None);
                    state.check_dock_hot_zone((new_location.x, new_location.y));
                }

                for (time, button, btn_state) in buttons {
                    let serial = SERIAL_COUNTER.next_serial();
                    // Mirror the click-to-focus / raise behavior that
                    // `on_pointer_button` does for real libinput events, so
                    // virtual-pointer clicks also focus the window under the
                    // cursor. Without this, clicks would dispatch to whatever
                    // surface already held pointer focus instead of the one
                    // the test harness intended to click on.
                    if btn_state == ButtonState::Pressed && !state.workspaces.get_show_all() {
                        // Re-resolve the lay-rs hover against live layer
                        // positions before dispatching, like the real-input
                        // button path — a layer that animated under the
                        // stationary cursor would otherwise eat the click.
                        let (cx, cy) = state.cursor_physical_position;
                        state
                            .layers_engine
                            .pointer_move(&(cx as f32, cy as f32).into(), None);
                        state.focus_window_under_cursor(serial);
                    }
                    pointer.button(
                        state,
                        &ButtonEvent {
                            button,
                            state: btn_state,
                            serial,
                            time,
                        },
                    );
                    match btn_state {
                        ButtonState::Pressed => state.layers_engine.pointer_button_down(),
                        ButtonState::Released => state.layers_engine.pointer_button_up(),
                    }
                }

                if let Some(axis_frame) = axis {
                    pointer.axis(state, axis_frame);
                }

                pointer.frame(state);

                // Request a redraw so remote (RDP / wlr-virtual-pointer) input
                // actually becomes visible. Setting `render_requested` makes the
                // event loop run a render cycle, which *ticks the scheduled
                // lay-rs transactions* (hover highlights, workspace scroll,
                // click feedback) — without it those transactions never advance
                // and the change stays invisible until unrelated input (e.g. the
                // physical trackpad) drives a render. This is the same thing the
                // synthetic-action path does (see `init.rs`, request_redraw after
                // a debug action). It is one-shot per input event, so it stays
                // event/damage-driven rather than continuously rendering.
                state.backend_data.request_redraw();
            }
            zwlr_virtual_pointer_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        _state: &mut Otto<BackendData>,
        _client: ClientId,
        _resource: &ZwlrVirtualPointerV1,
        _data: &VirtualPointerUserData,
    ) {
    }
}

/// Macro to register the virtual pointer dispatch delegates for a concrete
/// `Otto<BackendData>` instantiation. Mirrors smithay's
/// `delegate_virtual_keyboard_manager!` shape so callers can invoke it with
/// `delegate_virtual_pointer_manager!(@<BackendData: Backend + 'static> Otto<BackendData>);`.
#[macro_export]
macro_rules! delegate_virtual_pointer_manager {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty:ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!(
            $(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)?
            $ty: [
                smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1: ()
            ] => $crate::state::virtual_pointer::VirtualPointerManagerState
        );
        smithay::reexports::wayland_server::delegate_dispatch!(
            $(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)?
            $ty: [
                smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1: ()
            ] => $crate::state::virtual_pointer::VirtualPointerManagerState
        );
        smithay::reexports::wayland_server::delegate_dispatch!(
            $(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)?
            $ty: [
                smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1: $crate::state::virtual_pointer::VirtualPointerUserData
            ] => $crate::state::virtual_pointer::VirtualPointerManagerState
        );
    };
}
