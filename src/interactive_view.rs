use std::{
    hash::Hash,
    sync::{Arc, RwLock},
};

use smithay::{
    input::{
        keyboard::KeyboardTarget,
        pointer::{MotionEvent, PointerTarget},
        touch::TouchTarget,
    },
    utils::IsAlive,
};

use crate::{state::Backend, Otto};

pub trait ViewInteractions<B: Backend>: Sync + Send {
    fn id(&self) -> Option<usize>;
    fn is_alive(&self) -> bool;
    fn on_motion(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _event: &smithay::input::pointer::MotionEvent,
    ) {
    }
    fn on_relative_motion(&self, _event: &smithay::input::pointer::RelativeMotionEvent) {}
    fn on_button(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _event: &smithay::input::pointer::ButtonEvent,
    ) {
    }
    fn on_axis(&self, _event: &smithay::input::pointer::AxisFrame) {}
    fn on_enter(&self, _event: &smithay::input::pointer::MotionEvent) {}
    fn on_leave(&self, _serial: smithay::utils::Serial, _time: u32) {}
    fn on_frame(&self) {}
    fn on_gesture_hold_begin(&self, _event: &smithay::input::pointer::GestureHoldBeginEvent) {}
    fn on_gesture_hold_end(&self, _event: &smithay::input::pointer::GestureHoldEndEvent) {}
    fn on_gesture_pinch_begin(&self, _event: &smithay::input::pointer::GesturePinchBeginEvent) {}
    fn on_gesture_pinch_end(&self, _event: &smithay::input::pointer::GesturePinchEndEvent) {}
    fn on_gesture_pinch_update(&self, _event: &smithay::input::pointer::GesturePinchUpdateEvent) {}
    fn on_gesture_swipe_begin(&self, _event: &smithay::input::pointer::GestureSwipeBeginEvent) {}
    fn on_gesture_swipe_end(&self, _event: &smithay::input::pointer::GestureSwipeEndEvent) {}
    fn on_gesture_swipe_update(&self, _event: &smithay::input::pointer::GestureSwipeUpdateEvent) {}
    fn on_key(
        &self,
        _event: &smithay::input::keyboard::KeysymHandle<'_>,
        _state: smithay::backend::input::KeyState,
    ) {
    }
    fn on_key_with_data(
        &self,
        _event: &smithay::input::keyboard::KeysymHandle<'_>,
        _key_state: smithay::backend::input::KeyState,
        _data: &mut Otto<B>,
    ) {
    }
    fn on_modifiers(&self, _modifiers: smithay::input::keyboard::ModifiersState) {}
    fn on_keyboard_leave(&self) {}

    fn on_up(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _event: &smithay::input::touch::UpEvent,
        _seq: smithay::utils::Serial,
    ) {
    }
    fn on_down(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _event: &smithay::input::touch::DownEvent,
        _seq: smithay::utils::Serial,
    ) {
    }

    fn on_orientation(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _event: &smithay::input::touch::OrientationEvent,
        _seq: smithay::utils::Serial,
    ) {
    }
    fn on_shape(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _event: &smithay::input::touch::ShapeEvent,
        _seq: smithay::utils::Serial,
    ) {
    }
    fn on_cancel(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _seq: smithay::utils::Serial,
    ) {
    }
}
pub trait CloneBoxInteractions<B: Backend>: ViewInteractions<B> {
    fn clone_box(&self) -> Box<dyn CloneBoxInteractions<B>>;
}

impl<T, B: Backend> CloneBoxInteractions<B> for T
where
    T: 'static + ViewInteractions<B> + Clone,
{
    fn clone_box(&self) -> Box<dyn CloneBoxInteractions<B>> {
        Box::new(self.clone())
    }
}

pub struct InteractiveView<B: Backend> {
    pub view: Box<dyn CloneBoxInteractions<B>>,
}

impl<B: Backend> Clone for InteractiveView<B> {
    fn clone(&self) -> Self {
        InteractiveView {
            view: self.view.clone_box(),
        }
    }
}

impl<B: Backend> std::fmt::Debug for InteractiveView<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InteractiveView")
            .field("id", &self.view.id())
            .finish()
    }
}

impl<B: Backend> PartialEq for InteractiveView<B> {
    fn eq(&self, other: &Self) -> bool {
        self.view.id() == other.view.id()
    }
}
impl<S: Hash + Clone + 'static, B: Backend> ViewInteractions<B> for layers::prelude::View<S>
where
    Arc<RwLock<S>>: Send + Sync,
{
    fn id(&self) -> Option<usize> {
        self.layer.read().unwrap().as_ref().map(|l| l.id.0.into())
    }

    fn is_alive(&self) -> bool {
        !self
            .layer
            .read()
            .unwrap()
            .as_ref()
            .map(|l| l.hidden())
            .unwrap_or(true)
    }
}

impl<B: Backend> IsAlive for InteractiveView<B> {
    fn alive(&self) -> bool {
        self.view.is_alive()
    }
}

impl<B: Backend> PointerTarget<Otto<B>> for InteractiveView<B> {
    fn axis(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        frame: smithay::input::pointer::AxisFrame,
    ) {
        self.view.on_axis(&frame);
    }
    fn button(
        &self,
        seat: &smithay::input::Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        self.view.on_button(seat, data, event);
    }
    fn enter(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        self.view.on_enter(event);
    }
    fn frame(&self, _seat: &smithay::input::Seat<Otto<B>>, _data: &mut Otto<B>) {
        self.view.on_frame();
    }
    fn gesture_hold_begin(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::GestureHoldBeginEvent,
    ) {
        self.view.on_gesture_hold_begin(event);
    }
    fn gesture_hold_end(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::GestureHoldEndEvent,
    ) {
        self.view.on_gesture_hold_end(event);
    }
    fn gesture_pinch_begin(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::GesturePinchBeginEvent,
    ) {
        self.view.on_gesture_pinch_begin(event);
    }
    fn gesture_pinch_end(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::GesturePinchEndEvent,
    ) {
        self.view.on_gesture_pinch_end(event);
    }
    fn gesture_pinch_update(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::GesturePinchUpdateEvent,
    ) {
        self.view.on_gesture_pinch_update(event);
    }
    fn gesture_swipe_begin(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::GestureSwipeBeginEvent,
    ) {
        self.view.on_gesture_swipe_begin(event);
    }
    fn gesture_swipe_end(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::GestureSwipeEndEvent,
    ) {
        self.view.on_gesture_swipe_end(event);
    }
    fn gesture_swipe_update(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::GestureSwipeUpdateEvent,
    ) {
        self.view.on_gesture_swipe_update(event);
    }
    fn leave(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        serial: smithay::utils::Serial,
        time: u32,
    ) {
        self.view.on_leave(serial, time);
    }
    fn motion(
        &self,
        seat: &smithay::input::Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        self.view.on_motion(seat, data, event);
    }
    fn relative_motion(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        event: &smithay::input::pointer::RelativeMotionEvent,
    ) {
        self.view.on_relative_motion(event);
    }
}

impl<B: Backend> KeyboardTarget<Otto<B>> for InteractiveView<B> {
    fn enter(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _keys: Vec<smithay::input::keyboard::KeysymHandle<'_>>,
        _serial: smithay::utils::Serial,
    ) {
    }
    fn key(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        key: smithay::input::keyboard::KeysymHandle<'_>,
        _state: smithay::backend::input::KeyState,
        _serial: smithay::utils::Serial,
        _time: u32,
    ) {
        self.view.on_key(&key, _state);
        self.view.on_key_with_data(&key, _state, _data);
    }
    fn leave(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _serial: smithay::utils::Serial,
    ) {
        self.view.on_keyboard_leave();
    }
    fn modifiers(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        modifiers: smithay::input::keyboard::ModifiersState,
        _serial: smithay::utils::Serial,
    ) {
        self.view.on_modifiers(modifiers);
    }
}

impl<B: Backend> TouchTarget<Otto<B>> for InteractiveView<B> {
    fn up(
        &self,
        seat: &smithay::input::Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::UpEvent,
        seq: smithay::utils::Serial,
    ) {
        self.view.on_up(seat, data, event, seq);
    }
    fn down(
        &self,
        seat: &smithay::input::Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::DownEvent,
        seq: smithay::utils::Serial,
    ) {
        self.view.on_down(seat, data, event, seq);
    }
    fn motion(
        &self,
        seat: &smithay::input::Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::MotionEvent,
        seq: smithay::utils::Serial,
    ) {
        self.view.on_motion(
            seat,
            data,
            &MotionEvent {
                location: event.location,
                serial: seq,
                time: event.time,
            },
        );
    }
    fn frame(
        &self,
        _seat: &smithay::input::Seat<Otto<B>>,
        _data: &mut Otto<B>,
        _seq: smithay::utils::Serial,
    ) {
        self.view.on_frame();
    }
    fn orientation(
        &self,
        seat: &smithay::input::Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::OrientationEvent,
        seq: smithay::utils::Serial,
    ) {
        self.view.on_orientation(seat, data, event, seq);
    }
    fn shape(
        &self,
        seat: &smithay::input::Seat<Otto<B>>,
        data: &mut Otto<B>,
        event: &smithay::input::touch::ShapeEvent,
        seq: smithay::utils::Serial,
    ) {
        self.view.on_shape(seat, data, event, seq);
    }
    fn cancel(
        &self,
        seat: &smithay::input::Seat<Otto<B>>,
        data: &mut Otto<B>,
        seq: smithay::utils::Serial,
    ) {
        self.view.on_cancel(seat, data, seq);
    }
}
