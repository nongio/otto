//! Running the i3 command language, and answering `GetTree`.
//!
//! [`Otto::run_command`] parses a command string with
//! [`crate::workspaces::tiling::command`] and executes each command through
//! the same handlers the keybindings land on, so a script and a keystroke
//! take one path. The JSON getters answer in i3's shapes, so `i3-msg`-era
//! tooling — waybar, `jq` one-liners, `autotiling` — reads them unchanged.
//!
//! The wire contract is `docs/developer/shell-dbus-api.md`; the grammar is
//! `docs/developer/tiling-plan.md`, *Command language*.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use serde_json::{json, Value};
use smithay::output::Output;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::wayland::shell::xdg::XdgShellHandler;

use crate::config::Config;
use crate::state::{Backend, Otto};
use crate::workspaces::tiling::command::{
    self, Amount, AxisArg, Command, GapScope, GapTarget, Toggle, WorkspaceTarget,
};
use crate::workspaces::tiling::tree::{Axis, NodeId};
use crate::workspaces::tiling::{layout, Gaps, Rect};
use crate::workspaces::workspace::WorkspaceView;

/// One command's outcome, in the shape `swaymsg` prints: a success flag and,
/// when it failed, why.
pub type CommandResult = Result<(), String>;

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// Parse and run a `;`-separated command string, answering one result per
    /// command.
    ///
    /// A parse error stops the whole string — i3 and sway do the same — and
    /// comes back as a single failure naming the character it stumbled on.
    pub fn run_command(&mut self, text: &str) -> Vec<CommandResult> {
        let commands = match command::parse(text) {
            Ok(commands) => commands,
            Err(error) => {
                return vec![Err(format!(
                    "{} (at character {})",
                    error.message, error.offset
                ))]
            }
        };
        commands
            .into_iter()
            .map(|command| self.run_one(command))
            .collect()
    }

    fn run_one(&mut self, command: Command) -> CommandResult {
        match command {
            Command::Focus(direction) => {
                self.handle_tiling_focus(direction);
                Ok(())
            }
            Command::FocusParent => self.command_focus_step(true),
            Command::FocusChild => self.command_focus_step(false),
            Command::Move(direction) => {
                self.handle_tiling_move(direction);
                Ok(())
            }
            Command::MoveToWorkspace(number) => self.command_move_to_workspace(number),
            Command::Workspace(target) => self.command_workspace(target),
            Command::Split(arg) => self.command_split(arg),
            Command::Layout(arg) => self.command_layout(arg),
            Command::Resize { axis, grow, amount } => self.command_resize(axis, grow, amount),
            Command::Floating(toggle) => self.handle_tiling_floating(match toggle {
                Toggle::Toggle => None,
                Toggle::Enable => Some(true),
                Toggle::Disable => Some(false),
            }),
            Command::FocusMode(layer) => self.handle_tiling_focus_mode(layer),
            Command::Fullscreen => self.command_fullscreen(),
            Command::Kill => {
                self.close_focused_window();
                Ok(())
            }
            Command::Tiling(toggle) => self.command_tiling(toggle),
            Command::Gaps {
                scope,
                target,
                amount,
            } => self.command_gaps(scope, target, amount),
        }
    }

    // ── The commands ─────────────────────────────────────────────────────

    /// `focus parent` / `focus child`.
    fn command_focus_step(&mut self, up: bool) -> CommandResult {
        let Some(output) = self.tiling_output() else {
            return Err("no output has focus".to_string());
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return Err("no workspace has focus".to_string());
        };
        let Ok(mut state) = workspace.tiling.write() else {
            return Err("the tiling state is busy".to_string());
        };
        if !state.enabled {
            return Err("this workspace does not tile".to_string());
        }
        let moved = if up {
            state.focus_parent()
        } else {
            state.focus_child()
        };
        if moved {
            Ok(())
        } else if up {
            Err("focus is already on the whole workspace".to_string())
        } else {
            Err("focus is already on a window".to_string())
        }
    }

    /// `move container to workspace <n>`: the destination is created if the
    /// output does not have that many workspaces yet. An emptied workspace is
    /// never removed — Otto's workspaces are persistent, named and
    /// reorderable, unlike i3's (`docs/developer/tiling-plan.md`).
    fn command_move_to_workspace(&mut self, number: usize) -> CommandResult {
        let Some(window) = self.focused_window() else {
            return Err("no window has focus".to_string());
        };
        let Some(output) = self
            .workspaces
            .output_for_window(&window)
            .or_else(|| self.workspaces.focused_output().cloned())
        else {
            return Err("no output has focus".to_string());
        };
        let index = self.ensure_workspace(&output, number)?;
        self.workspaces
            .move_window_to_workspace_on_output(&output, &window, index, (0, 0));
        self.relayout_workspace(&output, true);
        Ok(())
    }

    /// `workspace <n|next|prev>`.
    fn command_workspace(&mut self, target: WorkspaceTarget) -> CommandResult {
        let Some(output) = self.workspaces.focused_output().cloned() else {
            return Err("no output has focus".to_string());
        };
        let count = self.workspace_count(&output);
        let current = self
            .workspaces
            .output_workspaces
            .get(&output.name())
            .map(|ows| ows.current_workspace)
            .unwrap_or(0);
        let index = match target {
            WorkspaceTarget::Number(number) => self.ensure_workspace(&output, number)?,
            WorkspaceTarget::Next => (current + 1).min(count.saturating_sub(1)),
            WorkspaceTarget::Prev => current.saturating_sub(1),
        };
        self.set_current_workspace_index(index);
        Ok(())
    }

    /// `split h|v|toggle`: arm the axis the next window splits the focused
    /// cell along.
    fn command_split(&mut self, arg: AxisArg) -> CommandResult {
        let axis = match arg {
            AxisArg::Axis(axis) => {
                self.handle_tiling_split(axis);
                return Ok(());
            }
            AxisArg::Toggle => {
                let Some(workspace) = self
                    .tiling_output()
                    .and_then(|output| self.workspaces.current_tiling_workspace(&output))
                else {
                    return Err("no workspace has focus".to_string());
                };
                let Ok(state) = workspace.tiling.read() else {
                    return Err("the tiling state is busy".to_string());
                };
                if !state.enabled {
                    return Err("this workspace does not tile".to_string());
                }
                // Flip whatever is armed; with nothing armed, flip the shape
                // the focused cell would otherwise split along.
                match state.preselect {
                    Some(axis) => axis.other(),
                    None => state
                        .focused_node()
                        .and_then(|node| state.tree.parent_of(node))
                        .and_then(|parent| state.tree.container_axis(parent))
                        .unwrap_or(Axis::Row)
                        .other(),
                }
            }
        };
        let Some(workspace) = self
            .tiling_output()
            .and_then(|output| self.workspaces.current_tiling_workspace(&output))
        else {
            return Err("no workspace has focus".to_string());
        };
        if let Ok(mut state) = workspace.tiling.write() {
            // Assigned rather than toggled: the axis was already worked out.
            state.preselect = Some(axis);
        }
        Ok(())
    }

    /// `layout splith|splitv|toggle split`: turn the container the focused
    /// cell sits in.
    fn command_layout(&mut self, arg: AxisArg) -> CommandResult {
        let Some(output) = self.tiling_output() else {
            return Err("no output has focus".to_string());
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return Err("no workspace has focus".to_string());
        };
        let changed = {
            let Ok(mut state) = workspace.tiling.write() else {
                return Err("the tiling state is busy".to_string());
            };
            if !state.enabled {
                return Err("this workspace does not tile".to_string());
            }
            // `focus parent` aims `layout` at the container it walked up to;
            // otherwise it is the focused window's own container.
            let Some(node) = state.focused_node() else {
                return Err("no window has focus".to_string());
            };
            let container = if state.tree.container_axis(node).is_some() {
                node
            } else {
                match state.tree.parent_of(node) {
                    Some(parent) => parent,
                    None => return Err("a lone tile has no container to lay out".to_string()),
                }
            };
            let axis = match arg {
                AxisArg::Axis(axis) => axis,
                AxisArg::Toggle => state
                    .tree
                    .container_axis(container)
                    .unwrap_or(Axis::Row)
                    .other(),
            };
            state.tree.set_container_axis(container, axis)
        };
        if changed {
            self.relayout_workspace(&output, true);
        }
        Ok(())
    }

    /// `resize grow|shrink width|height <n> px|ppt`.
    fn command_resize(&mut self, axis: Axis, grow: bool, amount: Amount) -> CommandResult {
        let Some(output) = self.tiling_output() else {
            return Err("no output has focus".to_string());
        };
        let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
            return Err("no workspace has focus".to_string());
        };
        let zone = self.tiling_area(&output);
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);

        let resized = {
            let Ok(mut state) = workspace.tiling.write() else {
                return Err("the tiling state is busy".to_string());
            };
            if !state.enabled {
                return Err("this workspace does not tile".to_string());
            }
            let Some(leaf) = state.focused.clone().filter(|id| state.tree.contains(id)) else {
                return Err("no tile has focus".to_string());
            };
            // A share, not pixels: `px` is resolved against the container the
            // resize actually moves, which is the extent its leaves span.
            let fraction = match amount {
                Amount::Ppt(percent) => percent / 100.0,
                Amount::Px(pixels) => {
                    let gaps = Config::with(|config| state.effective_gaps(&config.tiling));
                    let rects = layout::resolve(&state.tree, area, gaps);
                    let extent = state
                        .tree
                        .node_of(&leaf)
                        .and_then(|node| state.tree.parent_of(node))
                        .map(|container| container_extent(&state.tree, container, &rects, axis))
                        .unwrap_or(match axis {
                            Axis::Row => area.w,
                            Axis::Column => area.h,
                        });
                    if extent <= 0 {
                        return Err("that container has no extent to resize".to_string());
                    }
                    pixels as f32 / extent as f32
                }
            };
            let delta = if grow { fraction } else { -fraction };
            state.tree.resize(&leaf, axis, delta)
        };
        if resized {
            self.relayout_workspace(&output, true);
            Ok(())
        } else {
            Err("nothing to resize against on that axis".to_string())
        }
    }

    /// `fullscreen [toggle]`, through the same xdg-shell path a client's own
    /// request takes.
    fn command_fullscreen(&mut self) -> CommandResult {
        let Some(window) = self.focused_window() else {
            return Err("no window has focus".to_string());
        };
        let Some(toplevel) = window.toplevel().cloned() else {
            return Err("that window is not an xdg toplevel".to_string());
        };
        if window.is_fullscreen() {
            self.unfullscreen_request(toplevel);
        } else {
            self.fullscreen_request(toplevel, None);
        }
        Ok(())
    }

    /// `tiling toggle|enable|disable` — Otto's own command for the workspace
    /// mode i3 has no equivalent of.
    fn command_tiling(&mut self, toggle: Toggle) -> CommandResult {
        let Some(output) = self.tiling_output() else {
            return Err("no output has focus".to_string());
        };
        let enabled = match toggle {
            Toggle::Toggle => !self.workspaces.output_tiles(&output),
            Toggle::Enable => true,
            Toggle::Disable => false,
        };
        self.set_workspace_tiling(&output, enabled);
        Ok(())
    }

    /// `gaps inner|outer <n> [current|all]`.
    ///
    /// `current` — the default, as in sway — overrides the focused workspace
    /// alone and saves that override beside the workspace's name. `all` sets
    /// the `[tiling]` session default and drops every override, so one command
    /// can undo a session's worth of per-workspace tweaking.
    ///
    /// `smart_gaps` stays global either way: it is a preference about how a
    /// lone tile looks, not a measurement of one workspace.
    fn command_gaps(&mut self, scope: GapScope, target: GapTarget, amount: i32) -> CommandResult {
        // Which records have to be rewritten: every one for `all`, which
        // clears an override wherever there is one, and the focused
        // workspace's alone for `current`.
        let touched_all = matches!(target, GapTarget::All);
        let mut touched: Option<(String, usize)> = None;
        match target {
            GapTarget::All => {
                Config::update(|config| match scope {
                    GapScope::Inner => config.tiling.inner_gap = amount,
                    GapScope::Outer => config.tiling.outer_gap = amount,
                });
                self.clear_workspace_gap_overrides();
            }
            GapTarget::Current => {
                let Some(output) = self.tiling_output() else {
                    return Err("no output has focus".to_string());
                };
                let Some(workspace) = self.workspaces.current_tiling_workspace(&output) else {
                    return Err("no workspace has focus".to_string());
                };
                let name = output.name();
                if let Some(ows) = self.workspaces.output_workspaces.get(&name) {
                    touched = Some((name, ows.current_workspace));
                }
                let Ok(mut state) = workspace.tiling.write() else {
                    return Err("the tiling state is busy".to_string());
                };
                // Setting one gap on a workspace that has no override yet
                // takes the other from `[tiling]`, so the override describes
                // the whole workspace rather than half of one.
                let mut gaps: Gaps = Config::with(|config| state.effective_gaps(&config.tiling));
                match scope {
                    GapScope::Inner => gaps.inner = amount,
                    GapScope::Outer => gaps.outer = amount,
                }
                state.gaps = Some(gaps);
            }
        }
        // `all` has already persisted every entry, in the course of
        // clearing them.
        if !touched_all {
            if let Some((output, position)) = touched {
                self.workspaces.persist_workspace_entry(&output, position);
            }
        }
        let outputs: Vec<Output> = self.workspaces.outputs().cloned().collect();
        for output in outputs {
            self.relayout_workspace(&output, true);
        }
        Ok(())
    }

    // ── Workspaces on demand ─────────────────────────────────────────────

    /// How many workspaces `output` has.
    fn workspace_count(&self, output: &Output) -> usize {
        self.workspaces
            .output_workspaces
            .get(&output.name())
            .map(|ows| ows.spaces.len())
            .unwrap_or(0)
    }

    /// The 0-based index of i3's 1-based workspace `number` on `output`,
    /// appending workspaces until it exists.
    fn ensure_workspace(&mut self, output: &Output, number: usize) -> Result<usize, String> {
        if number == 0 {
            return Err("workspaces are numbered from 1".to_string());
        }
        let name = output.name();
        while self.workspace_count(output) < number {
            if self.workspaces.add_workspace_to_output(&name).is_none() {
                return Err(format!("could not add a workspace to {name}"));
            }
        }
        Ok(number - 1)
    }

    // ── The JSON getters ─────────────────────────────────────────────────

    /// The whole tree, in i3's `GetTree` node shape.
    pub fn tree_json(&self) -> Value {
        let mut outputs: Vec<Value> = Vec::new();
        let mut rect = json!({"x": 0, "y": 0, "width": 0, "height": 0});
        let focused_output = self.workspaces.focused_output().map(|o| o.name());
        let mut names: Vec<String> = self.workspaces.outputs().map(|o| o.name()).collect();
        names.sort();
        for name in names {
            let Some(output) = self
                .workspaces
                .outputs()
                .find(|o| o.name() == name)
                .cloned()
            else {
                continue;
            };
            outputs.push(self.output_node(&output, focused_output.as_deref() == Some(&name)));
        }
        if let Some(first) = outputs.first() {
            rect = first["rect"].clone();
        }
        json!({
            "id": 1,
            "type": "root",
            "name": "root",
            "layout": "splith",
            "orientation": "horizontal",
            "percent": Value::Null,
            "rect": rect,
            "focused": false,
            "focus": outputs.iter().map(|o| o["id"].clone()).collect::<Vec<_>>(),
            "urgent": false,
            "nodes": outputs,
            "floating_nodes": Vec::<Value>::new(),
        })
    }

    /// i3's `GetWorkspaces`: one entry per workspace on every output.
    pub fn workspaces_json(&self) -> Value {
        let focused_output = self.workspaces.focused_output().map(|o| o.name());
        let mut out: Vec<Value> = Vec::new();
        let mut names: Vec<String> = self.workspaces.outputs().map(|o| o.name()).collect();
        names.sort();
        for name in names {
            let Some(ows) = self.workspaces.output_workspaces.get(&name) else {
                continue;
            };
            let Some(output) = self.workspaces.outputs().find(|o| o.name() == name) else {
                continue;
            };
            let rect = self.output_rect(output);
            for (index, view) in ows.workspace_views.iter().enumerate() {
                let visible = index == ows.current_workspace;
                out.push(json!({
                    "num": view.display_number(),
                    "id": workspace_id(&name, index),
                    "name": view.display_name(),
                    "visible": visible,
                    "focused": visible && focused_output.as_deref() == Some(name.as_str()),
                    "urgent": false,
                    "output": name,
                    "rect": rect,
                }));
            }
        }
        Value::Array(out)
    }

    /// i3's `GetOutputs`.
    pub fn outputs_json(&self) -> Value {
        let focused_output = self.workspaces.focused_output().map(|o| o.name());
        let mut names: Vec<String> = self.workspaces.outputs().map(|o| o.name()).collect();
        names.sort();
        let out: Vec<Value> = names
            .into_iter()
            .filter_map(|name| {
                let output = self.workspaces.outputs().find(|o| o.name() == name)?;
                let ows = self.workspaces.output_workspaces.get(&name);
                let current = ows
                    .and_then(|ows| ows.workspace_views.get(ows.current_workspace))
                    .map(|view| view.display_name())
                    .unwrap_or_default();
                Some(json!({
                    "name": name,
                    "active": true,
                    "primary": self
                        .workspaces
                        .primary_output()
                        .is_some_and(|p| p.name() == name),
                    "focused": focused_output.as_deref() == Some(name.as_str()),
                    "current_workspace": current,
                    "rect": self.output_rect(output),
                }))
            })
            .collect();
        Value::Array(out)
    }

    /// The focused window as one `GetTree`-shaped node, or `None` when
    /// nothing has focus. What the `WindowChanged` signal carries.
    pub fn focused_container_node(&self) -> Option<Value> {
        let window = self.focused_window()?;
        let id = window.id();
        let geometry = self
            .workspaces
            .output_for_window(&window)
            .and_then(|output| self.workspaces.output_workspaces.get(&output.name()))
            .and_then(|ows| {
                ows.spaces
                    .iter()
                    .find_map(|space| space.element_geometry(&window))
            })
            .unwrap_or_default();
        Some(self.window_node(
            &id,
            Rect::new(
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            ),
            None,
            Some(&id),
        ))
    }

    // ── Node building ────────────────────────────────────────────────────

    fn output_rect(&self, output: &Output) -> Value {
        let geometry = self.workspaces.output_geometry(output).unwrap_or_default();
        json!({
            "x": geometry.loc.x,
            "y": geometry.loc.y,
            "width": geometry.size.w,
            "height": geometry.size.h,
        })
    }

    fn output_node(&self, output: &Output, focused: bool) -> Value {
        let name = output.name();
        let workspaces: Vec<Value> = self
            .workspaces
            .output_workspaces
            .get(&name)
            .map(|ows| {
                ows.workspace_views
                    .iter()
                    .enumerate()
                    .map(|(index, view)| {
                        self.workspace_node(
                            output,
                            &name,
                            index,
                            view,
                            index == ows.current_workspace,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "id": hash_id("output", &name),
            "type": "output",
            "name": name,
            "layout": "output",
            "orientation": "none",
            "percent": Value::Null,
            "rect": self.output_rect(output),
            "focused": focused,
            "focus": workspaces.iter().map(|w| w["id"].clone()).collect::<Vec<_>>(),
            "urgent": false,
            "active": true,
            "nodes": workspaces,
            "floating_nodes": Vec::<Value>::new(),
        })
    }

    fn workspace_node(
        &self,
        output: &Output,
        output_name: &str,
        index: usize,
        view: &WorkspaceView,
        visible: bool,
    ) -> Value {
        let zone = self.usable_zone(output);
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);
        let focused_id = self.focused_window().map(|w| w.id());

        // Windows the tree does not hold — a floating workspace's whole
        // contents, or the exceptions on a tiling one — are i3's
        // `floating_nodes`.
        let mut tiled: Vec<ObjectId> = Vec::new();
        let mut nodes: Vec<Value> = Vec::new();
        let mut layout_name = "splith";
        let mut orientation = "horizontal";

        // The workspace's own gaps when it has an override, else `[tiling]`.
        let (gaps, override_gaps) = view
            .tiling
            .read()
            .map(|state| {
                (
                    Config::with(|config| state.effective_gaps(&config.tiling)),
                    state.gaps,
                )
            })
            .unwrap_or_else(|_| (Config::with(|config| config.tiling.gaps()), None));

        if let Ok(state) = view.tiling.read() {
            if state.enabled {
                let rects = layout::resolve(&state.tree, area, gaps);
                tiled = state.tree.leaves();
                if let Some(root) = state.tree.root() {
                    if let Some(axis) = state.tree.container_axis(root) {
                        layout_name = axis_layout(axis);
                        orientation = axis_orientation(axis);
                    }
                    nodes = match state.tree.container_axis(root) {
                        // The root container's children are the workspace's,
                        // as they are in i3: a workspace *is* the container.
                        Some(_) => state
                            .tree
                            .children_of(root)
                            .iter()
                            .map(|child| {
                                self.tree_node(
                                    &state.tree,
                                    child.node,
                                    Some(child.share),
                                    &rects,
                                    focused_id.as_ref(),
                                    state.focused_container,
                                )
                            })
                            .collect(),
                        None => vec![self.tree_node(
                            &state.tree,
                            root,
                            None,
                            &rects,
                            focused_id.as_ref(),
                            state.focused_container,
                        )],
                    };
                }
            }
        }

        let floating: Vec<Value> = self
            .workspaces
            .output_workspaces
            .get(output_name)
            .and_then(|ows| ows.spaces.get(index))
            .map(|space| {
                space
                    .elements()
                    .filter(|window| !tiled.contains(&window.id()))
                    .map(|window| {
                        let geometry = space.element_geometry(window).unwrap_or_default();
                        self.window_node(
                            &window.id(),
                            Rect::new(
                                geometry.loc.x,
                                geometry.loc.y,
                                geometry.size.w,
                                geometry.size.h,
                            ),
                            None,
                            focused_id.as_ref(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        json!({
            "id": workspace_id(output_name, index),
            "type": "workspace",
            "num": view.display_number(),
            "name": view.display_name(),
            "layout": layout_name,
            "orientation": orientation,
            "percent": Value::Null,
            "rect": {"x": area.x, "y": area.y, "width": area.w, "height": area.h},
            // Otto's own extension: the workspace's gap override, or null when
            // it follows the `[tiling]` defaults.
            "gaps": override_gaps.map(|gaps| json!({"inner": gaps.inner, "outer": gaps.outer})),
            "output": output_name,
            "visible": visible,
            "focused": false,
            "urgent": false,
            "focus": nodes.iter().map(|n| n["id"].clone()).collect::<Vec<_>>(),
            "nodes": nodes,
            "floating_nodes": floating,
        })
    }

    /// One tree node — a split container or a window — and everything under it.
    fn tree_node(
        &self,
        tree: &crate::workspaces::tiling::tree::Tree<ObjectId>,
        node: NodeId,
        percent: Option<f32>,
        rects: &[(ObjectId, Rect)],
        focused: Option<&ObjectId>,
        focused_container: Option<NodeId>,
    ) -> Value {
        match tree.container_axis(node) {
            None => {
                let leaf = tree.leaves_under(node).into_iter().next();
                let rect = leaf
                    .as_ref()
                    .and_then(|id| rects.iter().find(|(l, _)| l == id))
                    .map(|(_, rect)| *rect)
                    .unwrap_or(Rect::new(0, 0, 0, 0));
                match leaf {
                    Some(id) => self.window_node(&id, rect, percent, focused),
                    None => json!({}),
                }
            }
            Some(axis) => {
                let children: Vec<Value> = tree
                    .children_of(node)
                    .iter()
                    .map(|child| {
                        self.tree_node(
                            tree,
                            child.node,
                            Some(child.share),
                            rects,
                            focused,
                            focused_container,
                        )
                    })
                    .collect();
                let rect = union_of(node, tree, rects);
                json!({
                    "id": container_id(node),
                    "type": "con",
                    "name": Value::Null,
                    "layout": axis_layout(axis),
                    "orientation": axis_orientation(axis),
                    "percent": percent,
                    "rect": {"x": rect.x, "y": rect.y, "width": rect.w, "height": rect.h},
                    "focused": focused_container == Some(node),
                    "urgent": false,
                    "focus": children.iter().map(|c| c["id"].clone()).collect::<Vec<_>>(),
                    "nodes": children,
                    "floating_nodes": Vec::<Value>::new(),
                })
            }
        }
    }

    fn window_node(
        &self,
        id: &ObjectId,
        rect: Rect,
        percent: Option<f32>,
        focused: Option<&ObjectId>,
    ) -> Value {
        let window = self.workspaces.windows_map.get(id);
        let title = window.map(|w| w.xdg_title()).unwrap_or_default();
        let app_id = window.map(|w| w.xdg_app_id()).unwrap_or_default();
        let mut node = json!({
            "id": hash_id("window", &format!("{id:?}")),
            "type": "con",
            "name": title,
            "layout": "none",
            "orientation": "none",
            "percent": percent,
            "rect": {"x": rect.x, "y": rect.y, "width": rect.w, "height": rect.h},
            "focused": focused == Some(id),
            "urgent": false,
            "app_id": app_id,
            "focus": Vec::<Value>::new(),
            "nodes": Vec::<Value>::new(),
            "floating_nodes": Vec::<Value>::new(),
        });
        // An X11 window answers to i3's `window_properties` as well, which is
        // what `for_window [class="…"]` scripts match on.
        #[cfg(feature = "xwayland")]
        if let Some(window) = window {
            if let Some(smithay::desktop::WindowSurface::X11(surface)) =
                Some(window.underlying_surface())
            {
                node["window_properties"] = json!({
                    "class": surface.class(),
                    "instance": surface.instance(),
                    "title": surface.title(),
                });
            }
        }
        node
    }
}

/// The rectangle a container spans: the union of the cells under it.
fn union_of(
    node: NodeId,
    tree: &crate::workspaces::tiling::tree::Tree<ObjectId>,
    rects: &[(ObjectId, Rect)],
) -> Rect {
    let mut bounds: Option<(i32, i32, i32, i32)> = None;
    for leaf in tree.leaves_under(node) {
        let Some((_, rect)) = rects.iter().find(|(l, _)| *l == leaf) else {
            continue;
        };
        bounds = Some(match bounds {
            None => (rect.x, rect.y, rect.x + rect.w, rect.y + rect.h),
            Some((x0, y0, x1, y1)) => (
                x0.min(rect.x),
                y0.min(rect.y),
                x1.max(rect.x + rect.w),
                y1.max(rect.y + rect.h),
            ),
        });
    }
    match bounds {
        Some((x0, y0, x1, y1)) => Rect::new(x0, y0, x1 - x0, y1 - y0),
        None => Rect::new(0, 0, 0, 0),
    }
}

/// The extent a container spans along `axis`, in logical pixels.
fn container_extent(
    tree: &crate::workspaces::tiling::tree::Tree<ObjectId>,
    container: NodeId,
    rects: &[(ObjectId, Rect)],
    axis: Axis,
) -> i32 {
    let rect = union_of(container, tree, rects);
    match axis {
        Axis::Row => rect.w,
        Axis::Column => rect.h,
    }
}

fn axis_layout(axis: Axis) -> &'static str {
    match axis {
        Axis::Row => "splith",
        Axis::Column => "splitv",
    }
}

fn axis_orientation(axis: Axis) -> &'static str {
    match axis {
        Axis::Row => "horizontal",
        Axis::Column => "vertical",
    }
}

/// A stable-per-session id in i3's opaque-integer shape. Containers are
/// arena indices, which are only unique within one tree, so they are salted
/// with a tag; windows hash their protocol object id.
fn hash_id(tag: &str, key: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    tag.hash(&mut hasher);
    key.hash(&mut hasher);
    // Keep it inside 2^53 so a JSON reader with float numbers (jq, JavaScript)
    // round-trips it exactly.
    hasher.finish() & 0x1f_ffff_ffff_ffff
}

fn container_id(node: NodeId) -> u64 {
    hash_id("con", &node.to_string())
}

fn workspace_id(output: &str, index: usize) -> u64 {
    hash_id("workspace", &format!("{output}:{index}"))
}
