use std::cell::RefCell;

use smithay::{
    desktop::WindowSurface,
    input::{
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
            GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
            RelativeMotionEvent,
        },
        touch::{GrabStartData as TouchGrabStartData, TouchGrab},
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{IsAlive, Logical, Point, Serial, Size},
    wayland::{compositor::with_states, shell::xdg::SurfaceCachedState},
};
#[cfg(feature = "xwayland")]
use smithay::{utils::Rectangle, xwayland::xwm::ResizeEdge as X11ResizeEdge};

use super::{SurfaceData, WindowElement};
use crate::{
    focus::PointerFocusTarget,
    state::{Backend, Otto},
};

pub struct PointerResizeSurfaceGrab<B: Backend + 'static> {
    pub start_data: PointerGrabStartData<Otto<B>>,
    pub window: WindowElement,
    pub edges: ResizeEdge,
    pub initial_window_location: Point<i32, Logical>,
    pub initial_window_size: Size<i32, Logical>,
    pub last_window_size: Size<i32, Logical>,
}

pub struct PointerMoveSurfaceGrab<B: Backend + 'static> {
    pub start_data: PointerGrabStartData<Otto<B>>,
    pub window: WindowElement,
    pub initial_window_location: Point<i32, Logical>,
    /// The snap zone currently previewed under the cursor while Ctrl is held
    /// during the drag. `None` when no zone is active. Applied on button release.
    pub active_zone: Option<crate::workspaces::TileZone>,
    /// The window is maximized or tiled and gets restored under the cursor —
    /// but only once the drag has actually travelled. Restoring at button-down
    /// would make a *click* unmaximize the window, and would eat the first
    /// press of the double click that zooms it.
    pub pending_restore: bool,
    /// Where the drag is measured from. The press location to begin with, and
    /// the pointer's position at the moment a pending restore fires — after
    /// which the window has a new origin under the cursor.
    pub drag_origin: Point<f64, Logical>,
}

/// How far the pointer travels before a press counts as a drag, and a
/// maximized window is restored into it.
const DRAG_THRESHOLD: f64 = 6.0;

/// Whether Ctrl is held right now, read from the keyboard itself.
///
/// The cached `current_modifiers` can lag behind on paths that bypass the key
/// filter, and a latched Ctrl silently arms drag-to-tile on every window drag.
fn ctrl_held<B: Backend>(state: &Otto<B>) -> bool {
    state
        .seat
        .get_keyboard()
        .map(|keyboard| keyboard.modifier_state().ctrl)
        .unwrap_or(false)
}

impl<B: Backend> PointerGrab<Otto<B>> for PointerMoveSurfaceGrab<B> {
    fn motion(
        &mut self,
        state: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        _focus: Option<(PointerFocusTarget<B>, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(state, None, event);

        // A dragged window is the one the user is watching: full-rate frame
        // callbacks for it, and full-rate blur rebuilds while it moves under
        // a frosted surface (the renderer reads this stamp's recency).
        state.pointer_interaction = Some((self.window.id(), std::time::Instant::now()));

        // The layer lives under the OWNING output's subtree — use that
        // output for both the scale and the global→output-local rebase.
        // (outputs_for_element can be empty mid-drag near output edges.)
        let Some(output) = state
            .workspaces
            .output_for_window(&self.window)
            .or_else(|| {
                state
                    .workspaces
                    .outputs_for_element(&self.window)
                    .first()
                    .cloned()
            })
            .or_else(|| state.workspaces.primary_output().cloned())
        else {
            return;
        };
        let scale = output.current_scale().fractional_scale();

        // A maximized or tiled window is restored into the drag, not at the
        // press: until the pointer has travelled, it stays where it is.
        if self.pending_restore {
            let travel = event.location - self.start_data.location;
            if travel.x.abs() < DRAG_THRESHOLD && travel.y.abs() < DRAG_THRESHOLD {
                return;
            }
            self.pending_restore = false;
            if let Some(location) = state.restore_window_for_drag(&self.window, event.location) {
                self.initial_window_location = location;
            }
            self.drag_origin = event.location;
        }

        let delta = event.location - self.drag_origin;
        let new_location = self.initial_window_location.to_f64() + delta;

        state
            .workspaces
            .map_window(&self.window, new_location.to_i32_round(), true, None);

        if let Some(view) = state.workspaces.get_window_view(&self.window.id()) {
            let local = new_location - output.current_location().to_f64();
            let location = local.to_physical(scale);
            view.window_layer.set_position(
                crate::workspaces::utils::snap_position_px(location.x, location.y),
                None,
            );
        }

        // While Ctrl is held, preview the snap zone the pointer is over;
        // release applies it (see `button`).
        if ctrl_held(state) {
            if let Some(output) = state
                .workspaces
                .outputs_for_element(&self.window)
                .first()
                .cloned()
            {
                let usable = state.usable_zone(&output);
                let zone = crate::workspaces::zone_from_pointer(usable, event.location);
                self.active_zone = zone;
                match zone {
                    Some(zone) => {
                        let target = zone.target_rect(usable);
                        let out_scale = output.current_scale().fractional_scale() as f32;
                        let x_px = target.loc.x as f32 * out_scale;
                        let y_px = target.loc.y as f32 * out_scale;
                        let w_px = target.size.w as f32 * out_scale;
                        let h_px = target.size.h as f32 * out_scale;
                        state
                            .workspaces
                            .tiling_overlay
                            .show_zone(x_px, y_px, w_px, h_px, out_scale);
                    }
                    None => state.workspaces.tiling_overlay.hide(),
                }
            }
        } else if self.active_zone.is_some() || state.workspaces.tiling_overlay.is_visible() {
            self.active_zone = None;
            state.workspaces.tiling_overlay.hide();
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        focus: Option<(PointerFocusTarget<B>, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(self, data, event.serial, event.time, true);

            // Snap the window into the previewed zone, if any.
            data.workspaces.tiling_overlay.hide();
            // Re-check the modifier at release: `active_zone` is only
            // refreshed on motion, so letting go of Ctrl and then of the
            // button without moving would otherwise still tile the window.
            let zone = self.active_zone.take();
            if let Some(zone) = zone.filter(|_| ctrl_held(data)) {
                data.apply_tile(&self.window, zone);
            }
        }
    }

    fn axis(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Otto<B>, handle: &mut PointerInnerHandle<'_, Otto<B>>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &PointerGrabStartData<Otto<B>> {
        &self.start_data
    }
    fn unset(&mut self, _data: &mut Otto<B>) {}
}

pub struct TouchMoveSurfaceGrab<BackendData: Backend + 'static> {
    pub start_data: TouchGrabStartData<Otto<BackendData>>,
    pub window: WindowElement,
    pub initial_window_location: Point<i32, Logical>,
}

impl<BackendData: Backend> TouchGrab<Otto<BackendData>> for TouchMoveSurfaceGrab<BackendData> {
    fn down(
        &mut self,
        _data: &mut Otto<BackendData>,
        _handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        _focus: Option<(
            <Otto<BackendData> as smithay::input::SeatHandler>::TouchFocus,
            Point<f64, Logical>,
        )>,
        _event: &smithay::input::touch::DownEvent,
        _seq: Serial,
    ) {
    }

    fn up(
        &mut self,
        data: &mut Otto<BackendData>,
        handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        event: &smithay::input::touch::UpEvent,
        seq: Serial,
    ) {
        if event.slot != self.start_data.slot {
            return;
        }

        handle.up(data, event, seq);
        handle.unset_grab(self, data);
    }

    fn motion(
        &mut self,
        data: &mut Otto<BackendData>,
        _handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        _focus: Option<(
            <Otto<BackendData> as smithay::input::SeatHandler>::TouchFocus,
            Point<f64, Logical>,
        )>,
        event: &smithay::input::touch::MotionEvent,
        _seq: Serial,
    ) {
        if event.slot != self.start_data.slot {
            return;
        }

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        data.workspaces
            .map_window(&self.window, new_location.to_i32_round(), true, None);
    }

    fn frame(
        &mut self,
        _data: &mut Otto<BackendData>,
        _handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        _seq: Serial,
    ) {
    }

    fn cancel(
        &mut self,
        data: &mut Otto<BackendData>,
        handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        seq: Serial,
    ) {
        handle.cancel(data, seq);
        handle.unset_grab(self, data);
    }

    fn shape(
        &mut self,
        data: &mut Otto<BackendData>,
        handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        event: &smithay::input::touch::ShapeEvent,
        seq: Serial,
    ) {
        handle.shape(data, event, seq);
    }

    fn orientation(
        &mut self,
        data: &mut Otto<BackendData>,
        handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        event: &smithay::input::touch::OrientationEvent,
        seq: Serial,
    ) {
        handle.orientation(data, event, seq);
    }

    fn start_data(&self) -> &smithay::input::touch::GrabStartData<Otto<BackendData>> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Otto<BackendData>) {}
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ResizeEdge: u32 {
        const NONE = 0;
        const TOP = 1;
        const BOTTOM = 2;
        const LEFT = 4;
        const TOP_LEFT = 5;
        const BOTTOM_LEFT = 6;
        const RIGHT = 8;
        const TOP_RIGHT = 9;
        const BOTTOM_RIGHT = 10;
    }
}

impl From<xdg_toplevel::ResizeEdge> for ResizeEdge {
    #[inline]
    fn from(x: xdg_toplevel::ResizeEdge) -> Self {
        Self::from_bits(x as u32).unwrap()
    }
}

impl From<ResizeEdge> for xdg_toplevel::ResizeEdge {
    #[inline]
    fn from(x: ResizeEdge) -> Self {
        Self::try_from(x.bits()).unwrap()
    }
}

#[cfg(feature = "xwayland")]
impl From<X11ResizeEdge> for ResizeEdge {
    fn from(edge: X11ResizeEdge) -> Self {
        match edge {
            X11ResizeEdge::Bottom => ResizeEdge::BOTTOM,
            X11ResizeEdge::BottomLeft => ResizeEdge::BOTTOM_LEFT,
            X11ResizeEdge::BottomRight => ResizeEdge::BOTTOM_RIGHT,
            X11ResizeEdge::Left => ResizeEdge::LEFT,
            X11ResizeEdge::Right => ResizeEdge::RIGHT,
            X11ResizeEdge::Top => ResizeEdge::TOP,
            X11ResizeEdge::TopLeft => ResizeEdge::TOP_LEFT,
            X11ResizeEdge::TopRight => ResizeEdge::TOP_RIGHT,
        }
    }
}

/// Information about the resize operation.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ResizeData {
    /// The edges the surface is being resized with.
    pub edges: ResizeEdge,
    /// The initial window location.
    pub initial_window_location: Point<i32, Logical>,
    /// The initial window size (geometry width and height).
    pub initial_window_size: Size<i32, Logical>,
}

/// State of the resize operation.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum ResizeState {
    /// The surface is not being resized.
    #[default]
    NotResizing,
    /// The surface is currently being resized.
    Resizing(ResizeData),
    /// The resize has finished, and the surface needs to ack the final configure.
    WaitingForFinalAck(ResizeData, Serial),
    /// The resize has finished, and the surface needs to commit its final state.
    WaitingForCommit(ResizeData),
}

impl<B: Backend> PointerGrab<Otto<B>> for PointerResizeSurfaceGrab<B> {
    fn motion(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        _focus: Option<(PointerFocusTarget<B>, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        data.is_resizing = true;
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        // Same as the move grab: the resized window is being watched — keep
        // its frame callbacks and the blur rebuilds at full rate.
        data.pointer_interaction = Some((self.window.id(), std::time::Instant::now()));

        // It is impossible to get `min_size` and `max_size` of dead toplevel, so we return early.
        if !self.window.alive() {
            handle.unset_grab(self, data, event.serial, event.time, true);
            return;
        }

        let (mut dx, mut dy) = (event.location - self.start_data.location).into();

        let mut new_window_width = self.initial_window_size.w;
        let mut new_window_height = self.initial_window_size.h;

        let left_right = ResizeEdge::LEFT | ResizeEdge::RIGHT;
        let top_bottom = ResizeEdge::TOP | ResizeEdge::BOTTOM;

        if self.edges.intersects(left_right) {
            if self.edges.intersects(ResizeEdge::LEFT) {
                dx = -dx;
            }

            new_window_width = (self.initial_window_size.w as f64 + dx) as i32;
        }

        if self.edges.intersects(top_bottom) {
            if self.edges.intersects(ResizeEdge::TOP) {
                dy = -dy;
            }

            new_window_height = (self.initial_window_size.h as f64 + dy) as i32;
        }

        let (min_size, max_size) = if let Some(surface) = self.window.wl_surface() {
            with_states(&surface, |states| {
                let mut guard = states.cached_state.get::<SurfaceCachedState>();
                let data = guard.current();
                (data.min_size, data.max_size)
            })
        } else {
            ((0, 0).into(), (0, 0).into())
        };

        let min_width = min_size.w.max(1);
        let min_height = min_size.h.max(1);
        let max_width = if max_size.w == 0 {
            i32::MAX
        } else {
            max_size.w
        };
        let max_height = if max_size.h == 0 {
            i32::MAX
        } else {
            max_size.h
        };

        new_window_width = new_window_width.max(min_width).min(max_width);
        new_window_height = new_window_height.max(min_height).min(max_height);

        self.last_window_size = (new_window_width, new_window_height).into();

        match &self.window.underlying_surface() {
            WindowSurface::Wayland(xdg) => {
                xdg.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Resizing);
                    // The drag is measured on the window rect, titlebar
                    // included; the client only ever owns what is under the
                    // bar. Configuring it with the full rect makes a
                    // server-decorated window gain the bar's height on every
                    // resize (see `WindowElement::client_size`).
                    state.size = Some(self.window.client_size(self.last_window_size));
                });
                xdg.send_pending_configure();

                // Reposition window during resize if resizing from top or left edges
                if self.edges.intersects(ResizeEdge::TOP_LEFT) {
                    let geometry = self.window.geometry();
                    let mut location = data.workspaces.element_location(&self.window).unwrap();

                    if self.edges.intersects(ResizeEdge::LEFT) {
                        location.x = self.initial_window_location.x
                            + (self.initial_window_size.w - geometry.size.w);
                    }
                    if self.edges.intersects(ResizeEdge::TOP) {
                        location.y = self.initial_window_location.y
                            + (self.initial_window_size.h - geometry.size.h);
                    }

                    data.workspaces
                        .map_window(&self.window, location, true, None);
                }
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(x11) => {
                let mut location = data.workspaces.element_location(&self.window).unwrap();

                // Reposition window during resize if resizing from top or left edges
                if self.edges.intersects(ResizeEdge::TOP_LEFT) {
                    let geometry = self.window.geometry();

                    if self.edges.intersects(ResizeEdge::LEFT) {
                        location.x = self.initial_window_location.x
                            + (self.initial_window_size.w - geometry.size.w);
                    }
                    if self.edges.intersects(ResizeEdge::TOP) {
                        location.y = self.initial_window_location.y
                            + (self.initial_window_size.h - geometry.size.h);
                    }

                    if let Some(space) = data.workspaces.space_mut() {
                        space.map_element(self.window.clone(), location, true);
                    }
                }

                x11.configure(Rectangle::new(location, self.last_window_size))
                    .unwrap();
            }
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        focus: Option<(PointerFocusTarget<B>, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        state: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &ButtonEvent,
    ) {
        handle.button(state, event);
        if handle.current_pressed().is_empty() {
            state.is_resizing = false;
            // No more buttons are pressed, release the grab.
            handle.unset_grab(self, state, event.serial, event.time, true);

            // If toplevel is dead, we can't resize it, so we return early.
            if !self.window.alive() {
                return;
            }

            match &self.window.underlying_surface() {
                WindowSurface::Wayland(xdg) => {
                    xdg.with_pending_state(|state| {
                        state.states.unset(xdg_toplevel::State::Resizing);
                        state.size = Some(self.window.client_size(self.last_window_size));
                    });
                    xdg.send_pending_configure();

                    with_states(&self.window.wl_surface().unwrap(), |states| {
                        let mut data = states
                            .data_map
                            .get::<RefCell<SurfaceData>>()
                            .unwrap()
                            .borrow_mut();
                        if let ResizeState::Resizing(resize_data) = data.resize_state {
                            data.resize_state =
                                ResizeState::WaitingForFinalAck(resize_data, event.serial);
                        } else {
                            panic!("invalid resize state: {:?}", data.resize_state);
                        }
                    });
                }
                #[cfg(feature = "xwayland")]
                WindowSurface::X11(x11) => {
                    let location = state.workspaces.element_location(&self.window).unwrap();
                    x11.configure(Rectangle::new(location, self.last_window_size))
                        .unwrap();

                    let Some(surface) = self.window.wl_surface() else {
                        // X11 Window got unmapped, abort
                        return;
                    };
                    with_states(&surface, |states| {
                        let mut data = states
                            .data_map
                            .get::<RefCell<SurfaceData>>()
                            .unwrap()
                            .borrow_mut();
                        if let ResizeState::Resizing(resize_data) = data.resize_state {
                            data.resize_state = ResizeState::WaitingForCommit(resize_data);
                        } else {
                            // panic!("invalid resize state: {:?}", data.resize_state);
                        }
                    });
                }
            }
        }
    }

    fn axis(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Otto<B>, handle: &mut PointerInnerHandle<'_, Otto<B>>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Otto<B>,
        handle: &mut PointerInnerHandle<'_, Otto<B>>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &PointerGrabStartData<Otto<B>> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Otto<B>) {}
}

pub struct TouchResizeSurfaceGrab<BackendData: Backend + 'static> {
    pub start_data: TouchGrabStartData<Otto<BackendData>>,
    pub window: WindowElement,
    pub edges: ResizeEdge,
    pub initial_window_location: Point<i32, Logical>,
    pub initial_window_size: Size<i32, Logical>,
    pub last_window_size: Size<i32, Logical>,
}

impl<BackendData: Backend> TouchGrab<Otto<BackendData>> for TouchResizeSurfaceGrab<BackendData> {
    fn down(
        &mut self,
        _data: &mut Otto<BackendData>,
        _handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        _focus: Option<(
            <Otto<BackendData> as smithay::input::SeatHandler>::TouchFocus,
            Point<f64, Logical>,
        )>,
        _event: &smithay::input::touch::DownEvent,
        _seq: Serial,
    ) {
    }

    fn up(
        &mut self,
        state: &mut Otto<BackendData>,
        handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        event: &smithay::input::touch::UpEvent,
        _seq: Serial,
    ) {
        if event.slot != self.start_data.slot {
            return;
        }
        handle.unset_grab(self, state);

        // If toplevel is dead, we can't resize it, so we return early.
        if !self.window.alive() {
            return;
        }

        match self.window.underlying_surface() {
            WindowSurface::Wayland(xdg) => {
                xdg.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Resizing);
                    state.size = Some(self.window.client_size(self.last_window_size));
                });
                xdg.send_pending_configure();

                with_states(&self.window.wl_surface().unwrap(), |states| {
                    let mut data = states
                        .data_map
                        .get::<RefCell<SurfaceData>>()
                        .unwrap()
                        .borrow_mut();
                    if let ResizeState::Resizing(resize_data) = data.resize_state {
                        data.resize_state =
                            ResizeState::WaitingForFinalAck(resize_data, event.serial);
                    } else {
                        panic!("invalid resize state: {:?}", data.resize_state);
                    }
                });
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(x11) => {
                let location = state.workspaces.element_location(&self.window).unwrap();
                x11.configure(Rectangle::new(location, self.last_window_size))
                    .unwrap();

                let Some(surface) = self.window.wl_surface() else {
                    // X11 Window got unmapped, abort
                    return;
                };
                with_states(&surface, |states| {
                    let mut data = states
                        .data_map
                        .get::<RefCell<SurfaceData>>()
                        .unwrap()
                        .borrow_mut();
                    if let ResizeState::Resizing(resize_data) = data.resize_state {
                        data.resize_state = ResizeState::WaitingForCommit(resize_data);
                    } else {
                        // panic!("invalid resize state: {:?}", data.resize_state);
                    }
                });
            }
        }
    }

    fn motion(
        &mut self,
        data: &mut Otto<BackendData>,
        handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        _focus: Option<(
            <Otto<BackendData> as smithay::input::SeatHandler>::TouchFocus,
            Point<f64, Logical>,
        )>,
        event: &smithay::input::touch::MotionEvent,
        _seq: Serial,
    ) {
        if event.slot != self.start_data.slot {
            return;
        }

        // It is impossible to get `min_size` and `max_size` of dead toplevel, so we return early.
        if !self.window.alive() {
            handle.unset_grab(self, data);
            return;
        }

        let (mut dx, mut dy) = (event.location - self.start_data.location).into();

        let mut new_window_width = self.initial_window_size.w;
        let mut new_window_height = self.initial_window_size.h;

        let left_right = ResizeEdge::LEFT | ResizeEdge::RIGHT;
        let top_bottom = ResizeEdge::TOP | ResizeEdge::BOTTOM;

        // println!("new_window_width: {}, new_window_height: {}", new_window_width, new_window_height);
        if self.edges.intersects(left_right) {
            if self.edges.intersects(ResizeEdge::LEFT) {
                dx = -dx;
            }

            new_window_width = (self.initial_window_size.w as f64 + dx) as i32;
        }

        if self.edges.intersects(top_bottom) {
            if self.edges.intersects(ResizeEdge::TOP) {
                dy = -dy;
            }

            new_window_height = (self.initial_window_size.h as f64 + dy) as i32;
        }

        let (min_size, max_size) = if let Some(surface) = self.window.wl_surface() {
            with_states(&surface, |states| {
                let mut guard = states.cached_state.get::<SurfaceCachedState>();
                let data = guard.current();
                (data.min_size, data.max_size)
            })
        } else {
            ((0, 0).into(), (0, 0).into())
        };

        let min_width = min_size.w.max(1);
        let min_height = min_size.h.max(1);
        let max_width = if max_size.w == 0 {
            i32::MAX
        } else {
            max_size.w
        };
        let max_height = if max_size.h == 0 {
            i32::MAX
        } else {
            max_size.h
        };

        new_window_width = new_window_width.max(min_width).min(max_width);
        new_window_height = new_window_height.max(min_height).min(max_height);

        self.last_window_size = (new_window_width, new_window_height).into();

        match self.window.underlying_surface() {
            WindowSurface::Wayland(xdg) => {
                xdg.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Resizing);
                    // The drag is measured on the window rect, titlebar
                    // included; the client only ever owns what is under the
                    // bar. Configuring it with the full rect makes a
                    // server-decorated window gain the bar's height on every
                    // resize (see `WindowElement::client_size`).
                    state.size = Some(self.window.client_size(self.last_window_size));
                });
                xdg.send_pending_configure();

                // Reposition window during resize if resizing from top or left edges
                if self.edges.intersects(ResizeEdge::TOP_LEFT) {
                    let mut location = data.workspaces.element_location(&self.window).unwrap();

                    if self.edges.intersects(ResizeEdge::LEFT) {
                        location.x = self.initial_window_location.x
                            + (self.initial_window_size.w - self.last_window_size.w);
                    }
                    if self.edges.intersects(ResizeEdge::TOP) {
                        location.y = self.initial_window_location.y
                            + (self.initial_window_size.h - self.last_window_size.h);
                    }

                    data.workspaces
                        .map_window(&self.window, location, true, None);
                }
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(x11) => {
                let mut location = data.workspaces.element_location(&self.window).unwrap();

                // Reposition window during resize if resizing from top or left edges
                if self.edges.intersects(ResizeEdge::TOP_LEFT) {
                    let geometry = self.window.geometry();

                    if self.edges.intersects(ResizeEdge::LEFT) {
                        location.x = self.initial_window_location.x
                            + (self.initial_window_size.w - geometry.size.w);
                    }
                    if self.edges.intersects(ResizeEdge::TOP) {
                        location.y = self.initial_window_location.y
                            + (self.initial_window_size.h - geometry.size.h);
                    }

                    if let Some(space) = data.workspaces.space_mut() {
                        space.map_element(self.window.clone(), location, true);
                    }
                }

                x11.configure(Rectangle::new(location, self.last_window_size))
                    .unwrap();
            }
        }
    }

    fn frame(
        &mut self,
        _data: &mut Otto<BackendData>,
        _handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        _seq: Serial,
    ) {
    }

    fn cancel(
        &mut self,
        data: &mut Otto<BackendData>,
        handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        seq: Serial,
    ) {
        handle.cancel(data, seq);
        handle.unset_grab(self, data);
    }

    fn shape(
        &mut self,
        data: &mut Otto<BackendData>,
        handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        event: &smithay::input::touch::ShapeEvent,
        seq: Serial,
    ) {
        handle.shape(data, event, seq);
    }

    fn orientation(
        &mut self,
        data: &mut Otto<BackendData>,
        handle: &mut smithay::input::touch::TouchInnerHandle<'_, Otto<BackendData>>,
        event: &smithay::input::touch::OrientationEvent,
        seq: Serial,
    ) {
        handle.orientation(data, event, seq);
    }

    fn start_data(&self) -> &smithay::input::touch::GrabStartData<Otto<BackendData>> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Otto<BackendData>) {}
}
