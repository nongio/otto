//! The compositor side of tiling: turning a workspace's tree into window
//! rectangles, and the hooks that keep the tree in step with what maps,
//! unmaps and takes focus.
//!
//! The tree and the layout are pure and live in `src/workspaces/tiling/`;
//! nothing here decides *where* a window goes, only how it gets there. See
//! `specs/tiling.md` for the behaviour and `docs/developer/tiling-plan.md`
//! for the phases.

use std::sync::Arc;

use layers::prelude::Transition;
use smithay::{
    desktop::WindowSurface,
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::backend::ObjectId,
    },
    utils::{Logical, Rectangle, Size},
    wayland::{
        compositor::with_states,
        shell::xdg::{dialog::ToplevelDialogHint, XdgToplevelSurfaceData},
    },
};

use otto_kit::components::titlebar::DecorationVariant;

use crate::{
    config::Config,
    shell::WindowElement,
    state::{Backend, Otto},
    workspaces::{
        tiling::{floating, layout, Axis, Direction, Gaps, Layer, Rect},
        workspace::WorkspaceView,
        Workspaces,
    },
};

/// How close a cell's edge has to be to the usable area's for that edge to
/// count as "abutting the screen" rather than a neighbour — the outer gap,
/// plus a pixel of slack for rounding.
fn edge_tolerance(gaps: Gaps) -> i32 {
    gaps.outer + 1
}

impl Workspaces {
    /// Drop `id` from every tiling tree that holds it, and mark those
    /// workspaces for a relayout.
    ///
    /// Called from unmap, minimize and the workspace move: a window that
    /// leaves a tiled workspace is removed from its tree and its share goes
    /// back to its siblings (`specs/tiling.md`, *Removal*). The relayout
    /// itself needs the compositor, so it rides on the dirty flag and is
    /// picked up by [`Otto::flush_tiling_relayout`] on the next event-loop
    /// iteration.
    pub fn tiling_forget_window(&self, id: &ObjectId) {
        for ows in self.output_workspaces.values() {
            for view in ows.workspace_views.iter() {
                let Ok(mut state) = view.tiling.write() else {
                    continue;
                };
                if !state.tree.remove(id) {
                    continue;
                }
                // Out of the tree is back to a floating window's chrome. The
                // window may be gone already (an unmap), in which case this
                // is a no-op on a value nobody reads again.
                if let Some(window) = self.windows_map.get(id) {
                    window.set_decoration_variant(
                        otto_kit::components::titlebar::DecorationVariant::Floating,
                    );
                }
                state.dirty = true;
                if state.focused.as_ref() == Some(id) {
                    state.focused = state.tree.leaves().first().cloned();
                }
            }
        }
    }

    /// Does this output's current workspace tile?
    pub fn output_tiles(&self, output: &Output) -> bool {
        self.current_tiling_workspace(output)
            .map(|view| view.tiling.read().map(|s| s.enabled).unwrap_or(false))
            .unwrap_or(false)
    }

    /// The workspace view currently on screen for `output`.
    pub fn current_tiling_workspace(&self, output: &Output) -> Option<Arc<WorkspaceView>> {
        let ows = self.output_workspaces.get(&output.name())?;
        ows.workspace_views.get(ows.current_workspace).cloned()
    }
}

impl<BackendData: Backend> Otto<BackendData> {
    // ── Where a command applies ──────────────────────────────────────────

    /// The output a tiling command acts on: the focused window's, else the
    /// focused output.
    pub(crate) fn tiling_output(&self) -> Option<Output> {
        self.focused_window()
            .and_then(|w| self.workspaces.output_for_window(&w))
            .or_else(|| self.workspaces.focused_output().cloned())
    }

    /// The window holding keyboard focus, if any.
    pub(crate) fn focused_window(&self) -> Option<WindowElement> {
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|focus| match focus {
                crate::focus::KeyboardFocusTarget::Window(window) => Some(window),
                _ => None,
            })
    }

    /// The leaf a command acts relative to: the tree's remembered focus, or
    /// the keyboard-focused window when that is a leaf.
    fn tiling_focused_leaf(&self, workspace: &WorkspaceView) -> Option<ObjectId> {
        let state = workspace.tiling.read().ok()?;
        if let Some(focused) = state.focused.clone() {
            if state.tree.contains(&focused) {
                return Some(focused);
            }
        }
        let window = self.focused_window()?;
        let id = window.id();
        state.tree.contains(&id).then_some(id)
    }

    // ── Eligibility ──────────────────────────────────────────────────────

    /// Can this window join a tree at all?
    ///
    /// The same refusals `maximize_request` makes, plus the spec's automatic
    /// float rules: a window with a parent (a dialog), one pinned to a single
    /// size, a fullscreen or minimized one stays floating above the tiles and
    /// the layout ignores it.
    pub fn is_tileable(&self, window: &WindowElement) -> bool {
        let Some(toplevel) = window.toplevel() else {
            // XWayland tiles in a later phase; an X11 window floats for now.
            return false;
        };
        if toplevel.parent().is_some() {
            return false;
        }
        // `xdg-dialog-v1`: only a *modal* hint floats a parentless window.
        // GTK 4 gives every toplevel a dialog object, so the plain hint says
        // nothing about the window — a terminal carries it too. A file
        // chooser served by a separate process has no parent surface of its
        // own unless the application exported one, and asks for modal when it
        // could not import one (`specs/tiling.md`, *Automatic*).
        if dialog_hint(window) == ToplevelDialogHint::Modal {
            return false;
        }
        if !window.is_resizable() {
            return false;
        }
        if window.is_fullscreen() || window.is_minimised() {
            return false;
        }
        toplevel.with_pending_state(|state| {
            state
                .capabilities
                .contains(xdg_toplevel::WmCapabilities::Maximize)
        })
    }

    // ── Resolving and applying a layout ──────────────────────────────────

    /// The area a tree fills on `output`, in logical pixels, with fresh
    /// exclusive zones — the same rectangle a maximized window gets.
    pub(crate) fn tiling_area(&mut self, output: &Output) -> Rectangle<i32, Logical> {
        self.recalculate_exclusive_zones(output);
        self.usable_zone(output)
    }

    /// Resolve the current workspace's tree on `output` and move every window
    /// whose cell changed.
    ///
    /// `animate` false means no transition at all rather than a very short
    /// one: the window is placed at its cell and the client configured once
    /// (`[tiling] layout_duration = 0` takes the same path).
    pub fn relayout_workspace(&mut self, output: &Output, animate: bool) {
        self.relayout_workspace_forced(output, animate, false);
    }

    /// [`Self::relayout_workspace`], with the option to configure every leaf
    /// whether or not its cell moved.
    ///
    /// A decoration change moves no cell — the *client* rectangle inside it
    /// changes, by the height of the bar — so the "skip what did not move"
    /// rule would skip the whole workspace.
    pub fn relayout_workspace_forced(&mut self, output: &Output, animate: bool, force: bool) {
        let transition = if animate {
            Config::with(|c| c.tiling.layout_transition())
        } else {
            None
        };
        self.relayout_workspace_with(output, transition, force);
    }

    /// [`Self::relayout_workspace_forced`] with the transition chosen by the
    /// caller, so an interactive edge drag can use its own spring while
    /// everything else keeps the layout one.
    pub fn relayout_workspace_with(
        &mut self,
        output: &Output,
        transition: Option<Transition>,
        force: bool,
    ) {
        let Some(workspace) = self.workspaces.current_tiling_workspace(output) else {
            return;
        };
        {
            let Ok(mut state) = workspace.tiling.write() else {
                return;
            };
            state.dirty = false;
            if !state.enabled {
                return;
            }
        }

        let zone = self.tiling_area(output);
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);
        let (gaps, rects) = {
            let Ok(state) = workspace.tiling.read() else {
                return;
            };
            // This workspace's own gaps, if it has an override.
            let gaps = Config::with(|c| state.effective_gaps(&c.tiling));
            (gaps, layout::resolve(&state.tree, area, gaps))
        };

        if rects.is_empty() {
            return;
        }
        // A lone tile that fills the usable area outright — gaps off, or smart
        // gaps with one window — really is maximized, and is the one case the
        // client is told so (`specs/tiling.md`, *What clients are told*).
        let lone_maximized = rects.len() == 1
            && rects[0].1 == Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);

        let cells: Vec<Rect> = rects.iter().map(|(_, rect)| *rect).collect();
        for (id, rect) in rects.iter() {
            let Some(window) = self.workspaces.windows_map.get(id).cloned() else {
                continue;
            };
            let target =
                Rectangle::<i32, Logical>::new((rect.x, rect.y).into(), (rect.w, rect.h).into());
            if !force && self.workspaces.element_geometry(&window) == Some(target) {
                continue;
            }
            let edges = tiled_edges(*rect, &cells, zone, edge_tolerance(gaps));
            self.apply_tiled_rect(
                &window,
                output,
                target,
                edges,
                lone_maximized,
                transition.clone(),
            );
        }

        // `map_window_on_output` restacked the workspace as it placed each
        // tile; the floating layer goes back on top of them all.
        self.restack_floating_above_tiles(output);
    }

    /// Animate one window into `target` and tell the client about it.
    ///
    /// This is [`Otto::apply_tile`]'s per-window body for an arbitrary
    /// rectangle and an arbitrary set of tiled edges. It is duplicated rather
    /// than shared because `apply_tile` is keyed off `TileZone`;
    /// TODO: unify the two once the half-snap path is folded into the tree.
    #[allow(clippy::too_many_arguments)]
    fn apply_tiled_rect(
        &mut self,
        window: &WindowElement,
        output: &Output,
        target: Rectangle<i32, Logical>,
        edges: (bool, bool, bool, bool),
        maximize: bool,
        transition: Option<Transition>,
    ) {
        let Some(current_geometry) = self.workspaces.element_geometry(window) else {
            return;
        };

        // A window in the tree owns no floating zone: the tree is what places
        // it, so the half-snap marker must not linger and fight the layout.
        let id = window.id();
        if let Some(mut view) = self.workspaces.get_window_view(&id) {
            if view.tiled_zone.is_some() {
                view.tiled_zone = None;
                self.workspaces.set_window_view(&id, view);
            }
        }

        match window.underlying_surface() {
            WindowSurface::Wayland(_) => {}
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(_) => return,
            #[cfg(not(feature = "xwayland"))]
            _ => return,
        }
        let Some(toplevel) = window.toplevel().cloned() else {
            return;
        };

        // Same reason as `maximize_request`: the state lands on the
        // animation's last frame, this flag has to be true now.
        window.set_is_maximized(maximize);
        // And the same for the decoration: the bar's height is subtracted
        // from every configure below, so the window has to be wearing the
        // tile's variant before the first one goes out.
        window.set_decoration_variant(DecorationVariant::tiled(Config::with(|c| {
            c.tiling.decoration()
        })));
        let (left, right, top, bottom) = edges;

        match transition {
            Some(ref transition) => {
                let animation = self
                    .layers_engine
                    .add_animation_from_transition(transition, false);

                // Both rects are decorated (space) rects; the client is
                // configured without the titlebar Otto draws on top of it —
                // stripped per frame by `animated_client_size`.
                let current_size = Size::<i32, Logical>::from((
                    current_geometry.size.w.max(1),
                    current_geometry.size.h.max(1),
                ));
                let new_size = target.size;

                let s = toplevel.clone();
                let w = window.clone();
                self.layers_engine.on_animation_update(
                    animation,
                    move |p: f32| {
                        let size = super::xdg::animated_client_size(&w, current_size, new_size, p);
                        s.with_pending_state(|state| {
                            if (p - 1.0).abs() < f32::EPSILON {
                                set_tiled_states(state, left, right, top, bottom, maximize);
                            }
                            state.size = Some(size);
                        });
                        s.send_configure();
                    },
                    false,
                );
                self.layers_engine.start_animation(animation, 0.0);
            }
            None => {
                let size = window.client_size(target.size);
                toplevel.with_pending_state(|state| {
                    set_tiled_states(state, left, right, top, bottom, maximize);
                    state.size = Some(size);
                });
                toplevel.send_configure();
            }
        }

        // Pin the destination output: `target` came from this output's usable
        // zone, and the window may still have its pre-layout size here.
        // `activate` is false — a relayout moves everything at once and must
        // not restack the workspace.
        self.workspaces
            .map_window_on_output(output, window, target.loc, false, transition);

        // The window sits at a new rect now, so its menus have to be placed
        // against it again.
        self.reposition_popups_for_window(window);
    }

    /// Relayout every output whose current workspace was left dirty by an
    /// unmap, a minimize or a workspace move. Called once per event-loop
    /// iteration; costs a flag read per output when nothing changed.
    pub fn flush_tiling_relayout(&mut self) {
        let dirty: Vec<Output> = self
            .workspaces
            .outputs()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .filter(|output| {
                self.workspaces
                    .current_tiling_workspace(output)
                    .and_then(|view| view.tiling.read().ok().map(|s| s.enabled && s.dirty))
                    .unwrap_or(false)
            })
            .collect();
        for output in dirty {
            self.tiling_readmit_returning(&output);
            self.relayout_workspace(&output, true);
        }
    }

    /// Insert the windows unminimize queued on `output`'s current workspace.
    ///
    /// Each one joins next to the focused leaf, like a window that was just
    /// mapped; one that is no longer tileable — it went fullscreen or was
    /// closed in the meantime — is dropped and stays floating.
    fn tiling_readmit_returning(&mut self, output: &Output) {
        let Some(workspace) = self.workspaces.current_tiling_workspace(output) else {
            return;
        };
        let returning = match workspace.tiling.write() {
            Ok(mut state) => std::mem::take(&mut state.returning),
            Err(_) => return,
        };
        for id in returning {
            let Some(window) = self.workspaces.windows_map.get(&id).cloned() else {
                continue;
            };
            let known = workspace
                .tiling
                .read()
                .map(|s| s.tree.contains(&id))
                .unwrap_or(true);
            if known || !self.is_tileable(&window) {
                continue;
            }
            self.tiling_insert_leaf(output, &workspace, id);
        }
    }

    // ── Entering and leaving tiling mode ─────────────────────────────────

    /// Put `output`'s current workspace into, or out of, tiling mode.
    ///
    /// Entering, every eligible window joins the tree most-recently-focused
    /// first and its current rect is remembered as its floating one; leaving,
    /// each one animates back to that rect with the tiled states dropped.
    pub fn set_workspace_tiling(&mut self, output: &Output, enabled: bool) {
        let Some(workspace) = self.workspaces.current_tiling_workspace(output) else {
            return;
        };
        let already = workspace.tiling.read().map(|s| s.enabled).unwrap_or(false);
        if already == enabled {
            return;
        }
        // The mode is persisted with the workspace's name and gaps, so a
        // workspace the user tiles is still tiled at the next login. Written
        // after the state has changed, from the state — see
        // `Workspaces::persist_workspace_entry`.
        let record = self
            .workspaces
            .output_workspaces
            .get(&output.name())
            .map(|ows| (output.name(), ows.current_workspace));

        if !enabled {
            let leaves = {
                let Ok(mut state) = workspace.tiling.write() else {
                    return;
                };
                let leaves = state.tree.leaves();
                state.clear();
                state.enabled = false;
                leaves
            };
            if let Some((output, position)) = record.clone() {
                self.workspaces.persist_workspace_entry(&output, position);
            }
            for id in leaves {
                let Some(window) = self.workspaces.windows_map.get(&id).cloned() else {
                    continue;
                };
                window.set_is_maximized(false);
                // Before the restore, for the same reason `apply_tiled_rect`
                // sets it before the first configure: the floating rect is
                // configured minus the *floating* bar.
                window.set_decoration_variant(DecorationVariant::Floating);
                self.restore_to_floating(&window);
            }
            return;
        }

        {
            let Ok(mut state) = workspace.tiling.write() else {
                return;
            };
            state.clear();
            state.enabled = true;
        }
        if let Some((output, position)) = record {
            self.workspaces.persist_workspace_entry(&output, position);
        }

        // `windows_list` is bottom-to-top, so walking it backwards gives the
        // stacking order most recently focused first, which is the order the
        // spec asks the tree to be built in.
        let ids: Vec<ObjectId> = workspace
            .windows_list
            .read()
            .map(|list| list.iter().rev().cloned().collect())
            .unwrap_or_default();

        for id in ids {
            let Some(window) = self.workspaces.windows_map.get(&id).cloned() else {
                continue;
            };
            if !self.is_tileable(&window) {
                continue;
            }
            // The rect it had before it was tiled is what leaving tiling mode
            // (and floating it by hand, later) restores it to.
            if let Some(geometry) = self.workspaces.element_geometry(&window) {
                if let Some(mut view) = self.workspaces.get_window_view(&id) {
                    view.unmaximised_rect = geometry;
                    view.tiled_zone = None;
                    self.workspaces.set_window_view(&id, view);
                }
            }
            self.tiling_insert_leaf(output, &workspace, id);
        }

        // Building the tree walked every window, so the tree's idea of focus
        // is the last one inserted. Hand it back to the window the user was
        // actually on, so the next insertion splits *that* cell.
        if let Some(focused) = self.focused_window().map(|w| w.id()) {
            if let Ok(mut state) = workspace.tiling.write() {
                if state.tree.contains(&focused) {
                    state.focused = Some(focused);
                }
            }
        }

        // Entering tiling mode moves the whole workspace at once rather than
        // one cell, so it rides its own spring (`[tiling] mode_duration`).
        let transition = Config::with(|c| c.tiling.mode_transition());
        self.relayout_workspace_with(output, transition, false);
    }

    /// Insert `id` into `workspace`'s tree next to whatever is focused there.
    fn tiling_insert_leaf(
        &mut self,
        output: &Output,
        workspace: &Arc<WorkspaceView>,
        id: ObjectId,
    ) {
        let zone = self.tiling_area(output);
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);

        let Ok(mut state) = workspace.tiling.write() else {
            return;
        };
        let gaps = Config::with(|c| state.effective_gaps(&c.tiling));
        let focused = state
            .focused
            .clone()
            .filter(|focused| state.tree.contains(focused));
        // Which way the focused cell splits follows its shape: a cell wider
        // than it is tall becomes a row, a taller one a column.
        let cell_is_wide = match focused.as_ref() {
            Some(focused) => layout::resolve(&state.tree, area, gaps)
                .iter()
                .find(|(leaf, _)| leaf == focused)
                .map(|(_, rect)| rect.is_wide())
                .unwrap_or(true),
            None => true,
        };
        let preselect = state.take_preselect();
        state
            .tree
            .insert_next_to(focused.as_ref(), id.clone(), preselect, cell_is_wide);
        state.focused = Some(id);
    }

    // ── Hooks into the rest of the compositor ────────────────────────────

    /// A freshly mapped window on a tiling workspace joins the tree.
    ///
    /// Returns true when the window belongs to a tree, so the caller skips
    /// the floating cascade placement — including for a window that is
    /// already a leaf, which a later commit must not re-place.
    pub fn tiling_adopt_window(&mut self, window: &WindowElement) -> bool {
        let Some(output) = self.workspaces.output_for_window(window) else {
            return false;
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return false;
        };
        let id = window.id();
        let (enabled, known) = {
            let Ok(state) = workspace.tiling.read() else {
                return false;
            };
            (state.enabled, state.tree.contains(&id))
        };
        if !enabled {
            return false;
        }
        if known {
            return true;
        }
        if !self.is_tileable(window) {
            return false;
        }

        self.tiling_insert_leaf(&output, &workspace, id);
        self.relayout_workspace(&output, true);
        true
    }

    /// Keyboard focus landed on `id`; if it is a leaf, the tree's commands
    /// act relative to it from now on.
    pub fn tiling_note_focus(&mut self, id: &ObjectId) {
        for ows in self.workspaces.output_workspaces.values() {
            for view in ows.workspace_views.iter() {
                let here = view
                    .windows_list
                    .read()
                    .map(|list| list.iter().any(|w| w == id))
                    .unwrap_or(false);
                let Ok(mut state) = view.tiling.write() else {
                    continue;
                };
                if !state.enabled {
                    continue;
                }
                if state.tree.contains(id) {
                    state.focused = Some(id.clone());
                } else if here {
                    // The other half of the same memory: `focus mode_toggle`
                    // comes back to whichever floating window was last on.
                    state.floating_focused = Some(id.clone());
                }
            }
        }
    }

    // ── Shortcut handlers ────────────────────────────────────────────────

    /// Drop every workspace's own gaps, so they all follow `[tiling]` again.
    ///
    /// The Settings sliders and `gaps <n> all` are the same instrument — the
    /// session default — and this is the half of that command which is not
    /// the number itself. Without it a workspace that was once given its own
    /// gaps keeps them for ever, and the slider does nothing at all on the
    /// very workspace the user is looking at while they drag it.
    pub fn clear_workspace_gap_overrides(&mut self) {
        for ows in self.workspaces.output_workspaces.values() {
            for view in ows.workspace_views.iter() {
                if let Ok(mut state) = view.tiling.write() {
                    state.gaps = None;
                }
            }
        }
        self.workspaces.persist_all_workspace_entries();
    }

    /// Re-dress every tile after `[tiling] decoration` changed.
    ///
    /// The bar's height is part of what the client is configured with, so a
    /// change of variant is a change of geometry: every tiling workspace is
    /// laid out again, which reconfigures each leaf against the new bar.
    /// Lay out every output's tiling workspace again.
    ///
    /// For a change to what the *layout* is resolved from rather than to the
    /// tree — the gaps — where no cell need have moved but every rectangle
    /// may have, so the leaves are configured whether or not they did.
    pub fn relayout_tiling_workspaces(&mut self) {
        let outputs: Vec<Output> = self.workspaces.outputs().cloned().collect();
        for output in outputs {
            self.relayout_workspace_forced(&output, true, true);
        }
    }

    pub fn refresh_tiling_decorations(&mut self) {
        let variant = DecorationVariant::tiled(Config::with(|c| c.tiling.decoration()));
        let mut leaves: Vec<ObjectId> = Vec::new();
        for ows in self.workspaces.output_workspaces.values() {
            for view in ows.workspace_views.iter() {
                let Ok(state) = view.tiling.read() else {
                    continue;
                };
                if state.enabled {
                    leaves.extend(state.tree.leaves());
                }
            }
        }
        for id in leaves {
            let Some(window) = self.workspaces.windows_map.get(&id).cloned() else {
                continue;
            };
            window.set_decoration_variant(variant);
        }
        let outputs: Vec<Output> = self.workspaces.outputs().cloned().collect();
        for output in outputs {
            self.relayout_workspace_forced(&output, false, true);
        }
    }

    // ── The floating layer ───────────────────────────────────────────────

    /// The windows on `workspace` that are not in its tree, bottom to top.
    ///
    /// Everything on a tiling workspace is either a leaf or a floating
    /// window; there is no third thing (`specs/tiling.md`, *Stacking*).
    fn tiling_floating_ids(&self, workspace: &Arc<WorkspaceView>) -> Vec<ObjectId> {
        let Ok(state) = workspace.tiling.read() else {
            return Vec::new();
        };
        if !state.enabled {
            return Vec::new();
        }
        workspace
            .windows_list
            .read()
            .map(|list| {
                list.iter()
                    .filter(|id| !state.tree.contains(id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Put the floating layer back on top of the tiles.
    ///
    /// A tile that takes focus, or a relayout that restacks the workspace,
    /// would otherwise leave a floating window underneath one — and a tile
    /// must never overlap it. Raising in `windows_list` order keeps the
    /// floating windows' own stacking, so only their relation to the tiles
    /// changes.
    pub fn restack_floating_above_tiles(&mut self, output: &Output) {
        let Some(workspace) = self.workspaces.current_tiling_workspace(output) else {
            return;
        };
        let floating: Vec<ObjectId> = self
            .tiling_floating_ids(&workspace)
            .into_iter()
            .filter(|id| {
                self.workspaces
                    .windows_map
                    .get(id)
                    .map(|window| !window.is_minimised())
                    .unwrap_or(false)
            })
            .collect();
        if floating.is_empty() {
            return;
        }
        // Nearly every relayout finds the floating layer already on top;
        // checking costs a walk of the stack and saves a workspace-model
        // rebuild. Both orders have to be looked at: the *space* is what
        // decides which window is topmost, and the workspace's own
        // `windows_list` is what decides the order the layers draw in, and a
        // relayout can move one without the other.
        let in_space = self
            .workspaces
            .output_workspaces
            .get(&output.name())
            .and_then(|ows| ows.spaces.get(ows.current_workspace))
            .map(|space| {
                space
                    .elements()
                    .rev()
                    .filter(|window| !window.is_minimised())
                    .take(floating.len())
                    .all(|window| floating.contains(&window.id()))
            })
            .unwrap_or(false);
        let in_list = workspace
            .windows_list
            .read()
            .map(|list| {
                list.iter()
                    .rev()
                    .filter(|id| {
                        self.workspaces
                            .windows_map
                            .get(*id)
                            .map(|window| !window.is_minimised())
                            .unwrap_or(false)
                    })
                    .take(floating.len())
                    .all(|id| floating.contains(id))
            })
            .unwrap_or(false);
        if in_space && in_list {
            return;
        }
        for id in floating {
            // `activate` false: this is a restack, not a focus change.
            self.workspaces.raise_element(&id, false, false);
        }
        self.workspaces.update_workspace_model();
    }

    /// Which layer `id` is on, on `workspace`.
    fn tiling_layer_of(&self, workspace: &Arc<WorkspaceView>, id: &ObjectId) -> Option<Layer> {
        let on_workspace = workspace
            .windows_list
            .read()
            .map(|list| list.iter().any(|w| w == id))
            .unwrap_or(false);
        if !on_workspace {
            return None;
        }
        let tiled = workspace
            .tiling
            .read()
            .map(|state| state.tree.contains(id))
            .unwrap_or(false);
        Some(if tiled { Layer::Tiled } else { Layer::Floating })
    }

    /// `floating toggle|enable|disable`, and the `FloatingToggle` action.
    ///
    /// `want` is what the window should end up as; `None` flips it. A window
    /// taken out of the tree goes back to the rectangle it had before it was
    /// tiled, or — having never had one — to a fraction of the workspace
    /// centred on the cell it is leaving. One put back in joins at the
    /// focused position, not the one it left (`specs/tiling.md`, *By hand*
    /// and *Restoring*).
    pub(crate) fn handle_tiling_floating(&mut self, want: Option<bool>) -> Result<(), String> {
        let Some(output) = self.tiling_output() else {
            return Err("no output has focus".to_string());
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return Err("no workspace has focus".to_string());
        };
        if !workspace
            .tiling
            .read()
            .map(|state| state.enabled)
            .unwrap_or(false)
        {
            return Err("this workspace does not tile".to_string());
        }
        let Some(window) = self.focused_window() else {
            return Err("no window has focus".to_string());
        };
        let id = window.id();
        let Some(layer) = self.tiling_layer_of(&workspace, &id) else {
            return Err("that window is not on this workspace".to_string());
        };
        let float = want.unwrap_or(layer == Layer::Tiled);
        match (layer, float) {
            (Layer::Tiled, true) => self.tiling_float_leaf(&output, &workspace, &window),
            (Layer::Floating, false) => self.tiling_tile_floating(&output, &workspace, &window),
            (Layer::Floating, true) => Err("that window already floats".to_string()),
            (Layer::Tiled, false) => Err("that window is already tiled".to_string()),
        }
    }

    /// Take `window` out of the tree and restore it to its floating rect.
    fn tiling_float_leaf(
        &mut self,
        output: &Output,
        workspace: &Arc<WorkspaceView>,
        window: &WindowElement,
    ) -> Result<(), String> {
        let id = window.id();
        let zone = self.tiling_area(output);
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);

        // The cell it is leaving, read before the removal: it is what a
        // window with no remembered rect is centred on.
        let cell = {
            let Ok(state) = workspace.tiling.read() else {
                return Err("the tiling state is busy".to_string());
            };
            let gaps = Config::with(|c| state.effective_gaps(&c.tiling));
            layout::resolve(&state.tree, area, gaps)
                .into_iter()
                .find(|(leaf, _)| *leaf == id)
                .map(|(_, rect)| rect)
                .unwrap_or(area)
        };

        {
            let Ok(mut state) = workspace.tiling.write() else {
                return Err("the tiling state is busy".to_string());
            };
            if !state.tree.remove(&id) {
                return Err("that window is not a tile".to_string());
            }
            if state.focused.as_ref() == Some(&id) {
                state.focused = state.tree.leaves().first().cloned();
            }
            state.floating_focused = Some(id.clone());
        }

        // A window opened straight into the tree has no floating rect to go
        // back to, so it is given one here rather than in `restore_to_floating`
        // — which would centre it on the output and lose the place on screen
        // the user is looking at.
        if let Some(mut view) = self.workspaces.get_window_view(&id) {
            if view.unmaximised_rect.size.is_empty() {
                let rect = floating::default_float_rect(area, cell);
                view.unmaximised_rect = Rectangle::<i32, Logical>::new(
                    (rect.x, rect.y).into(),
                    (rect.w, rect.h).into(),
                );
            }
            view.tiled_zone = None;
            self.workspaces.set_window_view(&id, view);
        }

        window.set_is_maximized(false);
        // Before the restore, for the same reason `apply_tiled_rect` sets the
        // tile's variant before its first configure: the rect the client is
        // configured with is this one minus the *floating* bar.
        window.set_decoration_variant(DecorationVariant::Floating);
        self.restore_to_floating(window);
        self.relayout_workspace(output, true);
        self.restack_floating_above_tiles(output);
        Ok(())
    }

    /// Put a floating `window` back into the tree, at the focused position.
    fn tiling_tile_floating(
        &mut self,
        output: &Output,
        workspace: &Arc<WorkspaceView>,
        window: &WindowElement,
    ) -> Result<(), String> {
        if !self.is_tileable(window) {
            return Err("that window cannot be tiled".to_string());
        }
        let id = window.id();
        // What it looks like now is what floating it again restores.
        if let Some(geometry) = self.workspaces.element_geometry(window) {
            if let Some(mut view) = self.workspaces.get_window_view(&id) {
                view.unmaximised_rect = geometry;
                view.tiled_zone = None;
                self.workspaces.set_window_view(&id, view);
            }
        }
        if let Ok(mut state) = workspace.tiling.write() {
            if state.floating_focused.as_ref() == Some(&id) {
                state.floating_focused = None;
            }
        }
        self.tiling_insert_leaf(output, workspace, id);
        // Forced: the window may already be sitting on its new cell, and it
        // still has to be dressed as a tile and told it is one.
        self.relayout_workspace_forced(output, true, true);
        self.restack_floating_above_tiles(output);
        Ok(())
    }

    /// `focus mode_toggle|floating|tiling`, and the `FocusModeToggle` action:
    /// move keyboard focus to the other layer's most recently focused window
    /// (`specs/tiling.md`, *Stacking*).
    pub(crate) fn handle_tiling_focus_mode(&mut self, target: Option<Layer>) -> Result<(), String> {
        let Some(output) = self.tiling_output() else {
            return Err("no output has focus".to_string());
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return Err("no workspace has focus".to_string());
        };
        if !workspace
            .tiling
            .read()
            .map(|state| state.enabled)
            .unwrap_or(false)
        {
            return Err("this workspace does not tile".to_string());
        }
        let here = self
            .focused_window()
            .and_then(|window| self.tiling_layer_of(&workspace, &window.id()));
        let target = match target {
            Some(target) => target,
            // With focus nowhere on this workspace, "the other layer" is the
            // tiled one: that is where a tiling workspace's windows are.
            None => here.unwrap_or(Layer::Floating).other(),
        };
        let Some(next) = self.tiling_layer_focus_candidate(&workspace, target) else {
            return Err(match target {
                Layer::Floating => "nothing floats on this workspace".to_string(),
                Layer::Tiled => "nothing is tiled on this workspace".to_string(),
            });
        };
        let Some(window) = self.workspaces.windows_map.get(&next).cloned() else {
            return Err("that window has gone".to_string());
        };
        self.workspaces.raise_element(&next, true, true);
        self.set_keyboard_focus_on_window(&window);
        if let Ok(mut state) = workspace.tiling.write() {
            match target {
                Layer::Tiled => state.focus_leaf(next),
                Layer::Floating => state.floating_focused = Some(next),
            }
        }
        // Raising a tile just put it over the floating layer.
        if target == Layer::Tiled {
            self.restack_floating_above_tiles(&output);
        }
        Ok(())
    }

    /// The window `focus mode_toggle` lands on in `layer`: the one that held
    /// focus there last, else the topmost window on it.
    fn tiling_layer_focus_candidate(
        &self,
        workspace: &Arc<WorkspaceView>,
        layer: Layer,
    ) -> Option<ObjectId> {
        let remembered = {
            let state = workspace.tiling.read().ok()?;
            match layer {
                Layer::Tiled => state.focused.clone().filter(|id| state.tree.contains(id)),
                Layer::Floating => state
                    .floating_focused
                    .clone()
                    .filter(|id| !state.tree.contains(id)),
            }
        };
        let usable = |id: &ObjectId| {
            self.workspaces
                .windows_map
                .get(id)
                .map(|window| !window.is_minimised())
                .unwrap_or(false)
        };
        if let Some(id) = remembered.filter(|id| usable(id)) {
            return Some(id);
        }
        let candidates: Vec<ObjectId> = match layer {
            Layer::Floating => self.tiling_floating_ids(workspace),
            Layer::Tiled => {
                let state = workspace.tiling.read().ok()?;
                workspace
                    .windows_list
                    .read()
                    .map(|list| {
                        list.iter()
                            .filter(|id| state.tree.contains(id))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default()
            }
        };
        // `windows_list` is bottom to top, so the topmost usable window is
        // the last one that is not in the dock.
        candidates.into_iter().rfind(|id| usable(id))
    }

    pub(crate) fn handle_tiling_toggle(&mut self) {
        let Some(output) = self.tiling_output() else {
            return;
        };
        let enabled = self.workspaces.output_tiles(&output);
        self.set_workspace_tiling(&output, !enabled);
    }

    pub(crate) fn handle_tiling_focus(&mut self, direction: Direction) {
        let Some(output) = self.tiling_output() else {
            return;
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return;
        };
        let Some(from) = self.tiling_focused_leaf(&workspace) else {
            return;
        };
        let zone = self.tiling_area(&output);
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);
        let next = {
            let Ok(state) = workspace.tiling.read() else {
                return;
            };
            if !state.enabled {
                return;
            }
            let gaps = Config::with(|c| state.effective_gaps(&c.tiling));
            let rects = layout::resolve(&state.tree, area, gaps);
            layout::neighbour(&rects, &from, direction)
        };
        let Some(next) = next else {
            // Focus never wraps inside a workspace.
            return;
        };
        let Some(window) = self.workspaces.windows_map.get(&next).cloned() else {
            return;
        };
        self.workspaces.raise_element(&window.id(), true, true);
        self.set_keyboard_focus_on_window(&window);
        if let Ok(mut state) = workspace.tiling.write() {
            state.focused = Some(next);
        }
        drop(workspace);
        // The raise just put a tile over the floating layer.
        self.restack_floating_above_tiles(&output);
    }

    pub(crate) fn handle_tiling_move(&mut self, direction: Direction) {
        let Some((output, workspace, leaf)) = self.tiling_target() else {
            return;
        };
        let moved = workspace
            .tiling
            .write()
            .map(|mut state| state.tree.move_dir(&leaf, direction))
            .unwrap_or(false);
        if moved {
            self.relayout_workspace(&output, true);
        }
    }

    pub(crate) fn handle_tiling_split(&mut self, axis: Axis) {
        let Some((_, workspace, _)) = self.tiling_target() else {
            return;
        };
        if let Ok(mut state) = workspace.tiling.write() {
            state.set_preselect(axis);
        }
        drop(workspace);
    }

    pub(crate) fn handle_tiling_resize(&mut self, axis: Axis, grow: bool) {
        let Some((output, workspace, leaf)) = self.tiling_target() else {
            return;
        };
        let step = Config::with(|c| c.tiling.step());
        let delta = if grow { step } else { -step };
        let resized = workspace
            .tiling
            .write()
            .map(|mut state| state.tree.resize(&leaf, axis, delta))
            .unwrap_or(false);
        if resized {
            self.relayout_workspace(&output, true);
        }
    }

    pub(crate) fn handle_tiling_equalize(&mut self) {
        let Some((output, workspace, leaf)) = self.tiling_target() else {
            return;
        };
        let changed = workspace
            .tiling
            .write()
            .map(|mut state| state.tree.equalize_container_of(&leaf))
            .unwrap_or(false);
        if changed {
            self.relayout_workspace(&output, true);
        }
    }

    /// The output, workspace and focused leaf a tree-editing command needs,
    /// or `None` when the focused workspace does not tile.
    fn tiling_target(&mut self) -> Option<(Output, Arc<WorkspaceView>, ObjectId)> {
        let output = self.tiling_output()?;
        let workspace = self.workspaces.current_tiling_workspace(&output)?;
        if !workspace.tiling.read().ok()?.enabled {
            return None;
        }
        let leaf = self.tiling_focused_leaf(&workspace)?;
        Some((output, workspace, leaf))
    }
}

/// What `xdg-dialog-v1` says about this window, or `Unknown` when the client
/// never said anything.
fn dialog_hint(window: &WindowElement) -> ToplevelDialogHint {
    let Some(toplevel) = window.toplevel() else {
        return ToplevelDialogHint::Unknown;
    };
    with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok())
            .map(|attributes| attributes.dialog_hint)
            .unwrap_or_default()
    })
}

/// Replace whatever maximized/tiled flags a client was told with this cell's.
///
/// A tiled window is never told it is maximized unless it is the one case
/// where its rectangle really is the whole usable area — a lone tile with the
/// gaps off (`specs/tiling.md`, *What clients are told*).
fn set_tiled_states(
    state: &mut smithay::wayland::shell::xdg::ToplevelState,
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
    maximize: bool,
) {
    state.states.unset(xdg_toplevel::State::Maximized);
    state.states.unset(xdg_toplevel::State::TiledLeft);
    state.states.unset(xdg_toplevel::State::TiledRight);
    state.states.unset(xdg_toplevel::State::TiledTop);
    state.states.unset(xdg_toplevel::State::TiledBottom);
    if maximize {
        state.states.set(xdg_toplevel::State::Maximized);
        return;
    }
    if left {
        state.states.set(xdg_toplevel::State::TiledLeft);
    }
    if right {
        state.states.set(xdg_toplevel::State::TiledRight);
    }
    if top {
        state.states.set(xdg_toplevel::State::TiledTop);
    }
    if bottom {
        state.states.set(xdg_toplevel::State::TiledBottom);
    }
}

/// Which of a cell's edges abut another tile or the edge of the usable area.
///
/// Returned as `(left, right, top, bottom)`; a client squares off the
/// corresponding corners (`specs/tiling.md`, *What clients are told*).
fn tiled_edges(
    rect: Rect,
    others: &[Rect],
    zone: Rectangle<i32, Logical>,
    tolerance: i32,
) -> (bool, bool, bool, bool) {
    let zone_left = zone.loc.x;
    let zone_right = zone.loc.x + zone.size.w;
    let zone_top = zone.loc.y;
    let zone_bottom = zone.loc.y + zone.size.h;

    let mut left = rect.x - zone_left <= tolerance;
    let mut right = zone_right - (rect.x + rect.w) <= tolerance;
    let mut top = rect.y - zone_top <= tolerance;
    let mut bottom = zone_bottom - (rect.y + rect.h) <= tolerance;

    for other in others {
        if *other == rect {
            continue;
        }
        let rows_overlap = other.y < rect.y + rect.h && rect.y < other.y + other.h;
        let cols_overlap = other.x < rect.x + rect.w && rect.x < other.x + other.w;
        if rows_overlap && other.x + other.w <= rect.x {
            left = true;
        }
        if rows_overlap && other.x >= rect.x + rect.w {
            right = true;
        }
        if cols_overlap && other.y + other.h <= rect.y {
            top = true;
        }
        if cols_overlap && other.y >= rect.y + rect.h {
            bottom = true;
        }
    }
    (left, right, top, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> Rectangle<i32, Logical> {
        Rectangle::new((0, 30).into(), (1000, 700).into())
    }

    #[test]
    fn a_lone_tile_abuts_the_usable_area_on_every_side() {
        let rect = Rect::new(8, 38, 984, 684);
        assert_eq!(
            tiled_edges(rect, &[rect], zone(), 9),
            (true, true, true, true)
        );
    }

    #[test]
    fn a_middle_column_abuts_its_neighbours_left_and_right() {
        let left = Rect::new(0, 30, 300, 700);
        let middle = Rect::new(300, 30, 400, 700);
        let right = Rect::new(700, 30, 300, 700);
        let cells = [left, middle, right];
        // Gaps off: every edge of the middle cell touches something.
        assert_eq!(
            tiled_edges(middle, &cells, zone(), 0),
            (true, true, true, true)
        );
        // The outer cells touch the screen on their outer edge and the middle
        // one on the other.
        assert_eq!(
            tiled_edges(left, &cells, zone(), 0),
            (true, true, true, true)
        );
    }

    #[test]
    fn a_cell_with_nothing_beyond_it_is_not_tiled_on_that_edge() {
        // A single narrow cell parked in the middle of a much larger zone
        // touches nothing at all.
        let rect = Rect::new(400, 300, 100, 100);
        assert_eq!(
            tiled_edges(rect, &[rect], zone(), 0),
            (false, false, false, false)
        );
    }
}
