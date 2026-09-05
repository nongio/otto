//! Design mode: entering and leaving it, keeping the pane grid in step with
//! the tree, and the edits the pointer makes.
//!
//! The geometry is worked out in [`crate::workspaces::tiling::design`] and
//! drawn by [`TilingDesignView`]; everything here is the compositor half —
//! which workspace is editing, what a press does, and running the relayout
//! that makes the windows follow.
//!
//! [`TilingDesignView`]: crate::workspaces::TilingDesignView

use std::sync::Arc;

use smithay::{
    input::pointer::CursorImageStatus, output::Output, reexports::wayland_server::backend::ObjectId,
};

use crate::{
    config::Config,
    state::{Backend, Otto},
    workspaces::{
        tiling::{
            design::{self, BarHandle},
            layout, Axis, Preset, Rect,
        },
        tiling::{state::DesignDrag, tree::NodeId},
        workspace::WorkspaceView,
        DesignCell, DesignGeometry, DesignHit,
    },
};

use crate::workspaces::tiling_design::CellAction;

impl<BackendData: Backend> Otto<BackendData> {
    // ── Entering and leaving ─────────────────────────────────────────────

    /// Toggle design mode on the focused output's current workspace.
    ///
    /// A no-op on a workspace that does not tile: there is no layout to shape.
    pub(crate) fn handle_tiling_design_toggle(&mut self) {
        let Some(output) = self.tiling_output() else {
            return;
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return;
        };
        let active = {
            let Ok(mut state) = workspace.tiling.write() else {
                return;
            };
            if !state.enabled {
                return;
            }
            if state.design.active {
                state.design.leave();
                false
            } else {
                state.design.active = true;
                state.design.focused_empty = state.tree.empty_slots().first().copied();
                true
            }
        };
        if active {
            self.relayout_workspace_with(&output, None, false);
        } else {
            self.workspaces.tiling_design.hide();
        }
    }

    /// Leave design mode wherever it is up. Escape, a click on a window's
    /// content and a workspace that scrolls away all end here.
    pub fn tiling_design_leave(&mut self) {
        let mut left = false;
        for ows in self.workspaces.output_workspaces.values() {
            for view in ows.workspace_views.iter() {
                let Ok(mut state) = view.tiling.write() else {
                    continue;
                };
                if state.design.active {
                    state.design.leave();
                    left = true;
                }
            }
        }
        if left {
            self.workspaces.tiling_design.hide();
        }
    }

    /// Is design mode up on `output`'s current workspace?
    pub fn tiling_design_active(&self, output: &Output) -> bool {
        self.workspaces
            .current_tiling_workspace(output)
            .and_then(|view| {
                view.tiling
                    .read()
                    .ok()
                    .map(|state| state.enabled && state.design.active)
            })
            .unwrap_or(false)
    }

    /// The output design mode is up on, if any.
    pub fn tiling_design_output(&self) -> Option<Output> {
        self.workspaces
            .outputs()
            .find(|o| self.tiling_design_active(o))
            .cloned()
    }

    /// End design mode on any workspace that has scrolled away or stopped
    /// tiling. Called once per event-loop iteration beside the relayout flush;
    /// costs a flag read per workspace.
    pub fn flush_tiling_design(&mut self) {
        if !self.workspaces.tiling_design.is_active() {
            return;
        }
        // The grid is up; it stays up only while some workspace is both the
        // one on screen for its output and still editing. Leaving tiling mode
        // clears the design flag outright, and scrolling away leaves it set on
        // a workspace that is no longer current — both end here.
        let mut live = false;
        for ows in self.workspaces.output_workspaces.values() {
            for (index, view) in ows.workspace_views.iter().enumerate() {
                let Ok(state) = view.tiling.read() else {
                    continue;
                };
                if state.design.active && state.enabled && index == ows.current_workspace {
                    live = true;
                }
            }
        }
        if !live {
            self.tiling_design_leave();
            self.workspaces.tiling_design.hide();
        }
    }

    // ── Keeping the grid in step ─────────────────────────────────────────

    /// Rebuild the pane grid from the tree as it stands.
    ///
    /// Called from every relayout while design mode is active, so the panes
    /// and the windows are driven by one set of rects and animate together.
    pub fn refresh_tiling_design(
        &mut self,
        output: &Output,
        transition: Option<layers::prelude::Transition>,
    ) {
        if !self.tiling_design_active(output) {
            if self.workspaces.tiling_design.is_active() {
                self.workspaces.tiling_design.hide();
            }
            return;
        }
        let Some(geometry) = self.tiling_design_geometry(output) else {
            return;
        };
        let origin = self
            .workspaces
            .output_geometry(output)
            .map(|g| (g.loc.x, g.loc.y))
            .unwrap_or((0, 0));
        self.workspaces
            .tiling_design
            .update(geometry, origin, transition);
    }

    /// Resolve the workspace's tree into panes, bars, corners and — on an
    /// empty tree — the row of presets.
    fn tiling_design_geometry(&mut self, output: &Output) -> Option<DesignGeometry> {
        let workspace = self.workspaces.current_tiling_workspace(output)?;
        let zone = self.tiling_area(output);
        let gaps = Config::with(|c| c.tiling.gaps());
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);
        let scale = output.current_scale().fractional_scale() as f32;

        let state = workspace.tiling.read().ok()?;
        if state.tree.is_empty() {
            return Some(DesignGeometry {
                active: true,
                area,
                cells: Vec::new(),
                bars: Vec::new(),
                corners: Vec::new(),
                presets: crate::workspaces::tiling_design::preset_rects(area),
                scale,
            });
        }

        // A pane covers its cell plus half the inner gap on each side, so the
        // panes tile the tree's area exactly and the gaps read as the handles
        // they are — clamped to the area inside the outer gap so the outermost
        // panes do not spill.
        let bounds = inset(area, gaps.outer.min(area.w / 2).min(area.h / 2));
        let half = gaps.inner / 2;
        let focused = state.focused.clone();
        let focused_empty = state.design.focused_empty;
        let cells = layout::resolve_cells(&state.tree, area, gaps)
            .into_iter()
            .zip(cell_nodes_in_order(&state.tree))
            .map(|((cell, rect), node)| DesignCell {
                node,
                rect: clamp_to(grow(rect, half), bounds),
                empty: cell.is_empty_slot(),
                focused: match cell {
                    crate::workspaces::tiling::Cell::Window(ref id) => focused.as_ref() == Some(id),
                    crate::workspaces::tiling::Cell::Empty(slot) => focused_empty == Some(slot),
                },
            })
            .collect();

        let bars = design::bar_handles(&state.tree, area, gaps);
        let corners = design::corner_handles(&bars);
        Some(DesignGeometry {
            active: true,
            area,
            cells,
            bars,
            corners,
            presets: Vec::new(),
            scale,
        })
    }

    // ── The pointer ──────────────────────────────────────────────────────

    /// Motion over the grid: light up what is under the pointer, name it with
    /// a cursor, and move a drag in flight.
    pub fn tiling_design_motion(&mut self, x: f64, y: f64) {
        if self.tiling_design_drag_is_active() {
            self.tiling_design_drag_to(x, y);
            return;
        }
        let hit = self.workspaces.tiling_design.hit(x, y);
        self.workspaces.tiling_design.set_hover(hit.as_ref());
        let cursor = hit
            .as_ref()
            .map(crate::workspaces::TilingDesignView::cursor_for)
            .unwrap_or_default();
        self.set_cursor(&CursorImageStatus::Named(cursor));
    }

    /// A press or release on the grid. `double` marks the second click of a
    /// double click, which equalises the bar it lands on.
    pub fn tiling_design_button(&mut self, pressed: bool, double: bool, x: f64, y: f64) {
        if !pressed {
            self.tiling_design_drag_end();
            return;
        }
        let Some(hit) = self.workspaces.tiling_design.hit(x, y) else {
            return;
        };
        match hit {
            DesignHit::Bar(bar) => {
                if double {
                    self.tiling_design_equalize(bar.container);
                } else {
                    self.tiling_design_drag_begin(bar, None);
                }
            }
            DesignHit::Corner(corner) => {
                // A corner drives one split each way; the rest of the cluster
                // follows through `tiling_design_drag_to`.
                let along_x = corner.along_x.first().copied();
                let along_y = corner.along_y.first().copied();
                match (along_x, along_y) {
                    (Some(h), Some(v)) => self.tiling_design_drag_begin(h, Some(v)),
                    (Some(h), None) => self.tiling_design_drag_begin(h, None),
                    (None, Some(v)) => self.tiling_design_drag_begin(v, None),
                    (None, None) => {}
                }
            }
            DesignHit::Toolbar(node, action) => match action {
                CellAction::SplitHorizontal => self.tiling_design_split(node, Axis::Row),
                CellAction::SplitVertical => self.tiling_design_split(node, Axis::Column),
                CellAction::Close => self.tiling_design_close(node),
            },
            DesignHit::Preset(preset) => self.tiling_design_apply_preset(preset),
            DesignHit::Pane(node) => self.tiling_design_pane_pressed(node),
        }
    }

    /// A click on a pane's body. An empty slot takes the focus — it is where
    /// the next window lands — and a window's cell leaves design mode, which
    /// is the plain "click the window to get back to work" gesture.
    fn tiling_design_pane_pressed(&mut self, node: NodeId) {
        let Some(output) = self.tiling_design_output() else {
            return;
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return;
        };
        let cell = workspace
            .tiling
            .read()
            .ok()
            .and_then(|state| state.tree.cell(node));
        match cell {
            Some(crate::workspaces::tiling::Cell::Empty(slot)) => {
                if let Ok(mut state) = workspace.tiling.write() {
                    state.design.focused_empty = Some(slot);
                }
                self.relayout_workspace_with(&output, None, false);
            }
            Some(crate::workspaces::tiling::Cell::Window(id)) => {
                self.tiling_design_leave();
                if let Some(window) = self.workspaces.windows_map.get(&id).cloned() {
                    self.set_keyboard_focus_on_window(&window);
                }
            }
            None => {}
        }
    }

    // ── Dragging a handle ────────────────────────────────────────────────

    pub fn tiling_design_drag_is_active(&self) -> bool {
        self.tiling_design_output()
            .and_then(|output| self.workspaces.current_tiling_workspace(&output))
            .and_then(|view| view.tiling.read().ok().map(|s| s.design.drag.is_some()))
            .unwrap_or(false)
    }

    /// Start dragging `bar`, optionally with a second split for a corner.
    pub fn tiling_design_drag_begin(&mut self, bar: BarHandle, corner: Option<BarHandle>) {
        let Some(output) = self.tiling_design_output() else {
            return;
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return;
        };
        if let Ok(mut state) = workspace.tiling.write() {
            // One snapshot per drag, not per motion event: undo steps back to
            // where the bar was before the user grabbed it.
            let snapshot = state.tree.clone();
            state.design.undo.push(&snapshot);
            state.design.drag = Some(DesignDrag {
                bar: (bar.container, bar.index),
                corner: corner.map(|c| (c.container, c.index)),
            });
        }
        drop(workspace);
    }

    /// Move the drag to the pointer and relay the workspace out on the design
    /// spring, so the panes and the windows chase it.
    pub fn tiling_design_drag_to(&mut self, x: f64, y: f64) {
        let Some(output) = self.tiling_design_output() else {
            return;
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return;
        };
        let Some(drag) = workspace
            .tiling
            .read()
            .ok()
            .and_then(|state| state.design.drag)
        else {
            return;
        };
        // `Shift` bypasses the snap, as everywhere else in Otto.
        let snap = !self.current_modifiers.shift;
        let Some(geometry) = self.tiling_design_geometry(&output) else {
            return;
        };
        let find = |key: (NodeId, usize)| -> Option<BarHandle> {
            geometry
                .bars
                .iter()
                .find(|b| b.container == key.0 && b.index == key.1)
                .copied()
        };

        let mut label = None;
        let mut changed = false;
        if let Ok(mut state) = workspace.tiling.write() {
            for key in [Some(drag.bar), drag.corner].into_iter().flatten() {
                let Some(bar) = find(key) else {
                    continue;
                };
                let along = bar.pointer_along(x, y);
                let first = bar.share_at(along, snap);
                // Every split at the junction moves together, so a corner on a
                // grid drags the whole crossing rather than one row's edge.
                let siblings: Vec<BarHandle> = geometry
                    .bars
                    .iter()
                    .filter(|b| {
                        b.axis == bar.axis
                            && (b.container == bar.container && b.index == bar.index
                                || corner_partner(&geometry, &bar, b))
                    })
                    .copied()
                    .collect();
                for sibling in siblings {
                    let share = sibling.share_at(along, snap);
                    changed |= state
                        .tree
                        .set_pair_shares(sibling.container, sibling.index, share);
                }
                if label.is_none() {
                    let pair = bar.shares.0 + bar.shares.1;
                    label = Some((bar, first / pair.max(1e-6), (pair - first) / pair.max(1e-6)));
                }
            }
        }
        self.workspaces.tiling_design.set_drag_label(label);
        if changed {
            let transition = Config::with(|c| c.tiling.design_transition());
            self.relayout_workspace_with(&output, transition, false);
        }
    }

    /// Let go. The spring is already running; nothing else has to happen.
    pub fn tiling_design_drag_end(&mut self) {
        let Some(output) = self.tiling_design_output() else {
            return;
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return;
        };
        let had = match workspace.tiling.write() {
            Ok(mut state) => state.design.drag.take().is_some(),
            Err(_) => false,
        };
        if had {
            self.workspaces.tiling_design.set_drag_label(None);
            let transition = Config::with(|c| c.tiling.design_transition());
            self.relayout_workspace_with(&output, transition, false);
        }
    }

    // ── Structural edits ─────────────────────────────────────────────────

    /// Split the cell at `node`, the new half an empty slot.
    pub fn tiling_design_split(&mut self, node: NodeId, axis: Axis) {
        let Some((output, workspace)) = self.tiling_design_target() else {
            return;
        };
        let split = {
            let Ok(mut state) = workspace.tiling.write() else {
                return;
            };
            let snapshot = state.tree.clone();
            let slot = state.tree.split_cell(node, axis);
            if let Some(slot) = slot {
                state.design.undo.push(&snapshot);
                state.design.focused_empty = Some(slot);
            }
            slot.is_some()
        };
        if split {
            self.tiling_design_relayout(&output);
        }
    }

    /// Close a cell: an empty slot goes away, a window's cell closes the
    /// window and the removal path takes it out of the tree.
    pub fn tiling_design_close(&mut self, node: NodeId) {
        let Some((output, workspace)) = self.tiling_design_target() else {
            return;
        };
        let cell = workspace
            .tiling
            .read()
            .ok()
            .and_then(|state| state.tree.cell(node));
        match cell {
            Some(crate::workspaces::tiling::Cell::Empty(slot)) => {
                if let Ok(mut state) = workspace.tiling.write() {
                    let snapshot = state.tree.clone();
                    if state.tree.remove_empty(slot) {
                        state.design.undo.push(&snapshot);
                        if state.design.focused_empty == Some(slot) {
                            state.design.focused_empty = state.tree.empty_slots().first().copied();
                        }
                    }
                }
                self.tiling_design_relayout(&output);
            }
            Some(crate::workspaces::tiling::Cell::Window(id)) => {
                self.tiling_design_close_window(&id);
            }
            None => {}
        }
    }

    fn tiling_design_close_window(&mut self, id: &ObjectId) {
        let Some(window) = self.workspaces.windows_map.get(id).cloned() else {
            return;
        };
        // The same close every other path uses; the tree is updated when the
        // window actually goes away.
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
        }
    }

    /// Equalise the container a bar divides.
    pub fn tiling_design_equalize(&mut self, container: NodeId) {
        let Some((output, workspace)) = self.tiling_design_target() else {
            return;
        };
        if let Ok(mut state) = workspace.tiling.write() {
            let snapshot = state.tree.clone();
            state.design.undo.push(&snapshot);
            state.tree.equalize(container);
            state.design.drag = None;
        }
        self.tiling_design_relayout(&output);
    }

    /// Replace an empty workspace's tree with a preset's shape, in slots.
    pub fn tiling_design_apply_preset(&mut self, preset: Preset) {
        let Some((output, workspace)) = self.tiling_design_target() else {
            return;
        };
        if let Ok(mut state) = workspace.tiling.write() {
            let snapshot = state.tree.clone();
            state.design.undo.push(&snapshot);
            design::apply_preset(&mut state.tree, preset);
            state.design.focused_empty = state.tree.empty_slots().first().copied();
        }
        self.tiling_design_relayout(&output);
    }

    /// Pop the last design-mode edit and lay the workspace out again.
    pub(crate) fn handle_tiling_undo(&mut self) {
        let Some((output, workspace)) = self.tiling_design_target() else {
            return;
        };
        let restored = workspace
            .tiling
            .write()
            .map(|mut state| match state.design.undo.pop() {
                Some(tree) => {
                    state.tree = tree;
                    // Focus may have pointed at something the undone edit
                    // created; fall back to the first cell that is still there.
                    if let Some(focused) = state.focused.clone() {
                        if !state.tree.contains(&focused) {
                            state.focused = state.tree.leaves().first().cloned();
                        }
                    }
                    state.design.focused_empty = state.tree.empty_slots().first().copied();
                    true
                }
                None => false,
            })
            .unwrap_or(false);
        if restored {
            self.tiling_design_relayout(&output);
        }
    }

    fn tiling_design_relayout(&mut self, output: &Output) {
        let transition = Config::with(|c| c.tiling.design_transition());
        self.relayout_workspace_with(output, transition, false);
    }

    fn tiling_design_target(&mut self) -> Option<(Output, Arc<WorkspaceView>)> {
        let output = self.tiling_design_output()?;
        let workspace = self.workspaces.current_tiling_workspace(&output)?;
        Some((output, workspace))
    }
}

/// Is `other` part of the same junction as `bar` — a split of the same axis
/// that lines up with it, so a corner drag moves both?
fn corner_partner(geometry: &DesignGeometry, bar: &BarHandle, other: &BarHandle) -> bool {
    if bar.axis != other.axis {
        return false;
    }
    geometry.corners.iter().any(|corner| {
        let same = |list: &[BarHandle]| {
            list.iter()
                .any(|b| b.container == bar.container && b.index == bar.index)
                && list
                    .iter()
                    .any(|b| b.container == other.container && b.index == other.index)
        };
        same(&corner.along_x) || same(&corner.along_y)
    })
}

/// The cell node ids in the same order [`layout::resolve_cells`] returns them.
fn cell_nodes_in_order(
    tree: &crate::workspaces::tiling::tree::Tree<ObjectId>,
) -> Vec<crate::workspaces::tiling::tree::NodeId> {
    tree.cell_nodes()
}

fn grow(rect: Rect, by: i32) -> Rect {
    Rect::new(rect.x - by, rect.y - by, rect.w + 2 * by, rect.h + 2 * by)
}

fn inset(rect: Rect, by: i32) -> Rect {
    Rect::new(
        rect.x + by,
        rect.y + by,
        (rect.w - 2 * by).max(0),
        (rect.h - 2 * by).max(0),
    )
}

fn clamp_to(rect: Rect, bounds: Rect) -> Rect {
    let x = rect.x.max(bounds.x);
    let y = rect.y.max(bounds.y);
    let right = (rect.x + rect.w).min(bounds.x + bounds.w);
    let bottom = (rect.y + rect.h).min(bounds.y + bounds.h);
    Rect::new(x, y, (right - x).max(0), (bottom - y).max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_takes_half_the_gap_on_each_side_without_spilling() {
        let bounds = Rect::new(0, 0, 1000, 600);
        // A cell in the middle grows on all four sides.
        let middle = clamp_to(grow(Rect::new(300, 200, 200, 100), 4), bounds);
        assert_eq!(middle, Rect::new(296, 196, 208, 108));
        // One against the edge is clipped to the bounds instead.
        let edge = clamp_to(grow(Rect::new(0, 0, 200, 100), 4), bounds);
        assert_eq!(edge, Rect::new(0, 0, 204, 104));
    }
}
