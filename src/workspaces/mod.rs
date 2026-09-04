use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicI32},
        Arc, RwLock, Weak,
    },
};

pub use apps_info::Application;
use layers::{
    engine::{Engine, TransactionRef},
    prelude::{taffy, Interpolate, Layer, Spring, TimingFunction, Transition},
    skia::{self, Contains},
    types::Size,
};
use smithay::{
    desktop::{layer_map_for_output, Space, WindowSurface},
    output::Output,
    reexports::{
        calloop::channel::{channel, Sender as CalloopSender},
        wayland_server::{
            backend::{GlobalId, ObjectId},
            Resource,
        },
    },
    utils::{IsAlive, Rectangle},
};

use wayland_server::DisplayHandle;
use workspace::WorkspaceView;

mod app_icons_manager;
mod app_switcher;
mod background;
mod dnd_view;
mod dock;
mod osd;
mod popup_overlay;
mod tiling_overlay;
pub mod trash;
pub mod workspace;

pub mod utils;

mod apps_info;
mod window_selector;
mod window_view;
mod workspace_selector;

pub use background::BackgroundView;
pub use window_selector::{WindowSelectorView, WindowSelectorWindow};
pub use window_view::{
    resize_edges_at, WindowDecorationModel, WindowDecorationView, WindowResizeView, WindowView,
    WindowViewBaseModel, WindowViewSurface,
};

pub use app_icons_manager::AppIconsManager;
pub use app_switcher::AppSwitcherView;
pub use apps_info::ApplicationsInfo;
pub use dnd_view::DndView;
pub use dock::{DockModel, DockView};
pub use osd::OsdView;
pub use popup_overlay::PopupOverlayView;
pub use tiling_overlay::{zone_from_pointer, TileZone, TilingOverlayView};
pub use workspace::WORKSPACE_SPACING;
pub use workspace_selector::{WorkspaceSelectorView, WORKSPACE_SELECTOR_PREVIEW_WIDTH};

use crate::{
    config::Config,
    shell::WindowElement,
    utils::{natural_layout::LayoutRect, Observable, Observer},
};

/// The transition a workspace scroll animates with when the caller has no
/// opinion of its own: a spring the user sizes through `[workspaces]` in the
/// config (`switch_duration`, `switch_bounce`).
fn workspace_switch_transition() -> Transition {
    let spring = Config::with(|c| c.workspaces.switch_spring());
    Transition {
        delay: 0.0,
        timing: TimingFunction::Spring(spring),
    }
}

/// Per-output workspace set: each output has its own independent workspaces.
pub struct OutputWorkspaces {
    pub current_workspace: usize,
    pub spaces: Vec<Space<WindowElement>>,
    /// Per-output container layer (physical size, positioned at output's physical location).
    /// Dock and app_switcher are sublayers of the primary output's container.
    pub output_layer: Layer,
    pub workspaces_layer: Layer,
    /// Per-output expose layer (window overview mode).
    pub expose_layer: Layer,
    /// Per-output container for wlr-layer-shell *background* surfaces — the
    /// wallpaper. Mirrored into each workspace view and expose window selector.
    pub layer_shell_background: Layer,
    /// Per-output container for wlr-layer-shell *bottom* surfaces — the desktop
    /// widget layer (and, later, desktop icons). Kept apart from the wallpaper
    /// because it is not part of the exposé overview: it is mirrored only into
    /// the workspace views, and fades out with the rest of the chrome when the
    /// overview opens.
    pub layer_shell_bottom: Layer,
    pub workspace_views: Vec<Arc<WorkspaceView>>,
    /// Single layer containing all workspace backgrounds, child of workspaces_layer.
    /// Rendered as a single KMS background plane; scrolls in sync automatically.
    pub background_plane: Layer,
    /// Single layer containing all workspace window containers, child of workspaces_layer.
    /// Rendered as a single KMS windows plane; scrolls in sync automatically.
    pub windows_plane: Layer,
    /// Container for the one window promoted to its own KMS plane ("subtree
    /// plane"). A promoted window's `window_layer` is reparented here, out of
    /// its workspace's `windows_layer`, so the windows plane stops drawing it
    /// and this container can be rendered into a buffer of its own. Sits
    /// directly above `windows_plane` and mirrors the current workspace's
    /// `windows_layer` geometry, so reparenting does not move the window.
    /// Empty (and skipped) whenever nothing is promoted.
    pub promoted_plane: Layer,
    /// Overlay UI plane: workspace selector, layer_shell_top,
    /// layer_shell_overlay, OSD, DnD and popups — chrome above windows that
    /// changes rarely. Dock and app switcher have their own planes.
    pub overlay_plane: Layer,
    /// App-switcher plane: full-screen container (so the switcher centers
    /// itself with normal layout) rendered through a strip-sized viewport
    /// onto its own KMS plane. Above overlay_plane.
    pub switcher_plane: Layer,
    /// Dock plane: full-screen container (dock positions itself bottom-center
    /// with normal layout) rendered through a bottom-strip viewport onto its
    /// own KMS plane. Topmost.
    pub dock_plane: Layer,
    /// Session-lock plane: the blank and the locker's surface for this output.
    /// Above everything else, including the dock and fullscreen windows, and
    /// hidden whenever the session is unlocked. See `src/lock.rs`.
    pub lock_plane: Layer,
    /// Per-output workspace selector strip (expose UI). Each output shows its
    /// own selector so previews reflect that output's content at its own
    /// resolution. Lives in `overlay_plane`.
    pub workspace_selector: Arc<WorkspaceSelectorView>,
}

impl OutputWorkspaces {
    pub fn current_space(&self) -> &Space<WindowElement> {
        &self.spaces[self.current_workspace]
    }
    /// Whether the current workspace has any windows. Checked lookup — the
    /// index can briefly trail workspace removal.
    pub fn current_workspace_has_windows(&self) -> bool {
        self.workspace_views
            .get(self.current_workspace)
            .map(|ws| !ws.windows_list.read().unwrap().is_empty())
            .unwrap_or(false)
    }
    pub fn current_space_mut(&mut self) -> &mut Space<WindowElement> {
        &mut self.spaces[self.current_workspace]
    }
}

#[derive(Debug, Default, Clone)]
pub struct WorkspacesModel {
    workspace_counter: usize,
    pub workspaces: Vec<Arc<WorkspaceView>>,
    pub current_workspace: usize,
    /// Name of the output currently under the pointer (drives workspace selector display)
    pub focused_output_name: Option<String>,

    pub app_windows_map: HashMap<String, Vec<ObjectId>>,
    /// list of applications in the order they are visually displayed
    /// mainly used for the app switcher
    pub zindex_application_list: Vec<String>,
    /// list of applications in the order they are launched
    /// mainly used for the dock
    pub application_list: VecDeque<String>,

    pub minimized_windows: Vec<(ObjectId, String)>,
    pub current_application: usize,
    /// The physical width of the workspace
    pub width: i32,
    /// The physical height of the workspace
    pub height: i32,
    pub scale: f64,
}

/// An output whose DRM surface was torn down but whose `Output` is kept alive
/// for the reconnect — see [`Workspaces::suspend_output`].
pub struct SuspendedOutput {
    pub output: Output,
    pub location: smithay::utils::Point<i32, smithay::utils::Logical>,
    pub was_primary: bool,
    /// The output's `wl_output`. Dropping this is what withdraws the global,
    /// so it is held here for as long as the output may come back.
    pub global: Option<GlobalId>,
}

pub struct Workspaces {
    model: Arc<RwLock<WorkspacesModel>>,
    pub output_workspaces: HashMap<String, OutputWorkspaces>,
    outputs: Vec<Output>,
    primary_output: Option<Output>,
    /// Outputs suspended via `suspend_output` (lid close), keyed by name.
    /// Consumed on reconnect so the panel returns with the same `Output` — and
    /// the same `wl_output` — it went away with, in its pre-suspend
    /// arrangement rather than auto-placed after outputs (e.g. virtual ones)
    /// that kept running meanwhile.
    suspended_outputs: HashMap<String, SuspendedOutput>,
    display_handle: DisplayHandle,

    pub windows_map: HashMap<ObjectId, WindowElement>,
    /// Windows in the order they were last focused, most recent LAST.
    ///
    /// Per-workspace stacking order cannot answer "which window of this app did
    /// I use last?" once the app's windows live on different workspaces — every
    /// space has its own top. This list is the cross-workspace answer, and it
    /// is what the app switcher and the cycle-windows shortcut navigate by.
    focus_history: Vec<ObjectId>,
    // views
    pub dock: Arc<DockView>,
    pub app_switcher: Arc<AppSwitcherView>,
    /// Name of the output currently hosting the app switcher panel — its
    /// `wrap_layer` is a sublayer of that output's `switcher_plane`. `None`
    /// until first shown, which means "primary" (where it is parented at
    /// output-add time). See `place_app_switcher`.
    app_switcher_output: Arc<RwLock<Option<String>>>,
    pub window_views: Arc<RwLock<HashMap<ObjectId, WindowView>>>,
    pub dnd_view: DndView,
    pub popup_overlay: PopupOverlayView,
    pub osd: OsdView,
    pub tiling_overlay: TilingOverlayView,
    pub app_icons_manager: Arc<AppIconsManager>,

    // gestures states
    pub show_all: Arc<AtomicBool>,
    pub show_desktop: Arc<AtomicBool>,
    pub show_all_gesture: Arc<AtomicI32>,
    pub show_desktop_gesture: Arc<AtomicI32>,
    /// Tracks whether the workspace is currently animating (e.g., scrolling between workspaces)
    pub is_animating: Arc<AtomicBool>,
    /// Set while an expose open/close animation is in flight, and cleared when
    /// that animation finishes. Unlike `is_animating` (shared with workspace
    /// scrolling) and the gesture accumulator (which lands on its final value
    /// the moment the animation is *scheduled*), this stays true for the whole
    /// duration of the animation — which is what `is_expose_transitioning`
    /// needs, or scanout promotion resumes while the previews are still flying.
    expose_animating: Arc<AtomicBool>,
    /// The show-desktop counterpart of `expose_animating`: set while the
    /// mirrors are flying out or back, cleared when they land. The gesture
    /// accumulator commits to its final 0/1000 the moment the spring is
    /// *scheduled*, so without this every frame of the exit animation reads as
    /// "show desktop is over" and the render paths push the real windows plane
    /// back at once — the windows snap home while the mirrors are still moving.
    show_desktop_animating: Arc<AtomicBool>,
    /// Windows currently flagged for direct scanout (their `content_layer` is
    /// hidden; the client buffer is pushed as a `ScanoutCandidate` element).
    /// The render call-site diffs against this to re-import departing windows
    /// before they are composited again.
    scanout_windows: Arc<RwLock<HashSet<ObjectId>>>,
    /// Per-output desired scanout sets — the global `scanout_windows` set is
    /// the union of these (see `set_scanout_windows_for_output`).
    scanout_windows_per_output: Arc<RwLock<HashMap<String, HashSet<ObjectId>>>>,
    /// Per-output window promoted to its own KMS plane — the middle tier
    /// between raw client-buffer scanout and compositing into the shared
    /// windows plane. Its whole lay-rs subtree (client texture *plus* the
    /// compositor-drawn style: rounded clip, border, background, blur,
    /// SSD decorations, shadow) is rendered into a plane-sized buffer, so
    /// unlike raw scanout it is not limited to windows whose client buffer
    /// already describes the finished window. See `set_promoted_window`.
    promoted_windows: Arc<RwLock<HashMap<String, ObjectId>>>,
    /// Set when a promoted (scanned-out) window commits: the commit skips the
    /// scene import, so this flag is the only signal that a frame must render
    /// to submit the client's new buffer to its plane. Consumed (swapped to
    /// false) by the backend's should_draw check.
    pub scanout_commit_pending: Arc<std::sync::atomic::AtomicBool>,
    /// True while a 3-finger expose gesture is physically in progress (fingers on trackpad).
    pub expose_gesture_active: Arc<AtomicBool>,
    /// True while a workspace label is being renamed in the selector. Shared
    /// with every output's selector; the keyboard path checks it so typing a
    /// name never triggers a compositor shortcut.
    pub label_editing: Arc<AtomicBool>,

    // layers
    pub layers_engine: Arc<Engine>,
    pub overlay_layer: Layer,

    pub layer_shell_top: Layer,
    /// Container for wlr-layer-shell overlay layer surfaces  
    pub layer_shell_overlay: Layer,
    expose_layer: Layer,
    observers: Vec<Weak<dyn Observer<WorkspacesModel>>>,
    expose_dragged_window: Arc<std::sync::Mutex<Option<ObjectId>>>,
    remove_workspace_sender: CalloopSender<(Option<String>, usize)>,
    /// Carries `(output_name, workspace_index, name)` from a selector whose
    /// rename ended without a `&mut Otto` at hand (losing keyboard focus).
    rename_workspace_sender: CalloopSender<(String, usize, String)>,
}

/// # Workspaces Layer Structure
///
/// ```diagram
/// Workspaces
/// root
/// ├── layer_shell_background (per output, hidden — content source only, not rendered directly)
/// ├── output_layer (per output)
/// │   ├── workspaces
/// │   │   ├── workspace_view_1
/// │   │   │   ├── background_view (config-driven)
/// │   │   │   ├── layer_shell_bg_mirror (mirror: layer_shell_background)
/// │   │   │   └── workspace_windows_container_1
/// │   │   │       ├── window_view_1
/// │   │   │       ├── window_view_2
/// │   │   │       ...
/// │   │   ├── workspace_view_2
/// │   │   ...
/// │   ├── expose
/// │   │   ├── window_selector_root_1
/// │   │   │   ├── window_selector_background_1 (mirror: background_view)
/// │   │   │   ├── layer_shell_bg_expose_mirror (mirror: layer_shell_background)
/// │   │   │   ├── window_selector_windows_container_1
/// │   │   │   │   ├── mirror_window_1
/// │   │   │   │   ├── mirror_window_2
/// │   │   │   │   ...
/// │   │   │   ├── window_selector_view_1
/// │   │   ├── expose_view
/// │   │   ├── app_switcher
/// │   │
/// │   ├── dock (primary only)
/// │   ├── layer_shell_top (primary only)
/// │   ├── popup_overlay (primary only)
/// │   ├── layer_shell_overlay (primary only)
/// │   ├── overlay (primary only)
/// │   ├── workspace_selector_{output} (per output, in overlay_plane)
/// │   │   ├── workspace_selector_view_content
/// │   │   │   ├── workspace_selector_desktop_1
/// │   │   │   │   ├── workspace_selector_desktop_content_1
/// │   │   │   │   │   ├── workspace_selector_desktop_content_1 (mirror: workspace_view_1)
/// │   │   │   │   │   ├── workspace_selector_desktop_border_1
/// │   │   │   │   │   ├── workspace_selector_desktop_remove_1
/// │   │   │   │   ├── workspace_selector_desktop_label_1
/// │   │   │   ├── workspace_selector_desktop_2
/// │   │   │   │   ├── ...
/// │   │   │   ...
/// │   │   ├── workspace_selector_workspace_add
/// ```
///
/// Removal requests from a selector: `(Some(output), position)` for a
/// per-output removal, `(None, position)` for a lockstep one.
pub type RemoveWorkspaceChannel =
    smithay::reexports::calloop::channel::Channel<(Option<String>, usize)>;

/// Rename requests from a selector: `(output, workspace index, name)`.
pub type RenameWorkspaceChannel =
    smithay::reexports::calloop::channel::Channel<(String, usize, String)>;

/// The user-chosen name saved for workspace `position` on `output`, if any.
///
/// Names are keyed by position rather than by workspace identity: indices are
/// handed out by a counter that never repeats across restarts, so position is
/// the only thing that survives. Adding or removing a workspace shifts the
/// positions after it, and the names shift with them — the same trade-off every
/// position-keyed workspace name has.
fn persisted_workspace_name(output: &str, position: usize) -> Option<String> {
    Config::with(|c| {
        c.workspaces
            .names
            .get(&crate::config::workspace_name_key(output, position))
            .cloned()
    })
}

/// The outcome of one promotion pass over an output's windows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PlaneCandidates {
    /// Tier 1 — windows whose raw client buffer can go straight to a KMS
    /// plane. Zero GPU work per client frame, but only for windows whose
    /// buffer already describes the finished window.
    pub raw: Vec<ObjectId>,
    /// Tier 2 — the window whose lay-rs subtree is rendered into a plane
    /// buffer of its own. Costs one GPU pass per client frame; buys damage
    /// isolation from the shared windows plane, and works for windows tier 1
    /// cannot take (compositor-drawn style, decorations, SHM clients).
    pub subtree: Option<ObjectId>,
}

impl PlaneCandidates {
    /// Nothing promoted on this output this frame.
    pub fn none() -> Self {
        Self::default()
    }
}

/// Keep the window-content sampling in step with exposé.
///
/// The previews ARE the windows — the same layers, scaled by an ancestor
/// transform — and the sampling each surface is drawn with is baked into its
/// recorded picture, which lay-rs does not re-record on a pure scale change.
/// So flipping the flag has to be followed by forcing those pictures to be
/// recorded again. Cheap because the flag only changes twice per exposé.
fn apply_preview_sampling(
    window_views: &Arc<RwLock<HashMap<ObjectId, WindowView>>>,
    downscaled: bool,
) {
    if !crate::workspaces::utils::set_content_downscaled(downscaled) {
        return;
    }
    let Ok(views) = window_views.read() else {
        return;
    };
    for view in views.values() {
        crate::workspaces::utils::redraw_subtree(&view.content_layer);
    }
}

impl Workspaces {
    /// Re-draw everything that paints with `theme::accent_color()`.
    ///
    /// These views read the accent inside their render function rather than
    /// carrying it in their state, so `update_state` would hash to the same
    /// value and skip the render — the layer trees are rebuilt directly.
    /// Re-read the desktop background from the live configuration and push it
    /// to every workspace.
    ///
    /// Every workspace has its own `BackgroundView`, so a wallpaper that
    /// changed on one and not the others would show up the moment the user
    /// swiped sideways. Returns whether the image loaded — an unreadable path
    /// is reported rather than leaving the old wallpaper up and claiming
    /// success.
    pub fn reload_background(&self) -> Result<(), String> {
        let path = Config::with(|c| c.background_image.clone());
        let color = crate::utils::parse_hex_color(&Config::with(|c| c.background_color.clone()));

        let mut failed = false;
        for workspace in self.with_model(|m| m.workspaces.clone()) {
            workspace.background_view.set_fallback_color(color);
            if !workspace.background_view.set_image_path(&path) {
                failed = true;
            }
        }

        if failed {
            return Err(format!("cannot read a background image at `{path}`"));
        }
        Ok(())
    }

    pub fn rerender_accent_colored_views(&self) {
        for output in self.output_workspaces.values() {
            let selector = &output.workspace_selector;
            selector.view.render(&selector.layer);
        }
        for workspace in self.with_model(|m| m.workspaces.clone()) {
            let selector = &workspace.window_selector_view;
            selector.view.render(&selector.window_selector_view);
        }
        // The titlebar controls are tinted from the accent, by otto-kit.
        for view in self.window_views.read().unwrap().values() {
            view.rerender_decoration();
        }
    }

    pub fn start_window_selector_drag(&self, window_id: &ObjectId) {
        *self.expose_dragged_window.lock().unwrap() = Some(window_id.clone());
        // Hide the selection overlay while dragging; it is restored in end_window_selector_drag
        // once the animation completes.
        let num_workspaces = self.with_model(|m| m.workspaces.len());
        for i in 0..num_workspaces {
            if let Some(workspace_view) = self.get_workspace_at(i) {
                workspace_view
                    .window_selector_view
                    .window_selector_view
                    .set_opacity(0.0_f32, None);
            }
        }
        self.expose_update_if_needed();
    }

    pub fn is_window_selector_dragging(&self) -> bool {
        self.expose_dragged_window.lock().unwrap().is_some()
    }
    pub fn end_window_selector_drag(&self, window_id: &ObjectId) {
        let mut dragging = self.expose_dragged_window.lock().unwrap();
        if dragging.as_ref() == Some(window_id) {
            *dragging = None;
        }
        drop(dragging);
        self.expose_set_visible(true);
    }
    /// Returns a clone of the expose_dragged_window Arc for use in animation callbacks.
    pub fn expose_dragged_window_handle(&self) -> Arc<std::sync::Mutex<Option<ObjectId>>> {
        self.expose_dragged_window.clone()
    }
    pub fn new(
        layers_engine: Arc<Engine>,
        display_handle: DisplayHandle,
    ) -> (Self, RemoveWorkspaceChannel, RenameWorkspaceChannel) {
        let model = WorkspacesModel::default();

        let expose_layer = layers_engine.new_layer();
        expose_layer.set_key("expose");
        expose_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        expose_layer.set_size(layers::types::Size::percent(1.0, 1.0), None);
        expose_layer.set_pointer_events(false);
        expose_layer.set_hidden(true);
        expose_layer.set_picture_cached(false);
        expose_layer.set_image_cached(false);
        // attached to output_layer in map_output_with_primary

        let overlay_layer = layers_engine.new_layer();
        overlay_layer.set_key("overlay_view");
        overlay_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        overlay_layer.set_pointer_events(false);
        let dnd_view = DndView::new(layers_engine.clone());
        // dnd attached to overlay_layer in map_output_with_primary

        let app_icons_manager = Arc::new(AppIconsManager::new(layers_engine.clone()));

        let dock = DockView::new(layers_engine.clone(), app_icons_manager.clone());
        let dock = Arc::new(dock);
        // The greeter owns the whole screen: no dock, no switcher, no expose.
        if !crate::login::is_login_mode() {
            dock.show(None);
        }

        // Layer shell top layer (z-order: above workspaces)
        let layer_shell_top = layers_engine.new_layer();
        layer_shell_top.set_key("layer_shell_top");
        layer_shell_top.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        layer_shell_top.set_pointer_events(false);
        layer_shell_top.set_hidden(true);
        // attached to output_layer in map_output_with_primary

        // Create popup overlay AFTER dock so it renders on top
        let popup_overlay = PopupOverlayView::new(layers_engine.clone());

        let app_switcher = AppSwitcherView::new(layers_engine.clone(), app_icons_manager.clone());
        let app_switcher = Arc::new(app_switcher);

        // Layer shell overlay layer (z-order: above overlay_layer, below popups)
        let layer_shell_overlay = layers_engine.new_layer();
        layer_shell_overlay.set_key("layer_shell_overlay");
        layer_shell_overlay.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        layer_shell_overlay.set_pointer_events(false);
        layer_shell_overlay.set_hidden(true);
        // attached to output_layer in map_output_with_primary

        let (remove_workspace_sender, remove_receiver) = channel::<(Option<String>, usize)>();
        let (rename_workspace_sender, rename_receiver) = channel::<(String, usize, String)>();

        // Create OSD view; attach it to overlay_layer in map_output_with_primary
        let osd = OsdView::new(layers_engine.clone());

        // Window-tiling drop-zone overlay; attached to overlay_layer in map_output_with_primary
        let tiling_overlay = TilingOverlayView::new(layers_engine.clone());

        let mut workspaces = Self {
            // layer,
            output_workspaces: HashMap::new(),
            outputs: Vec::new(),
            primary_output: None,
            suspended_outputs: HashMap::new(),
            model: Arc::new(RwLock::new(model)),
            windows_map: HashMap::new(),
            focus_history: Vec::new(),
            expose_layer,
            app_switcher: app_switcher.clone(),
            app_switcher_output: Arc::new(RwLock::new(None)),
            dock: dock.clone(),
            dnd_view,
            popup_overlay,
            osd,
            tiling_overlay,
            app_icons_manager,
            overlay_layer,
            layer_shell_top,
            layer_shell_overlay,
            show_all: Arc::new(AtomicBool::new(false)),
            show_desktop: Arc::new(AtomicBool::new(false)),
            show_all_gesture: Arc::new(AtomicI32::new(0)),
            show_desktop_gesture: Arc::new(AtomicI32::new(0)),
            is_animating: Arc::new(AtomicBool::new(false)),
            expose_animating: Arc::new(AtomicBool::new(false)),
            show_desktop_animating: Arc::new(AtomicBool::new(false)),
            scanout_windows: Arc::new(RwLock::new(HashSet::new())),
            scanout_windows_per_output: Arc::new(RwLock::new(HashMap::new())),
            promoted_windows: Arc::new(RwLock::new(HashMap::new())),
            scanout_commit_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            expose_gesture_active: Arc::new(AtomicBool::new(false)),
            label_editing: Arc::new(AtomicBool::new(false)),
            window_views: Arc::new(RwLock::new(HashMap::new())),
            observers: Vec::new(),
            layers_engine,
            expose_dragged_window: Arc::new(std::sync::Mutex::new(None)),
            remove_workspace_sender,
            rename_workspace_sender,
            display_handle,
        };

        workspaces.add_listener(dock.clone());
        workspaces.add_listener(app_switcher.clone());
        (workspaces, remove_receiver, rename_receiver)
    }

    fn primary_output_name(&self) -> Option<String> {
        self.primary_output.as_ref().map(|o| o.name())
    }

    pub fn primary_output_workspaces(&self) -> Option<&OutputWorkspaces> {
        self.primary_output_name()
            .as_ref()
            .and_then(|n| self.output_workspaces.get(n))
    }

    pub fn primary_output_workspaces_mut(&mut self) -> Option<&mut OutputWorkspaces> {
        let name = self.primary_output_name()?;
        self.output_workspaces.get_mut(&name)
    }

    /// The output whose workspaces the flattened model currently mirrors
    /// (focused if set, else primary) — `sync_model_from_primary` fills
    /// `model.workspaces` from it, so expose/gesture code must use this
    /// output's spaces and dimensions.
    pub fn focused_output_workspaces(&self) -> Option<&OutputWorkspaces> {
        self.focused_output()
            .map(|o| o.name())
            .and_then(|n| self.output_workspaces.get(&n))
    }

    /// Name of the output currently hosting the app switcher panel, falling
    /// back to primary (where it is parented until first placed).
    pub fn app_switcher_output_name(&self) -> Option<String> {
        self.app_switcher_output
            .read()
            .unwrap()
            .clone()
            .filter(|n| self.output_workspaces.contains_key(n))
            .or_else(|| self.primary_output_name())
    }

    /// Whether this output is the one showing the app switcher panel. Only
    /// that output pushes a switcher plane and counts the panel as an
    /// occluder.
    pub fn is_app_switcher_output(&self, output: &Output) -> bool {
        self.app_switcher_output_name().as_deref() == Some(output.name().as_str())
    }

    /// Park the app switcher panel on the output it should appear on: the one
    /// under the pointer when `appswitcher.follow_cursor` is set, else the
    /// primary. Called just before the panel is shown, so a switcher already
    /// on screen never jumps mid-cycle.
    ///
    /// Re-parents `wrap_layer` into the target output's `switcher_plane` (the
    /// node the per-CRTC switcher plane element scans out) and hands the view
    /// that output's physical width and fractional scale, so the panel is
    /// sized for the screen it lands on rather than for the primary.
    pub fn place_app_switcher(&self) {
        let follow_cursor = Config::with(|c| c.appswitcher.follow_cursor);
        let target = if follow_cursor {
            self.focused_output().cloned()
        } else {
            self.primary_output.clone()
        };
        let Some(output) = target.or_else(|| self.primary_output.clone()) else {
            return;
        };
        let name = output.name();
        let Some(ows) = self.output_workspaces.get(&name) else {
            return;
        };
        {
            let mut host = self.app_switcher_output.write().unwrap();
            if host.as_deref() != Some(name.as_str()) {
                if let Err(err) = ows
                    .switcher_plane
                    .add_sublayer(&self.app_switcher.wrap_layer)
                {
                    tracing::warn!("app switcher re-parent to {name} failed: {err:?}");
                    return;
                }
                *host = Some(name.clone());
            }
        }
        let width_px = output
            .current_mode()
            .map(|m| m.size.w)
            .unwrap_or_else(|| self.with_model(|m| m.width));
        let scale = output.current_scale().fractional_scale() as f32;
        self.app_switcher.set_host_metrics(width_px, scale);
    }

    /// Is a workspace label being renamed right now?
    pub fn is_label_editing(&self) -> bool {
        self.label_editing
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Rename the workspace with global index `workspace_index` on `output`.
    /// An empty name clears the custom name, falling back to `Workspace N`.
    ///
    /// The new name is persisted keyed by position, so it comes back on the
    /// next start (see [`persisted_workspace_name`]).
    pub fn rename_workspace(&self, output: &str, workspace_index: usize, name: Option<String>) {
        let Some(ows) = self.output_workspaces.get(output) else {
            return;
        };
        let Some(workspace) = ows
            .workspace_views
            .iter()
            .find(|w| w.index == workspace_index)
        else {
            return;
        };
        workspace.set_custom_name(name);
        self.save_workspace_names();
        self.refresh_output_selectors();
    }

    /// Write every output's custom workspace names to the config, keyed by
    /// position. Called after a rename; positions that hold no custom name are
    /// simply absent.
    fn save_workspace_names(&self) {
        let mut names = std::collections::BTreeMap::new();
        for (output, ows) in self.output_workspaces.iter() {
            for (position, workspace) in ows.workspace_views.iter().enumerate() {
                if let Some(name) = workspace.get_custom_name() {
                    names.insert(crate::config::workspace_name_key(output, position), name);
                }
            }
        }
        crate::config::save_workspace_names(&names);
    }

    /// The workspace selector belonging to a given output.
    pub fn output_selector(&self, output_name: &str) -> Option<&Arc<WorkspaceSelectorView>> {
        self.output_workspaces
            .get(output_name)
            .map(|ows| &ows.workspace_selector)
    }

    /// The workspace selector on the focused (pointer) output — the one the
    /// user is interacting with (e.g. during an expose window drag).
    pub fn focused_output_selector(&self) -> Option<&Arc<WorkspaceSelectorView>> {
        self.focused_output_workspaces()
            .map(|ows| &ows.workspace_selector)
    }

    /// Repopulate every output's selector from its own workspace set, sized to
    /// that output's physical resolution. Called whenever workspaces, the
    /// current index, or output geometry change.
    fn refresh_output_selectors(&self) {
        let (fallback_w, fallback_h) = self.with_model(|m| (m.width as f32, m.height as f32));
        for (name, ows) in self.output_workspaces.iter() {
            let output = self.outputs.iter().find(|o| o.name() == *name);
            let (w, h) = output
                .and_then(|o| o.current_mode())
                .map(|m| (m.size.w as f32, m.size.h as f32))
                .unwrap_or((fallback_w, fallback_h));
            let scale = output
                .map(|o| o.current_scale().fractional_scale() as f32)
                .unwrap_or(1.0);
            let origin = output
                .map(|o| {
                    let loc = o.current_location();
                    (loc.x as f64, loc.y as f64)
                })
                .unwrap_or((0.0, 0.0));
            ows.workspace_selector.set_output_origin(origin);
            ows.workspace_selector.set_output_name(name);
            ows.workspace_selector.set_workspaces(
                &ows.workspace_views,
                ows.current_workspace,
                w,
                h,
                scale,
            );
        }
    }

    /// Get all spaces across all outputs and all workspaces (for window search)
    #[allow(dead_code)]
    fn all_spaces(&self) -> impl Iterator<Item = &Space<WindowElement>> {
        self.output_workspaces
            .values()
            .flat_map(|ows| ows.spaces.iter())
    }

    /// Get workspaces_layer for primary output (for animations/expose)
    pub fn primary_workspaces_layer(&self) -> Option<&Layer> {
        self.primary_output_workspaces()
            .map(|ows| &ows.workspaces_layer)
    }

    /// Sync model.workspaces and model.current_workspace from primary output
    fn sync_model_from_primary(&self) {
        // Use focused output if available, otherwise primary
        let focused_name = self.with_model(|m| m.focused_output_name.clone());
        let source_name = focused_name
            .as_deref()
            .filter(|n| self.output_workspaces.contains_key(*n))
            .map(|s| s.to_owned())
            .or_else(|| self.primary_output_name());

        if let Some(name) = source_name {
            if let Some(ows) = self.output_workspaces.get(&name) {
                let views = ows.workspace_views.clone();
                let cur = ows.current_workspace;
                self.with_model_mut(|m| {
                    m.workspaces = views;
                    m.current_workspace = cur;
                });
            }
        }
    }

    /// Set which output is currently focused (under the pointer).
    /// This drives the workspace selector display.
    pub fn set_focused_output(&self, output: Option<&Output>) {
        let name = output.map(|o| o.name());
        let changed = self.with_model(|m| m.focused_output_name != name);
        if !changed {
            return;
        }
        self.with_model_mut(|m| {
            m.focused_output_name = name;
        });
        self.sync_model_from_primary();
        self.with_model(|m| self.notify_observers(m));
    }

    pub fn space(&self) -> Option<&Space<WindowElement>> {
        self.primary_output_workspaces()
            .and_then(|ows| ows.spaces.get(ows.current_workspace))
    }

    pub fn space_mut(&mut self) -> Option<&mut Space<WindowElement>> {
        let name = self.primary_output_name()?;
        let ows = self.output_workspaces.get_mut(&name)?;
        let idx = ows.current_workspace;
        ows.spaces.get_mut(idx)
    }
    /// Set the workspace screen physical size
    pub fn set_screen_dimension(&self, width: i32, height: i32) {
        let scale = Config::with(|c| c.screen_scale);
        let current_workspace = self.with_model_mut(|model| {
            model.width = width;
            model.height = height;
            model.scale = scale;
            let event = model.clone();
            self.notify_observers(&event);
            model.current_workspace
        });

        self.update_workspaces_layout();
        self.scroll_to_workspace_index(current_workspace, Some(Transition::ease_out_quad(0.0)));
        self.refresh_dock_metrics_with(width, height);
    }

    /// Tell the dock how much room it has: the screen, and the part of it left
    /// over once the layer-shell exclusive zones (the top bar) are taken out.
    /// Call it whenever those zones change — the dock's icon-size cap and its
    /// resize limit are derived from them.
    ///
    /// The caller must not be holding a layer map guard: this takes one.
    pub fn refresh_dock_metrics(&self) {
        let (width, height) = self.with_model(|model| (model.width, model.height));
        self.refresh_dock_metrics_with(width, height);
    }

    fn refresh_dock_metrics_with(&self, width: i32, height: i32) {
        // The dock lives on the primary output; fall back to the whole screen
        // while there isn't one yet (early startup).
        let usable = self
            .primary_output()
            .map(|output| {
                let scale = output.current_scale().fractional_scale() as f32;
                let zone = layer_map_for_output(output).non_exclusive_zone();
                (
                    (zone.size.w as f32 * scale).round() as i32,
                    (zone.size.h as f32 * scale).round() as i32,
                )
            })
            .unwrap_or((width, height));
        self.dock.set_screen_size(width, height, usable);
    }

    pub fn get_logical_rect(&self) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
        self.with_model(|model| {
            let scale = model.scale as f32;
            smithay::utils::Rectangle::new(
                (0, 0).into(),
                (
                    ((model.width as f32 / scale) as i32),
                    ((model.height as f32 / scale) as i32),
                )
                    .into(),
            )
        })
    }
    // Data model management

    pub fn with_model<T>(&self, f: impl FnOnce(&WorkspacesModel) -> T) -> T {
        let model = self.model.read().unwrap();
        f(&model)
    }

    /// Re-run the multi-output layout pass — public entry for backends
    /// after mapping/unmapping an output without touching the flattened
    /// model's (primary) screen dimension.
    pub fn relayout_outputs(&self) {
        self.update_workspaces_layout();
    }

    fn update_workspaces_layout(&self) {
        let (primary_width, primary_height) =
            self.with_model(|model| (model.width as f32, model.height as f32));

        if primary_width <= 0.0 || primary_height <= 0.0 {
            return;
        }

        self.expose_layer
            .set_size(Size::points(primary_width, primary_height), None);

        // Output subtrees all live at scene (0,0) and OVERLAP: every output
        // renders only its own `output_layer` subtree (plane elements /
        // `for_output_layer`), so scene coordinates are output-local by
        // construction and nothing needs a per-output origin correction.
        // The outputs' positions in the GLOBAL space (smithay `Space`,
        // input, window locations) are unrelated to scene placement.
        let mut max_phys_w = 0.0f32;
        let mut max_phys_h = 0.0f32;
        for o in &self.outputs {
            if let Some(mode) = o.current_mode() {
                max_phys_w = max_phys_w.max(mode.size.w as f32);
                max_phys_h = max_phys_h.max(mode.size.h as f32);
            }
        }

        // Wallpapers are decoded for the largest output; when a bigger one
        // shows up (including the first one, at startup) the images loaded so
        // far are now being upscaled, so decode them again at the new size.
        if background::set_wallpaper_target_px(max_phys_w, max_phys_h) {
            let _ = self.reload_background();
        }

        // The scene root only needs to fit the largest output.
        if max_phys_w > 0.0 && max_phys_h > 0.0 {
            self.layers_engine.scene_set_size(max_phys_w, max_phys_h);
            if let Some(root) = self
                .layers_engine
                .scene_root()
                .and_then(|id| self.layers_engine.get_layer(&id))
            {
                root.set_size(Size::points(max_phys_w, max_phys_h), None);
            }
        }

        for (output_name, ows) in self.output_workspaces.iter() {
            let output = self.outputs.iter().find(|o| o.name() == *output_name);
            let (width, height) = output
                .and_then(|o| o.current_mode())
                .map(|m| (m.size.w as f32, m.size.h as f32))
                .unwrap_or((primary_width, primary_height));

            let w = if width > 0.0 { width } else { primary_width };
            let h = if height > 0.0 { height } else { primary_height };
            let scale = output
                .map(|o| o.current_scale().fractional_scale() as f32)
                .unwrap_or(1.0);
            // All output layers overlap at scene (0,0) — see above.
            ows.output_layer.set_position((0.0, 0.0), None);
            ows.output_layer.set_size(Size::points(w, h), None);

            ows.expose_layer.set_size(Size::points(w, h), None);
            ows.workspaces_layer.set_size(Size::points(w, h), None);
            ows.overlay_plane.set_size(Size::points(w, h), None);
            ows.switcher_plane.set_size(Size::points(w, h), None);
            ows.dock_plane.set_size(Size::points(w, h), None);
            ows.lock_plane.set_size(Size::points(w, h), None);
            // The switcher panel sizes itself from its host output — a mode or
            // scale change under it must re-render it at the new geometry.
            if self.app_switcher_output_name().as_deref() == Some(output_name.as_str()) {
                self.app_switcher.set_host_metrics(w as i32, scale);
            }

            for (logical_index, workspace) in ows.workspace_views.iter().enumerate() {
                workspace.update_layout(logical_index, w, h, scale);
                let selector_layer = workspace.window_selector_view.window_selector_root.clone();
                selector_layer.set_size(Size::points(w, h), None);
                let workspace_gap_px = WORKSPACE_SPACING * scale;
                selector_layer
                    .set_position((logical_index as f32 * (w + workspace_gap_px), 0.0), None);
            }
        }

        // Keep every output's selector previews in sync with the new sizes and
        // workspace set.
        self.refresh_output_selectors();
    }

    pub fn with_model_mut<T>(&self, f: impl FnOnce(&mut WorkspacesModel) -> T) -> T {
        let mut model = self.model.write().unwrap();
        f(&mut model)
    }

    // Gestures

    /// Check if the current workspace has a fullscreen surface and is ready for direct scanout.
    /// Returns true only when:
    /// - The current workspace is in fullscreen mode
    /// - The workspace is not animating (not scrolling between workspaces)
    /// - The fullscreen window is not animating
    /// - Not in expose/show-all mode
    /// - App switcher is not visible
    /// - The workspace has exactly one window (the fullscreen window only)
    pub fn is_fullscreen_and_stable(&self) -> bool {
        self.focused_output()
            .map(|o| self.is_fullscreen_and_stable_on_output(o))
            .unwrap_or(false)
    }

    /// Per-output fullscreen-scanout check: is this output's CURRENT workspace
    /// a stable fullscreen one (single window, no popups, nothing animating)?
    pub fn is_fullscreen_and_stable_on_output(&self, output: &Output) -> bool {
        // Check if expose mode is active
        if self.get_show_all() {
            return false;
        }

        // Check if the app switcher is visible on THIS output — the panel
        // follows the pointer, so one on another screen is irrelevant here.
        if self.app_switcher.is_visible() && self.is_app_switcher_output(output) {
            return false;
        }

        // Check if OSD (volume/brightness) is visible
        if self.osd.is_visible() {
            return false;
        }

        // Check if workspace is animating
        if self.is_animating.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }

        let Some(ows) = self.output_workspaces.get(&output.name()) else {
            return false;
        };
        let current_index = ows.current_workspace;
        let Some(current_workspace) = ows.workspace_views.get(current_index) else {
            return false;
        };
        if !current_workspace.get_fullscreen_mode() {
            return false;
        }

        // Check if the fullscreen window is still animating
        if current_workspace.get_fullscreen_animating() {
            return false;
        }

        // Check that the workspace has exactly one window (only the fullscreen window)
        // If there are additional windows (e.g., dialogs), disable direct scanout
        let Some(space) = ows.spaces.get(current_index) else {
            return false;
        };
        if space.elements().count() != 1 {
            return false;
        }

        // An open popup (menu, tooltip) renders in the overlay plane, which
        // fullscreen direct scanout drops entirely — the popup would be
        // invisible. Composite normally while any popup is mapped.
        let has_popups = space.elements().any(|w| {
            w.wl_surface()
                .map(|s| surface_has_mapped_popup(&s))
                .unwrap_or(false)
        });
        if has_popups {
            return false;
        }

        true
    }

    /// Get the fullscreen window from any output's current workspace, if any.
    pub fn get_fullscreen_window(&self) -> Option<WindowElement> {
        self.outputs
            .iter()
            .find_map(|o| self.get_fullscreen_window_on_output(o))
    }

    /// The fullscreen window on this output's CURRENT workspace, if any.
    pub fn get_fullscreen_window_on_output(&self, output: &Output) -> Option<WindowElement> {
        let ows = self.output_workspaces.get(&output.name())?;
        let current_workspace = ows.workspace_views.get(ows.current_workspace)?;
        if !current_workspace.get_fullscreen_mode() {
            return None;
        }
        ows.spaces
            .get(ows.current_workspace)?
            .elements()
            .find(|w| w.is_fullscreen())
            .cloned()
    }

    /// Return if we are in window selection mode
    pub fn get_show_all(&self) -> bool {
        self.show_all.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Check if expose mode is currently transitioning (either via gesture or animation)
    /// Returns true if we're in the middle of opening or closing expose mode
    pub fn is_expose_transitioning(&self) -> bool {
        let gesture_value = self
            .show_all_gesture
            .load(std::sync::atomic::Ordering::Relaxed);
        let is_animating = self.is_animating.load(std::sync::atomic::Ordering::Relaxed);

        // We're transitioning if:
        // 1. A finger gesture is in flight, whatever the accumulator reads, OR
        // 2. Gesture value is between 0 and 1000 (not fully closed or fully open), OR
        // 3. Animation is in progress AND we're not at a stable state (0 or 1000)
        //
        // (1) is not redundant with (2): `expose_gesture_start` sets the active
        // flag and resets the accumulator to exactly 0 (or 1000 when closing) in
        // the same breath — the two values (2) excludes — and a fast swipe then
        // saturates at the clamp and stays there for a run of frames. Without
        // (1), `expose_active` reads false for those frames while `show_all` has
        // not been committed yet, so the expose plane leaves the plane stack and
        // the windows plane is pushed instead: mid-gesture the screen flicks back
        // to the normal layout, then snaps to expose when the gesture ends. A slow
        // gesture hides the bug by spending most frames strictly inside (0,1000).
        //
        // The `expose_animating` clause covers the close (and open) animation
        // driven from a click or the keyboard: `expose_set_visible` commits the
        // accumulator to its final 0/1000 the moment it schedules the spring, so
        // clauses (2) and (3) both read false for every frame of that animation
        // and the windows would be promoted to scanout planes at their final
        // positions while the previews are still flying back to them — the exit
        // looks instantaneous. The finger gesture never hit this because
        // `expose_gesture_active` (1) stays set until the same animation ends.
        self.expose_gesture_active
            .load(std::sync::atomic::Ordering::Relaxed)
            || self
                .expose_animating
                .load(std::sync::atomic::Ordering::Relaxed)
            || (gesture_value > 0 && gesture_value < 1000)
            || (is_animating && gesture_value != 0 && gesture_value != 1000)
    }

    /// Returns true if we're in the middle of opening or closing show desktop mode
    pub fn is_show_desktop_transitioning(&self) -> bool {
        let gesture_value = self
            .show_desktop_gesture
            .load(std::sync::atomic::Ordering::Relaxed);
        let is_animating = self.is_animating.load(std::sync::atomic::Ordering::Relaxed);

        // We're transitioning if:
        // 1. A mirror animation is in flight (see `show_desktop_animating` —
        //    the only clause that covers a click- or keyboard-driven exit), OR
        // 2. Gesture value is between 0 and 1000 (not fully closed or fully open), OR
        // 3. Animation is in progress AND we're not at a stable state (0 or 1000)
        self.show_desktop_animating
            .load(std::sync::atomic::Ordering::Relaxed)
            || (gesture_value > 0 && gesture_value < 1000)
            || (is_animating && gesture_value != 0 && gesture_value != 1000)
    }

    /// True while the windows on screen are represented by their expose
    /// mirror layers rather than the real windows plane — exposé proper, the
    /// show-desktop gesture, or either transition. Render paths use this to
    /// drop the windows plane; testing only `get_show_all` left show-desktop
    /// drawing the untouched windows on top of the mirrors sliding away.
    pub fn mirrors_active(&self) -> bool {
        self.is_expose_transitioning()
            || self.get_show_all()
            || self.get_show_desktop()
            || self.is_show_desktop_transitioning()
    }

    /// Set the window selection mode
    #[allow(dead_code)]
    fn set_show_all(&self, show_all: bool) {
        self.show_all
            .store(show_all, std::sync::atomic::Ordering::Relaxed);
    }

    /// Return if we are in show desktop mode
    pub fn get_show_desktop(&self) -> bool {
        self.show_desktop.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Set the show desktop mode
    fn set_show_desktop(&self, show_all: bool) {
        self.show_desktop
            .store(show_all, std::sync::atomic::Ordering::Relaxed);
    }

    /// Set the mode to window selection mode using a delta for gestures
    ///
    /// # Arguments
    /// * `delta` - The incremental change value from a gesture:
    ///   - For continuous gestures (e.g., three-finger swipe): use incremental values (typically -1.0 to 1.0 range)
    ///   - For layout updates without mode change: use `0.0` or `1.0` as needed
    /// * `end_gesture` - Whether the gesture/action has completed:
    ///   - `true`: Finalize the transition with animations and state commitment (snaps to nearest state based on threshold)
    ///   - `false`: Track finger movement without animation smoothing (direct 1:1 response during gesture)
    ///
    /// # Behavior
    /// The function uses a hysteresis mechanism: you must swipe at least 10% to enter the mode,
    /// but must swipe back past 90% to exit when already active, preventing accidental toggles.
    ///
    /// # Usage Examples
    /// - Keyboard toggle on: `expose_set_visible(true)`
    /// - Keyboard toggle off: `expose_set_visible(false)`
    /// - Gesture update mid-swipe: `expose_update(0.05)` (5% progress increment)
    /// - Gesture completion: `expose_end()` (finalize at current position)
    /// - Update layout during window drag: `expose_show_all(0.0, false)` (recalculate without animation)
    pub fn expose_show_all(&self, delta: f32, end_gesture: bool) {
        let current_workspace_index = self.get_current_workspace_index();
        let num_workspaces = self.with_model(|m| m.workspaces.len());

        // Update all workspaces during gesture AND at end for consistent overlay visibility
        for i in 0..num_workspaces {
            let animated = end_gesture && i == current_workspace_index;
            self.expose_show_all_workspace(i, delta, end_gesture, animated);
        }
    }

    /// Update expose mode during a gesture (no animation).
    pub fn expose_update(&self, delta: f32) {
        tracing::trace!("Updating expose gesture with delta: {}", delta);
        let num_workspaces = self.with_model(|m| m.workspaces.len());
        for i in 0..num_workspaces {
            if let Some(workspace_view) = self.get_workspace_at(i) {
                let ol = &workspace_view.window_selector_view.window_selector_view;
                let before = ol.opacity();
                ol.set_opacity(0.0_f32, None);
                let after = ol.opacity();
                if before > 0.0 || after > 0.0 {
                    tracing::debug!(
                        "expose_update ws={} selector opacity before={} after={}",
                        i,
                        before,
                        after
                    );
                }
            }
        }
        self.expose_show_all(delta, false);
        // Log opacity after expose_show_all to catch if something inside sets it back
        for i in 0..num_workspaces {
            if let Some(workspace_view) = self.get_workspace_at(i) {
                let after = workspace_view
                    .window_selector_view
                    .window_selector_view
                    .opacity();
                if after > 0.0 {
                    tracing::debug!(
                        "expose_update ws={} selector opacity AFTER expose_show_all={}",
                        i,
                        after
                    );
                }
            }
        }
    }

    /// Start an expose gesture: reset accumulator and set initial layer visibility.
    ///
    /// Called once when the gesture direction is committed (vertical swipe detected).
    /// Sets layers to their correct initial state so that `expose_gesture_update` only
    /// needs to drive positions/opacities without toggling `set_hidden` every frame.
    ///
    /// - If expose is already open: selector stays visible (closing gesture).
    /// - If expose is closed: selector stays hidden until the open animation completes.
    pub fn expose_gesture_start(&self) {
        let current_show_all = self.get_show_all();
        self.expose_gesture_active
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Reset gesture accumulator to current state
        let reset_value = if current_show_all { 1000 } else { 0 };
        self.show_all_gesture
            .store(reset_value, std::sync::atomic::Ordering::Relaxed);

        // Expose background layer: always visible during gesture

        self.expose_layer.set_hidden(false);
        for ows in self.output_workspaces.values() {
            if ows.expose_layer != self.expose_layer {
                ows.expose_layer.set_hidden(false);
            }
        }
        self.refresh_expose_background_mirrors();

        // Each output owns its selector (already parented to its overlay
        // plane). Only visible if expose was already open.
        for ows in self.output_workspaces.values() {
            ows.workspace_selector.layer.set_hidden(!current_show_all);
        }

        // Suppress popups during gesture
        self.popup_overlay.set_hidden(true);

        // window_selector_root (.layer) must be visible during gesture and animation so that
        // window mirrors can animate to their expose positions.
        // overlay_layer (the selection UI) stays hidden until the open animation completes.
        let num_workspaces = self.with_model(|m| m.workspaces.len());
        for i in 0..num_workspaces {
            if let Some(workspace_view) = self.get_workspace_at(i) {
                workspace_view
                    .window_selector_view
                    .window_selector_root
                    .set_hidden(false);
                workspace_view
                    .window_selector_view
                    .window_selector_view
                    .set_opacity(0.0_f32, None);
                // Clear any leftover selection/state and force a fresh layout recalculation.
                workspace_view.window_selector_view.take_pre_close_hovered();
                workspace_view.window_selector_view.clear_selection();
                workspace_view.window_selector_view.invalidate_layout();
            }
            // Compute layout and trigger a view render so the selection overlay is ready
            // to show immediately when the open animation completes.
            self.expose_show_all_layout(i);
        }
        // Secondary outputs: lay out THEIR current workspace so expose
        // opens on every screen simultaneously.
        {
            let focused = self.focused_output().map(|o| o.name());
            let others: Vec<(String, usize)> = self
                .output_workspaces
                .iter()
                .filter(|(n, _)| focused.as_deref() != Some(n.as_str()))
                .map(|(n, ows)| (n.clone(), ows.current_workspace))
                .collect();
            for (name, ws) in others {
                self.expose_show_all_layout_for(&name, ws);
            }
        }

        // Hide the workspace content layers: during expose the windows are shown via mirror
        // layers inside the expose_layer, so the real workspace content doesn't need to be
        // visible.  They are restored by the on_finish callback when the closing animation ends.
        for ows in self.output_workspaces.values() {
            ows.workspaces_layer.set_hidden(true);
        }
    }

    /// Called once when a closing gesture is committed (expose is open, user swipes down).
    /// Resets the accumulator and immediately hides the selection overlay so it doesn't
    /// remain visible while the user drags downward.
    pub fn expose_gesture_close_start(&self) {
        self.expose_gesture_active
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // Reset gesture accumulator to fully-open position
        self.show_all_gesture
            .store(1000, std::sync::atomic::Ordering::Relaxed);

        // Cancel any in-flight fade-in and hide the overlay immediately
        let num_workspaces = self.with_model(|m| m.workspaces.len());
        for i in 0..num_workspaces {
            if let Some(workspace_view) = self.get_workspace_at(i) {
                tracing::debug!("wsv: gesture_close_start workspace={} → opacity=0", i);
                workspace_view
                    .window_selector_view
                    .window_selector_view
                    .set_opacity(0.0_f32, None);
                workspace_view.window_selector_view.save_pre_close_hovered();
                workspace_view.window_selector_view.clear_selection();
            }
        }
    }

    /// Reset the accumulated expose gesture value.
    /// Called when starting a new expose gesture to prevent accumulation.
    pub fn reset_expose_gesture(&self) {
        let current_state = self.show_all.load(std::sync::atomic::Ordering::Relaxed);
        let reset_value = if current_state { 1000 } else { 0 };
        self.show_all_gesture
            .store(reset_value, std::sync::atomic::Ordering::Relaxed);
    }

    /// Reset the accumulated show desktop gesture value.
    /// Called when starting a new show desktop gesture to prevent accumulation.
    pub fn reset_show_desktop_gesture(&self) {
        let current_state = self.show_desktop.load(std::sync::atomic::Ordering::Relaxed);
        let reset_value = if current_state { 1000 } else { 0 };
        self.show_desktop_gesture
            .store(reset_value, std::sync::atomic::Ordering::Relaxed);
    }

    /// Finalize expose gesture and snap to the nearest state.
    pub fn expose_end(&self) {
        self.expose_show_all(0.0, true);
    }

    /// Finalize expose gesture with velocity-based spring animation.
    /// The velocity from the gesture is used to initialize the spring's momentum.
    pub fn expose_end_with_velocity(&self, raw_velocity: f32) {
        use layers::prelude::*;

        const MULTIPLIER: f32 = 1000.0;
        let current_gesture = self
            .show_all_gesture
            .load(std::sync::atomic::Ordering::Relaxed);
        let current_show_all = self.get_show_all();
        let gesture_progress = current_gesture as f32 / MULTIPLIER;

        tracing::debug!(
            raw_velocity = raw_velocity,
            gesture_progress = gesture_progress,
            current_show_all = current_show_all,
            "Expose gesture ending with velocity"
        );

        // Calculate projected position based on velocity
        // TIME_CONSTANT represents how far into the future (in gesture units) we project
        const TIME_CONSTANT: f32 = 0.15;
        let projected_progress = gesture_progress + raw_velocity * TIME_CONSTANT;

        // Determine if gesture should complete based on:
        // 1. Current position threshold (10% to open, 90% to close)
        // 2. Velocity direction and magnitude
        // 3. Projected final position
        let should_complete = if current_show_all {
            // Currently in expose mode - deciding whether to close
            // Close if: gesture is < 50% OR (< 70% AND velocity is downward)
            let velocity_suggests_close = raw_velocity < -20.0;
            gesture_progress < 0.5
                || (gesture_progress < 0.7 && velocity_suggests_close)
                || projected_progress < 0.5
        } else {
            // Currently closed - deciding whether to open expose
            // Open if: gesture is > 50% OR (> 30% AND velocity is upward)
            let velocity_suggests_open = raw_velocity > 20.0;
            gesture_progress > 0.5
                || (gesture_progress > 0.3 && velocity_suggests_open)
                || projected_progress > 0.5
        };

        let target_show_all = if current_show_all {
            !should_complete // If should_complete, we're completing the close action
        } else {
            should_complete // If should_complete, we're completing the open action
        };

        // Scale velocity to spring units
        const VELOCITY_SCALE: f32 = 0.01;
        let spring_velocity = raw_velocity * VELOCITY_SCALE;

        // Create spring with initial velocity from gesture
        let spring = Spring::with_duration_bounce_and_velocity(
            0.3,             // duration
            0.1,             // bounce
            spring_velocity, // initial velocity from gesture
        );

        let transition = Transition {
            delay: 0.0,
            timing: TimingFunction::Spring(spring),
        };

        let current_workspace = self.get_current_workspace_index();
        // Use current delta so the spring animation can transition FROM current state TO target state
        let current_delta = if target_show_all { 1.0 } else { 0.0 };

        // Update show_all state immediately so next gesture starts from correct position
        self.show_all
            .store(target_show_all, std::sync::atomic::Ordering::Relaxed);

        // Reset gesture value to target state to prevent jumping on next gesture
        let target_gesture = if target_show_all { 1000 } else { 0 };
        self.show_all_gesture
            .store(target_gesture, std::sync::atomic::Ordering::Relaxed);

        // Gesture is finished — clear the active flag so on_finish callbacks can act
        self.expose_gesture_active
            .store(false, std::sync::atomic::Ordering::Relaxed);

        // Update all workspaces so they all transition together
        let num_workspaces = self.with_model(|m| m.workspaces.len());
        for i in 0..num_workspaces {
            let animated = i == current_workspace;
            let workspace_transition = if animated {
                Some(transition.clone())
            } else {
                None
            };
            self.expose_show_all_end(i, current_delta, target_show_all, workspace_transition);
        }
    }

    /// Redraw exposé's wallpaper mirrors on every workspace. See
    /// [`WindowSelectorView::refresh_background_mirrors`].
    pub fn refresh_expose_background_mirrors(&self) {
        for workspace in self.with_model(|m| m.workspaces.clone()) {
            workspace.window_selector_view.refresh_background_mirrors();
        }
    }

    /// Explicitly show or hide expose mode (keyboard toggle).
    pub fn expose_set_visible(&self, show: bool) {
        use layers::prelude::*;

        // Already in the target state — nothing to animate.
        // Avoids scheduling a no-op animation whose on_finish callback would
        // reset layer shell opacity, conflicting with fullscreen fade-outs.
        if show == self.get_show_all() {
            return;
        }

        // If show desktop is active, exit it first
        if self.get_show_desktop() && show {
            self.expose_show_desktop(-1.0, true);
            return;
        }

        // Set the gesture state to target value
        const MULTIPLIER: f32 = 1000.0;
        let target_gesture = if show { MULTIPLIER as i32 } else { 0 };
        self.show_all_gesture
            .store(target_gesture, std::sync::atomic::Ordering::Relaxed);
        self.show_all
            .store(show, std::sync::atomic::Ordering::Relaxed);

        // Create smooth spring transition (zero velocity for keyboard shortcuts)
        let spring = Spring::with_duration_and_bounce(0.3, 0.1);
        let transition = Transition {
            delay: 0.0,
            timing: TimingFunction::Spring(spring),
        };

        let current_workspace = self.get_current_workspace_index();
        let delta_normalized = if show { 1.0 } else { 0.0 };

        // Clear hover state on both open and close
        let num_workspaces = self.with_model(|m| m.workspaces.len());
        for i in 0..num_workspaces {
            if let Some(workspace_view) = self.get_workspace_at(i) {
                workspace_view.window_selector_view.clear_selection();
            }
        }

        // Lay out every output's current workspace (expose opens on all
        // screens together).
        if show {
            self.refresh_expose_background_mirrors();
            let layouts: Vec<(String, usize)> = self
                .output_workspaces
                .iter()
                .map(|(n, ows)| (n.clone(), ows.current_workspace))
                .collect();
            for (name, ws) in layouts {
                self.expose_show_all_layout_for(&name, ws);
            }
        }
        // When showing expose via keyboard, hide workspace content layers now (mirrors take over).
        self.expose_show_all_end(current_workspace, delta_normalized, show, Some(transition));
    }

    /// Process expose mode for a specific workspace
    /// Manages gesture state and delegates to layout/animation functions
    fn expose_show_all_workspace(
        &self,
        workspace_index: usize,
        delta: f32,
        end_gesture: bool,
        animated: bool,
    ) {
        tracing::trace!(
            workspace_index,
            delta,
            end_gesture,
            animated,
            "Processing expose gesture for workspace"
        );
        const MULTIPLIER: f32 = 1000.0;
        let gesture = self
            .show_all_gesture
            .load(std::sync::atomic::Ordering::Relaxed);

        let mut new_gesture = gesture + (delta * MULTIPLIER) as i32;
        let mut show_all = self.get_show_all();

        if end_gesture {
            if show_all {
                if new_gesture <= (9.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = 0;
                    show_all = false;
                } else {
                    new_gesture = MULTIPLIER as i32;
                    show_all = true;
                }
            } else {
                // animation_duration = 0.200;
                #[allow(clippy::collapsible_else_if)]
                if new_gesture >= (1.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = MULTIPLIER as i32;
                    show_all = true;
                } else {
                    new_gesture = 0;
                    show_all = false;
                }
            }
        }

        // Persist desired show_all state immediately on gesture end so that
        // follow-up calls (e.g. focus_app_with_window) don't re-open expose
        if end_gesture {
            self.show_all
                .store(show_all, std::sync::atomic::Ordering::Relaxed);
        }

        let delta_normalized = new_gesture as f32 / 1000.0;

        let transition = if animated {
            Some(Transition {
                delay: 0.0,
                timing: TimingFunction::Spring(Spring::with_duration_and_bounce(0.3, 0.1)),
            })
        } else {
            None
        };

        self.show_all_gesture
            .store(new_gesture, std::sync::atomic::Ordering::Relaxed);

        // Update/animate based on current state
        if end_gesture {
            self.expose_show_all_end(workspace_index, delta_normalized, show_all, transition);
        } else {
            self.expose_show_all_update(workspace_index, delta_normalized, show_all);
        }
    }

    /// Update the layout bin and window selector state for a workspace
    /// This ensures the bin has correct layout positions for all windows
    /// Returns true when a relayout was performed.
    fn expose_show_all_layout(&self, workspace_index: usize) -> bool {
        let Some(name) = self.focused_output().map(|o| o.name()) else {
            return false;
        };
        self.expose_show_all_layout_for(&name, workspace_index)
    }

    /// Compute the expose grid for one output's workspace. Bins and mirrors
    /// are per workspace view; geometry is output-local.
    fn expose_show_all_layout_for(&self, output_name: &str, workspace_index: usize) -> bool {
        let Some(ows) = self.output_workspaces.get(output_name) else {
            return false;
        };
        let Some(workspace) = ows.workspace_views.get(workspace_index).cloned() else {
            tracing::warn!("Workspace {} not found for expose layout", workspace_index);
            return false;
        };

        // FIXME: remove hardcoded values
        let workspace_selector_height = 250.0;
        let padding_top = 10.0;
        let padding_bottom = 10.0;

        let size = ows.workspaces_layer.render_size_transformed();
        // Per-output scale: the expose grid geometry is output-local physical.
        let scale = self
            .outputs
            .iter()
            .find(|o| o.name() == output_name)
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or_else(|| Config::with(|c| c.screen_scale));
        let screen_size_w = size.x;
        let screen_size_h = size.y - padding_top - padding_bottom - workspace_selector_height;

        let offset_y = 200.0;
        let layout_rect = LayoutRect::new(
            0.0,
            workspace_selector_height,
            screen_size_w,
            screen_size_h - offset_y,
        );
        let dragging_window = self.expose_dragged_window.lock().unwrap().clone();
        let origin = self
            .outputs
            .iter()
            .find(|o| o.name() == output_name)
            .map(|o| o.current_location())
            .unwrap_or_default();
        let mut windows = {
            {
                let windows_list = workspace.windows_list.read().unwrap();
                tracing::debug!(target: "otto::expose",
                    "layout out={} ws={} list_len={}",
                    output_name, workspace_index, windows_list.len());
                let Some(space) = ows.spaces.get(workspace_index) else {
                    return false;
                };
                // Space geometry is global; the selector containers live in
                // the output's local scene space.
                let mut windows = Vec::new();

                for window_id in windows_list.iter() {
                    if dragging_window.as_ref() == Some(window_id) {
                        continue;
                    }
                    if let Some(window) = self.get_window_for_surface(window_id) {
                        if window.is_minimised() {
                            continue;
                        }
                        if let Some(mut bbox) = space.element_geometry(window) {
                            bbox.loc -= origin;
                            let bbox = bbox.to_f64().to_physical(scale);
                            window.mirror_layer().set_size(
                                Size::points(bbox.size.w as f32, bbox.size.h as f32),
                                None,
                            );
                            windows.push(WindowSelectorWindow {
                                id: window_id.clone(),
                                rect: LayoutRect::new(
                                    bbox.loc.x as f32,
                                    bbox.loc.y as f32,
                                    bbox.size.w as f32,
                                    bbox.size.h as f32,
                                ),
                                title: window.xdg_title().to_string(),
                            });
                        }
                    }
                }

                windows
            }
        };

        // Snapshot the pre-expose stacking order the first time layout runs
        // while expose is active (or the gesture is starting).  This avoids
        // capturing a stale snapshot during normal window-mapping which also
        // calls expose_show_all_layout.
        let expose_active = self.get_show_all()
            || self
                .expose_gesture_active
                .load(std::sync::atomic::Ordering::Relaxed);
        if expose_active && workspace.peek_pre_expose_order_empty() {
            if let Some(space) = self
                .focused_output_workspaces()
                .and_then(|ows| ows.spaces.get(workspace_index))
            {
                let order: Vec<ObjectId> = space.elements().map(|e| e.id()).collect();
                workspace.save_pre_expose_order(order);
            }
        }

        // Keep expose layout stable regardless of runtime z-order changes.
        // Window stacking may change (hover raise/focus), but tiling positions should not.
        windows.sort_by_key(|w| w.id.protocol_id());

        // Skip relayout if window set and geometry match previous layout
        if workspace
            .window_selector_view
            .is_layout_up_to_date(&layout_rect, offset_y, &windows)
        {
            return false;
        }

        workspace
            .window_selector_view
            .update_windows(layout_rect, offset_y, &windows);
        true
    }

    /// Update expose state during gesture (no animation).
    fn expose_show_all_update(&self, workspace_index: usize, delta: f32, show_all: bool) {
        self.expose_show_all_apply(workspace_index, delta, None, show_all, false);
    }

    /// Finalize expose state and animate to the target.
    fn expose_show_all_end(
        &self,
        workspace_index: usize,
        delta: f32,
        show_all: bool,
        transition: Option<Transition>,
    ) {
        let velocity = if let Some(Transition {
            timing: TimingFunction::Spring(spring),
            ..
        }) = &transition
        {
            spring.initial_velocity
        } else {
            0.0
        };

        tracing::debug!(
            workspace = workspace_index,
            delta = delta,
            show_all = show_all,
            animated = transition.is_some(),
            velocity = velocity,
            "Ending expose show all gesture animation"
        );
        self.expose_show_all_apply(workspace_index, delta, transition, show_all, true);
    }

    /// Apply expose window positions and UI elements based on current delta and state.
    fn expose_show_all_apply(
        &self,
        workspace_index: usize,
        delta: f32,
        transition: Option<Transition>,
        show_all: bool,
        end_gesture: bool,
    ) {
        let delta = delta.clamp(0.0, 1.0);
        let is_gesture_ongoing = delta > 0.0 && delta < 1.0 && !end_gesture;
        let is_starting_animation = transition.is_some();
        // expose_layer and workspaces_layer are mutually exclusive:
        // expose must stay visible for the full duration of a gesture (even at delta=0) and during
        // any transition.  It is only safe to hide expose (and show workspaces) once end_gesture
        // is true AND delta has landed at 0 AND no closing animation is running.
        let show_expose = !end_gesture || delta > 0.0 || transition.is_some();

        // Exposé draws the windows downscaled by scaling their layers, which the
        // per-surface sampling gate cannot see. Tell it, so the previews are
        // resampled rather than point sampled — see
        // [`crate::workspaces::utils::set_content_downscaled`]. The flag is
        // lowered again by the animation's `on_finish` below, once the windows
        // are back at 1:1.
        apply_preview_sampling(&self.window_views, show_expose || show_all);

        // Check if this is the current workspace early, so we can use it for window animations
        let current_workspace_index = self.get_current_workspace_index();
        let is_current_workspace = workspace_index == current_workspace_index;

        // Popups belong to the normal desktop: keep them hidden for the whole
        // expose lifetime (gesture, open/close animation and while expose is
        // open). The on_finish below restores them once expose is fully closed.
        self.popup_overlay
            .set_hidden(is_gesture_ongoing || show_expose);
        let scale = Config::with(|c| c.screen_scale);

        let offset_y = 200.0;
        let mut changes = Vec::new();
        let Some(workspace_view) = self.get_workspace_at(workspace_index) else {
            tracing::warn!(
                "Workspace {} not found for expose animation",
                workspace_index
            );
            return;
        };
        let dragged_window = self.expose_dragged_window.lock().unwrap().clone();

        // window_selector_root (.layer) must be visible during gesture and animation.
        workspace_view
            .window_selector_view
            .window_selector_root
            .set_hidden(false);
        workspace_view
            .window_selector_view
            .window_selector_windows_container
            .set_hidden(false);

        // overlay_layer (selection UI) stays hidden throughout; revealed only in on_finish when open.
        let window_selector_view = workspace_view
            .window_selector_view
            .window_selector_view
            .clone();
        if is_starting_animation {
            tracing::debug!(target: "otto::popups", "is_animating(true) site=selector-start");
        }
        self.is_animating
            .store(is_starting_animation, std::sync::atomic::Ordering::Relaxed);

        window_selector_view.set_opacity(0.0_f32, None);

        // Create animation if transition is specified
        let animation = transition
            .as_ref()
            .map(|t| self.layers_engine.add_animation_from_transition(t, false));

        // Animate window layers on EVERY output: expose opens and closes on
        // all outputs together, each showing its own current workspace.
        // `workspace_index` addresses the focused output's workspace; other
        // outputs animate their own current workspace.
        let focused_name = self.focused_output().map(|o| o.name());
        for (output_name, ows) in self.output_workspaces.iter() {
            let is_focused_output = focused_name.as_deref() == Some(output_name.as_str());
            let ws_idx = if is_focused_output {
                workspace_index
            } else {
                ows.current_workspace
            };
            let Some(workspace) = ows.workspace_views.get(ws_idx) else {
                continue;
            };
            let Some(space) = ows.spaces.get(ws_idx) else {
                continue;
            };
            let (origin, out_scale) = self
                .outputs
                .iter()
                .find(|o| o.name() == *output_name)
                .map(|o| (o.current_location(), o.current_scale().fractional_scale()))
                .unwrap_or_else(|| (Default::default(), scale));
            let window_selector = workspace.window_selector_view.clone();
            // The gesture path only unhides the focused output's views —
            // secondary outputs need theirs visible too.
            window_selector.window_selector_root.set_hidden(false);
            window_selector
                .window_selector_windows_container
                .set_hidden(false);
            let ws_bin = window_selector.expose_bin.read().unwrap();
            let windows_list = workspace.windows_list.read().unwrap().clone();
            // Focused output keeps the old semantics (animate only its
            // current workspace); secondary outputs always animate theirs.
            let animate_this = transition.is_some() && (is_current_workspace || !is_focused_output);
            for window_id in windows_list.iter() {
                if dragged_window.as_ref() == Some(window_id) {
                    continue;
                }
                if let Some(window) = self.get_window_for_surface(window_id) {
                    if window.is_minimised() {
                        continue;
                    }
                    if let Some(mut bbox) = space.element_geometry(window) {
                        bbox.loc -= origin;
                        let bbox = bbox.to_f64().to_physical(out_scale);
                        if let Some(rect) = ws_bin.get(window_id) {
                            let to_x = rect.x;
                            let to_y = rect.y + offset_y;
                            let to_width = rect.width;
                            let to_height = rect.height;
                            let (window_width, window_height) =
                                (bbox.size.w as f32, bbox.size.h as f32);

                            let scale_x = to_width / window_width;
                            let scale_y = to_height / window_height;
                            let target_scale = scale_x.min(scale_y).min(1.0);

                            // Interpolate between current and target positions
                            let scale = 1.0.interpolate(&target_scale, delta);
                            let delta_clamped = delta.clamp(0.0, 1.0);
                            let window_x = bbox.loc.x as f32;
                            let window_y = bbox.loc.y as f32;
                            let x = window_x.interpolate(&to_x, delta_clamped);
                            let y = window_y.interpolate(&to_y, delta_clamped);

                            if let Some(layer) = window_selector.layer_for_window(window_id) {
                                if animate_this {
                                    let translation =
                                        layer.change_position(layers::types::Point { x, y });
                                    let scale_change = layer
                                        .change_scale(layers::types::Point { x: scale, y: scale });
                                    changes.push(translation);
                                    changes.push(scale_change);
                                } else {
                                    // Non-current workspaces: instant update without animation
                                    layer.set_position(layers::types::Point { x, y }, None);
                                    layer.set_scale(
                                        layers::types::Point { x: scale, y: scale },
                                        None,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        let current_workspace = self.get_workspace_at(workspace_index);

        // Schedule layer changes with animation
        if let Some(anim_ref) = animation {
            let _transactions = self.layers_engine.schedule_changes(&changes, anim_ref);
        };

        // Animate workspace selector and dock (only affects UI when is_current_workspace)
        let delta = delta.max(0.0);

        if is_current_workspace {
            // Workspace selector
            let workspace_selector_y = (-400.0).interpolate(&0.0, delta);
            let workspace_selector_y = workspace_selector_y.clamp(-400.0, 0.0);

            // Layer shell overlay and top fade out when entering expose.
            // When closing expose on a fullscreen workspace the resting opacity
            // must stay at 0.0 (fullscreen hides the top bar / overlay).
            let current_ws_fullscreen = self
                .get_workspace_at(current_workspace_index)
                .map(|ws| ws.get_fullscreen_mode())
                .unwrap_or(false);
            let layer_shell_resting_opacity = if current_ws_fullscreen { 0.0 } else { 1.0 };
            let layer_shell_fade_opacity = layer_shell_resting_opacity.interpolate(&0.0, delta);
            let layer_shell_fade_opacity = layer_shell_fade_opacity.clamp(0.0, 1.0);

            // Set overlay opacity to match the workspace selector opacity (fade in as we enter expose)

            let window_selector_view_ref = window_selector_view.clone();
            let expose_layer = self.expose_layer.clone();
            let layer_shell_overlay_ref = self.layer_shell_overlay.clone();
            let layer_shell_top_ref = self.layer_shell_top.clone();
            // The widget layer fades with the chrome, but its mirrors live one
            // per workspace view rather than in a single container, so they are
            // collected here and driven as a group.
            // Exposé's copies, not the workspace ones. `workspaces_layer` —
            // which owns the workspace copy — is hidden outright as soon as
            // the gesture starts, so fading that one animates a layer nobody
            // can see. The exposé mirror is what stays on screen through the
            // transition, so that is what has to fade.
            let layer_shell_bottom_mirrors: Vec<Layer> = self
                .output_workspaces
                .values()
                .flat_map(|ows| ows.workspace_views.iter())
                .map(|workspace| {
                    workspace
                        .window_selector_view
                        .layer_shell_bottom_expose_mirror
                        .clone()
                })
                .collect();
            let layer_shell_bottom_mirrors_ref = layer_shell_bottom_mirrors.clone();
            let show_all_ref = self.show_all.clone();
            let show_all_gesture_ref = self.show_all_gesture.clone();
            let expose_gesture_active_ref = self.expose_gesture_active.clone();
            let expose_dragged_window_ref = self.expose_dragged_window.clone();
            let popup_overlay_layer = self.popup_overlay.layer.clone();
            let model_ref = self.model.clone();
            let window_views_ref = self.window_views.clone();

            // Collect secondary output expose layers to sync with primary
            let secondary_expose_layers: Vec<Layer> = self
                .output_workspaces
                .values()
                .filter(|ows| ows.expose_layer != expose_layer)
                .map(|ows| ows.expose_layer.clone())
                .collect();

            // Collect all output workspaces_layers so the on_finish callback can restore them
            let all_workspaces_layers: Vec<Layer> = self
                .output_workspaces
                .values()
                .map(|ows| ows.workspaces_layer.clone())
                .collect();

            // Collect all overlay_layers (selection UI) so on_finish can show them when expose opens
            let all_window_selector_views: Vec<Layer> = self
                .with_model(|m| m.workspaces.clone())
                .iter()
                .map(|wv| wv.window_selector_view.window_selector_view.clone())
                .collect();

            expose_layer.set_hidden(!show_expose);
            for el in &secondary_expose_layers {
                el.set_hidden(!show_expose);
            }
            // workspaces_layer and expose_layer are mutually exclusive alternatives.
            // Keep them in sync: when expose is visible, workspace content is hidden, and vice versa.
            for wl in &all_workspaces_layers {
                wl.set_hidden(show_expose);
            }

            // Slide every output's selector strip into place together. Capture
            // one representative transaction to attach the shared on_finish to.
            let selector_layers: Vec<Layer> = self
                .output_workspaces
                .values()
                .map(|ows| ows.workspace_selector.layer.clone())
                .collect();
            let mut selector_transaction = None;
            for layer in &selector_layers {
                layer.set_hidden(false);
                let tr = layer.set_position(
                    layers::types::Point {
                        x: 0.0,
                        y: workspace_selector_y,
                    },
                    transition.clone(),
                );
                if selector_transaction.is_none() {
                    selector_transaction = Some(tr);
                }
            }
            if let (true, Some(transaction)) = (transition.is_some(), selector_transaction) {
                window_selector_view_ref.set_position((0.0, 0.0), None);
                let is_animating_ref = self.is_animating.clone();
                // Set here rather than next to `is_animating` above so the flag
                // can only ever be raised together with the `on_finish` that
                // lowers it again — a raised flag with no clearer would block
                // scanout promotion for the rest of the session.
                self.expose_animating
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                let expose_animating_ref = self.expose_animating.clone();
                transaction.on_finish(
                    move |_: &Layer, _: f32| {
                        // The expose open/close animation is over. Clear the
                        // flag BEFORE any early bail below — leaving it set
                        // wedges `is_animating` true forever (line ~1333 sets
                        // it for every animated apply and nothing else clears
                        // it), permanently blocking scanout promotion. A
                        // reversed gesture schedules a new animation which
                        // sets the flag again, so clearing here is safe.
                        is_animating_ref.store(false, std::sync::atomic::Ordering::Relaxed);
                        expose_animating_ref.store(false, std::sync::atomic::Ordering::Relaxed);
                        let is_dragging =
                            expose_dragged_window_ref.lock().unwrap().is_some();
                        // Re-read the current state at finish time — the gesture may have
                        // reversed since this animation started.
                        let current_show_all =
                            show_all_ref.load(std::sync::atomic::Ordering::Relaxed);
                        let gesture_value =
                            show_all_gesture_ref.load(std::sync::atomic::Ordering::Relaxed);
                        let gesture_at_rest = gesture_value == 0 || gesture_value == 1000;
                        let gesture_active =
                            expose_gesture_active_ref.load(std::sync::atomic::Ordering::Relaxed);
                        tracing::debug!("wsv: on_finish check: show_all={} current={} gesture={} at_rest={} gesture_active={}", show_all, current_show_all, gesture_value, gesture_at_rest, gesture_active);
                        // Bail out if state changed, gesture still in progress, or gesture is active.
                        if current_show_all != show_all || !gesture_at_rest || gesture_active {
                            tracing::debug!("wsv: on_finish bailed");
                            return;
                        }
                        // Reveal or hide the selection overlay based on final state,
                        // but not while a window drag is in progress.
                        if !is_dragging {
                            for ol in &all_window_selector_views {
                                if show_all {
                                    tracing::debug!("wsv: on_finish(open) → opacity=1.0 fade");
                                    let fade_in = Transition {
                                        delay: 0.05,
                                        timing: TimingFunction::ease_in_out(0.2),
                                    };
                                    ol.set_opacity(1.0_f32, Some(fade_in));
                                } else {
                                    tracing::debug!("wsv: on_finish(close) → opacity=0");
                                    ol.set_opacity(0.0_f32, None);
                                }
                            }
                        } else {
                            tracing::debug!("wsv: on_finish skipped (dragging)");
                        }
                        expose_layer.set_hidden(!show_all);
                        for el in &secondary_expose_layers {
                            el.set_hidden(!show_all);
                        }
                        // Popups only come back once expose is fully closed.
                        popup_overlay_layer.set_hidden(show_all);
                        // workspace_selector_view stays visible (positioned off-screen when closed)
                        // Restore workspace content layers when closing expose
                        for wl in &all_workspaces_layers {
                            wl.set_hidden(show_all);
                        }
                        // Restore layer shell overlay and top when exiting expose mode,
                        // but only if the current workspace isn't fullscreen (which has
                        // its own opacity management via set_fullscreen_overlay_visibility).
                        let is_fullscreen = model_ref
                            .read()
                            .ok()
                            .and_then(|m| {
                                m.workspaces
                                    .get(m.current_workspace)
                                    .map(|ws| ws.get_fullscreen_mode())
                            })
                            .unwrap_or(false);
                        if !is_fullscreen {
                            layer_shell_overlay_ref.set_opacity(if show_all { 0.0_f32 } else { 1.0_f32 }, None);
                            layer_shell_top_ref.set_opacity(if show_all { 0.0_f32 } else { 1.0_f32 }, None);
                            layer_shell_top_ref.set_hidden(show_all);
                            for mirror in layer_shell_bottom_mirrors_ref.iter() {
                                mirror.set_opacity(if show_all { 0.0_f32 } else { 1.0_f32 }, None);
                                mirror.set_hidden(show_all);
                            }
                        } else {
                            // Fullscreen: keep layers hidden and transparent
                            layer_shell_top_ref.set_hidden(true);
                        }

                        show_all_ref.store(show_all, std::sync::atomic::Ordering::Relaxed);
                        // The windows are at their final scale now: 1:1 again
                        // when expose closed, still downscaled when it opened.
                        apply_preview_sampling(&window_views_ref, show_all);
                    },
                    true,
                );
            }
            for layer in &selector_layers {
                layer.set_opacity(1.0_f32, None);
            }

            // Animate layer shell overlay and top opacity (fade out when entering expose)
            if layer_shell_fade_opacity > 0.0 {
                self.layer_shell_top.set_hidden(false);
            }
            self.layer_shell_overlay
                .set_opacity(layer_shell_fade_opacity, transition.clone());
            self.layer_shell_top
                .set_opacity(layer_shell_fade_opacity, transition.clone());
            tracing::debug!(
                target: "otto::fade",
                "delta={delta:.3} opacity={layer_shell_fade_opacity:.3} show_all={show_all} \
                 end={end_gesture} animated={} mirrors={} hidden={:?}",
                transition.is_some(),
                layer_shell_bottom_mirrors.len(),
                layer_shell_bottom_mirrors
                    .iter()
                    .map(|m| m.hidden())
                    .collect::<Vec<_>>(),
            );
            for mirror in layer_shell_bottom_mirrors.iter() {
                if layer_shell_fade_opacity > 0.0 {
                    mirror.set_hidden(false);
                }
                mirror.set_opacity(layer_shell_fade_opacity, transition.clone());
            }
            // When fully faded out without an ongoing animation, hide immediately
            if layer_shell_fade_opacity == 0.0 && transition.is_none() {
                self.layer_shell_top.set_hidden(true);
                for mirror in layer_shell_bottom_mirrors.iter() {
                    mirror.set_hidden(true);
                }
            }
        }
        // Animate dock position
        if let Some(current_workspace) = current_workspace {
            if !is_current_workspace {
                return;
            }
            // When autohide is on and the dock is visible, animate it down with the expose
            // gesture (same interpolation as the non-autohide path). The hot zone is suppressed
            // during expose (see check_dock_hot_zone).
            if self.dock.is_autohide_enabled() && !self.dock.is_hidden() {
                let dock_slide = 0.0_f32.interpolate(&250.0, delta);
                self.dock.view_layer.set_position(
                    self.dock.slide_position(dock_slide.clamp(0.0, 250.0)),
                    transition,
                );
                if end_gesture && show_all {
                    self.dock.schedule_autohide();
                }
            } else if !self.dock.is_autohide_enabled() {
                let mut start_position = 0.0;
                let mut end_position = 250.0;
                // Only keep dock hidden in fullscreen mode when NOT in expose mode
                // During expose mode, we want the dock to animate normally
                if current_workspace.get_fullscreen_mode() {
                    start_position = 250.0;
                    end_position = 250.0;
                }
                let dock_slide = start_position.interpolate(&end_position, delta);
                let dock_slide = dock_slide.clamp(0.0, 250.0);
                self.dock
                    .view_layer
                    .set_position(self.dock.slide_position(dock_slide), transition);
            }

            if let Some(anim_ref) = animation {
                self.layers_engine.start_animation(anim_ref, 0.0);
            }
        }
    }

    /// Set layer_shell_overlay and layer_shell_top visibility when entering/exiting fullscreen
    /// When entering fullscreen (is_fullscreen=true), fades out both layers and hides them
    /// When exiting fullscreen (is_fullscreen=false), shows and fades in both layers
    pub fn set_fullscreen_overlay_visibility(&self, is_fullscreen: bool) {
        let target_opacity = if is_fullscreen { 0.0_f32 } else { 1.0_f32 };
        let transition = Some(Transition::ease_in_out_quad(1.4));

        if !is_fullscreen {
            // Unhide before fading in so the animation is visible
            self.layer_shell_overlay.set_hidden(false);
            self.layer_shell_top.set_hidden(false);
        }
        self.layer_shell_overlay
            .set_opacity(target_opacity, transition.clone());
        let layer_shell_top_ref = self.layer_shell_top.clone();
        let layer_shell_overlay_ref = self.layer_shell_overlay.clone();
        self.layer_shell_top
            .set_opacity(target_opacity, transition)
            .on_finish(
                move |_: &Layer, _| {
                    if is_fullscreen {
                        layer_shell_top_ref.set_hidden(true);
                        layer_shell_overlay_ref.set_hidden(true);
                    }
                },
                true,
            );
    }

    /// Set the mode to show desktop mode using a delta for gestures
    pub fn expose_show_desktop(&self, delta: f32, end_gesture: bool) {
        // Don't allow mode switches while expose is animating
        if self.is_expose_transitioning() {
            return;
        }

        // If we're in expose mode, exit it first instead of transitioning directly
        if self.get_show_all() && end_gesture && delta > 0.0 {
            self.expose_set_visible(false);
            return;
        }

        const MULTIPLIER: f32 = 1000.0;
        let gesture = self
            .show_desktop_gesture
            .load(std::sync::atomic::Ordering::Relaxed);

        let mut new_gesture = gesture + (delta * MULTIPLIER) as i32;
        let show_desktop = self.get_show_desktop();

        let _model = self.model.read().unwrap();

        if end_gesture {
            if show_desktop {
                if new_gesture <= (9.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = 0;
                    self.set_show_desktop(false);
                } else {
                    new_gesture = MULTIPLIER as i32;
                    self.set_show_desktop(true);
                }
            } else {
                #[allow(clippy::collapsible_else_if)]
                if new_gesture >= (1.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = MULTIPLIER as i32;
                    self.set_show_desktop(true);
                } else {
                    new_gesture = 0;
                    self.set_show_desktop(false);
                }
            }
        }

        let delta = new_gesture as f32 / 1000.0;
        let delta = delta.clamp(0.0, 1.0);

        // Store the accumulated gesture value back for next update
        self.show_desktop_gesture
            .store(new_gesture, std::sync::atomic::Ordering::Relaxed);

        // Use same spring transition as expose_show_all for consistency
        let mut transition = Some(Transition::spring(0.5, 0.1));
        if !end_gesture {
            transition = None;
        }

        // Get screen dimensions for calculating center
        let size = self
            .focused_output_workspaces()
            .map(|ows| ows.workspaces_layer.render_size_transformed())
            .unwrap_or_default();
        let scale = Config::with(|c| c.screen_scale);
        let screen_center_x = size.x / 2.0;
        let screen_center_y = size.y / 2.0;

        let current_workspace_index = self.get_current_workspace_index();
        let Some(workspace) = self.get_current_workspace() else {
            return;
        };
        let windows_list = workspace.windows_list.read().unwrap();
        let Some(space) = self
            .primary_output_workspaces()
            .and_then(|ows| ows.spaces.get(current_workspace_index))
        else {
            return;
        };

        // Similar to expose_show_all: show_desktop_active when delta > 0
        let show_desktop_active = delta > 0.0 || transition.is_some();

        // The transaction the completion hook rides on: the FIRST mirror that
        // actually animates. Hanging it off a no-op change (setting the
        // workspaces layer to the opacity it already has) finished on the spot,
        // so closing show desktop restored the real windows in a single frame
        // while the mirrors were still flying back — the exit looked instant.
        let mut mirror_transaction: Option<layers::engine::TransactionRef> = None;

        // Show/hide expose_layer (master container) when showing desktop
        // This matches the pattern in expose_show_all_apply
        let expose_layer = self.expose_layer.clone();
        let show_desktop_ref = self.show_desktop.clone();

        expose_layer.set_hidden(!show_desktop_active);
        // Sync secondary output expose layers
        for ows in self.output_workspaces.values() {
            if ows.expose_layer != expose_layer {
                ows.expose_layer.set_hidden(!show_desktop_active);
            }
        }

        // Hide the real workspace content while the desktop is being revealed:
        // the windows are shown through their mirror layers inside the expose
        // layer, exactly as in expose. Without this the untouched windows keep
        // rendering on top and the gesture looks like it does nothing.
        let workspaces_layers: Vec<Layer> = self
            .output_workspaces
            .values()
            .map(|ows| ows.workspaces_layer.clone())
            .collect();
        for layer in workspaces_layers.iter() {
            layer.set_hidden(show_desktop_active);
        }

        // Show mirror windows layer when showing desktop, hide when not
        workspace
            .window_selector_view
            .window_selector_windows_container
            .set_hidden(!show_desktop_active);

        for window_id in windows_list.iter() {
            let window = self.get_window_for_surface(window_id).unwrap();
            if window.is_minimised() {
                continue;
            }

            // Get the original position from the workspace Space
            let Some(geometry) = space.element_geometry(window) else {
                continue;
            };
            let geometry = geometry.to_f64().to_physical(scale);

            let window_width = geometry.size.w as f32;
            let window_height = geometry.size.h as f32;
            let window_x = geometry.loc.x as f32;
            let window_y = geometry.loc.y as f32;

            // Set mirror layer size to match window
            window
                .mirror_layer()
                .set_size(Size::points(window_width, window_height), None);

            // Calculate window center
            let window_center_x = window_x + window_width / 2.0;
            let window_center_y = window_y + window_height / 2.0;

            // Calculate direction from screen center to window center
            let mut direction_x = window_center_x - screen_center_x;
            let mut direction_y = window_center_y - screen_center_y;

            // Normalize direction vector (with fallback for windows at exact center)
            let length = (direction_x * direction_x + direction_y * direction_y).sqrt();
            if length > 0.0 {
                direction_x /= length;
                direction_y /= length;
            } else {
                // If window is at exact center, push it to the right
                direction_x = 1.0;
                direction_y = 0.0;
            }

            // Calculate how far to push: just beyond the screen edge
            // Use screen size to push windows offscreen without going too far
            let push_distance = size.x.max(size.y);

            // Calculate target position offscreen in the direction
            let to_x = window_x + direction_x * push_distance;
            let to_y = window_y + direction_y * push_distance;

            // Interpolate between original workspace position and offscreen target
            let x = window_x.interpolate(&to_x, delta);
            let y = window_y.interpolate(&to_y, delta);

            // Animate the mirror layer, not the actual window
            let tr = window
                .mirror_layer()
                .set_position(layers::types::Point { x, y }, transition.clone());
            if mirror_transaction.is_none() {
                mirror_transaction = Some(tr);
            }
        }

        // If there's a transition, set up a callback to finalize visibility after animation
        if transition.is_some() {
            // Mark as animating when we have a transition
            tracing::debug!(target: "otto::popups", "is_animating(true) site=show-desktop");
            self.is_animating
                .store(true, std::sync::atomic::Ordering::Relaxed);
            self.show_desktop_animating
                .store(true, std::sync::atomic::Ordering::Relaxed);

            let expose_layer_ref = expose_layer.clone();
            let window_selector_layer = workspace
                .window_selector_view
                .window_selector_windows_container
                .clone();
            let is_animating_ref = self.is_animating.clone();
            let show_desktop_animating_ref = self.show_desktop_animating.clone();

            // Ride the mirrors' own animation. With no window to animate there
            // is nothing to wait for, so apply the end state right away.
            let Some(transaction) = mirror_transaction else {
                let is_active = show_desktop_ref.load(std::sync::atomic::Ordering::Relaxed);
                expose_layer.set_hidden(!is_active);
                workspace
                    .window_selector_view
                    .window_selector_windows_container
                    .set_hidden(!is_active);
                for layer in workspaces_layers.iter() {
                    layer.set_hidden(is_active);
                }
                self.is_animating
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                self.show_desktop_animating
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                return;
            };
            {
                transaction.on_finish(
                    move |_: &Layer, _: f32| {
                        let is_active = show_desktop_ref.load(std::sync::atomic::Ordering::Relaxed);
                        expose_layer_ref.set_hidden(!is_active);
                        window_selector_layer.set_hidden(!is_active);
                        for layer in workspaces_layers.iter() {
                            layer.set_hidden(is_active);
                        }
                        // Clear animating flags when animation completes
                        is_animating_ref.store(false, std::sync::atomic::Ordering::Relaxed);
                        show_desktop_animating_ref
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                    },
                    true,
                );
            }
        }
    }

    /// Repaint the workspace-selector thumbnail of the workspace containing
    /// `wid`, after one of its windows committed new content.
    ///
    /// The thumbnail is an image-cached `replicate_node` mirror of the
    /// workspace's `windows_layer` container, and its cached surface only
    /// re-renders when the FOLLOWED node's frame advances — a commit repaints
    /// a surface layer deep inside the container, which never bumps the
    /// container itself. Reporting the damage on the container is what lets
    /// the thumb track live content; without it, the preview of every
    /// non-current workspace freezes on whatever it showed when exposé opened.
    pub fn damage_workspace_thumbnail(&self, wid: &ObjectId) {
        let ws = self.with_model(|model| {
            model
                .workspaces
                .iter()
                .find(|ws| ws.windows_list.read().unwrap().contains(wid))
                .map(|ws| ws.windows_layer.clone())
        });
        if let Some(windows_layer) = ws {
            let size = windows_layer.render_size();
            windows_layer.add_damage(layers::skia::Rect::from_wh(size.x, size.y));
        }
    }

    pub fn expose_update_if_needed(&self) {
        let current_workspace_index = self.get_current_workspace_index();
        self.expose_update_if_needed_workspace(current_workspace_index);
    }
    pub fn expose_update_if_needed_workspace(&self, workspace_index: usize) {
        let relayout = self.expose_show_all_layout(workspace_index);
        // Keep secondary outputs' grids fresh too (window mapped/closed on
        // another screen while expose is open).
        {
            let focused = self.focused_output().map(|o| o.name());
            let others: Vec<(String, usize)> = self
                .output_workspaces
                .iter()
                .filter(|(n, _)| focused.as_deref() != Some(n.as_str()))
                .map(|(n, ows)| (n.clone(), ows.current_workspace))
                .collect();
            for (name, ws) in others {
                self.expose_show_all_layout_for(&name, ws);
            }
        }
        let gesture_active = self
            .expose_gesture_active
            .load(std::sync::atomic::Ordering::Relaxed);
        if self.get_show_all() && relayout && !gesture_active {
            let transition = Transition {
                delay: 0.0,
                timing: TimingFunction::Spring(Spring::with_duration_and_bounce(0.3, 0.1)),
            };
            self.expose_show_all_end(workspace_index, 1.0, true, Some(transition));
            // `expose_show_all_apply` blanks the selection overlay for the
            // duration of the open animation. Expose is already open here —
            // this is only a grid re-layout — so put it straight back, or the
            // highlight and label blink out on every re-layout (any client
            // commit that changes a window's geometry triggers one).
            self.show_selection_overlays();
        }
    }

    /// Make the per-workspace selection overlay (hover highlight + label)
    /// visible again after a re-layout that ran while expose was already open.
    pub fn show_selection_overlays(&self) {
        if self.expose_dragged_window.lock().unwrap().is_some() {
            return;
        }
        for workspace_view in self.with_model(|m| m.workspaces.clone()).iter() {
            let selector = &workspace_view.window_selector_view;
            // Re-record the overlay's content before revealing it. Its layer
            // keeps the last picture it was *rasterized* with, and it was
            // hidden for the whole drag: the highlight the pointer left on the
            // preview that has since been dragged away is still in that
            // picture, and un-hiding paints it back even though the state has
            // long since dropped the selection.
            selector.view.render(&selector.window_selector_view);
            selector.window_selector_view.set_opacity(1.0_f32, None);
        }
    }

    /// Close all the windows of an app by its id
    pub fn quit_app(&self, app_id: &str) {
        for window_id in self.get_app_windows(app_id) {
            let window = self.get_window_for_surface(&window_id);
            if let Some(we) = window {
                match we.underlying_surface() {
                    WindowSurface::Wayland(t) => t.send_close(),
                    #[cfg(feature = "xwayland")]
                    WindowSurface::X11(w) => {
                        let _ = w.close();
                    }
                }
            }
        }
    }

    /// Close all the windows of the current focused App
    pub fn quit_current_app(&self) {
        if let Some(app_id) = self.get_current_app_id() {
            self.quit_app(&app_id);
        }
    }

    /// Close all the windows of the current focused App in th app switcher
    pub fn quit_appswitcher_app(&self) {
        if let Some(app_id) = self.app_switcher.get_current_app_id() {
            self.quit_app(&app_id);
        }
    }

    /// Minimise a WindowElement.
    ///
    /// The animation runs in three phases:
    ///   1. Dock slide-in (if auto-hidden) + drawer expand — in parallel.
    ///   2. Move window layer into the drawer, run genie effect.
    ///   3. Schedule dock auto-hide (if it was revealed for this minimize).
    pub fn minimize_window(&mut self, we: &WindowElement) -> Option<ObjectId> {
        let id = we.id();

        // Already minimised — nothing to do (guards against rapid double-clicks).
        if we.is_minimised() {
            return None;
        }

        if let Some(window) = self.windows_map.get_mut(&id) {
            window.set_is_minimised(true);
        }

        // Unmap from all spaces so hit-testing ignores the minimised window.
        for output in &self.outputs {
            if let Some(ows) = self.output_workspaces.get_mut(&output.name()) {
                for space in &mut ows.spaces {
                    space.unmap_elem(we);
                }
            }
        }

        let dock_was_hidden = self.dock.is_autohide_enabled() && self.dock.is_hidden();
        let show_tr = if dock_was_hidden {
            self.dock.show_autohide()
        } else {
            None
        };

        self.with_model_mut(|model| {
            model
                .minimized_windows
                .push((id.clone(), we.xdg_title().to_string()));

            if let Some(view) = self.get_window_view(&id) {
                // add_window_element triggers render_dock → magnify_elements_with_scale
                // which animates the drawer from width 0 → icon_size and stores the
                // AnimationRef in last_layout_animation.
                let (drawer, inner) = self.dock.add_window_element(we);
                let layout_anim = self.dock.last_layout_animation();

                view.mirror_layer.set_hidden(true);

                let layers_engine = self.layers_engine.clone();
                let dock_ref = self.dock.clone();
                let view = view.clone();
                let drawer = drawer.clone();
                let inner = inner.clone();

                tokio::spawn(async move {
                    // Run the genie while the window is STILL in the windows
                    // plane subtree: the destination rect is global scene
                    // coordinates, so the animation squeezes toward the dock
                    // without moving the layer. Reparenting into the drawer
                    // first put the whole animation under the strip-sized
                    // dock plane (≤480px tall buffer) — the genie rendered
                    // clipped into that band (i.e. invisible) while the
                    // windows plane kept the stale window pixels.
                    //
                    // Capture the unscaled window size NOW — the layer is
                    // hidden during the post-genie settle (display: none →
                    // zero layout bounds), so it can't be read later.
                    let base_bounds = view.window_layer.render_bounds_with_children();
                    let base_size = (base_bounds.width(), base_bounds.height());
                    let inner_bounds = inner.render_bounds_transformed();
                    // The window is sucked towards the dock, so the genie runs
                    // sideways when the dock is on a screen side.
                    let dock_position = dock_ref.position();
                    view.genie_effect.set_direction(
                        dock_position.is_vertical(),
                        dock_position == crate::config::DockPosition::Left,
                    );
                    let minimize_tr = view.minimize(skia::Rect::from_xywh(
                        inner_bounds.x(),
                        inner_bounds.y(),
                        inner_bounds.width(),
                        inner_bounds.height(),
                    ));

                    // Track the expanding drawer: update the genie destination
                    // while the animation runs, keep the miniwindow fitted
                    // after the genie finishes.
                    let genie_ref = view.genie_effect.clone();
                    let view_ref = view.clone();
                    let inner_ref = inner.clone();
                    drawer.clear_on_change_size_handlers();
                    drawer.on_change_size(
                        move |_layer: &Layer, _| {
                            let bounds = inner_ref.render_bounds_transformed();
                            if view_ref.is_minimizing() {
                                genie_ref.set_destination(
                                    skia::Rect::from_xywh(
                                        bounds.x(),
                                        bounds.y(),
                                        bounds.width(),
                                        bounds.height(),
                                    ),
                                    true,
                                );
                            } else {
                                view_ref.apply_minimized_scale(bounds);
                            }
                        },
                        false,
                    );

                    // Wait for all animations to complete.
                    let show_fut = async {
                        if let Some(tr) = show_tr {
                            tr.await;
                        }
                    };
                    let layout_fut = async {
                        if let Some(anim) = layout_anim {
                            anim.await;
                        }
                    };
                    tokio::join!(minimize_tr, show_fut, layout_fut);

                    // Genie done — NOW move the (effect-free, hidden) window
                    // layer into the drawer and fit it. From here on it
                    // renders in the dock plane as the mini-window.
                    // is_minimizing is still TRUE (holding the backend in
                    // forced-composite mode); it clears below once the layer
                    // is settled so the planes return with one clean full
                    // redraw.
                    if !view.is_unmapped() {
                        view.window_layer.set_layout_style(taffy::Style {
                            position: taffy::Position::Absolute,
                            ..Default::default()
                        });
                        if let Err(e) = layers_engine
                            .add_layer_to_positioned(view.window_layer.clone(), Some(inner.id))
                        {
                            tracing::warn!("minimize: failed to reparent window into drawer: {e}");
                        }
                        view.set_is_minimizing(false);
                        let bounds = inner.render_bounds_transformed();
                        // apply_minimized_scale_with_base also positions the
                        // window centred in the drawer — don't reset it to the
                        // drawer's top-left corner here.
                        let scale_tr = view.apply_minimized_scale_with_base(base_size, bounds);
                        // scale/position are SCHEDULED engine changes while
                        // set_hidden is IMMEDIATE: unhiding before they are
                        // applied lets a render draw the window in the
                        // drawer at FULL scale for one frame. Await the
                        // scale transaction (applied by the engine on its
                        // own thread — calling engine.update() from this
                        // task instead panics the scene arena), THEN show.
                        if let Some(tr) = scale_tr {
                            tr.await;
                        }
                        // Hidden since the genie's on_finish (ghost-frame
                        // guard) — safe to show now that it lives in the
                        // drawer at mini scale.
                        view.window_layer.set_hidden(false);
                    } else {
                        view.set_is_minimizing(false);
                    }

                    // Re-hide the dock if we revealed it for this minimize.
                    if dock_was_hidden {
                        dock_ref.schedule_autohide();
                    }
                });
            }

            self.notify_observers(model);
        });
        self.refresh_output_selectors();

        // Find the next topmost non-minimized window for focus.
        let index = self.with_model(|m| m.current_workspace);
        self.primary_output_workspaces()
            .and_then(|ows| ows.spaces.get(index))
            .and_then(|s| {
                s.elements().rev().find_map(|e| {
                    let eid = e.id();
                    let dominated = self
                        .windows_map
                        .get(&eid)
                        .map(|w| w.is_minimised() || w.id() == we.id())
                        .unwrap_or(false);
                    if dominated {
                        None
                    } else {
                        Some(eid)
                    }
                })
            })
    }

    /// Unminimise a WindowElement
    pub fn unminimize_window(&mut self, wid: &ObjectId) -> Option<ObjectId> {
        // Already not minimised — nothing to do (guards against rapid double-clicks).
        if let Some(window) = self.windows_map.get(wid) {
            if !window.is_minimised() {
                return None;
            }
        }

        let workspace_for_window = self.with_model(|model| {
            model
                .workspaces
                .iter()
                .position(|ws| ws.windows_list.read().unwrap().contains(wid))
        });
        if workspace_for_window.is_none() {
            tracing::warn!(
                "Trying to unminimize a window that is not in any workspace: {}",
                wid
            );
            return None;
        }
        let workspace_for_window = workspace_for_window.unwrap();
        let current_workspace_index = self.get_current_workspace_index();

        // The window was unmapped from the space during minimize (for hit-test
        // exclusion).  Re-map it now so build_unminimize_context can find it.
        let window = self.get_window_for_surface(wid)?.clone();
        let view = self.get_window_view(wid)?;
        let loc = view.unmaximised_rect.loc;
        if let Some(ows) = self.primary_output_workspaces_mut() {
            if let Some(space) = ows.spaces.get_mut(workspace_for_window) {
                space.map_element(window, loc, false);
            }
        }

        let ctx = self.build_unminimize_context(wid)?;

        if workspace_for_window != current_workspace_index {
            if let Some(tr) = self.set_current_workspace_index(
                workspace_for_window,
                Some(Transition::ease_out_quad(0.2)),
            ) {
                let ctx_clone = ctx.clone();
                tr.on_finish(
                    move |_: &Layer, _: f32| {
                        ctx_clone.run();
                    },
                    true,
                );
                return Some(ctx.wid.clone());
            }
        }

        self.unminimize_window_in_workspace(ctx);
        Some(wid.clone())
    }

    fn unminimize_window_in_workspace(&self, ctx: UnminimizeContext) {
        ctx.run();
    }

    fn build_unminimize_context(&self, wid: &ObjectId) -> Option<UnminimizeContext> {
        let scale = Config::with(|c| c.screen_scale) as f32;
        let primary_ows = self.primary_output_workspaces()?;
        let (index, space) = primary_ows
            .spaces
            .iter()
            .enumerate()
            .find(|(_, space)| space.elements().any(|e| e.id() == *wid))?;

        let workspace = self.with_model(|m| m.workspaces[index].clone());
        let window = self.get_window_for_surface(wid)?.clone();
        let view = self.get_window_view(wid)?;
        let window_geometry = space.element_geometry(&window)?;
        let pos_x = window_geometry.loc.x;
        let pos_y = window_geometry.loc.y;
        let layer_pos = crate::workspaces::utils::snap_position_px(
            pos_x as f64 * scale as f64,
            pos_y as f64 * scale as f64,
        );
        let (layer_pos_x, layer_pos_y) = (layer_pos.x, layer_pos.y);

        Some(UnminimizeContext {
            wid: wid.clone(),
            workspace,
            window,
            view,
            dock: self.dock.clone(),
            layers_engine: self.layers_engine.clone(),
            model: self.model.clone(),
            observers: self.observers.clone(),
            layer_pos: (layer_pos_x, layer_pos_y),
            pos_logical: (pos_x, pos_y),
        })
    }

    // Helpers / Windows Management

    /// The area of `output` a normal window may occupy: the output geometry
    /// minus layer-shell exclusive zones and the dock.
    pub fn usable_geometry(
        &self,
        output: &Output,
    ) -> Option<Rectangle<i32, smithay::utils::Logical>> {
        let geo = self.output_geometry(output)?;
        let map = layer_map_for_output(output);
        let zone = map.non_exclusive_zone();
        let mut adjusted = Rectangle::new(geo.loc + zone.loc, zone.size);

        // Account for the dock geometry (internal compositor UI, not layer-shell)
        self.subtract_dock(&mut adjusted);

        Some(adjusted)
    }

    /// Shrink `zone` so it stops at the dock, whichever screen edge the dock is
    /// docked to. A no-op when the dock has no geometry yet.
    pub fn subtract_dock(&self, zone: &mut Rectangle<i32, smithay::utils::Logical>) {
        subtract_dock_rect(
            self.dock.position(),
            self.get_dock_geometry(),
            self.dock.is_autohide_enabled(),
            zone,
        );
    }

    /// Determine the initial placement of a new window within the workspace.
    /// It calculates the appropriate position and bounds for the window based
    /// on the current pointer location and the output geometry under the pointer.
    pub fn new_window_placement_at(
        &self,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> (
        smithay::utils::Rectangle<i32, smithay::utils::Logical>,
        smithay::utils::Point<i32, smithay::utils::Logical>,
    ) {
        let output = self
            .output_under(pointer_location)
            .next()
            .or_else(|| self.default_client_output())
            .cloned();
        let output_geometry = output
            .and_then(|o| self.usable_geometry(&o))
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (800, 800).into()));

        // The client's real size is unknown until it configures; assume a typical
        // size for placement and clamp it to the usable area.
        const DEFAULT_WINDOW_WIDTH: i32 = 800;
        const DEFAULT_WINDOW_HEIGHT: i32 = 600;
        let usable = output_geometry;
        let win_w = DEFAULT_WINDOW_WIDTH.min(usable.size.w);
        let win_h = DEFAULT_WINDOW_HEIGHT.min(usable.size.h);

        // Existing window rectangles (current workspace) to avoid overlapping.
        let existing: Vec<Rectangle<i32, smithay::utils::Logical>> = self
            .spaces_elements()
            .filter_map(|we| self.element_geometry(we))
            .collect();

        // Clamp a top-left position so the assumed window stays inside `usable`.
        let clamp = |p: smithay::utils::Point<i32, smithay::utils::Logical>| {
            smithay::utils::Point::<i32, smithay::utils::Logical>::from((
                p.x.clamp(usable.loc.x, usable.loc.x + (usable.size.w - win_w).max(0)),
                p.y.clamp(usable.loc.y, usable.loc.y + (usable.size.h - win_h).max(0)),
            ))
        };

        // Candidate top-left positions in priority order: clockwise corners from
        // top-left, then snapped to the right/bottom edges of existing windows.
        // The least-overlap pick favours disjoint placement so multiple windows
        // stay eligible for direct scanout.
        let right = usable.loc.x + usable.size.w - win_w;
        let bottom = usable.loc.y + usable.size.h - win_h;
        let mut candidates: Vec<smithay::utils::Point<i32, smithay::utils::Logical>> = vec![
            (usable.loc.x, usable.loc.y).into(), // top-left
            (right, usable.loc.y).into(),        // top-right
            (right, bottom).into(),              // bottom-right
            (usable.loc.x, bottom).into(),       // bottom-left
        ];
        for r in &existing {
            candidates.push((r.loc.x + r.size.w, r.loc.y).into()); // to the right
            candidates.push((r.loc.x, r.loc.y + r.size.h).into()); // below
        }

        // Total overlap area of the assumed window placed at `p` against existing
        // windows. `min_by_key` keeps the first candidate on ties, so the
        // clockwise corners win and a disjoint top-left placement is preferred.
        let overlap_area = |p: smithay::utils::Point<i32, smithay::utils::Logical>| -> i64 {
            let rect = Rectangle::new(p, (win_w, win_h).into());
            existing
                .iter()
                .filter_map(|r| r.intersection(rect))
                .map(|i| i.size.w as i64 * i.size.h as i64)
                .sum()
        };

        let location = candidates
            .into_iter()
            .map(clamp)
            .min_by_key(|&p| overlap_area(p))
            .unwrap_or_else(|| clamp((usable.loc.x, usable.loc.y).into()));

        tracing::debug!(
            "new_window_placement: {} existing windows, placed at {:?}",
            existing.len(),
            location
        );

        (output_geometry, location)
    }

    /// map the window element, in the position on the current space,
    /// should be called on every window move / drag
    /// sets the position of the window layer in the scene
    /// The output whose workspaces currently contain this window, if any.
    pub fn output_for_window(&self, window_element: &WindowElement) -> Option<Output> {
        self.output_workspaces.iter().find_map(|(name, ows)| {
            ows.spaces
                .iter()
                .any(|s| s.elements().any(|e| e.id() == window_element.id()))
                .then(|| self.outputs.iter().find(|o| o.name() == *name).cloned())
                .flatten()
        })
    }

    pub fn map_window(
        &mut self,
        window_element: &WindowElement,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
        activate: bool,
        transition: Option<Transition>,
    ) {
        let location: smithay::utils::Point<i32, smithay::utils::Logical> = location.into();

        // Route by the window's center: a drag that crosses into another
        // output's region MIGRATES the window there (space + scene subtree).
        // Fallbacks: current owner, focused output, primary.
        let size = window_element.geometry().size;
        let center = smithay::utils::Point::<f64, smithay::utils::Logical>::from((
            location.x as f64 + size.w as f64 / 2.0,
            location.y as f64 + size.h as f64 / 2.0,
        ));
        let target = self
            .output_under(center)
            .next()
            .cloned()
            .or_else(|| self.output_for_window(window_element))
            .or_else(|| {
                self.with_model(|m| m.focused_output_name.clone())
                    .and_then(|n| self.outputs.iter().find(|o| o.name() == n).cloned())
            })
            .or_else(|| self.primary_output.clone());
        let Some(target) = target else {
            return;
        };

        self.map_window_on_output(&target, window_element, location, activate, transition);
    }

    /// Map a window at `location` onto an explicit `output`, migrating it off
    /// its previous owner when that changes.
    ///
    /// Use this instead of [`Self::map_window`] whenever `location` was derived
    /// from a known output's geometry (tile, maximize). The center-based routing
    /// in `map_window` measures the window's CURRENT size, which is still the
    /// pre-resize one while the resize animates: a maximized window snapped to
    /// the right half puts that stale center on the output's right edge, so it
    /// would migrate onto the output next to it and land at a negative
    /// output-local x — flying off-screen to the left.
    pub fn map_window_on_output(
        &mut self,
        output: &Output,
        window_element: &WindowElement,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
        activate: bool,
        transition: Option<Transition>,
    ) {
        let location: smithay::utils::Point<i32, smithay::utils::Logical> = location.into();

        if let Some(owning) = self.output_for_window(window_element) {
            if owning.name() != output.name() {
                // Migrate: drop the window from the old output's spaces and
                // views. The base layer is re-parented into the target
                // output's workspace view by `map_window_for_output` below.
                // Use `unmap_window_keep_mirror` (not `unmap_window`) so the
                // window's expose-mirror scene node survives the migration —
                // see the doc comment on `unmap_window_keep_mirror`.
                if let Some(old) = self.output_workspaces.get_mut(&owning.name()) {
                    for space in old.spaces.iter_mut() {
                        space.unmap_elem(window_element);
                    }
                    for view in old.workspace_views.iter() {
                        view.unmap_window_keep_mirror(&window_element.id());
                    }
                }
            }
        }

        self.map_window_for_output(output, window_element, location, activate, transition);
    }

    /// Map a window onto a specific output's current workspace.
    /// Falls back to primary output if the output has no workspace set yet.
    pub fn map_window_for_output(
        &mut self,
        output: &Output,
        window_element: &WindowElement,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
        activate: bool,
        transition: Option<Transition>,
    ) {
        let name = output.name();
        let location = location.into();

        // Map into the correct output's current space
        let mapped = if let Some(ows) = self.output_workspaces.get_mut(&name) {
            let idx = ows.current_workspace;
            ows.spaces[idx].map_element(window_element.clone(), location, activate);
            true
        } else {
            false
        };

        if !mapped {
            // Fallback to primary
            if let Some(space) = self.space_mut() {
                space.map_element(window_element.clone(), location, activate);
            }
        }

        if let std::collections::hash_map::Entry::Vacant(e) =
            self.windows_map.entry(window_element.id())
        {
            e.insert(window_element.clone());
            self.update_workspace_model();
        }

        {
            let loc = self
                .output_workspaces
                .get(&name)
                .and_then(|ows| ows.spaces[ows.current_workspace].element_location(window_element))
                .unwrap_or_else(|| self.element_location(window_element).unwrap_or_default());

            // Use the correct output's workspace view
            let workspace_view = self
                .output_workspaces
                .get(&name)
                .map(|ows| ows.workspace_views[ows.current_workspace].clone())
                .or_else(|| self.get_current_workspace());
            let Some(workspace_view) = workspace_view else {
                return;
            };

            // Space locations are global; the workspace view's layers live
            // under the output's scene container — convert to output-local.
            let local_loc = loc - output.current_location();
            workspace_view.map_window(window_element, local_loc, transition);
            let _view = self.get_or_add_window_view(window_element);
        }
        self.refresh_space();
        self.expose_update_if_needed();
    }

    /// remove a WindowElement from the workspace model,
    /// remove the window layer from the scene,
    /// Returns the surface IDs from removed popups that need cleanup
    pub fn unmap_window(&mut self, window_id: &ObjectId) -> Vec<ObjectId> {
        tracing::info!("workspaces::unmap_window: {:?}", window_id);

        let mut workspace_index = None;

        if let Some(element) = self.get_window_for_surface(window_id).cloned() {
            // Find workspace index from primary output spaces
            if let Some(pows) = self.primary_output_workspaces() {
                for (i, space) in pows.spaces.iter().enumerate() {
                    if space.elements().any(|e| e.id() == element.id()) {
                        workspace_index = Some(i);
                        break;
                    }
                }
            }
            // Unmap from all outputs' spaces
            for ows in self.output_workspaces.values_mut() {
                for space in ows.spaces.iter_mut() {
                    space.unmap_elem(&element);
                }
            }
        }

        self.with_model(|m| {
            for workspace_view in m.workspaces.iter() {
                workspace_view.unmap_window(window_id);
            }
        });
        self.windows_map.remove(window_id);
        self.forget_window_focus(window_id);
        // Remove debug texture snapshot for this surface
        crate::textures_storage::remove(window_id);
        let removed_surface_ids = self.remove_window_view(window_id);

        self.refresh_space();
        self.update_workspace_model();

        // Recalculate expose layout if in expose mode
        if let Some(index) = workspace_index {
            self.expose_update_if_needed_workspace(index);
        }

        // Return the surface IDs so the compositor can clean up surface_layers and sc_layers
        removed_surface_ids
    }
    /// Return if the current coordinates are over the dock
    pub fn is_cursor_over_dock(&self, x: f32, y: f32) -> bool {
        self.dock.alive()
            && self
                .dock
                .view_layer
                .render_bounds_transformed()
                .contains(skia::Point::new(x, y))
            || self.dock.has_menu_open()
    }

    /// Return the actual rendered height of the dock in logical pixels
    pub fn get_dock_height(&self) -> i32 {
        if self.dock.alive() {
            let bounds = self.dock.bar_layer.render_bounds_transformed();
            let scale = Config::with(|c| c.screen_scale);
            (bounds.height() / scale as f32).ceil() as i32
        } else {
            0
        }
    }

    /// Return the actual rendered geometry of the dock in logical coordinates
    pub fn get_dock_geometry(&self) -> Rectangle<i32, smithay::utils::Logical> {
        if self.dock.alive() {
            let bounds = self.dock.bar_layer.render_bounds_transformed();
            let scale = Config::with(|c| c.screen_scale) as f32;
            let x = (bounds.x() / scale) as i32;
            let y = (bounds.y() / scale) as i32;
            let w = (bounds.width() / scale).ceil() as i32;
            let h = (bounds.height() / scale).ceil() as i32;
            // Grow the rect 2pt towards the screen interior so windows keep a
            // sliver of clearance from the dock's inner edge.
            match self.dock.position() {
                crate::config::DockPosition::Bottom => {
                    Rectangle::new((x, y - 2).into(), (w, h).into())
                }
                crate::config::DockPosition::Left => {
                    Rectangle::new((x, y).into(), (w + 2, h).into())
                }
                crate::config::DockPosition::Right => {
                    Rectangle::new((x - 2, y).into(), (w + 2, h).into())
                }
            }
        } else {
            Rectangle::new((0, 0).into(), (0, 0).into())
        }
    }

    /// Return the list of WlSurface ids of an app by its id
    pub fn get_app_windows(&self, app_id: &str) -> Vec<ObjectId> {
        let model = self.model.read().unwrap();
        model
            .app_windows_map
            .get(app_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Return the list of Spaces where an app has windows by its id
    pub fn get_app_spaces(&self, app_id: &str) -> Vec<&Space<WindowElement>> {
        let model = self.model.read().unwrap();
        let mut spaces = Vec::new();

        model
            .app_windows_map
            .get(app_id)
            .cloned()
            .unwrap_or_default()
            .iter()
            .for_each(|id| {
                let window = self.get_window_for_surface(id);
                if let Some(we) = window {
                    for space in self
                        .output_workspaces
                        .values()
                        .flat_map(|ows| ows.spaces.iter())
                    {
                        if space.elements().any(|e| e == we) {
                            spaces.push(space);
                            break;
                        }
                    }
                }
            });

        spaces
    }

    /// Return the current focused Application
    pub fn get_current_app_id(&self) -> Option<String> {
        let model = self.model.read().unwrap();
        model.zindex_application_list.last().cloned()
    }

    /// Return the list of WlSurface ids of the current focused Application
    pub fn get_current_app_windows(&self) -> Vec<ObjectId> {
        self.get_current_app_id()
            .map(|app_id| self.get_app_windows(&app_id))
            .unwrap_or_default()
    }

    /// Record `wid` as the most recently focused window (see `focus_history`).
    pub fn note_window_focused(&mut self, wid: &ObjectId) {
        if self.focus_history.last() == Some(wid) {
            return;
        }
        self.focus_history.retain(|id| id != wid);
        self.focus_history.push(wid.clone());
    }

    /// Drop a dead window from the focus history.
    pub fn forget_window_focus(&mut self, wid: &ObjectId) {
        self.focus_history.retain(|id| id != wid);
    }

    /// The most recently focused window that is still mapped.
    pub fn last_focused_window(&self) -> Option<ObjectId> {
        self.focus_history
            .iter()
            .rev()
            .find(|id| self.windows_map.contains_key(id))
            .cloned()
    }

    fn window_matches_app(&self, we: &WindowElement, app_id: &str) -> bool {
        we.xdg_app_id() == app_id || we.display_app_id(&self.display_handle) == app_id
    }

    /// Windows of `app_id` in a stable, workspace-aware order: by output, then
    /// workspace index, then stacking position within that workspace.
    ///
    /// Unlike `app_windows_map`, this order does not depend on which workspace
    /// happens to be current, so cycling through it visits every window once.
    /// Minimised windows are skipped — raising them is a different gesture.
    pub fn app_windows_in_stable_order(&self, app_id: &str) -> Vec<ObjectId> {
        let mut output_names: Vec<&String> = self.output_workspaces.keys().collect();
        output_names.sort();
        let mut windows = Vec::new();
        for name in output_names {
            let Some(ows) = self.output_workspaces.get(name) else {
                continue;
            };
            for space in ows.spaces.iter() {
                for we in space.elements() {
                    if we.is_minimised() || !self.window_matches_app(we, app_id) {
                        continue;
                    }
                    windows.push(we.id());
                }
            }
        }
        windows
    }

    /// Windows of `app_id` ordered most-recently-focused first; windows never
    /// focused come last, in stable order.
    pub fn app_windows_by_recency(&self, app_id: &str) -> Vec<ObjectId> {
        let mut windows = self.app_windows_in_stable_order(app_id);
        windows.sort_by_key(|id| {
            // Position from the end of the history: 0 = most recent. Never
            // focused sorts after everything, keeping the stable order.
            self.focus_history
                .iter()
                .rev()
                .position(|h| h == id)
                .unwrap_or(usize::MAX)
        });
        windows
    }

    /// The app to cycle within: the app owning the last focused window, falling
    /// back to the topmost app in the z-order.
    fn cycling_app_id(&self) -> Option<String> {
        self.last_focused_window()
            .and_then(|id| self.windows_map.get(&id).cloned())
            .map(|we| {
                let raw = we.xdg_app_id();
                if raw.is_empty() {
                    we.display_app_id(&self.display_handle)
                } else {
                    raw
                }
            })
            .filter(|app_id| !app_id.is_empty())
            .or_else(|| self.get_current_app_id())
    }

    /// Step `offset` windows through the current app's stable window order,
    /// starting from the focused one, and raise the window we land on.
    fn cycle_app_window(&mut self, offset: isize) -> Option<ObjectId> {
        let app_id = self.cycling_app_id()?;
        let windows = self.app_windows_in_stable_order(&app_id);
        if windows.is_empty() {
            return None;
        }
        let current = self
            .last_focused_window()
            .and_then(|id| windows.iter().position(|w| *w == id))
            // Nothing of this app is focused — start from the end so that
            // stepping forward lands on the first window.
            .unwrap_or(windows.len() - 1) as isize;
        let len = windows.len() as isize;
        let index = (current + offset).rem_euclid(len) as usize;
        let window_id = windows[index].clone();
        self.raise_element(&window_id, true, true);
        // The window may live on another workspace — follow it there.
        self.switch_to_workspace_of_window(&window_id);
        Some(window_id)
    }

    /// Return the Window object of WlSurface by its id
    pub fn get_window_for_surface(&self, id: &ObjectId) -> Option<&WindowElement> {
        self.windows_map.get(id)
    }

    pub fn get_or_add_window_view(
        &self,
        // object_id: &ObjectId,
        window: &WindowElement,
    ) -> WindowView {
        let mut window_views = self.window_views.write().unwrap();
        let wid = window.id();
        let entry = window_views
            .entry(wid.clone())
            .or_insert_with(|| WindowView::new(self.layers_engine.clone(), window));
        entry.clone()
    }

    /// Remove a WindowView from the scene and delete it from the window_views map
    /// Returns the surface IDs from removed popups that need cleanup
    pub fn remove_window_view(&mut self, object_id: &ObjectId) -> Vec<ObjectId> {
        // Remove any popups that belong to this window
        let removed_surface_ids = self.popup_overlay.remove_popups_for_window(object_id);

        let mut window_views = self.window_views.write().unwrap();
        if let Some(view) = window_views.remove(object_id) {
            view.set_is_unmapped(true);
            view.window_layer.remove();
        }

        removed_surface_ids
    }

    pub fn get_window_view(&self, id: &ObjectId) -> Option<WindowView> {
        let window_views = self.window_views.read().unwrap();

        window_views.get(id).cloned()
    }

    /// Whether any window is currently running its minimize/unminimize
    /// (genie) animation. The genie's image filter paints far outside the
    /// layer's damage rects, so the per-plane partial-redraw pipeline
    /// corrupts it — the backend switches to the full-GPU scene composite
    /// for the duration (see `render_output_frame`'s `force_composite`).
    pub fn has_minimizing_window(&self) -> bool {
        self.window_views
            .read()
            .unwrap()
            .values()
            .any(|v| v.is_minimizing())
    }

    pub fn set_window_view(&self, id: &ObjectId, window_view: WindowView) {
        let mut window_views = self.window_views.write().unwrap();

        window_views.insert(id.clone(), window_view);
    }

    /// unmap the window from the current space and workspaceview
    /// map the window to the new space and workspaceview
    pub fn move_window_to_workspace(
        &mut self,
        we: &WindowElement,
        workspace_index: usize,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
    ) {
        self.move_window_to_workspace_with_activate(we, workspace_index, location, false);
    }

    pub fn move_window_to_workspace_with_activate(
        &mut self,
        we: &WindowElement,
        workspace_index: usize,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
        activate: bool,
    ) {
        let location = location.into();
        // A workspace move stays on the window's own output — spaces and
        // scene views are per-output.
        let Some(output) = self
            .output_for_window(we)
            .or_else(|| self.primary_output.clone())
        else {
            return;
        };
        self.move_window_to_workspace_on_output_with_activate(
            &output,
            we,
            workspace_index,
            location,
            activate,
        );
    }

    pub fn raise_next_app_window(&mut self) -> Option<ObjectId> {
        self.cycle_app_window(1)
    }

    pub fn raise_prev_app_window(&mut self) -> Option<ObjectId> {
        self.cycle_app_window(-1)
    }

    /// Scroll the output owning `wid` to the workspace that holds it, so a
    /// window raised from another workspace actually becomes visible.
    fn switch_to_workspace_of_window(&mut self, wid: &ObjectId) {
        let owner = self.output_workspaces.iter().find_map(|(name, ows)| {
            ows.spaces
                .iter()
                .position(|s| s.elements().any(|e| e.id() == *wid))
                .map(|i| (name.clone(), i))
        });
        if let Some((name, index)) = owner {
            if let Some(output) = self.outputs.iter().find(|o| o.name() == name).cloned() {
                self.set_workspace_for_output(&output, index, None);
            }
        } else {
            let current_space_index = self.with_model(|m| m.current_workspace);
            self.set_current_workspace_index(current_space_index, None);
        }
    }

    /// Raise thw windowelement on top of all the windows in its space
    /// activate: will set the window as active
    /// update: will update the workspace model
    pub fn raise_element(&mut self, window_id: &ObjectId, activate: bool, update: bool) {
        // get the space with the window
        // tracing::info!("workspaces::raise_element: {:?}", window_id);
        // A window lives in exactly one output's space, so search every output
        // to find its owner instead of assuming the primary output.
        let owner = self.output_workspaces.iter().find_map(|(name, ows)| {
            ows.spaces
                .iter()
                .position(|s| s.elements().any(|e| e.id() == *window_id))
                .map(|i| (name.clone(), i))
        });

        if let Some((name, index)) = owner {
            let window = self.windows_map.get(window_id).cloned();
            if let Some(window) = window {
                // Check if already on top in the owning output's space
                let already_on_top = self
                    .output_workspaces
                    .get(&name)
                    .and_then(|ows| ows.spaces.get(index))
                    .and_then(|s| s.elements().last())
                    .map(|last| last.id() == *window_id)
                    .unwrap_or(false);
                if already_on_top {
                    return;
                }
                if window.is_minimised() && !activate {
                    return;
                }
                // FIXME: this is a hack to prevent raising a window that is already fullscreen
                // ideally we avoid resort a window already on top
                if window.is_fullscreen() {
                    return;
                }

                // Get the currently top window from the owning output's space before raising
                let previous_top = self
                    .output_workspaces
                    .get(&name)
                    .and_then(|ows| ows.spaces.get(index))
                    .and_then(|s| s.elements().last())
                    .map(|w| w.id());

                // Raise only in the owning output's space at that index
                if let Some(space) = self
                    .output_workspaces
                    .get_mut(&name)
                    .and_then(|ows| ows.spaces.get_mut(index))
                {
                    space.raise_element(&window, activate);
                }

                // Explicitly send configure to ensure activation state is communicated to client
                if activate {
                    if let Some(toplevel) = window.toplevel() {
                        toplevel.send_pending_configure();
                    }
                }

                // When activating a window, manage popup visibility
                if activate {
                    // Hide popups for the previous top window
                    if let Some(prev_id) = previous_top {
                        if prev_id != *window_id {
                            self.popup_overlay.hide_popups_for_window(&prev_id);
                        }
                    }
                    // Show popups for the newly activated window
                    self.popup_overlay.show_popups_for_window(window_id);
                }

                let workspace = self
                    .output_workspaces
                    .get(&name)
                    .and_then(|ows| ows.workspace_views.get(index).cloned());
                if let Some(workspace) = workspace {
                    workspace.raise_window_to_front(window_id);
                }
                if update {
                    self.update_workspace_model();
                }
            }
        } else {
            tracing::warn!("workspaces::raise_element: window not found");
        }
    }

    /// Raise all the windows of a given app
    /// returns the window id of the last window raised, if any
    fn raise_app_elements(
        &mut self,
        app_id: &str,
        focus_window: Option<&ObjectId>,
    ) -> Option<ObjectId> {
        // for every window in the app, raise it
        let windows = self.get_app_windows(app_id);
        let mut focus_wid = None;
        for (i, window_id) in windows.iter().enumerate() {
            if let Some(we) = self.get_window_for_surface(window_id) {
                if !we.is_minimised() {
                    if i == windows.len() - 1 {
                        self.raise_element(window_id, true, false);
                        focus_wid = Some(window_id.clone());
                    } else {
                        self.raise_element(window_id, false, false);
                    }
                } else {
                    // if minimised and there is only one window in the app, unminimize it
                    if windows.len() == 1 {
                        focus_wid = self.unminimize_window(window_id);
                    }
                }
            }
        }
        if let Some(wid) = focus_window {
            if let Some(we) = self.get_window_for_surface(wid) {
                if !we.is_minimised() {
                    self.raise_element(wid, true, false);
                    focus_wid = Some(wid.clone());
                }
            }
        }

        focus_wid
    }

    /// Raise all the windows of a given app
    /// returns the window id of the last window raised, and set to active, if any
    pub fn focus_app(&mut self, app_id: &str) -> Option<ObjectId> {
        tracing::trace!("workspaces::focus_app: {:?}", app_id);

        // Hide show desktop when focusing an app (use -2.0 to force dismiss)
        if self.get_show_desktop() {
            self.expose_show_desktop(-2.0, true);
        }

        // Target the app's most recently focused window, not whichever of its
        // windows happens to sit on the current workspace — otherwise an app
        // with windows on two workspaces always resolves to the one already in
        // view and the switcher appears to do nothing.
        let target = self.app_windows_by_recency(app_id).first().cloned();
        let wid = self.raise_app_elements(app_id, target.as_ref());
        if wid.is_none() {
            // return early
            return wid;
        }
        let wid = wid.unwrap();
        // Find the output that owns the raised window's space rather than
        // assuming the primary output, so we scroll only that output.
        self.switch_to_workspace_of_window(&wid);

        Some(wid)
    }

    pub fn focus_app_with_window(&mut self, wid: &ObjectId) -> Option<ObjectId> {
        let app_id = self
            .get_window_for_surface(wid)
            .map(|w| w.xdg_app_id())
            .unwrap_or_default();
        tracing::trace!("workspaces::focus_app_with_window {:?}", app_id);

        // Hide show desktop when focusing an app (use -2.0 to force dismiss)
        if self.get_show_desktop() {
            self.expose_show_desktop(-2.0, true);
        }

        let wid = self.raise_app_elements(&app_id, Some(wid));
        if wid.is_none() {
            // return early
            return wid;
        }
        let wid = wid.unwrap();
        // Find the output that owns the raised window's space rather than
        // assuming the primary output, so we scroll only that output.
        self.switch_to_workspace_of_window(&wid);

        Some(wid)
    }

    /// Update the workspace model using elements from Space: windows_list, app_windows_map, zindex_application_list
    /// - app_windows_map: is a map of app_id to a list of toplevel surfaces
    /// - applications_list: is the list of app_id in the order they are opened
    /// - zindex_application_list: is the list of app_id in the order they are in the zindex
    pub(crate) fn update_workspace_model(&self) {
        let windows: Vec<(ObjectId, WindowElement)> = self
            .spaces_elements()
            .filter_map(|we| we.wl_surface().map(|s| (s.as_ref().id(), we.clone())))
            .collect();

        // Include minimized windows — they are unmapped from Space but still alive.
        let minimized: Vec<(ObjectId, WindowElement)> = {
            let model = self.model.read().unwrap();
            model
                .minimized_windows
                .iter()
                .filter_map(|(id, _)| {
                    self.windows_map
                        .get(id)
                        .and_then(|we| we.wl_surface().map(|s| (s.as_ref().id(), we.clone())))
                })
                .collect()
        };

        let all_windows: Vec<&(ObjectId, WindowElement)> =
            minimized.iter().chain(windows.iter()).collect();

        {
            // reset the model
            if let Ok(mut model_mut) = self.model.write() {
                model_mut.zindex_application_list = Vec::new();
                model_mut.app_windows_map = HashMap::new();
            } else {
                return;
            }
        }

        let mut app_set = HashSet::new();
        for (window_id, we) in all_windows.iter() {
            let raw_app_id = we.xdg_app_id();
            let display_app_id = we.display_app_id(&self.display_handle);

            // Skip only if both raw and display app_id are empty
            if raw_app_id.is_empty() && display_app_id.is_empty() {
                tracing::warn!("[update_workspace_model] Skipping window with no app_id");
                continue;
            }
            if let Ok(mut model_mut) = self.model.write() {
                // Use raw_app_id for window mapping if available, otherwise use display_app_id
                let map_key = if !raw_app_id.is_empty() {
                    raw_app_id.clone()
                } else {
                    display_app_id.clone()
                };

                model_mut
                    .app_windows_map
                    .entry(map_key)
                    .or_default()
                    .push(window_id.clone());

                // Use display_app_id for UI lists (shows actual programs)
                if !model_mut.application_list.contains(&display_app_id) {
                    model_mut
                        .application_list
                        .push_front(display_app_id.clone());
                }
                if app_set.insert(display_app_id.clone()) {
                    model_mut
                        .zindex_application_list
                        .push(display_app_id.clone());
                }
            }
        }

        // Order the switcher list by focus recency (most recent LAST — the
        // consumer reverses it). Space iteration order alone puts the windows
        // of non-current workspaces first, so an app whose window lives on
        // another workspace would sink to the bottom of the switcher no matter
        // how recently it was used.
        // An app is ranked by how recently it was used, but only against the
        // apps it shares a workspace with: one whose windows are all somewhere
        // else belongs behind everything here, however recently it was used.
        #[allow(clippy::mutable_key_type)]
        let here: HashSet<ObjectId> = self
            .focused_output_workspaces()
            .and_then(|ows| ows.spaces.get(ows.current_workspace))
            .map(|space| {
                space
                    .elements()
                    .filter_map(|we| we.wl_surface().map(|s| s.as_ref().id()))
                    .collect()
            })
            .unwrap_or_default();
        {
            let mut model = self.model.write().unwrap();
            // Per app: whether any of its windows is on this workspace, and the
            // best (lowest) focus rank among them.
            let mut ranks: HashMap<String, (bool, usize)> = HashMap::new();
            for (window_id, we) in all_windows.iter() {
                let app_id = we.display_app_id(&self.display_handle);
                if app_id.is_empty() {
                    continue;
                }
                // Distance from the end of the history: 0 = most recent.
                let rank = self
                    .focus_history
                    .iter()
                    .rev()
                    .position(|h| h == window_id)
                    .unwrap_or(usize::MAX);
                let entry = ranks.entry(app_id).or_insert((false, usize::MAX));
                entry.0 |= here.contains(window_id);
                entry.1 = entry.1.min(rank);
            }
            // Stable sort, and the consumer reads the list in reverse: the
            // most recent app on this workspace has to end up LAST, and the
            // apps that are not on it first.
            model.zindex_application_list.sort_by_key(|app_id| {
                let (on_this_workspace, rank) =
                    ranks.get(app_id).copied().unwrap_or((false, usize::MAX));
                (on_this_workspace, std::cmp::Reverse(rank))
            });
        }

        // keep only app in application_list that are in zindex_application_list
        {
            let mut model: std::sync::RwLockWriteGuard<'_, WorkspacesModel> =
                self.model.write().unwrap();

            let app_list = model.zindex_application_list.clone();
            {
                // update app list
                model
                    .application_list
                    .retain(|app_id| app_list.contains(app_id));
            }

            {
                // update minimized windows — keep entries that are either still
                // mapped in a Space *or* still tracked in windows_map (minimized
                // windows are unmapped from Space but remain in windows_map).
                model
                    .minimized_windows
                    .retain(|(id, _)| all_windows.iter().any(|(wid, _)| wid == id));
            }
        }

        let model = self.model.read().unwrap();
        let event = model.clone();

        self.notify_observers(&event);
        drop(model);
        self.refresh_output_selectors();
    }

    /// Returns all the window elements from all the spaces
    /// starting from current space
    pub fn spaces_elements(&self) -> impl Iterator<Item = &WindowElement> {
        // ALL outputs' windows — this list feeds frame-callback delivery
        // (post_repaint); omitting an output freezes its clients. Per
        // output, non-current workspaces first, current last (kept from the
        // single-output ordering: later entries are treated as topmost).
        let mut result: Vec<&WindowElement> = Vec::new();
        for ows in self.output_workspaces.values() {
            for (i, space) in ows.spaces.iter().enumerate() {
                if i != ows.current_workspace {
                    result.extend(space.elements());
                }
            }
            if let Some(space) = ows.spaces.get(ows.current_workspace) {
                result.extend(space.elements());
            }
        }
        result.into_iter()
    }

    // Outputs Management

    /// Returns the list of outputs associated with the current workspace
    pub fn outputs(&self) -> impl Iterator<Item = &Output> {
        self.outputs.iter()
    }

    /// Dump every output's geometry, mode, scale and transform to the log.
    ///
    /// Called whenever the output set changes (map / unmap / suspend /
    /// restore) so a log captured while the pointer or maximize geometry goes
    /// wrong pins down which transition corrupted the layout. Both bugs read
    /// [`Workspaces::output_geometry`], so this prints exactly what that
    /// returns, plus the raw output state it is derived from.
    ///
    /// Anything the derived state cannot explain is flagged inline:
    /// `MISSING-GEOMETRY` for an output that is live but has no space mapped
    /// (an `output_geometry(..).unwrap()` on that output would panic), and
    /// `ORPHANED` for workspace data left behind by an output that is no
    /// longer live.
    pub fn log_output_topology(&self, reason: &str) {
        let primary = self.primary_output.as_ref().map(|o| o.name());
        tracing::info!(
            target: "otto::outputs",
            "output set changed ({reason}): {} live, {} suspended, primary={}",
            self.outputs.len(),
            self.suspended_outputs.len(),
            primary.as_deref().unwrap_or("<none>"),
        );

        for output in &self.outputs {
            let name = output.name();
            let geo = self.output_geometry(output);
            let geo_str = match geo {
                Some(g) => format!(
                    "loc=({},{}) size={}x{}",
                    g.loc.x, g.loc.y, g.size.w, g.size.h
                ),
                None => "MISSING-GEOMETRY".to_string(),
            };
            let mode = output
                .current_mode()
                .map(|m| format!("{}x{}@{}", m.size.w, m.size.h, m.refresh))
                .unwrap_or_else(|| "<no mode>".to_string());
            let kind = if crate::virtual_output::is_virtual_output(output) {
                if crate::virtual_output::is_unreachable_virtual_output(output) {
                    "virtual/non-interactive"
                } else {
                    "virtual/interactive"
                }
            } else {
                "physical"
            };
            tracing::info!(
                target: "otto::outputs",
                "  {name}{}: {kind} logical[{geo_str}] mode={mode} scale={:.2} transform={:?}",
                if primary.as_deref() == Some(name.as_str()) {
                    " (primary)"
                } else {
                    ""
                },
                output.current_scale().fractional_scale(),
                output.current_transform(),
            );
        }

        for (name, suspended) in &self.suspended_outputs {
            tracing::info!(
                target: "otto::outputs",
                "  {name}: suspended at ({},{}) was_primary={}",
                suspended.location.x,
                suspended.location.y,
                suspended.was_primary,
            );
        }

        for name in self.output_workspaces.keys() {
            if !self.outputs.iter().any(|o| o.name() == *name)
                && !self.suspended_outputs.contains_key(name)
            {
                tracing::warn!(
                    target: "otto::outputs",
                    "  {name}: ORPHANED workspace data (no live or suspended output)",
                );
            }
        }
    }

    /// Attach a new output to every workspace
    pub fn map_output(
        &mut self,
        output: &Output,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
    ) {
        self.map_output_with_primary(output, location, false);
    }

    /// Attach a new output to every workspace, optionally marking it as primary.
    pub fn map_output_with_primary(
        &mut self,
        output: &Output,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
        is_primary: bool,
    ) {
        let location = location.into();

        // Guard against re-mapping an already-mapped output (e.g., on resize or
        // resume after suspend).  Re-register the output with each space so
        // Smithay knows the new geometry, then refresh layout sizes.
        if self.output_workspaces.contains_key(&output.name()) {
            // The output may have been suspended (removed from self.outputs but
            // output_workspaces kept intact). Re-add it if missing.
            if !self.outputs.iter().any(|o| o.name() == output.name()) {
                self.outputs.push(output.clone());
            }
            if is_primary || self.primary_output.is_none() {
                self.primary_output = Some(output.clone());
            }

            if let Some(ows) = self.output_workspaces.get_mut(&output.name()) {
                for space in ows.spaces.iter_mut() {
                    space.map_output(output, location);
                }
                // Keep layer sizes in sync with the (possibly new) output mode.
                if let Some((w, h)) = output
                    .current_mode()
                    .map(|m| (m.size.w as f32, m.size.h as f32))
                {
                    ows.layer_shell_background
                        .set_size(layers::types::Size::points(w, h), None);
                    ows.layer_shell_bottom
                        .set_size(layers::types::Size::points(w, h), None);
                    ows.output_layer
                        .set_size(layers::types::Size::points(w, h), None);
                }
                // Output subtrees overlap at scene (0,0) — scene coords are
                // output-local; `location` only positions the smithay Space.
                ows.output_layer.set_position((0.0, 0.0_f32), None);
            }
            self.sync_model_from_primary();
            self.update_workspaces_layout();
            self.log_output_topology(&format!("remap {}", output.name()));
            return;
        }

        self.outputs.push(output.clone());
        if is_primary || self.primary_output.is_none() {
            self.primary_output = Some(output.clone());
        }

        // Count workspaces from primary output (default 2 if no primary yet)
        let n_workspaces = self
            .primary_output_name()
            .as_ref()
            .and_then(|n| self.output_workspaces.get(n))
            .map(|ows| ows.workspace_views.len())
            .unwrap_or(0)
            .max(2);

        // Physical size from output mode
        let (phys_w, phys_h) = output
            .current_mode()
            .map(|m| (m.size.w as f32, m.size.h as f32))
            .unwrap_or_else(|| self.with_model(|m| (m.width as f32, m.height as f32)));

        // Per-output container layer. All output subtrees overlap at scene
        // (0,0): each output renders only its own subtree, so scene
        // coordinates are output-local by construction. `location` positions
        // the output in the GLOBAL space (smithay Space / input) only.
        let output_layer = self.layers_engine.new_layer();
        output_layer.set_key(format!("output_{}", output.name()));
        output_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        output_layer.set_size(layers::types::Size::points(phys_w, phys_h), None);
        output_layer.set_position((0.0, 0.0), None);
        output_layer.set_pointer_events(false);
        let _ = self.layers_engine.add_layer(&output_layer);

        // Create the workspaces_layer for this output (workspace content, inside container)
        let workspaces_layer = self.layers_engine.new_layer();
        workspaces_layer.set_key(format!("workspaces_{}", output.name()));
        workspaces_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            display: taffy::Display::Flex,
            ..Default::default()
        });
        workspaces_layer.set_size(layers::types::Size::auto(), None);
        workspaces_layer.set_pointer_events(false);

        let is_this_primary = self.primary_output.as_ref().map(|o| o.name()) == Some(output.name());

        // Create per-output layer_shell_background container, sized to the output's physical
        // dimensions so mirror bounds are output-local rather than scene-root-relative.
        let layer_shell_background = self.layers_engine.new_layer();
        layer_shell_background.set_key(format!("layer_shell_background_{}", output.name()));
        layer_shell_background.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        layer_shell_background.set_size(layers::types::Size::points(phys_w, phys_h), None);
        layer_shell_background.set_pointer_events(false);
        layer_shell_background.set_hidden(true);

        // Attach layer_shell_background to the scene root (not the output_layer) so it
        // doesn't get rendered directly — it only serves as a content source for mirror
        // layers inside workspaces and expose views.
        // Same treatment for the widget layer: an offscreen content source,
        // mirrored where it should appear. Sized and positioned identically so
        // the two mirrors line up pixel for pixel.
        let layer_shell_bottom = self.layers_engine.new_layer();
        layer_shell_bottom.set_key(format!("layer_shell_bottom_{}", output.name()));
        layer_shell_bottom.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        layer_shell_bottom.set_size(layers::types::Size::points(phys_w, phys_h), None);
        layer_shell_bottom.set_pointer_events(false);
        layer_shell_bottom.set_hidden(true);

        if let Some(root) = self
            .layers_engine
            .scene_root()
            .and_then(|id| self.layers_engine.get_layer(&id))
        {
            let _ = root.add_sublayer(&layer_shell_background);
            let _ = root.add_sublayer(&layer_shell_bottom);
        }

        // Single container for all workspace backgrounds, first child of workspaces_layer
        // so it scrolls in sync automatically. The SceneDmabufElement for the background
        // KMS plane renders this node.
        let background_plane = self.layers_engine.new_layer();
        background_plane.set_key(format!("background_plane_{}", output.name()));
        background_plane.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        background_plane.set_size(layers::types::Size::auto(), None);
        background_plane.set_pointer_events(false);
        let _ = workspaces_layer.add_sublayer(&background_plane);

        // Single container for all workspace window layers, child of workspaces_layer.
        // Rendered as a single KMS windows plane; scrolls in sync automatically.
        let windows_plane = self.layers_engine.new_layer();
        windows_plane.set_key(format!("windows_plane_{}", output.name()));
        windows_plane.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        windows_plane.set_size(layers::types::Size::auto(), None);
        windows_plane.set_pointer_events(false);
        let _ = workspaces_layer.add_sublayer(&windows_plane);

        // Container for the window promoted to its own KMS plane. Added after
        // windows_plane so it draws above it — a promoted window is always the
        // topmost window (that is an eligibility rule), so the scene order
        // matches the plane order.
        let promoted_plane = self.layers_engine.new_layer();
        promoted_plane.set_key(format!("promoted_plane_{}", output.name()));
        promoted_plane.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        promoted_plane.set_size(layers::types::Size::auto(), None);
        promoted_plane.set_pointer_events(false);
        // Same clipping as the `windows_layer` it stands in for, so a window
        // straddling a workspace edge is cropped identically in either plane.
        promoted_plane.set_clip_children(true, None);
        promoted_plane.set_clip_content(true, None);
        let _ = workspaces_layer.add_sublayer(&promoted_plane);

        // Attach layers to output_layer in z-order (bottom to top):
        // workspaces (background_plane, windows_plane, expose_layer) →
        // dock (primary) → overlay_plane (layer_shell_top, workspace_selector,
        // app_switcher, layer_shell_overlay, dnd/osd, popups)
        let _ = output_layer.add_sublayer(&workspaces_layer);

        // Create a per-output expose layer
        let expose_layer = self.layers_engine.new_layer();
        expose_layer.set_key(format!("expose_{}", output.name()));
        expose_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        expose_layer.set_pointer_events(false);
        expose_layer.set_hidden(true);
        expose_layer.set_picture_cached(false);
        expose_layer.set_image_cached(false);
        let _ = workspaces_layer.add_sublayer(&expose_layer);

        // Black floor under the exposé previews. The gaps between workspace
        // slots (and the overscroll bands past the first/last workspace) have
        // no exposé content, so whatever plane sits below shows through —
        // which is the background plane, still holding the pre-exposé
        // wallpaper. The gap is supposed to read as black. First child of
        // `expose_layer` so every selector root draws above it; sized far
        // beyond any realistic workspace extent instead of tracking layout,
        // because it scrolls with `expose_layer` and just has to cover the
        // viewport at every reachable offset.
        let expose_backdrop = self.layers_engine.new_layer();
        expose_backdrop.set_key(format!("expose_backdrop_{}", output.name()));
        expose_backdrop.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        // Sized to cover ~10 workspaces of horizontal scroll plus overscroll,
        // not "infinite": pathological bounds propagate into
        // bounds_with_children and can blow up downstream surface allocations.
        expose_backdrop.set_position((-(phys_w * 2.0), 0.0), None);
        expose_backdrop.set_size(layers::types::Size::points(phys_w * 14.0, phys_h), None);
        expose_backdrop
            .set_background_color(layers::types::Color::new_rgba(0.0, 0.0, 0.0, 1.0), None);
        expose_backdrop.set_pointer_events(false);
        let _ = expose_layer.add_sublayer(&expose_backdrop);

        // overlay_plane: workspace selector, app switcher, layer shell UI,
        // OSD and DnD — above windows/expose, below popups and dock.
        let overlay_plane = self.layers_engine.new_layer();
        overlay_plane.set_key(format!("overlay_plane_{}", output.name()));
        overlay_plane.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        overlay_plane.set_size(layers::types::Size::points(phys_w, phys_h), None);
        overlay_plane.set_pointer_events(false);

        // Per-output workspace selector strip. Each output owns its own so the
        // expose UI appears on every screen with correctly-sized previews of
        // that output's own workspaces. Lives in this output's overlay_plane.
        let selector_layer = self.layers_engine.new_layer();
        selector_layer.set_key(format!("workspace_selector_{}", output.name()));
        selector_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        selector_layer.set_pointer_events(false);
        let workspace_selector = Arc::new(WorkspaceSelectorView::new(
            self.layers_engine.clone(),
            selector_layer.clone(),
            self.remove_workspace_sender.clone(),
            self.label_editing.clone(),
            self.rename_workspace_sender.clone(),
        ));
        let _ = overlay_plane.add_sublayer(&selector_layer);

        let switcher_plane = self.layers_engine.new_layer();
        switcher_plane.set_key(format!("switcher_plane_{}", output.name()));
        switcher_plane.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        switcher_plane.set_size(layers::types::Size::points(phys_w, phys_h), None);
        switcher_plane.set_pointer_events(false);

        let dock_plane = self.layers_engine.new_layer();
        dock_plane.set_key(format!("dock_plane_{}", output.name()));
        dock_plane.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        dock_plane.set_size(layers::types::Size::points(phys_w, phys_h), None);
        dock_plane.set_pointer_events(false);

        // The lock plane covers this output whole while the session is locked.
        // It is created for every output up front so a lock can blank a screen
        // that no locker has drawn on yet, and hotplug needs no extra wiring.
        let lock_plane = self.layers_engine.new_layer();
        lock_plane.set_key(format!("lock_plane_{}", output.name()));
        lock_plane.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        lock_plane.set_size(layers::types::Size::points(phys_w, phys_h), None);
        lock_plane.set_background_color(
            layers::types::PaintColor::Solid {
                color: layers::types::Color::new_rgba(0.0, 0.0, 0.0, 1.0),
            },
            None,
        );
        // Pointer events on, so a click while locked cannot reach the session
        // underneath even before the locker has mapped a surface.
        lock_plane.set_pointer_events(true);
        lock_plane.set_hidden(true);

        if is_this_primary {
            // Wire the primary output's expose layer into self.expose_layer so all
            // existing show/hide logic works unchanged.
            self.expose_layer = expose_layer.clone();
            // overlay_layer contains dnd and osd
            let _ = self.overlay_layer.add_sublayer(&self.dnd_view.layer);
            let _ = self
                .overlay_layer
                .add_sublayer(&self.tiling_overlay.wrap_layer);
            let _ = self.overlay_layer.add_sublayer(&self.osd.wrap_layer);
            // App icons manager lives at the root — sibling of output layers, never rendered
            // on any output, but present in the scene so its subtree gets laid out.
            if let Some(root) = self
                .layers_engine
                .scene_root()
                .and_then(|id| self.layers_engine.get_layer(&id))
            {
                let _ = root.add_sublayer(&self.app_icons_manager.container);
            }
            // Group overlay UI into the overlay_plane node (single KMS plane),
            // including popups which sit above other overlay content.
            let _ = overlay_plane.add_sublayer(&self.layer_shell_top);
            let _ = overlay_plane.add_sublayer(&self.layer_shell_overlay);
            let _ = overlay_plane.add_sublayer(&self.overlay_layer);
            let _ = overlay_plane.add_sublayer(&self.popup_overlay.layer.clone());
            let _ = output_layer.add_sublayer(&overlay_plane.clone());
            // App switcher and dock get their own full-screen containers (so
            // their existing centering layout keeps working) rendered through
            // strip viewports onto dedicated KMS planes — their animations no
            // longer redraw the shared overlay buffer.
            let _ = switcher_plane.add_sublayer(&self.app_switcher.wrap_layer.clone());
            let _ = output_layer.add_sublayer(&switcher_plane.clone());
            let _ = dock_plane.add_sublayer(&self.dock.wrap_layer.clone());
            let _ = output_layer.add_sublayer(&dock_plane.clone());
        } else {
            let _ = output_layer.add_sublayer(&overlay_plane.clone());
            let _ = output_layer.add_sublayer(&switcher_plane.clone());
            let _ = output_layer.add_sublayer(&dock_plane.clone());
        }
        // Added last on every output: nothing may draw above a locked screen.
        let _ = output_layer.add_sublayer(&lock_plane.clone());

        // Login mode keeps the scene shape identical (so nothing downstream has
        // to special-case a missing node) but never lets session chrome become
        // visible: the greeter is the only thing on screen.
        if crate::login::is_login_mode() {
            dock_plane.set_hidden(true);
            switcher_plane.set_hidden(true);
            selector_layer.set_hidden(true);
        }

        let workspace_counter_start = self.with_model(|m| m.workspace_counter);
        let mut spaces = Vec::new();
        let mut workspace_views = Vec::new();

        for i in 0..n_workspaces {
            let mut space = Space::default();
            space.map_output(output, location);
            spaces.push(space);

            let idx = workspace_counter_start + i + 1;
            let workspace = Arc::new(WorkspaceView::new(
                idx,
                self.layers_engine.clone(),
                &workspaces_layer,
                self.overlay_layer.clone(),
                &layer_shell_background,
                &layer_shell_bottom,
            ));
            let _ = expose_layer.add_sublayer(&workspace.window_selector_view.window_selector_root);
            // Attach this workspace's background into the shared background_plane
            // so all workspace backgrounds live under one node for the KMS plane.
            let _ = background_plane.add_sublayer(&workspace.workspace_background);
            let _ = windows_plane.add_sublayer(&workspace.windows_layer);
            if let Some(name) = persisted_workspace_name(&output.name(), i) {
                workspace.set_custom_name(Some(name));
            }
            workspace_views.push(workspace);
        }

        self.with_model_mut(|m| {
            m.workspace_counter = workspace_counter_start + n_workspaces;
        });

        let ows = OutputWorkspaces {
            current_workspace: 0,
            spaces,
            output_layer,
            workspaces_layer,
            expose_layer,
            layer_shell_background,
            layer_shell_bottom,
            workspace_views,
            background_plane,
            windows_plane,
            promoted_plane,
            overlay_plane,
            switcher_plane,
            dock_plane,
            lock_plane,
            workspace_selector,
        };
        self.output_workspaces.insert(output.name(), ows);
        self.sync_model_from_primary();
        self.update_workspaces_layout();
        self.with_model(|m| self.notify_observers(m));
        self.log_output_topology(&format!("map {}", output.name()));
    }

    /// Returns the primary output (used for dock placement and hot zone).
    pub fn primary_output(&self) -> Option<&Output> {
        self.primary_output.as_ref()
    }

    /// Return the currently focused output (last output under the pointer), or primary.
    /// Safe to call from input handlers — reads from the model cache, no seat lock needed.
    pub fn focused_output(&self) -> Option<&Output> {
        let name = self.with_model(|m| m.focused_output_name.clone());
        name.as_deref()
            .and_then(|n| self.outputs.iter().find(|o| o.name() == n))
            .or(self.primary_output.as_ref())
    }

    /// The output to use for a client that did not name one (layer-shell
    /// surfaces with a NULL `wl_output`, fullscreen requests without an
    /// output). Never a virtual output: those exist for remote/mirror
    /// consumers and carry none of the primary output's chrome, so a surface
    /// assigned to one is effectively invisible on the physical screen.
    pub fn default_client_output(&self) -> Option<&Output> {
        let real = |o: &&Output| !crate::virtual_output::is_virtual_output(o);
        self.focused_output()
            .filter(real)
            .or_else(|| self.primary_output.as_ref().filter(real))
            .or_else(|| self.outputs.iter().find(real))
            .or_else(|| self.outputs.first())
    }

    /// Detach an output from every workspace
    pub fn unmap_output(&mut self, output: &Output) {
        self.suspended_outputs.remove(&output.name());
        self.outputs.retain(|o| o != output);
        if self.primary_output.as_ref() == Some(output) {
            self.primary_output = self.outputs.first().cloned();
        }
        // Remove the output's workspace set (dropping workspaces_layer removes it from scene)
        self.output_workspaces.remove(&output.name());
        // The app switcher may have been parented into the departing output's
        // switcher plane — that node is gone now, so bring the panel home to
        // the primary output instead of leaving it detached from the scene.
        let hosted_here =
            self.app_switcher_output.read().unwrap().as_deref() == Some(&output.name());
        if hosted_here {
            *self.app_switcher_output.write().unwrap() = None;
            if let Some(ows) = self.primary_output_workspaces() {
                let _ = ows
                    .switcher_plane
                    .add_sublayer(&self.app_switcher.wrap_layer);
            }
        }
        self.sync_model_from_primary();
        self.log_output_topology(&format!("unmap {}", output.name()));
    }

    /// Suspend an output without destroying its workspace data.
    ///
    /// Used for lid-close: the DRM surface is torn down (no rendering) but
    /// all workspaces, windows, and scene-graph layers are preserved so they
    /// can be instantly restored when the output comes back.
    /// `global` is the output's `wl_output`, handed over so it outlives the
    /// DRM surface the caller is about to drop: everything that holds an
    /// `Output` — layer surfaces and their layer map, lock surfaces, window
    /// assignments — keys off the object, and a client that watched its
    /// output go away has no reason to come back on its own.
    pub fn suspend_output(&mut self, output: &Output, global: Option<GlobalId>) {
        // Remember where the output was and whether it was primary, so a
        // reconnect of the same connector restores the pre-suspend
        // arrangement (position AND primary status).
        let location = self
            .output_geometry(output)
            .map(|g| g.loc)
            .unwrap_or_default();
        let was_primary = self.primary_output.as_ref() == Some(output);
        self.suspended_outputs.insert(
            output.name(),
            SuspendedOutput {
                output: output.clone(),
                location,
                was_primary,
                global,
            },
        );

        self.outputs.retain(|o| o != output);
        if self.primary_output.as_ref() == Some(output) {
            self.primary_output = self.outputs.first().cloned();
        }
        // Intentionally do NOT remove from output_workspaces — keep windows alive.
        self.sync_model_from_primary();
        self.log_output_topology(&format!("suspend {}", output.name()));
    }

    /// Take (consume) a previously suspended output, if any.
    pub fn take_suspended_output(&mut self, output_name: &str) -> Option<SuspendedOutput> {
        self.suspended_outputs.remove(output_name)
    }

    // Workspaces Management

    pub fn add_workspace(&mut self) -> (usize, Arc<WorkspaceView>) {
        let output_names: Vec<String> = self.output_workspaces.keys().cloned().collect();

        let counter = self.with_model_mut(|m| {
            m.workspace_counter += 1;
            m.workspace_counter
        });

        let layers_engine = self.layers_engine.clone();
        let overlay_layer = self.overlay_layer.clone();
        let outputs = self.outputs.clone();

        let mut primary_result: Option<(usize, Arc<WorkspaceView>)> = None;
        let primary_name = self.primary_output_name();

        for name in &output_names {
            if let Some(ows) = self.output_workspaces.get_mut(name) {
                let output = outputs.iter().find(|o| o.name() == *name).cloned();

                let mut new_space = Space::default();
                if let Some(ref o) = output {
                    if let Some(existing_space) = ows.spaces.first() {
                        let geo = existing_space.output_geometry(o).unwrap_or_default();
                        new_space.map_output(o, geo.loc);
                    } else {
                        new_space.map_output(o, (0, 0));
                    }
                }
                ows.spaces.push(new_space);

                let workspace = Arc::new(WorkspaceView::new(
                    counter,
                    layers_engine.clone(),
                    &ows.workspaces_layer,
                    overlay_layer.clone(),
                    &ows.layer_shell_background,
                    &ows.layer_shell_bottom,
                ));
                let _ = ows
                    .expose_layer
                    .add_sublayer(&workspace.window_selector_view.window_selector_root);

                let index = ows.workspace_views.len();
                if let Some(custom) = persisted_workspace_name(name, index) {
                    workspace.set_custom_name(Some(custom));
                }
                ows.workspace_views.push(workspace.clone());

                if primary_name.as_deref() == Some(name.as_str()) {
                    primary_result = Some((index, workspace));
                }
            }
        }

        self.sync_model_from_primary();
        self.with_model(|m| self.notify_observers(m));
        self.update_workspaces_layout();

        primary_result.unwrap_or_else(|| {
            (
                0,
                self.primary_output_workspaces().unwrap().workspace_views[0].clone(),
            )
        })
    }

    /// Add a workspace to a SINGLE output only. Workspaces are independent per
    /// output, so this leaves other outputs' workspace sets untouched. Returns
    /// the new workspace's position index on that output and its view.
    pub fn add_workspace_to_output(
        &mut self,
        output_name: &str,
    ) -> Option<(usize, Arc<WorkspaceView>)> {
        if !self.output_workspaces.contains_key(output_name) {
            return None;
        }
        let counter = self.with_model_mut(|m| {
            m.workspace_counter += 1;
            m.workspace_counter
        });
        let layers_engine = self.layers_engine.clone();
        let overlay_layer = self.overlay_layer.clone();
        let output = self
            .outputs
            .iter()
            .find(|o| o.name() == output_name)
            .cloned();

        let result = {
            let ows = self.output_workspaces.get_mut(output_name)?;

            let mut new_space = Space::default();
            if let Some(ref o) = output {
                if let Some(existing_space) = ows.spaces.first() {
                    let geo = existing_space.output_geometry(o).unwrap_or_default();
                    new_space.map_output(o, geo.loc);
                } else {
                    new_space.map_output(o, (0, 0));
                }
            }
            ows.spaces.push(new_space);

            let workspace = Arc::new(WorkspaceView::new(
                counter,
                layers_engine.clone(),
                &ows.workspaces_layer,
                overlay_layer.clone(),
                &ows.layer_shell_background,
                &ows.layer_shell_bottom,
            ));
            // Wire the new workspace into this output's planes exactly like
            // map_output_with_primary does, so its background/windows actually
            // render when scrolled to (not only in the selector preview).
            let _ = ows
                .expose_layer
                .add_sublayer(&workspace.window_selector_view.window_selector_root);
            let _ = ows
                .background_plane
                .add_sublayer(&workspace.workspace_background);
            let _ = ows.windows_plane.add_sublayer(&workspace.windows_layer);

            let index = ows.workspace_views.len();
            if let Some(name) = persisted_workspace_name(output_name, index) {
                workspace.set_custom_name(Some(name));
            }
            ows.workspace_views.push(workspace.clone());
            (index, workspace)
        };

        self.sync_model_from_primary();
        self.with_model(|m| self.notify_observers(m));
        self.update_workspaces_layout();
        Some(result)
    }

    /// Remove the workspace at `pos` from a SINGLE output only. Windows on the
    /// removed workspace are moved to that output's current workspace. No-op if
    /// the output has one workspace, `pos` is invalid, or the target is a
    /// fullscreen workspace that still has windows.
    pub fn remove_workspace_from_output(&mut self, output_name: &str, pos: usize) {
        let num_spaces = self
            .output_workspaces
            .get(output_name)
            .map(|ows| ows.spaces.len())
            .unwrap_or(0);
        if num_spaces <= 1 || pos >= num_spaces {
            return;
        }

        // Never remove a fullscreen workspace that still has windows.
        let blocked = self
            .output_workspaces
            .get(output_name)
            .map(|ows| {
                let fullscreen = ows
                    .workspace_views
                    .get(pos)
                    .map(|w| w.get_fullscreen_mode())
                    .unwrap_or(false);
                let count = ows
                    .spaces
                    .get(pos)
                    .map(|s| s.elements().count())
                    .unwrap_or(0);
                fullscreen && count > 0
            })
            .unwrap_or(false);
        if blocked {
            return;
        }

        // Destination = the output's current workspace, adjusted for the removal
        // shift (a workspace at/after `pos` moves down by one).
        let cur = self
            .output_workspaces
            .get(output_name)
            .map(|ows| ows.current_workspace)
            .unwrap_or(0);
        let dest_after = if cur > pos {
            cur - 1
        } else {
            cur.min(num_spaces - 2)
        };

        // Collect windows to relocate off the removed workspace.
        let windows_to_move: Vec<(
            WindowElement,
            smithay::utils::Point<i32, smithay::utils::Logical>,
        )> = self
            .output_workspaces
            .get(output_name)
            .and_then(|ows| ows.spaces.get(pos))
            .map(|space| {
                space
                    .elements()
                    .map(|e| (e.clone(), space.element_location(e).unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default();

        // Remove the space + view at `pos` and its scene sublayers.
        if let Some(ows) = self.output_workspaces.get_mut(output_name) {
            if pos < ows.workspace_views.len() {
                let view = ows.workspace_views.remove(pos);
                view.workspace_background.remove();
                view.windows_layer.remove();
                view.window_selector_view.window_selector_root.remove();
            }
            if pos < ows.spaces.len() {
                ows.spaces.remove(pos);
            }
            if !ows.spaces.is_empty() && ows.current_workspace >= ows.spaces.len() {
                ows.current_workspace = ows.spaces.len() - 1;
            }
        }

        // Re-home the removed workspace's windows onto the destination workspace
        // of THIS output, keeping the move scoped to one output.
        for (window, location) in windows_to_move {
            if window.is_fullscreen() {
                window.set_fullscreen(false, dest_after);
            }
            if let Some(ows) = self.output_workspaces.get_mut(output_name) {
                if let Some(space) = ows.spaces.get_mut(dest_after) {
                    space.map_element(window.clone(), location, false);
                }
            }
            if let Some(view) = self
                .output_workspaces
                .get(output_name)
                .and_then(|ows| ows.workspace_views.get(dest_after))
                .cloned()
            {
                if let Some(w) = self.windows_map.get(&window.id()) {
                    view.map_window(w, location, None);
                }
            }
        }

        self.sync_model_from_primary();
        self.update_workspaces_layout();
        if let Some(output) = self
            .outputs
            .iter()
            .find(|o| o.name() == output_name)
            .cloned()
        {
            let dest = self
                .output_workspaces
                .get(output_name)
                .map(|ows| ows.current_workspace)
                .unwrap_or(0);
            self.set_workspace_for_output(
                &output,
                dest,
                Some(Transition {
                    delay: 0.0,
                    timing: TimingFunction::linear(0.0),
                }),
            );
        }
        self.with_model(|m| self.notify_observers(m));
    }

    pub fn get_next_free_workspace(&mut self) -> (usize, Arc<WorkspaceView>) {
        let current_workspace = self.get_current_workspace_index();
        let num_spaces = self
            .primary_output_workspaces()
            .map(|ows| ows.spaces.len())
            .unwrap_or(0);
        if current_workspace < num_spaces.saturating_sub(1) {
            for i in current_workspace + 1..num_spaces {
                let is_empty = self
                    .primary_output_workspaces()
                    .and_then(|ows| ows.spaces.get(i))
                    .map(|s| s.elements().count() == 0)
                    .unwrap_or(false);
                if is_empty {
                    return (i, self.with_model(|m| m.workspaces[i].clone()));
                }
            }
        }
        self.add_workspace()
    }

    /// Per-output variant of `get_next_free_workspace`: the first empty
    /// workspace after the output's current one, or a new workspace created on
    /// that output alone.
    pub fn get_next_free_workspace_on_output(
        &mut self,
        output_name: &str,
    ) -> Option<(usize, Arc<WorkspaceView>)> {
        let (current, num_spaces) = {
            let ows = self.output_workspaces.get(output_name)?;
            (ows.current_workspace, ows.spaces.len())
        };
        for i in current + 1..num_spaces {
            let ows = self.output_workspaces.get(output_name)?;
            let empty = ows
                .spaces
                .get(i)
                .map(|s| s.elements().count() == 0)
                .unwrap_or(false);
            if empty {
                if let Some(ws) = ows.workspace_views.get(i).cloned() {
                    return Some((i, ws));
                }
            }
        }
        self.add_workspace_to_output(output_name)
    }

    /// Move a window into `workspace_index` on a single output, unmapping it
    /// from wherever it currently lives. `location` is in global logical
    /// coordinates.
    pub fn move_window_to_workspace_on_output(
        &mut self,
        output: &Output,
        we: &WindowElement,
        workspace_index: usize,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
    ) {
        self.move_window_to_workspace_on_output_with_activate(
            output,
            we,
            workspace_index,
            location,
            false,
        );
    }

    pub fn move_window_to_workspace_on_output_with_activate(
        &mut self,
        output: &Output,
        we: &WindowElement,
        workspace_index: usize,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
        activate: bool,
    ) {
        let location = location.into();
        let id = we.id();

        // Unmap from every space (and view) that currently holds the window.
        let mut source_indices: Vec<(String, usize)> = Vec::new();
        for (name, ows) in self.output_workspaces.iter() {
            for (i, space) in ows.spaces.iter().enumerate() {
                if space.elements().any(|e| e.id() == id) {
                    source_indices.push((name.clone(), i));
                }
            }
        }
        for (name, i) in &source_indices {
            if let Some(ows) = self.output_workspaces.get_mut(name) {
                if let Some(view) = ows.workspace_views.get(*i) {
                    // Keep the mirror node alive — the destination view
                    // re-parents the same one — but drop it from the source
                    // selector's map, or that selector goes on holding a
                    // window it no longer shows.
                    view.unmap_window_keep_mirror(&id);
                }
                if let Some(space) = ows.spaces.get_mut(*i) {
                    space.unmap_elem(we);
                }
            }
        }

        let name = output.name();
        {
            let Some(ows) = self.output_workspaces.get_mut(&name) else {
                return;
            };
            let Some(space) = ows.spaces.get_mut(workspace_index) else {
                return;
            };
            space.map_element(we.clone(), location, activate);
        }
        let view = self
            .output_workspaces
            .get(&name)
            .and_then(|ows| ows.workspace_views.get(workspace_index).cloned());
        if let Some(view) = view {
            if let Some(window) = self.windows_map.get(&id) {
                // Scene layers are output-local; space locations are global.
                let local = location - output.current_location();
                view.map_window(window, local, None);
            }
        }

        // A move is the one moment both grids are certain to have changed, so
        // neither may be skipped by the cached-layout check. The exposé drag
        // already re-laid the source grid out *without* the dragged window
        // when it was picked up, which leaves the cached hash equal to the
        // post-move one: the drop would then apply nothing, and the grid kept
        // the layout it was dropped on until some unrelated client commit
        // moved the hash again seconds later.
        for (name, src) in &source_indices {
            self.invalidate_expose_layout(name, *src);
            self.expose_update_if_needed_workspace(*src);
        }
        self.invalidate_expose_layout(&name, workspace_index);
        self.expose_update_if_needed_workspace(workspace_index);

        // The window changed workspace, so everything derived from the model
        // — the selector previews' window counts, the app switcher's z-order,
        // the dock's app list — is now stale. Without this the stale state
        // survives until some unrelated window maps or closes.
        self.update_workspace_model();
    }

    /// Drop the cached exposé grid of one workspace, so the next layout pass
    /// recomputes and re-applies it even if the window set hashes the same,
    /// and drop the selection with it: the highlight and its label address a
    /// preview by index into that grid, and the window that was under the
    /// pointer has just left it.
    fn invalidate_expose_layout(&self, output_name: &str, workspace_index: usize) {
        if let Some(workspace) = self
            .output_workspaces
            .get(output_name)
            .and_then(|ows| ows.workspace_views.get(workspace_index))
        {
            workspace.window_selector_view.clear_selection();
            workspace.window_selector_view.invalidate_layout();
        }
    }

    /// Per-output variant of `defer_remove_workspace_at`: remove workspace `n`
    /// from a single output once `transaction` finishes.
    pub fn defer_remove_workspace_on_output(
        &self,
        output_name: &str,
        n: usize,
        transaction: &TransactionRef,
    ) {
        let sender = self.remove_workspace_sender.clone();
        let name = output_name.to_string();
        transaction.on_finish(
            move |_: &Layer, _: f32| {
                let _ = sender.send((Some(name.clone()), n));
            },
            true,
        );
    }

    /// Schedule workspace removal after an animation completes.
    /// The workspace at position `n` will be removed when `transaction` finishes.
    /// Schedule removal of the workspace at position `n` across ALL outputs
    /// (lockstep) once `transaction` finishes. Used by the fullscreen-close
    /// path, whose dedicated workspace is created lockstep. Per-output removal
    /// (from a selector) goes through `remove_workspace_from_output`.
    pub fn defer_remove_workspace_at(&self, n: usize, transaction: &TransactionRef) {
        let sender = self.remove_workspace_sender.clone();
        transaction.on_finish(
            move |_: &Layer, _: f32| {
                let _ = sender.send((None, n));
            },
            true,
        );
    }

    pub fn remove_workspace_at(&mut self, n: usize) {
        let num_spaces = self
            .primary_output_workspaces()
            .map(|ows| ows.spaces.len())
            .unwrap_or(0);
        if num_spaces <= 1 {
            return;
        }

        let workspace_model = self.with_model_mut(|m| {
            if m.workspaces.len() == 1 {
                return m.clone();
            }

            if n < m.workspaces.len() {
                m.workspaces.remove(n);
                if m.current_workspace >= m.workspaces.len() {
                    m.current_workspace = m.workspaces.len() - 1;
                }
            }
            m.clone()
        });

        if n < num_spaces {
            if let Some(ws) = self.get_workspace_at(n) {
                let window_count = self
                    .primary_output_workspaces()
                    .and_then(|ows| ows.spaces.get(n))
                    .map(|s| s.elements().count())
                    .unwrap_or(0);
                if ws.get_fullscreen_mode() && window_count > 0 {
                    // Do not remove a fullscreen workspace that still has windows
                    return;
                }
            }
            // Collect windows to move from primary output space
            let windows_to_move: Vec<(
                WindowElement,
                smithay::utils::Point<i32, smithay::utils::Logical>,
            )> = if let Some(pows) = self.primary_output_workspaces() {
                if let Some(space) = pows.spaces.get(n) {
                    space
                        .elements()
                        .map(|e| {
                            let location = space.element_location(e).unwrap_or_default();
                            (e.clone(), location)
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // Remove space at index n from all outputs
            for ows in self.output_workspaces.values_mut() {
                if n < ows.spaces.len() {
                    ows.spaces.remove(n);
                }
                if n < ows.workspace_views.len() {
                    ows.workspace_views.remove(n);
                }
                // Clamp current_workspace so it never exceeds the new spaces length.
                if !ows.spaces.is_empty() && ows.current_workspace >= ows.spaces.len() {
                    ows.current_workspace = ows.spaces.len() - 1;
                }
            }

            for (e, location) in windows_to_move {
                if e.is_fullscreen() {
                    e.set_fullscreen(false, workspace_model.current_workspace);
                    if let Some(ws) = self.get_workspace_at(workspace_model.current_workspace) {
                        ws.set_fullscreen_mode(false);
                        ws.set_fullscreen_animating(false);
                        ws.set_name(None);
                    }
                }
                self.move_window_to_workspace(&e, workspace_model.current_workspace, location);
            }

            if self.get_show_all() {
                self.expose_update_if_needed_workspace(workspace_model.current_workspace);
            }
        }
        self.update_workspaces_layout();
        self.scroll_to_workspace_index(
            workspace_model.current_workspace,
            Some(Transition {
                delay: 0.0,
                timing: TimingFunction::linear(0.0),
            }),
        );
        self.notify_observers(&workspace_model);
    }

    pub fn get_workspace_at(&self, i: usize) -> Option<Arc<WorkspaceView>> {
        self.with_model(|m| m.workspaces.get(i).cloned())
    }

    /// Windows eligible for direct client-buffer scanout ("shadow-only" mode).
    ///
    /// Ported from the reference implementation in `../otto`
    /// (feat/window-scanout-new) and adapted to the plane pipeline: the app
    /// switcher, OSD and layer-shell panels render on the overlay plane, so
    /// they do not gate scanout globally — only windows they geometrically
    /// overlap are demoted (their pixels must be in the windows plane for the
    /// overlay's backdrop blur to sample).
    ///
    /// Selection is intentionally based on *stable* geometry (dock bar,
    /// switcher and OSD layer bounds, layer-shell rects), never on per-frame
    /// scene state like bubbled blur regions — those are cleared and rebuilt
    /// every engine update, so sampling them oscillates between promote and
    /// demote and flickers the window content.
    pub fn get_scanout_candidates(&self, output: &Output) -> Vec<ObjectId> {
        self.get_plane_candidates(output).raw
    }

    /// Both promotion tiers for `output`, computed in one top-to-bottom walk
    /// (they share every stability gate and the same occlusion state).
    pub fn get_plane_candidates(&self, output: &Output) -> PlaneCandidates {
        use smithay::utils::{Physical, Rectangle};

        // ---- global stable gates ----
        // Debug: `touch /tmp/otto-no-scanout` disables promotion entirely so
        // scanout-vs-composite cost can be A/B measured at runtime. The file
        // is polled at 1 Hz by the renderer; this is just an atomic read.
        if crate::render_elements::scene_dmabuf_element::NO_SCANOUT
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return PlaneCandidates::none();
        }
        if self.get_show_all() || self.is_expose_transitioning() {
            return PlaneCandidates::none();
        }
        if self.is_animating.load(std::sync::atomic::Ordering::Relaxed) {
            return PlaneCandidates::none();
        }
        // Candidates are strictly PER OUTPUT: a window may only be promoted
        // onto the CRTC of the output whose space contains it — promoting the
        // primary's topmost window on every CRTC painted it on all screens.
        let Some(ows) = self.output_workspaces.get(&output.name()) else {
            return PlaneCandidates::none();
        };
        let Some(current_workspace) = ows.workspace_views.get(ows.current_workspace) else {
            return PlaneCandidates::none();
        };
        // Fullscreen has its own dedicated direct-scanout path.
        if current_workspace.get_fullscreen_mode() || current_workspace.get_fullscreen_animating() {
            return PlaneCandidates::none();
        }
        // The tiling drop-zone overlay composites above windows.
        if self.tiling_overlay.is_visible() {
            return PlaneCandidates::none();
        }
        let is_primary = self
            .primary_output
            .as_ref()
            .map(|p| p.name() == output.name())
            .unwrap_or(false);
        let scale = output.current_scale().fractional_scale();

        // ---- occluders (physical px): overlay-plane UI compositing above
        // windows. A window overlapping any of these must stay in the windows
        // plane so the overlay's backdrop blur can sample its pixels.
        fn layer_rect(layer: &layers::prelude::Layer) -> Option<Rectangle<i32, Physical>> {
            let b = layer.render_bounds_with_children_transformed();
            if b.width() <= 0.0 || b.height() <= 0.0 {
                return None;
            }
            Some(Rectangle::new(
                (b.x() as i32, b.y() as i32).into(),
                (b.width() as i32, b.height() as i32).into(),
            ))
        }
        let mut occluders: Vec<Rectangle<i32, Physical>> = Vec::new();
        // Use the *visible* layers, not the wrap/positioning containers —
        // those can span the whole output and would demote every window.
        // Dock / OSD chrome is attached to the primary output only — it never
        // occludes windows on a secondary output.
        if is_primary {
            if !self.dock.is_hidden() {
                occluders.extend(layer_rect(&self.dock.bar_layer));
            }
            if self.osd.is_visible() {
                occluders.extend(layer_rect(&self.osd.view_layer));
            }
        }
        // The switcher, unlike the dock, follows the pointer across outputs.
        if self.app_switcher.is_visible() && self.is_app_switcher_output(output) {
            if let Some(layer) = self.app_switcher.view.layer.read().unwrap().as_ref() {
                occluders.extend(layer_rect(layer));
            }
        }
        {
            use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
            let map = smithay::desktop::layer_map_for_output(output);
            for l in map
                .layers()
                .filter(|l| matches!(l.layer(), WlrLayer::Top | WlrLayer::Overlay))
            {
                if let Some(geo) = map.layer_geometry(l) {
                    occluders.push(Rectangle::new(
                        geo.loc.to_f64().to_physical(scale).to_i32_round(),
                        geo.size.to_f64().to_physical(scale).to_i32_round(),
                    ));
                }
            }
        }

        // ---- per-window eligibility, top-to-bottom ----
        let Some(space) = ows.spaces.get(ows.current_workspace) else {
            return PlaneCandidates::none();
        };

        tracing::debug!(target: "otto::planes", "scanout occluders: {occluders:?}");
        let mut promoted = Vec::new();
        let mut subtree: Option<ObjectId> = None;
        // Union of visible-window rects seen so far (everything above current).
        let mut covered: Vec<Rectangle<i32, Physical>> = Vec::new();
        // Space::elements yields bottom-to-top; rev() => top-to-bottom.
        for window in space.elements().rev() {
            if window.is_minimised() {
                continue; // not visible — does not occlude
            }
            let view = self.get_window_view(&window.id());
            if view.as_ref().map(|v| v.is_unmapped()).unwrap_or(true) {
                continue; // gone — does not occlude
            }
            let Some(location) = space.element_location(window) else {
                continue;
            };
            // Space locations are global — rebase to output-local physical
            // px, the coordinate space of the scene-layer occluder rects.
            let local = location - output.current_location();
            let rect = Rectangle::new(
                local.to_f64().to_physical(scale).to_i32_round(),
                window
                    .geometry()
                    .size
                    .to_f64()
                    .to_physical(scale)
                    .to_i32_round(),
            );
            let overlaps_above = covered.iter().any(|c| c.overlaps(rect));
            // Occupies space for everything below it, promoted or not.
            covered.push(rect);

            // A minimizing window is still visible (animating to the dock) so
            // it occludes, but cannot be promoted (it has a live transform).
            let animating = view.as_ref().map(|v| v.is_minimizing()).unwrap_or(true);
            // v1 does not scan out popups; a window with a MAPPED popup must
            // composite normally or the (scene-drawn) popup would be hidden
            // under the window's overlay plane. Mapped, not merely alive —
            // GTK keeps closed popovers' surfaces around for reuse.
            let has_popups = window
                .wl_surface()
                .map(|s| surface_has_mapped_popup(&s))
                .unwrap_or(false);
            let overlaps_occluder = occluders.iter().any(|r| r.overlaps(rect));
            // Only dmabuf-backed buffers can scan out. An SHM client (e.g. a
            // CPU-rendered terminal) would take the whole promotion path just
            // to have its element GPU-composite anyway — and a composited
            // element in front demotes every plane below it (z-order), a net
            // loss over leaving the window in the windows plane.
            //
            // The headless test harness has no GPU, so its clients can only
            // attach SHM: requiring a dmabuf there would make every candidate
            // test vacuous. Under `headless` the buffer type is not part of
            // eligibility, leaving the occlusion/animation/popup/cap rules —
            // the logic those tests actually cover — under test.
            #[cfg(feature = "headless")]
            let has_dmabuf_buffer = true;
            #[cfg(not(feature = "headless"))]
            let has_dmabuf_buffer = window
                .wl_surface()
                .map(|s| {
                    smithay::wayland::compositor::with_states(&s, |states| {
                        states
                            .data_map
                            .get::<smithay::backend::renderer::utils::RendererSurfaceStateUserData>(
                            )
                            .map(|data| {
                                data.lock()
                                    .unwrap()
                                    .buffer()
                                    .map(|b| smithay::wayland::dmabuf::get_dmabuf(b).is_ok())
                                    .unwrap_or(false)
                            })
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            // Cap the promoted set: the hardware admits ~5 simultaneous
            // planes and bg/windows/dock/cursor already take four — a second
            // client plane evicts the windows plane (measured), which costs
            // more than compositing the extra window. Windows past the cap
            // still occlude the ones below.
            //
            // Windows with subsurfaces (SSD decorations) are eligible too:
            // only their ROOT surface goes to the plane, the decoration
            // subsurfaces keep rendering in the windows plane
            // (see `set_scanout_windows`).
            // A subsurface overlapping the root (window content, e.g. Chrome's
            // account panel) would be hidden behind the scanned-out root plane,
            // so such a window must composite. Checked last — it only runs for
            // an otherwise-eligible topmost candidate.
            //
            // A window the compositor draws something for — a background
            // colour, a backdrop blur, a rounded clip, a border, all asked for
            // through `otto-surface-style` — is NOT eligible here. The plane
            // carries the client's buffer and nothing else, and none of that
            // is in the buffer: the rounding in particular exists only as a
            // clip in the composite path, so scanning the buffer out raw
            // squares off the corners. Those windows fall through to the
            // subtree tier below, which renders the whole thing.
            const MAX_PROMOTED: usize = 1;
            // Gates both tiers share: the window must be the unobstructed
            // topmost one, holding still, with nothing composited over it.
            let stable = !overlaps_above && !animating && !has_popups && !overlaps_occluder;
            if promoted.len() < MAX_PROMOTED
                && stable
                && has_dmabuf_buffer
                && !window.has_material()
                && !window_has_overlapping_subsurface(window)
            {
                promoted.push(window.id());
                continue;
            }
            // Tier 2 — subtree plane. The compositor re-renders the window's
            // own lay-rs subtree into a dedicated buffer instead of handing
            // KMS the raw client buffer, so none of tier 1's buffer-shape
            // rules apply: the style the client asked for through
            // `otto-surface-style` (rounded corners, border, background,
            // blur), SSD decoration subsurfaces, overlapping subsurfaces and
            // SHM clients are all drawn into the plane exactly as they are
            // drawn when compositing. What it does NOT save is the per-frame
            // GPU pass — it buys damage isolation and a page flip, not zero
            // work — so it is strictly the fallback for a window tier 1
            // cannot take, never a replacement for it.
            //
            // Only one, and only when tier 1 took nothing: the two tiers
            // compete for the same scarce overlay plane, and tier 1 is the
            // cheaper occupant.
            if promoted.is_empty()
                && subtree.is_none()
                && stable
                && !crate::render_elements::scene_dmabuf_element::NO_WINDOW_PLANE
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                subtree = Some(window.id());
            }
        }
        if !promoted.is_empty() {
            subtree = None;
        }
        PlaneCandidates {
            raw: promoted,
            subtree,
        }
    }

    /// Whether any overlay-UI chrome is visible on `output` — used by the
    /// backend to decide if the overlay plane deserves a hardware plane
    /// slot this frame. Overlay chrome is on demand: an empty full-screen
    /// ARGB buffer must not waste a plane. Covers layer-shell Top/Overlay
    /// surfaces, popups, the workspace selector (expose state), the OSD and
    /// the tiling overlay. DnD is NOT included — the drag icon lives on
    /// `Otto`, so the caller ORs it in.
    pub fn is_overlay_ui_active(&self, output: &Output) -> bool {
        use smithay::desktop::layer_map_for_output;
        use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
        let has_chrome_layer = |o: &Output| {
            layer_map_for_output(o)
                .layers()
                .any(|l| matches!(l.layer(), WlrLayer::Top | WlrLayer::Overlay))
        };
        // `layer_shell_top`/`layer_shell_overlay` are attached to the PRIMARY
        // output's overlay plane no matter which output a surface was mapped
        // to, so the primary must count surfaces on every output — otherwise a
        // layer surface assigned elsewhere renders into a plane that is never
        // pushed and stays invisible.
        let layer_shell_active = if self.primary_output().is_some_and(|p| p == output) {
            self.outputs().any(has_chrome_layer)
        } else {
            has_chrome_layer(output)
        };
        let popups = !self.popup_overlay.layer.children().is_empty();
        // Selector and DnD layers are never `hidden()` — they are
        // empty containers until used, so check content/state instead.
        let selector = self.is_expose_transitioning()
            || self.get_show_all()
            || self.is_animating.load(std::sync::atomic::Ordering::Relaxed);
        let osd = self.osd.is_visible();
        let tiling = self.tiling_overlay.is_visible();
        let active = layer_shell_active || popups || selector || osd || tiling;
        if active {
            tracing::debug!(
                target: "otto::planes",
                "overlay active: shell={layer_shell_active} popups={popups} selector={selector} osd={osd} tiling={tiling}",
            );
        }
        active
    }

    /// Windows on the current workspace fully contained in a single
    /// non-minimized window stacked above them. Feeds the frame-callback
    /// throttle classifier's Occluded bucket. Union coverage is deliberately
    /// not attempted: the single-cover case (a maximized or large window over
    /// smaller ones) is the one that matters, and containment in one window
    /// cannot false-positive on a partially visible window. Windows in
    /// `translucent` (blur-effect clients, see
    /// `window_throttle::translucent_window_ids`) can be occluded but never
    /// occlude: what is behind them shows through.
    #[allow(clippy::mutable_key_type)]
    pub fn occluded_window_ids(&self, translucent: &HashSet<ObjectId>) -> HashSet<ObjectId> {
        use smithay::utils::{Physical, Rectangle};
        let mut occluded = HashSet::new();
        let scale = Config::with(|c| c.screen_scale);
        let current_index = self.get_current_workspace_index();
        let Some(space) = self
            .primary_output_workspaces()
            .and_then(|ows| ows.spaces.get(current_index))
        else {
            return occluded;
        };
        let mut above: Vec<Rectangle<i32, Physical>> = Vec::new();
        // Space::elements yields bottom-to-top; rev() => top-to-bottom.
        for window in space.elements().rev() {
            if window.is_minimised() {
                continue;
            }
            let Some(location) = space.element_location(window) else {
                continue;
            };
            let rect = Rectangle::new(
                location.to_f64().to_physical(scale).to_i32_round(),
                window
                    .geometry()
                    .size
                    .to_f64()
                    .to_physical(scale)
                    .to_i32_round(),
            );
            if above.iter().any(|a| a.contains_rect(rect)) {
                occluded.insert(window.id());
            }
            if !translucent.contains(&window.id()) {
                above.push(rect);
            }
        }
        occluded
    }

    /// Snapshot of the windows currently flagged for scanout.
    #[allow(clippy::mutable_key_type)]
    pub fn scanout_window_ids(&self) -> HashSet<ObjectId> {
        self.scanout_windows.read().unwrap().clone()
    }

    /// The promoted (direct-scanout) window set of a single output.
    #[allow(clippy::mutable_key_type)]
    pub fn scanout_window_ids_for_output(&self, output_name: &str) -> HashSet<ObjectId> {
        self.scanout_windows_per_output
            .read()
            .unwrap()
            .get(output_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Replace the scanout window set: hides `content_layer` for new entrants,
    /// unhides it for departures. Idempotent. The caller must re-import any
    /// departing window's buffer (via `update_window_view`) *after* this call
    /// so the unhidden `content_layer` shows the current frame, not a stale one.
    #[allow(clippy::mutable_key_type)]
    /// Update one output's desired scanout set and apply the union of all
    /// outputs' sets. Each CRTC computes its own candidates; applying them
    /// directly to the global set made two outputs demote each other's
    /// promoted windows every frame.
    pub fn set_scanout_windows_for_output(&self, output_name: &str, ids: &[ObjectId]) {
        let union: HashSet<ObjectId> = {
            let mut per_output = self.scanout_windows_per_output.write().unwrap();
            let entry = per_output.entry(output_name.to_string()).or_default();
            let new_set: HashSet<ObjectId> = ids.iter().cloned().collect();
            if *entry == new_set {
                return;
            }
            *entry = new_set;
            per_output.values().flatten().cloned().collect()
        };
        let union_vec: Vec<ObjectId> = union.into_iter().collect();
        self.set_scanout_windows(&union_vec);
    }

    /// Promote (or demote) one window to its own KMS plane on `output_name`.
    ///
    /// Promotion reparents the window's whole `window_layer` — shadow, client
    /// content, SSD decorations and the compositor-drawn style on it — out of
    /// its workspace's `windows_layer` and into the output's `promoted_plane`
    /// container, which the backend renders into a dedicated buffer. The
    /// windows plane stops drawing the window purely because it is no longer
    /// part of that subtree, so there is no hidden/blank state to keep in sync
    /// and nothing to re-import on demotion: the same layers keep rendering,
    /// in a different buffer.
    ///
    /// The container mirrors the current workspace's `windows_layer` geometry,
    /// so the move is geometrically a no-op. Re-applied every frame (cheap
    /// when unchanged) because other code paths — `raise_window_to_front`
    /// above all — reparent window layers back under `windows_layer` without
    /// knowing about promotion.
    ///
    /// Returns whether the promoted window changed, so the caller can force a
    /// full redraw of both affected planes on the transition.
    pub fn set_promoted_window(&self, output_name: &str, id: Option<&ObjectId>) -> bool {
        let Some(ows) = self.output_workspaces.get(output_name) else {
            return false;
        };
        let prev = self
            .promoted_windows
            .read()
            .unwrap()
            .get(output_name)
            .cloned();
        let changed = prev.as_ref() != id;

        // Demote first: a window leaving the plane must be back under its
        // workspace before the new one is pulled out, or a swap between two
        // windows would briefly have both in the container.
        if let Some(prev_id) = prev.as_ref().filter(|p| Some(*p) != id) {
            self.demote_window_layer(ows, prev_id);
        }

        match id {
            Some(id) => {
                let Some(workspace) = ows.workspace_views.get(ows.current_workspace) else {
                    return changed;
                };
                // Keep the container aligned with the workspace it is standing
                // in for: `windows_layer` is offset by the workspace's index
                // inside `windows_plane`, and both scroll together.
                let wl = &workspace.windows_layer;
                let pos = wl.position();
                if ows.promoted_plane.position() != pos {
                    ows.promoted_plane.set_position(pos, None);
                }
                ows.promoted_plane.set_size(wl.size(), None);
                if let Some(view) = self.get_window_view(id) {
                    // lay-rs has no parent lookup; the container holds at most
                    // one window, so checking its children is equivalent.
                    let already_here = ows
                        .promoted_plane
                        .children_nodes()
                        .contains(&view.window_layer.id());
                    if !already_here {
                        if let Err(e) = ows.promoted_plane.add_sublayer(&view.window_layer) {
                            tracing::warn!(
                                target: "otto::planes",
                                "promote: reparent into promoted_plane failed: {e}"
                            );
                            return changed;
                        }
                    }
                }
                self.promoted_windows
                    .write()
                    .unwrap()
                    .insert(output_name.to_string(), id.clone());
            }
            None => {
                // No `set_hidden` either way: an empty container draws
                // nothing, and a hidden flag reaches the scene arena a frame
                // late — long enough for the window to blink out of both
                // planes on the promotion edge.
                self.promoted_windows.write().unwrap().remove(output_name);
            }
        }
        if changed {
            tracing::info!(
                target: "otto::planes",
                "subtree plane on {output_name}: {prev:?} -> {id:?}"
            );
        }
        changed
    }

    /// Put a promoted window's layer back under its workspace's windows
    /// container, restoring the normal z-order among its siblings.
    fn demote_window_layer(&self, ows: &OutputWorkspaces, id: &ObjectId) {
        let Some(view) = self.get_window_view(id) else {
            return;
        };
        let Some(workspace) = ows.workspace_views.get(ows.current_workspace) else {
            return;
        };
        if let Err(e) = workspace.windows_layer.add_sublayer(&view.window_layer) {
            tracing::warn!(
                target: "otto::planes",
                "demote: reparent back into windows_layer failed: {e}"
            );
            return;
        }
        // Reparenting appends, which is a raise: without this the window comes
        // back on top of everything mapped while it was promoted, and stays
        // there. The workspace's own list is the stack.
        workspace.restack_windows();
    }

    /// The window currently rendered into its own plane on `output_name`.
    pub fn promoted_window_for_output(&self, output_name: &str) -> Option<ObjectId> {
        self.promoted_windows
            .read()
            .unwrap()
            .get(output_name)
            .cloned()
    }

    /// Output-local bounds (physical px) of the promoted window's subtree,
    /// shadow included — the buffer size and plane position the backend needs.
    /// `None` when nothing is promoted, or the window has no view yet.
    pub fn promoted_plane_bounds(
        &self,
        output_name: &str,
    ) -> Option<(
        smithay::utils::Point<i32, smithay::utils::Physical>,
        (i32, i32),
    )> {
        let id = self.promoted_window_for_output(output_name)?;
        let ows = self.output_workspaces.get(output_name)?;
        let view = self.get_window_view(&id)?;
        // The shadow layer is the widest node in the subtree (it extends a
        // fixed safe area past the window on every side), so the subtree's
        // own bounds are what the buffer has to cover.
        let bounds = view.window_layer.render_bounds_with_children_transformed();
        let origin = ows.output_layer.render_position();
        let w = bounds.width().ceil() as i32;
        let h = bounds.height().ceil() as i32;
        if w <= 0 || h <= 0 {
            return None;
        }
        Some((
            (
                (bounds.x() - origin.x).floor() as i32,
                (bounds.y() - origin.y).floor() as i32,
            )
                .into(),
            (w, h),
        ))
    }

    /// Remove one window from every output's scanout set (pre-animation
    /// demotion) and apply the new union.
    pub fn remove_scanout_window(&self, id: &ObjectId) {
        #[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
        let union: HashSet<ObjectId> = {
            let mut per_output = self.scanout_windows_per_output.write().unwrap();
            for set in per_output.values_mut() {
                set.remove(id);
            }
            per_output.values().flatten().cloned().collect()
        };
        let union_vec: Vec<ObjectId> = union.into_iter().collect();
        self.set_scanout_windows(&union_vec);
    }

    fn set_scanout_windows(&self, ids: &[ObjectId]) {
        #[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
        let new_ids: HashSet<ObjectId> = ids.iter().cloned().collect();
        #[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
        let prev_ids = self.scanout_windows.read().unwrap().clone();
        if prev_ids == new_ids {
            return;
        }
        tracing::info!(
            target: "otto::planes",
            "scanout window set changed: {:?} -> {:?}",
            prev_ids,
            new_ids
        );
        for prev in prev_ids.difference(&new_ids) {
            if let Some(view) = self.get_window_view(prev) {
                view.set_content_hidden(false);
            }
            if let Some(window) = self.get_window_for_surface(prev) {
                window.set_scanned_out(false);
            }
        }
        for new in new_ids.difference(&prev_ids) {
            let window = self.get_window_for_surface(new);
            // Two reasons to blank the surface layer's texture rather than
            // hide the whole subtree. An SSD window keeps its decoration
            // subsurfaces rendering; a window with a material — a background
            // colour or a `BackgroundBlur` it asked the compositor for through
            // `otto-surface-style` — keeps those pixels rendering. Both live on
            // layers the plane cannot carry: the KMS plane holds the client
            // buffer and nothing else. Left in the windows plane they land
            // under the scanout plane, which is where they belong, and the
            // blur seeds its backdrop from the planes below exactly as it does
            // for an unpromoted window.
            let blank_texture_only = window.map(window_has_subsurfaces).unwrap_or(false)
                || window.map(|w| w.has_material()).unwrap_or(false);
            if let Some(view) = self.get_window_view(new) {
                if blank_texture_only {
                    // Blank just the root surface layer's draw content, so
                    // everything else on and under that layer keeps rendering
                    // in the windows plane: an SSD window's decoration
                    // subsurfaces (its children: titlebar, buttons, borders),
                    // and the layer's own material. Hiding the root layer
                    // instead would take both with it. The no-op closure
                    // reports full-bounds damage so the old pixels clear;
                    // the demotion re-import (`update_window_view` →
                    // `configure_surface_layer`) reinstalls the real draw
                    // closure and the opaque flag.
                    for root in view.content_layer.children() {
                        root.set_content_opaque(false);
                        root.set_draw_content(|_: &layers::skia::Canvas, w: f32, h: f32| {
                            layers::skia::Rect::from_wh(w, h)
                        });
                    }
                    // The draw closure was replaced behind
                    // `configure_surface_layer`'s back, so its idempotence key
                    // no longer describes the layer — drop it, or the demotion
                    // re-import would match the stale key and leave the window
                    // showing this blank closure forever.
                    crate::surface_config_cache::invalidate(new);
                } else {
                    view.set_content_hidden(true);
                }
            }
            if let Some(window) = self.get_window_for_surface(new) {
                window.set_scanned_out(true);
            }
        }
        *self.scanout_windows.write().unwrap() = new_ids;
    }

    pub fn get_current_workspace(&self) -> Option<Arc<WorkspaceView>> {
        // Go directly to the authoritative per-output data first
        let focused_name = self.with_model(|m| m.focused_output_name.clone());
        let ows = focused_name
            .as_deref()
            .and_then(|n| self.output_workspaces.get(n))
            .or_else(|| self.primary_output_workspaces());
        if let Some(ows) = ows {
            return ows.workspace_views.get(ows.current_workspace).cloned();
        }
        // Fallback: model cache (populated after first map_output)
        self.with_model(|m| m.workspaces.get(m.current_workspace).cloned())
    }

    pub fn get_current_workspace_index(&self) -> usize {
        self.with_model(|m| m.current_workspace)
    }

    /// Get the top (non-minimized) window of a workspace, or None if the workspace is empty.
    pub fn get_top_window_of_workspace(&self, workspace_index: usize) -> Option<ObjectId> {
        let pows = self.primary_output_workspaces()?;
        if workspace_index >= pows.spaces.len() {
            return None;
        }
        pows.spaces[workspace_index].elements().rev().find_map(|e| {
            let id = e.id();
            if let Some(window) = self.windows_map.get(&id) {
                if window.is_minimised() {
                    return None;
                }
            }
            Some(id)
        })
    }

    /// Top (non-minimized) window of an output's CURRENT workspace.
    pub fn get_top_window_of_workspace_on_output(&self, output: &Output) -> Option<ObjectId> {
        let ows = self.output_workspaces.get(&output.name())?;
        let space = ows.spaces.get(ows.current_workspace)?;
        space.elements().rev().find_map(|e| {
            let id = e.id();
            if let Some(window) = self.windows_map.get(&id) {
                if window.is_minimised() {
                    return None;
                }
            }
            Some(id)
        })
    }

    /// Apply the current expose selector order back to the real workspace stacking.
    ///
    /// This is intended to be called when leaving expose mode: while expose is open,
    /// only mirror previews are reordered; on close, the same order is committed to
    /// the actual Space/layers/windows_list ordering.
    pub fn apply_window_selector_order_to_workspace(&mut self, workspace_index: usize) {
        let Some(workspace) = self.get_workspace_at(workspace_index) else {
            return;
        };

        // Restore the stacking order that was saved when expose opened.
        // This avoids replacing the user's z-order with the sorted layout order.
        let saved = workspace.take_pre_expose_order();
        let order = if saved.is_empty() {
            // Fallback: use expose rects order (should not normally happen).
            let state = workspace.window_selector_view.view.get_state().clone();
            state
                .rects
                .iter()
                .filter_map(|rect| rect.window_id.clone())
                .collect()
        } else {
            saved
        };

        for window_id in order {
            self.raise_element(&window_id, false, false);
        }

        self.update_workspace_model();
    }

    /// Given a workspace view index (WorkspaceView.index), return its current
    /// position in the workspaces vector (zero-based). Useful when external
    /// components keep the view index while the internal ordering may change.
    pub fn workspace_position_by_view_index(&self, workspace_index: usize) -> Option<usize> {
        self.with_model(|m| {
            m.workspaces
                .iter()
                .position(|ws| ws.index == workspace_index)
        })
    }
    /// Switch the workspace for a specific output independently.
    pub fn set_workspace_for_output(
        &mut self,
        output: &Output,
        i: usize,
        transition: Option<Transition>,
    ) -> Option<TransactionRef> {
        let name = output.name();
        let valid = self
            .output_workspaces
            .get(&name)
            .map(|ows| i < ows.spaces.len())
            .unwrap_or(false);
        if !valid {
            return None;
        }
        let scale = output.current_scale().fractional_scale() as f32;
        // Get workspace width for this output — use physical mode size (same as update_layout)
        let workspace_width = self
            .outputs
            .iter()
            .find(|o| o.name() == name)
            .and_then(|o| o.current_mode())
            .map(|m| m.size.w as f32)
            .unwrap_or_else(|| self.with_model(|m| m.width as f32));

        if let Some(ows) = self.output_workspaces.get_mut(&name) {
            ows.current_workspace = i;
        }
        self.sync_model_from_primary();
        self.update_workspace_model();

        // Dock and layer-shell chrome live on the primary output only — drive
        // them from the TARGET OUTPUT's own workspace view, and only when
        // switching the primary. A fullscreen workspace on a secondary output
        // must not hide the primary's dock or fade its top bar.
        let is_primary = self.primary_output_name().as_deref() == Some(name.as_str());
        let target_view = self
            .output_workspaces
            .get(&name)
            .and_then(|ows| ows.workspace_views.get(i).cloned());
        let resolved_transition = transition
            .clone()
            .unwrap_or_else(workspace_switch_transition);
        // Expose hides the dock and the layer-shell chrome itself, and restores
        // them when it closes. Switching workspace underneath an open (or
        // animating) expose must not fade them back in.
        let expose_active = self.get_show_all() || self.is_expose_transitioning();
        if is_primary && !expose_active {
            if let Some(workspace) = &target_view {
                if workspace.get_fullscreen_mode() {
                    self.dock.hide(Some(resolved_transition.clone()));
                } else if !self.dock.is_autohide_enabled() {
                    self.dock.show(Some(resolved_transition.clone()));
                }
            }

            // Animate layer_shell_top opacity based on target workspace fullscreen state
            if let Some(workspace) = &target_view {
                let is_fullscreen = workspace.get_fullscreen_mode();
                let target_opacity = if is_fullscreen { 0.0_f32 } else { 1.0_f32 };
                if !is_fullscreen {
                    self.layer_shell_top.set_hidden(false);
                }
                self.layer_shell_overlay
                    .set_opacity(target_opacity, Some(resolved_transition.clone()));
                let layer_shell_top_ref = self.layer_shell_top.clone();
                self.layer_shell_top
                    .set_opacity(target_opacity, Some(resolved_transition))
                    .on_finish(
                        move |_: &Layer, _| {
                            if is_fullscreen {
                                layer_shell_top_ref.set_hidden(true);
                            }
                        },
                        true,
                    );
            }
        }

        // Scroll only this output's layer
        let workspace_gap_px = WORKSPACE_SPACING * scale;
        let offset = i as f32 * (workspace_width + workspace_gap_px);
        let transition = transition.unwrap_or_else(workspace_switch_transition);
        self.apply_scroll_offset_filtered(offset, Some(transition), Some(&name.clone()))
    }

    /// Switch workspace index for all outputs simultaneously (compat shim).
    pub fn set_current_workspace_index(
        &mut self,
        i: usize,
        transition: Option<Transition>,
    ) -> Option<TransactionRef> {
        let valid = self
            .primary_output_workspaces()
            .map(|ows| i <= ows.spaces.len().saturating_sub(1))
            .unwrap_or(false);
        if !valid {
            return None;
        }
        for ows in self.output_workspaces.values_mut() {
            if i < ows.spaces.len() {
                ows.current_workspace = i;
            }
        }
        self.sync_model_from_primary();
        self.update_workspace_model();
        self.scroll_to_workspace_index(i, transition)
    }
    /// Scroll to the workspace at index i, default transition is 1.0s spring
    fn scroll_to_workspace_index(
        &self,
        i: usize,
        transition: Option<Transition>,
    ) -> Option<TransactionRef> {
        let transition = transition.unwrap_or_else(workspace_switch_transition);
        let x = 0.0_f32;
        if let Some(workspace) = self.get_workspace_at(i) {
            // Control dock visibility based on workspace fullscreen state
            // Only skip dock control when actively IN expose mode (show_all)

            // Don't use hide/show during:
            // - expose mode or expose transitions (let expose system control position)
            // - fullscreen animations (let fullscreen transition complete smoothly)
            let expose_active = self.get_show_all() || self.is_expose_transitioning();
            if !expose_active {
                if workspace.get_fullscreen_mode() {
                    self.dock.hide(Some(transition.clone()));
                } else if !self.dock.is_autohide_enabled() {
                    // With autohide on, dock visibility is managed by the hot zone /
                    // show_autohide(); calling show() would hide it (see DockView::show).
                    self.dock.show(Some(transition.clone()));
                }

                // Animate layer_shell_overlay and layer_shell_top based on target workspace fullscreen state
                let is_fullscreen = workspace.get_fullscreen_mode();
                let target_opacity = if is_fullscreen { 0.0_f32 } else { 1.0_f32 };
                if !is_fullscreen {
                    self.layer_shell_top.set_hidden(false);
                }
                self.layer_shell_overlay
                    .set_opacity(target_opacity, Some(transition.clone()));
                let layer_shell_top_ref = self.layer_shell_top.clone();
                self.layer_shell_top
                    .set_opacity(target_opacity, Some(transition.clone()))
                    .on_finish(
                        move |_: &Layer, _| {
                            if is_fullscreen {
                                layer_shell_top_ref.set_hidden(true);
                            }
                        },
                        true,
                    );
            }

            if self.get_show_all() {
                // In expose mode, ensure the target workspace has its layout calculated
                // and windows positioned, but don't animate dock (it's shared across workspaces)
                self.expose_update_if_needed_workspace(i);
            }

            let _ = x; // per-output offsets computed below
        }

        // Compute per-output offset: each output slides by i * (own_phys_width + SPACING)
        let primary_width = self.with_model(|m| m.width as f32);
        let mut changes = Vec::new();
        for (output_name, ows) in self.output_workspaces.iter() {
            let output = self.outputs.iter().find(|o| o.name() == *output_name);
            let output_width = output
                .and_then(|o| o.current_mode())
                .map(|m| m.size.w as f32)
                .unwrap_or(primary_width);
            let scale = output
                .map(|o| o.current_scale().fractional_scale() as f32)
                .unwrap_or(1.0);
            let w = output_width;

            let workspace_gap_px = WORKSPACE_SPACING * scale;
            let offset = i as f32 * (w + workspace_gap_px);
            // Skip outputs already resting at the target offset. Activation
            // paths re-request the CURRENT workspace on every focus change;
            // scheduling the no-op spring anyway pulses `is_animating` for a
            // frame, which demotes/re-promotes the scanout window and resets
            // the compositor swapchain — a visible full-screen flicker per
            // click (and per hover that re-activates).
            if (ows.workspaces_layer.render_position().x - (-offset)).abs() > 0.5 {
                changes.push(ows.workspaces_layer.change_position((-offset, 0.0)));
            }
        }
        if changes.is_empty() {
            return None;
        }
        tracing::debug!(target: "otto::popups", "is_animating(true) site=scroll-workspace i={i}");
        self.is_animating
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let animation = self
            .layers_engine
            .add_animation_from_transition(&transition, true);
        let tr = self
            .layers_engine
            .schedule_changes(&changes, animation)
            .into_iter()
            .next();
        if let Some(tr) = &tr {
            let is_animating = self.is_animating.clone();
            tr.on_finish(
                move |_: &Layer, _: f32| {
                    is_animating.store(false, std::sync::atomic::Ordering::Relaxed);
                },
                true,
            );
        } else {
            // No transaction was scheduled (e.g. no outputs yet), so no
            // on_finish will ever fire — clear the flag here or it wedges
            // `true` forever, permanently blocking scanout promotion and
            // misreporting expose transitions.
            self.is_animating
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        tr
    }

    /// Update workspace position during 3-finger horizontal swipe gesture.
    /// Applies delta immediately (no animation) with rubber-band resistance at edges.
    pub fn workspace_swipe_update(&self, output_name: &str, delta_x: f32) {
        let ows = match self.output_workspaces.get(output_name) {
            Some(o) => o,
            None => return,
        };
        let num_workspaces = ows.workspace_views.len();
        let output = self.outputs.iter().find(|o| o.name() == output_name);
        let scale = output
            .map(|o| o.current_scale().fractional_scale() as f32)
            .unwrap_or_else(|| self.with_model(|m| m.scale as f32));

        // Get workspace width for this output (physical pixels, matching how positions are set)
        let workspace_width = output
            .and_then(|o| o.current_mode())
            .map(|m| m.size.w as f32)
            .unwrap_or_else(|| self.with_model(|m| m.width as f32));

        if num_workspaces == 0 || workspace_width <= 0.0 {
            return;
        }

        // Get current scroll position from the active layer.
        // In expose/transition states, expose_layer is authoritative; otherwise workspaces_layer is.
        let current_pos = if self.get_show_all() || self.is_expose_transitioning() {
            ows.expose_layer.render_position()
        } else {
            ows.workspaces_layer.render_position()
        };
        let current_offset = -current_pos.x;

        const SWIPE_DAMPENING: f32 = 0.6;
        let physical_delta = delta_x * scale * SWIPE_DAMPENING;
        let workspace_gap_px = WORKSPACE_SPACING * scale;
        let max_offset = (num_workspaces - 1) as f32 * (workspace_width + workspace_gap_px);

        let new_offset = if current_offset < 0.0 {
            let resistance_factor = 1.0 / (1.0 + (-current_offset) / 100.0);
            current_offset - (physical_delta * resistance_factor)
        } else if current_offset > max_offset {
            let resistance_factor = 1.0 / (1.0 + (current_offset - max_offset) / 100.0);
            current_offset - (physical_delta * resistance_factor)
        } else {
            current_offset - physical_delta
        };

        ows.workspaces_layer.set_position((-new_offset, 0.0), None);

        // Interpolate layer_shell_top opacity during swipe based on fullscreen state
        // of the two workspaces we're swiping between. Expose owns that opacity
        // while it is open (or animating) — driving it from here would bring the
        // top bar back on screen mid-expose.
        let step = workspace_width + workspace_gap_px;
        let expose_active = self.get_show_all() || self.is_expose_transitioning();
        if step > 0.0 && !expose_active {
            let progress = new_offset / step;
            let left_index = (progress.floor() as usize).min(num_workspaces.saturating_sub(1));
            let right_index = (left_index + 1).min(num_workspaces.saturating_sub(1));
            let t = (progress - left_index as f32).clamp(0.0, 1.0);

            let left_fs = ows
                .workspace_views
                .get(left_index)
                .map(|ws| ws.get_fullscreen_mode())
                .unwrap_or(false);
            let right_fs = ows
                .workspace_views
                .get(right_index)
                .map(|ws| ws.get_fullscreen_mode())
                .unwrap_or(false);

            let left_opacity: f32 = if left_fs { 0.0 } else { 1.0 };
            let right_opacity: f32 = if right_fs { 0.0 } else { 1.0 };
            let opacity = left_opacity.interpolate(&right_opacity, t);

            self.layer_shell_top.set_opacity(opacity, None);
            self.layer_shell_top.set_hidden(opacity == 0.0);
            self.layer_shell_overlay.set_opacity(opacity, None);
        }
    }

    /// End workspace swipe gesture and snap to nearest workspace.
    /// Uses velocity to determine target workspace for natural momentum-based snapping.
    /// Returns the target workspace index.
    pub fn workspace_swipe_end(&mut self, output_name: &str, velocity: f32) -> usize {
        let output = self
            .outputs
            .iter()
            .find(|o| o.name() == output_name)
            .cloned();
        let scale = output
            .as_ref()
            .map(|o| o.current_scale().fractional_scale() as f32)
            .unwrap_or_else(|| self.with_model(|m| m.scale as f32));

        let (num_workspaces, workspace_width, current_index) = {
            let ows = match self.output_workspaces.get(output_name) {
                Some(o) => o,
                None => {
                    let idx = self.with_model(|m| m.current_workspace);
                    let _ = self.set_current_workspace_index(idx, None);
                    return idx;
                }
            };
            let w = self
                .outputs
                .iter()
                .find(|o| o.name() == output_name)
                .and_then(|o| o.current_mode())
                .map(|m| m.size.w as f32)
                .unwrap_or_else(|| self.with_model(|m| m.width as f32));
            (ows.workspace_views.len(), w, ows.current_workspace)
        };

        if num_workspaces == 0 || workspace_width <= 0.0 {
            let _ = self.set_current_workspace_index(current_index, None);
            return current_index;
        }

        let current_pos = self
            .output_workspaces
            .get(output_name)
            .map(|ows| {
                if self.get_show_all() || self.is_expose_transitioning() {
                    ows.expose_layer.render_position()
                } else {
                    ows.workspaces_layer.render_position()
                }
            })
            .unwrap_or_default();
        let current_offset = -current_pos.x;

        let physical_velocity = velocity * scale;
        const VELOCITY_THRESHOLD: f32 = 15.0;

        let workspace_gap_px = WORKSPACE_SPACING * scale;
        let progress = current_offset / (workspace_width + workspace_gap_px);

        let target_index = if physical_velocity.abs() > VELOCITY_THRESHOLD {
            if physical_velocity > 0.0 {
                current_index.saturating_sub(1)
            } else {
                (current_index + 1).min(num_workspaces - 1)
            }
        } else {
            (progress.round() as usize).min(num_workspaces - 1)
        };

        let transition = Transition {
            delay: 0.0,
            timing: TimingFunction::Spring(Spring::with_duration_and_bounce(0.5, 0.05)),
        };

        if let Some(output) = output {
            let _ = self.set_workspace_for_output(&output, target_index, Some(transition));
        } else {
            let _ = self.set_current_workspace_index(target_index, Some(transition));
        }
        target_index
    }

    // Space management

    pub fn outputs_for_element(&self, element: &WindowElement) -> Vec<Output> {
        // Windows live in their owning output's space, not necessarily the
        // primary's — search every output's spaces.
        //
        // An *interactive* virtual output hosts windows exactly like a
        // physical screen, so it must be reported here: callers use this to
        // answer "which output is this window on?" (maximize, tiling,
        // fractional scale, screenshare). Filtering every virtual output out
        // made a window maximized on a virtual output fall back to the
        // primary physical screen and jump there. Non-interactive virtual
        // outputs stay excluded — nothing is ever placed on them.
        let mut outputs: Vec<Output> = self
            .output_workspaces
            .values()
            .flat_map(|ows| ows.spaces.iter())
            .flat_map(|s| s.outputs_for_element(element))
            .filter(|o| !crate::virtual_output::is_unreachable_virtual_output(o))
            .collect();
        // A window overlapping both kinds belongs to the physical one:
        // callers take `.first()` as "the window's output".
        outputs.sort_by_key(crate::virtual_output::is_virtual_output);
        outputs.dedup_by_key(|o| o.name());
        outputs
    }
    #[allow(dead_code)]
    fn apply_scroll_offset(
        &self,
        offset: f32,
        transition: Option<Transition>,
    ) -> Option<TransactionRef> {
        self.apply_scroll_offset_filtered(offset, transition, None)
    }

    /// Scroll only the specified output's workspaces_layer (or all if None).
    fn apply_scroll_offset_filtered(
        &self,
        offset: f32,
        transition: Option<Transition>,
        output_name: Option<&str>,
    ) -> Option<TransactionRef> {
        if !offset.is_finite() {
            return None;
        }
        if let Some(transition) = &transition {
            // Mark as animating
            tracing::debug!(target: "otto::popups", "is_animating(true) site=apply-scroll offset={offset}");
            self.is_animating
                .store(true, std::sync::atomic::Ordering::Relaxed);

            let animation = self
                .layers_engine
                .add_animation_from_transition(transition, true);
            let mut changes = Vec::new();
            for (name, ows) in self.output_workspaces.iter() {
                if output_name.is_none() || output_name == Some(name.as_str()) {
                    changes.push(ows.workspaces_layer.change_position((-offset, 0.0)));
                }
            }
            let tr = self
                .layers_engine
                .schedule_changes(&changes, animation)
                .into_iter()
                .next();

            // Clear animating flag when animation completes
            if let Some(tr) = &tr {
                let is_animating = self.is_animating.clone();
                tr.on_finish(
                    move |_: &Layer, _: f32| {
                        is_animating.store(false, std::sync::atomic::Ordering::Relaxed);
                    },
                    true,
                );
            } else {
                // See scroll variant above: without a transaction the flag
                // would wedge `true` forever.
                self.is_animating
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }

            return tr;
        }
        None
    }

    pub fn element_under(
        &self,
        point: impl Into<smithay::utils::Point<f64, smithay::utils::Logical>>,
    ) -> Option<(
        &WindowElement,
        smithay::utils::Point<i32, smithay::utils::Logical>,
    )> {
        // Windows live in their owning output's space — search every
        // output's CURRENT workspace (spaces are disjoint in global coords,
        // so at most one output matches the point).
        let point = point.into();
        self.output_workspaces
            .values()
            .find_map(|ows| ows.spaces.get(ows.current_workspace)?.element_under(point))
    }

    pub fn output_geometry(
        &self,
        output: &Output,
    ) -> Option<smithay::utils::Rectangle<i32, smithay::utils::Logical>> {
        // Search all per-output spaces, not just the primary one
        for ows in self.output_workspaces.values() {
            for space in &ows.spaces {
                if let Some(geo) = space.output_geometry(output) {
                    return Some(geo);
                }
            }
        }
        None
    }

    pub fn refresh_space(&mut self) {
        // Refresh EVERY output's spaces — `Space::refresh` drives the
        // wl_surface enter/leave bookkeeping that per-output frame-callback
        // delivery depends on. Refreshing only the primary left clients on
        // secondary outputs without frame callbacks (frozen content).
        for ows in self.output_workspaces.values_mut() {
            for space in ows.spaces.iter_mut() {
                space.refresh();
            }
        }
    }

    pub fn element_location(
        &self,
        we: &WindowElement,
    ) -> Option<smithay::utils::Point<i32, smithay::utils::Logical>> {
        // The window lives in exactly one output's space — search them all.
        self.output_workspaces
            .values()
            .find_map(|ows| ows.spaces.iter().find_map(|s| s.element_location(we)))
    }

    pub fn output_under<P: Into<smithay::utils::Point<f64, smithay::utils::Logical>>>(
        &self,
        point: P,
    ) -> impl Iterator<Item = &Output> {
        let point = point.into();
        // Non-interactive virtual outputs are pointer-unreachable by
        // contract: never focusable, never a window-placement target.
        self.outputs
            .iter()
            .filter(|o| !crate::virtual_output::is_unreachable_virtual_output(o))
            .filter(move |o| {
                if let Some(ows) = self.output_workspaces.get(&o.name()) {
                    ows.spaces
                        .first()
                        .map(|s| s.output_under(point).any(|_| true))
                        .unwrap_or(false)
                } else {
                    false
                }
            })
    }

    pub fn element_geometry(
        &self,
        we: &WindowElement,
    ) -> Option<smithay::utils::Rectangle<i32, smithay::utils::Logical>> {
        // The window lives in exactly one output's space — search them all.
        self.output_workspaces
            .values()
            .find_map(|ows| ows.spaces.iter().find_map(|s| s.element_geometry(we)))
    }

    // Add these helper methods
    #[allow(dead_code)]
    fn find_space_for_element(&self, element: &WindowElement) -> Option<&Space<WindowElement>> {
        self.primary_output_workspaces()?
            .spaces
            .iter()
            .find(|space| space.elements().any(|e| e.id() == element.id()))
    }

    #[allow(dead_code)]
    fn find_space_index_for_element(&self, element: &WindowElement) -> Option<usize> {
        self.primary_output_workspaces()?
            .spaces
            .iter()
            .position(|space| space.elements().any(|e| e.id() == element.id()))
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Layer Shell Support
    // ─────────────────────────────────────────────────────────────────────────────

    /// Create a new lay_rs layer for a layer shell surface and add it to the appropriate container.
    /// Returns the new layer.
    pub fn create_layer_shell_layer(
        &self,
        wlr_layer: smithay::wayland::shell::wlr_layer::Layer,
        namespace: &str,
        output: &Output,
    ) -> Layer {
        use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;

        let layer = self.layers_engine.new_layer();
        layer.set_key(format!(
            "layer_shell_{}_{}",
            wlr_layer_to_str(wlr_layer),
            namespace
        ));
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        // Background layer surfaces sit behind everything — no pointer events.
        // Other layer shell surfaces handle their own pointer events.
        let has_pointer = !matches!(wlr_layer, WlrLayer::Background);
        layer.set_pointer_events(has_pointer);

        // Add to appropriate container based on layer
        match wlr_layer {
            WlrLayer::Background | WlrLayer::Bottom => {
                if let Some(ows) = self.output_workspaces.get(&output.name()) {
                    // Wallpaper and widgets go to different containers: only
                    // the wallpaper belongs in the exposé overview.
                    let container = if matches!(wlr_layer, WlrLayer::Bottom) {
                        &ows.layer_shell_bottom
                    } else {
                        &ows.layer_shell_background
                    };
                    container.set_hidden(false);
                    let _ = container.add_sublayer(&layer);
                } else {
                    tracing::warn!(
                        "create_layer_shell_layer: no output_workspaces entry for output '{}' \
                         when creating {:?} layer; layer-shell surface may not be visible",
                        output.name(),
                        wlr_layer,
                    );
                }
            }
            WlrLayer::Top => {
                // Only unhide if current workspace is not fullscreen
                let is_fullscreen = self
                    .get_current_workspace()
                    .map(|ws| ws.get_fullscreen_mode())
                    .unwrap_or(false);
                if !is_fullscreen {
                    self.layer_shell_top.set_hidden(false);
                    self.layer_shell_top.set_opacity(1.0_f32, None);
                }
                if let Err(e) = self.layer_shell_top.add_sublayer(&layer) {
                    tracing::warn!("layer_shell: failed to add top layer: {e}");
                }
            }
            WlrLayer::Overlay => {
                self.layer_shell_overlay.set_hidden(false);
                if let Err(e) = self.layer_shell_overlay.add_sublayer(&layer) {
                    tracing::warn!("layer_shell: failed to add overlay layer: {e}");
                }
            }
        }

        layer
    }

    /// Remove a layer shell layer from the scene graph.
    pub fn remove_layer_shell_layer(&self, layer: &Layer) {
        layer.remove();
    }
}

/// Helper to convert WlrLayer to string for layer keys
fn wlr_layer_to_str(layer: smithay::wayland::shell::wlr_layer::Layer) -> &'static str {
    use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
    match layer {
        WlrLayer::Background => "background",
        WlrLayer::Bottom => "bottom",
        WlrLayer::Top => "top",
        WlrLayer::Overlay => "overlay",
    }
}

#[derive(Clone)]
struct UnminimizeContext {
    wid: ObjectId,
    workspace: Arc<WorkspaceView>,
    window: WindowElement,
    view: WindowView,
    dock: Arc<DockView>,
    layers_engine: Arc<Engine>,
    model: Arc<RwLock<WorkspacesModel>>,
    observers: Vec<Weak<dyn Observer<WorkspacesModel>>>,
    layer_pos: (f32, f32),
    pos_logical: (i32, i32),
}

impl UnminimizeContext {
    fn run(&self) {
        let wid = self.wid.clone();
        let workspace = self.workspace.clone();
        let window = self.window.clone();
        let view = self.view.clone();
        let dock = self.dock.clone();
        let layers_engine = self.layers_engine.clone();
        let model = self.model.clone();
        let observers = self.observers.clone();
        let layer_pos = self.layer_pos;
        let pos_logical = self.pos_logical;

        let event = {
            let mut model = model.write().unwrap();
            model.minimized_windows.retain(|(w, _title)| w != &wid);
            model.clone()
        };

        window.set_is_minimised(false);

        if let Some(drawer) = dock.remove_window_element(&wid) {
            // If the window layer was cleaned up (stale handle), skip the
            // animation and just remap the window so it reappears.
            if !view.is_alive() {
                tracing::warn!("unminimize: window layer is stale, skipping animation");
                drawer.remove();
                workspace.map_window(&window, (pos_logical.0, pos_logical.1).into(), None);
                crate::utils::notify_observers(&observers, &event);
                return;
            }

            let windows_layer_ref = workspace.windows_layer.clone();
            // The mirror belongs to THIS workspace's window selector, not to
            // the output-wide expose layer: parking it there makes the window
            // vanish from its own workspace's exposé and float on top of
            // whichever workspace is on screen.
            let expose_windows_ref = workspace
                .window_selector_view
                .window_selector_windows_container
                .clone();
            let layer_ref = view.window_layer.clone();
            let mirror_ref = view.mirror_layer.clone();
            let target_pos = layer_pos;
            layer_ref.set_hidden(true);
            mirror_ref.set_hidden(true);

            // Clear any color filter that might have been applied during dock interaction
            layer_ref.set_color_filter(None);
            mirror_ref.set_color_filter(None);

            layers_engine.update(0.0);

            let drawer_bounds = drawer.render_bounds_transformed();
            drawer.clear_on_change_size_handlers();
            // Restore the window into the workspace tree NOW, inline. This
            // used to ride on the drawer-shrink transaction's `on_start`,
            // which silently never fires when another dock relayout replaces
            // the transaction — the window then stays parented in the drawer
            // forever and is drawn at full size inside the dock plane.
            layer_ref.remove_draw_content();
            if let Err(e) = windows_layer_ref.add_sublayer(&layer_ref) {
                tracing::warn!("unminimize: failed to reparent window layer: {e}");
            }
            if let Err(e) = expose_windows_ref.add_sublayer(&mirror_ref) {
                tracing::warn!("unminimize: failed to reparent mirror layer: {e}");
            }
            layer_ref.set_position(target_pos, None);
            drawer
                .set_size(
                    Size::points(0.0, 130.0),
                    Transition {
                        delay: 0.2,
                        timing: TimingFunction::ease_out_quad(0.3),
                    },
                )
                .then(move |layer: &Layer, _| {
                    layer.remove();
                });

            let dock_position = self.dock.position();
            view.genie_effect.set_direction(
                dock_position.is_vertical(),
                dock_position == crate::config::DockPosition::Left,
            );
            view.unminimize(drawer_bounds);

            // Make sure the mirror layer is visible again for expose
            view.mirror_layer.set_hidden(false);
        }

        workspace.map_window(&window, (pos_logical.0, pos_logical.1).into(), None);

        crate::utils::notify_observers(&observers, &event);
    }
}

impl Observable<WorkspacesModel> for Workspaces {
    fn add_listener(&mut self, observer: std::sync::Arc<dyn Observer<WorkspacesModel>>) {
        let observer = std::sync::Arc::downgrade(&observer);
        self.observers.push(observer);
    }

    fn observers<'a>(
        &'a self,
    ) -> Box<dyn Iterator<Item = std::sync::Weak<dyn Observer<WorkspacesModel>>> + 'a> {
        Box::new(self.observers.iter().cloned())
    }
}

/// Whether the window's surface tree contains subsurfaces (e.g. the SSD
/// Whether `surface` has any MAPPED popup (menu, tooltip). "Alive" is not
/// enough: GTK keeps a popover's xdg_popup surface alive after popdown for
/// reuse, so an aliveness check would block scanout promotion forever after
/// the first menu. A popup only occludes when it has a committed buffer.
fn surface_has_mapped_popup(
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) -> bool {
    smithay::desktop::PopupManager::popups_for_surface(surface).any(|(popup, _)| {
        smithay::backend::renderer::utils::with_renderer_surface_state(
            popup.wl_surface(),
            |state| state.buffer().is_some(),
        )
        .unwrap_or(false)
    })
}

/// decoration strips: titlebar, buttons, borders). Such windows are
/// promoted in "base-only" mode: the root surface scans out on a KMS
/// plane while the decorations keep rendering in the windows plane.
fn window_has_subsurfaces(window: &WindowElement) -> bool {
    window
        .wl_surface()
        .map(|s| {
            let mut count = 0u32;
            smithay::wayland::compositor::with_surface_tree_downward(
                &s,
                (),
                |_, _, _| smithay::wayland::compositor::TraversalAction::DoChildren(()),
                |_, _, _| count += 1,
                |_, _, _| true,
            );
            count > 1
        })
        .unwrap_or(false)
}

/// Whether the window has any subsurface that overlaps the root surface's own
/// rect. Such a subsurface is window *content* (e.g. Chrome's account panel,
/// which Chrome maps as a `wl_subsurface`), not an out-of-bounds SSD decoration.
///
/// A window like that must NOT be scanned out base-only: the scanout push only
/// offers the root `wl_surface` (whose buffer does not contain the subsurface),
/// and the subsurface renders in the windows dmabuf *below* the root's overlay
/// plane — so an overlapping subsurface would be hidden behind the plane. Such a
/// window is demoted to full compositing; a purely decorative (out-of-bounds)
/// subsurface still qualifies for base-only promotion.
fn window_has_overlapping_subsurface(window: &WindowElement) -> bool {
    use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
    use smithay::utils::{Logical, Point, Rectangle};
    use smithay::wayland::compositor::{
        with_states, with_surface_tree_downward, SubsurfaceCachedState, TraversalAction,
    };

    let Some(root) = window.wl_surface() else {
        return false;
    };

    // Root rect in root-local logical coordinates.
    let root_size = with_states(&root, |states| {
        states
            .data_map
            .get::<RendererSurfaceStateUserData>()
            .and_then(|d| d.lock().unwrap().surface_size())
    });
    let Some(root_size) = root_size else {
        return false;
    };
    let root_rect: Rectangle<i32, Logical> = Rectangle::new((0, 0).into(), root_size);

    let overlaps = std::cell::Cell::new(false);
    with_surface_tree_downward(
        &root,
        Point::<i32, Logical>::from((0, 0)),
        // Accumulate each surface's position (parent + its subsurface offset).
        |_surface, states, parent_loc| {
            let mut cs = states.cached_state.get::<SubsurfaceCachedState>();
            TraversalAction::DoChildren(*parent_loc + cs.current().location)
        },
        |surface, states, parent_loc| {
            if surface.id() == root.id() {
                return;
            }
            let mut cs = states.cached_state.get::<SubsurfaceCachedState>();
            let loc = *parent_loc + cs.current().location;
            if let Some(size) = states
                .data_map
                .get::<RendererSurfaceStateUserData>()
                .and_then(|d| d.lock().unwrap().surface_size())
            {
                if Rectangle::new(loc, size).overlaps(root_rect) {
                    overlaps.set(true);
                }
            }
        },
        |_, _, _| !overlaps.get(),
    );
    overlaps.get()
}

/// The largest share of a zone's cross-axis extent the dock is ever allowed to
/// reserve. The dock is a thin band on one edge; anything past this is not a
/// dock rect for `position` but a stale one from the edge the dock just left,
/// and honouring it would collapse the usable zone (and with it any maximized
/// window) to nothing.
const MAX_DOCK_ZONE_FRACTION: f32 = 0.5;

/// Shrink `zone` so it stops at `dock_geom`, the dock's rect on the `position`
/// edge. Both rects are logical (points).
///
/// Split out of [`Workspaces::subtract_dock`] so the geometry is testable
/// without a live compositor.
pub(crate) fn subtract_dock_rect(
    position: crate::config::DockPosition,
    dock_geom: Rectangle<i32, smithay::utils::Logical>,
    autohide: bool,
    zone: &mut Rectangle<i32, smithay::utils::Logical>,
) {
    // An autohidden dock reserves nothing — that is what autohide means. It
    // also cannot be measured reliably: `get_dock_geometry` reads the bar's
    // live bounds, which mid-slide are wherever the animation has got to. A
    // window is placed once and never re-placed, so one mapped while the dock
    // was still on its way out would keep a dock-sized gap beside it for good.
    if autohide {
        return;
    }
    if dock_geom.size.w <= 0 || dock_geom.size.h <= 0 || zone.size.w <= 0 || zone.size.h <= 0 {
        return;
    }

    // How much of the zone this rect would eat on the dock's own axis.
    let (taken, budget) = match position {
        crate::config::DockPosition::Bottom => {
            ((zone.loc.y + zone.size.h) - dock_geom.loc.y, zone.size.h)
        }
        crate::config::DockPosition::Left => (
            (dock_geom.loc.x + dock_geom.size.w) - zone.loc.x,
            zone.size.w,
        ),
        crate::config::DockPosition::Right => {
            ((zone.loc.x + zone.size.w) - dock_geom.loc.x, zone.size.w)
        }
    };

    // Already clear of the zone: nothing to subtract.
    if taken <= 0 {
        return;
    }
    // Implausible for a dock band — treat the rect as stale and leave the zone
    // alone rather than shrinking the window to a sliver.
    if taken as f32 > budget as f32 * MAX_DOCK_ZONE_FRACTION {
        return;
    }

    match position {
        crate::config::DockPosition::Bottom => zone.size.h -= taken,
        crate::config::DockPosition::Left => {
            zone.loc.x += taken;
            zone.size.w -= taken;
        }
        crate::config::DockPosition::Right => zone.size.w -= taken,
    }
}

#[cfg(test)]
mod dock_zone_tests {
    use super::subtract_dock_rect;
    use crate::config::DockPosition;
    use smithay::utils::Rectangle;

    fn screen() -> Rectangle<i32, smithay::utils::Logical> {
        Rectangle::new((0, 0).into(), (1920, 1080).into())
    }

    /// A bottom dock band, as `get_dock_geometry` reports it: full width, thin.
    fn bottom_dock() -> Rectangle<i32, smithay::utils::Logical> {
        Rectangle::new((0, 980).into(), (1920, 100).into())
    }

    /// A left dock band: thin, full height.
    fn left_dock() -> Rectangle<i32, smithay::utils::Logical> {
        Rectangle::new((0, 0).into(), (100, 1080).into())
    }

    fn right_dock() -> Rectangle<i32, smithay::utils::Logical> {
        Rectangle::new((1820, 0).into(), (100, 1080).into())
    }

    /// Autohide means the dock is not there: it must not reserve a strip that
    /// initial window placement would then leave empty for the window's whole
    /// life. Regression — placement kept a dock-sized gap beside every window
    /// on a config with `autohide = true`.
    #[test]
    fn an_autohidden_dock_reserves_nothing() {
        for (position, geom) in [
            (DockPosition::Bottom, bottom_dock()),
            (DockPosition::Left, left_dock()),
            (DockPosition::Right, right_dock()),
        ] {
            let mut zone = screen();
            subtract_dock_rect(position, geom, true, &mut zone);
            assert_eq!(
                zone,
                screen(),
                "{position:?} dock reserved space while autohidden"
            );
        }
    }

    #[test]
    fn bottom_dock_takes_only_its_own_height() {
        let mut zone = screen();
        subtract_dock_rect(DockPosition::Bottom, bottom_dock(), false, &mut zone);
        assert_eq!(zone, Rectangle::new((0, 0).into(), (1920, 980).into()));
    }

    #[test]
    fn side_docks_take_only_their_own_width() {
        let mut zone = screen();
        subtract_dock_rect(DockPosition::Left, left_dock(), false, &mut zone);
        assert_eq!(zone, Rectangle::new((100, 0).into(), (1820, 1080).into()));

        let mut zone = screen();
        subtract_dock_rect(DockPosition::Right, right_dock(), false, &mut zone);
        assert_eq!(zone, Rectangle::new((0, 0).into(), (1820, 1080).into()));
    }

    /// Regression: moving the dock from an edge to another one re-maximizes the
    /// open windows, and the dock's laid-out rect can still be the one from the
    /// edge it just left. Read naively, a full-height left band as a *bottom*
    /// dock reserves the whole screen height (and a full-width bottom band as a
    /// *side* dock the whole width), so the maximized window is configured at
    /// its minimum size. A stale rect must leave the zone untouched instead.
    #[test]
    fn stale_dock_rect_from_the_previous_edge_never_collapses_the_zone() {
        // Bottom -> Left, still holding the bottom band's rect.
        let mut zone = screen();
        subtract_dock_rect(DockPosition::Left, bottom_dock(), false, &mut zone);
        assert_eq!(
            zone,
            screen(),
            "a bottom band read as a left dock ate the screen"
        );

        // Bottom -> Right.
        let mut zone = screen();
        subtract_dock_rect(DockPosition::Right, bottom_dock(), false, &mut zone);
        assert_eq!(
            zone,
            screen(),
            "a bottom band read as a right dock ate the screen"
        );

        // Left -> Bottom, still holding the left band's rect.
        let mut zone = screen();
        subtract_dock_rect(DockPosition::Bottom, left_dock(), false, &mut zone);
        assert_eq!(
            zone,
            screen(),
            "a left band read as a bottom dock ate the screen"
        );

        // Right -> Bottom.
        let mut zone = screen();
        subtract_dock_rect(DockPosition::Bottom, right_dock(), false, &mut zone);
        assert_eq!(
            zone,
            screen(),
            "a right band read as a bottom dock ate the screen"
        );
    }

    #[test]
    fn a_dock_clear_of_the_zone_is_a_no_op() {
        // Second output: the zone sits to the right of the primary screen, the
        // dock band is on the primary.
        let mut zone = Rectangle::new((1920, 0).into(), (1920, 1080).into());
        let before = zone;
        subtract_dock_rect(DockPosition::Left, left_dock(), false, &mut zone);
        assert_eq!(zone, before);
    }

    #[test]
    fn an_empty_dock_rect_is_a_no_op() {
        let mut zone = screen();
        subtract_dock_rect(
            DockPosition::Bottom,
            Rectangle::new((0, 0).into(), (0, 0).into()),
            false,
            &mut zone,
        );
        assert_eq!(zone, screen());
    }
}
