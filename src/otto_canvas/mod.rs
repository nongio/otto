//! Side canvas: a column of client surfaces that slides in over the desktop
//! from the right edge of the output.
//!
//! The column is the compositor's; what is in it belongs to clients, which
//! place surfaces there through `otto-canvas-v1` (see [`handlers`]). Items
//! stack top to bottom by the order their clients set (creation order where
//! it is the same), each at the column's width, and each as tall as the
//! buffer its client attached. Whatever does not fit in
//! the usable height is clipped. Clients bound at version 5 say how tall
//! their content is and are told how tall they may be, so the column's
//! height is shared rather than cut off at the bottom (see [`allot`]).
//!
//! The canvas is an overlay: it slides over the desktop and moves nothing
//! underneath. It lives in the output's overlay plane (`canvas_plane` in
//! [`crate::workspaces::OutputWorkspaces`]), above the dock and every
//! window, fullscreen ones included, and below the lock screen.
//!
//! It is driven by a trackpad swipe from the right edge (the input side calls
//! the `canvas_gesture_*` methods), by the `CanvasToggle` action, and by
//! clients asking for it to be shown or dismissed. A canvas a client showed
//! is passive until the user acts on it: the keyboard stays with the app and
//! Escape goes to the app. Shown either way, a press outside it reaches what
//! is under the pointer, and the canvas hides when that button comes up
//! outside it, unless the press started a drag: something dragged from a
//! window can then be dropped on an item.
//!
//! A drag and drop operation that rests at the right edge of an output opens
//! the canvas, passive, and every manager is told when a drag starts and
//! ends, so a client can put an item there to take the drop (see [`drag`]).
//! While it is off screen its items get no frame callbacks and are told
//! `hidden`, so they can stop drawing.
//!
//! See `specs/side-canvas.md` and `docs/developer/side-canvas.md`.

pub mod allot;
pub mod drag;
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
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{IsAlive, Logical, Rectangle, SERIAL_COUNTER};
use smithay::wayland::compositor::with_states;
use smithay::wayland::fractional_scale::with_fractional_scale;

use crate::config::Config;
use crate::focus::{KeyboardFocusTarget, PointerFocusTarget};
use crate::otto_canvas::allot::Demand;
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
    /// When the item takes the keyboard.
    keyboard: ItemKeyboard,
    /// Where the item sits: lower is higher in the column.
    order: i32,
    /// When the item was created, for items with the same order.
    seq: u64,
    /// How tall the item's content is, in logical points, once its client
    /// has said (version 5).
    content_height: Option<u32>,
    /// The last `max_height` the item was sent, in logical points.
    max_height: Option<u32>,
}

/// When a canvas item takes the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKeyboard {
    /// When the user presses on it (`none` in the protocol).
    OnPress,
    /// When the user presses on it, and whenever the user shows the canvas.
    OnShow,
    /// Never: presses reach it as pointer events only.
    Never,
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
    /// Creation counter for [`CanvasItem::seq`].
    next_seq: u64,
    /// The canvas was shown by a client and the user has not acted on it
    /// since. See the module docs.
    passive: bool,
    /// A button pressed outside the shown canvas. The canvas hides when it
    /// is released outside the column, unless it started a drag.
    press_outside: Option<drag::PressOutside>,
    /// Every bound manager, told when drags start and end.
    managers: Vec<OttoCanvasManagerV1>,
    /// The drag and drop operation going on, if any.
    drag: Option<drag::DragSession>,
    /// The timer that watches for the pointer resting at the edge during a
    /// drag.
    drag_timer: Option<RegistrationToken>,
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
            next_seq: 0,
            passive: false,
            press_outside: None,
            managers: Vec::new(),
            drag: None,
            drag_timer: None,
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

/// The point to find the pointer's output from. Pushed against the right
/// edge of the layout, the pointer is clamped to exactly the output's right
/// bound, which no output rectangle contains; looking a little to the left
/// finds the output it is resting on.
fn edge_probe(
    location: smithay::utils::Point<f64, Logical>,
) -> smithay::utils::Point<f64, Logical> {
    (location.x - 1.0, location.y).into()
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
        self.canvas.passive = false;
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
            self.canvas_settle_open(true);
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

    /// Slide the canvas in on the output under the pointer, for the user.
    /// Does nothing when no client has put anything in it.
    pub fn canvas_show(&mut self) {
        self.canvas_finish_close_if_done();
        if matches!(self.canvas.phase, Phase::Shown) || !self.canvas_available() {
            return;
        }
        self.canvas.passive = false;
        if matches!(self.canvas.phase, Phase::Hidden) {
            if !self.canvas_bring_on_screen() {
                return;
            }
            self.canvas_place_column(0.0, false);
        }
        self.canvas_settle_open(true);
    }

    /// A client asked for the canvas to be shown. It slides in as for the
    /// user, but passive: the keyboard stays where it is. Ignored while the
    /// canvas is shown or following the fingers, while the session is
    /// locked and during exposé.
    pub(crate) fn canvas_item_show(&mut self, item: &OttoCanvasItemV1) {
        if !self
            .canvas
            .items
            .iter()
            .any(|entry| &entry.resource == item)
        {
            return;
        }
        if self.canvas_show_passive() {
            tracing::debug!("canvas shown by a client");
        }
    }

    /// Slide the canvas in without moving the keyboard, as a client's show
    /// or a drag resting at the edge does. Returns whether it is now opening.
    fn canvas_show_passive(&mut self) -> bool {
        self.canvas_finish_close_if_done();
        if self.is_session_locked() || self.canvas_suspended() || !self.canvas_available() {
            return false;
        }
        match self.canvas.phase {
            Phase::Shown | Phase::Dragging { .. } => return false,
            Phase::Hidden => {
                if !self.canvas_bring_on_screen() {
                    return false;
                }
                self.canvas_place_column(0.0, false);
            }
            Phase::Closing { .. } => {}
        }
        self.canvas.passive = true;
        self.canvas_settle_open(false);
        true
    }

    /// Whether Escape is the canvas's, to hide it: the canvas is shown and
    /// active, and no item has the keyboard. A passive canvas leaves Escape
    /// to the app that has the keyboard.
    pub fn canvas_owns_escape(&self) -> bool {
        self.canvas_is_shown()
            && !self.canvas.passive
            && !self.canvas_suspended()
            && !self.canvas_item_has_keyboard()
    }

    /// Whether a client showed the canvas and the user has not acted on it
    /// since.
    pub fn canvas_is_passive(&self) -> bool {
        self.canvas.passive && self.canvas_is_shown()
    }

    /// Whether `pos` is on the column, while the canvas is on screen.
    pub fn canvas_column_contains(&self, pos: smithay::utils::Point<f64, Logical>) -> bool {
        self.canvas.on_screen()
            && self
                .canvas_column_rect()
                .is_some_and(|rect| rect.contains(pos))
    }

    /// The canvas items' surfaces, top to bottom.
    pub fn canvas_item_surfaces(&self) -> Vec<WlSurface> {
        self.canvas
            .items
            .iter()
            .map(|item| item.surface.clone())
            .collect()
    }

    /// Slide the canvas out, and give the keyboard back to whoever had it.
    pub fn canvas_hide(&mut self) {
        match self.canvas.phase {
            Phase::Hidden | Phase::Closing { .. } => return,
            Phase::Shown | Phase::Dragging { .. } => {}
        }
        self.canvas.passive = false;
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

    /// Whether a canvas item has the keyboard. It is then the item's to say
    /// what Escape means.
    pub fn canvas_item_has_keyboard(&self) -> bool {
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .is_some_and(|focus| matches!(focus, KeyboardFocusTarget::CanvasItem(_)))
    }

    /// Whether exposé has the screen. The canvas stays open but fades out
    /// with the rest of the overlay chrome and takes no input until exposé
    /// closes.
    pub fn canvas_suspended(&self) -> bool {
        self.workspaces.get_show_all()
    }

    /// The canvas item under `pos`, if the canvas is on screen there.
    pub fn canvas_surface_under(
        &self,
        pos: smithay::utils::Point<f64, Logical>,
    ) -> Option<(PointerFocusTarget<B>, smithay::utils::Point<f64, Logical>)> {
        if !self.canvas.on_screen() || self.canvas_suspended() {
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
    /// it. A press on an item gives that item the keyboard; a press on the
    /// column between items is taken, as is its release.
    ///
    /// A press outside a shown canvas goes through to what is under the
    /// pointer, and the canvas hides when that button is released outside
    /// it, unless the press started a drag: something dragged from a window
    /// can then be dropped on an item.
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
            if let Some(press) = self
                .canvas
                .press_outside
                .filter(|press| press.button == button)
            {
                self.canvas.press_outside = None;
                let location = self.pointer.current_location();
                let inside = self
                    .canvas_column_rect()
                    .is_some_and(|rect| rect.contains(location));
                if drag::hide_on_release(press, inside) {
                    self.canvas_hide();
                }
            }
            return CanvasButton::Pass;
        }
        if self.is_session_locked()
            || self.canvas_suspended()
            || !matches!(self.canvas.phase, Phase::Shown)
        {
            return CanvasButton::Pass;
        }
        let location = self.pointer.current_location();
        let inside = self
            .canvas_column_rect()
            .is_some_and(|rect| rect.contains(location));
        if !inside {
            self.canvas.press_outside = Some(drag::PressOutside {
                button,
                dragged: false,
            });
            return CanvasButton::Pass;
        }
        match self.canvas_item_root_under(location) {
            Some(surface) => {
                let keyboard = self
                    .canvas
                    .items
                    .iter()
                    .find(|item| item.surface == surface)
                    .map_or(ItemKeyboard::OnPress, |item| item.keyboard);
                if keyboard != ItemKeyboard::Never {
                    self.canvas.passive = false;
                    self.canvas_focus_item(surface);
                }
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
        self.canvas_prefer_scale(&surface);

        let seq = self.canvas.next_seq;
        self.canvas.next_seq += 1;
        self.canvas.items.push(CanvasItem {
            resource: resource.clone(),
            surface,
            slot,
            layer,
            keyboard: ItemKeyboard::OnPress,
            order: 0,
            seq,
            content_height: None,
            max_height: None,
        });
        self.canvas_sort_items();
        // The new item's share comes before it hears whether it is seen.
        self.canvas_share_height();
        if self.canvas.on_screen() {
            resource.shown();
        } else {
            resource.hidden();
        }
        tracing::debug!(items = self.canvas.items.len(), "canvas item added");
    }

    /// A client said when its item takes the keyboard. `OnShow` applies from
    /// the next show; `Never` from the next press.
    pub(crate) fn canvas_item_set_keyboard(
        &mut self,
        item: &OttoCanvasItemV1,
        keyboard: ItemKeyboard,
    ) {
        if let Some(item) = self
            .canvas
            .items
            .iter_mut()
            .find(|entry| &entry.resource == item)
        {
            item.keyboard = keyboard;
        }
    }

    /// A client moved its item in the column. The column is laid out again
    /// at once.
    pub(crate) fn canvas_item_set_order(&mut self, item: &OttoCanvasItemV1, order: i32) {
        let Some(entry) = self
            .canvas
            .items
            .iter_mut()
            .find(|entry| &entry.resource == item)
        else {
            return;
        };
        if entry.order == order {
            return;
        }
        entry.order = order;
        self.canvas_sort_items();
        self.canvas_relayout();
        if self.canvas.on_screen() {
            self.canvas_request_redraw();
        }
    }

    /// A client said how tall its item's content is. The column's height is
    /// shared out again, which the item and its neighbours hear of as
    /// `max_height`.
    pub(crate) fn canvas_item_set_content_height(&mut self, item: &OttoCanvasItemV1, height: u32) {
        let Some(entry) = self
            .canvas
            .items
            .iter_mut()
            .find(|entry| &entry.resource == item)
        else {
            return;
        };
        if entry.content_height == Some(height) {
            return;
        }
        entry.content_height = Some(height);
        self.canvas_share_height();
    }

    /// The column's height on the output it is on, or would open on, in
    /// logical points: what the items share.
    pub fn canvas_available_height(&self) -> Option<u32> {
        let output = self.canvas_target_output()?;
        let geometry = self.canvas_geometry_on(&output)?;
        Some(available_points(&geometry))
    }

    /// Share the column's height among the items again, and tell those
    /// bound at version 5 whose share changed.
    ///
    /// An item that has not said how tall its content is counts at its
    /// buffer's height for the others, and is offered what it could have if
    /// it wanted all of the column.
    pub(crate) fn canvas_share_height(&mut self) {
        if self.canvas.items.is_empty() {
            return;
        }
        let Some(available) = self.canvas_available_height() else {
            return;
        };
        let gap = Config::with(|c| c.canvas.gap);
        let demands: Vec<Demand> = self
            .canvas
            .items
            .iter()
            .map(|item| match item.content_height {
                Some(height) if item.resource.version() >= MAX_HEIGHT_SINCE => {
                    Demand::Content(height)
                }
                _ => Demand::Fixed(buffer_height_points(&item.surface)),
            })
            .collect();
        let shares = allot::allot(available, gap, &demands);
        for (index, item) in self.canvas.items.iter_mut().enumerate() {
            if item.resource.version() < MAX_HEIGHT_SINCE || !item.resource.is_alive() {
                continue;
            }
            let share = if item.content_height.is_some() {
                shares[index]
            } else {
                let mut wanting = demands.clone();
                wanting[index] = Demand::Content(available);
                allot::allot(available, gap, &wanting)[index]
            };
            if item.max_height != Some(share) {
                item.max_height = Some(share);
                item.resource.max_height(share);
            }
        }
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

    // ── Drag and drop ────────────────────────────────────────────────────

    /// A client bound the manager. One that binds while a drag goes on is
    /// told about it at once.
    pub(crate) fn canvas_manager_bound(&mut self, manager: OttoCanvasManagerV1) {
        if let Some(session) = self.canvas.drag.as_ref() {
            send_drag_started(&manager, &session.mime_types);
        }
        self.canvas.managers.push(manager);
    }

    /// A manager is gone, by request or with its client.
    pub(crate) fn canvas_manager_destroyed(&mut self, id: &ObjectId) {
        self.canvas
            .managers
            .retain(|manager| &manager.id() != id && manager.is_alive());
    }

    /// Whether a drag and drop operation is going on.
    pub fn canvas_drag_active(&self) -> bool {
        self.canvas.drag.is_some()
    }

    /// A drag and drop operation began, offering its data as `mime_types`.
    /// Every manager is told, and the pointer is watched for a rest at the
    /// right edge of an output, which opens the canvas.
    pub fn canvas_drag_started(&mut self, mime_types: Vec<String>) {
        if self.canvas.drag.is_some() {
            self.canvas_drag_ended(None);
        }
        if let Some(press) = self.canvas.press_outside.as_mut() {
            press.dragged = true;
        }
        self.canvas.managers.retain(Resource::is_alive);
        for manager in &self.canvas.managers {
            send_drag_started(manager, &mime_types);
        }
        self.canvas.drag = Some(drag::DragSession {
            mime_types,
            ..Default::default()
        });
        let inserted =
            self.handle
                .insert_source(Timer::from_duration(drag::POLL), |_, _, state| {
                    if state.canvas_drag_tick() {
                        TimeoutAction::ToDuration(drag::POLL)
                    } else {
                        state.canvas.drag_timer = None;
                        TimeoutAction::Drop
                    }
                });
        match inserted {
            Ok(token) => self.canvas.drag_timer = Some(token),
            Err(err) => {
                tracing::warn!(error = %err.error, "cannot watch the drag for the canvas edge");
            }
        }
        tracing::debug!("drag started");
    }

    /// The drag and drop operation ended, dropped on `target` or cancelled
    /// (`None`). A canvas the drag opened goes again, unless the drop
    /// landed on one of its items.
    pub fn canvas_drag_ended(&mut self, target: Option<&WlSurface>) {
        let Some(session) = self.canvas.drag.take() else {
            return;
        };
        if let Some(token) = self.canvas.drag_timer.take() {
            self.handle.remove(token);
        }
        let landed_on_item =
            target.is_some_and(|surface| self.canvas_item_root_for(surface).is_some());
        if drag::hide_when_drag_ends(session.opened_canvas, landed_on_item)
            && self.canvas_is_shown()
        {
            self.canvas_hide();
        }
        for manager in &self.canvas.managers {
            if manager.is_alive() && manager.version() >= DRAG_SINCE {
                manager.drag_ended();
            }
        }
        tracing::debug!(landed_on_item, "drag ended");
    }

    /// Look at the pointer during a drag: resting at the right edge of an
    /// output long enough opens the canvas there. Returns whether the drag
    /// is still going on.
    fn canvas_drag_tick(&mut self) -> bool {
        if self.canvas.drag.is_none() {
            return false;
        }
        let location = self.pointer.current_location();
        let at_edge = self
            .workspaces
            .output_under(edge_probe(location))
            .next()
            .and_then(|output| self.workspaces.output_geometry(output))
            .is_some_and(|geo| {
                drag::at_right_edge(
                    location.x,
                    f64::from(geo.loc.x + geo.size.w),
                    f64::from(canvas_width_points()),
                )
            });
        let now = std::time::Instant::now();
        let Some(session) = self.canvas.drag.as_mut() else {
            return false;
        };
        if !session.dwell.observe(at_edge, now) {
            return true;
        }
        if self.canvas_is_shown() {
            return true;
        }
        if self.canvas_show_passive() {
            if let Some(session) = self.canvas.drag.as_mut() {
                session.opened_canvas = true;
            }
            tracing::debug!("canvas opened by a drag at the edge");
        } else if let Some(session) = self.canvas.drag.as_mut() {
            // Nothing to show yet: a client may still be adding its item.
            session.dwell.rearm();
        }
        true
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
            .output_under(edge_probe(pointer))
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

    /// Slide the column to fully shown and, when `focus` says so, give the
    /// keyboard to the item that asks for it on show.
    fn canvas_settle_open(&mut self, focus: bool) {
        self.canvas.phase = Phase::Shown;
        self.canvas_place_column(1.0, true);
        if focus {
            self.canvas_focus_on_show();
        }
        self.canvas_request_redraw();
    }

    /// Keep the items in column order: by order, then by creation.
    fn canvas_sort_items(&mut self) {
        self.canvas.items.sort_by_key(|item| (item.order, item.seq));
    }

    /// Give the keyboard to the first item that takes it on show, unless an
    /// item has it already: one the user pressed on keeps it.
    fn canvas_focus_on_show(&mut self) {
        if self.is_session_locked() || self.canvas_item_has_keyboard() {
            return;
        }
        let surface = self
            .canvas
            .items
            .iter()
            .find(|item| item.keyboard == ItemKeyboard::OnShow && item.surface.alive())
            .map(|item| item.surface.clone());
        if let Some(surface) = surface {
            self.canvas_focus_item(surface);
        }
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
    /// buffer's height, a gap apart. Their shares of the column's height are
    /// worked out again first, since a buffer or the column may have
    /// changed.
    fn canvas_relayout(&mut self) {
        self.canvas.items.retain(|item| item.resource.is_alive());
        self.canvas_share_height();
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
        self.canvas_geometry_on(self.canvas.output.as_ref()?)
    }

    /// The output the column is on, or the one it would open on: under the
    /// pointer, or the primary output.
    fn canvas_target_output(&self) -> Option<Output> {
        self.canvas
            .output
            .as_ref()
            .or_else(|| {
                self.workspaces
                    .output_under(edge_probe(self.pointer.current_location()))
                    .next()
            })
            .or_else(|| self.workspaces.primary_output())
            .cloned()
    }

    /// Where the column goes on `output`.
    fn canvas_geometry_on(&self, output: &Output) -> Option<ColumnGeometry> {
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
        self.canvas.passive = false;
        self.canvas.press_outside = None;
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

/// The manager version that brought the drag events.
const DRAG_SINCE: u32 = 4;

/// Tell `manager` a drag began, offering `mime_types`.
fn send_drag_started(manager: &OttoCanvasManagerV1, mime_types: &[String]) {
    if !manager.is_alive() || manager.version() < DRAG_SINCE {
        return;
    }
    for mime_type in mime_types {
        manager.drag_mime_type(mime_type.clone());
    }
    manager.drag_started();
}

/// Send `resource` a configure for `width` logical points.
fn send_configure(resource: &OttoCanvasItemV1, width: u32) {
    let serial = SERIAL_COUNTER.next_serial();
    resource.configure(serial.into(), width);
}

/// The item version that brought `set_content_height` and `max_height`.
const MAX_HEIGHT_SINCE: u32 = 5;

/// The column's height in whole logical points, rounded down so that
/// items sized to it never reach past the bottom.
fn available_points(geometry: &ColumnGeometry) -> u32 {
    let points = (geometry.height_px / geometry.scale.max(f64::EPSILON)).floor();
    // A height is never negative, and no output is four billion points tall.
    points.clamp(0.0, f64::from(u32::MAX)) as u32
}

/// The logical height of the buffer `surface` has attached; zero without
/// one.
fn buffer_height_points(surface: &WlSurface) -> u32 {
    item_size_points(surface).map_or(0, |(_, h)| u32::try_from(h).unwrap_or(0))
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
