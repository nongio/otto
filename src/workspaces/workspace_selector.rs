use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

use layers::prelude::*;
use otto_kit::components::{
    label::TextAlign as KitTextAlign,
    text_input::{
        KeyMods, TextInput, TextInputKey, TextInputRenderer, TextInputResponse, TextInputState,
        TextInputStyle,
    },
};
use smithay::{
    backend::input::{ButtonState, KeyState},
    input::{
        keyboard::{Keysym, ModifiersState},
        pointer::{CursorIcon, CursorImageStatus},
    },
    reexports::calloop::channel::Sender as CalloopSender,
    utils::Coordinate,
};

use crate::{
    interactive_view::ViewInteractions,
    theme::{self, theme_colors},
    utils::{button_press_scale, button_release_scale, draw_named_icon_any, draw_text_content},
};

use super::workspace::WorkspaceView;

pub const WORKSPACE_SELECTOR_PREVIEW_WIDTH: f32 = 300.0;
const WORKSPACE_SELECTOR_GAP: f32 = 50.0;
/// Two clicks closer together than this on the same label open the rename
/// editor.
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);
/// Fraction of the preview width the rename field spans.
const RENAME_FIELD_WIDTH_RATIO: f32 = 0.8;
/// Longest workspace name the field accepts.
const WORKSPACE_NAME_MAX_CHARS: usize = 32;
/// How long a newly added workspace takes to expand into the strip.
const WORKSPACE_ENTER_SECS: f32 = 0.5;
/// Grace period after a workspace appears during which the post-render hook
/// leaves its width alone, so re-renders can't cut the enter animation short.
const WORKSPACE_ENTER_SETTLE: Duration = Duration::from_millis(700);

/// The width every workspace item occupies in the strip once settled.
fn workspace_item_size() -> layers::types::Size {
    layers::types::Size {
        width: layers::taffy::style::Dimension::Length(
            WORKSPACE_SELECTOR_PREVIEW_WIDTH + WORKSPACE_SELECTOR_GAP,
        ),
        height: layers::taffy::style::Dimension::Percent(1.0),
    }
}

/// Collapsed width of a workspace item on its way out. Only the width changes —
/// the height stays a percentage so a scale or fullscreen change mid-animation
/// can't leave the departing item the wrong size.
fn workspace_item_collapsed_size() -> layers::types::Size {
    layers::types::Size {
        width: layers::taffy::style::Dimension::Length(0.0),
        height: layers::taffy::style::Dimension::Percent(1.0),
    }
}

/// The gap closing as a workspace leaves. The siblings slide because taffy
/// re-lays the row out against this width every frame.
fn workspace_collapse_transition() -> Transition {
    Transition::spring(0.5, 0.1)
}

#[derive(Clone, Debug)]
pub struct WorkspaceDropTarget {
    pub workspace_index: usize,
    pub drop_layer: layers::prelude::Layer,
}

#[derive(Clone, Debug)]
pub struct WorkspaceViewState {
    name: String,
    index: usize,
    workspace_node: Option<NodeRef>,
    background_node: Option<NodeRef>,
    workspace_width: f32,
    workspace_height: f32,
    fullscreen: bool,
    window_count: usize,
}

impl Hash for WorkspaceViewState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.index.hash(state);
        self.workspace_node.hash(state);
        self.background_node.hash(state);
        self.workspace_width.to_bits().hash(state);
        self.workspace_height.to_bits().hash(state);
        self.fullscreen.hash(state);
        self.window_count.hash(state);
    }
}

/// Snapshot of an in-progress in-place rename, carried in the view state so
/// the label re-renders on every keystroke, caret move and blink.
#[derive(Clone, Debug)]
struct LabelEditState {
    /// Global index of the workspace being renamed.
    index: usize,
    input: TextInputState,
    style: TextInputStyle,
    caret_visible: bool,
}

impl Hash for LabelEditState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.input.hash(state);
        self.style.hash(state);
        self.caret_visible.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceSelectorViewState {
    workspaces: Vec<WorkspaceViewState>,
    current: usize,
    drop_hover_index: Option<usize>,
    /// Fractional scale of the output this selector renders on. Drives label
    /// sizing and pointer hit-test coordinate conversion (per-output).
    scale: f32,
    /// `Some` while a workspace label is being renamed on this output.
    editing: Option<LabelEditState>,
    /// Workspaces (by global index) collapsing out of the strip. They stay in
    /// the tree until the compositor drops them, so the row can animate.
    removing: Vec<usize>,
}

impl Hash for WorkspaceSelectorViewState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.workspaces.hash(state);
        self.current.hash(state);
        self.drop_hover_index.hash(state);
        self.scale.to_bits().hash(state);
        self.editing.hash(state);
        self.removing.hash(state);
    }
}

/// A live rename session: the widget plus which workspace it belongs to.
struct LabelEdit {
    index: usize,
    input: TextInput,
    /// Left edge of the field inside the label layer, in scene points. Pointer
    /// x is translated by it before reaching the widget.
    field_origin_x: f32,
    caret_visible: bool,
}

#[derive(Clone)]
pub struct WorkspaceSelectorView {
    pub layer: layers::prelude::Layer,
    pub view: layers::prelude::View<WorkspaceSelectorViewState>,
    pub cursor_location: Arc<RwLock<Point>>,
    pub drop_targets: Arc<RwLock<Vec<WorkspaceDropTarget>>>,
    pub drop_hover_index: Arc<RwLock<Option<usize>>>,
    /// When each workspace first appeared in this selector, so the enter
    /// animation is left alone while it runs.
    known_indices: Arc<RwLock<HashMap<usize, Instant>>>,
    /// Workspaces the user asked to remove, collapsing out of the strip.
    removing: Arc<RwLock<HashSet<usize>>>,
    pressed_action: Arc<RwLock<Option<String>>>,
    /// Global logical origin of the output this selector lives on. Pointer
    /// events arrive in global logical space; subtracting this yields
    /// output-local coordinates for hit-testing (all output subtrees render
    /// at scene origin).
    output_origin: Arc<RwLock<(f64, f64)>>,
    /// Name of the output this selector belongs to. Add/remove act on this
    /// output only, so each display manages its own independent workspaces.
    output_name: Arc<RwLock<String>>,
    /// The rename session, if a label is being edited on this output.
    editing: Arc<RwLock<Option<LabelEdit>>>,
    /// Mirrors `editing.is_some()` for the keyboard path, which must not take
    /// the lock (and must see it from `Workspaces`, not from a selector).
    editing_flag: Arc<AtomicBool>,
    /// Last press: layer key, when, and how many clicks in a row — drives
    /// double-click-to-rename and word/all selection inside the field.
    last_click: Arc<RwLock<Option<(String, Instant, u32)>>>,
    /// Modifier state from the last key event, for shift-extended selection.
    modifiers: Arc<RwLock<ModifiersState>>,
    /// Carries `(output_name, workspace_index, name)` for a rename that ends
    /// without a `&mut Otto` at hand — losing keyboard focus mid-edit.
    rename_sender: CalloopSender<(String, usize, String)>,
}

/// # WorkspaceSelectorView Layer Structure
///
/// ```diagram
/// WorkspaceSelectorView
/// ├── layer (view(render_workspace_selector_view))
/// │   ├── workspace_selector_view_content
/// │   │   └── workspace_selector_desktop_{x}
/// │   │       └── workspace_selector_desktop_content_{x}
/// │   │           ├── workspace_desktop_content_preview_{x}
/// │   │           └── workspace_selector_desktop_remove_{x}
/// │   └── workspace_selector_add
/// ```
///
/// - `layer`: The root layer for the window selector view.
///
///
impl WorkspaceSelectorView {
    pub fn new(
        _layers_engine: Arc<Engine>,
        layer: Layer,
        // Carries `(Some(output_name), workspace_position)` — removal is scoped
        // to the output this selector belongs to (workspaces are independent
        // per output). The `Option` lets the fullscreen-close path share the
        // channel with `None` for a lockstep removal.
        remove_sender: CalloopSender<(Option<String>, usize)>,
        editing_flag: Arc<AtomicBool>,
        rename_sender: CalloopSender<(String, usize, String)>,
    ) -> Self {
        let state = WorkspaceSelectorViewState {
            workspaces: Vec::new(),
            current: 0,
            drop_hover_index: None,
            scale: 1.0,
            editing: None,
            removing: Vec::new(),
        };
        let view = View::new(
            "workspace_selector_view",
            state,
            render_workspace_selector_view,
        );
        layer.set_pointer_events(false);
        layer.set_position((0.0, -400.0), None);
        layer.set_opacity(1.0_f32, None);
        view.set_layer(layer.clone());

        let drop_targets = Arc::new(RwLock::new(Vec::new()));
        let drop_hover_index = Arc::new(RwLock::new(None));
        let pressed_action = Arc::new(RwLock::new(None));
        let removing: Arc<RwLock<HashSet<usize>>> = Arc::new(RwLock::new(HashSet::new()));
        let output_name = Arc::new(RwLock::new(String::new()));

        // Setup post-render hook to update drop targets and animate new workspaces
        let drop_targets_clone = drop_targets.clone();
        let known_indices: Arc<RwLock<HashMap<usize, Instant>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let known_indices_for_hook = known_indices.clone();
        // Indices whose collapse animation has already been started, so a
        // re-render can't restart it (which would drop the completion callback
        // that actually removes the workspace).
        let collapsing: Arc<RwLock<HashSet<usize>>> = Arc::new(RwLock::new(HashSet::new()));
        let output_name_for_hook: Arc<RwLock<String>> = output_name.clone();
        let remove_sender_for_hook = remove_sender.clone();

        view.add_post_render_hook(move |state, view, _layer| {
            let targets: Vec<WorkspaceDropTarget> = state
                .workspaces
                .iter()
                .filter(|w| !state.removing.contains(&w.index))
                .filter_map(|w| {
                    let key = format!("workspace_selector_desktop_content_{}", w.index);
                    view.layer_by_key(&key).map(|layer| WorkspaceDropTarget {
                        workspace_index: w.index,
                        drop_layer: layer.clone(),
                    })
                })
                .collect();
            *drop_targets_clone.write().unwrap() = targets;

            // This hook is the single owner of every item's width: the render
            // function never declares it, so a re-render can't cut an enter or
            // leave animation short, and an item that never gets removed can't
            // be left collapsed to nothing.
            let mut known = known_indices_for_hook.write().unwrap();
            let mut collapsing = collapsing.write().unwrap();
            for w in state.workspaces.iter() {
                let is_new = !known.contains_key(&w.index);
                let appeared_at = *known.entry(w.index).or_insert_with(Instant::now);
                let entering = appeared_at.elapsed() < WORKSPACE_ENTER_SETTLE;
                let leaving = state.removing.contains(&w.index);
                let workspace_width = w.workspace_width.max(1.0);

                let key = format!("workspace_selector_desktop_{}", w.index);
                let wrap_key = format!("workspace_selector_desktop_wrap_{}", w.index);
                // Arm the collapse exactly once per workspace.
                let start_leaving = leaving && collapsing.insert(w.index);

                if let Some(layer) = view.layer_by_key(&key) {
                    if start_leaving {
                        let index = w.index;
                        let sender = remove_sender_for_hook.clone();
                        let output = output_name_for_hook.read().unwrap().clone();
                        let view_ref = view.clone();
                        layer
                            .set_size(
                                workspace_item_collapsed_size(),
                                workspace_collapse_transition(),
                            )
                            .then(move |_layer: &Layer, _| {
                                // The channel carries a POSITION, so resolve it
                                // now rather than at click time — an add or
                                // remove elsewhere may have shifted it while
                                // this item was collapsing.
                                let pos = view_ref
                                    .get_state()
                                    .workspaces
                                    .iter()
                                    .position(|w| w.index == index);
                                if let Some(pos) = pos {
                                    let _ = sender.send((Some(output.clone()), pos));
                                }
                            });
                    } else if is_new {
                        layer.set_size(layers::types::Size::points(0.0, 0.0), None);
                        layer.set_size(
                            workspace_item_size(),
                            Transition::ease_out(WORKSPACE_ENTER_SECS),
                        );
                    } else if !leaving && !entering {
                        layer.set_size(workspace_item_size(), None);
                    }
                }

                if let Some(wrap) = view.layer_by_key(&wrap_key) {
                    if start_leaving {
                        // Crop the preview against the shrinking wrap instead of
                        // scaling or fading it. Clipping is armed only for the
                        // collapse: at rest the remove button and its shadow
                        // overhang the wrap and must not be cut off.
                        wrap.set_clip_children(true, None);
                        // That overhang is why the button goes now rather than
                        // fading with the pointer — it would be clipped in half
                        // on the first frame of the collapse.
                        if let Some(button) = view
                            .layer_by_key(&format!("workspace_selector_desktop_remove_{}", w.index))
                        {
                            button.set_opacity(0.0_f32, None);
                        }
                    } else if is_new {
                        wrap.set_clip_children(false, None);
                        let offset_x = workspace_width / 2.0;
                        wrap.set_position(Point::new(offset_x, 0.0), None);
                        wrap.set_position(Point::new(0.0, 0.0), Transition::spring(1.2, 0.1));
                    } else if !leaving {
                        wrap.set_clip_children(false, None);
                        wrap.set_position(Point::new(0.0, 0.0), None);
                        wrap.set_opacity(1.0_f32, None);
                    }
                }
            }
            // Forget removed indices so re-added workspaces animate again
            known.retain(|idx, _| state.workspaces.iter().any(|w| w.index == *idx));
            collapsing.retain(|idx| state.removing.contains(idx));
        });

        Self {
            // engine: layers_engine,
            layer,
            view,
            cursor_location: Arc::new(RwLock::new(Point::default())),
            drop_targets,
            drop_hover_index,
            known_indices,
            removing,
            pressed_action,
            output_origin: Arc::new(RwLock::new((0.0, 0.0))),
            output_name,
            editing: Arc::new(RwLock::new(None)),
            editing_flag,
            last_click: Arc::new(RwLock::new(None)),
            modifiers: Arc::new(RwLock::new(ModifiersState::default())),
            rename_sender,
        }
    }

    /// Populate this selector from a single output's workspace set. Each
    /// output drives its own selector so previews reflect that output's
    /// content at that output's physical resolution.
    pub fn set_workspaces(
        &self,
        workspaces: &[Arc<WorkspaceView>],
        current: usize,
        width: f32,
        height: f32,
        scale: f32,
    ) {
        let mut state = self.view.get_state();
        {
            let mut known = self.known_indices.write().unwrap();
            state.workspaces = workspaces
                .iter()
                .enumerate()
                .map(|(i, w)| WorkspaceViewState {
                    name: w.display_name(i),
                    index: w.index,
                    workspace_node: Some(w.windows_layer.id()),
                    background_node: Some(w.workspace_background.id()),
                    workspace_width: width,
                    workspace_height: height,
                    fullscreen: w.get_fullscreen_mode(),
                    window_count: w.windows_list.read().unwrap().len(),
                })
                .collect();
            known.retain(|idx, _| state.workspaces.iter().any(|w| w.index == *idx));
        }
        {
            // A workspace the compositor has dropped is no longer "removing";
            // one it refused to drop (last workspace, fullscreen with windows)
            // stays in the set until the collapse finishes, and the hook then
            // restores its width.
            let mut removing = self.removing.write().unwrap();
            removing.retain(|idx| state.workspaces.iter().any(|w| w.index == *idx));
            state.removing = {
                let mut list: Vec<usize> = removing.iter().copied().collect();
                list.sort_unstable();
                list
            };
        }
        state.current = current;
        state.scale = scale;
        self.view.update_state(&state);
    }

    /// Start collapsing `index` out of the strip. The compositor-side removal
    /// is sent when the collapse finishes (see the post-render hook), so the
    /// row can close the gap before the item leaves the tree.
    fn begin_remove(&self, index: usize) {
        {
            let mut removing = self.removing.write().unwrap();
            if !removing.insert(index) {
                return;
            }
        }
        let mut state = self.view.get_state();
        if !state.removing.contains(&index) {
            state.removing.push(index);
            state.removing.sort_unstable();
        }
        self.view.update_state(&state);
    }

    /// Is this workspace on its way out of the strip?
    fn is_removing(&self, index: usize) -> bool {
        self.removing.read().unwrap().contains(&index)
    }

    /// `(workspace index, window count)` for every preview, in the order they
    /// are laid out. What the previews are currently drawn from — stale here
    /// means a preview showing a window that has since moved or closed.
    pub fn preview_window_counts(&self) -> Vec<(usize, usize)> {
        self.view
            .get_state()
            .workspaces
            .iter()
            .map(|w| (w.index, w.window_count))
            .collect()
    }

    /// Set the global logical origin of the output hosting this selector, so
    /// pointer events (delivered in global logical space) can be converted to
    /// output-local coordinates for hit-testing.
    pub fn set_output_origin(&self, origin: (f64, f64)) {
        *self.output_origin.write().unwrap() = origin;
    }

    /// Set the name of the output this selector belongs to. Add/remove use it
    /// to scope the operation to this output.
    pub fn set_output_name(&self, name: &str) {
        let mut n = self.output_name.write().unwrap();
        if *n != name {
            *n = name.to_owned();
        }
    }

    /// Get current drop targets (updated after each render)
    pub fn get_drop_targets(&self) -> Vec<WorkspaceDropTarget> {
        self.drop_targets.read().unwrap().clone()
    }

    /// Set which workspace is being hovered during drag (for visual feedback)
    pub fn set_drop_hover(&self, workspace_index: Option<usize>) {
        *self.drop_hover_index.write().unwrap() = workspace_index;

        // Update view state to trigger re-render with new hover indication
        let mut state = self.view.get_state().clone();
        state.drop_hover_index = workspace_index;
        self.view.update_state(&state);
    }

    /// Get the currently hovered workspace index
    pub fn get_drop_hover(&self) -> Option<usize> {
        *self.drop_hover_index.read().unwrap()
    }

    /// Is a label being renamed on this output?
    pub fn is_editing(&self) -> bool {
        self.editing.read().unwrap().is_some()
    }

    /// The workspace whose label is being renamed, if any.
    pub fn editing_index(&self) -> Option<usize> {
        self.editing.read().unwrap().as_ref().map(|e| e.index)
    }

    /// Open the in-place editor on `index`, pre-filled with its current name
    /// and fully selected — typing replaces the name, as everywhere else.
    fn start_editing(&self, index: usize, name: String) {
        let state = self.view.get_state();
        let key = format!("workspace_selector_desktop_label_{index}");
        let Some(label) = self.view.layer_by_key(&key) else {
            return;
        };
        let bounds = label.render_bounds_transformed();
        let field_width = rename_field_width(bounds.width());

        let mut input = TextInput::editing(name, rename_field_style(state.scale));
        input.state.max_chars = Some(WORKSPACE_NAME_MAX_CHARS);
        input.set_size(field_width, bounds.height());

        *self.editing.write().unwrap() = Some(LabelEdit {
            index,
            input,
            field_origin_x: bounds.x() + (bounds.width() - field_width) / 2.0,
            caret_visible: true,
        });
        self.editing_flag.store(true, Ordering::Relaxed);
        self.sync_edit_state();
    }

    /// Push the live widget's state into the view so the label redraws.
    fn sync_edit_state(&self) {
        let snapshot = self
            .editing
            .read()
            .unwrap()
            .as_ref()
            .map(|edit| LabelEditState {
                index: edit.index,
                input: edit.input.state.clone(),
                style: edit.input.style.clone(),
                caret_visible: edit.caret_visible,
            });
        let mut state = self.view.get_state();
        state.editing = snapshot;
        self.view.update_state(&state);
    }

    /// End the session and return `(workspace index, typed name)` — the caller
    /// decides whether to keep the name (commit) or drop it (cancel).
    fn end_editing(&self) -> Option<(usize, String)> {
        let edit = self.editing.write().unwrap().take()?;
        self.editing_flag.store(false, Ordering::Relaxed);
        self.sync_edit_state();
        Some((edit.index, edit.input.value().to_string()))
    }

    /// Toggle the caret for the blink timer. Returns false once the session is
    /// over, so the timer can drop itself.
    pub fn blink_caret(&self) -> bool {
        {
            let mut editing = self.editing.write().unwrap();
            let Some(edit) = editing.as_mut() else {
                return false;
            };
            edit.caret_visible = !edit.caret_visible;
        }
        self.sync_edit_state();
        true
    }

    /// Feed a pointer press at scene-space `x` into the field, counting clicks
    /// for word (2) and all (3) selection.
    fn field_pointer_down(&self, x: f32, click_count: u32, shift: bool) {
        {
            let mut editing = self.editing.write().unwrap();
            let Some(edit) = editing.as_mut() else {
                return;
            };
            let local_x = x - edit.field_origin_x;
            edit.input.on_pointer_down(local_x, click_count, shift);
            edit.caret_visible = true;
        }
        self.sync_edit_state();
    }

    /// Extend the selection while the button is held.
    fn field_pointer_drag(&self, x: f32) {
        {
            let mut editing = self.editing.write().unwrap();
            let Some(edit) = editing.as_mut() else {
                return;
            };
            let local_x = x - edit.field_origin_x;
            edit.input.on_pointer_drag(local_x);
        }
        self.sync_edit_state();
    }

    fn field_pointer_up(&self) {
        if let Some(edit) = self.editing.write().unwrap().as_mut() {
            edit.input.on_pointer_up();
        }
    }

    /// Count this press as part of a click run on `key`.
    fn register_click(&self, key: &str) -> u32 {
        let mut last = self.last_click.write().unwrap();
        let count = match last.as_ref() {
            Some((last_key, at, count))
                if last_key == key && at.elapsed() < DOUBLE_CLICK_INTERVAL =>
            {
                count + 1
            }
            _ => 1,
        };
        *last = Some((key.to_string(), Instant::now(), count));
        count
    }
}

/// Width of the rename field inside a label layer `label_width` wide.
fn rename_field_width(label_width: f32) -> f32 {
    label_width * RENAME_FIELD_WIDTH_RATIO
}

/// The rename field's style for an output at `scale`, matching the label it
/// replaces (same family and size, centered).
fn rename_field_style(scale: f32) -> TextInputStyle {
    TextInputStyle::with_theme(crate::theme::kit_theme())
        .with_scale(scale)
        .with_align(KitTextAlign::Center)
        .with_text_style(otto_kit::typography::styles::TITLE_2)
}

/// Draw the rename field centered in the label layer it replaces.
fn draw_rename_field(edit: LabelEditState) -> Option<ContentDrawFunction> {
    let draw = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        let field_width = rename_field_width(w);
        canvas.save();
        canvas.translate(((w - field_width) / 2.0, 0.0));
        TextInputRenderer::render(
            canvas,
            &edit.input,
            &edit.style,
            field_width,
            h,
            edit.caret_visible,
        );
        canvas.restore();
        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    Some(draw.into())
}

/// Is the scene-space point `(x, y)` inside this layer's transformed bounds?
fn layer_contains(layer: &Layer, x: f32, y: f32) -> bool {
    let r = layer.render_bounds_transformed();
    x >= r.x() && x <= r.x() + r.width() && y >= r.y() && y <= r.y() + r.height()
}

/// The darkening applied to a workspace preview while it is pressed or is the
/// drop target.
fn press_darken_filter() -> Option<layers::skia::ColorFilter> {
    let darken_color = layers::skia::Color::from_argb(100, 100, 100, 100);
    let add = layers::skia::Color::from_argb(0, 0, 0, 0);
    layers::skia::color_filters::lighting(darken_color, add)
}

/// Drop the darkening from both mirrors of a preview (windows and wallpaper).
fn clear_preview_filter(
    content_mirror: &Layer,
    view: &View<WorkspaceSelectorViewState>,
    workspace_index: usize,
) {
    content_mirror.set_color_filter(None);
    if let Some(bg) = view
        .layer_by_key(format!("workspace_selector_desktop_bg_mirror_{}", workspace_index).as_str())
    {
        bg.set_color_filter(None);
    }
}

fn render_workspace_selector_view(
    state: &WorkspaceSelectorViewState,
    view: &View<WorkspaceSelectorViewState>,
) -> LayerTree {
    let workspaces = state.workspaces.clone();

    let (_ww, wh) = workspaces.iter().fold((0.0, 0.0), |(max_w, max_h), w| {
        let workspace_width = w.workspace_width.max(1.0);
        let workspace_height = w.workspace_height.max(1.0);
        let preview_width = WORKSPACE_SELECTOR_PREVIEW_WIDTH;
        let scale = preview_width / workspace_width;
        let preview_height = workspace_height * scale;
        let label_height = 30.0 * state.scale;
        (
            max_w.max(preview_width + WORKSPACE_SELECTOR_GAP),
            max_h.max(preview_height + label_height),
        )
    });
    let workspaces_tree = workspaces
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let workspace_index = w.index;
            let current = i == state.current;
            // The hover carries the workspace *view* index (that is what the
            // drop targets are keyed by). Comparing it to the position only
            // held while the two ran one apart — adding and removing
            // workspaces breaks that, and the wrong item lit up.
            let is_drop_hover = state.drop_hover_index == Some(workspace_index) && !current;

            let mut border_width = 0.0;
            let border_color = crate::theme::accent_color();

            if current {
                border_width = 8.0;
            }
            let mut color_filter = None;
            if is_drop_hover {
                color_filter = press_darken_filter();
            }
            let workspace_width = w.workspace_width.max(1.0);
            let workspace_height = w.workspace_height.max(1.0);
            let preview_width = WORKSPACE_SELECTOR_PREVIEW_WIDTH;
            let scale = preview_width / workspace_width;
            let preview_height = workspace_height * scale;
            let label_height = 30.0 * state.scale;

            LayerTreeBuilder::with_key(format!(
                "workspace_selector_desktop_{}",
                workspace_index.clone()
            ))
            .layout_style(taffy::Style {
                position: taffy::Position::Relative,
                display: taffy::Display::Flex,
                flex_direction: taffy::FlexDirection::Column,
                align_items: Some(taffy::AlignItems::Center),
                justify_content: Some(taffy::AlignContent::Center),
                ..Default::default()
            })
            .children(vec![LayerTreeBuilder::with_key(format!(
                "workspace_selector_desktop_wrap_{}",
                workspace_index.clone()
            ))
            .layout_style(taffy::Style {
                position: taffy::Position::Absolute,
                display: taffy::Display::Flex,
                flex_direction: taffy::FlexDirection::Column,
                // Centred, so the collapse crops the preview evenly from both
                // sides instead of eating it from the right.
                align_items: Some(taffy::AlignItems::Center),
                justify_content: Some(taffy::AlignContent::Center),
                // Half a gap of inset on each side, with an auto width, so the
                // wrap is always the item's width minus one gap. During the
                // collapse that keeps a full gap on both sides of the cropped
                // preview instead of letting the neighbour crowd in — taffy
                // clamps the width at zero once the item is narrower than a gap.
                inset: taffy::Rect {
                    left: taffy::LengthPercentageAuto::Length(WORKSPACE_SELECTOR_GAP / 2.0),
                    right: taffy::LengthPercentageAuto::Length(WORKSPACE_SELECTOR_GAP / 2.0),
                    top: taffy::LengthPercentageAuto::Auto,
                    bottom: taffy::LengthPercentageAuto::Auto,
                },
                ..Default::default()
            })
            .size((
                layers::types::Size {
                    width: layers::taffy::style::Dimension::Auto,
                    height: layers::taffy::style::Dimension::Length(preview_height + label_height),
                },
                None,
            ))
            .children(vec![
                LayerTreeBuilder::with_key(format!(
                    "workspace_selector_desktop_content_{}",
                    workspace_index.clone()
                ))
                .layout_style(taffy::Style {
                    ..Default::default()
                })
                .position(Point::new(0.0, 0.0))
                .size((
                    layers::types::Size {
                        width: layers::taffy::style::Dimension::Length(preview_width),
                        height: layers::taffy::style::Dimension::Length(preview_height),
                    },
                    None,
                ))
                .on_pointer_move({
                    let view_ref = view.clone();
                    move |_layer: &Layer, _x, _y| {
                        let key = format!("workspace_selector_desktop_remove_{}", workspace_index);
                        if let Some(remove_button) = view_ref.layer_by_key(key.as_str()) {
                            remove_button.set_opacity(1.0_f32, Transition::spring(0.3, 0.1));
                            remove_button
                                .set_scale(Point::new(1.0, 1.0), Transition::spring(0.3, 0.1));
                        }
                    }
                })
                .on_pointer_out({
                    let view_ref = view.clone();
                    move |layer: &Layer, x, y| {
                        let key = format!("workspace_selector_desktop_remove_{}", workspace_index);
                        let Some(remove_button) = view_ref.layer_by_key(key.as_str()) else {
                            return;
                        };
                        // Reaching for the button crosses out of the preview, so the
                        // engine emits Out here (after the Move that revealed it) and
                        // the button would vanish under the cursor. Keep it shown while
                        // the pointer is still over the preview or over the button.
                        if layer_contains(layer, x, y) || layer_contains(&remove_button, x, y) {
                            return;
                        }
                        remove_button.set_opacity(0.0_f32, Transition::spring(0.3, 0.1));
                        remove_button.set_scale(Point::new(0.8, 0.8), Transition::spring(0.3, 0.1));
                    }
                })
                .children::<LayerTree>({
                    let children: Vec<Option<LayerTree>> = vec![
                        // The wallpaper lives in the shared background_plane (own KMS
                        // plane), outside the windows subtree mirrored below — mirror
                        // it separately so the preview shows the full desktop.
                        Some(
                            LayerTreeBuilder::with_key(format!(
                                "workspace_selector_desktop_bg_mirror_{}",
                                workspace_index.clone()
                            ))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .size((
                                layers::types::Size {
                                    width: layers::taffy::style::Dimension::Length(workspace_width),
                                    height: layers::taffy::style::Dimension::Length(
                                        workspace_height,
                                    ),
                                },
                                None,
                            ))
                            .scale(Point::new(scale, scale))
                            .replicate_node(w.background_node)
                            .picture_cached(true)
                            .image_cache(true)
                            .color_filter(color_filter.clone())
                            .border_corner_radius(BorderRadius::new_single(20.0 / scale))
                            .clip_children(true)
                            .clip_content(true)
                            .pointer_events(false)
                            .build()
                            .unwrap(),
                        ),
                        Some(
                            LayerTreeBuilder::with_key(format!(
                                "workspace_selector_desktop_content_mirror_{}",
                                workspace_index.clone()
                            ))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .size((
                                layers::types::Size {
                                    width: layers::taffy::style::Dimension::Length(workspace_width),
                                    height: layers::taffy::style::Dimension::Length(
                                        workspace_height,
                                    ),
                                },
                                None,
                            ))
                            .scale(Point::new(scale, scale))
                            .replicate_node(w.workspace_node)
                            .picture_cached(true)
                            .image_cache(true)
                            .color_filter(color_filter)
                            .border_corner_radius(BorderRadius::new_single(20.0 / scale))
                            .clip_children(true)
                            .clip_content(true)
                            .pointer_events(true)
                            // The wallpaper is mirrored in a sibling layer, so darken
                            // both on press — otherwise only the windows dim.
                            .on_pointer_press({
                                let view_ref = view.clone();
                                move |layer: &Layer, _x, _y| {
                                    let filter = press_darken_filter();
                                    layer.set_color_filter(filter.clone());
                                    if let Some(bg) = view_ref.layer_by_key(
                                        format!(
                                            "workspace_selector_desktop_bg_mirror_{}",
                                            workspace_index
                                        )
                                        .as_str(),
                                    ) {
                                        bg.set_color_filter(filter);
                                    }
                                }
                            })
                            .on_pointer_release({
                                let view_ref = view.clone();
                                move |layer: &Layer, _x, _y| {
                                    clear_preview_filter(layer, &view_ref, workspace_index);
                                }
                            })
                            .on_pointer_out({
                                let view_ref = view.clone();
                                move |layer: &Layer, _x, _y| {
                                    clear_preview_filter(layer, &view_ref, workspace_index);
                                }
                            })
                            .build()
                            .unwrap(),
                        ),
                        Some(
                            LayerTreeBuilder::with_key(format!(
                                "workspace_selector_desktop_border_{}",
                                w.index
                            ))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .position(Point::new(0.0, 0.0))
                            .size((
                                layers::types::Size {
                                    width: layers::taffy::style::Dimension::Percent(1.0),
                                    height: layers::taffy::style::Dimension::Percent(1.0),
                                },
                                None,
                            ))
                            .border_width((border_width, None))
                            .border_color(border_color)
                            .border_corner_radius(BorderRadius::new_single(20.0))
                            .pointer_events(false)
                            .build()
                            .unwrap(),
                        ),
                        // Only show remove button if not current workspace and not a non-empty fullscreen workspace
                        (!(current || w.fullscreen && w.window_count > 0)).then(|| -> LayerTree {
                            LayerTreeBuilder::with_key(format!(
                                "workspace_selector_desktop_remove_{}",
                                w.index
                            ))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .anchor_point(Point::new(0.5, 0.5))
                            .scale(Point::new(0.2, 0.2))
                            .opacity((0.0, None))
                            .position(Point::new(preview_width, 0.0))
                            .size((
                                layers::types::Size {
                                    width: layers::taffy::style::Dimension::Length(50.0),
                                    height: layers::taffy::style::Dimension::Length(50.0),
                                },
                                None,
                            ))
                            .background_color(theme_colors().materials_ultrathick)
                            .blend_mode(BlendMode::BackgroundBlur)
                            .border_corner_radius(BorderRadius::new_single(25.0))
                            .content(draw_named_icon_any(&[
                                "close-symbolic",
                                "window-close-symbolic",
                            ]))
                            .shadow_color((Color::new_rgba(0.0, 0.0, 0.0, 0.2), None))
                            .shadow_offset(((0.0, 0.0).into(), None))
                            .shadow_radius((5.0, None))
                            .image_cache(true)
                            .on_pointer_press(button_press_scale(0.9))
                            .on_pointer_release(button_release_scale())
                            .build()
                            .unwrap()
                        }),
                    ];
                    children
                })
                .build()
                .unwrap(),
                LayerTreeBuilder::with_key(format!("workspace_selector_desktop_label_{}", w.index))
                    .layout_style(taffy::Style {
                        position: taffy::Position::Relative,
                        ..Default::default()
                    })
                    // A fixed width, not a percentage: the label is cropped
                    // along with the preview while the item collapses instead
                    // of re-centring its text in a shrinking box.
                    .size((
                        layers::types::Size {
                            width: layers::taffy::style::Dimension::Length(preview_width),
                            height: layers::taffy::style::Dimension::Length(label_height),
                        },
                        None,
                    ))
                    .content(
                        match state.editing.as_ref().filter(|e| e.index == w.index) {
                            // Renaming: the field takes the label's place, so the
                            // row keeps its geometry and nothing below it moves.
                            Some(edit) => draw_rename_field(edit.clone()),
                            None => draw_text_content(
                                w.name.clone(),
                                theme::text_styles::title_3_regular(),
                                layers::skia::textlayout::TextAlign::Center,
                            ),
                        },
                    )
                    .build()
                    .unwrap(),
            ])
            .build()
            .unwrap()])
            .build()
            .unwrap()
        })
        .collect();
    LayerTreeBuilder::with_key("workspace_selector_view")
        .layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            display: taffy::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            ..Default::default()
        })
        .size((
            layers::types::Size {
                width: layers::taffy::style::Dimension::Percent(1.0),
                height: layers::taffy::style::Dimension::Auto,
            },
            None,
        ))
        .background_color(theme_colors().materials_medium)
        .blend_mode(BlendMode::BackgroundBlur)
        .shadow_color(theme_colors().shadow_color)
        .shadow_offset(((0.0, -5.0).into(), None))
        .shadow_radius((20.0, None))
        .children(vec![
            LayerTreeBuilder::with_key("workspace_selector_view_content")
                .layout_style(taffy::Style {
                    display: taffy::Display::Flex,
                    flex_direction: taffy::FlexDirection::Row,
                    align_items: Some(taffy::AlignItems::Center),
                    justify_content: Some(taffy::AlignContent::Center),
                    gap: taffy::length(0.0_f32),
                    padding: taffy::Rect {
                        bottom: taffy::length(20.0_f32),
                        top: taffy::length(30.0_f32),
                        left: taffy::length(10.0_f32),
                        right: taffy::length(10.0_f32),
                    },
                    ..Default::default()
                })
                .size((
                    layers::types::Size {
                        width: layers::taffy::style::Dimension::Percent(1.0),
                        height: layers::taffy::style::Dimension::Length(wh + 50.0),
                    },
                    None,
                ))
                .children(workspaces_tree)
                .build()
                .unwrap(),
            LayerTreeBuilder::default()
                .key("workspace_selector_desktop_add")
                .layout_style(taffy::Style {
                    ..Default::default()
                })
                .size((
                    layers::types::Size {
                        width: layers::taffy::style::Dimension::Length(80.0),
                        height: layers::taffy::style::Dimension::Length(80.0),
                    },
                    None,
                ))
                .content(draw_named_icon_any(&["plus-symbolic", "list-add-symbolic"]))
                .image_cache(true)
                .on_pointer_press(button_press_scale(0.9))
                .on_pointer_release(button_release_scale())
                .build()
                .unwrap(),
        ])
        .build()
        .unwrap()
}

/// How long the caret stays on (and off) while renaming.
const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(530);

impl WorkspaceSelectorView {
    /// Open the rename editor on `index`: grab the keyboard so keys reach the
    /// field instead of the focused window, and start the caret blinking.
    fn begin_rename<B: crate::state::Backend>(
        &self,
        index: usize,
        otto: &mut crate::Otto<B>,
        seat: &smithay::input::Seat<crate::Otto<B>>,
        serial: smithay::utils::Serial,
    ) {
        let name = self
            .view
            .get_state()
            .workspaces
            .iter()
            .find(|w| w.index == index)
            .map(|w| w.name.clone())
            .unwrap_or_default();
        self.start_editing(index, name);
        if !self.is_editing() {
            return;
        }

        if let Some(keyboard) = seat.get_keyboard() {
            let view = crate::interactive_view::InteractiveView {
                view: Box::new(self.clone()),
            };
            keyboard.set_focus(
                otto,
                Some(crate::focus::KeyboardFocusTarget::View(view)),
                serial,
            );
        }

        let selector = self.clone();
        let _ = otto.handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(CARET_BLINK_INTERVAL),
            move |_, _, _| {
                if selector.blink_caret() {
                    smithay::reexports::calloop::timer::TimeoutAction::ToDuration(
                        CARET_BLINK_INTERVAL,
                    )
                } else {
                    smithay::reexports::calloop::timer::TimeoutAction::Drop
                }
            },
        );
    }

    /// Close the editor, keeping the typed name when `commit`.
    fn finish_rename<B: crate::state::Backend>(&self, otto: &mut crate::Otto<B>, commit: bool) {
        let Some((index, value)) = self.end_editing() else {
            return;
        };
        if commit {
            let output = self.output_name.read().unwrap().clone();
            otto.workspaces
                .rename_workspace(&output, index, Some(value));
        }
        // The keyboard was ours for the duration of the edit — hand it back to
        // the workspace the user is looking at.
        //
        // Deferred to an idle callback because Enter and Escape arrive from
        // inside key delivery, which runs with the seat keyboard's internal
        // lock held: setting focus there takes the same lock and deadlocks the
        // compositor.
        let current = self.view.get_state().current;
        otto.handle.insert_idle(move |otto| {
            otto.focus_top_window_or_clear(current);
        });
    }

    /// Pointer location in this output's scene space. Events arrive in global
    /// logical coordinates; output subtrees render at the scene origin.
    fn scene_location(
        &self,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> Point {
        let origin = *self.output_origin.read().unwrap();
        let scale = self.view.get_state().scale as f64;
        Point::new(
            ((location.x - origin.0) * scale) as f32,
            ((location.y - origin.1) * scale) as f32,
        )
    }

    /// How many clicks in a row landed on `key`, according to the last press.
    fn click_count_for(&self, key: &str) -> u32 {
        self.last_click
            .read()
            .unwrap()
            .as_ref()
            .filter(|(last_key, _, _)| last_key == key)
            .map(|(_, _, count)| *count)
            .unwrap_or(0)
    }

    /// Translate a keysym into an edit the field understands. Keys with no
    /// meaning here return `None` and are swallowed (the grab is exclusive).
    fn key_for(keysym: Keysym, mods: &ModifiersState) -> Option<TextInputKey> {
        let key = match keysym {
            Keysym::Left => TextInputKey::Left,
            Keysym::Right => TextInputKey::Right,
            Keysym::Home => TextInputKey::Home,
            Keysym::End => TextInputKey::End,
            Keysym::BackSpace => TextInputKey::Backspace,
            Keysym::Delete => TextInputKey::Delete,
            Keysym::Return | Keysym::KP_Enter => TextInputKey::Enter,
            Keysym::Escape => TextInputKey::Escape,
            Keysym::a | Keysym::A if mods.ctrl => TextInputKey::SelectAll,
            _ if mods.ctrl || mods.alt || mods.logo => return None,
            _ => TextInputKey::Char(keysym.key_char()?),
        };
        Some(key)
    }
}

impl<Backend: crate::state::Backend> ViewInteractions<Backend> for WorkspaceSelectorView {
    fn id(&self) -> Option<usize> {
        self.view
            .layer
            .read()
            .unwrap()
            .as_ref()
            .map(|l| l.id.0.into())
    }

    fn is_alive(&self) -> bool {
        !self
            .view
            .layer
            .read()
            .unwrap()
            .as_ref()
            .map(|l| l.hidden())
            .unwrap_or(true)
    }
    /// Entering the strip carries a position but no motion event, so record it
    /// here too — a click right after the pointer arrives must not act on the
    /// last position the strip saw.
    fn on_enter(&self, event: &smithay::input::pointer::MotionEvent) {
        *self.cursor_location.write().unwrap() = self.scene_location(event.location);
    }

    fn on_motion(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        data: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        let state = self.view.get_state().clone();
        let location = self.scene_location(event.location);
        // A drag inside the rename field extends the selection, and the cursor
        // stays an I-beam over it.
        if let Some(edit_index) = self.editing_index() {
            let field_key = format!("workspace_selector_desktop_label_{edit_index}");
            if self.view.hover_layer(&field_key, &location) {
                self.field_pointer_drag(location.x);
                data.set_cursor(&CursorImageStatus::Named(CursorIcon::Text));
                *self.cursor_location.write().unwrap() = location;
                return;
            }
        }

        let mut hover = false;
        if self
            .view
            .hover_layer("workspace_selector_desktop_add", &location)
        {
            hover = true;
        }
        for w in state.workspaces.iter() {
            if self.is_removing(w.index) {
                continue;
            }
            if self.view.hover_layer(
                &format!("workspace_selector_desktop_{}", w.index),
                &location,
            ) {
                hover = true;
                break;
            }
            if self.view.hover_layer(
                &format!("workspace_selector_desktop_remove_{}", w.index),
                &location,
            ) {
                hover = true;
                break;
            }
        }

        if hover {
            let cursor = CursorImageStatus::Named(CursorIcon::Pointer);
            data.set_cursor(&cursor);
        } else {
            let cursor = CursorImageStatus::Named(CursorIcon::default());
            data.set_cursor(&cursor);
        }
        let mut cursor_location = self.cursor_location.write().unwrap();
        *cursor_location = location;
    }
    fn on_button(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        otto: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        let location = self.cursor_location.read().unwrap();
        let state = self.view.get_state().clone();
        let get_position_worspace_by_index = |index: usize| -> Option<usize> {
            state.workspaces.iter().position(|w| w.index == index)
        };
        let hovered_key = |loc: &Point| -> Option<String> {
            // check add first so it has priority over overlaps
            if self.view.hover_layer("workspace_selector_desktop_add", loc) {
                return Some("workspace_selector_desktop_add".to_string());
            }

            for w in state.workspaces.iter() {
                // A workspace collapsing out of the strip is not a target: it
                // is shrinking under the cursor and about to disappear.
                if self.is_removing(w.index) {
                    continue;
                }
                let remove_key = format!("workspace_selector_desktop_remove_{}", w.index);
                if self.view.hover_layer(&remove_key, loc) {
                    return Some(remove_key);
                }

                // The label is checked before the workspace it belongs to:
                // clicking it renames, clicking the preview switches.
                let label_key = format!("workspace_selector_desktop_label_{}", w.index);
                if self.view.hover_layer(&label_key, loc) {
                    return Some(label_key);
                }

                let workspace_key = format!("workspace_selector_desktop_{}", w.index);
                if self.view.hover_layer(&workspace_key, loc) {
                    return Some(workspace_key);
                }
            }
            None
        };

        match event.state {
            ButtonState::Pressed => {
                let key = hovered_key(&location);

                if let Some(edit_index) = self.editing_index() {
                    let field_key = format!("workspace_selector_desktop_label_{edit_index}");
                    if key.as_deref() == Some(field_key.as_str()) {
                        // Inside the field: place the caret, or select a word
                        // (2 clicks) / everything (3).
                        let count = self.register_click(&field_key);
                        let shift = self.modifiers.read().unwrap().shift;
                        self.field_pointer_down(location.x, count, shift);
                        *self.pressed_action.write().unwrap() = key;
                        return;
                    }
                    // Anywhere else commits, then the click does its usual job.
                    self.finish_rename(otto, true);
                }

                if let Some(key) = key.as_deref() {
                    self.register_click(key);
                }
                let mut pressed = self.pressed_action.write().unwrap();
                *pressed = key;
            }
            ButtonState::Released => {
                let release_key = hovered_key(&location);

                if self.is_editing() {
                    self.field_pointer_up();
                    *self.pressed_action.write().unwrap() = None;
                    return;
                }
                let mut pressed = self.pressed_action.write().unwrap();
                if let (Some(pressed_key), Some(release_key)) = (pressed.clone(), release_key) {
                    if pressed_key == release_key {
                        if release_key == "workspace_selector_desktop_add" {
                            // Add a workspace to THIS output only (workspaces are
                            // independent per output).
                            let name = self.output_name.read().unwrap().clone();
                            otto.workspaces.add_workspace_to_output(&name);
                        } else if let Some(index) = release_key
                            .strip_prefix("workspace_selector_desktop_remove_")
                            .and_then(|idx| idx.parse::<usize>().ok())
                        {
                            // Collapse it out of the strip; the post-render hook
                            // owns the animation and tells Otto to drop the
                            // workspace once the gap has closed.
                            self.begin_remove(index);
                        } else if let Some(index) = release_key
                            .strip_prefix("workspace_selector_desktop_label_")
                            .and_then(|idx| idx.parse::<usize>().ok())
                        {
                            if self.click_count_for(&release_key) >= 2 {
                                // Second click on the label: rename in place,
                                // and don't switch workspace under the editor.
                                self.begin_rename(index, otto, _seat, event.serial);
                            } else if let Some(pos) = get_position_worspace_by_index(index) {
                                otto.set_current_workspace_index(pos);
                            }
                        } else if let Some(index) = release_key
                            .strip_prefix("workspace_selector_desktop_")
                            .and_then(|idx| idx.parse::<usize>().ok())
                        {
                            // Navigate to workspace
                            if let Some(pos) = get_position_worspace_by_index(index) {
                                otto.set_current_workspace_index(pos);
                            }
                        }
                    }
                }
                *pressed = None;
            }
        }
    }

    fn on_modifiers(&self, modifiers: ModifiersState) {
        *self.modifiers.write().unwrap() = modifiers;
    }

    fn on_key_with_data(
        &self,
        event: &smithay::input::keyboard::KeysymHandle<'_>,
        key_state: KeyState,
        data: &mut crate::Otto<Backend>,
    ) {
        if key_state != KeyState::Pressed || !self.is_editing() {
            return;
        }
        let mods = *self.modifiers.read().unwrap();
        let Some(key) = Self::key_for(event.modified_sym(), &mods) else {
            return;
        };

        let response = {
            let mut editing = self.editing.write().unwrap();
            let Some(edit) = editing.as_mut() else {
                return;
            };
            edit.caret_visible = true;
            edit.input.on_key(
                key,
                KeyMods {
                    shift: mods.shift,
                    ctrl: mods.ctrl,
                },
            )
        };

        match response {
            TextInputResponse::Commit => self.finish_rename(data, true),
            TextInputResponse::Cancel => self.finish_rename(data, false),
            // Clipboard integration needs a data device on the seat; the field
            // itself is ready for it (see `TextInputResponse::Clipboard`).
            TextInputResponse::Clipboard(_) | TextInputResponse::Ignored => {}
            TextInputResponse::Changed | TextInputResponse::Moved => self.sync_edit_state(),
        }
    }

    /// Losing the keyboard (a window took focus, expose closed) commits what
    /// was typed rather than dropping it.
    fn on_keyboard_leave(&self) {
        if let Some((index, value)) = self.end_editing() {
            let output = self.output_name.read().unwrap().clone();
            let _ = self.rename_sender.send((output, index, value));
        }
    }
}
