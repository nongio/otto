//! Side canvas: a column of client surfaces that slides in over the desktop
//! from the right edge of the output.
//!
//! The column is the compositor's; what is in it belongs to clients, which
//! place surfaces there through `otto-canvas-v1` (see [`handlers`]). Items
//! stack top to bottom in creation order, each at the column's width, and
//! each as tall as the buffer its client attached. Whatever does not fit in
//! the usable height is clipped.
//!
//! The canvas is an overlay: it slides over the desktop and moves nothing
//! underneath. It lives in the output's overlay plane (`canvas_plane` in
//! [`crate::workspaces::OutputWorkspaces`]), above the dock and every
//! window, fullscreen ones included, and below the lock screen.
//!
//! It is driven by a trackpad swipe from the right edge (the input side calls
//! the `canvas_gesture_*` methods), by the `CanvasToggle` action, and by
//! clients asking to be dismissed. While it is off screen its items get no
//! frame callbacks and are told `hidden`, so they can stop drawing.
//!
//! See `specs/side-canvas.md` and `docs/developer/side-canvas.md`.

pub mod handlers;
pub mod protocol;

// Rust guideline compliant 2026-02-21

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use layers::prelude::{taffy, Layer, Spring, TimingFunction, Transition};
use layers::types::{Point, Size};
use smithay::desktop::utils::{send_frames_surface_tree, under_from_surface_tree};
use smithay::desktop::WindowSurfaceType;
use smithay::output::Output;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{IsAlive, Logical, Rectangle, SERIAL_COUNTER};
use smithay::wayland::compositor::with_states;
use smithay::wayland::fractional_scale::with_fractional_scale;

use crate::config::Config;
use crate::focus::{KeyboardFocusTarget, PointerFocusTarget};
use crate::state::{Backend, Otto};

pub use handlers::{CanvasGlobal, CanvasItemData, CANVAS_ITEM_ROLE};
pub use protocol::{OttoCanvasItemV1, OttoCanvasManagerV1};

/// How long the column takes to settle, in seconds, and how much it bounces.
///
/// The same spring a workspace swipe settles with, so the two gestures feel
/// alike under the fingers.
const SETTLE_DURATION: f32 = 0.5;
const SETTLE_BOUNCE: f32 = 0.05;

/// Finger speed, in logical points per second, past which a released swipe
/// goes the way it was moving regardless of how far it got.
///
/// Low enough that a short flick opens or closes the canvas, high enough
/// that fingers coming to rest before lifting do not count as a flick.
const FLICK_VELOCITY: f64 = 300.0;

/// One client surface in the column.
struct CanvasItem {
    resource: OttoCanvasItemV1,
    surface: WlSurface,
    /// The node this item hangs from, a child of the column. The canvas
    /// positions and sizes it; the client's own layer inside it is placed by
    /// the commit path, which owns everything it registers in
    /// `surface_layers`.
    slot: Layer,
    /// The client surface's layer, registered in `surface_layers`.
    layer: Layer,
}

/// Where the canvas is in its life on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Off screen. Items get no frame callbacks.
    Hidden,
    /// Following the fingers. `was_shown` is where the gesture started, which
    /// is where a cancelled gesture goes back to.
    Dragging { was_shown: bool },
    /// On screen, or sliding in.
    Shown,
    /// Sliding out. `generation` names the slide, so the end of an earlier
    /// one cannot finish a later one.
    Closing { generation: u64 },
}

/// The column's placement on one output, in output-local physical pixels.
#[derive(Debug, Clone, Copy)]
struct ColumnGeometry {
    scale: f64,
    /// Left edge at rest, fully shown.
    shown_x_px: f64,
    /// Left edge when hidden: the output's right edge, so nothing shows.
    hidden_x_px: f64,
    top_px: f64,
    width_px: f64,
    height_px: f64,
}

impl ColumnGeometry {
    /// The left edge for `progress` (0 hidden, 1 shown).
    fn x_for(&self, progress: f64) -> f64 {
        (self.hidden_x_px + (self.shown_x_px - self.hidden_x_px) * progress).round()
    }

    /// How far out the column is at left edge `x_px`, from 0 to 1.
    fn progress_at(&self, x_px: f64) -> f64 {
        let travel = self.hidden_x_px - self.shown_x_px;
        if travel <= 0.0 {
            return 1.0;
        }
        ((self.hidden_x_px - x_px) / travel).clamp(0.0, 1.0)
    }

    /// The distance, in physical pixels, between hidden and shown.
    fn travel_px(&self) -> f64 {
        (self.hidden_x_px - self.shown_x_px).max(1.0)
    }
}

/// What a pointer button meant to the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasButton {
    /// Not the canvas's business: handle it as usual.
    Pass,
    /// A press on a canvas item. It is delivered to the item, which now has
    /// the keyboard; the usual click-to-focus must not move it elsewhere.
    Item,
    /// Taken by the canvas (a click outside it, which hides it). Nothing
    /// else may see it.
    Consumed,
}

/// Compositor-side state of the side canvas.
pub struct CanvasState<B: Backend> {
    items: Vec<CanvasItem>,
    phase: Phase,
    /// The output the column is on while it is anywhere but hidden.
    output: Option<Output>,
    /// The column, created with the first item.
    column: Option<Layer>,
    /// How far out the column is, from 0 (hidden) to 1 (shown), while it
    /// follows the fingers.
    drag_progress: f64,
    /// Counter naming each closing slide; see [`Phase::Closing`].
    close_generation: u64,
    /// The last closing slide that reached its end.
    closed_generation: Arc<AtomicU64>,
    /// Keyboard focus from before an item took it, given back on hide.
    previous_focus: Option<KeyboardFocusTarget<B>>,
    /// Buttons whose press the canvas took, so their release is taken too.
    swallowed_buttons: Vec<u32>,
}

impl<B: Backend> Default for CanvasState<B> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            phase: Phase::Hidden,
            output: None,
            column: None,
            drag_progress: 0.0,
            close_generation: 0,
            closed_generation: Arc::new(AtomicU64::new(0)),
            previous_focus: None,
            swallowed_buttons: Vec::new(),
        }
    }
}

impl<B: Backend> std::fmt::Debug for CanvasState<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasState")
            .field("items", &self.items.len())
            .field("phase", &self.phase)
            .field("output", &self.output.as_ref().map(Output::name))
            .finish_non_exhaustive()
    }
}

impl<B: Backend> CanvasState<B> {
    /// Whether any part of the canvas may be on screen.
    pub fn on_screen(&self) -> bool {
        !matches!(self.phase, Phase::Hidden)
    }
}

impl<B: Backend> Otto<B> {
    /// Whether any client has placed a surface in the canvas.
    pub fn canvas_available(&self) -> bool {
        self.canvas.items.iter().any(|item| item.surface.alive())
    }

    /// Whether the canvas is shown or on its way in.
    pub fn canvas_is_shown(&self) -> bool {
        match self.canvas.phase {
            Phase::Shown => true,
            Phase::Dragging { was_shown } => was_shown,
            Phase::Hidden | Phase::Closing { .. } => false,
        }
    }

    /// Whether any part of the canvas may be on screen: shown, following
    /// the fingers, or sliding in or out.
    pub fn canvas_on_screen(&self) -> bool {
        self.canvas.on_screen()
    }

    /// Starts an interactive slide driven by the fingers.
    pub fn canvas_gesture_begin(&mut self) {
        self.canvas_finish_close_if_done();
        let was_shown = self.canvas_is_shown();
        let progress = match self.canvas.phase {
            Phase::Hidden => {
                if !self.canvas_available() || !self.canvas_bring_on_screen() {
                    return;
                }
                0.0
            }
            // Caught mid-slide: carry on from wherever the column is now.
            _ => match (self.canvas_geometry(), self.canvas.column.as_ref()) {
                (Some(geometry), Some(column)) => geometry.progress_at(column.position().x as f64),
                _ => return,
            },
        };
        self.canvas.drag_progress = progress;
        self.canvas.phase = Phase::Dragging { was_shown };
        self.canvas_place_column(progress, false);
    }

    /// Moves the canvas by `delta_points` logical points; positive reveals it.
    pub fn canvas_gesture_update(&mut self, delta_points: f64) {
        if !matches!(self.canvas.phase, Phase::Dragging { .. }) {
            return;
        }
        let Some(geometry) = self.canvas_geometry() else {
            return;
        };
        // The column follows the fingers one to one: points become physical
        // pixels at this output's scale, then a fraction of the travel.
        let delta_px = delta_points * geometry.scale;
        let progress =
            (self.canvas.drag_progress + delta_px / geometry.travel_px()).clamp(0.0, 1.0);
        self.canvas.drag_progress = progress;
        self.canvas_place_column(progress, false);
    }

    /// Ends the interactive slide and settles open or closed.
    /// `velocity_points_per_s` is positive towards revealing.
    pub fn canvas_gesture_end(&mut self, velocity_points_per_s: f64, cancelled: bool) {
        let Phase::Dragging { was_shown } = self.canvas.phase else {
            return;
        };
        let open = if cancelled {
            was_shown
        } else if velocity_points_per_s.abs() >= FLICK_VELOCITY {
            velocity_points_per_s > 0.0
        } else {
            self.canvas.drag_progress >= 0.5
        };
        if open {
            self.canvas_settle_open();
        } else {
            self.canvas_hide();
        }
    }

    /// Shows the canvas if hidden, hides it if shown.
    pub fn canvas_toggle(&mut self) {
        if self.canvas_is_shown() {
            self.canvas_hide();
        } else {
            self.canvas_show();
        }
    }

    /// Slide the canvas in on the output under the pointer. Does nothing when
    /// no client has put anything in it.
    pub fn canvas_show(&mut self) {
        self.canvas_finish_close_if_done();
        if matches!(self.canvas.phase, Phase::Shown) || !self.canvas_available() {
            return;
        }
        if matches!(self.canvas.phase, Phase::Hidden) {
            if !self.canvas_bring_on_screen() {
                return;
            }
            self.canvas_place_column(0.0, false);
        }
        self.canvas_settle_open();
    }

    /// Slide the canvas out, and give the keyboard back to whoever had it.
    pub fn canvas_hide(&mut self) {
        match self.canvas.phase {
            Phase::Hidden | Phase::Closing { .. } => return,
            Phase::Shown | Phase::Dragging { .. } => {}
        }
        self.canvas_restore_focus();
        self.canvas.close_generation += 1;
        let generation = self.canvas.close_generation;
        self.canvas.phase = Phase::Closing { generation };

        let (Some(geometry), Some(column)) = (self.canvas_geometry(), self.canvas.column.clone())
        else {
            self.canvas_finish_close();
            return;
        };
        let done = self.canvas.closed_generation.clone();
        column
            .set_position(
                Point {
                    x: geometry.x_for(0.0) as f32,
                    y: geometry.top_px as f32,
                },
                Some(settle_transition()),
            )
            .on_finish(
                move |_: &Layer, _| {
                    done.fetch_max(generation, Ordering::Relaxed);
                },
                true,
            );
        self.canvas_request_redraw();
    }

    /// Re-read `[canvas]` and lay the column out again. Every item is sent a
    /// configure with the new width.
    pub fn canvas_config_changed(&mut self) {
        let width = canvas_width_points();
        for item in &self.canvas.items {
            send_configure(&item.resource, width);
        }
        if self.canvas.on_screen() {
            let progress = match self.canvas.phase {
                Phase::Dragging { .. } => self.canvas.drag_progress,
                Phase::Closing { .. } => 0.0,
                _ => 1.0,
            };
            self.canvas_place_column(progress, false);
        }
        self.canvas_relayout();
        self.canvas_request_redraw();
    }

    /// A frame has been presented on `output`: send the canvas items theirs,
    /// and finish a slide out that has reached its end.
    pub fn canvas_frame_presented(&mut self, output: &Output) {
        self.canvas_finish_close_if_done();
        if !self.canvas.on_screen() || self.canvas.output.as_ref() != Some(output) {
            return;
        }
        let time = self.clock.now();
        for item in &self.canvas.items {
            if item.surface.alive() {
                send_frames_surface_tree(&item.surface, output, time, None, |_, _| {
                    Some(output.clone())
                });
            }
        }
    }

    /// Whether `point` (global logical coordinates) is over the column while
    /// the canvas is on screen, gaps between items included.
    pub fn canvas_contains_point(&self, point: smithay::utils::Point<f64, Logical>) -> bool {
        self.canvas.on_screen()
            && self
                .canvas_column_rect()
                .is_some_and(|rect| rect.contains(point))
    }

    /// The canvas item under `pos`, if the canvas is on screen there.
    pub fn canvas_surface_under(
        &self,
        pos: smithay::utils::Point<f64, Logical>,
    ) -> Option<(PointerFocusTarget<B>, smithay::utils::Point<f64, Logical>)> {
        if !self.canvas.on_screen() {
            return None;
        }
        let column = self.canvas_column_rect()?;
        if !column.contains(pos) {
            return None;
        }
        let output = self.canvas.output.as_ref()?;
        let origin = self.workspaces.output_geometry(output)?.loc.to_f64();
        let scale = output.current_scale().fractional_scale();
        self.canvas.items.iter().find_map(|item| {
            if !item.surface.alive() {
                return None;
            }
            let at = item.slot.render_position();
            let item_loc = smithay::utils::Point::<f64, Logical>::from((
                origin.x + at.x as f64 / scale,
                origin.y + at.y as f64 / scale,
            ));
            let (surface, surface_loc) = under_from_surface_tree(
                &item.surface,
                pos - item_loc,
                (0, 0),
                WindowSurfaceType::ALL,
            )?;
            Some((
                PointerFocusTarget::from(&surface),
                item_loc + surface_loc.to_f64(),
            ))
        })
    }

    /// Route a pointer button through the canvas before anything else sees
    /// it. A press outside a shown canvas hides it and is taken, as is its
    /// release; a press on an item gives that item the keyboard.
    pub fn canvas_pointer_button(&mut self, button: u32, pressed: bool) -> CanvasButton {
        if !pressed {
            if let Some(index) = self
                .canvas
                .swallowed_buttons
                .iter()
                .position(|b| *b == button)
            {
                self.canvas.swallowed_buttons.swap_remove(index);
                return CanvasButton::Consumed;
            }
            return CanvasButton::Pass;
        }
        if self.is_session_locked() || !matches!(self.canvas.phase, Phase::Shown) {
            return CanvasButton::Pass;
        }
        let location = self.pointer.current_location();
        let inside = self
            .canvas_column_rect()
            .is_some_and(|rect| rect.contains(location));
        if !inside {
            self.canvas.swallowed_buttons.push(button);
            self.canvas_hide();
            return CanvasButton::Consumed;
        }
        match self.canvas_item_root_under(location) {
            Some(surface) => {
                self.canvas_focus_item(surface);
                CanvasButton::Item
            }
            // The gap between two items, or the space below the last one:
            // part of the canvas, so nothing underneath may have it.
            None => {
                self.canvas.swallowed_buttons.push(button);
                CanvasButton::Consumed
            }
        }
    }

    /// Whether `surface` is a canvas item's surface or one of its
    /// subsurfaces, and so belongs to the canvas.
    pub fn canvas_item_root_for(&self, surface: &WlSurface) -> Option<WlSurface> {
        // Every commit asks, and almost always with no canvas items at all.
        if self.canvas.items.is_empty() {
            return None;
        }
        let mut root = surface.clone();
        while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
            root = parent;
        }
        self.canvas
            .items
            .iter()
            .find(|item| item.surface == root)
            .map(|item| item.surface.clone())
    }

    /// Mirror a committed canvas item into the scene and lay the column out
    /// again, since its height may have changed.
    pub fn canvas_item_committed(&mut self, root: &WlSurface) {
        let Some(item) = self.canvas.items.iter().find(|item| &item.surface == root) else {
            return;
        };
        let layer = item.layer.clone();
        let scale = self.canvas_scale();
        self.sync_surface_tree_layers(root, scale, "canvas_item");
        if item_size_points(root).is_some() {
            layer.set_hidden(false);
        }
        self.canvas_relayout();
        // Off screen the scene changed but nothing that shows did.
        if self.canvas.on_screen() {
            self.canvas_request_redraw();
        }
    }

    /// A client asked for `surface` to become a canvas item.
    pub(crate) fn canvas_item_created(&mut self, resource: OttoCanvasItemV1, surface: WlSurface) {
        let column = self.canvas_column();

        let slot = self.layers_engine.new_layer();
        slot.set_key("canvas_slot");
        slot.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        slot.set_pointer_events(false);
        let _ = column.add_sublayer(&slot);

        let layer = self.layers_engine.new_layer();
        layer.set_key("canvas_item");
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        layer.set_hidden(true);
        let _ = slot.add_sublayer(&layer);
        self.surface_layers.insert(surface.id(), layer.clone());

        send_configure(&resource, canvas_width_points());
        if self.canvas.on_screen() {
            resource.shown();
        } else {
            resource.hidden();
        }
        self.canvas_prefer_scale(&surface);

        self.canvas.items.push(CanvasItem {
            resource,
            surface,
            slot,
            layer,
        });
        tracing::debug!(items = self.canvas.items.len(), "canvas item added");
    }

    /// A client acknowledged a configure. Nothing waits on it: the next
    /// buffer is laid out at whatever size it is.
    pub(crate) fn canvas_item_acked(&mut self, _item: &OttoCanvasItemV1, _serial: u32) {}

    /// A canvas item is gone, by request or with its client.
    pub(crate) fn canvas_item_destroyed(&mut self, id: &ObjectId) {
        let Some(index) = self
            .canvas
            .items
            .iter()
            .position(|item| &item.resource.id() == id)
        else {
            return;
        };
        let item = self.canvas.items.remove(index);
        self.surface_layers.remove(&item.surface.id());
        item.slot.remove();

        let focused_here = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .is_some_and(
                |focus| matches!(focus, KeyboardFocusTarget::CanvasItem(s) if s == item.surface),
            );
        if focused_here {
            self.canvas_restore_focus();
        }

        tracing::debug!(items = self.canvas.items.len(), "canvas item removed");
        if !self.canvas_available() {
            // An empty column has nothing to show; take it down at once
            // rather than slide an empty frame away.
            self.canvas_finish_close();
        } else {
            self.canvas_relayout();
        }
        self.canvas_request_redraw();
    }

    // ── Internals ────────────────────────────────────────────────────────

    /// The column layer, created on first use.
    fn canvas_column(&mut self) -> Layer {
        if let Some(column) = &self.canvas.column {
            return column.clone();
        }
        let column = self.layers_engine.new_layer();
        column.set_key("canvas_column");
        column.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        column.set_pointer_events(false);
        // Overflow is clipped: items taller than the space left are cut at
        // the bottom of the usable area.
        column.set_clip_children(true, None);
        column.set_clip_content(true, None);
        self.canvas.column = Some(column.clone());
        column
    }

    /// Put the column on the output under the pointer and tell the items
    /// they are about to be seen. Returns false when there is nowhere to
    /// show it.
    fn canvas_bring_on_screen(&mut self) -> bool {
        let pointer = self.pointer.current_location();
        let output = self
            .workspaces
            .output_under(pointer)
            .next()
            .or_else(|| self.workspaces.primary_output())
            .cloned();
        let Some(output) = output else {
            return false;
        };
        let column = self.canvas_column();
        for ows in self.workspaces.output_workspaces.values() {
            ows.canvas_plane.set_hidden(true);
        }
        let Some(ows) = self.workspaces.output_workspaces.get(&output.name()) else {
            return false;
        };
        let _ = ows.canvas_plane.add_sublayer(&column);
        ows.canvas_plane.set_hidden(false);
        self.canvas.output = Some(output);

        // The output may have a different scale from the one the items were
        // last mirrored at.
        let surfaces: Vec<WlSurface> = self
            .canvas
            .items
            .iter()
            .map(|i| i.surface.clone())
            .collect();
        let scale = self.canvas_scale();
        for surface in &surfaces {
            self.canvas_prefer_scale(surface);
            if surface.alive() {
                self.sync_surface_tree_layers(surface, scale, "canvas_item");
            }
        }
        self.canvas_relayout();
        for item in &self.canvas.items {
            item.resource.shown();
        }
        tracing::debug!(output = %self.canvas.output.as_ref().map(Output::name).unwrap_or_default(), "canvas on screen");
        true
    }

    /// Slide the column to fully shown.
    fn canvas_settle_open(&mut self) {
        self.canvas.phase = Phase::Shown;
        self.canvas_place_column(1.0, true);
        self.canvas_request_redraw();
    }

    /// Move the column to `progress` of the way out, animated or at once.
    fn canvas_place_column(&mut self, progress: f64, animated: bool) {
        let (Some(geometry), Some(column)) = (self.canvas_geometry(), self.canvas.column.clone())
        else {
            return;
        };
        column.set_size(
            Size::points(geometry.width_px as f32, geometry.height_px as f32),
            None,
        );
        let target = Point {
            x: geometry.x_for(progress) as f32,
            y: geometry.top_px as f32,
        };
        column.set_position(target, animated.then(settle_transition));
        self.canvas_request_redraw();
    }

    /// Stack the items top to bottom, each at the column's width and at its
    /// buffer's height, a gap apart.
    fn canvas_relayout(&mut self) {
        self.canvas.items.retain(|item| item.resource.is_alive());
        let Some(geometry) = self.canvas_geometry() else {
            return;
        };
        let gap_px = (f64::from(Config::with(|c| c.canvas.gap)) * geometry.scale).round();
        let mut y_px = 0.0;
        for item in &self.canvas.items {
            let height_px = item_size_points(&item.surface)
                .map(|(_, h)| (f64::from(h) * geometry.scale).round())
                .unwrap_or(0.0);
            item.slot.set_position(
                Point {
                    x: 0.0,
                    y: y_px as f32,
                },
                None,
            );
            item.slot.set_size(
                Size::points(geometry.width_px as f32, height_px as f32),
                None,
            );
            if height_px > 0.0 {
                y_px += height_px + gap_px;
            }
        }
    }

    /// Where the column goes on the canvas output.
    fn canvas_geometry(&self) -> Option<ColumnGeometry> {
        let output = self.canvas.output.as_ref()?;
        let output_geo = self.workspaces.output_geometry(output)?;
        let scale = output.current_scale().fractional_scale();
        let usable = self.usable_zone(output);
        let (width, margin) = Config::with(|c| (c.canvas.clamped_width(), c.canvas.margin));
        let width = f64::from(width);
        let margin = f64::from(margin);

        // Output-local logical points, then physical pixels.
        let right = f64::from(usable.loc.x - output_geo.loc.x + usable.size.w) - margin;
        let top = f64::from(usable.loc.y - output_geo.loc.y) + margin;
        let height = (f64::from(usable.size.h) - 2.0 * margin).max(0.0);
        Some(ColumnGeometry {
            scale,
            shown_x_px: ((right - width) * scale).round(),
            hidden_x_px: (f64::from(output_geo.size.w) * scale).round(),
            top_px: (top * scale).round(),
            width_px: (width * scale).round(),
            height_px: (height * scale).round(),
        })
    }

    /// The column's current bounds in global logical coordinates.
    fn canvas_column_rect(&self) -> Option<Rectangle<f64, Logical>> {
        let output = self.canvas.output.as_ref()?;
        let column = self.canvas.column.as_ref()?;
        let origin = self.workspaces.output_geometry(output)?.loc.to_f64();
        let scale = output.current_scale().fractional_scale();
        let at = column.render_position();
        let size = column.render_size();
        Some(Rectangle::new(
            (
                origin.x + at.x as f64 / scale,
                origin.y + at.y as f64 / scale,
            )
                .into(),
            (size.x as f64 / scale, size.y as f64 / scale).into(),
        ))
    }

    /// The item whose surface tree is under `location`, as its root surface.
    fn canvas_item_root_under(
        &self,
        location: smithay::utils::Point<f64, Logical>,
    ) -> Option<WlSurface> {
        let (target, _) = self.canvas_surface_under(location)?;
        let PointerFocusTarget::WlSurface(surface) = target else {
            return None;
        };
        self.canvas_item_root_for(&surface)
    }

    /// Give `surface` the keyboard, remembering who had it.
    fn canvas_focus_item(&mut self, surface: WlSurface) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let current = keyboard.current_focus();
        if matches!(&current, Some(KeyboardFocusTarget::CanvasItem(s)) if *s == surface) {
            return;
        }
        if !matches!(current, Some(KeyboardFocusTarget::CanvasItem(_))) {
            self.canvas.previous_focus = current;
        }
        keyboard.set_focus(
            self,
            Some(KeyboardFocusTarget::CanvasItem(surface)),
            SERIAL_COUNTER.next_serial(),
        );
    }

    /// Hand the keyboard back to whoever had it before a canvas item took
    /// it. A no-op when no item has it.
    fn canvas_restore_focus(&mut self) {
        let previous = self.canvas.previous_focus.take().filter(IsAlive::alive);
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if matches!(
            keyboard.current_focus(),
            Some(KeyboardFocusTarget::CanvasItem(_))
        ) {
            keyboard.set_focus(self, previous, SERIAL_COUNTER.next_serial());
        }
    }

    /// Finish a slide out whose animation has reached its end.
    fn canvas_finish_close_if_done(&mut self) {
        if let Phase::Closing { generation } = self.canvas.phase {
            if self.canvas.closed_generation.load(Ordering::Relaxed) >= generation {
                self.canvas_finish_close();
            }
        }
    }

    /// Take the canvas off screen for good and tell the items.
    fn canvas_finish_close(&mut self) {
        let was_on_screen = self.canvas.on_screen();
        self.canvas_restore_focus();
        self.canvas.phase = Phase::Hidden;
        self.canvas.drag_progress = 0.0;
        for ows in self.workspaces.output_workspaces.values() {
            ows.canvas_plane.set_hidden(true);
        }
        if was_on_screen {
            for item in &self.canvas.items {
                item.resource.hidden();
            }
        }
        self.canvas.output = None;
        tracing::debug!("canvas off screen");
    }

    /// The scale items are mirrored at: the canvas output's, or the
    /// primary output's while the canvas is on no output.
    fn canvas_scale(&self) -> f64 {
        self.canvas
            .output
            .as_ref()
            .or_else(|| self.workspaces.primary_output())
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0)
    }

    /// Tell an item's client the scale it will be shown at, so it draws its
    /// buffer at the right density before it is seen.
    fn canvas_prefer_scale(&self, surface: &WlSurface) {
        if !surface.alive() {
            return;
        }
        let scale = self.canvas_scale();
        with_states(surface, |states| {
            with_fractional_scale(states, |fractional| {
                fractional.set_preferred_scale(scale);
            });
        });
    }

    /// Ask the backend to draw: a canvas change is a scene change nobody
    /// else is going to ask for.
    fn canvas_request_redraw(&mut self) {
        self.backend_data.invalidate_scene_prefetch();
        self.backend_data.request_redraw();
        self.schedule_event_loop_dispatch();
    }
}

/// The spring the column settles with.
fn settle_transition() -> Transition {
    Transition {
        delay: 0.0,
        timing: TimingFunction::Spring(Spring::with_duration_and_bounce(
            SETTLE_DURATION,
            SETTLE_BOUNCE,
        )),
    }
}

/// The configured column width, in logical points.
fn canvas_width_points() -> u32 {
    Config::with(|c| c.canvas.clamped_width())
}

/// Send `resource` a configure for `width` logical points.
fn send_configure(resource: &OttoCanvasItemV1, width: u32) {
    let serial = SERIAL_COUNTER.next_serial();
    resource.configure(serial.into(), width);
}

/// The logical size of the buffer `surface` has attached, if any.
fn item_size_points(surface: &WlSurface) -> Option<(i32, i32)> {
    if !surface.alive() {
        return None;
    }
    with_states(surface, |states| {
        states
            .data_map
            .get::<smithay::backend::renderer::utils::RendererSurfaceStateUserData>()
            .and_then(|data| {
                data.lock()
                    .ok()?
                    .view()
                    .map(|view| (view.dst.w, view.dst.h))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::ColumnGeometry;

    fn geometry() -> ColumnGeometry {
        ColumnGeometry {
            scale: 2.0,
            shown_x_px: 1000.0,
            hidden_x_px: 1800.0,
            top_px: 80.0,
            width_px: 776.0,
            height_px: 1000.0,
        }
    }

    #[test]
    fn hidden_is_fully_past_the_right_edge() {
        let g = geometry();
        assert_eq!(g.x_for(0.0), 1800.0);
        assert_eq!(g.x_for(1.0), 1000.0);
    }

    #[test]
    fn progress_round_trips_through_position() {
        let g = geometry();
        for progress in [0.0, 0.25, 0.5, 1.0] {
            assert!((g.progress_at(g.x_for(progress)) - progress).abs() < 1e-3);
        }
    }

    #[test]
    fn progress_is_clamped_past_either_end() {
        let g = geometry();
        assert_eq!(g.progress_at(2000.0), 0.0);
        assert_eq!(g.progress_at(500.0), 1.0);
    }
}
