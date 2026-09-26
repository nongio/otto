//! A file drag passing over the dock, and the Trash icon it can be dropped on.
//!
//! The compositor is the drop target here, not a client: the drag reaches the
//! dock through [`crate::focus`], which asks these methods what is under the
//! pointer and lets the dock answer the way it answers a hover.

use layers::skia::{self, Contains};
use smithay::utils::{Logical, Point};

use super::view::trash_match_id;
use super::DockView;
use crate::config::Config;

impl DockView {
    /// A drag came onto the dock: wake it the way the pointer does.
    pub fn file_drag_enter(&self) {
        self.show_autohide();
        self.magnify_elements_animated();
    }

    /// The drag moved to `location`, global and logical, which is `local_px`
    /// in the physical pixels of the dock's own output.
    ///
    /// Returns whether it is over the Trash, which lights up while it is.
    pub fn file_drag_motion(&self, location: Point<f64, Logical>, local_px: (f32, f32)) -> bool {
        let scale = Config::with(|c| c.screen_scale);
        let along = if self.position().is_vertical() {
            location.y
        } else {
            location.x
        };
        self.update_magnification_position((along * scale) as f32);

        let over_trash = self
            .app_icon_bounds(&trash_match_id())
            .is_some_and(|bounds| bounds.contains(skia::Point::new(local_px.0, local_px.1)));
        self.highlight_trash(over_trash);
        over_trash
    }

    /// The drag left the dock, or was dropped on it.
    pub fn file_drag_leave(&self) {
        self.highlight_trash(false);
        if !self.is_icon_dragging() {
            self.demagnify_elements();
        }
    }

    /// Darken the Trash and show its label, as a press on it does.
    fn highlight_trash(&self, on: bool) {
        let (icon, label) = {
            let layers = self.app_layers.read().unwrap();
            let Some(entry) = layers.get(&trash_match_id()) else {
                return;
            };
            (entry.icon_scaler.clone(), entry.label_layer.clone())
        };
        let lit = self.is_pressed(&icon);
        if on && !lit {
            self.darken_pressed(&icon);
            self.set_active_label(Some(label));
        } else if !on && lit {
            self.clear_pressed();
            self.set_active_label(None);
        }
    }
}
