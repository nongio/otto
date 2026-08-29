//! Otto's own chrome as an AT-SPI application.
//!
//! The dock, the app switcher and the workspace selector are drawn by the
//! compositor, not by any client, so nothing on the accessibility bus knows
//! they exist. A screen reader user would find their applications perfectly
//! readable and the desktop around them silent. This publishes the shell as one
//! more accessible application, built from the models the views already draw
//! from — so what is announced and what is on screen cannot disagree.
//!
//! Nothing is built unless an assistive technology has attached: AccessKit only
//! calls back once one has, and until then this is an idle bus connection.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use accesskit::{
    Action, ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node, NodeId,
    Rect, Role, Tree, TreeId, TreeUpdate,
};
use accesskit_unix::Adapter;
use smithay::reexports::calloop::channel::Sender;
use smithay::reexports::wayland_server::backend::ObjectId;
use tracing::{trace, warn};

use crate::screenshare::CompositorCommand;
use crate::utils::Observer;
use crate::workspaces::{AppSwitcherView, Application, DockModel, DockView, WorkspacesModel};

/// The shell's window node. Everything else hangs off it.
const ROOT: NodeId = NodeId(1);
const DOCK: NodeId = NodeId(2);
const SWITCHER: NodeId = NodeId(3);
const WORKSPACES: NodeId = NodeId(4);
const WINDOWS: NodeId = NodeId(5);

/// Which part of the shell an entry belongs to.
///
/// One application can be in both the dock and the switcher, and a node may
/// appear once in a tree — so the section is part of an entry's identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Dock,
    Switcher,
}

/// An application as the shell shows it.
#[derive(Clone)]
struct ShellApp {
    /// How the dock tells one application from another. Two entries with the
    /// same one are the same application, however they were spelled.
    match_id: String,
    /// What `CompositorCommand::FocusApp` wants.
    identifier: String,
    /// What a screen reader says. The desktop entry's name where there is one,
    /// since an application id read out letter by letter is no use.
    name: String,
    running: bool,
    /// Where it is on screen, in logical pixels. `None` for anything the shell
    /// cannot place — an entry with no bounds can be read, but not found by
    /// pointing at it, which is what mouse review does.
    bounds: Option<Rect>,
}

impl ShellApp {
    fn node_id(&self, section: Section) -> NodeId {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.match_id.hash(&mut hasher);
        match section {
            Section::Dock => 0u8.hash(&mut hasher),
            Section::Switcher => 1u8.hash(&mut hasher),
        }
        // Above the handful of fixed ids, and distinct from workspace nodes.
        NodeId(hasher.finish() | 1 << 63)
    }
}

/// A window as the overview shows it.
///
/// Keyed by a string rather than by the `wl_surface` it stands for, so what is
/// announced can be built and tested without a Wayland connection; the window
/// itself is resolved where an action is dispatched.
#[derive(Clone)]
struct ShellWindow {
    key: String,
    /// Its title, which is the only thing a screen reader can tell windows
    /// apart by.
    title: String,
    active: bool,
    bounds: Option<Rect>,
}

/// A workspace's node.
///
/// Hashed into the same space as every other generated id rather than counted
/// up from a fixed one: an id derived by adding to a container's number
/// collides with the next container the moment one is added, and a collision
/// here is a panic that takes the session down.
fn workspace_node_id(index: usize) -> NodeId {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    index.hash(&mut hasher);
    3u8.hash(&mut hasher);
    NodeId(hasher.finish() | 1 << 63)
}

impl ShellWindow {
    fn node_id(&self) -> NodeId {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.key.hash(&mut hasher);
        2u8.hash(&mut hasher);
        NodeId(hasher.finish() | 1 << 63)
    }
}

/// What the shell looked like at the last model change.
///
/// Kept because AccessKit may ask for the whole tree at any moment, from its
/// own thread, and the views it would otherwise have to be read from are the
/// compositor thread's.
#[derive(Default)]
struct Snapshot {
    dock: Vec<ShellApp>,
    switcher: Vec<ShellApp>,
    switcher_selected: usize,
    switcher_open: bool,
    workspaces: Vec<String>,
    current_workspace: usize,
    /// The all-windows overview is up, so the windows below are what the user
    /// is looking at — and the shell, not any application, is what a screen
    /// reader should be reading.
    expose: bool,
    windows: Vec<ShellWindow>,
}

impl Snapshot {
    /// What each clickable node stands for.
    fn targets(&self) -> HashMap<NodeId, Target> {
        let dock = self.dock.iter().map(|app| {
            (
                app.node_id(Section::Dock),
                Target::App(app.identifier.clone()),
            )
        });
        let switcher = self.switcher.iter().map(|app| {
            (
                app.node_id(Section::Switcher),
                Target::App(app.identifier.clone()),
            )
        });
        dock.chain(switcher).collect()
    }

    fn build(&self) -> TreeUpdate {
        let mut nodes: Vec<(NodeId, Node)> = Vec::new();
        // A node may appear once in a tree: AccessKit panics on a repeat, and a
        // panic here takes the whole compositor with it. The models are meant
        // to be free of repeats by the time they get here, so this is the belt
        // to that braces — no tree Otto builds can bring the session down.
        let mut seen: std::collections::HashSet<NodeId> = std::collections::HashSet::new();

        let mut root = Node::new(Role::Window);
        root.set_label("Otto");

        let mut dock = Node::new(Role::Toolbar);
        dock.set_label(otto_kit::t!("a11y-dock"));
        for app in &self.dock {
            let id = app.node_id(Section::Dock);
            if !seen.insert(id) {
                warn!(app = %app.match_id, "a11y: skipping a repeated dock entry");
                continue;
            }
            let mut node = Node::new(Role::Button);
            node.set_label(app.name.clone());
            // "running" is the dot under the icon; without it a screen reader
            // cannot tell a launcher from an open application.
            node.set_description(if app.running {
                otto_kit::t!("a11y-app-running").to_owned()
            } else {
                otto_kit::t!("a11y-app-not-running").to_owned()
            });
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            if let Some(bounds) = app.bounds {
                node.set_bounds(bounds);
            }
            dock.push_child(id);
            nodes.push((id, node));
        }
        root.push_child(DOCK);
        seen.insert(DOCK);
        nodes.push((DOCK, dock));

        let mut focus = ROOT;

        if self.switcher_open {
            let mut switcher = Node::new(Role::ListBox);
            switcher.set_label(otto_kit::t!("a11y-app-switcher"));
            for (index, app) in self.switcher.iter().enumerate() {
                let id = app.node_id(Section::Switcher);
                if !seen.insert(id) {
                    warn!(app = %app.match_id, "a11y: skipping a repeated switcher entry");
                    continue;
                }
                let mut node = Node::new(Role::ListBoxOption);
                node.set_label(app.name.clone());
                node.set_selected(index == self.switcher_selected);
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                if index == self.switcher_selected {
                    // The switcher's selection *is* the keyboard focus while it
                    // is open: that is the entry the user is moving through.
                    focus = id;
                }
                switcher.push_child(id);
                nodes.push((id, node));
            }
            root.push_child(SWITCHER);
            seen.insert(SWITCHER);
            nodes.push((SWITCHER, switcher));
        }

        if self.expose {
            let mut windows = Node::new(Role::ListBox);
            windows.set_label(otto_kit::t!("a11y-windows"));
            for window in &self.windows {
                let id = window.node_id();
                if !seen.insert(id) {
                    continue;
                }
                let mut node = Node::new(Role::ListBoxOption);
                node.set_label(window.title.clone());
                node.set_selected(window.active);
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                if let Some(bounds) = window.bounds {
                    node.set_bounds(bounds);
                }
                if window.active {
                    // In the overview the active window is where the keyboard
                    // would land on Escape, so it is what has the focus.
                    focus = id;
                }
                windows.push_child(id);
                nodes.push((id, node));
            }
            root.push_child(WINDOWS);
            seen.insert(WINDOWS);
            nodes.push((WINDOWS, windows));
        }

        let mut workspaces = Node::new(Role::ListBox);
        workspaces.set_label(otto_kit::t!("a11y-workspaces"));
        for (index, name) in self.workspaces.iter().enumerate() {
            let id = workspace_node_id(index);
            if !seen.insert(id) {
                continue;
            }
            let mut node = Node::new(Role::ListBoxOption);
            node.set_label(name.clone());
            node.set_selected(index == self.current_workspace);
            workspaces.push_child(id);
            nodes.push((id, node));
        }
        root.push_child(WORKSPACES);
        seen.insert(WORKSPACES);
        nodes.push((WORKSPACES, workspaces));

        nodes.push((ROOT, root));

        TreeUpdate {
            nodes,
            tree: Some(Tree {
                root: ROOT,
                toolkit_name: Some("otto".into()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            tree_id: TreeId::ROOT,
            focus,
        }
    }
}

/// What acting on a node means.
#[derive(Clone)]
enum Target {
    /// Focus an application, wherever its windows are.
    App(String),
    /// Focus one particular window — what the overview chooses between.
    Window(ObjectId),
}

/// Shared between the compositor thread, which refreshes it, and AccessKit's,
/// which reads it.
#[derive(Default)]
struct Shared {
    snapshot: Snapshot,
    targets: HashMap<NodeId, Target>,
}

struct Activation(Arc<Mutex<Shared>>);

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // Answerable straight away: the snapshot is exactly what the tree is
        // built from, and it is never mid-update.
        Some(self.0.lock().unwrap().snapshot.build())
    }
}

struct Actions {
    shared: Arc<Mutex<Shared>>,
    compositor: Sender<CompositorCommand>,
}

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        if !matches!(request.action, Action::Click | Action::Focus) {
            return;
        }

        let target = self
            .shared
            .lock()
            .unwrap()
            .targets
            .get(&request.target_node)
            .cloned();
        let Some(target) = target else {
            return;
        };

        // The same commands the D-Bus interface uses, so an assistive
        // technology's click and a scripted one take the same path.
        let command = match target {
            Target::App(app_id) => {
                trace!(%app_id, "a11y: focusing an application on request");
                CompositorCommand::FocusApp { app_id }
            }
            Target::Window(window_id) => {
                trace!(?window_id, "a11y: focusing a window on request");
                CompositorCommand::FocusWindow { window_id }
            }
        };
        if let Err(err) = self.compositor.send(command) {
            warn!("a11y: could not dispatch a focus request: {err}");
        }
    }
}

struct Deactivation;

impl DeactivationHandler for Deactivation {
    fn deactivate_accessibility(&mut self) {}
}

/// Publishes the shell, and keeps it in step with the models behind it.
pub struct ShellAccessibility {
    adapter: Mutex<Adapter>,
    shared: Arc<Mutex<Shared>>,
    /// The views the models alone cannot answer for: the dock and the switcher
    /// hold the resolved [`crate::workspaces::apps_info::Application`]s, which
    /// is where an application's real name comes from. Weak, because the shell
    /// must not keep either alive.
    dock: Weak<DockView>,
    switcher: Weak<AppSwitcherView>,
    /// Whether the all-windows overview is up. Shared with `Workspaces`, which
    /// is the only thing that changes it.
    expose: Arc<std::sync::atomic::AtomicBool>,
    /// The window views, for their titles and where they have been laid out.
    /// A screen reader has nothing else to tell one window from another.
    window_views: Arc<std::sync::RwLock<HashMap<ObjectId, crate::workspaces::WindowView>>>,
    /// The last model the tree was built from.
    ///
    /// The dock does not follow the workspace model directly — it applies it on
    /// a timer of its own — so its running dots land after the notification
    /// that caused them. The tree is rebuilt again when they do, and it needs
    /// the model that was current at the time to rebuild from.
    last_model: std::sync::RwLock<Option<WorkspacesModel>>,
}

impl ShellAccessibility {
    pub fn new(
        compositor: Sender<CompositorCommand>,
        dock: Weak<DockView>,
        switcher: Weak<AppSwitcherView>,
        expose: Arc<std::sync::atomic::AtomicBool>,
        window_views: Arc<std::sync::RwLock<HashMap<ObjectId, crate::workspaces::WindowView>>>,
    ) -> Arc<Self> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let adapter = Adapter::new(
            Activation(shared.clone()),
            Actions {
                shared: shared.clone(),
                compositor,
            },
            Deactivation,
        );

        Arc::new(Self {
            adapter: Mutex::new(adapter),
            shared,
            dock,
            switcher,
            expose,
            window_views,
            last_model: std::sync::RwLock::new(None),
        })
    }

    /// Whether the shell itself is what the user is working in. A screen reader
    /// reads the focused window, so the dock must not claim the focus away from
    /// the application the user is actually in — only an open switcher does.
    fn set_focused(&self, focused: bool) {
        self.adapter
            .lock()
            .unwrap()
            .update_window_focus_state(focused);
    }

    /// Rebuilds the snapshot from a model change and pushes it.
    fn refresh(&self, model: &WorkspacesModel) {
        let Some(dock) = self.dock.upgrade() else {
            return;
        };

        // Layer geometry is in physical pixels; everything an assistive
        // technology is told — here and in the pointer locator — is in logical
        // ones, so the two can be compared. `model.scale` is the global
        // fallback: a dock on a second output with a different scale is
        // reported slightly off, which costs precision, not correctness.
        let scale = model.scale.max(0.1);
        let to_logical = move |rect: layers::skia::Rect| {
            Rect::new(
                f64::from(rect.left) / scale,
                f64::from(rect.top) / scale,
                f64::from(rect.right) / scale,
                f64::from(rect.bottom) / scale,
            )
        };

        let describe = |app: &Application, running: bool| ShellApp {
            match_id: app.match_id.clone(),
            identifier: app.identifier.clone(),
            name: app.desktop_name().unwrap_or_else(|| app.identifier.clone()),
            running,
            bounds: dock.app_icon_bounds(&app.match_id).map(to_logical),
        };

        // `display_entries` is what the dock draws: launchers and running apps
        // folded into one list, an application that is both appearing once. The
        // raw `launchers` and `running_apps` overlap, and announcing them
        // chained would both lie about the dock and repeat a node.
        let dock_apps: Vec<ShellApp> = dock
            .get_state()
            .display_entries()
            .iter()
            .map(|(app, running)| describe(app, *running))
            .collect();

        let switcher = self.switcher.upgrade();
        let switcher_open = switcher
            .as_ref()
            .map(|switcher| smithay::utils::IsAlive::alive(switcher.as_ref()))
            .unwrap_or(false);
        let switcher_model = switcher.as_ref().map(|switcher| switcher.view.get_state());
        // The switcher lists what is running, and a window's application can be
        // listed twice in a stale model; the tree takes the first of each.
        let mut seen = std::collections::HashSet::new();
        let switcher_apps: Vec<ShellApp> = switcher_model
            .as_ref()
            .map(|model| {
                model
                    .apps
                    .iter()
                    .filter(|app| seen.insert(app.match_id.clone()))
                    .map(|app| ShellApp {
                        bounds: None,
                        ..describe(app, true)
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The selection is an index into the model's own list, which the dedupe
        // above may have shortened: carry it across by identity, not position.
        let switcher_selected = switcher_model
            .as_ref()
            .and_then(|model| model.apps.get(model.current_app))
            .and_then(|selected| {
                switcher_apps
                    .iter()
                    .position(|app| app.match_id == selected.match_id)
            })
            .unwrap_or(0);

        let workspaces = model
            .workspaces
            .iter()
            .enumerate()
            .map(|(position, workspace)| workspace.display_name(position))
            .collect();

        // The overview, when it is up. Windows come from the workspace on
        // screen, in its own order, so what is announced is the order they are
        // laid out in. Their geometry is physical, like the dock's, and is
        // reported in the same logical pixels as everything else.
        let expose = self.expose.load(std::sync::atomic::Ordering::Relaxed);
        let windows = if expose {
            let views = self.window_views.read().unwrap();
            model
                .workspaces
                .get(model.current_workspace)
                .map(|workspace| workspace.windows_list.read().unwrap().clone())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|id| {
                    let view = views.get(&id)?;
                    let base = view.view_base.get_state();
                    Some((
                        id.clone(),
                        ShellWindow {
                            key: format!("{id:?}"),
                            title: if base.title.is_empty() {
                                otto_kit::t!("a11y-untitled-window").to_owned()
                            } else {
                                base.title.clone()
                            },
                            active: base.active,
                            bounds: Some(to_logical(layers::skia::Rect::from_xywh(
                                base.x, base.y, base.w, base.h,
                            ))),
                        },
                    ))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let snapshot = Snapshot {
            dock: dock_apps,
            switcher: switcher_apps,
            switcher_selected,
            switcher_open,
            workspaces,
            current_workspace: model.current_workspace,
            expose,
            windows: windows.iter().map(|(_, window)| window.clone()).collect(),
        };

        let update = {
            let mut shared = self.shared.lock().unwrap();
            let mut targets = snapshot.targets();
            targets.extend(
                windows
                    .into_iter()
                    .map(|(id, window)| (window.node_id(), Target::Window(id))),
            );
            shared.targets = targets;
            shared.snapshot = snapshot;
            shared.snapshot.build()
        };

        // Where the shell "window" is, so the coordinates above are read as
        // screen coordinates: the whole screen, at the origin.
        let screen = Rect::new(
            0.0,
            0.0,
            f64::from(model.width) / scale,
            f64::from(model.height) / scale,
        );
        self.adapter
            .lock()
            .unwrap()
            .set_root_window_bounds(screen, screen);

        // While the switcher or the overview is up, the shell *is* what the
        // user is working in; at any other time it is furniture around whatever
        // application has the keyboard.
        self.set_focused(switcher_open || expose);
        self.adapter.lock().unwrap().update_if_active(|| update);
    }
}

impl Observer<WorkspacesModel> for ShellAccessibility {
    fn notify(&self, event: &WorkspacesModel) {
        *self.last_model.write().unwrap() = Some(event.clone());
        self.refresh(event);
    }
}

/// The dock resolving its own state is the second half of a workspace change,
/// and the half that carries whether an application is running.
///
/// It arrives up to half a second after the model that caused it, on the dock's
/// own task, so a tree built from the model alone says "not running" about an
/// application that has just started and goes on saying it until something else
/// changes the workspace. Rebuilding here is what makes the dock's dots and
/// what a screen reader says about them the same thing.
impl Observer<DockModel> for ShellAccessibility {
    fn notify(&self, _event: &DockModel) {
        let model = self.last_model.read().unwrap().clone();
        if let Some(model) = model {
            self.refresh(&model);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(identifier: &str, name: &str, running: bool) -> ShellApp {
        ShellApp {
            match_id: identifier.to_owned(),
            identifier: identifier.to_owned(),
            name: name.to_owned(),
            running,
            bounds: None,
        }
    }

    fn node<'a>(update: &'a TreeUpdate, id: NodeId) -> &'a Node {
        &update
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .expect("node missing")
            .1
    }

    #[test]
    fn the_dock_is_announced_by_name_not_by_application_id() {
        let snapshot = Snapshot {
            dock: vec![app("org.gnome.Nautilus", "Files", true)],
            workspaces: vec!["Workspace 1".to_owned()],
            ..Snapshot::default()
        };

        let update = snapshot.build();
        let dock = node(&update, DOCK);
        assert_eq!(dock.children().len(), 1);

        let item = node(&update, dock.children()[0]);
        assert_eq!(item.label().as_deref(), Some("Files"));
        // Against the catalogue, not against English: the description follows
        // the desktop's language, and what matters here is that the running
        // dot is described at all.
        assert_eq!(
            item.description().as_deref(),
            Some(otto_kit::t!("a11y-app-running"))
        );
        assert!(item.supports_action(Action::Click));
    }

    #[test]
    fn a_closed_switcher_is_not_in_the_tree_at_all() {
        let snapshot = Snapshot {
            dock: vec![app("a", "A", false)],
            switcher: vec![app("a", "A", false)],
            switcher_open: false,
            ..Snapshot::default()
        };

        let update = snapshot.build();
        assert!(!update.nodes.iter().any(|(id, _)| *id == SWITCHER));
        // With nothing else focused, the shell reports its own window.
        assert_eq!(update.focus, ROOT);
    }

    #[test]
    fn an_open_switcher_puts_the_focus_on_the_selected_application() {
        let snapshot = Snapshot {
            switcher: vec![app("a", "A", true), app("b", "B", true)],
            switcher_selected: 1,
            switcher_open: true,
            ..Snapshot::default()
        };

        let update = snapshot.build();
        assert_eq!(update.focus, app("b", "B", true).node_id(Section::Switcher));
        assert_eq!(node(&update, update.focus).is_selected(), Some(true));
    }

    /// The overview is where a screen reader picks a window, so it has to be
    /// in the tree while it is up — and gone when it is not, so a closed
    /// overview cannot be read as a list of things to choose from.
    #[test]
    fn the_overview_is_in_the_tree_only_while_it_is_up() {
        let window = ShellWindow {
            key: "term".to_owned(),
            title: "Terminal".to_owned(),
            active: true,
            bounds: Some(Rect::new(0.0, 0.0, 400.0, 300.0)),
        };
        let closed = Snapshot {
            windows: vec![window.clone()],
            expose: false,
            ..Snapshot::default()
        };
        assert!(!closed.build().nodes.iter().any(|(id, _)| *id == WINDOWS));

        let open = Snapshot {
            windows: vec![window],
            expose: true,
            ..Snapshot::default()
        };
        let update = open.build();
        let windows = node(&update, WINDOWS);
        assert_eq!(windows.children().len(), 1);

        let entry = node(&update, windows.children()[0]);
        assert_eq!(entry.label().as_deref(), Some("Terminal"));
        assert!(entry.supports_action(Action::Click));
        // The active window is what the keyboard would return to, so it is
        // what the overview reports as focused.
        assert_eq!(update.focus, windows.children()[0]);
    }

    #[test]
    fn the_current_workspace_is_the_selected_one() {
        let snapshot = Snapshot {
            workspaces: vec!["Code".to_owned(), "Mail".to_owned()],
            current_workspace: 1,
            ..Snapshot::default()
        };

        let update = snapshot.build();
        let workspaces = node(&update, WORKSPACES);
        let selected: Vec<&str> = workspaces
            .children()
            .iter()
            .filter(|id| node(&update, **id).is_selected() == Some(true))
            .filter_map(|id| node(&update, *id).label())
            .collect();
        assert_eq!(selected, vec!["Mail"]);
    }

    /// The crash this guards against took the whole session down: an
    /// application pinned to the dock *and* running was announced twice, and
    /// AccessKit panics on a repeated node.
    #[test]
    fn no_tree_can_repeat_a_node() {
        let files = app("org.gnome.Nautilus", "Files", true);
        let window = ShellWindow {
            key: "term".to_owned(),
            title: "Terminal".to_owned(),
            active: true,
            bounds: None,
        };
        // Every section at once, with repeats inside each: the id spaces of
        // the sections must not overlap, and nor must they collide with the
        // fixed nodes the sections themselves use. A workspace numbered from a
        // container's id once landed exactly on another container's.
        let snapshot = Snapshot {
            dock: vec![files.clone(), files.clone()],
            switcher: vec![files.clone(), files.clone()],
            switcher_selected: 0,
            switcher_open: true,
            workspaces: vec![
                "Workspace 1".to_owned(),
                "Workspace 2".to_owned(),
                "Workspace 3".to_owned(),
            ],
            current_workspace: 0,
            expose: true,
            windows: vec![window.clone(), window],
        };

        let update = snapshot.build();
        let mut ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "the tree repeats a node");
    }

    #[test]
    fn one_application_in_two_sections_is_two_nodes() {
        let files = app("org.gnome.Nautilus", "Files", true);
        assert_ne!(
            files.node_id(Section::Dock),
            files.node_id(Section::Switcher)
        );
    }

    /// Everything an assistive technology is told is in logical pixels, the
    /// space the pointer is reported in. Layer geometry is physical, and a
    /// report that mixed the two would put mouse review a scale factor away
    /// from what it was pointing at.
    #[test]
    fn what_is_reported_is_in_the_same_space_as_the_pointer() {
        let scale = 1.5;
        let to_logical = |rect: layers::skia::Rect| {
            Rect::new(
                f64::from(rect.left) / scale,
                f64::from(rect.top) / scale,
                f64::from(rect.right) / scale,
                f64::from(rect.bottom) / scale,
            )
        };

        // A window filling a 2880x1920 panel at 150%: 1920x1280 in points.
        let bounds = to_logical(layers::skia::Rect::from_xywh(0.0, 0.0, 2880.0, 1920.0));
        assert_eq!(bounds, Rect::new(0.0, 0.0, 1920.0, 1280.0));
    }

    /// An entry with no bounds can still be read; one with bounds can also be
    /// found by pointing at it, which is what mouse review does.
    #[test]
    fn a_dock_entry_carries_where_it_is() {
        let placed = ShellApp {
            bounds: Some(Rect::new(10.0, 20.0, 58.0, 68.0)),
            ..app("org.gnome.Nautilus", "Files", true)
        };
        let snapshot = Snapshot {
            dock: vec![placed, app("unplaced", "Unplaced", false)],
            ..Snapshot::default()
        };

        let update = snapshot.build();
        let dock = node(&update, DOCK);
        assert_eq!(
            node(&update, dock.children()[0]).bounds(),
            Some(Rect::new(10.0, 20.0, 58.0, 68.0))
        );
        assert_eq!(node(&update, dock.children()[1]).bounds(), None);
    }

    #[test]
    fn a_click_resolves_to_the_application_it_stands_for() {
        let snapshot = Snapshot {
            dock: vec![app("org.gnome.Nautilus", "Files", true)],
            ..Snapshot::default()
        };
        let targets = snapshot.targets();

        assert!(matches!(
            targets.get(&app("org.gnome.Nautilus", "Files", true).node_id(Section::Dock)),
            Some(Target::App(id)) if id == "org.gnome.Nautilus"
        ));
    }
}
