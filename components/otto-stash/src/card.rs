//! Where the card lives: in Otto's side canvas when the compositor offers
//! `otto-canvas-v1` at version 3 or later, and otherwise as a floating
//! balloon on an overlay that can be dragged anywhere.

// Rust guideline compliant 2026-02-21

use wayland_client::protocol::wl_surface::WlSurface;

use crate::canvas::CanvasCard;
use crate::panel::Panel;

/// The card on screen, from the first add until it is sent or cancelled.
#[derive(Debug)]
pub enum Card {
    /// A balloon floating over the desktop.
    Floating(Box<Panel>),
    /// An item in the side canvas.
    Canvas(Box<CanvasCard>),
}

impl Card {
    /// The surface the card is drawn on.
    pub fn card(&self) -> &WlSurface {
        match self {
            Self::Floating(panel) => panel.card(),
            Self::Canvas(card) => card.card(),
        }
    }

    /// Whether `surface` is the one that takes the card's pointer, and so
    /// the one drops arrive on.
    pub fn takes_pointer(&self, surface: &WlSurface) -> bool {
        match self {
            Self::Floating(panel) => panel.takes_pointer(surface),
            Self::Canvas(card) => card.card() == surface,
        }
    }

    /// Whether the card can be drawn: the compositor has sized it.
    pub fn configured(&self) -> bool {
        match self {
            Self::Floating(panel) => panel.configured(),
            Self::Canvas(card) => card.configured(),
        }
    }

    /// Whether the compositor draws the card's material.
    pub fn frosted(&self) -> bool {
        match self {
            Self::Floating(panel) => panel.frosted(),
            Self::Canvas(card) => card.frosted(),
        }
    }

    /// The width to lay the card out at and the tallest it may be, in
    /// logical pixels.
    pub fn bounds(&self) -> (f32, f32) {
        match self {
            Self::Floating(panel) => (crate::balloon::WIDTH, panel.max_card_height()),
            Self::Canvas(card) => (card.width(), card.max_height()),
        }
    }

    /// The card was laid out `height` logical pixels tall with room for
    /// everything: in the canvas, the column shares its height by it.
    pub fn set_content_height(&mut self, height: f32) {
        if let Self::Canvas(card) = self {
            card.set_content_height(height);
        }
    }

    /// How tall the card's buffer is drawn, in logical pixels.
    pub fn buffer_height(&self, card_height: f32) -> f32 {
        match self {
            Self::Floating(panel) => panel.buffer_height(card_height),
            // The column is laid out by the buffer's height.
            Self::Canvas(_) => card_height,
        }
    }

    /// The card is about to be drawn at `size`; `follow` says the size is
    /// moving frame by frame.
    pub fn set_card_size(&mut self, size: (i32, i32), follow: bool) {
        match self {
            Self::Floating(panel) => panel.set_card_size(size, follow),
            Self::Canvas(card) => card.set_card_size(size),
        }
    }

    /// Whether frame callbacks come back, so animations can run on them.
    /// The canvas sends none while it is hidden.
    pub fn animates(&self) -> bool {
        match self {
            Self::Floating(_) => true,
            Self::Canvas(card) => card.shown(),
        }
    }

    /// Bring the card into view: the side canvas is asked to show itself.
    /// A floating card is always in view.
    pub fn reveal(&self) {
        if let Self::Canvas(card) = self {
            card.show();
        }
    }

    /// Track the pointer. Returns whether the card moved, which only a
    /// floating card being dragged does.
    pub fn pointer_moved(&mut self, x: f64, y: f64) -> bool {
        match self {
            Self::Floating(panel) => panel.pointer_moved(x, y),
            Self::Canvas(card) => {
                card.pointer_moved(x, y);
                false
            }
        }
    }

    /// The pointer left the card's surface.
    pub fn pointer_left(&mut self) {
        match self {
            Self::Floating(panel) => panel.pointer_released(),
            Self::Canvas(card) => card.pointer_left(),
        }
    }

    /// Where the pointer is on the card, if it is over it.
    pub fn pointer_on_card(&self) -> Option<(f64, f64)> {
        match self {
            Self::Floating(panel) => panel.pointer_on_card(),
            Self::Canvas(card) => card.pointer_on_card(),
        }
    }

    /// Where the card's top-left corner is, to tell a click from a drag.
    pub fn card_origin(&self) -> Option<(i32, i32)> {
        match self {
            Self::Floating(panel) => panel.card_origin(),
            Self::Canvas(_) => Some((0, 0)),
        }
    }

    /// A left press away from the buttons: a floating card is taken hold
    /// of, to be dragged.
    pub fn pointer_pressed(&mut self) {
        if let Self::Floating(panel) = self {
            panel.pointer_pressed();
        }
    }

    pub fn pointer_released(&mut self) {
        if let Self::Floating(panel) = self {
            panel.pointer_released();
        }
    }

    /// Fade the card away. Returns whether it fades; the caller destroys a
    /// card that doesn't at once. A card in the canvas leaves at once.
    pub fn fade_out(&self) -> bool {
        match self {
            Self::Floating(panel) => panel.fade_out(),
            Self::Canvas(_) => false,
        }
    }

    pub fn destroy(self) {
        match self {
            Self::Floating(panel) => panel.destroy(),
            Self::Canvas(card) => card.destroy(),
        }
    }
}
