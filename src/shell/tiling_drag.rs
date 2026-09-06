//! Pointer interaction with tiles: dragging one out of the tree and into a new
//! slot, and dragging the edge between two of them.
//!
//! Two gestures, one idea — the pointer edits the tree, never a window's free
//! geometry:
//!
//! * a **titlebar drag** detaches the leaf, lets the tree close up behind it,
//!   shows the slot it would land in with [`TilingOverlayView`]'s pane, and on
//!   release inserts, swaps or fills (`specs/tiling.md`, *Dragging a window*);
//! * an **edge drag** is a drag of the split that edge sits on, moving the two
//!   shares either side of it. The window itself never gets a free size, so a
//!   relayout has nothing to reassert.
//!
//! The arithmetic is pure and lives in [`crate::workspaces::tiling::drag`];
//! this is the compositor half.
//!
//! [`TilingOverlayView`]: crate::workspaces::TilingOverlayView

use layers::prelude::{Point as LayerPoint, Transition};
use smithay::{
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::backend::ObjectId,
    },
    utils::{Logical, Point},
    wayland::{compositor::with_states, shell::xdg::SurfaceCachedState},
};

use crate::{
    config::Config,
    shell::{ResizeEdge, WindowElement},
    state::{Backend, Otto},
    workspaces::tiling::{
        drag::{self, DropSide},
        layout::{self, Rect},
        tree::{Axis, NodeId, Tree},
    },
};

/// How far a detached window shrinks while it follows the pointer.
///
/// The spec's "reduced size": a scale on the window's own layer, not a client
/// resize — the client keeps drawing its cell-sized buffer throughout, so the
/// drag costs it nothing and the drop does not wait on it.
pub const DRAG_SCALE: f32 = 0.6;

/// A titlebar drag that took a window out of a tree.
pub struct TilingDrag {
    /// The window following the pointer.
    pub window: ObjectId,
    /// The output whose workspace it came from.
    pub output: Output,
    /// The tree as it stood before the detach, for Escape to put back.
    pub snapshot: Tree<ObjectId>,
    /// The focus that went with it.
    pub focused: Option<ObjectId>,
    /// The slot the pointer is over, as of the last motion.
    pub target: Option<(NodeId, DropSide)>,
}

/// An edge drag that is moving one or two splits.
pub struct TilingResize {
    pub window: ObjectId,
    pub output: Output,
    /// The splits under the dragged edges, as `(container, bar index)` —
    /// one per axis, so a corner holds two.
    pub splits: Vec<(NodeId, usize)>,
}

/// What starting an edge drag on a window found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilingResizeStart {
    /// Not a tile: the caller's own free-resize grab applies.
    NotTiled,
    /// A tile, but this edge is the outside of the tree — nothing to drag.
    NoSplit,
    /// A tile with a split under the edge; the drag is armed.
    Started,
}

impl<BackendData: Backend> Otto<BackendData> {
    // ── Dragging a window ────────────────────────────────────────────────

    /// Is a titlebar drag out of a tree in flight?
    pub fn tiling_drag_is_active(&self) -> bool {
        self.tiling_drag.is_some()
    }

    /// Take `window` out of its workspace's tree and let it follow the
    /// pointer.
    ///
    /// Returns false — and changes nothing — when the window is not a leaf of
    /// a tiling workspace, which is every ordinary floating drag.
    pub fn tiling_drag_begin(&mut self, window: &WindowElement) -> bool {
        if self.tiling_drag.is_some() {
            return false;
        }
        let id = window.id();
        let Some(output) = self.workspaces.output_for_window(window) else {
            return false;
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return false;
        };
        let (snapshot, focused) = {
            let Ok(mut state) = workspace.tiling.write() else {
                return false;
            };
            if !state.enabled || !state.tree.contains(&id) {
                return false;
            }
            let snapshot = state.tree.clone();
            let focused = state.focused.clone();
            state.tree.remove(&id);
            if state.focused.as_ref() == Some(&id) {
                state.focused = state.tree.leaves().first().cloned();
            }
            (snapshot, focused)
        };
        drop(workspace);

        // Out of the tree is a floating window again: full chrome, no tiled
        // states, no maximized flag left over from being a lone tile.
        window.set_decoration_variant(otto_kit::components::titlebar::DecorationVariant::Floating);
        window.set_is_maximized(false);
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Maximized);
                state.states.unset(xdg_toplevel::State::TiledLeft);
                state.states.unset(xdg_toplevel::State::TiledRight);
                state.states.unset(xdg_toplevel::State::TiledTop);
                state.states.unset(xdg_toplevel::State::TiledBottom);
            });
            toplevel.send_pending_configure();
        }
        self.set_drag_scale(&id, DRAG_SCALE);

        self.tiling_drag = Some(TilingDrag {
            window: id,
            output: output.clone(),
            snapshot,
            focused,
            target: None,
        });
        // The tree closes up behind it on the layout spring.
        self.relayout_workspace(&output, true);
        true
    }

    /// Follow the pointer: work out the slot under it and draw the pane.
    pub fn tiling_drag_motion(&mut self, x: f64, y: f64) {
        let Some(output) = self.tiling_drag.as_ref().map(|d| d.output.clone()) else {
            return;
        };
        let Some((area, cells)) = self.tiling_cells(&output) else {
            if let Some(drag) = self.tiling_drag.as_mut() {
                drag.target = None;
            }
            self.workspaces.tiling_overlay.hide();
            return;
        };
        let target = drag::drop_target(&cells, x, y);
        if let Some(drag) = self.tiling_drag.as_mut() {
            drag.target = target;
        }

        // Over a tile: the half nearer the pointer, or the whole of it for a
        // swap. Over nothing — an empty workspace, or the space a tree does
        // not reach — the whole area, which is where the drop would land.
        let preview = match target {
            Some((node, side)) => cells
                .iter()
                .find(|(n, _)| *n == node)
                .map(|(_, rect)| drag::preview_rect(*rect, side)),
            None => Some(area),
        };
        match preview {
            Some(rect) => {
                let scale = output.current_scale().fractional_scale() as f32;
                self.workspaces.tiling_overlay.show_zone(
                    rect.x as f32 * scale,
                    rect.y as f32 * scale,
                    rect.w as f32 * scale,
                    rect.h as f32 * scale,
                    scale,
                );
            }
            None => self.workspaces.tiling_overlay.hide(),
        }
    }

    /// Let go: put the window in the slot the overlay was showing.
    pub fn tiling_drag_drop(&mut self, x: f64, y: f64) {
        let Some(drag) = self.tiling_drag.take() else {
            return;
        };
        self.workspaces.tiling_overlay.hide();
        self.set_drag_scale(&drag.window, 1.0);

        let output = drag.output.clone();
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return;
        };
        let area = self
            .tiling_cells(&output)
            .map(|(area, _)| area)
            .unwrap_or_else(|| {
                let zone = self.tiling_area(&output);
                Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h)
            });

        if let Ok(mut state) = workspace.tiling.write() {
            if !state.enabled {
                return;
            }
            let id = drag.window.clone();
            match drag.target {
                // An empty slot takes the window whole, whichever part of it
                // the pointer was over.
                Some((node, _))
                    if matches!(
                        state.tree.cell(node),
                        Some(crate::workspaces::tiling::Cell::Empty(_))
                    ) =>
                {
                    if let Some(crate::workspaces::tiling::Cell::Empty(slot)) =
                        state.tree.cell(node)
                    {
                        state.tree.fill_empty(slot, id.clone());
                    }
                }
                Some((node, DropSide::Centre)) => {
                    match state
                        .tree
                        .cell(node)
                        .and_then(|cell| cell.window().cloned())
                    {
                        // A swap puts the layout back the way it was and
                        // exchanges the two windows in it: the dragged one
                        // takes this tile, and this tile's window takes the
                        // cell the drag came out of.
                        Some(other) => {
                            state.tree = drag.snapshot.clone();
                            state.tree.swap_leaves(&id, &other);
                        }
                        None => {
                            state.tree.insert_at_root_edge(id.clone(), Axis::Row, true);
                        }
                    }
                }
                Some((node, side)) => {
                    let (axis, after) = side.axis().unwrap_or((Axis::Row, true));
                    state.tree.insert_beside(node, id.clone(), axis, after);
                }
                None => {
                    let (axis, after) = drag::nearest_root_edge(area, x, y);
                    state.tree.insert_at_root_edge(id.clone(), axis, after);
                }
            }
            state.focused = Some(id);
        }
        drop(workspace);
        // Forced: the window may land on the very rectangle it is already at,
        // and it still has to be given the tile's decoration and states back.
        self.relayout_workspace_forced(&output, true, true);
    }

    /// Escape during a drag: put the leaf back, and let go of the pointer so
    /// the window stops following it.
    pub(crate) fn handle_tiling_drag_cancel(&mut self) {
        if !self.tiling_drag_is_active() {
            return;
        }
        self.tiling_drag_cancel();
        if let Some(pointer) = self.seat.get_pointer() {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            pointer.unset_grab(self, serial, 0);
        }
    }

    /// Escape, or the grab going away: put the leaf back where it was, with
    /// the share it had.
    pub fn tiling_drag_cancel(&mut self) {
        let Some(drag) = self.tiling_drag.take() else {
            return;
        };
        self.workspaces.tiling_overlay.hide();
        self.set_drag_scale(&drag.window, 1.0);
        let output = drag.output.clone();
        if let Some(workspace) = self.workspaces.current_tiling_workspace(&output) {
            if let Ok(mut state) = workspace.tiling.write() {
                state.tree = drag.snapshot;
                state.focused = drag.focused;
            }
        }
        self.relayout_workspace_forced(&output, true, true);
    }

    /// Scale the window's own layer, animated. Not a client resize: the
    /// buffer it has drawn is simply drawn smaller while it is in the air.
    fn set_drag_scale(&mut self, id: &ObjectId, scale: f32) {
        let Some(view) = self.workspaces.get_window_view(id) else {
            return;
        };
        view.window_layer.set_scale(
            LayerPoint { x: scale, y: scale },
            Some(Transition::ease_out_quad(0.15)),
        );
    }

    /// The current workspace's area and every cell in it, in logical pixels.
    fn tiling_cells(&mut self, output: &Output) -> Option<(Rect, Vec<(NodeId, Rect)>)> {
        let workspace = self.workspaces.current_tiling_workspace(output)?;
        let zone = self.tiling_area(output);
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);
        let state = workspace.tiling.read().ok()?;
        if !state.enabled {
            return None;
        }
        let gaps = Config::with(|c| state.effective_gaps(&c.tiling));
        let cells = layout::resolve_nodes(&state.tree, area, gaps)
            .into_iter()
            .filter(|(node, _)| state.tree.cell(*node).is_some())
            .collect();
        Some((area, cells))
    }

    // ── Dragging an edge ─────────────────────────────────────────────────

    /// Is an edge drag in flight?
    pub fn tiling_resize_is_active(&self) -> bool {
        self.tiling_resize.is_some()
    }

    /// Is this window a leaf of a tiling workspace's tree?
    pub fn window_is_tiled(&self, window: &WindowElement) -> bool {
        let id = window.id();
        self.workspaces
            .output_for_window(window)
            .and_then(|output| self.workspaces.current_tiling_workspace(&output))
            .and_then(|view| {
                view.tiling
                    .read()
                    .ok()
                    .map(|state| state.enabled && state.tree.contains(&id))
            })
            .unwrap_or(false)
    }

    /// The splits `edges` would drag on this window, if it is a tile.
    fn splits_for_edges(
        &self,
        window: &WindowElement,
        edges: ResizeEdge,
    ) -> Option<Vec<(NodeId, usize)>> {
        let id = window.id();
        let output = self.workspaces.output_for_window(window)?;
        let workspace = self.workspaces.current_tiling_workspace(&output)?;
        let state = workspace.tiling.read().ok()?;
        if !state.enabled || !state.tree.contains(&id) {
            return None;
        }
        let mut splits = Vec::new();
        for (edge, axis, forward) in [
            (ResizeEdge::LEFT, Axis::Row, false),
            (ResizeEdge::RIGHT, Axis::Row, true),
            (ResizeEdge::TOP, Axis::Column, false),
            (ResizeEdge::BOTTOM, Axis::Column, true),
        ] {
            if edges.intersects(edge) {
                if let Some(split) = drag::split_for_edge(&state.tree, &id, axis, forward) {
                    splits.push(split);
                }
            }
        }
        Some(splits)
    }

    /// Would a border grab on these edges do anything? Drives the cursor: an
    /// edge that is the outside of the tree keeps the default arrow.
    pub fn tiling_edge_is_draggable(&self, window: &WindowElement, edges: ResizeEdge) -> bool {
        match self.splits_for_edges(window, edges) {
            // Not a tile at all: an ordinary window resizes freely.
            None => true,
            Some(splits) => !splits.is_empty(),
        }
    }

    /// Arm an edge drag on `window`.
    pub fn tiling_resize_begin(
        &mut self,
        window: &WindowElement,
        edges: ResizeEdge,
    ) -> TilingResizeStart {
        let Some(splits) = self.splits_for_edges(window, edges) else {
            return TilingResizeStart::NotTiled;
        };
        if splits.is_empty() {
            return TilingResizeStart::NoSplit;
        }
        let Some(output) = self.workspaces.output_for_window(window) else {
            return TilingResizeStart::NoSplit;
        };
        self.tiling_resize = Some(TilingResize {
            window: window.id(),
            output,
            splits,
        });
        TilingResizeStart::Started
    }

    /// Move the dragged splits to the pointer and lay the workspace out again.
    pub fn tiling_resize_to(&mut self, x: f64, y: f64) {
        let Some((output, splits)) = self
            .tiling_resize
            .as_ref()
            .map(|r| (r.output.clone(), r.splits.clone()))
        else {
            return;
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return;
        };
        let zone = self.tiling_area(&output);
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);
        // `Shift` bypasses the snap, as it does on a design-mode bar.
        let snap = !self.current_modifiers.shift;
        let minimums = self.tiling_minimums();

        let mut changed = false;
        if let Ok(mut state) = workspace.tiling.write() {
            let gaps = Config::with(|c| state.effective_gaps(&c.tiling));
            let bars = crate::workspaces::tiling::design::bar_handles(&state.tree, area, gaps);
            for (container, index) in splits {
                let Some(bar) = bars
                    .iter()
                    .find(|b| b.container == container && b.index == index)
                    .copied()
                else {
                    continue;
                };
                let along = bar.pointer_along(x, y);
                let share = bar.share_at(along, snap);
                let children = state.tree.children(container);
                let (Some(first), Some(second)) = (children.get(index), children.get(index + 1))
                else {
                    continue;
                };
                let leaf_min = |id: &ObjectId| minimum_for(&minimums, id, bar.axis);
                let min_first =
                    drag::min_extent(&state.tree, first.node, bar.axis, &leaf_min) as f32;
                let min_second =
                    drag::min_extent(&state.tree, second.node, bar.axis, &leaf_min) as f32;
                let pair = bar.shares.0 + bar.shares.1;
                let share = drag::clamp_share_to_minimums(
                    share,
                    pair,
                    bar.pair_px as f32,
                    min_first,
                    min_second,
                );
                changed |= state.tree.set_pair_shares(container, index, share);
            }
        }
        drop(workspace);
        if changed {
            let transition = Config::with(|c| c.tiling.design_transition());
            self.relayout_workspace_with(&output, transition, false);
        }
    }

    /// Let go of an edge: one last relayout so the clients land on the final
    /// rectangles even if the spring was still running.
    pub fn tiling_resize_end(&mut self) {
        let Some(resize) = self.tiling_resize.take() else {
            return;
        };
        let transition = Config::with(|c| c.tiling.design_transition());
        self.relayout_workspace_with(&resize.output, transition, false);
    }

    /// Every tiled window's minimum size, decoration included, in logical
    /// pixels. Gathered once per motion rather than per split.
    fn tiling_minimums(&self) -> Vec<(ObjectId, (i32, i32))> {
        self.workspaces
            .windows_map
            .iter()
            .filter_map(|(id, window)| {
                let surface = window.wl_surface()?;
                let min = with_states(&surface, |states| {
                    let mut guard = states.cached_state.get::<SurfaceCachedState>();
                    guard.current().min_size
                });
                // The client's minimum is what it can draw; the cell has to
                // hold the titlebar Otto puts on top of it too.
                Some((
                    id.clone(),
                    (min.w.max(0), min.h.max(0) + window.decoration_height()),
                ))
            })
            .collect()
    }
}

/// One window's minimum along `axis`, from the gathered table.
fn minimum_for(minimums: &[(ObjectId, (i32, i32))], id: &ObjectId, axis: Axis) -> i32 {
    minimums
        .iter()
        .find(|(other, _)| other == id)
        .map(|(_, (w, h))| match axis {
            Axis::Row => *w,
            Axis::Column => *h,
        })
        .unwrap_or(0)
}

/// The point a drag is measured from, so a reduced-size window keeps the spot
/// the user grabbed under the pointer.
///
/// The layer is scaled about its origin, so the offset from that origin to the
/// grab point shrinks with it.
pub fn scaled_drag_origin(
    pointer: Point<f64, Logical>,
    grab_offset: Point<f64, Logical>,
    scale: f64,
) -> Point<f64, Logical> {
    Point::from((
        pointer.x - grab_offset.x * scale,
        pointer.y - grab_offset.y * scale,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shrunk_window_keeps_the_grab_point_under_the_pointer() {
        // Grabbed 100px into a window, at 0.6 the same spot is 60px in.
        let out = scaled_drag_origin((500.0, 300.0).into(), (100.0, 20.0).into(), 0.6);
        assert!((out.x - 440.0).abs() < 1e-6);
        assert!((out.y - 288.0).abs() < 1e-6);
    }
}
