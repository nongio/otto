use std::{borrow::Cow, fmt::Debug, hash::Hash, sync::RwLock};

use smithay::utils::Logical;
pub use smithay::{
    backend::input::KeyState,
    desktop::{LayerSurface, PopupKind},
    input::{
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerTarget, RelativeMotionEvent},
        Seat,
    },
    reexports::wayland_server::{backend::ObjectId, Resource},
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
};
use smithay::{
    desktop::WindowSurface,
    input::{
        dnd::{DndFocus, Source},
        pointer::{
            GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
            GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
            GestureSwipeEndEvent, GestureSwipeUpdateEvent,
        },
        touch::TouchTarget,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::Point,
    wayland::selection::data_device::DataDeviceHandler,
};

use crate::{
    interactive_view::InteractiveView,
    shell::WindowElement,
    state::{Backend, Otto},
    workspaces::{AppSwitcherView, DockView, WindowSelectorView, WorkspaceSelectorView},
};

pub enum KeyboardFocusTarget<B: Backend> {
    Window(WindowElement),
    LayerSurface(LayerSurface),
    Popup(PopupKind),
    View(InteractiveView<B>),
}

impl<B: Backend> PartialEq for KeyboardFocusTarget<B> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (KeyboardFocusTarget::Window(w1), KeyboardFocusTarget::Window(w2)) => w1 == w2,
            (KeyboardFocusTarget::LayerSurface(l1), KeyboardFocusTarget::LayerSurface(l2)) => {
                l1 == l2
            }
            (KeyboardFocusTarget::Popup(p1), KeyboardFocusTarget::Popup(p2)) => p1 == p2,
            (KeyboardFocusTarget::View(d1), KeyboardFocusTarget::View(d2)) => d1 == d2,
            _ => false,
        }
    }
}
impl<B: Backend> Clone for KeyboardFocusTarget<B> {
    fn clone(&self) -> Self {
        match self {
            KeyboardFocusTarget::Window(w) => KeyboardFocusTarget::Window(w.clone()),
            KeyboardFocusTarget::LayerSurface(l) => KeyboardFocusTarget::LayerSurface(l.clone()),
            KeyboardFocusTarget::Popup(p) => KeyboardFocusTarget::Popup(p.clone()),
            KeyboardFocusTarget::View(d) => KeyboardFocusTarget::View(d.clone()),
        }
    }
}

impl<B: Backend> Debug for KeyboardFocusTarget<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyboardFocusTarget::Window(w) => write!(f, "KeyboardFocusTarget::Window({:?})", w),
            KeyboardFocusTarget::LayerSurface(l) => {
                write!(f, "KeyboardFocusTarget::LayerSurface({:?})", l)
            }
            KeyboardFocusTarget::Popup(p) => write!(f, "KeyboardFocusTarget::Popup({:?})", p),
            KeyboardFocusTarget::View(d) => write!(f, "KeyboardFocusTarget::View({:?})", d),
        }
    }
}
impl<B: Backend> IsAlive for KeyboardFocusTarget<B> {
    #[inline]
    fn alive(&self) -> bool {
        match self {
            KeyboardFocusTarget::Window(w) => w.alive(),
            KeyboardFocusTarget::LayerSurface(l) => l.alive(),
            KeyboardFocusTarget::Popup(p) => p.alive(),
            KeyboardFocusTarget::View(d) => d.alive(),
        }
    }
}

pub enum PointerFocusTarget<B: Backend> {
    WlSurface(WlSurface),
    #[cfg(feature = "xwayland")]
    X11Surface(X11Surface),
    View(InteractiveView<B>),
}

impl<B: Backend> Debug for PointerFocusTarget<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PointerFocusTarget::WlSurface(w) => write!(f, "PointerFocusTarget::WlSurface({:?})", w),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                write!(f, "PointerFocusTarget::X11Surface({:?})", w)
            }
            PointerFocusTarget::View(d) => write!(f, "PointerFocusTarget::View({:?})", d),
        }
    }
}

impl<B: Backend> PartialEq for PointerFocusTarget<B> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PointerFocusTarget::WlSurface(w1), PointerFocusTarget::WlSurface(w2)) => w1 == w2,
            #[cfg(feature = "xwayland")]
            (PointerFocusTarget::X11Surface(w1), PointerFocusTarget::X11Surface(w2)) => w1 == w2,
            (PointerFocusTarget::View(d1), PointerFocusTarget::View(d2)) => d1 == d2,
            _ => false,
        }
    }
}

impl<B: Backend> Clone for PointerFocusTarget<B> {
    fn clone(&self) -> Self {
        match self {
            PointerFocusTarget::WlSurface(w) => PointerFocusTarget::WlSurface(w.clone()),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => PointerFocusTarget::X11Surface(w.clone()),
            PointerFocusTarget::View(d) => PointerFocusTarget::View(d.clone()),
        }
    }
}

impl<B: Backend> IsAlive for PointerFocusTarget<B> {
    #[inline]
    fn alive(&self) -> bool {
        match self {
            PointerFocusTarget::WlSurface(w) => w.alive(),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => w.alive(),
            PointerFocusTarget::View(w) => w.alive(),
        }
    }
}

impl<B: Backend> From<PointerFocusTarget<B>> for WlSurface {
    #[inline]
    fn from(target: PointerFocusTarget<B>) -> Self {
        target.wl_surface().unwrap().into_owned()
    }
}

impl<B: Backend> PointerTarget<Otto<B>> for PointerFocusTarget<B> {
    fn enter(&self, seat: &Seat<Otto<B>>, data: &mut Otto<B>, event: &MotionEvent) {
        match self {
            PointerFocusTarget::WlSurface(w) => PointerTarget::enter(w, seat, data, event),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => PointerTarget::enter(w, seat, data, event),
            PointerFocusTarget::View(w) => PointerTarget::enter(w, seat, data, event),
        }
    }
    fn motion(&self, seat: &Seat<Otto<B>>, data: &mut Otto<B>, event: &MotionEvent) {
        match self {
            PointerFocusTarget::WlSurface(w) => PointerTarget::motion(w, seat, data, event),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => PointerTarget::motion(w, seat, data, event),
            PointerFocusTarget::View(w) => PointerTarget::motion(w, seat, data, event),
        }
    }
    fn relative_motion(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &RelativeMotionEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                PointerTarget::relative_motion(w, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                PointerTarget::relative_motion(w, seat, data, event)
            }
            PointerFocusTarget::View(w) => PointerTarget::relative_motion(w, seat, data, event),
        }
    }
    fn button(&self, seat: &Seat<Otto<B>>, data: &mut Otto<B>, event: &ButtonEvent) {
        match self {
            PointerFocusTarget::WlSurface(w) => PointerTarget::button(w, seat, data, event),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => PointerTarget::button(w, seat, data, event),
            PointerFocusTarget::View(w) => PointerTarget::button(w, seat, data, event),
        }
    }
    fn axis(&self, seat: &Seat<Otto<B>>, data: &mut Otto<B>, frame: AxisFrame) {
        match self {
            PointerFocusTarget::WlSurface(w) => PointerTarget::axis(w, seat, data, frame),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => PointerTarget::axis(w, seat, data, frame),
            PointerFocusTarget::View(w) => PointerTarget::axis(w, seat, data, frame),
        }
    }
    fn frame(&self, seat: &Seat<Otto<B>>, data: &mut Otto<B>) {
        match self {
            PointerFocusTarget::WlSurface(w) => PointerTarget::frame(w, seat, data),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => PointerTarget::frame(w, seat, data),
            PointerFocusTarget::View(w) => PointerTarget::frame(w, seat, data),
        }
    }
    fn leave(&self, seat: &Seat<Otto<B>>, data: &mut Otto<B>, serial: Serial, time: u32) {
        match self {
            PointerFocusTarget::WlSurface(w) => PointerTarget::leave(w, seat, data, serial, time),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => PointerTarget::leave(w, seat, data, serial, time),
            PointerFocusTarget::View(w) => PointerTarget::leave(w, seat, data, serial, time),
        }
    }
    fn gesture_swipe_begin(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &GestureSwipeBeginEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                PointerTarget::gesture_swipe_begin(w, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                PointerTarget::gesture_swipe_begin(w, seat, data, event)
            }
            PointerFocusTarget::View(w) => PointerTarget::gesture_swipe_begin(w, seat, data, event),
        }
    }
    fn gesture_swipe_update(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &GestureSwipeUpdateEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                PointerTarget::gesture_swipe_update(w, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                PointerTarget::gesture_swipe_update(w, seat, data, event)
            }
            PointerFocusTarget::View(w) => {
                PointerTarget::gesture_swipe_update(w, seat, data, event)
            }
        }
    }
    fn gesture_swipe_end(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &GestureSwipeEndEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                PointerTarget::gesture_swipe_end(w, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                PointerTarget::gesture_swipe_end(w, seat, data, event)
            }
            PointerFocusTarget::View(w) => PointerTarget::gesture_swipe_end(w, seat, data, event),
        }
    }
    fn gesture_pinch_begin(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &GesturePinchBeginEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                PointerTarget::gesture_pinch_begin(w, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                PointerTarget::gesture_pinch_begin(w, seat, data, event)
            }
            PointerFocusTarget::View(w) => PointerTarget::gesture_pinch_begin(w, seat, data, event),
        }
    }
    fn gesture_pinch_update(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &GesturePinchUpdateEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                PointerTarget::gesture_pinch_update(w, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                PointerTarget::gesture_pinch_update(w, seat, data, event)
            }
            PointerFocusTarget::View(w) => {
                PointerTarget::gesture_pinch_update(w, seat, data, event)
            }
        }
    }
    fn gesture_pinch_end(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &GesturePinchEndEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                PointerTarget::gesture_pinch_end(w, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                PointerTarget::gesture_pinch_end(w, seat, data, event)
            }
            PointerFocusTarget::View(w) => PointerTarget::gesture_pinch_end(w, seat, data, event),
        }
    }
    fn gesture_hold_begin(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &GestureHoldBeginEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                PointerTarget::gesture_hold_begin(w, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                PointerTarget::gesture_hold_begin(w, seat, data, event)
            }
            PointerFocusTarget::View(w) => PointerTarget::gesture_hold_begin(w, seat, data, event),
        }
    }
    fn gesture_hold_end(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &GestureHoldEndEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                PointerTarget::gesture_hold_end(w, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                PointerTarget::gesture_hold_end(w, seat, data, event)
            }
            PointerFocusTarget::View(w) => PointerTarget::gesture_hold_end(w, seat, data, event),
        }
    }
}

impl<B: Backend> KeyboardTarget<Otto<B>> for KeyboardFocusTarget<B> {
    fn enter(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        match self {
            KeyboardFocusTarget::Window(w) => match w.underlying_surface() {
                WindowSurface::Wayland(w) => {
                    KeyboardTarget::enter(w.wl_surface(), seat, data, keys, serial)
                }
                #[cfg(feature = "xwayland")]
                WindowSurface::X11(s) => KeyboardTarget::enter(s, seat, data, keys, serial),
            },
            KeyboardFocusTarget::LayerSurface(l) => {
                KeyboardTarget::enter(l.wl_surface(), seat, data, keys, serial)
            }
            KeyboardFocusTarget::Popup(p) => {
                KeyboardTarget::enter(p.wl_surface(), seat, data, keys, serial)
            }
            KeyboardFocusTarget::View(d) => KeyboardTarget::enter(d, seat, data, keys, serial),
        }
    }
    fn leave(&self, seat: &Seat<Otto<B>>, data: &mut Otto<B>, serial: Serial) {
        // Show popups for the window gaining focus
        if let KeyboardFocusTarget::Window(w) = self {
            let window_id = w.wl_surface().map(|s| s.id());
            if let Some(id) = window_id {
                data.workspaces.popup_overlay.show_popups_for_window(&id);
            }
        }
        // Hide popups for the window losing focus
        if let KeyboardFocusTarget::Window(w) = self {
            let window_id = w.wl_surface().map(|s| s.id());
            if let Some(id) = window_id {
                data.workspaces.popup_overlay.hide_popups_for_window(&id);
            }
        }

        match self {
            KeyboardFocusTarget::Window(w) => match w.underlying_surface() {
                WindowSurface::Wayland(w) => {
                    KeyboardTarget::leave(w.wl_surface(), seat, data, serial)
                }
                #[cfg(feature = "xwayland")]
                WindowSurface::X11(s) => KeyboardTarget::leave(s, seat, data, serial),
            },
            KeyboardFocusTarget::LayerSurface(l) => {
                KeyboardTarget::leave(l.wl_surface(), seat, data, serial)
            }
            KeyboardFocusTarget::Popup(p) => {
                KeyboardTarget::leave(p.wl_surface(), seat, data, serial)
            }
            KeyboardFocusTarget::View(d) => KeyboardTarget::leave(d, seat, data, serial),
        }
    }
    fn key(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        match self {
            KeyboardFocusTarget::Window(w) => match w.underlying_surface() {
                WindowSurface::Wayland(w) => {
                    KeyboardTarget::key(w.wl_surface(), seat, data, key, state, serial, time)
                }
                #[cfg(feature = "xwayland")]
                WindowSurface::X11(s) => {
                    KeyboardTarget::key(s, seat, data, key, state, serial, time)
                }
            },
            KeyboardFocusTarget::LayerSurface(l) => {
                KeyboardTarget::key(l.wl_surface(), seat, data, key, state, serial, time)
            }
            KeyboardFocusTarget::Popup(p) => {
                KeyboardTarget::key(p.wl_surface(), seat, data, key, state, serial, time)
            }
            KeyboardFocusTarget::View(d) => {
                KeyboardTarget::key(d, seat, data, key, state, serial, time)
            }
        }
    }
    /// Hold modifiers were changed on a keyboard from a given seat
    fn modifiers(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        match self {
            KeyboardFocusTarget::Window(w) => match w.underlying_surface() {
                WindowSurface::Wayland(w) => {
                    KeyboardTarget::modifiers(w.wl_surface(), seat, data, modifiers, serial)
                }
                #[cfg(feature = "xwayland")]
                WindowSurface::X11(s) => {
                    KeyboardTarget::modifiers(s, seat, data, modifiers, serial)
                }
            },
            KeyboardFocusTarget::LayerSurface(l) => {
                KeyboardTarget::modifiers(l.wl_surface(), seat, data, modifiers, serial)
            }
            KeyboardFocusTarget::Popup(p) => {
                KeyboardTarget::modifiers(p.wl_surface(), seat, data, modifiers, serial)
            }
            KeyboardFocusTarget::View(d) => {
                KeyboardTarget::modifiers(d, seat, data, modifiers, serial)
            }
        }
    }
}

impl<B: Backend> TouchTarget<Otto<B>> for PointerFocusTarget<B> {
    fn down(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::DownEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => TouchTarget::down(w, seat, data, event, seq),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => TouchTarget::down(w, seat, data, event, seq),
            PointerFocusTarget::View(w) => TouchTarget::down(w, seat, data, event, seq),
        }
    }

    fn up(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::UpEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => TouchTarget::up(w, seat, data, event, seq),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => TouchTarget::up(w, seat, data, event, seq),
            PointerFocusTarget::View(w) => TouchTarget::up(w, seat, data, event, seq),
        }
    }

    fn motion(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::MotionEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => TouchTarget::motion(w, seat, data, event, seq),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => TouchTarget::motion(w, seat, data, event, seq),
            PointerFocusTarget::View(w) => TouchTarget::motion(w, seat, data, event, seq),
        }
    }

    fn frame(&self, seat: &Seat<Otto<B>>, data: &mut Otto<B>, seq: Serial) {
        match self {
            PointerFocusTarget::WlSurface(w) => TouchTarget::frame(w, seat, data, seq),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => TouchTarget::frame(w, seat, data, seq),
            PointerFocusTarget::View(w) => TouchTarget::frame(w, seat, data, seq),
        }
    }

    fn cancel(&self, seat: &Seat<Otto<B>>, data: &mut Otto<B>, seq: Serial) {
        match self {
            PointerFocusTarget::WlSurface(w) => TouchTarget::cancel(w, seat, data, seq),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => TouchTarget::cancel(w, seat, data, seq),
            PointerFocusTarget::View(w) => TouchTarget::cancel(w, seat, data, seq),
        }
    }

    fn shape(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::ShapeEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => TouchTarget::shape(w, seat, data, event, seq),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => TouchTarget::shape(w, seat, data, event, seq),
            PointerFocusTarget::View(w) => TouchTarget::shape(w, seat, data, event, seq),
        }
    }

    fn orientation(
        &self,
        seat: &Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::OrientationEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => TouchTarget::orientation(w, seat, data, event, seq),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                TouchTarget::orientation(w, seat, data, event, seq)
            }
            PointerFocusTarget::View(w) => TouchTarget::orientation(w, seat, data, event, seq),
        }
    }
}

impl<B: Backend> WaylandFocus for PointerFocusTarget<B> {
    #[inline]
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            PointerFocusTarget::WlSurface(w) => w.wl_surface(),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => w.wl_surface().map(Cow::Owned),
            PointerFocusTarget::View(_) => None,
        }
    }
    #[inline]
    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        match self {
            PointerFocusTarget::WlSurface(w) => w.same_client_as(object_id),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => w.same_client_as(object_id),
            PointerFocusTarget::View(_) => false,
        }
    }
}

impl<B: Backend> WaylandFocus for KeyboardFocusTarget<B> {
    #[inline]
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            KeyboardFocusTarget::Window(w) => w.wl_surface(),
            KeyboardFocusTarget::LayerSurface(l) => Some(Cow::Borrowed(l.wl_surface())),
            KeyboardFocusTarget::Popup(p) => Some(Cow::Borrowed(p.wl_surface())),
            KeyboardFocusTarget::View(_) => None,
        }
    }
}

impl<B: Backend> From<WindowElement> for PointerFocusTarget<B> {
    fn from(w: WindowElement) -> Self {
        match w.underlying_surface() {
            WindowSurface::Wayland(w) => PointerFocusTarget::from(w.wl_surface()),
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(s) => PointerFocusTarget::X11Surface(s.clone()),
        }
    }
}

impl<B: Backend> From<WindowElement> for KeyboardFocusTarget<B> {
    fn from(w: WindowElement) -> Self {
        KeyboardFocusTarget::Window(w)
    }
}

impl<B: Backend> From<WlSurface> for PointerFocusTarget<B> {
    #[inline]
    fn from(value: WlSurface) -> Self {
        PointerFocusTarget::WlSurface(value)
    }
}

impl<B: Backend> From<&WlSurface> for PointerFocusTarget<B> {
    #[inline]
    fn from(value: &WlSurface) -> Self {
        PointerFocusTarget::from(value.clone())
    }
}
impl<B: Backend> From<LayerSurface> for PointerFocusTarget<B> {
    fn from(l: LayerSurface) -> Self {
        PointerFocusTarget::from(l.wl_surface())
    }
}

impl<B: Backend> From<LayerSurface> for KeyboardFocusTarget<B> {
    fn from(w: LayerSurface) -> Self {
        KeyboardFocusTarget::LayerSurface(w)
    }
}
impl<B: Backend> From<PopupKind> for PointerFocusTarget<B> {
    fn from(p: PopupKind) -> Self {
        PointerFocusTarget::from(p.wl_surface())
    }
}
impl<B: Backend> From<PopupKind> for KeyboardFocusTarget<B> {
    fn from(p: PopupKind) -> Self {
        KeyboardFocusTarget::Popup(p)
    }
}

impl<B: Backend> From<KeyboardFocusTarget<B>> for PointerFocusTarget<B> {
    fn from(k: KeyboardFocusTarget<B>) -> Self {
        PointerFocusTarget::from(&*k.wl_surface().unwrap())
    }
}
impl<S: Hash + Clone + 'static, B: Backend> From<layers::prelude::View<S>> for PointerFocusTarget<B>
where
    std::sync::Arc<RwLock<S>>: Send + Sync,
{
    fn from(value: layers::prelude::View<S>) -> Self {
        let view = value.clone();
        let d = InteractiveView {
            view: Box::new(view),
        };
        PointerFocusTarget::View(d)
    }
}

impl<B: Backend> From<WindowSelectorView> for PointerFocusTarget<B> {
    fn from(value: WindowSelectorView) -> Self {
        let view = value.clone();
        let d = InteractiveView {
            view: Box::new(view),
        };
        PointerFocusTarget::View(d)
    }
}

impl<B: Backend> From<AppSwitcherView> for PointerFocusTarget<B> {
    fn from(value: AppSwitcherView) -> Self {
        let view = value.clone();
        let d = InteractiveView {
            view: Box::new(view),
        };
        PointerFocusTarget::View(d)
    }
}

impl<B: Backend> From<DockView> for PointerFocusTarget<B> {
    fn from(value: DockView) -> Self {
        let view = value.clone();
        let d = InteractiveView {
            view: Box::new(view),
        };
        PointerFocusTarget::View(d)
    }
}

impl<B: Backend> From<WorkspaceSelectorView> for PointerFocusTarget<B> {
    fn from(value: WorkspaceSelectorView) -> Self {
        let view = value.clone();
        let d = InteractiveView {
            view: Box::new(view),
        };
        PointerFocusTarget::View(d)
    }
}

impl<B: Backend + 'static> DndFocus<Otto<B>> for PointerFocusTarget<B>
where
    Otto<B>: DataDeviceHandler,
{
    type OfferData<S: Source> = <WlSurface as DndFocus<Otto<B>>>::OfferData<S>;

    fn enter<S: Source>(
        &self,
        data: &mut Otto<B>,
        dh: &smithay::reexports::wayland_server::DisplayHandle,
        source: std::sync::Arc<S>,
        seat: &Seat<Otto<B>>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<Self::OfferData<S>> {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                DndFocus::enter(w, data, dh, source, seat, location, serial)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                DndFocus::enter(w, data, dh, source, seat, location, serial)
            }
            PointerFocusTarget::View(_) => None,
        }
    }

    fn motion<S: Source>(
        &self,
        data: &mut Otto<B>,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<Otto<B>>,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => {
                DndFocus::motion(w, data, offer, seat, location, time)
            }
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => {
                DndFocus::motion(w, data, offer, seat, location, time)
            }
            PointerFocusTarget::View(_) => {}
        }
    }

    fn leave<S: Source>(
        &self,
        data: &mut Otto<B>,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<Otto<B>>,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => DndFocus::leave(w, data, offer, seat),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => DndFocus::leave(w, data, offer, seat),
            PointerFocusTarget::View(_) => {}
        }
    }

    fn drop<S: Source>(
        &self,
        data: &mut Otto<B>,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<Otto<B>>,
    ) {
        match self {
            PointerFocusTarget::WlSurface(w) => DndFocus::drop(w, data, offer, seat),
            #[cfg(feature = "xwayland")]
            PointerFocusTarget::X11Surface(w) => DndFocus::drop(w, data, offer, seat),
            PointerFocusTarget::View(_) => {}
        }
    }
}
