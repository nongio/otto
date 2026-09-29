//! The card as an item in Otto's side canvas (`otto-canvas-v1`, version 3).
//!
//! The canvas decides the card's width and where it sits; the card sits at
//! the top of the column (a negative order) and is as tall as its content.
//! It asks the canvas to show itself when something is added, never takes
//! the keyboard, not even when clicked, and does no frame work while the
//! canvas is hidden. It does not move: the column is its place.

// Rust guideline compliant 2026-02-21

use wayland_client::protocol::{wl_compositor::WlCompositor, wl_surface::WlSurface};
use wayland_client::QueueHandle;

use otto_kit::protocols::{
    otto_canvas_item_v1::{KeyboardInteractivity, OttoCanvasItemV1},
    otto_canvas_manager_v1::OttoCanvasManagerV1,
    otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1,
    otto_surface_style_v1::OttoSurfaceStyleV1,
};

use crate::State;

/// The first `otto-canvas-v1` version with `show`, `set_order` and the
/// `never` keyboard interactivity, all of which the card needs.
pub const CANVAS_VERSION: u32 = 3;

/// Where the card sits in the column: above every item that keeps the
/// default order of 0, the Agents panel included.
const ORDER: i32 = -100;

/// The card in the side canvas, from the first add until the stash is sent
/// or cancelled, or Ask shows it instead.
#[derive(Debug)]
pub struct CanvasCard {
    item: OttoCanvasItemV1,
    card: WlSurface,
    style: Option<OttoSurfaceStyleV1>,
    /// The column's width in logical pixels; zero until configured.
    width: i32,
    /// Whether the canvas is on screen.
    shown: bool,
    /// The card's size in logical pixels, as last drawn.
    card_size: (i32, i32),
    /// Where the pointer is on the card, while it is over it.
    pointer: Option<(f64, f64)>,
}

impl CanvasCard {
    /// Add the card to the canvas. It shows once the compositor has sent its
    /// width and the card has been drawn.
    pub fn new(
        manager: &OttoCanvasManagerV1,
        compositor: &WlCompositor,
        styles: Option<&OttoSurfaceStyleManagerV1>,
        qh: &QueueHandle<State>,
        scale: i32,
    ) -> Self {
        let card = compositor.create_surface(qh, ());
        card.set_buffer_scale(scale);
        let item = manager.get_canvas_item(&card, qh, ());
        // The app keeps the keyboard, and with it its selection and caret.
        item.set_keyboard_interactivity(KeyboardInteractivity::Never);
        item.set_order(ORDER);
        let style = styles.map(|manager| {
            let style = manager.get_surface_style(&card, qh, ());
            crate::panel::apply_material(&style);
            style
        });
        Self {
            item,
            card,
            style,
            width: 0,
            shown: false,
            card_size: (0, 0),
            pointer: None,
        }
    }

    /// Whether `item` is this card's.
    pub fn is(&self, item: &OttoCanvasItemV1) -> bool {
        self.item == *item
    }

    /// The surface the card is drawn on, which also takes its pointer.
    pub fn card(&self) -> &WlSurface {
        &self.card
    }

    /// Acknowledge the column's width.
    pub fn configure(&mut self, serial: u32, width: u32) {
        self.item.ack_configure(serial);
        self.width = i32::try_from(width).unwrap_or(i32::MAX);
    }

    /// Whether the compositor has said how wide to draw.
    pub fn configured(&self) -> bool {
        self.width > 0
    }

    /// The width to draw at, in logical pixels.
    pub fn width(&self) -> f32 {
        self.width as f32
    }

    /// Whether the compositor draws the card's material.
    pub fn frosted(&self) -> bool {
        self.style.is_some()
    }

    /// Whether the canvas is on screen, so frames come back.
    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }

    /// Ask the compositor to show the canvas. The keyboard stays put.
    pub fn show(&self) {
        self.item.show();
    }

    /// Ask the compositor to hide the canvas.
    pub fn dismiss(&self) {
        self.item.dismiss();
    }

    /// The card is about to be drawn at `size`, in logical pixels.
    pub fn set_card_size(&mut self, size: (i32, i32)) {
        self.card_size = size;
    }

    /// Track the pointer over the card.
    pub fn pointer_moved(&mut self, x: f64, y: f64) {
        self.pointer = Some((x, y));
    }

    pub fn pointer_left(&mut self) {
        self.pointer = None;
    }

    /// Where the pointer is on the card, if it is over it.
    pub fn pointer_on_card(&self) -> Option<(f64, f64)> {
        let (x, y) = self.pointer?;
        let (w, h) = self.card_size;
        let inside = (0.0..f64::from(w)).contains(&x) && (0.0..f64::from(h)).contains(&y);
        inside.then_some((x, y))
    }

    /// Take the card out of the canvas. The canvas stays as it is.
    pub fn destroy(self) {
        if let Some(style) = self.style {
            style.destroy();
        }
        self.item.destroy();
        self.card.destroy();
    }
}
