//! Input handling for the server-side titlebar.
//!
//! The pixels come from otto-kit's `WindowDecoration`; so do the hit regions —
//! this view asks that same struct what is under the pointer, so a click can
//! never land somewhere different from what was drawn.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use otto_kit::components::titlebar::WindowControl;
use smithay::{
    backend::input::ButtonState,
    input::pointer::{CursorIcon, CursorImageStatus},
    reexports::wayland_server::Resource,
    utils::IsAlive,
};

use crate::{
    interactive_view::ViewInteractions, shell::WindowElement,
    workspaces::window_view::render::decoration_for,
};

const BTN_LEFT: u32 = 0x110;

/// How long after a press on the bar a second one still counts as a double
/// click, and how far it may wander in the meantime.
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_SLOP: f32 = 6.0;

/// When and where a press on the titlebar landed.
type PressMark = Arc<Mutex<Option<(Instant, (f32, f32))>>>;

/// A window's titlebar, as an input target.
#[derive(Clone)]
pub struct WindowDecorationView {
    pub window: WindowElement,
    /// A press that started on the bar (rather than on a control) — the drag
    /// that follows moves the window.
    press_on_bar: Arc<AtomicBool>,
    /// When and where the last press on the bar landed, for double-click
    /// detection.
    last_press: PressMark,
}

impl WindowDecorationView {
    pub fn new(window: WindowElement) -> Self {
        Self {
            window,
            press_on_bar: Arc::new(AtomicBool::new(false)),
            last_press: Arc::new(Mutex::new(None)),
        }
    }

    fn control_index(control: WindowControl) -> u8 {
        match control {
            WindowControl::Close => 0,
            WindowControl::Minimize => 1,
            WindowControl::Zoom => 2,
        }
    }

    /// Which control is under a titlebar-local point.
    fn control_at<B: crate::state::Backend>(
        &self,
        data: &crate::Otto<B>,
        x: f32,
        y: f32,
    ) -> Option<WindowControl> {
        let view = self.view(data)?;
        decoration_for(&view.decoration_state()).control_at(x, y)
    }

    fn view<B: crate::state::Backend>(
        &self,
        data: &crate::Otto<B>,
    ) -> Option<crate::workspaces::WindowView> {
        let id = self.window.wl_surface().map(|s| s.id())?;
        data.workspaces.get_window_view(&id).clone()
    }

    /// Whether a press at this titlebar-local point closes a double click,
    /// remembering it either way.
    fn take_double_click(&self, x: f32, y: f32) -> bool {
        let now = Instant::now();
        let mut last = self.last_press.lock().unwrap();
        let doubled = last.is_some_and(|(at, (px, py))| {
            now.duration_since(at) < DOUBLE_CLICK_INTERVAL
                && (x - px).abs() <= DOUBLE_CLICK_SLOP
                && (y - py).abs() <= DOUBLE_CLICK_SLOP
        });
        // A double click consumes its history, so a third press starts over.
        *last = if doubled { None } else { Some((now, (x, y))) };
        doubled
    }

    /// Toggle the window between maximized and its restored size.
    ///
    /// Deferred to an idle callback: callers run with the pointer's inner
    /// lock held, and (un)maximizing moves the focus, which takes it again.
    fn toggle_maximized<B: crate::state::Backend>(&self, data: &mut crate::Otto<B>) {
        let Some(toplevel) = self.window.toplevel().cloned() else {
            return;
        };
        let maximized = self.window.is_maximized();
        data.handle.insert_idle(move |state| {
            if maximized {
                smithay::wayland::shell::xdg::XdgShellHandler::unmaximize_request(state, toplevel);
            } else {
                smithay::wayland::shell::xdg::XdgShellHandler::maximize_request(state, toplevel);
            }
        });
    }

    /// Mutate the decoration model in place, repainting only on a real change.
    fn update_model<B: crate::state::Backend>(
        &self,
        data: &crate::Otto<B>,
        hovered: bool,
        pressed: Option<u8>,
    ) {
        let Some(view) = self.view(data) else {
            return;
        };
        let mut model = view.decoration_state();
        model.controls_hovered = hovered;
        model.pressed = pressed;
        view.update_decoration(model);
    }
}

impl PartialEq for WindowDecorationView {
    fn eq(&self, other: &Self) -> bool {
        self.window == other.window
    }
}

impl std::fmt::Debug for WindowDecorationView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowDecorationView")
            .field("window", &self.window.id())
            .finish()
    }
}

impl<Backend: crate::state::Backend> ViewInteractions<Backend> for WindowDecorationView {
    fn id(&self) -> Option<usize> {
        Some(self.window.base_layer().id.0.into())
    }

    fn is_alive(&self) -> bool {
        self.window.alive()
    }

    fn on_motion(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        data: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        let (x, y) = (event.location.x as f32, event.location.y as f32);
        let hovered = self.control_at(data, x, y).is_some();
        let pressed = self.view(data).and_then(|v| v.decoration_state().pressed);
        self.update_model(data, hovered, pressed);
        data.set_cursor(&CursorImageStatus::Named(CursorIcon::default()));
    }

    fn on_leave(&self, _serial: smithay::utils::Serial, _time: u32) {
        self.press_on_bar.store(false, Ordering::SeqCst);
    }

    fn on_leave_with_data(
        &self,
        data: &mut crate::Otto<Backend>,
        _serial: smithay::utils::Serial,
        _time: u32,
    ) {
        // The pointer can leave the bar without a last motion off the
        // controls — straight onto the client surface below, or out of the
        // window entirely — and the revealed glyphs would stay drawn.
        self.update_model(data, false, None);
    }

    fn on_button(
        &self,
        seat: &smithay::input::Seat<crate::Otto<Backend>>,
        data: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        if event.button != BTN_LEFT {
            return;
        }
        let location = data.last_pointer_location;
        let origin = data
            .workspaces
            .element_location(&self.window)
            .unwrap_or_default();
        let (x, y) = (
            location.0 as f32 - origin.x as f32,
            location.1 as f32 - origin.y as f32,
        );
        let control = self.control_at(data, x, y);

        match event.state {
            ButtonState::Pressed => {
                if let Some(control) = control {
                    self.last_press.lock().unwrap().take();
                    self.update_model(data, true, Some(Self::control_index(control)));
                } else if self.take_double_click(x, y) {
                    // Two clicks on the bar zoom the window instead of moving
                    // it, so no move grab is started for this press.
                    self.toggle_maximized(data);
                } else {
                    // Anywhere else on the bar starts a window move.
                    //
                    // The move grab CANNOT be installed from here: this runs
                    // inside `PointerHandle::button`, which holds the pointer's
                    // inner lock across the whole dispatch, and both
                    // `grab_start_data` and `set_grab` take that same
                    // non-reentrant lock — doing it inline deadlocks the
                    // compositor. Defer to an idle callback, which runs after
                    // the dispatch has released the lock.
                    self.press_on_bar.store(true, Ordering::SeqCst);
                    let window = self.window.clone();
                    let seat = seat.clone();
                    let serial = event.serial;
                    data.handle.insert_idle(move |state| {
                        state.move_request_ssd(&window, &seat, serial);
                    });
                }
            }
            ButtonState::Released => {
                self.press_on_bar.store(false, Ordering::SeqCst);
                let was_pressed = self
                    .view(data)
                    .and_then(|v| v.decoration_state().pressed)
                    .and_then(crate::workspaces::window_view::render::control_from_index);
                self.update_model(data, control.is_some(), None);

                // A control only fires when press and release land on it, the
                // same rule a button follows anywhere else.
                if was_pressed.is_none() || was_pressed != control {
                    return;
                }
                if was_pressed == Some(WindowControl::Zoom) {
                    self.toggle_maximized(data);
                    return;
                }
                let Some(toplevel) = self.window.toplevel().cloned() else {
                    return;
                };
                // Deferred for the same reason as the move grab above: this
                // runs with the pointer's inner lock held, and minimizing
                // moves the focus, which takes that lock again.
                data.handle.insert_idle(move |state| match was_pressed {
                    Some(WindowControl::Close) => toplevel.send_close(),
                    Some(WindowControl::Minimize) => {
                        smithay::wayland::shell::xdg::XdgShellHandler::minimize_request(
                            state, toplevel,
                        );
                    }
                    _ => {}
                });
            }
        }
    }
}
