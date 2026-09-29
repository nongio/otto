//! `otto-canvas-v1` items: surfaces placed in Otto's side canvas.
//!
//! The side canvas is a column that slides in from the right edge of the
//! screen. The compositor owns the column and decides the width of every
//! item in it; an item decides only its own height, through the size of the
//! buffer it draws. Items stack in the order they were created, unless they
//! say where they sit with [`CanvasItemSurface::set_order`].
//!
//! The compositor also says when the canvas is on screen
//! ([`CanvasItemEvent::Shown`]) and when it is not
//! ([`CanvasItemEvent::Hidden`]). A hidden item gets no frame callbacks, so
//! anything animated should wait for the next `Shown`.
//!
//! An item gets the keyboard when it is pressed on, or as the canvas is
//! shown when it asks for that with
//! [`CanvasItemSurface::set_keyboard_interactivity`]. While it has the
//! keyboard, Escape is the item's to handle: the compositor no longer hides
//! the canvas on it, so an item that has no other use for Escape should call
//! [`CanvasItemSurface::dismiss`].
//!
//! An item can ask for the canvas to be shown with
//! [`CanvasItemSurface::show`], when it has something new to see. That
//! show leaves the keyboard with the app that has it.
//!
//! A drag that rests at the right edge of the screen opens the canvas too,
//! so an item can take the drop through the data device like any other
//! surface. Apps hear of every drag through
//! [`App::on_canvas_drag_started`](crate::app_runner::App::on_canvas_drag_started)
//! and [`App::on_canvas_drag_ended`](crate::app_runner::App::on_canvas_drag_ended),
//! so one with nothing in the canvas can add an item for the drop.

// Rust guideline compliant 2026-02-21

use std::cell::RefCell;
use std::rc::Rc;

use wayland_client::Proxy;

use super::common::{BaseWaylandSurface, SurfaceError};
use crate::app_runner::AppContext;
use crate::protocols::otto_canvas_item_v1::OttoCanvasItemV1;

pub use crate::protocols::otto_canvas_item_v1::KeyboardInteractivity as CanvasKeyboardInteractivity;

/// The protocol version that brought `set_keyboard_interactivity`.
const KEYBOARD_INTERACTIVITY_SINCE: u32 = 2;
/// The protocol version that brought `show`, `set_order` and
/// [`CanvasKeyboardInteractivity::Never`].
const SHOW_SINCE: u32 = 3;

/// Something the compositor told a canvas item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasItemEvent {
    /// Draw at `width` logical points. Already acknowledged by the time the
    /// item's handler sees it, and the canvas to draw on already has the new
    /// width.
    Configure { serial: u32, width: i32 },
    /// The canvas is on screen, or on its way in.
    Shown,
    /// The canvas is off screen.
    Hidden,
}

type Handler = Box<dyn FnMut(&CanvasItemSurface, CanvasItemEvent)>;

struct Inner {
    base_surface: BaseWaylandSurface,
    item: OttoCanvasItemV1,
    configured: bool,
    shown: bool,
}

/// A surface in the side canvas, with a Skia canvas to draw it into.
///
/// Created with the height the item wants; the width comes from the
/// compositor. Nothing can be drawn until the first configure, which
/// [`CanvasItemSurface::on_event`] reports.
///
/// # Examples
///
/// ```no_run
/// use otto_kit::surfaces::{CanvasItemEvent, CanvasItemSurface};
///
/// let item = CanvasItemSurface::new(200).expect("the compositor has a side canvas");
/// item.on_event(|item, event| {
///     if let CanvasItemEvent::Configure { .. } = event {
///         item.draw(|canvas| {
///             canvas.clear(otto_kit::skia::Color::TRANSPARENT);
///         });
///     }
/// });
/// ```
#[derive(Clone)]
pub struct CanvasItemSurface {
    inner: Rc<RefCell<Inner>>,
    handler: Rc<RefCell<Option<Handler>>>,
}

impl CanvasItemSurface {
    /// Place a new surface at the bottom of the side canvas, `height`
    /// logical points tall.
    ///
    /// # Errors
    ///
    /// Fails when the compositor does not offer `otto_canvas_manager_v1`.
    pub fn new(height: i32) -> Result<Self, SurfaceError> {
        let manager = AppContext::otto_canvas_manager().ok_or_else(|| {
            SurfaceError::WaylandError("otto-canvas-v1 not available".to_string())
        })?;
        let compositor = AppContext::compositor_state();
        let qh = AppContext::queue_handle();

        let wl_surface = compositor.create_surface(qh);
        // The 2x buffer the rest of otto-kit draws at, laid out in points.
        let buffer_scale = 2;
        wl_surface.set_buffer_scale(buffer_scale);
        let item = manager.get_canvas_item(&wl_surface, qh, ());
        // Width 0 until the compositor says; the height is ours.
        let base_surface = BaseWaylandSurface::new(wl_surface, 0, height, buffer_scale);

        let surface = Self {
            inner: Rc::new(RefCell::new(Inner {
                base_surface,
                item: item.clone(),
                configured: false,
                shown: false,
            })),
            handler: Rc::new(RefCell::new(None)),
        };

        let this = surface.clone();
        AppContext::register_canvas_item_callback(item.id(), move |event| {
            this.handle(event);
        });
        Ok(surface)
    }

    /// Call `handler` with every event, after the item has dealt with it.
    /// Replaces any handler set before.
    pub fn on_event<F>(&self, handler: F)
    where
        F: FnMut(&CanvasItemSurface, CanvasItemEvent) + 'static,
    {
        *self.handler.borrow_mut() = Some(Box::new(handler));
    }

    /// Whether a configure has arrived and there is a canvas to draw on.
    pub fn is_configured(&self) -> bool {
        self.inner.borrow().configured
    }

    /// Whether the side canvas is on screen.
    pub fn is_shown(&self) -> bool {
        self.inner.borrow().shown
    }

    /// Whether a frame drawn on the item has yet to reach the screen.
    ///
    /// Something that redraws continuously, a scroll gliding to a stop, should
    /// draw only when this is false: it then runs at the compositor's pace,
    /// and stops altogether while the canvas is hidden, which sends no frame
    /// callbacks.
    pub fn frame_in_flight(&self) -> bool {
        self.inner.borrow().base_surface.frame_in_flight()
    }

    /// Width and height in logical points.
    pub fn dimensions(&self) -> (i32, i32) {
        self.inner.borrow().base_surface.dimensions()
    }

    /// The surface's otto-surface-style object, for corner radius and
    /// material.
    pub fn surface_style(
        &self,
    ) -> Option<crate::protocols::otto_surface_style_v1::OttoSurfaceStyleV1> {
        self.inner.borrow().base_surface.surface_style().cloned()
    }

    /// Change the item's height. The column makes room from the next draw.
    pub fn set_height(&self, height: i32) {
        let mut inner = self.inner.borrow_mut();
        let (width, _) = inner.base_surface.dimensions();
        if inner.configured {
            inner.base_surface.resize(width, height);
        } else {
            inner.base_surface.height = height;
        }
    }

    /// Draw the item. Does nothing before the first configure.
    pub fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        let base = self.inner.borrow().base_surface.clone();
        base.draw(draw_fn);
    }

    /// Say when the item takes the keyboard: only when pressed on
    /// ([`CanvasKeyboardInteractivity::None`], the default), as soon as the
    /// canvas is shown ([`CanvasKeyboardInteractivity::OnShow`]), or never
    /// ([`CanvasKeyboardInteractivity::Never`]), which keeps the keyboard
    /// with the app even when the item is clicked. `OnShow` applies from the
    /// next time the canvas is shown, `Never` from the next press.
    ///
    /// Returns false, and sends nothing, when the compositor has no such
    /// request (version 1), or no `Never` (version 2): the item then gets the
    /// keyboard when it is pressed on.
    pub fn set_keyboard_interactivity(&self, interactivity: CanvasKeyboardInteractivity) -> bool {
        let inner = self.inner.borrow();
        let since = match interactivity {
            CanvasKeyboardInteractivity::Never => SHOW_SINCE,
            _ => KEYBOARD_INTERACTIVITY_SINCE,
        };
        if inner.item.version() < since {
            return false;
        }
        inner.item.set_keyboard_interactivity(interactivity);
        true
    }

    /// Whether the item's surface has the keyboard.
    pub fn has_keyboard(&self) -> bool {
        let inner = self.inner.borrow();
        AppContext::keyboard_focus().as_ref() == Some(&inner.base_surface.wl_surface().id())
    }

    /// Ask the compositor to show the side canvas, for example because the
    /// item has something new on it. The keyboard stays with the app that
    /// has it; the canvas stays until the user hides it.
    ///
    /// Returns false, and sends nothing, when the compositor has no such
    /// request (before version 3).
    pub fn show(&self) -> bool {
        let inner = self.inner.borrow();
        if inner.item.version() < SHOW_SINCE {
            return false;
        }
        inner.item.show();
        true
    }

    /// Say where the item sits in the column: a lower `order` sits higher,
    /// and items with the same order sit in the order they were created.
    /// The default is 0.
    ///
    /// Returns false, and sends nothing, when the compositor has no such
    /// request (before version 3): items then stack in creation order.
    pub fn set_order(&self, order: i32) -> bool {
        let inner = self.inner.borrow();
        if inner.item.version() < SHOW_SINCE {
            return false;
        }
        inner.item.set_order(order);
        true
    }

    /// Ask the compositor to hide the side canvas.
    pub fn dismiss(&self) {
        self.inner.borrow().item.dismiss();
    }

    /// Take the item out of the canvas.
    pub fn destroy(&self) {
        let inner = self.inner.borrow();
        AppContext::unregister_canvas_item_callback(&inner.item.id());
        inner.item.destroy();
    }

    fn handle(&self, event: CanvasItemEvent) {
        {
            let mut inner = self.inner.borrow_mut();
            match event {
                CanvasItemEvent::Configure { serial, width } => {
                    inner.item.ack_configure(serial);
                    let (_, height) = inner.base_surface.dimensions();
                    if inner.configured {
                        inner.base_surface.resize(width, height);
                    } else {
                        inner.base_surface.width = width;
                        if let Err(err) = inner.base_surface.create_skia_surface() {
                            tracing::error!(?err, "could not create the canvas item's surface");
                            return;
                        }
                        if let Some(layer) = inner.base_surface.layer_node() {
                            layer.set_size(
                                layers::types::Size::points(width as f32, height as f32),
                                None,
                            );
                            layer.engine.update(0.0);
                        }
                        inner.configured = true;
                    }
                }
                CanvasItemEvent::Shown => inner.shown = true,
                CanvasItemEvent::Hidden => inner.shown = false,
            }
        }
        // Taken out while it runs, so it may draw, and even set a new handler.
        let taken = self.handler.borrow_mut().take();
        if let Some(mut handler) = taken {
            handler(self, event);
            let mut slot = self.handler.borrow_mut();
            if slot.is_none() {
                *slot = Some(handler);
            }
        }
    }
}
