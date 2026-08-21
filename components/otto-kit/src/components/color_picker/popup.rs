//! Client half: the picker popup that opens from a [`super::well`].
//!
//! Structured exactly like [`dropdown::menu`](crate::components::dropdown::menu) —
//! read that module's docs first, because the same `AppContext` reentrancy
//! trap applies here verbatim: [`ColorPickerPopup::new`] registers a pointer
//! callback in `AppContext`'s thread-local callback list, and that list has
//! no unregister. So [`ColorPickerPopup`] is built exactly once, during
//! window setup, and [`open`](ColorPickerPopup::open) only ever creates and
//! tears down the popup surface itself — never a second pointer callback.
//! Building it lazily inside a pointer handler reproduces the same
//! `RefCell already borrowed` panic dropdown's menu module documents.
//!
//! Unlike a dropdown, this popup draws its own content directly (a mode
//! switcher plus one of three interactive regions) rather than hosting a
//! [`ContextMenu`](crate::components::context_menu::ContextMenu) — there is
//! no list of [`MenuItem`](crate::components::menu_item::MenuItem)s to
//! reuse that machinery for, so this owns a raw
//! [`PopupSurface`](crate::surfaces::PopupSurface) the same way
//! `ContextMenu` itself does internally, and does its own pointer routing
//! into [`panel`](super::panel)'s hit-test helpers.
//!
//! **Mode switching never resizes the popup.** [`panel::panel_size`] is
//! sized to the tallest of the three modes ([`panel::max_content_height`]),
//! so clicking a switcher segment is a pure redraw — no positioner churn,
//! no reconfigure round-trip.
//!
//! **The hex/RGB mode has no inline text editing.** Wiring a full
//! `TextInput` into a popup that also has to route drag events for the HSV
//! square turned out to be a second complete input-focus story on top of
//! this one; a click on a hex/RGB row only selects it (drawn as a
//! highlighted field) as a hook for a future numeric-entry pass. Every
//! value shown there stays in sync with `color` regardless.

use std::cell::RefCell;
use std::rc::Rc;

use skia_safe::Rect;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::XdgPositioner;
use wayland_client::backend::ObjectId;
use wayland_client::Proxy;
use wayland_protocols::xdg::shell::client::{xdg_positioner, xdg_surface};

use crate::app_runner::AppContext;
use crate::protocols::otto_surface_style_v1::{BlendMode, ClipMode};
use crate::surfaces::PopupSurface;
use crate::theme::Theme;
use skia_safe::Color;

use super::hsv::rgb_to_hsv;
use super::panel::{self, HexField, Mode, Swatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    None,
    Square,
    Hue,
}

struct PopupInner {
    popup: Option<PopupSurface>,
    surface_id: Option<ObjectId>,
    mode: Mode,
    color: Color,
    swatches: Vec<Swatch>,
    selected_swatch: Option<usize>,
    selected_field: Option<HexField>,
    theme: Theme,
    drag: Drag,
    on_change: Option<Rc<dyn Fn(Color)>>,
    on_close: Option<Rc<dyn Fn()>>,
    /// A press landed on a surface of this client that is not the popup.
    /// Acted on at the end of the pointer batch — see [`ColorPickerPopup::new`].
    dismiss_pending: bool,
}

/// Owns the popup lifecycle for one colour well. The caller keeps one of
/// these per well (it is not `Clone` — there is no reason to share it),
/// alongside whatever colour state the well itself needs. Construct it
/// during window setup, not from inside a pointer-event handler — see the
/// module docs above.
pub struct ColorPickerPopup {
    inner: Rc<RefCell<PopupInner>>,
}

impl ColorPickerPopup {
    /// `swatches` is the caller-supplied preset list for [`Mode::Swatches`]
    /// — fixed for the popup's lifetime, the way Otto's `accent_color`
    /// setting only ever offers a fixed named set.
    pub fn new(swatches: Vec<Swatch>) -> Self {
        let inner = Rc::new(RefCell::new(PopupInner {
            popup: None,
            surface_id: None,
            mode: Mode::Swatches,
            color: Color::from_rgb(0x0A, 0x84, 0xFF),
            swatches,
            selected_swatch: None,
            selected_field: None,
            theme: Theme::light(),
            drag: Drag::None,
            on_change: None,
            on_close: None,
            dismiss_pending: false,
        }));

        let cb_inner = inner.clone();
        AppContext::register_pointer_callback(move |events| {
            for event in events {
                let surface_id = event.surface.id();
                let is_ours = cb_inner.borrow().surface_id.as_ref() == Some(&surface_id);
                if !is_ours {
                    // The compositor's popup grab is "owner-events": a press
                    // on another surface of our own client is delivered to us
                    // rather than dismissing the popup, so an outside click
                    // is ours to act on. Only armed here — the well that owns
                    // this popup must get the same batch first, so that
                    // clicking it toggles the popup shut instead of closing
                    // and immediately reopening it.
                    let mut state = cb_inner.borrow_mut();
                    if state.popup.is_some() && matches!(event.kind, PointerEventKind::Press { .. })
                    {
                        state.dismiss_pending = true;
                    }
                    continue;
                }
                let (x, y) = (event.position.0 as f32, event.position.1 as f32);
                match event.kind {
                    PointerEventKind::Press { button: 0x110, .. } => {
                        Self::handle_press(&cb_inner, x, y);
                    }
                    PointerEventKind::Motion { .. } => {
                        Self::handle_motion(&cb_inner, x, y);
                    }
                    PointerEventKind::Release { button: 0x110, .. } => {
                        Self::handle_release(&cb_inner);
                    }
                    _ => {}
                }
            }
        });

        let end_inner = inner.clone();
        AppContext::register_pointer_batch_end_callback(move || {
            let pending = {
                let mut state = end_inner.borrow_mut();
                std::mem::replace(&mut state.dismiss_pending, false) && state.popup.is_some()
            };
            if pending {
                Self::dismiss(&end_inner);
            }
        });

        Self { inner }
    }

    /// Whether the popup is currently up.
    pub fn is_open(&self) -> bool {
        self.inner.borrow().popup.is_some()
    }

    /// The mode last shown — useful for a caller that wants to remember it
    /// across opens (e.g. reopen straight into HSV after the user picked
    /// there last).
    pub fn mode(&self) -> Mode {
        self.inner.borrow().mode
    }

    /// Dismiss the popup if it is open. Safe to call unconditionally.
    pub fn close(&self) {
        let mut state = self.inner.borrow_mut();
        if let Some(mut popup) = state.popup.take() {
            popup.destroy();
        }
        state.surface_id = None;
        state.drag = Drag::None;
        state.dismiss_pending = false;
    }

    /// Open the popup, anchored to `well_rect` — the same rect passed to
    /// [`super::well::draw`] — parented to `parent_xdg`, starting from
    /// `initial_color`.
    ///
    /// `on_change` fires with every colour picked (a swatch click, or any
    /// HSV drag update) — including intermediate values while dragging, the
    /// same way a native picker previews live rather than only on release.
    /// `on_close` fires when the popup dismisses without the caller having
    /// closed it itself (ESC, or a click outside) — use it to
    /// `request_frame()` so the well's "open" look clears promptly.
    #[allow(clippy::too_many_arguments)]
    pub fn open<C, D>(
        &self,
        parent_xdg: &xdg_surface::XdgSurface,
        well_rect: Rect,
        serial: u32,
        initial_color: Color,
        theme: Theme,
        on_change: C,
        on_close: D,
    ) where
        C: Fn(Color) + 'static,
        D: Fn() + 'static,
    {
        {
            let state = self.inner.borrow();
            if state.popup.is_some() {
                return;
            }
        }

        let swatch_count = self.inner.borrow().swatches.len();
        let (w, h) = panel::panel_size(swatch_count);

        let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state()) else {
            return;
        };
        positioner.set_size(w as i32, h as i32);
        positioner.set_anchor_rect(
            well_rect.left as i32,
            well_rect.top as i32,
            well_rect.width().max(1.0) as i32,
            well_rect.height().max(1.0) as i32,
        );
        positioner.set_anchor(xdg_positioner::Anchor::BottomLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        positioner.set_offset(0, 4);
        positioner.set_constraint_adjustment(
            xdg_positioner::ConstraintAdjustment::SlideX
                | xdg_positioner::ConstraintAdjustment::SlideY
                | xdg_positioner::ConstraintAdjustment::FlipX
                | xdg_positioner::ConstraintAdjustment::FlipY,
        );

        let Ok(popup) =
            PopupSurface::new_with_grab(parent_xdg, &positioner, w as i32, h as i32, Some(serial))
        else {
            return;
        };
        let surface_id = popup.wl_surface().id();

        {
            let mut state = self.inner.borrow_mut();
            state.color = initial_color;
            state.theme = theme;
            state.selected_swatch = state.swatches.iter().position(|s| s.color == initial_color);
            state.selected_field = None;
            state.drag = Drag::None;
            state.surface_id = Some(surface_id.clone());
            state.dismiss_pending = false;
            state.on_change = Some(Rc::new(on_change));
            state.on_close = Some(Rc::new(on_close));
            state.popup = Some(popup);
        }

        let done_inner = self.inner.clone();
        AppContext::register_popup_done_callback(surface_id.clone(), move || {
            Self::dismiss(&done_inner);
        });

        let configure_inner = self.inner.clone();
        AppContext::register_popup_configure_callback(surface_id, move |_serial| {
            {
                let mut state = configure_inner.borrow_mut();
                let theme = state.theme.clone();
                if let Some(popup) = state.popup.as_mut() {
                    Self::apply_surface_effects(popup, &theme);
                    popup.mark_configured();
                }
            }
            Self::render(&configure_inner);
        });
    }

    /// Round, shadow and blur the popup's own surface, so the panel reads as
    /// the same material as a menu rather than as a flat card: `panel::draw`
    /// paints a translucent background, and without a blur behind it that
    /// translucency just shows whatever happens to be underneath.
    ///
    /// The radius has to agree with the one `panel::draw` rounds its
    /// background to, or the compositor's clip cuts a differently rounded
    /// corner off the painted one. Shadow geometry is in physical pixels, so
    /// it scales with the output — the values match `ContextMenu`'s, so a
    /// picker and a menu cast the same shadow.
    fn apply_surface_effects(popup: &PopupSurface, theme: &Theme) {
        let Some(surface_style) = popup.base_surface().surface_style() else {
            return;
        };
        let scale = AppContext::fractional_scale();
        let shadow = theme.shadow;
        surface_style.set_corner_radius(panel::CORNER_RADIUS as f64);
        surface_style.set_masks_to_bounds(ClipMode::Enabled);
        surface_style.set_shadow(
            shadow.a() as f64 / 255.0,
            16.0 * scale,
            0.0,
            4.0 * scale,
            shadow.r() as f64 / 255.0,
            shadow.g() as f64 / 255.0,
            shadow.b() as f64 / 255.0,
        );
        surface_style.set_blend_mode(BlendMode::BackgroundBlur);
    }

    fn handle_press(inner: &Rc<RefCell<PopupInner>>, x: f32, y: f32) {
        let rect = Self::panel_rect(inner);
        let Some(rect) = rect else { return };

        if let Some(mode) = panel::mode_at(rect, x, y) {
            inner.borrow_mut().mode = mode;
            Self::render(inner);
            return;
        }

        let mode = inner.borrow().mode;
        match mode {
            Mode::Swatches => {
                let count = inner.borrow().swatches.len();
                if let Some(index) = panel::swatch_at(rect, count, x, y) {
                    let color = {
                        let mut state = inner.borrow_mut();
                        state.selected_swatch = Some(index);
                        let color = state.swatches[index].color;
                        state.color = color;
                        color
                    };
                    Self::notify_change(inner, color);
                    // A preset is a discrete choice: commit and dismiss,
                    // rather than leaving the popup up with nothing left to
                    // do. Dragging in HSV keeps it open, because there the
                    // pointer is still mid-gesture.
                    Self::dismiss(inner);
                }
            }
            Mode::Hsv => {
                let square = panel::hsv_square_rect(rect);
                let strip = panel::hsv_hue_rect(rect);
                if x >= square.left && x <= square.right && y >= square.top && y <= square.bottom {
                    inner.borrow_mut().drag = Drag::Square;
                    let color = Self::apply_sv(inner, square, x, y);
                    Self::notify_change(inner, color);
                    Self::render(inner);
                } else if x >= strip.left && x <= strip.right && y >= strip.top && y <= strip.bottom
                {
                    inner.borrow_mut().drag = Drag::Hue;
                    let color = Self::apply_hue(inner, strip, y);
                    Self::notify_change(inner, color);
                    Self::render(inner);
                }
            }
            Mode::Hex => {
                if let Some(field) = panel::hex_field_at(rect, x, y) {
                    inner.borrow_mut().selected_field = Some(field);
                    Self::render(inner);
                }
            }
        }
    }

    fn handle_motion(inner: &Rc<RefCell<PopupInner>>, x: f32, y: f32) {
        let drag = inner.borrow().drag;
        if drag == Drag::None {
            return;
        }
        let Some(rect) = Self::panel_rect(inner) else {
            return;
        };
        let color = match drag {
            Drag::Square => Self::apply_sv(inner, panel::hsv_square_rect(rect), x, y),
            Drag::Hue => Self::apply_hue(inner, panel::hsv_hue_rect(rect), y),
            Drag::None => return,
        };
        Self::notify_change(inner, color);
        Self::render(inner);
    }

    fn handle_release(inner: &Rc<RefCell<PopupInner>>) {
        inner.borrow_mut().drag = Drag::None;
    }

    /// Update the colour's saturation/value from a point in the SV square,
    /// keeping the hue the current colour already has.
    fn apply_sv(inner: &Rc<RefCell<PopupInner>>, square: Rect, x: f32, y: f32) -> Color {
        let (s, v) = panel::sv_at(square, x, y);
        let mut state = inner.borrow_mut();
        let (h, _, _) = rgb_to_hsv(state.color);
        let color = super::hsv::hsv_to_rgb(h, s, v);
        state.color = color;
        state.selected_swatch = None;
        color
    }

    /// Update the colour's hue from a point in the hue strip, keeping the
    /// current saturation/value.
    fn apply_hue(inner: &Rc<RefCell<PopupInner>>, strip: Rect, y: f32) -> Color {
        let h = panel::hue_at(strip, y);
        let mut state = inner.borrow_mut();
        let (_, s, v) = rgb_to_hsv(state.color);
        let color = super::hsv::hsv_to_rgb(h, s, v);
        state.color = color;
        state.selected_swatch = None;
        color
    }

    /// Tear the popup down and tell the caller it went away.
    ///
    /// Shared by the compositor's `popup_done` (ESC, click outside) and by a
    /// swatch commit, so a picker can only ever close one way.
    fn dismiss(inner: &Rc<RefCell<PopupInner>>) {
        let on_close = inner.borrow().on_close.clone();
        {
            let mut state = inner.borrow_mut();
            if state.popup.is_none() {
                return;
            }
            if let Some(mut popup) = state.popup.take() {
                popup.destroy();
            }
            state.surface_id = None;
            state.drag = Drag::None;
            state.dismiss_pending = false;
        }
        if let Some(cb) = on_close {
            cb();
        }
    }

    fn notify_change(inner: &Rc<RefCell<PopupInner>>, color: Color) {
        let cb = inner.borrow().on_change.clone();
        if let Some(cb) = cb {
            cb(color);
        }
    }

    fn panel_rect(inner: &Rc<RefCell<PopupInner>>) -> Option<Rect> {
        let state = inner.borrow();
        let popup = state.popup.as_ref()?;
        let (w, h) = popup.dimensions();
        Some(Rect::from_xywh(0.0, 0.0, w as f32, h as f32))
    }

    fn render(inner: &Rc<RefCell<PopupInner>>) {
        let state = inner.borrow();
        let Some(popup) = state.popup.as_ref() else {
            return;
        };
        let (w, h) = popup.dimensions();
        let rect = Rect::from_xywh(0.0, 0.0, w as f32, h as f32);
        let mode = state.mode;
        let color = state.color;
        let selected_swatch = state.selected_swatch;
        let selected_field = state.selected_field;
        let swatches = &state.swatches;
        let theme = &state.theme;
        popup.draw(|canvas| {
            panel::draw(
                canvas,
                rect,
                mode,
                color,
                swatches,
                selected_swatch,
                selected_field,
                theme,
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_popup_is_closed() {
        // `ColorPickerPopup::new` touches `AppContext`'s thread-local pointer
        // callback list, which is safe outside a live Wayland connection —
        // it only pushes a closure, the same as `DropdownMenu::new` does.
        let popup = ColorPickerPopup::new(vec![Swatch::new("Blue", Color::from_rgb(10, 20, 255))]);
        assert!(!popup.is_open());
        assert_eq!(popup.mode(), Mode::Swatches);
    }

    // `open()`/`close()` beyond this reach `PopupSurface`, which reads
    // `AppContext`'s live Wayland globals — exercised instead by the
    // `color_picker_demo` example against a running Otto session, the same
    // split `dropdown::menu`'s tests use.
}
