use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    sync::{atomic::AtomicBool, Arc, RwLock},
    time::Duration,
};

use layers::{
    engine::{
        animation::{Easing, KeyframeSegment, Transition},
        AnimationRef, Engine, NodeRef, TransactionRef,
    },
    prelude::{taffy, Layer, Point, Spring, TimingFunction},
    skia,
    taffy::{prelude::FromLength, style::Style},
    types::{BlendMode, Size},
    view::{BuildLayerTree, LayerTreeBuilder},
};
use otto_kit::prelude::{ContextMenuStyle, MenuItem};
use smithay::{reexports::wayland_server::backend::ObjectId, utils::IsAlive};
use tokio::sync::mpsc;

use crate::{
    config::{Config, DockBookmark, DockPosition},
    shell::WindowElement,
    theme::theme_colors,
    utils::Observer,
    workspaces::{
        app_icons_manager::AppIconsManager, apps_info::ApplicationsInfo, utils::ContextMenuView,
        Application, WorkspacesModel,
    },
};

use super::{
    model::DockModel,
    render::{
        icon_color_filter, label_reach, setup_app_icon, setup_label, setup_miniwindow_icon,
        setup_resize_grip, setup_running_dot,
    },
};

pub const BASE_ICON_SIZE: f32 = 300.0;
/// How far a launch bounce lifts an icon out of the dock, as a fraction of the icon.
const BOUNCE_HOP: f32 = 0.7;
/// As far as the bounce grows on a magnified icon, however large it gets.
const BOUNCE_HOP_CEILING: f32 = 1.3;
const ICON_SCALER_FILL: f32 = 0.9; // The percentage of the icon_scaler that the icon should fill at scale=1.0. Leaves some padding for magnification.

#[derive(Debug, Clone)]
pub(super) struct AppLayerEntry {
    pub(super) layer: Layer,
    /// Icon scaler: fixed-size wrapper that applies a uniform scale to fit the magnified slot.
    pub(super) icon_scaler: Layer,
    /// Mirror layer: replicates the icon stack from `AppIconsManager` (icon + badge + progress).
    pub(super) icon_mirror: Layer,
    pub(super) label_layer: Layer,
    /// The tooltip text, kept so the balloon can be rebuilt when the dock moves
    /// to another screen edge and the arrow has to point elsewhere.
    pub(super) label_text: String,
    pub(super) dot_layer: Layer,
    pub(super) running: bool,
    pub(super) identifier: String,
}

type MiniWindowLayers = (Layer, Layer, Layer, Option<u32>);

/// A press on a dock icon, from the moment it lands — when it could still turn
/// out to be a plain click — through to the drop that commits a new order.
///
/// Only launchers can be reordered: the running apps that are not bookmarked
/// live in a trailing section of the dock and have no persisted place. Dragging
/// one promotes it to a bookmark at the end of the launcher list first, so that
/// from then on the drag is an ordinary reorder.
#[derive(Debug, Clone)]
pub(super) struct IconDrag {
    /// The app being dragged, keyed as `app_layers` is.
    match_id: String,
    /// Pointer position along the dock's long axis when the press landed, in
    /// physical pixels — the same space slot geometry is expressed in.
    grab_px: f32,
    /// Whether the pointer has travelled far enough for this to be a drag
    /// rather than a click. Everything below is only meaningful once it has.
    active: bool,
    /// Index in the launcher list the drag started from.
    start_index: usize,
    /// Index the icon occupies right now.
    index: usize,
    /// How many launchers there are — the drag is clamped to that range.
    launchers: usize,
    /// One slot along the long axis, in physical pixels.
    pitch: f32,
    /// The icon that follows the pointer: a mirror of the app's icon stack,
    /// parented to the drag overlay so it paints over its neighbours while the
    /// real slot stays in the layout, empty, holding the gap.
    ghost: Option<Layer>,
    /// The scale the ghost settles at, i.e. the one an unmagnified icon has.
    ghost_scale: f32,
}

#[derive(Debug, Clone)]
pub struct DockView {
    layers_engine: Arc<Engine>,
    // layers
    pub wrap_layer: layers::prelude::Layer,
    pub view_layer: layers::prelude::Layer,
    pub bar_layer: layers::prelude::Layer,
    pub resize_handle: layers::prelude::Layer,
    dock_apps_container: layers::prelude::Layer,
    /// The places strip, between the handle and the minimized windows: the
    /// Trash today, folders later. Its slots are app slots — a place is a
    /// desktop entry — they just live past the divider.
    dock_places_container: layers::prelude::Layer,
    dock_windows_container: layers::prelude::Layer,
    /// Sits above the icon strips and holds the icon being dragged, so it
    /// paints over its neighbours instead of under the ones that follow it.
    drag_overlay: layers::prelude::Layer,

    pub(super) app_layers: Arc<RwLock<HashMap<String, AppLayerEntry>>>,
    miniwindow_layers: Arc<RwLock<HashMap<ObjectId, MiniWindowLayers>>>,
    state: Arc<RwLock<DockModel>>,
    active: Arc<AtomicBool>,
    notify_tx: tokio::sync::mpsc::Sender<WorkspacesModel>,
    /// Watchers of the dock's own model. See [`DockView::add_model_listener`].
    model_observers: Arc<RwLock<Vec<std::sync::Weak<dyn Observer<DockModel>>>>>,
    latest_event: Arc<tokio::sync::RwLock<Option<WorkspacesModel>>>,
    magnification_position: Arc<RwLock<f32>>,
    pub dragging: Arc<AtomicBool>,
    app_icons_manager: Arc<AppIconsManager>,

    pub context_menu: Arc<RwLock<Option<ContextMenuView>>>,
    /// Counter of context-menu teardowns, bumped when the menu's fade-out
    /// finishes. The dock's menu lives in the dock plane's subtree and paints
    /// past the bounds its damage is derived from (drop shadow, blur rim), so
    /// the plane pipeline redraws that plane in full once per teardown —
    /// otherwise a stale swapchain slot shows the menu again as a trail the
    /// next time the dock repaints partially (magnification, autohide).
    pub menu_teardown_gen: Arc<std::sync::atomic::AtomicUsize>,
    /// The identifier of the app whose icon is currently showing the context-menu pressed state.
    pub(super) context_menu_app_id: Arc<RwLock<Option<String>>>,
    /// Runtime magnification toggle. Mirrors `dock.magnification` so the
    /// magnification hot path is one atomic load rather than a config read;
    /// [`DockView::apply_magnification`] keeps it in step.
    magnification_enabled: Arc<AtomicBool>,
    /// Live drag on the dock handle: the pointer position along the dock's
    /// thickness axis (logical) where the press landed and the `dock.size`
    /// multiplier at that moment. `Some` only while a resize drag is in flight;
    /// the new size is written to the config file when the button is released.
    pub(super) resize_drag: Arc<RwLock<Option<(f64, f64)>>>,
    /// The press or drag currently in flight on a dock icon. See [`IconDrag`].
    pub(super) icon_drag: Arc<RwLock<Option<IconDrag>>>,
    /// Physical screen dimensions, kept in sync by the compositor via `set_screen_size`.
    screen_size: Arc<RwLock<(i32, i32)>>,
    /// Physical dimensions of the area left over after the layer-shell
    /// exclusive zones (the top bar) — the space the dock may actually fill.
    usable_size: Arc<RwLock<(i32, i32)>>,
    /// Pre-computed autohide hot-zone rect, rebuilt by `render_dock` every time the dock
    /// layout changes. `check_dock_hot_zone` reads this without doing any computation.
    pub cached_hot_zone: Arc<RwLock<Option<skia::Rect>>>,
    /// Full dock bounds (at rest) used to decide when to *hide* the dock.
    pub cached_dock_bounds: Arc<RwLock<Option<skia::Rect>>>,
    /// The label layer currently shown as a tooltip — only one visible at a time.
    active_label: Arc<RwLock<Option<Layer>>>,
    /// The `AnimationRef` from the most recent `magnify_elements_with_scale` call,
    /// so callers can attach `on_finish` callbacks to the dock layout animation.
    last_layout_animation: Arc<RwLock<Option<AnimationRef>>>,
    /// The layer currently showing the "pressed" darkening effect.
    pressed_layer: Arc<RwLock<Option<Layer>>>,
    /// Apps whose icon is currently bouncing while a launch is in flight, keyed by
    /// `match_id`. The flag stays `true` while bouncing; setting it `false` (or
    /// removing the entry) stops the bounce loop once a window appears.
    bouncing: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
}
impl PartialEq for DockView {
    fn eq(&self, other: &Self) -> bool {
        self.wrap_layer == other.wrap_layer
    }
}
impl IsAlive for DockView {
    fn alive(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::Relaxed)
    }
}

// FIXME: DockView Layer Structure rename

/// # DockView Layer Structure
///
/// ```diagram
/// DockView
/// └── wrap_layer: `dock`
///     └── view_layer `dock-view`
///         ├── bar_layer `dock-bar`
///         ├── dock_apps_container `dock_app_container`
///         │   ├── App
///         │   │   ├── Icon
///         │   │   └── Label
///         │   └── App
///         │       ├── Icon
///         │       └── Label
///         ├── dock_handle `dock_handle`
///         ├── dock_places_container `dock_places_container`
///         │   └── Place (the Trash)
///         └── dock_windows_container `dock_windows_container`
///             ├── miniwindow
///             └── miniwindow
/// ```
///
///
/// The place whose icon follows the wastebasket, as a `match_id` — the stem of
/// the desktop id in `[dock] trash_desktop_id`, which is what a launcher's
/// `match_id` resolves to.
///
/// It is a desktop entry like any other place: what a click opens and what the
/// menu offers are that entry's own `Exec` and `Actions=`, and pointing the
/// setting at another file manager's entry is all it takes to use that one.
pub(crate) fn trash_match_id() -> String {
    let id = Config::with(|c| c.dock.trash_desktop_id.clone());
    id.strip_suffix(".desktop").unwrap_or(&id).to_string()
}

impl DockView {
    /// Calculate dock bar height based on icon size
    /// Bar height = app container height + top padding + bottom padding
    fn calculate_bar_height(icon_size: f32, scale: f32) -> f32 {
        icon_size + 3.0 * scale
    }

    /// How the dock is pinned to its screen edge: the single flex child
    /// (`view_layer`) is pushed against that edge and centred along it.
    fn wrap_layout_style(position: DockPosition) -> Style {
        let (justify, align) = match position {
            DockPosition::Bottom => (taffy::JustifyContent::Center, taffy::AlignItems::FlexEnd),
            DockPosition::Left => (taffy::JustifyContent::FlexStart, taffy::AlignItems::Center),
            DockPosition::Right => (taffy::JustifyContent::FlexEnd, taffy::AlignItems::Center),
        };
        Style {
            position: layers::taffy::style::Position::Absolute,
            display: layers::taffy::style::Display::Flex,
            justify_content: Some(justify),
            align_items: Some(align),
            justify_items: Some(taffy::JustifyItems::Center),
            ..Default::default()
        }
    }

    /// The `view_layer` translation that slides the dock `amount` physical
    /// pixels off its own screen edge — the axis depends on which edge that is.
    pub fn slide_offset(position: DockPosition, amount: f32) -> (f32, f32) {
        match position {
            DockPosition::Bottom => (0.0, amount),
            DockPosition::Left => (-amount, 0.0),
            DockPosition::Right => (amount, 0.0),
        }
    }

    /// [`Self::slide_offset`] for the dock's current position.
    pub fn slide_position(&self, amount: f32) -> (f32, f32) {
        Self::slide_offset(self.position(), amount)
    }

    /// The screen edge the dock is currently docked to.
    pub fn position(&self) -> DockPosition {
        Config::with(|c| c.dock.position)
    }

    pub fn new(layers_engine: Arc<Engine>, app_icons_manager: Arc<AppIconsManager>) -> Self {
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = Config::with(|config| config.dock.size.clamp(0.5, 2.0)) as f32;
        let position = Config::with(|config| config.dock.position);
        let base_icon_size = 95.0;
        let scaled_icon_size = base_icon_size * dock_size_multiplier * draw_scale;

        let wrap_layer = layers_engine.new_layer();
        wrap_layer.set_key("dock_view");
        wrap_layer.set_pointer_events(false);
        wrap_layer.set_size(Size::percent(1.0, 1.0), None);
        wrap_layer.set_layout_style(Self::wrap_layout_style(position));

        let view_layer = layers_engine.new_layer();

        let _ = wrap_layer.add_sublayer(&view_layer);
        // FIXME: initial dock position
        view_layer.set_position(Self::slide_offset(position, 1000.0), None);
        let view_tree = LayerTreeBuilder::default()
            .key("dock_layout")
            .size(Size::auto())
            .build()
            .unwrap();

        view_layer.build_layer_tree(&view_tree);

        let bar_layer = layers_engine.new_layer();
        let _ = view_layer.add_sublayer(&bar_layer);
        let initial_bar_height =
            Self::calculate_bar_height(scaled_icon_size, dock_size_multiplier * draw_scale);
        let bar_tree = LayerTreeBuilder::default()
            .key("dock_background_bar")
            .pointer_events(false)
            .size(Size {
                width: taffy::percent(1.0_f32),
                height: taffy::Dimension::Length(initial_bar_height),
            })
            .blend_mode(BlendMode::BackgroundBlur)
            .background_color(theme_colors().materials_medium)
            // The same hairline the menus and the labels carry.
            .border_width((otto_kit::theme::Theme::HAIRLINE_WIDTH * draw_scale, None))
            .border_color(theme_colors().hairline)
            .shadow_color(theme_colors().shadow_color)
            .shadow_offset(((0.0, 0.0).into(), None))
            .shadow_radius((20.0, None))
            .layout_style(taffy::Style {
                position: taffy::Position::Absolute,
                ..Default::default()
            })
            .build()
            .unwrap();

        bar_layer.build_layer_tree(&bar_tree);

        let dock_apps_container = layers_engine.new_layer();
        let _ = view_layer.add_sublayer(&dock_apps_container);

        let dot_area_height = 3.0 * draw_scale;
        let container_tree = LayerTreeBuilder::default()
            .key("dock_app_container")
            .pointer_events(false)
            .size(Size {
                width: taffy::Dimension::Auto,
                height: taffy::Dimension::Length(scaled_icon_size + dot_area_height),
            })
            .layout_style(taffy::Style {
                display: taffy::Display::Flex,
                justify_content: Some(taffy::JustifyContent::FlexEnd),
                justify_items: Some(taffy::JustifyItems::FlexEnd),
                align_items: Some(taffy::AlignItems::FlexEnd),
                gap: taffy::Size::<taffy::LengthPercentage>::from_length(0.0_f32),
                min_size: taffy::Size {
                    width: taffy::Dimension::Length(20.0 * draw_scale),
                    height: taffy::Dimension::Length(0.0),
                },
                ..Default::default()
            })
            .build()
            .unwrap();
        dock_apps_container.build_layer_tree(&container_tree);
        dock_apps_container.set_position(Point::new(0.0, 0.0), None);
        let resize_handle = layers_engine.new_layer();
        let _ = view_layer.add_sublayer(&resize_handle);

        let handle_tree = LayerTreeBuilder::default()
            .key("dock_handle")
            .pointer_events(true)
            .size(Size {
                width: taffy::Dimension::Length(scaled_icon_size * 0.4),
                height: taffy::Dimension::Length(initial_bar_height),
            })
            // .background_color(Color::new_rgba(0.0, 0.0, 0.0, 0.0     ))
            .build()
            .unwrap();
        resize_handle.build_layer_tree(&handle_tree);
        setup_resize_grip(&resize_handle, draw_scale, dock_size_multiplier);

        let dock_places_container = layers_engine.new_layer();
        let _ = view_layer.add_sublayer(&dock_places_container);

        let places_tree = LayerTreeBuilder::default()
            .key("dock_places_container")
            .pointer_events(false)
            .position(Point::new(0.0, 0.0))
            .size(Size {
                width: taffy::Dimension::Auto,
                height: taffy::Dimension::Length(scaled_icon_size + dot_area_height),
            })
            .layout_style(taffy::Style {
                display: taffy::Display::Flex,
                justify_content: Some(taffy::JustifyContent::FlexEnd),
                justify_items: Some(taffy::JustifyItems::FlexEnd),
                align_items: Some(taffy::AlignItems::FlexEnd),
                gap: taffy::Size::<taffy::LengthPercentage>::from_length(0.0_f32),
                min_size: taffy::Size {
                    width: taffy::Dimension::Length(0.0),
                    height: taffy::Dimension::Length(0.0),
                },
                ..Default::default()
            })
            .build()
            .unwrap();
        dock_places_container.build_layer_tree(&places_tree);

        let dock_windows_container = layers_engine.new_layer();
        let _ = view_layer.add_sublayer(&dock_windows_container);

        let container_tree = LayerTreeBuilder::default()
            .key("dock_windows_container")
            .pointer_events(false)
            .position(Point::new(0.0, 0.0))
            .size(Size {
                width: taffy::Dimension::Auto,
                height: taffy::Dimension::Length(scaled_icon_size),
            })
            .layout_style(taffy::Style {
                display: taffy::Display::Flex,
                justify_content: Some(taffy::JustifyContent::FlexEnd),
                justify_items: Some(taffy::JustifyItems::FlexEnd),
                align_items: Some(taffy::AlignItems::FlexEnd),
                min_size: taffy::Size {
                    width: taffy::Dimension::Length(20.0 * draw_scale),
                    height: taffy::Dimension::Length(0.0),
                },
                ..Default::default()
            })
            .build()
            .unwrap();
        dock_windows_container.build_layer_tree(&container_tree);

        // Last child of the view, so a dragged icon paints over every strip.
        // It spans the dock and positions nothing itself: the drag reads its
        // rendered origin and places the ghost relative to that.
        let drag_overlay = layers_engine.new_layer();
        let _ = view_layer.add_sublayer(&drag_overlay);
        let overlay_tree = LayerTreeBuilder::default()
            .key("dock_drag_overlay")
            .pointer_events(false)
            .size(Size::percent(1.0, 1.0))
            .layout_style(taffy::Style {
                position: taffy::Position::Absolute,
                ..Default::default()
            })
            .build()
            .unwrap();
        drag_overlay.build_layer_tree(&overlay_tree);

        let mut initial_state = DockModel::new();
        initial_state.width = 1000;

        let (notify_tx, notify_rx) = mpsc::channel(5);
        let dock = Self {
            layers_engine,

            wrap_layer,
            view_layer,
            bar_layer,
            resize_handle,
            dock_apps_container,
            dock_places_container,
            dock_windows_container,
            drag_overlay,
            app_layers: Arc::new(RwLock::new(HashMap::new())),
            miniwindow_layers: Arc::new(RwLock::new(HashMap::new())),
            state: Arc::new(RwLock::new(initial_state)),
            active: Arc::new(AtomicBool::new(true)),
            notify_tx,
            model_observers: Arc::new(RwLock::new(Vec::new())),
            latest_event: Arc::new(tokio::sync::RwLock::new(None)),
            magnification_position: Arc::new(RwLock::new(-500.0)),
            dragging: Arc::new(AtomicBool::new(false)),
            context_menu: Arc::new(RwLock::new(None)),
            menu_teardown_gen: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            context_menu_app_id: Arc::new(RwLock::new(None)),
            magnification_enabled: Arc::new(AtomicBool::new(Config::with(|c| {
                c.dock.magnification
            }))),
            resize_drag: Arc::new(RwLock::new(None)),
            icon_drag: Arc::new(RwLock::new(None)),
            screen_size: Arc::new(RwLock::new((0, 0))),
            usable_size: Arc::new(RwLock::new((0, 0))),
            cached_hot_zone: Arc::new(RwLock::new(None)),
            cached_dock_bounds: Arc::new(RwLock::new(None)),
            active_label: Arc::new(RwLock::new(None)),
            last_layout_animation: Arc::new(RwLock::new(None)),
            pressed_layer: Arc::new(RwLock::new(None)),
            bouncing: Arc::new(RwLock::new(HashMap::new())),
            app_icons_manager,
        };
        dock.refresh_theme();
        dock.notification_handler(notify_rx);
        dock.load_configured_bookmarks();
        dock.load_configured_places();
        dock.watch_the_trash();

        dock
    }

    /// Keep the Trash's icon in step with the can: a full wastebasket while
    /// there is anything in it, an empty one when there is not.
    ///
    /// The compositor watches the directory rather than waiting to be told, so
    /// the icon is right whether or not the Trash window is open — which is
    /// the whole reason this lives here and not in `otto-files`.
    fn watch_the_trash(&self) {
        let dock = self.clone();
        crate::workspaces::trash::watch(move |has_content| {
            dock.set_trash_full(has_content);
        });
    }

    /// Draw the Trash's icon for a can that is full (or not).
    fn set_trash_full(&self, full: bool) {
        let icon_name = if full {
            "user-trash-full"
        } else {
            "user-trash"
        };
        let image = crate::utils::find_icon_with_theme(icon_name, 512, 1)
            .and_then(|path| otto_kit::icons::image_from_path(&path, (512, 512)));
        // A theme without the icon we asked for leaves the desktop entry's own
        // icon in place rather than a hole in the strip.
        let trash = trash_match_id();
        let place = self.bookmark_application(&trash);
        tracing::debug!(
            full,
            icon_name,
            resolved = image.is_some(),
            "trash icon follows the can"
        );
        self.app_icons_manager
            .set_icon_override(&trash, image, place.as_ref());
    }

    /// Re-resolve every icon in the strip against the icon theme in force now.
    ///
    /// Icons are decoded once and kept — in otto-kit's own cache, in
    /// [`ApplicationsInfo`], and in the layer each app's stack draws from — so
    /// a new icon theme reaches nothing until all three are dropped. The
    /// caches go first, then every application in the model is looked up
    /// again; a bookmark's or a place's user-given label is carried across, so
    /// re-resolving does not rename anything.
    pub fn reload_icons(&self) {
        otto_kit::icons::clear_cache();
        let dock = self.clone();
        tokio::spawn(async move {
            ApplicationsInfo::forget_all().await;

            let state = dock.get_state();
            let running = Self::reresolved(&state.running_apps).await;
            let launchers = Self::reresolved(&state.launchers).await;
            let places = Self::reresolved(&state.places).await;
            dock.update_state(&DockModel {
                running_apps: running,
                launchers,
                places,
                ..state
            });
            // The Trash's icon is an override rather than the desktop entry's
            // own, so it is not in the model and has to be re-read separately.
            dock.set_trash_full(crate::workspaces::trash::has_content());
        });
    }

    /// Look every application up again, keeping the label it was given.
    async fn reresolved(apps: &[Application]) -> Vec<Application> {
        let mut resolved = Vec::with_capacity(apps.len());
        for app in apps {
            match ApplicationsInfo::get_app_info_by_id(app.identifier.clone()).await {
                Some(mut fresh) => {
                    fresh.override_name = app.override_name.clone();
                    resolved.push(fresh);
                }
                // Nothing to replace it with: an entry that has gone missing
                // is a worse dock than a stale icon.
                None => resolved.push(app.clone()),
            }
        }
        resolved
    }

    /// Load `[dock] places` into the places strip. Same shape as
    /// [`Self::load_configured_bookmarks`]: a place is a desktop entry, and a
    /// missing one is a warning rather than a hole in the strip.
    fn load_configured_places(&self) {
        let places = Config::with(|c| c.dock.places.clone());
        if places.is_empty() {
            let mut state = self.get_state();
            state.places.clear();
            self.update_state(&state);
            return;
        }

        let dock = self.clone();
        tokio::spawn(async move {
            let mut loaded = Vec::new();
            for place in places {
                let id = place
                    .desktop_id
                    .strip_suffix(".desktop")
                    .unwrap_or(&place.desktop_id)
                    .to_string();
                if let Some(mut app) = ApplicationsInfo::get_app_info_by_id(id).await {
                    app.override_name = place.label.clone();
                    loaded.push(app);
                } else {
                    tracing::warn!("dock place not found: {}", place.desktop_id);
                }
            }

            let mut state = dock.get_state();
            state.places = loaded;
            dock.update_state(&state);
        });
    }

    fn load_configured_bookmarks(&self) {
        let bookmarks = Config::with(|c| c.dock.bookmarks.clone());
        if bookmarks.is_empty() {
            let mut state = self.get_state();
            state.launchers.clear();
            self.update_state(&state);
            return;
        }

        let dock = self.clone();
        tokio::spawn(async move {
            let mut launchers = Vec::new();

            for bookmark in bookmarks {
                let id = bookmark
                    .desktop_id
                    .strip_suffix(".desktop")
                    .unwrap_or(&bookmark.desktop_id)
                    .to_string();
                if let Some(mut app) = ApplicationsInfo::get_app_info_by_id(id).await {
                    app.override_name = bookmark.label.clone();
                    launchers.push(app);
                } else {
                    tracing::warn!("dock bookmark not found: {}", bookmark.desktop_id);
                }
            }

            let mut state = dock.get_state();
            state.launchers = launchers;
            dock.update_state(&state);
        });
    }
    pub fn update_state(&self, state: &DockModel) {
        {
            *self.state.write().unwrap() = state.clone();
        }
        self.render_dock();
        self.notify_model_observers(state);
    }

    /// Watch the dock's own model.
    ///
    /// The dock does not apply a workspace change when it is told about one: it
    /// resolves the running applications on a task of its own, up to half a
    /// second later. Anything that has to agree with what the dock *draws* —
    /// the shell's accessible tree does — has to hear about it then rather than
    /// when the workspace changed.
    pub fn add_model_listener(&self, observer: Arc<dyn Observer<DockModel>>) {
        self.model_observers
            .write()
            .unwrap()
            .push(Arc::downgrade(&observer));
    }

    fn notify_model_observers(&self, state: &DockModel) {
        let observers: Vec<_> = self.model_observers.read().unwrap().clone();
        for observer in observers {
            if let Some(observer) = observer.upgrade() {
                observer.notify(state);
            }
        }
    }
    pub fn get_state(&self) -> DockModel {
        self.state.read().unwrap().clone()
    }
    pub fn is_hidden(&self) -> bool {
        !self.active.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn is_autohide_enabled(&self) -> bool {
        Config::with(|c| c.dock.autohide)
    }
    /// Whether the dock still has to be composited this frame.
    ///
    /// `is_hidden()` flips as soon as a hide is *scheduled*, which is the right
    /// signal for input (the hot zone must arm immediately) but the wrong one
    /// for rendering: gating the dock plane on it drops the dock from the frame
    /// before its slide-out has run, so it appears to vanish instead of sliding
    /// away. Every slide-out ends by setting `hidden` on the layer, so that flag
    /// is the accurate "nothing left to draw" signal.
    pub fn is_hidden_for_render(&self) -> bool {
        self.view_layer.hidden()
    }
    pub fn set_active_flag(&self, active: bool) {
        self.active
            .store(active, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn hide(&self, transition: Option<Transition>) -> TransactionRef {
        tracing::debug!("dock: hide");
        self.active
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let has_transition = transition.is_some();
        let tr = self
            .view_layer
            .set_position(self.slide_position(250.0), transition);
        if has_transition {
            // Only mark the layer hidden once the slide-out has run — the
            // render path gates the dock plane on that flag, so setting it up
            // front would drop the dock from the frame and make it snap away
            // (e.g. when a window animates to fullscreen).
            tr.on_finish(
                |l: &Layer, _| {
                    l.set_hidden(true);
                },
                true,
            );
        } else {
            self.view_layer.set_hidden(true);
        }
        tr
    }
    pub fn show(&self, transition: Option<Transition>) -> TransactionRef {
        if self.is_autohide_enabled() {
            // When autohide is on, external show() calls should keep the dock hidden.
            // Mark active=false so is_hidden() returns true and the hot zone can trigger it.
            self.active
                .store(false, std::sync::atomic::Ordering::Relaxed);
            let tr = self
                .view_layer
                .set_position(self.slide_position(250.0), None);
            self.view_layer.set_hidden(true);
            return tr;
        }
        tracing::debug!("dock: show");
        self.active
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // Unhide before animating in, so the slide is actually composited.
        self.view_layer.set_hidden(false);
        self.view_layer.set_position((0.0, 0.0), transition)
    }
    fn display_entries(&self, state: &DockModel) -> Vec<(Application, bool)> {
        state.display_entries()
    }

    /// The places strip's entries, in the order they are drawn.
    fn display_places(&self, state: &DockModel) -> Vec<(Application, bool)> {
        state.display_places()
    }

    /// Whether `match_id` names a place rather than an application. Places do
    /// not reorder by dragging and are not bookmarks, so a few paths have to
    /// tell the two apart.
    pub(super) fn is_place(&self, match_id: &str) -> bool {
        self.get_state()
            .places
            .iter()
            .any(|place| place.match_id == match_id)
    }
    fn render_elements_layers(&self, available_icon_width: f32) {
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = Config::with(|c| c.dock.size.clamp(0.5, 2.0)) as f32;
        let icon_color_filter = icon_color_filter();
        let state = self.get_state();
        let display_apps = self.display_entries(&state);
        let app_height = available_icon_width * (1.0 + 20.0 / 95.0);
        let miniwindow_height = available_icon_width * (1.0 + 60.0 / 95.0);

        // The dock's "thickness": its height for a bottom dock, its width for a
        // side one. Everything below is expressed along the thickness axis and
        // the long axis, then mapped onto x/y by `position`.
        // Sized from the icon size that actually fits rather than the configured
        // one: once the long axis runs out of room the icons shrink, and a bar
        // that kept the configured thickness would grow fat around tiny icons.
        let bar_thickness =
            Self::calculate_bar_height(available_icon_width, draw_scale * dock_size_multiplier);
        let position = self.position();
        let vertical = position.is_vertical();
        let edge_padding = 4.0 * draw_scale;
        let end_padding = available_icon_width * 10.0 / 95.0;

        // Update view layer padding to match current icon size
        self.view_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Relative,
            display: taffy::Display::Flex,
            flex_direction: if vertical {
                taffy::FlexDirection::Column
            } else {
                taffy::FlexDirection::Row
            },
            justify_content: Some(taffy::JustifyContent::Center),
            justify_items: Some(taffy::JustifyItems::Center),
            align_items: Some(taffy::AlignItems::Center),
            gap: taffy::Size::<taffy::LengthPercentage>::from_length(0.0_f32),
            padding: if vertical {
                taffy::Rect {
                    top: taffy::length(end_padding),
                    bottom: taffy::length(end_padding),
                    right: taffy::length(edge_padding),
                    left: taffy::length(edge_padding),
                }
            } else {
                taffy::Rect {
                    top: taffy::length(edge_padding),
                    bottom: taffy::length(edge_padding),
                    right: taffy::length(end_padding),
                    left: taffy::length(end_padding),
                }
            },
            ..Default::default()
        });

        self.bar_layer
            .set_border_corner_radius(otto_kit::corners::radius(bar_thickness / 3.5), None);

        let handle_long = 25.0 * draw_scale;
        self.resize_handle.set_size(
            if vertical {
                Size {
                    width: taffy::Dimension::Length(bar_thickness),
                    height: taffy::length(handle_long),
                }
            } else {
                Size {
                    width: taffy::length(handle_long),
                    height: taffy::Dimension::Length(bar_thickness),
                }
            },
            None,
        );

        self.bar_layer.set_size(
            if vertical {
                Size {
                    width: taffy::Dimension::Length(bar_thickness),
                    height: taffy::percent(1.0_f32),
                }
            } else {
                Size {
                    width: taffy::percent(1.0_f32),
                    height: taffy::Dimension::Length(bar_thickness),
                }
            },
            None,
        );

        // Icon strips stack along the dock's long axis. Slots grow across it as
        // they magnify, so they have to be aligned to the *screen edge* side of
        // the strip — a left dock grows its icons rightwards, into the screen.
        let cross_align = if position == DockPosition::Left {
            taffy::AlignItems::FlexStart
        } else {
            taffy::AlignItems::FlexEnd
        };
        // The strips keep a minimum length so an empty dock is still a dock
        // with a grabbable handle — except the places strip, whose stub past
        // the divider would read as a gap when the user has removed the Trash.
        for (container, min_length) in [
            (&self.dock_apps_container, 20.0 * draw_scale),
            (&self.dock_places_container, 0.0),
            (&self.dock_windows_container, 20.0 * draw_scale),
        ] {
            container.set_layout_style(taffy::Style {
                display: taffy::Display::Flex,
                flex_direction: if vertical {
                    taffy::FlexDirection::Column
                } else {
                    taffy::FlexDirection::Row
                },
                justify_content: Some(taffy::JustifyContent::FlexEnd),
                justify_items: Some(taffy::JustifyItems::FlexEnd),
                align_items: Some(cross_align),
                gap: taffy::Size::<taffy::LengthPercentage>::from_length(0.0_f32),
                min_size: if vertical {
                    taffy::Size {
                        width: taffy::Dimension::Length(0.0),
                        height: taffy::Dimension::Length(min_length),
                    }
                } else {
                    taffy::Size {
                        width: taffy::Dimension::Length(min_length),
                        height: taffy::Dimension::Length(0.0),
                    }
                },
                ..Default::default()
            });
        }
        self.dock_places_container.set_size(
            if vertical {
                Size {
                    width: taffy::Dimension::Length(available_icon_width),
                    height: taffy::Dimension::Auto,
                }
            } else {
                Size {
                    width: taffy::Dimension::Auto,
                    height: taffy::Dimension::Length(available_icon_width),
                }
            },
            None,
        );
        // The minimized-window strip is sized along the long axis by its
        // content; across it, it must follow the dock like the apps strip does
        // (it kept the horizontal dock's fixed height otherwise, reserving a
        // whole icon of empty space on a side dock).
        self.dock_windows_container.set_size(
            if vertical {
                Size {
                    width: taffy::Dimension::Length(available_icon_width),
                    height: taffy::Dimension::Auto,
                }
            } else {
                Size {
                    width: taffy::Dimension::Auto,
                    height: taffy::Dimension::Length(available_icon_width),
                }
            },
            None,
        );

        let mut previous_app_layers = self.get_app_layers();
        // Apps that just gained their first window this render — used to stop launch bounces.
        let mut newly_running: Vec<String> = Vec::new();
        let mut apps_layers_map = self.app_layers.write().unwrap();
        // Both strips are drawn by the same loop: a place is a desktop entry
        // like a launcher is, and the only difference is which container its
        // slot is added to.
        let display_places = self.display_places(&state);
        let slots = display_apps
            .iter()
            .map(|entry| (&self.dock_apps_container, entry))
            .chain(
                display_places
                    .iter()
                    .map(|entry| (&self.dock_places_container, entry)),
            );
        for (container, (app, running)) in slots {
            let match_id = app.match_id.clone();
            let app_copy = app.clone();
            let app_name = app.clone().desktop_name().unwrap_or(app.identifier.clone());

            match apps_layers_map.entry(match_id.clone()) {
                Entry::Occupied(mut occ) => {
                    let entry = occ.get_mut();
                    entry.identifier = app.identifier.clone();

                    let icon_mirror = entry.icon_mirror.clone();
                    let layer = entry.layer.clone();

                    icon_mirror.set_color_filter(icon_color_filter.clone());

                    // Update icon content if the icon changed (AppIconsManager tracks icon_id).
                    self.app_icons_manager.update_app(&match_id, &app_copy);

                    if !entry.running && *running {
                        newly_running.push(match_id.clone());
                    }
                    entry.running = *running;
                    entry.dot_layer.set_hidden(!*running);

                    previous_app_layers.retain(|l| l.id() != layer.id());
                }
                Entry::Vacant(vac) => {
                    let new_layer = self.layers_engine.new_layer();
                    // icon_scaler wraps icon_mirror: fixed size, scales to fill the magnified slot
                    let icon_scaler = self.layers_engine.new_layer();

                    // Dummy icon_layer just to set up new_layer's container layout via setup_app_icon.
                    // The real icon content lives in AppIconsManager.
                    let dummy_icon_layer = self.layers_engine.new_layer();
                    setup_app_icon(
                        &new_layer,
                        &dummy_icon_layer,
                        app_copy.clone(),
                        available_icon_width,
                        *running,
                    );

                    // Set up icon_scaler as an absolute-positioned square with a fixed size.
                    // Its scale is animated during magnification to fill the parent slot;
                    // it never changes its layout size.
                    {
                        use layers::view::BuildLayerTree;

                        let scaler_tree = layers::view::LayerTreeBuilder::default()
                            .key(format!("icon_scaler_{}", app.identifier))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .size(Size::points(BASE_ICON_SIZE, BASE_ICON_SIZE))
                            .anchor_point(layers::types::Point::new(0.5, 0.5))
                            .picture_cached(true)
                            .image_cache(true)
                            .pointer_events(false)
                            .build()
                            .unwrap();
                        icon_scaler.build_layer_tree(&scaler_tree);
                    }
                    icon_scaler.set_position(
                        Point::new(available_icon_width / 2.0, available_icon_width / 2.0),
                        None,
                    );
                    let initial_scaler = (available_icon_width * ICON_SCALER_FILL) / BASE_ICON_SIZE;
                    icon_scaler.set_scale(Point::new(initial_scaler, initial_scaler), None);

                    // Get or create the permanent icon stack from AppIconsManager.
                    // This stack (icon + badge + progress) is owned by AppIconsManager and
                    // never freed — mirrors pointing at it are always valid.
                    let icon_stack = self
                        .app_icons_manager
                        .get_or_create_stack(&match_id, &app_copy);
                    let icon_stack_ref = icon_stack.id();

                    // Create a mirror layer in the dock slot that replicates the managed stack.
                    let icon_mirror = self.layers_engine.new_layer();
                    {
                        use layers::view::BuildLayerTree;

                        let mirror_tree = layers::view::LayerTreeBuilder::default()
                            .key(format!("dock_icon_mirror_{}", app.identifier))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .size(Size::points(BASE_ICON_SIZE, BASE_ICON_SIZE))
                            .replicate_node(Some(icon_stack_ref))
                            .picture_cached(true)
                            .image_cache(true)
                            .pointer_events(false)
                            .build()
                            .unwrap();
                        icon_mirror.build_layer_tree(&mirror_tree);
                    }
                    icon_mirror.set_color_filter(icon_color_filter.clone());

                    let label_layer = self.layers_engine.new_layer();
                    let app_name_for_label = app_name.clone();
                    setup_label(&label_layer, app_name, position);

                    // Running indicator dot — absolute-positioned against the
                    // screen edge the dock sits on, rendered on top of the icon
                    // because it's the last child.
                    let dot_layer = self.layers_engine.new_layer();
                    let dot_radius = 2.0 * draw_scale;
                    let dot_height = 5.0 * draw_scale;
                    {
                        use layers::view::BuildLayerTree;
                        let dot_tree = layers::view::LayerTreeBuilder::default()
                            .key("_dot")
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                inset: Self::dot_inset(position),
                                ..Default::default()
                            })
                            .size(Self::dot_size(position, dot_height))
                            .pointer_events(false)
                            .build()
                            .unwrap();
                        dot_layer.build_layer_tree(&dot_tree);
                    }
                    setup_running_dot(&dot_layer, dot_radius);
                    dot_layer.set_hidden(!*running);

                    let _ = container.add_sublayer(&new_layer);
                    let _ = new_layer.add_sublayer(&icon_scaler);
                    let _ = icon_scaler.add_sublayer(&icon_mirror);
                    let _ = new_layer.add_sublayer(&label_layer);
                    let _ = new_layer.add_sublayer(&dot_layer);

                    vac.insert(AppLayerEntry {
                        layer: new_layer.clone(),
                        icon_scaler: icon_scaler.clone(),
                        icon_mirror: icon_mirror.clone(),
                        label_layer: label_layer.clone(),
                        label_text: app_name_for_label,
                        dot_layer: dot_layer.clone(),
                        running: *running,
                        identifier: app.identifier.clone(),
                    });

                    new_layer.remove_all_pointer_handlers();

                    previous_app_layers.retain(|l| l.id() != new_layer.id());
                }
            }
        }

        let mut previous_miniwindows = self.get_miniwin_layers();
        let mut miniwindows_layers_map = self.miniwindow_layers.write().unwrap();
        {
            for (win, title) in state.minimized_windows {
                let (layer, _, label, ..) = miniwindows_layers_map
                    .entry(win.clone())
                    .or_insert_with(|| {
                        let new_layer = self.layers_engine.new_layer();
                        let inner_layer = self.layers_engine.new_layer();
                        let label_layer = self.layers_engine.new_layer();

                        let _ = self.dock_windows_container.add_sublayer(&new_layer);

                        setup_miniwindow_icon(&new_layer, &inner_layer, available_icon_width);
                        let _ = new_layer.add_sublayer(&inner_layer);

                        setup_label(&label_layer, title.clone(), position);
                        let _ = new_layer.add_sublayer(&label_layer);

                        (new_layer, inner_layer, label_layer, None)
                    });

                let _label = label.clone();
                layer.remove_all_pointer_handlers();

                previous_miniwindows.retain(|l| l.id() != layer.id());
            }
        }

        // Cleanup layers

        // App layers
        for layer in previous_app_layers {
            let animation = self
                .layers_engine
                .add_animation_from_transition(&Transition::ease_out_quad(0.3), false);
            let mut changes = vec![
                layer.change_opacity(0.0_f32),
                layer.change_size(layers::types::Size::points(0.0, app_height)),
            ];
            if let Some(entry) = apps_layers_map
                .values()
                .find(|entry| entry.layer.id() == layer.id())
            {
                entry
                    .icon_scaler
                    .set_anchor_point_preserving_position(Point::new(0.0, 1.0));
                changes.push(entry.icon_scaler.change_scale(Point::new(0.1, 0.1)));
            }

            let transactions = self.layers_engine.schedule_changes(&changes, animation);
            if let Some(tr) = transactions.into_iter().next() {
                tr.on_finish(
                    |l: &Layer, _| {
                        l.remove();
                    },
                    true,
                );
            }
            self.layers_engine.start_animation(animation, 0.0);
            apps_layers_map.retain(|_, entry| entry.layer.id() != layer.id());
        }

        // Mini window layers
        for layer in previous_miniwindows {
            layer.set_opacity(0.0_f32, Transition::ease_out_quad(0.2));
            layer.set_size(
                layers::types::Size::points(0.0, miniwindow_height),
                Transition::ease_out_quad(0.3),
            );

            miniwindows_layers_map.retain(|_k, (v, ..)| v.id() != layer.id());
        }

        // Stop launch bounces for apps that just got their first window.
        // Drop the layer locks first — `stop_bounce` re-acquires `app_layers`.
        drop(apps_layers_map);
        drop(miniwindows_layers_map);
        for match_id in newly_running {
            self.stop_bounce(&match_id);
        }
    }
    /// Where the running-indicator dot sits inside an icon slot: against the
    /// screen edge the dock is docked to.
    fn dot_inset(position: DockPosition) -> taffy::Rect<taffy::LengthPercentageAuto> {
        match position {
            DockPosition::Bottom => taffy::Rect {
                left: taffy::length(0.0_f32),
                right: taffy::length(0.0_f32),
                top: taffy::LengthPercentageAuto::Auto,
                bottom: taffy::length(0.0_f32),
            },
            DockPosition::Left => taffy::Rect {
                left: taffy::length(0.0_f32),
                right: taffy::LengthPercentageAuto::Auto,
                top: taffy::length(0.0_f32),
                bottom: taffy::length(0.0_f32),
            },
            DockPosition::Right => taffy::Rect {
                left: taffy::LengthPercentageAuto::Auto,
                right: taffy::length(0.0_f32),
                top: taffy::length(0.0_f32),
                bottom: taffy::length(0.0_f32),
            },
        }
    }

    /// The dot strip spans the icon slot across the dock's thickness axis.
    fn dot_size(position: DockPosition, dot_thickness: f32) -> Size {
        if position.is_vertical() {
            Size {
                width: taffy::Dimension::Length(dot_thickness),
                height: taffy::Dimension::Percent(1.0),
            }
        } else {
            Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Length(dot_thickness),
            }
        }
    }

    /// Move the dock to another screen edge and persist the choice.
    ///
    /// The per-icon layers that bake the orientation in (the running dot and the
    /// tooltip balloon) are rebuilt here rather than on every render: they only
    /// ever change when the dock moves.
    /// Draw everything the dock took from the palette again.
    ///
    /// The strip's material, its hairline and its shadow are layer properties
    /// set once, when the dock was built, and the grip, the running dots and
    /// the labels are cached pictures — a re-render moves none of them, so a
    /// change of colour scheme would leave the whole dock in the old palette
    /// while every icon around it followed. Same shape as
    /// [`Self::apply_dock_position`]: the layers stay where they are, what was
    /// baked into them is drawn again.
    pub(crate) fn refresh_theme(&self) {
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = Config::with(|config| config.dock.size.clamp(0.5, 2.0)) as f32;

        self.bar_layer
            .set_background_color(theme_colors().materials_medium, None);
        self.bar_layer
            .set_border_color(theme_colors().hairline, None);
        self.bar_layer
            .set_shadow_color(theme_colors().shadow_color, None);
        setup_resize_grip(&self.resize_handle, draw_scale, dock_size_multiplier);

        let position = self.position();
        let dot_radius = 2.0 * draw_scale;
        {
            let app_layers = self.app_layers.read().unwrap();
            for entry in app_layers.values() {
                setup_running_dot(&entry.dot_layer, dot_radius);
                setup_label(&entry.label_layer, entry.label_text.clone(), position);
            }
        }
        {
            let miniwindows = self.miniwindow_layers.read().unwrap();
            for (win, title) in self.get_state().minimized_windows {
                if let Some((_, _, label_layer, ..)) = miniwindows.get(&win) {
                    setup_label(label_layer, title, position);
                }
            }
        }
    }

    pub(crate) fn apply_dock_position(&self) {
        let position = self.position();
        self.wrap_layer
            .set_layout_style(Self::wrap_layout_style(position));

        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dot_thickness = 5.0 * draw_scale;
        {
            let app_layers = self.app_layers.read().unwrap();
            for entry in app_layers.values() {
                entry.dot_layer.set_layout_style(taffy::Style {
                    position: taffy::Position::Absolute,
                    inset: Self::dot_inset(position),
                    ..Default::default()
                });
                entry
                    .dot_layer
                    .set_size(Self::dot_size(position, dot_thickness), None);
                setup_label(&entry.label_layer, entry.label_text.clone(), position);
            }
        }
        {
            let miniwindows = self.miniwindow_layers.read().unwrap();
            for (win, title) in self.get_state().minimized_windows {
                if let Some((_, _, label_layer, ..)) = miniwindows.get(&win) {
                    setup_label(label_layer, title, position);
                }
            }
        }
        // The dock is at rest wherever it was: re-anchor it to the new edge.
        self.view_layer.set_position(
            if self.is_hidden() {
                Self::slide_offset(position, 250.0)
            } else {
                (0.0, 0.0)
            },
            None,
        );
        // The long axis (and with it the icon-size budget) swaps with the edge.
        let mut state = self.get_state();
        state.width = self.long_axis_budget();
        self.update_state(&state);
        // Everything above only *requests* a layout: the dock's layers keep
        // their old rects until the engine lays them out. Callers read the dock
        // rect right away — moving the dock re-maximizes the open windows —
        // and would otherwise reserve the band on the edge the dock just left.
        self.view_layer.engine.update(0.0);
    }

    pub fn available_icon_size(&self) -> (f32, f32) {
        let state = self.get_state();
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let available_width = state.width as f32 - 20.0 * draw_scale;
        let base_icon_size = 95.0;
        let dock_size_multiplier = Config::with(|c| c.dock.size.clamp(0.5, 2.0)) as f32;
        let icon_size: f32 = base_icon_size * dock_size_multiplier * draw_scale;

        let apps_len = self.display_entries(&state).len() as f32;
        let windows_len = state.minimized_windows.len() as f32;

        let mut component_padding_h: f32 = icon_size * 0.09 * draw_scale;
        if component_padding_h > 5.0 * draw_scale {
            component_padding_h = 5.0 * draw_scale;
        }

        let available_icon_size =
            (available_width - component_padding_h * 2.0) / (apps_len + windows_len);
        (icon_size.min(available_icon_size), icon_size)
    }

    /// How far from its screen edge the dock can ever reach, in physical
    /// pixels. The KMS strip that carries the dock plane is sized from this:
    /// anything past the strip is cropped, so the envelope covers the largest
    /// icon the dock can show, fully magnified, at the top of a launch bounce,
    /// with its label balloon open above it.
    pub fn plane_strip_thickness_px(&self) -> i32 {
        let scale = Config::with(|c| c.screen_scale) as f32;
        let position = self.position();
        // The icon size the dock is configured for — icons only shrink from
        // there when the dock runs out of room.
        let (_, icon_size) = self.available_icon_size();
        let genie_scale = if self
            .magnification_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            Config::with(|c| c.dock.genie_scale) as f32
        } else {
            0.0
        };
        // A slot under the pointer grows to `1 + genie_scale` of its size.
        let magnified = icon_size * (1.0 + genie_scale);
        // A launch bounce lifts the slot by `BOUNCE_HOP` of an icon, and a
        // magnified slot's hop grows up to `BOUNCE_HOP_CEILING` times that.
        let bounce = icon_size * BOUNCE_HOP * BOUNCE_HOP_CEILING;
        let label = label_reach(position, scale);
        // Bar padding around the icons, the strip's own margin from the edge,
        // and shadow bleed: generous, since a cropped icon costs more than a
        // few rows of plane.
        let chrome = Self::calculate_bar_height(0.0, scale) + 24.0 * scale;
        (magnified + bounce + label + chrome).ceil() as i32
    }

    /// How long an icon slot is along the dock when nothing is magnified — the
    /// size every magnified slot is a multiple of.
    fn base_slot_length(&self) -> f32 {
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = Config::with(|c| c.dock.size.clamp(0.5, 2.0)) as f32;
        80.0 * dock_size_multiplier * draw_scale
    }

    /// Render dock elements (app icons and miniwindow icons) based on the current state.
    /// This is called whenever the state changes to update the dock appearance.
    pub(crate) fn render_dock(&self) {
        let (available_icon_size, _) = self.available_icon_size();

        self.render_elements_layers(available_icon_size);
        // When magnification is enabled, re-apply the current hover position so a
        // state-driven re-render (e.g. window focus change) doesn't snap icons back
        // to base size while the pointer is still over the dock.
        // When magnification is disabled, pass genie_scale=0 to size icons correctly.
        let scale_override = if self
            .magnification_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            None
        } else {
            Some(0.0_f64)
        };
        self.magnify_elements_with_scale(scale_override, Some(Transition::spring(0.5, 0.1)));

        // Recompute and cache the autohide hot zone from the new dock dimensions.
        let screen_scale = Config::with(|c| c.screen_scale) as f32;
        let bar_h = Self::calculate_bar_height(available_icon_size, 1.0) / screen_scale;
        let bar_h = bar_h / 2.0;
        let (screen_w, screen_h) = *self.screen_size.read().unwrap();
        // println!("screen x=0, w={}, h={} scale={}", screen_w, screen_h, screen_scale);

        let screen_h = screen_h as f32 / screen_scale;
        let screen_w = screen_w as f32 / screen_scale;
        let position = self.position();
        let has_screen = screen_w > 0.0 && screen_h > 0.0;
        // Both rects hug the edge the dock is docked to: a thin reveal strip on
        // the edge itself, and the dock's own band outset by 40 pts across and
        // 80 pts past the edge, so leaving it in any direction re-hides it.
        let dock_thickness = bar_h * 2.0;
        let hot_zone_thickness = dock_thickness * 0.3;
        *self.cached_hot_zone.write().unwrap() = has_screen.then(|| match position {
            DockPosition::Bottom => skia::Rect::from_xywh(
                0.0,
                screen_h - hot_zone_thickness,
                screen_w,
                hot_zone_thickness,
            ),
            DockPosition::Left => skia::Rect::from_xywh(0.0, 0.0, hot_zone_thickness, screen_h),
            DockPosition::Right => skia::Rect::from_xywh(
                screen_w - hot_zone_thickness,
                0.0,
                hot_zone_thickness,
                screen_h,
            ),
        });
        *self.cached_dock_bounds.write().unwrap() = has_screen.then(|| match position {
            DockPosition::Bottom => skia::Rect::from_xywh(
                -40.0,
                screen_h - dock_thickness,
                screen_w + 80.0,
                dock_thickness + 80.0,
            ),
            DockPosition::Left => {
                skia::Rect::from_xywh(-80.0, -40.0, dock_thickness + 80.0, screen_h + 80.0)
            }
            DockPosition::Right => skia::Rect::from_xywh(
                screen_w - dock_thickness,
                -40.0,
                dock_thickness + 80.0,
                screen_h + 80.0,
            ),
        });
    }
    fn notification_handler(&self, mut rx: tokio::sync::mpsc::Receiver<WorkspacesModel>) {
        // let view = self.view.clone();
        let latest_event = self.latest_event.clone();
        // Task to receive events
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                // Store the latest event
                *latest_event.write().await = Some(event.clone());
            }
        });
        let latest_event = self.latest_event.clone();
        let dock = self.clone();

        tokio::spawn(async move {
            loop {
                // dock updates don't need to be instantanious
                tokio::time::sleep(Duration::from_secs_f32(0.5)).await;

                let event = {
                    let mut latest_event_lock = latest_event.write().await;
                    latest_event_lock.take()
                };

                if let Some(workspace) = event {
                    tracing::info!(target: "otto::dock", "dock event: {} running apps in application_list", workspace.application_list.len());
                    let mut app_set = HashSet::new();
                    let mut apps: Vec<Application> = Vec::new();

                    for app_id in workspace.application_list.iter().rev() {
                        if app_set.insert(app_id.clone()) {
                            if let Some(app) = ApplicationsInfo::get_app_info_by_id(app_id).await {
                                apps.push(app);
                            }
                        }
                    }

                    let minimized_windows = workspace.minimized_windows.clone();

                    let state = dock.get_state();

                    tracing::info!(target: "otto::dock", "dock update_state: {} resolved apps, running={:?}", apps.len(), apps.iter().map(|a| &a.match_id).collect::<Vec<_>>());
                    dock.update_state(&DockModel {
                        running_apps: apps,
                        minimized_windows,
                        ..state
                    });
                }
            }
        });
    }
    /// Where an application's dock icon is on screen, in physical pixels.
    ///
    /// Read by the accessibility layer so an assistive technology can find the
    /// icon spatially — mouse review reads whatever is under the pointer, and
    /// without bounds there is nothing under it. The icon's own layer is the
    /// authority, so magnification is accounted for by construction.
    pub fn app_icon_bounds(&self, match_id: &str) -> Option<skia::Rect> {
        let layers = self.app_layers.read().unwrap();
        let bounds = layers.get(match_id)?.layer.render_bounds_transformed();
        (!bounds.is_empty()).then_some(bounds)
    }

    fn get_app_layers(&self) -> Vec<Layer> {
        let app_layers = self.app_layers.read().unwrap();
        app_layers
            .values()
            .map(|entry| entry.layer.clone())
            .collect()
    }
    fn get_miniwin_layers(&self) -> Vec<Layer> {
        let miniwin_layers = self.miniwindow_layers.read().unwrap();
        miniwin_layers
            .values()
            .cloned()
            .map(|(layer, ..)| layer)
            .collect()
    }
    pub fn get_app_from_layer(&self, layer: &NodeRef) -> Option<(String, String)> {
        let layers_map = self.app_layers.read().unwrap();
        layers_map
            .iter()
            .find(|(_, entry)| entry.layer.id() == *layer)
            .map(|(match_id, entry)| (entry.identifier.clone(), match_id.clone()))
    }

    pub fn is_handle_layer(&self, layer: &NodeRef) -> bool {
        self.resize_handle.id() == *layer
    }
    pub fn get_window_from_layer(&self, layer: &NodeRef) -> Option<ObjectId> {
        let miniwindow_layers = self.miniwindow_layers.read().unwrap();
        if let Some((window, ..)) = miniwindow_layers
            .iter()
            .find(|(_win, (l, ..))| l.id() == *layer)
        {
            return Some(window.clone());
        }

        None
    }
    /// Return the label for the currently hovered dock item, if any.
    pub(super) fn hovered_label(&self) -> Option<Layer> {
        self.layers_engine
            .current_hover()
            .and_then(|layer_id| self.get_label_for_layer(&layer_id))
    }
    /// Return the label layer for the dock item owning `layer`, if any.
    pub(super) fn get_label_for_layer(&self, layer: &NodeRef) -> Option<Layer> {
        if let Some((_, entry)) = self
            .app_layers
            .read()
            .unwrap()
            .iter()
            .find(|(_, e)| e.layer.id() == *layer)
        {
            return Some(entry.label_layer.clone());
        }
        if let Some((_, (_, _, label, _))) = self
            .miniwindow_layers
            .read()
            .unwrap()
            .iter()
            .find(|(_, (l, ..))| l.id() == *layer)
        {
            return Some(label.clone());
        }
        None
    }
    /// Show `label` and hide the previously active label if different.
    pub(super) fn set_active_label(&self, label: Option<Layer>) {
        let mut active = self.active_label.write().unwrap();
        if let Some(prev) = active.as_ref() {
            if label.as_ref().map(|l| l.id() != prev.id()).unwrap_or(true) {
                prev.set_opacity(0.0_f32, None);
            }
        }
        if let Some(l) = label.as_ref() {
            l.set_opacity(1.0_f32, None);
        }
        *active = label;
    }

    /// Apply the "pressed" darkening filter to the given layer and track it
    /// so it can be cleared later. Clears any previously pressed layer first.
    pub(super) fn darken_pressed(&self, layer: &Layer) {
        self.clear_pressed();
        let darken = skia::Color::from_argb(100, 100, 100, 100);
        let add = skia::Color::from_argb(0, 0, 0, 0);
        layer.set_color_filter(skia::color_filters::lighting(darken, add));
        *self.pressed_layer.write().unwrap() = Some(layer.clone());
    }

    /// Remove the darkening filter from the currently pressed layer, if any.
    pub(super) fn clear_pressed(&self) {
        if let Some(layer) = self.pressed_layer.write().unwrap().take() {
            layer.set_color_filter(None);
        }
    }

    /// Returns `true` when the currently hovered layer resolves to the same
    /// darkening target that was recorded on press.
    pub(super) fn is_released_on_pressed(&self, layer_id: &layers::engine::NodeRef) -> bool {
        let pressed = self.pressed_layer.read().unwrap();
        let Some(pressed) = pressed.as_ref() else {
            return false;
        };
        self.darkening_target_for_hover(layer_id)
            .map(|(target, _)| target.id() == pressed.id())
            .unwrap_or(false)
    }

    /// Resolve the hovered `NodeRef` into the layer that should receive the
    /// press darkening effect. Returns the target layer and (optionally) the
    /// label layer that should be shown alongside it.
    pub(super) fn darkening_target_for_hover(
        &self,
        layer_id: &layers::engine::NodeRef,
    ) -> Option<(Layer, Option<Layer>)> {
        // App icon — darken the icon_scaler, show its label.
        if let Some((_, match_id)) = self.get_app_from_layer(layer_id) {
            let app_layers = self.app_layers.read().unwrap();
            if let Some(entry) = app_layers.get(&match_id) {
                return Some((entry.icon_scaler.clone(), Some(entry.label_layer.clone())));
            }
        }
        // Miniwindow — darken the inner content layer (image_cache=true).
        if let Some(wid) = self.get_window_from_layer(layer_id) {
            let miniwindow_layers = self.miniwindow_layers.read().unwrap();
            if let Some((_drawer, inner, label, _)) = miniwindow_layers.get(&wid) {
                return Some((inner.clone(), Some(label.clone())));
            }
        }
        None
    }

    pub fn add_window_element(&self, window: &WindowElement) -> (Layer, Layer) {
        let state = self.get_state();
        let mut minimized_windows = state.minimized_windows.clone();
        minimized_windows.push((window.id(), window.xdg_title().to_string()));

        self.update_state(&DockModel {
            minimized_windows,
            ..self.get_state()
        });
        let layers_map = self.miniwindow_layers.read().unwrap();

        // If the window element was just added, it should exist in the map
        // If it doesn't, it means state update hasn't processed yet - create fallback layers
        if let Some((drawer, inner, ..)) = layers_map.get(&window.id()) {
            (drawer.clone(), inner.clone())
        } else {
            drop(layers_map); // Release read lock
            tracing::warn!(
                "Window {} not in miniwindow_layers map after state update, creating fallback",
                window.id()
            );

            // Create fallback layers
            let new_layer = self.layers_engine.new_layer();
            let inner_layer = self.layers_engine.new_layer();
            let _ = self.dock_windows_container.add_sublayer(&new_layer);
            (new_layer, inner_layer)
        }
    }
    pub fn remove_window_element(&self, wid: &ObjectId) -> Option<Layer> {
        let mut drawer = None;
        let mut miniwindow_layers = self.miniwindow_layers.write().unwrap();
        if let Some((d, _, label, ..)) = miniwindow_layers.get(wid) {
            drawer = Some(d.clone());
            // hide the label
            label.set_opacity(0.0_f32, None);
            miniwindow_layers.remove(wid);
        }
        drawer
    }
    /// Returns the resting icon size used for miniwindow drawers when
    /// magnification is at rest (same formula as `magnify_elements_with_scale`).
    pub fn miniwindow_icon_size(&self) -> f32 {
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = Config::with(|c| c.dock.size.clamp(0.5, 2.0)) as f32;
        let base_icon_size = 80.0;
        base_icon_size * dock_size_multiplier * draw_scale
    }

    /// Returns the `AnimationRef` from the most recent dock layout animation
    /// (set by `magnify_elements_with_scale`), if any.
    pub fn last_layout_animation(&self) -> Option<AnimationRef> {
        *self.last_layout_animation.read().unwrap()
    }

    // Magnify elements
    fn magnify_elements(&self) {
        self.magnify_elements_with_scale(None, Some(Transition::spring(0.005, 0.0)));
    }

    fn magnify_elements_with_scale(
        &self,
        scale_override: Option<f64>,
        transition: Option<Transition>,
    ) {
        let magnification_enabled = self
            .magnification_enabled
            .load(std::sync::atomic::Ordering::SeqCst);
        if scale_override.is_none() && !magnification_enabled {
            return;
        }
        // A drag keeps every slot exactly one pitch wide, which is what lets the
        // dragged icon be placed by counting slots; magnifying under the pointer
        // would move the ground that count is measured against.
        if self.is_icon_dragging() {
            return;
        }
        // Magnification runs along the dock's long axis: x for a bottom dock,
        // y for a side one. `magnification_position` is already the pointer
        // coordinate on that axis (see `on_motion`).
        let position = self.position();
        let vertical = position.is_vertical();
        let axis_start = |r: &skia::Rect| if vertical { r.y() } else { r.x() };
        let axis_len = |r: &skia::Rect| if vertical { r.height() } else { r.width() };

        let pos = *self.magnification_position.read().unwrap();
        let bounds = self.view_layer.render_bounds_transformed();
        let pos = pos - axis_start(&bounds);
        let state = self.get_state();
        let display_apps = self.display_entries(&state);

        let icon_size = self.base_slot_length();

        // Compute focus as a normalized position [0, 1] across all icon slots.
        // The view holds three strips — [apps | handle | places | windows] —
        // with gaps between them that hold no icons at all. The pointer's
        // position is mapped onto the icons alone: gaps are skipped, and a
        // pointer *inside* a gap stays at the end of the strip before it.
        // Subtracting a gap the moment the pointer crosses the last icon of a
        // strip would make `focus` jump backwards and then creep forward
        // again, which reads as the icons wiggling as the pointer passes the
        // divider.
        let apps_bounds = self.dock_apps_container.render_bounds_transformed();
        let places_bounds = self.dock_places_container.render_bounds_transformed();
        let windows_bounds = self.dock_windows_container.render_bounds_transformed();
        let strips: Vec<(f32, f32)> = [&apps_bounds, &places_bounds, &windows_bounds]
            .into_iter()
            .map(|rect| (axis_start(rect) - axis_start(&bounds), axis_len(rect)))
            .filter(|(_, len)| *len > 0.0)
            .collect();
        let elements_width: f32 = strips.iter().map(|(_, len)| len).sum::<f32>().max(1.0);
        // Before the first strip the distance is kept as it is, negative and
        // growing: the pointer resting far off the dock — where it starts —
        // must leave every icon alone, and clamping it to zero would magnify
        // the first one as though the pointer were on it.
        let first_start = strips.first().map(|(start, _)| *start).unwrap_or(0.0);
        let mut consumed = 0.0_f32;
        let mut pos_in_elements = pos - first_start;
        for (index, (start, len)) in strips.iter().enumerate() {
            if pos < *start {
                // In a gap between two strips the pointer holds at the end of
                // the one before it, rather than jumping the gap's width.
                if index > 0 {
                    pos_in_elements = consumed;
                }
                break;
            }
            if pos <= start + len {
                pos_in_elements = consumed + (pos - start);
                break;
            }
            consumed += len;
            // Past the last strip the distance keeps growing, for the same
            // reason it does before the first one.
            pos_in_elements = if index + 1 == strips.len() {
                consumed + (pos - (start + len))
            } else {
                consumed
            };
        }
        let focus = pos_in_elements / elements_width;

        let display_places = self.display_places(&state);
        let apps_len = display_apps.len() as f32;
        let places_len = display_places.len() as f32;
        let windows_len = state.minimized_windows.len() as f32;

        let tot_elements = apps_len + places_len + windows_len;

        let animation =
            transition.map(|t| self.layers_engine.add_animation_from_transition(&t, false));
        let mut changes = Vec::new();
        let genie_scale = scale_override.unwrap_or_else(|| Config::with(|c| c.dock.genie_scale));
        let genie_span = Config::with(|c| c.dock.genie_span);
        {
            let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
            let dot_area_height = 3.0 * draw_scale;
            let container_thickness = icon_size + dot_area_height;
            let change = self.dock_apps_container.change_size(if vertical {
                Size {
                    width: taffy::Dimension::Length(container_thickness),
                    height: taffy::Dimension::Auto,
                }
            } else {
                Size {
                    width: taffy::Dimension::Auto,
                    height: taffy::Dimension::Length(container_thickness),
                }
            });
            changes.push(change);
            let position_change = self
                .dock_apps_container
                .change_position(Point { x: 0.0, y: 0.0 });

            changes.push(position_change);
            // The places strip holds the same slots the apps strip does and is
            // changed in exactly the same way — but a strip whose own size has
            // stopped changing falls back to its content, and a slot's content
            // includes the hidden label balloon, which is half again as tall as
            // the icon. Start it from wherever the apps strip actually is, so
            // the two animate as one instead of the places icons dropping to
            // the bottom of a strip the height of the whole dock.
            //
            // The minimized-window strip is in the same boat: its thickness
            // settles whenever its windows stop changing size, and it then
            // bobbed on a resize exactly as the places strip did.
            let apps_size = self.dock_apps_container.render_size();
            for strip in [&self.dock_places_container, &self.dock_windows_container] {
                let current = strip.render_size();
                strip.set_size(
                    if vertical {
                        Size::points(apps_size.x, current.y)
                    } else {
                        Size::points(current.x, apps_size.y)
                    },
                    None,
                );
            }
            changes.push(self.dock_places_container.change_size(if vertical {
                Size {
                    width: taffy::Dimension::Length(container_thickness),
                    height: taffy::Dimension::Auto,
                }
            } else {
                Size {
                    width: taffy::Dimension::Auto,
                    height: taffy::Dimension::Length(container_thickness),
                }
            }));
            changes.push(
                self.dock_places_container
                    .change_position(Point { x: 0.0, y: 0.0 }),
            );

            // App slots reserve a dot area on the screen-edge side, minimized
            // windows do not — so shift their strip by the same amount, or the
            // two rows of icons sit a few pixels out of line.
            let windows_offset = match position {
                DockPosition::Bottom => Point {
                    x: 0.0,
                    y: -dot_area_height,
                },
                DockPosition::Left => Point {
                    x: dot_area_height,
                    y: 0.0,
                },
                DockPosition::Right => Point {
                    x: -dot_area_height,
                    y: 0.0,
                },
            };
            changes.push(self.dock_windows_container.change_position(windows_offset));
            // Across the dock, the minimized-window strip has to be exactly as
            // thick as an app slot (icon + dot area), or its icons — aligned to
            // the screen-edge side like the apps are — sit lower than the app
            // icons, and their tooltips with them.
            changes.push(self.dock_windows_container.change_size(if vertical {
                Size {
                    width: taffy::Dimension::Length(container_thickness),
                    height: taffy::Dimension::Auto,
                }
            } else {
                Size {
                    width: taffy::Dimension::Auto,
                    height: taffy::Dimension::Length(container_thickness),
                }
            }));
            let layers_map = self.app_layers.read().unwrap_or_else(|e| e.into_inner());
            for (index, (app, _running)) in
                display_apps.iter().chain(display_places.iter()).enumerate()
            {
                if let Some(entry) = layers_map.get(&app.match_id) {
                    let layer = entry.layer.clone();
                    let icon_pos = 1.0 / tot_elements * index as f32 + 1.0 / (tot_elements * 2.0);
                    let icon_focus =
                        1.0 + magnify_function(focus - icon_pos, genie_span) * genie_scale;
                    let focused_icon_size = icon_size * icon_focus as f32;

                    // The slot grows along the dock's long axis with the
                    // magnification, and is one dot-area thicker across it so
                    // the running indicator sits beside the icon.
                    let slot_thickness = focused_icon_size + dot_area_height;
                    let change = layer.change_size(if vertical {
                        Size::points(slot_thickness, focused_icon_size)
                    } else {
                        Size::points(focused_icon_size, slot_thickness)
                    });
                    changes.push(change);

                    let change = entry
                        .icon_scaler
                        .change_size(Size::points(BASE_ICON_SIZE, BASE_ICON_SIZE));
                    changes.push(change);
                    // icon_scaler has a fixed size of 100.0; animate its scale to stretch it
                    // to focused_icon_size. badge and progress scale with it as children.
                    let scaler = (focused_icon_size * ICON_SCALER_FILL) / BASE_ICON_SIZE;

                    // Centre the icon in the part of the slot the dot does not
                    // take: the dot hugs the screen edge, so on a left dock the
                    // icon starts one dot-area in.
                    let scaler_change_position =
                        entry.icon_scaler.change_position(match position {
                            DockPosition::Left => Point {
                                x: dot_area_height + focused_icon_size / 2.0,
                                y: focused_icon_size / 2.0,
                            },
                            _ => Point {
                                x: focused_icon_size / 2.0,
                                y: focused_icon_size / 2.0,
                            },
                        });
                    changes.push(scaler_change_position);
                    let scaler_change = entry.icon_scaler.change_scale(Point {
                        x: scaler,
                        y: scaler,
                    });
                    changes.push(scaler_change);
                }
            }
        }

        let miniwindow_layers = self.miniwindow_layers.read().unwrap();
        let miniwindow_start_index = display_apps.len() + display_places.len();

        for (index, (win, _title)) in state.minimized_windows.iter().enumerate() {
            if let Some((layer, ..)) = miniwindow_layers.get(win) {
                // Use the number of dock entries we actually render (launchers + running)
                // so minimized window magnification lines up with their on-screen order.
                let index = index + miniwindow_start_index;
                let icon_pos = 1.0 / tot_elements * index as f32 + 1.0 / (tot_elements * 2.0);
                let icon_focus = 1.0 + magnify_function(focus - icon_pos, genie_span) * genie_scale;
                let focused_icon_size = icon_size * icon_focus as f32;

                // let ratio = win.w / win.h;
                // let icon_height = focused_icon_size / ratio + 60.0;
                let change = layer.change_size(Size::points(focused_icon_size, focused_icon_size));
                changes.push(change);
            }
        }

        // Update bar height to accommodate magnified icons using helper function

        self.layers_engine.schedule_changes(&changes, animation);
        // self.layers_engine.schedule_changes(&changes, None);
        *self.last_layout_animation.write().unwrap() = animation;
        // The divider is laid out between the strips, so taffy moves it every
        // time the icons around it grow — but the engine only re-reads a
        // node's layout position when that node itself has changed, and
        // nothing about the handle does. Left alone it stood still while the
        // strips slid past it, overlapping the places icons by a dozen pixels
        // until some later render moved it in one jump. Nudging it here, where
        // the icons are given their new sizes, is enough: every frame that
        // moves the strips starts with one of these calls.
        self.resize_handle.redraw();
        if let Some(animation) = animation {
            self.layers_engine.start_animation(animation, 0.0);
        }
    }
    /// Update the physical screen dimensions so `render_dock` can compute a
    /// correct hot zone, and the usable dimensions — the screen minus the
    /// layer-shell exclusive zones (the top bar) — which is the space the dock
    /// itself may occupy.
    ///
    /// The usable extent along the dock's long axis is its budget:
    /// `available_icon_size` divides it by the number of entries and caps the
    /// icon size with the result, and `max_dock_size` derives how far the resize
    /// drag may go from it. Leaving it at the placeholder width meant that cap
    /// was tighter than the configured icon size on any real screen, so
    /// `[dock] size` had no visible effect once a handful of icons were in the
    /// dock.
    pub fn set_screen_size(&self, w: i32, h: i32, usable: (i32, i32)) {
        {
            let mut screen_size = self.screen_size.write().unwrap();
            let mut usable_size = self.usable_size.write().unwrap();
            if *screen_size == (w, h) && *usable_size == usable {
                return;
            }
            *screen_size = (w, h);
            *usable_size = usable;
        }
        let mut state = self.get_state();
        let budget = self.long_axis_budget();
        if state.width != budget {
            state.width = budget;
            self.update_state(&state);
        } else {
            self.render_dock();
        }
    }

    /// How much room the dock has along the axis it stretches on, in physical
    /// pixels: the screen minus whatever layer-shell panels reserved.
    fn long_axis_budget(&self) -> i32 {
        let (usable_w, usable_h) = *self.usable_size.read().unwrap();
        if self.position().is_vertical() {
            usable_h
        } else {
            usable_w
        }
    }

    /// Start bouncing the icon for `match_id` to signal that a launch is in progress.
    /// The icon keeps hopping until [`Self::stop_bounce`] is called (a window appeared)
    /// or a safety cap is reached. No-op if the app is already running or already bouncing.
    pub fn start_bounce(&self, match_id: &str) {
        // Capacity guard: only bounce launchers that aren't already running, and
        // grab the container layer to animate.
        let layer = {
            let layers = self.app_layers.read().unwrap();
            match layers.get(match_id) {
                Some(entry) if !entry.running => entry.layer.clone(),
                _ => return,
            }
        };

        let mut bouncing = self.bouncing.write().unwrap();
        if bouncing.contains_key(match_id) {
            return;
        }
        let flag = Arc::new(AtomicBool::new(true));
        bouncing.insert(match_id.to_string(), flag.clone());
        drop(bouncing);

        // Bounce roughly two-thirds of an icon out of the dock, away from the
        // screen edge it is docked to.
        let distance = self.available_icon_size().0 * BOUNCE_HOP;
        let hop = match self.position() {
            DockPosition::Bottom => Point::new(0.0, -distance),
            DockPosition::Left => Point::new(distance, 0.0),
            DockPosition::Right => Point::new(-distance, 0.0),
        };
        // What a slot measures with nothing magnified. Slots are sized by
        // `magnify_elements_with_scale` whether magnification is on or off —
        // off, the dock just renders them at scale zero — so this is the
        // resting length in both cases.
        let resting_length = self.base_slot_length();
        // Each hop lasts ~0.8s; cap the loop so a failed launch settles after ~20s.
        Self::schedule_bounce_hop(
            layer,
            flag,
            hop,
            resting_length,
            24,
            self.bouncing.clone(),
            match_id.to_string(),
        );
    }

    /// Stop bouncing the icon for `match_id` and settle it back into the dock.
    pub fn stop_bounce(&self, match_id: &str) {
        let flag = self.bouncing.write().unwrap().remove(match_id);
        if let Some(flag) = flag {
            flag.store(false, std::sync::atomic::Ordering::Relaxed);
            // Settle immediately; this also cancels any in-flight hop animation
            // because it targets the same position value.
            if let Some(entry) = self.app_layers.read().unwrap().get(match_id) {
                entry
                    .layer
                    .set_position(Point::new(0.0, 0.0), Some(Transition::spring(0.3, 0.2)));
            }
        }
    }

    /// Run one bounce hop (up, down, small rebound, pause) and, while still flagged
    /// and under the hop cap, schedule the next one from the transaction's finish callback.
    fn schedule_bounce_hop(
        layer: Layer,
        flag: Arc<AtomicBool>,
        hop: Point,
        base_length: f32,
        remaining: u32,
        bouncing: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
        match_id: String,
    ) {
        if remaining == 0 || !flag.load(std::sync::atomic::Ordering::Relaxed) {
            layer.set_position(Point::new(0.0, 0.0), Some(Transition::spring(0.3, 0.2)));
            bouncing.write().unwrap().remove(&match_id);
            return;
        }

        // The keyframes drive the position from rest (0) out to `hop` and back
        // to rest, so the icon settles exactly where it started at the end of
        // every hop.
        let hop = Self::magnified_hop(&layer, hop, base_length);
        layer
            .set_position(hop, Some(Self::bounce_transition()))
            .on_finish(
                move |l: &Layer, _| {
                    Self::schedule_bounce_hop(
                        l.clone(),
                        flag.clone(),
                        hop,
                        base_length,
                        remaining - 1,
                        bouncing.clone(),
                        match_id.clone(),
                    );
                },
                true,
            );
    }

    /// `hop` scaled by how magnified the icon is as this hop starts.
    ///
    /// The hop is a distance in pixels, so an icon grown by the pointer used to
    /// jump the same absolute distance a small one does — barely clearing a
    /// dock its own growth had already made taller. Measuring the slot at the
    /// top of every hop keeps the jump proportionate whatever the pointer is
    /// doing.
    ///
    /// Only a fraction of the growth reaches the jump, and it stops climbing
    /// well before the icon does. A dock configured with a big `genie_scale`
    /// magnifies to nearly twice the icon, and a jump twice as tall reads as
    /// the icon being flung out of the dock rather than hopping in it.
    fn magnified_hop(layer: &Layer, hop: Point, resting_length: f32) -> Point {
        /// How much of the magnification the jump takes on.
        const DAMPING: f32 = 0.4;
        const CEILING: f32 = BOUNCE_HOP_CEILING;

        if resting_length <= 0.0 {
            return hop;
        }
        let size = layer.render_size();
        // The slot grows along the dock's long axis, which is the one the hop
        // does *not* travel along.
        let length = if hop.x == 0.0 { size.x } else { size.y };
        let magnification = (length / resting_length).max(1.0);
        let scale = (1.0 + (magnification - 1.0) * DAMPING).min(CEILING);
        Point::new(hop.x * scale, hop.y * scale)
    }

    /// Keyframe timing for a single launch-bounce hop. `progress` is the fraction of the
    /// target offset applied: a tall hop, a short rebound, then a brief pause at rest.
    fn bounce_transition() -> Transition {
        Transition {
            delay: 0.0,
            timing: TimingFunction::keyframes(vec![
                KeyframeSegment {
                    duration: 0.18,
                    easing: Easing::ease_out_quad(),
                    start_progress: 0.0,
                    end_progress: 1.0,
                },
                KeyframeSegment {
                    duration: 0.16,
                    easing: Easing::ease_in_quad(),
                    start_progress: 1.0,
                    end_progress: 0.0,
                },
                KeyframeSegment {
                    duration: 0.09,
                    easing: Easing::ease_out_quad(),
                    start_progress: 0.0,
                    end_progress: 0.18,
                },
                KeyframeSegment {
                    duration: 0.09,
                    easing: Easing::ease_in_quad(),
                    start_progress: 0.18,
                    end_progress: 0.0,
                },
                KeyframeSegment {
                    duration: 0.30,
                    easing: Easing::linear(),
                    start_progress: 0.0,
                    end_progress: 0.0,
                },
            ]),
        }
    }

    pub(super) fn magnify_elements_animated(&self) {
        self.magnify_elements_with_scale(None, Some(Transition::spring(0.2, 0.1)));
    }

    pub(super) fn demagnify_elements(&self) {
        *self.magnification_position.write().unwrap() = -500.0;
        self.magnify_elements_with_scale(Some(0.0), Some(Transition::spring(0.2, 0.1)));
    }

    pub fn update_magnification_position(&self, pos: f32) {
        *self.magnification_position.write().unwrap() = pos;
        if self.has_menu_open() {
            return;
        }
        self.magnify_elements();
    }
    pub fn bookmark_config_for(&self, match_id: &str) -> Option<DockBookmark> {
        // Places are looked up here too: clicking one launches it exactly the
        // way clicking a bookmark does, and only the strip it is drawn in and
        // the menu it opens tell them apart.
        Config::with(|c| {
            c.dock
                .bookmarks
                .iter()
                .chain(c.dock.places.iter())
                .cloned()
                .collect::<Vec<_>>()
        })
        .iter()
        .find(|b| {
            b.desktop_id
                .strip_suffix(".desktop")
                .unwrap_or(&b.desktop_id)
                == match_id
        })
        .cloned()
    }
    /// Returns the icon_stack layer for `identifier` from AppIconsManager (always valid).
    pub fn get_icon_stack_for_app(&self, identifier: &str) -> Option<Layer> {
        self.app_icons_manager.get_stack(identifier)
    }

    pub fn bookmark_application(&self, match_id: &str) -> Option<Application> {
        let state = self.state.read().unwrap();
        state
            .launchers
            .iter()
            .chain(state.places.iter())
            .find(|app| app.match_id == match_id)
            .cloned()
    }

    /// Update the badge shown on the dock icon for `app_id`.
    /// Pass `None` or an empty string to hide the badge.
    pub fn update_badge_for_app(&self, app_id: &str, text: Option<String>) {
        self.app_icons_manager.update_badge(app_id, text);
    }

    /// Update the progress bar shown on the dock icon for `app_id`.
    /// Pass `None` or a negative value to hide the progress bar.
    pub fn update_progress_for_app(&self, app_id: &str, value: Option<f64>) {
        self.app_icons_manager.update_progress(app_id, value);
    }

    /// Open the dock-settings context menu anchored to the handle.
    pub fn open_handle_context_menu(&self) {
        let scale = Config::with(|c| c.screen_scale) as f32;
        let handle_bounds = self.resize_handle.render_bounds_transformed();
        let wrap_bounds = self.wrap_layer.render_bounds_transformed();
        let (anchor, pos) = self.menu_anchor_for(&handle_bounds, &wrap_bounds, scale, 0.0);

        let (autohide, magnification) = Config::with(|c| (c.dock.autohide, c.dock.magnification));
        let position = self.position();
        // Flat entries rather than a "Position ▸" submenu: the dock's own click
        // handling only hit-tests the top level.
        // The tick is part of the catalogue string rather than glued on here:
        // where it sits relative to the label is a local convention, and a
        // language that wants it elsewhere can move it.
        let position_item =
            |on: &'static str, off: &'static str, value: DockPosition, action: &str| {
                MenuItem::action(if position == value { on } else { off }).with_action_id(action)
            };

        let items = vec![
            MenuItem::action(if autohide {
                otto_kit::t!("dock-auto-hide-on")
            } else {
                otto_kit::t!("dock-auto-hide")
            })
            .with_action_id("toggle_autohide"),
            MenuItem::action(if magnification {
                otto_kit::t!("dock-magnification-on")
            } else {
                otto_kit::t!("dock-magnification")
            })
            .with_action_id("toggle_magnification"),
            MenuItem::separator(),
            position_item(
                otto_kit::t!("dock-position-bottom-on"),
                otto_kit::t!("dock-position-bottom"),
                DockPosition::Bottom,
                "position_bottom",
            ),
            position_item(
                otto_kit::t!("dock-position-left-on"),
                otto_kit::t!("dock-position-left"),
                DockPosition::Left,
                "position_left",
            ),
            position_item(
                otto_kit::t!("dock-position-right-on"),
                otto_kit::t!("dock-position-right"),
                DockPosition::Right,
                "position_right",
            ),
        ];

        let mut context_menu_lock = self.context_menu.write().unwrap();
        if context_menu_lock.is_none() {
            let menu = ContextMenuView::with_teardown_counter(
                &self.wrap_layer,
                items.clone(),
                self.menu_teardown_gen.clone(),
            );
            let s = Config::with(|c| c.screen_scale) as f32;
            menu.set_style(
                ContextMenuStyle::default_with_scale(s).with_theme(crate::theme::kit_theme()),
            );
            *context_menu_lock = Some(menu);
        }
        if let Some(menu) = context_menu_lock.as_ref() {
            menu.set_items(items);
            menu.set_anchor(anchor.0, anchor.1);
            menu.show_at(pos.x, pos.y);
        }
        drop(context_menu_lock);

        // Use a sentinel app_id so actions can be distinguished
        *self.context_menu_app_id.write().unwrap() = Some("__dock__".to_string());
    }

    /// Find the `match_id` (bookmark key) for an app by its `identifier`.
    pub fn match_id_for(&self, identifier: &str) -> Option<String> {
        self.app_layers
            .read()
            .unwrap()
            .iter()
            .find(|(_, e)| e.identifier == identifier)
            .map(|(match_id, _)| match_id.clone())
    }

    /// Whether an app is currently running (has open windows).
    pub fn is_app_running(&self, identifier: &str) -> bool {
        self.app_layers
            .read()
            .unwrap()
            .values()
            .any(|e| e.identifier == identifier && e.running)
    }

    /// Build context-menu items for the given app `identifier`,
    /// reflecting its current running and bookmarked state.
    pub fn build_context_menu_items(&self, identifier: &str) -> Vec<MenuItem> {
        let running = self.is_app_running(identifier);
        let match_id = self.match_id_for(identifier);
        let bookmarked = match_id
            .as_deref()
            .map(|mid| self.bookmark_config_for(mid).is_some())
            .unwrap_or(false);

        let mut items = Vec::new();

        if running {
            items.push(MenuItem::separator());
        } else {
            items.push(MenuItem::action(otto_kit::t!("dock-open")).with_action_id("open"));
            items.push(MenuItem::separator());
        }

        // The app's own menu, straight out of its desktop entry's `Actions=`:
        // Empty Trash for the Trash, a private window for a browser. They come
        // first because they are what this particular icon does — the entries
        // below are what the dock does with any icon.
        let actions = match_id
            .as_deref()
            .and_then(|mid| self.bookmark_application(mid))
            .map(|app| app.actions())
            .unwrap_or_default();
        if !actions.is_empty() {
            for action in actions {
                items.push(
                    MenuItem::action(action.name).with_action_id(format!("action:{}", action.id)),
                );
            }
            items.push(MenuItem::separator());
        }

        // A place is in the dock because it is a place; there is nothing to
        // keep or stop keeping.
        if match_id.as_deref().is_some_and(|mid| self.is_place(mid)) {
            if running {
                items.push(
                    MenuItem::action(otto_kit::t!("dock-quit"))
                        .with_action_id("quit")
                        .with_shortcut("⌘Q"),
                );
            }
            return items;
        }

        let keep_label = if bookmarked {
            otto_kit::t!("dock-keep-in-dock-on")
        } else {
            otto_kit::t!("dock-keep-in-dock")
        };
        let keep_action = if bookmarked {
            "remove_from_dock"
        } else {
            "keep_in_dock"
        };
        items.push(MenuItem::action(keep_label).with_action_id(keep_action));

        if running {
            items.push(MenuItem::separator());
            items.push(
                MenuItem::action(otto_kit::t!("dock-quit"))
                    .with_action_id("quit")
                    .with_shortcut("⌘Q"),
            );
        }

        items
    }

    /// Where a context menu opened off a dock element goes, as an
    /// `(anchor_point, position)` pair in the wrap layer's logical coordinates.
    ///
    /// A bottom dock grows its menus upwards out of the element; a side dock
    /// grows them inwards, away from the screen edge.
    fn menu_anchor_for(
        &self,
        element: &skia::Rect,
        wrap: &skia::Rect,
        scale: f32,
        gap: f32,
    ) -> ((f32, f32), Point) {
        let left = (element.x() - wrap.x()) / scale;
        let right = (element.right - wrap.x()) / scale;
        let top = (element.y() - wrap.y()) / scale;
        let bottom = (element.bottom - wrap.y()) / scale;
        match self.position() {
            DockPosition::Bottom => (
                (0.5, 1.0),
                Point::new((left + right) / 2.0, top - gap * scale),
            ),
            DockPosition::Left => ((0.0, 1.0), Point::new(right + gap * scale, bottom)),
            DockPosition::Right => ((1.0, 1.0), Point::new(left - gap * scale, bottom)),
        }
    }

    pub fn open_context_menu(&self, _pos: Point, app_id: String) {
        // Compute position from the app icon layer to anchor the menu next to it
        let scale = Config::with(|c| c.screen_scale) as f32;
        let (menu_anchor, menu_pos) = {
            let app_layers = self.app_layers.read().unwrap();
            let entry = app_layers.values().find(|e| e.identifier == app_id);
            if let Some(e) = entry {
                let icon_bounds = e.layer.render_bounds_transformed();
                let wrap_bounds = self.wrap_layer.render_bounds_transformed();
                self.menu_anchor_for(&icon_bounds, &wrap_bounds, scale, 10.0)
            } else {
                ((0.5, 1.0), _pos)
            }
        };

        // Hide any visible tooltip before showing the context menu.
        self.set_active_label(None);

        let mut context_menu_lock = self.context_menu.write().unwrap();
        if context_menu_lock.is_some() {
            // If a context menu is already open, close it
            if let Some(menu) = context_menu_lock.as_ref() {
                menu.hide();
            }
        } else {
            let items = self.build_context_menu_items(&app_id);
            let menu = ContextMenuView::with_teardown_counter(
                &self.wrap_layer,
                items,
                self.menu_teardown_gen.clone(),
            );
            let scale = Config::with(|c| c.screen_scale) as f32;
            menu.set_style(
                ContextMenuStyle::default_with_scale(scale).with_theme(crate::theme::kit_theme()),
            );
            *context_menu_lock = Some(menu);
        }

        if let Some(menu) = context_menu_lock.as_ref() {
            // Refresh items in case the menu was reused (app state may have changed)
            let items = self.build_context_menu_items(&app_id);
            menu.set_items(items);
            menu.set_anchor(menu_anchor.0, menu_anchor.1);
            menu.show_at(menu_pos.x, menu_pos.y);
        }
        drop(context_menu_lock);

        // Darken the icon and hide the tooltip for the right-clicked app.
        *self.context_menu_app_id.write().unwrap() = Some(app_id.clone());
        self.set_app_context_menu_active(&app_id, true);
    }
    /// Apply or remove the "context menu open" visual state on an app icon:
    /// - active=true  → darken the icon, hide the label
    /// - active=false → clear the colour filter, restore label visibility
    fn set_app_context_menu_active(&self, app_id: &str, active: bool) {
        let darken_color = skia::Color::from_argb(100, 100, 100, 100);
        let add = skia::Color::from_argb(0, 0, 0, 0);
        let filter = skia::color_filters::lighting(darken_color, add);
        let app_layers = self.app_layers.read().unwrap();
        if let Some(entry) = app_layers.values().find(|e| e.identifier == app_id) {
            if active {
                entry.icon_scaler.set_color_filter(filter);
                entry.icon_scaler.set_opacity(1.0_f32, None);
            } else {
                entry.icon_scaler.set_color_filter(None);
                entry.icon_scaler.set_opacity(1.0_f32, None);
            }
        }
    }

    pub fn has_menu_open(&self) -> bool {
        if let Some(menu) = self.context_menu.read().unwrap().as_ref() {
            menu.is_active()
        } else {
            false
        }
    }

    /// Reconcile the dock with `dock.magnification`.
    pub(crate) fn apply_magnification(&self) {
        let enabled = Config::with(|c| c.dock.magnification);
        self.magnification_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
        if !enabled {
            // Reset all icons to base size immediately
            self.update_magnification_position(-500.0);
        }
    }

    /// Reconcile the dock with `dock.autohide`: hiding it right away when the
    /// setting goes on, and bringing it back when it goes off.
    pub(crate) fn apply_autohide(&self) {
        if Config::with(|c| c.dock.autohide) {
            self.hide(Some(Transition::ease_out_quad(0.3)));
        } else {
            self.show(Some(Transition::ease_out_quad(0.3)));
        }
    }

    /// Persist the dock keys the dock itself owns — currently only the
    /// bookmarks, which are a list rather than a setting and so have no schema
    /// identifier of their own. Every scalar dock setting is written by
    /// [`crate::settings::set`].
    pub(super) fn update_bookmarks(&self, f: impl FnOnce(&mut Vec<crate::config::DockBookmark>)) {
        let config = crate::config::Config::update(|config| f(&mut config.dock.bookmarks));
        crate::config::save_dock_bookmarks(&config.dock.bookmarks);
    }

    /// Logical pixels of dock thickness per unit of `dock.size`, i.e. how far
    /// the pointer has to travel to change the multiplier by 1. The icon is
    /// `95 * size * screen_scale * 0.8` physical pixels across, and the pointer
    /// position is logical, so the scale cancels out.
    const RESIZE_PIXELS_PER_UNIT: f64 = 95.0 * 0.8;

    /// The pointer coordinate along the axis a resize drag runs on, oriented so
    /// that dragging *away* from the dock's screen edge — up for a bottom dock,
    /// inwards for a side one — always grows it.
    fn resize_axis_pos(&self, pointer: (f64, f64)) -> f64 {
        match self.position() {
            DockPosition::Bottom => -pointer.1,
            DockPosition::Left => pointer.0,
            DockPosition::Right => -pointer.0,
        }
    }

    /// The largest `dock.size` there is room for, bounded two ways:
    ///
    /// * along the dock — every icon must still fit in the usable extent (the
    ///   screen minus the top bar), or the dock runs off the screen and takes
    ///   the resize handle with it, leaving no way to drag it back;
    /// * across it — the dock may take at most a quarter of its thickness axis.
    fn max_dock_size(&self) -> f64 {
        let draw_scale = Config::with(|config| config.screen_scale) * 0.8;
        if draw_scale <= 0.0 {
            return 2.0;
        }
        let mut max = 2.0_f64;

        let budget = self.long_axis_budget() as f64;
        if budget > 0.0 {
            let state = self.get_state();
            let entries =
                (self.display_entries(&state).len() + state.minimized_windows.len()).max(1) as f64;
            // One slot is `95 * size * draw_scale` wide, plus the view's end
            // padding (10/95 of an icon each side); the handle and a margin are
            // fixed. Solve `used(size) <= budget` for size.
            let per_unit = (entries * 95.0 + 20.0) * draw_scale;
            let fixed = 65.0 * draw_scale; // resize handle + breathing room
            max = max.min((budget - fixed).max(0.0) / per_unit);
        }

        let (screen_w, screen_h) = *self.screen_size.read().unwrap();
        let cross_px = if self.position().is_vertical() {
            screen_w
        } else {
            screen_h
        } as f64;
        if cross_px > 0.0 {
            // `calculate_bar_height` is `95 * size * draw_scale` plus a hair of padding.
            max = max.min(cross_px * 0.25 / (98.0 * draw_scale));
        }

        max.clamp(0.5, 2.0)
    }

    /// Remember where a resize drag on the dock handle started. The size is
    /// updated live from `resize_drag_update` and only written to the config
    /// file when the drag ends.
    pub(super) fn begin_resize_drag(&self, pointer: (f64, f64)) {
        let size = Config::with(|c| c.dock.size);
        *self.resize_drag.write().unwrap() = Some((self.resize_axis_pos(pointer), size));
    }

    /// Apply an in-flight resize drag. Returns whether a drag was actually in
    /// flight.
    pub(super) fn resize_drag_update(&self, pointer: (f64, f64)) -> bool {
        let Some((start, start_size)) = *self.resize_drag.read().unwrap() else {
            return false;
        };
        let travel = self.resize_axis_pos(pointer) - start;
        let size =
            (start_size + travel / Self::RESIZE_PIXELS_PER_UNIT).clamp(0.5, self.max_dock_size());
        if (size - Config::with(|c| c.dock.size)).abs() < f64::EPSILON {
            return true;
        }
        // Live during the drag, without touching the file: persistence and the
        // `Changed` signal wait until the interaction settles, in
        // `end_resize_drag`.
        Config::update(|config| config.dock.size = size);
        self.render_dock();
        true
    }

    /// End a resize drag and persist the size that was landed on. Returns
    /// whether a drag was in flight, so the caller can swallow the click.
    pub(super) fn end_resize_drag<B: crate::state::Backend + 'static>(
        &self,
        state: &mut crate::Otto<B>,
    ) -> bool {
        let Some((_, start_size)) = self.resize_drag.write().unwrap().take() else {
            return false;
        };
        let size = Config::with(|c| c.dock.size);
        if (size - start_size).abs() > f64::EPSILON {
            // The size is already live; going through `set` is what persists it
            // and tells every observer, exactly as a settings app would.
            if let Err(err) = crate::settings::set(
                state,
                "dock.size",
                crate::settings::value::SettingValue::Double(size),
            ) {
                tracing::warn!("Could not save the dock size: {err}");
            }
        }
        true
    }

    /// Physical pixels the pointer has to travel along the dock before a press
    /// on an icon becomes a reorder drag rather than a click.
    const DRAG_THRESHOLD_PX: f32 = 8.0;

    /// A pointer position projected onto the dock's long axis, in physical
    /// pixels — the space slot sizes and positions are expressed in.
    fn drag_axis_px(&self, pointer: (f64, f64)) -> f32 {
        let scale = Config::with(|c| c.screen_scale) as f32;
        let along = if self.position().is_vertical() {
            pointer.1
        } else {
            pointer.0
        };
        along as f32 * scale
    }

    /// A displacement along the dock's long axis, as a point.
    fn along_axis(&self, amount: f32) -> Point {
        if self.position().is_vertical() {
            Point::new(0.0, amount)
        } else {
            Point::new(amount, 0.0)
        }
    }

    /// Where the dragged icon has to sit, in the drag overlay's coordinates, to
    /// be under `along_px` — a position on the dock's long axis, in physical
    /// pixels — and in line with the row of icons.
    ///
    /// Every part of this is read live rather than cached from the press: the
    /// dock is still settling out of its magnified shape when a drag starts,
    /// and a magnified dock is both fatter and differently placed than the flat
    /// one the drag ends up working against. An icon positioned from what the
    /// dock looked like at the press hangs off the pointer by the difference.
    fn ghost_point(&self, along_px: f32, pitch: f32) -> Point {
        let overlay = self.drag_overlay.render_bounds_transformed();
        let icons = self.dock_apps_container.render_bounds_transformed();
        // The strip of icons is exactly one unmagnified icon thick plus the
        // running-indicator dot, whatever the magnification is doing, and the
        // dot hugs the screen edge — so the middle of the row of icons is a
        // pitch's half in from the edge the dock is not docked to.
        match self.position() {
            DockPosition::Bottom => Point::new(
                along_px - overlay.left,
                icons.top + pitch / 2.0 - overlay.top,
            ),
            DockPosition::Left => Point::new(
                icons.right - pitch / 2.0 - overlay.left,
                along_px - overlay.top,
            ),
            DockPosition::Right => Point::new(
                icons.left + pitch / 2.0 - overlay.left,
                along_px - overlay.top,
            ),
        }
    }

    /// The distance between two icon slots along the dock's long axis: an
    /// unmagnified icon, which is what every slot measures while a drag is in
    /// flight (see [`Self::begin_icon_drag`]).
    fn slot_pitch(&self) -> f32 {
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = Config::with(|c| c.dock.size.clamp(0.5, 2.0)) as f32;
        80.0 * dock_size_multiplier * draw_scale
    }

    /// Whether an icon is being dragged right now. Magnification and the
    /// tooltip stand down while it is.
    pub(super) fn is_icon_dragging(&self) -> bool {
        self.icon_drag
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(|drag| drag.active)
    }

    /// Record a press on the icon of `match_id`. Nothing happens yet: the press
    /// only becomes a drag once the pointer has moved
    /// [`Self::DRAG_THRESHOLD_PX`] along the dock, so a plain click still
    /// launches or focuses the app.
    pub(super) fn begin_icon_drag(&self, match_id: &str, pointer: (f64, f64)) {
        // A place is not part of the launcher order and cannot be reordered
        // into it: dragging one would count slots against a strip it is not in.
        if self.is_place(match_id) {
            return;
        }
        *self.icon_drag.write().unwrap() = Some(IconDrag {
            match_id: match_id.to_string(),
            grab_px: self.drag_axis_px(pointer),
            active: false,
            start_index: 0,
            index: 0,
            launchers: 0,
            pitch: 0.0,
            ghost: None,
            ghost_scale: 1.0,
        });
    }

    /// Advance an in-flight icon drag. Returns whether the drag has taken over
    /// the pointer, in which case the caller must not treat the motion as
    /// hovering.
    pub(super) fn icon_drag_update(&self, pointer: (f64, f64)) -> bool {
        let Some(mut drag) = self.icon_drag.read().unwrap().clone() else {
            return false;
        };
        let px = self.drag_axis_px(pointer);
        if !drag.active {
            if (px - drag.grab_px).abs() < Self::DRAG_THRESHOLD_PX {
                return false;
            }
            if !self.activate_icon_drag(&mut drag) {
                // Nothing draggable under the press after all — forget it, so
                // the release still counts as a click.
                *self.icon_drag.write().unwrap() = None;
                return false;
            }
        }

        // Clamp to the launcher section: the running apps that follow it have
        // no persisted order to take part in.
        let min = -(drag.start_index as f32) * drag.pitch;
        let max = drag
            .launchers
            .saturating_sub(1)
            .saturating_sub(drag.start_index) as f32
            * drag.pitch;
        let travel = (px - drag.grab_px).clamp(min, max);
        if let Some(ghost) = drag.ghost.as_ref() {
            ghost.set_position(self.ghost_point(drag.grab_px + travel, drag.pitch), None);
        }

        // Round to the nearest slot, so the icon changes places when it has
        // covered half of one.
        let target = (drag.start_index as f32 + travel / drag.pitch).round() as isize;
        let target = target.clamp(0, drag.launchers.saturating_sub(1) as isize) as usize;
        if target != drag.index {
            self.move_dragged_icon(&mut drag, target);
        }

        *self.icon_drag.write().unwrap() = Some(drag);
        true
    }

    /// Turn a press that has moved far enough into a real drag: promote the app
    /// to a bookmark if it is only running, flatten the magnification so every
    /// slot is one pitch wide, and lift the icon into the drag overlay.
    ///
    /// Returns `false` when the app cannot be dragged (it disappeared, or it has
    /// no icon to lift), leaving the dock untouched.
    fn activate_icon_drag(&self, drag: &mut IconDrag) -> bool {
        let match_id = drag.match_id.clone();
        let mut state = self.get_state();
        let index = match state.launchers.iter().position(|a| a.match_id == match_id) {
            Some(index) => index,
            None => {
                // Only running: give it a place of its own before moving it.
                let Some((app, _)) = state
                    .display_entries()
                    .into_iter()
                    .find(|(app, _)| app.match_id == match_id)
                else {
                    return false;
                };
                self.update_bookmarks(|bookmarks| {
                    if !bookmarks.iter().any(|b| {
                        b.desktop_id
                            .strip_suffix(".desktop")
                            .unwrap_or(&b.desktop_id)
                            == match_id
                    }) {
                        bookmarks.push(crate::config::DockBookmark {
                            desktop_id: match_id.clone(),
                            label: None,
                            exec_args: Vec::new(),
                        });
                    }
                });
                state.launchers.push(app);
                *self.state.write().unwrap() = state.clone();
                // Promotion moves the icon out of the running section and into
                // the launcher one, which is a change of order like any other.
                self.reorder_app_layers();
                state.launchers.len() - 1
            }
        };

        let app_layers = self.app_layers.read().unwrap();
        let Some(entry) = app_layers.get(&match_id) else {
            return false;
        };
        let slot = entry.layer.clone();
        let scaler = entry.icon_scaler.clone();
        drop(app_layers);

        let Some(icon_stack) = self.get_icon_stack_for_app(&match_id) else {
            return false;
        };

        // Every slot has to be the same width for the drag to be a matter of
        // counting pitches, so the magnification stands down for the duration.
        self.set_active_label(None);
        self.magnify_elements_with_scale(Some(0.0), Some(Transition::spring(0.2, 0.1)));

        let ghost = self.layers_engine.new_layer();
        let ghost_tree = LayerTreeBuilder::default()
            .key(format!("dock_drag_ghost_{match_id}"))
            .layout_style(taffy::Style {
                position: taffy::Position::Absolute,
                ..Default::default()
            })
            .size(Size::points(BASE_ICON_SIZE, BASE_ICON_SIZE))
            .anchor_point(layers::types::Point::new(0.5, 0.5))
            .replicate_node(Some(icon_stack.id()))
            .picture_cached(true)
            .image_cache(true)
            .pointer_events(false)
            .build()
            .unwrap();
        ghost.build_layer_tree(&ghost_tree);
        // The ghost stands in for the icon that was just lifted out of the
        // strip, so it carries the same tint the strip does.
        ghost.set_color_filter(icon_color_filter());
        let _ = self.drag_overlay.add_sublayer(&ghost);

        // The ghost takes the pointer with it from the first frame, and keeps
        // the scale the icon had so the lift itself is not a jump.
        let pitch = self.slot_pitch();
        let settled_scale = (pitch * ICON_SCALER_FILL) / BASE_ICON_SIZE;
        let current_scale = scaler.scale();
        ghost.set_position(self.ghost_point(drag.grab_px, pitch), None);
        ghost.set_scale(current_scale, None);
        ghost.set_scale(
            Point::new(settled_scale * 1.1, settled_scale * 1.1),
            Some(Transition::ease_out_quad(0.15)),
        );

        // The slot keeps its place in the layout — it is the gap the icon
        // leaves behind — but shows nothing while the ghost stands in for it.
        slot.set_opacity(0.0_f32, None);

        drag.active = true;
        drag.start_index = index;
        drag.index = index;
        drag.launchers = state.launchers.len();
        drag.pitch = pitch;
        drag.ghost = Some(ghost);
        drag.ghost_scale = settled_scale;
        self.dragging
            .store(true, std::sync::atomic::Ordering::SeqCst);
        true
    }

    /// Move the dragged icon to `new_index` in the launcher list and slide every
    /// icon it displaced one slot the other way.
    fn move_dragged_icon(&self, drag: &mut IconDrag, new_index: usize) {
        let old_index = drag.index;
        if new_index == old_index {
            return;
        }
        let mut state = self.get_state();
        if old_index >= state.launchers.len() || new_index >= state.launchers.len() {
            return;
        }
        let moved: Vec<String> = if new_index > old_index {
            state.launchers[old_index + 1..=new_index].iter()
        } else {
            state.launchers[new_index..old_index].iter()
        }
        .map(|app| app.match_id.clone())
        .collect();

        let app = state.launchers.remove(old_index);
        state.launchers.insert(new_index, app);
        *self.state.write().unwrap() = state;
        self.reorder_app_layers();
        drag.index = new_index;

        // The displaced icons have just been re-laid-out one slot along. Put
        // them back where they were and let them slide into the new place, or
        // the reorder reads as a jump.
        let shift = if new_index > old_index {
            drag.pitch
        } else {
            -drag.pitch
        };
        let app_layers = self.app_layers.read().unwrap();
        for match_id in moved {
            if let Some(entry) = app_layers.get(&match_id) {
                entry.layer.set_position(self.along_axis(shift), None);
                entry
                    .layer
                    .set_position(Point::new(0.0, 0.0), Some(Transition::ease_out_quad(0.16)));
            }
        }
    }

    /// Finish an icon drag: fly the ghost into the slot it landed on, hand the
    /// icon back to that slot and persist the new order. Returns whether a drag
    /// was in flight, so the caller can swallow the click that ends it.
    pub(super) fn end_icon_drag(&self) -> bool {
        let Some(drag) = self.icon_drag.write().unwrap().take() else {
            return false;
        };
        if !drag.active {
            return false;
        }
        self.dragging
            .store(false, std::sync::atomic::Ordering::SeqCst);

        let slot = self
            .app_layers
            .read()
            .unwrap()
            .get(&drag.match_id)
            .map(|entry| entry.layer.clone());

        if let Some(ghost) = drag.ghost {
            // The dock has been flat and still for the length of the drag, so
            // the slot the icon landed in can simply be measured.
            let target = match slot.as_ref() {
                Some(slot) => {
                    let bounds = slot.render_bounds_transformed();
                    let centre = if self.position().is_vertical() {
                        (bounds.top + bounds.bottom) / 2.0
                    } else {
                        (bounds.left + bounds.right) / 2.0
                    };
                    self.ghost_point(centre, drag.pitch)
                }
                None => ghost.position(),
            };
            let animation = self
                .layers_engine
                .add_animation_from_transition(&Transition::ease_out_quad(0.18), false);
            let changes = [
                ghost.change_position(target),
                ghost.change_scale(Point::new(drag.ghost_scale, drag.ghost_scale)),
            ];
            self.layers_engine.schedule_changes(&changes, animation);
            // Animation-scoped, not transaction-scoped: a transaction handler is
            // dropped if anything writes the same value before it ends, and the
            // ghost has to be taken down whatever happens.
            self.layers_engine.on_animation_finish(
                animation,
                move |_| {
                    ghost.remove();
                    if let Some(slot) = slot.as_ref() {
                        slot.set_opacity(1.0_f32, None);
                    }
                },
                true,
            );
            self.layers_engine.start_animation(animation, 0.0);
        } else if let Some(slot) = slot {
            slot.set_opacity(1.0_f32, None);
        }

        if drag.index != drag.start_index {
            self.persist_launcher_order();
        }
        // The pointer is still over the dock, so pick the magnification back up.
        self.magnify_elements_animated();
        true
    }

    /// Drop a press that never became a drag.
    pub(super) fn cancel_icon_drag(&self) {
        *self.icon_drag.write().unwrap() = None;
    }

    /// Re-append every app slot in display order. Slots are laid out and painted
    /// in the order they are attached to the container, and both the
    /// magnification (which finds an icon's centre from its index) and the drag
    /// assume that order is the one [`DockModel::display_entries`] gives.
    fn reorder_app_layers(&self) {
        let entries = self.get_state().display_entries();
        let app_layers = self.app_layers.read().unwrap();
        for (app, _) in entries.iter() {
            if let Some(entry) = app_layers.get(&app.match_id) {
                let _ = self.dock_apps_container.add_sublayer(&entry.layer);
            }
        }
    }

    /// Write the launcher order back to the bookmark list. Bookmarks the dock
    /// does not know about — one whose desktop entry failed to load, say — keep
    /// their relative order at the end rather than being dropped.
    fn persist_launcher_order(&self) {
        let order: Vec<String> = self
            .get_state()
            .launchers
            .iter()
            .map(|app| app.match_id.clone())
            .collect();
        self.update_bookmarks(|bookmarks| super::model::sort_bookmarks_to(bookmarks, &order));
    }

    /// Schedule hiding the dock after a short delay (if autohide is enabled).
    /// The delay is handled by the animation itself; a subsequent show() call
    /// overrides the pending animation and cancels the hide naturally.
    pub fn schedule_autohide(&self) {
        if !self.is_autohide_enabled() || self.is_hidden() || self.has_menu_open() {
            return;
        }
        self.active
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.view_layer
            .set_position(
                self.slide_position(250.0),
                Some(Transition {
                    timing: TimingFunction::Spring(Spring::with_duration_and_bounce(0.5, 0.0)),
                    delay: 0.4,
                }),
            )
            .on_finish(
                |l: &Layer, _| {
                    l.set_hidden(true);
                },
                true,
            );
    }

    /// Show the dock (used from the hot-zone when autohide is on).
    ///
    /// Returns `Some(TransactionRef)` when the dock was hidden and an animation
    /// was started, so callers can chain work via `on_finish`. Returns `None` if
    /// autohide is off or the dock is already visible.
    pub fn show_autohide(&self) -> Option<TransactionRef> {
        if !self.is_autohide_enabled() {
            return None;
        }
        if self.active.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        self.view_layer.set_hidden(false);
        tracing::debug!("dock: show (override pending hide)");
        self.active
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Some(self.view_layer.set_position(
            (0.0, 0.0),
            Some(Transition {
                timing: TimingFunction::Spring(Spring::with_duration_and_bounce(0.5, 0.2)),
                delay: 0.0,
            }),
        ))
    }

    /// Hide the context menu and immediately re-run magnification so the dock
    /// resizes to the current pointer position.
    pub fn close_context_menu(&self) {
        let menu_lock = self.context_menu.read().unwrap();
        if let Some(menu) = menu_lock.as_ref() {
            menu.hide();
        }
        drop(menu_lock);

        // Restore the pressed icon to its normal appearance.
        if let Some(app_id) = self.context_menu_app_id.write().unwrap().take() {
            self.set_app_context_menu_active(&app_id, false);
        }

        // menu.hide() sets is_active() to false, so the guard in
        // update_magnification_position will pass and the dock will resize.
        let pos = *self.magnification_position.read().unwrap();
        self.update_magnification_position(pos);
    }
}

// Dock view observer
impl Observer<WorkspacesModel> for DockView {
    fn notify(&self, event: &WorkspacesModel) {
        let _ = self.notify_tx.try_send(event.clone());
    }
}

// https://www.wolframalpha.com/input?i=plot+e%5E%28-8*x%5E2%29
use std::f64::consts::E;
pub fn magnify_function(x: impl Into<f64>, genie_span: f64) -> f64 {
    let x = x.into();
    E.powf(-genie_span * x.powi(2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspaces::app_icons_manager::AppIconsManager;
    use serial_test::serial;

    /// A dock with two launchers, laid out on a 1000×1000 screen at the given
    /// edge. The tokio runtime is kept alive by the caller: `DockView::new`
    /// spawns its notification tasks on it.
    fn dock_at(position: DockPosition) -> (Arc<Engine>, DockView) {
        dock_at_with_magnification(position, true)
    }

    fn dock_at_with_magnification(
        position: DockPosition,
        magnification: bool,
    ) -> (Arc<Engine>, DockView) {
        Config::update(|c| {
            c.dock.position = position;
            c.dock.bookmarks.clear();
            c.dock.magnification = magnification;
            c.screen_scale = 1.0;
        });
        let engine = Engine::create(1000.0, 1000.0);
        let root = engine.new_layer();
        root.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        root.set_size(Size::points(1000.0, 1000.0), None);
        let _ = engine.add_layer(&root);

        let icons = Arc::new(AppIconsManager::new(engine.clone()));
        let dock = DockView::new(engine.clone(), icons);
        let _ = root.add_sublayer(&dock.wrap_layer);
        dock.set_screen_size(1000, 1000, (1000, 1000));

        let mut state = dock.get_state();
        state.launchers = vec![
            Application::test_new("calculator"),
            Application::test_new("editor"),
        ];
        dock.update_state(&state);
        // A fresh dock is parked off screen until it is shown; slide it in, or
        // there is nothing to paint.
        dock.show(None);
        settle(&engine);
        (engine, dock)
    }

    /// Run the engine until the layout animations land. Dock geometry is
    /// applied through spring transitions — a single `update(0.0)` leaves every
    /// icon slot at the size it was built with, which is not what the dock
    /// looks like a moment later.
    fn settle(engine: &Arc<Engine>) {
        for _ in 0..300 {
            engine.update(0.016);
        }
    }

    /// (slot bounds, balloon bounds) for the first app in the dock. The balloon
    /// is the shape drawn inside the label layer — the layer itself is padded
    /// by a safe margin all around, so its bounds say nothing about placement.
    fn first_label_geometry(dock: &DockView) -> (skia::Rect, skia::Rect) {
        let entry = first_entry(dock);
        (
            entry.0.render_bounds_transformed(),
            entry.1.render_layer().global_shape_bounds,
        )
    }

    /// (slot layer, label layer, dot layer) of the first app in the dock.
    fn first_entry(dock: &DockView) -> (Layer, Layer, Layer) {
        let app_layers = dock.app_layers.read().unwrap();
        // By name, not by iteration order: the entries live in a HashMap, and
        // two docks holding the same apps would otherwise be compared through
        // two different icons.
        let entry = app_layers
            .get("calculator")
            .expect("the dock should have laid out its launchers");
        (
            entry.layer.clone(),
            entry.label_layer.clone(),
            entry.dot_layer.clone(),
        )
    }

    /// Where the balloon sits relative to its icon slot.
    #[derive(Debug, PartialEq, Eq)]
    enum Side {
        Above,
        Left,
        Right,
        Elsewhere,
    }

    fn side_of(slot: skia::Rect, balloon: skia::Rect) -> Side {
        let centred_x = (balloon.center_x() - slot.center_x()).abs() < 2.0;
        let centred_y = (balloon.center_y() - slot.center_y()).abs() < 2.0;
        if balloon.bottom <= slot.y() + 2.0 && centred_x {
            Side::Above
        } else if balloon.right <= slot.x() + 2.0 && centred_y {
            Side::Left
        } else if balloon.x() >= slot.right - 2.0 && centred_y {
            Side::Right
        } else {
            Side::Elsewhere
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// The match ids of the app slots in the order they are attached to the
    /// apps container — which is the order they are laid out and painted in.
    fn slot_order(engine: &Arc<Engine>, dock: &DockView) -> Vec<String> {
        let app_layers = dock.app_layers.read().unwrap();
        engine
            .node_children(&dock.dock_apps_container.id())
            .into_iter()
            .filter_map(|child| {
                app_layers
                    .iter()
                    .find(|(_, entry)| entry.layer.id() == child)
                    .map(|(match_id, _)| match_id.clone())
            })
            .collect()
    }

    /// A bottom dock holding `apps` as launchers, settled.
    fn dock_with_launchers(apps: &[&str]) -> (Arc<Engine>, DockView) {
        dock_at_with_launchers(DockPosition::Bottom, apps)
    }

    /// A dock at `position` holding `apps` as launchers, settled.
    fn dock_at_with_launchers(position: DockPosition, apps: &[&str]) -> (Arc<Engine>, DockView) {
        let (engine, dock) = dock_at(position);
        let mut state = dock.get_state();
        state.launchers = apps.iter().map(|id| Application::test_new(id)).collect();
        dock.update_state(&state);
        settle(&engine);
        (engine, dock)
    }

    /// The strip's material and a label's balloon are set on their layers when
    /// the dock is built, not derived from its model, so nothing about a
    /// re-render moves them: switching the desktop from light to dark used to
    /// leave the dock painted in the light palette until the session was
    /// restarted.
    #[test]
    #[serial]
    fn the_dock_repaints_itself_in_the_new_colour_scheme() {
        let rt = runtime();
        let _guard = rt.enter();
        let _ = Config::update(|c| c.theme_scheme = crate::theme::ThemeScheme::Light);
        let (engine, dock) = dock_at_with_launchers(DockPosition::Bottom, &["calculator"]);

        let light_bar = dock.bar_layer.render_layer().background_color;
        let light_label = first_entry(&dock).1.render_layer().background_color;

        let _ = Config::update(|c| c.theme_scheme = crate::theme::ThemeScheme::Dark);
        dock.refresh_theme();
        settle(&engine);

        assert_ne!(
            dock.bar_layer.render_layer().background_color,
            light_bar,
            "the strip should be drawn in the dark palette's material"
        );
        assert_ne!(
            first_entry(&dock).1.render_layer().background_color,
            light_label,
            "a label balloon should be drawn in the dark palette's material"
        );

        let _ = Config::update(|c| c.theme_scheme = crate::theme::ThemeScheme::Light);
    }

    /// A place's slot lands in the places strip, not among the applications:
    /// past the divider is what says it is a location rather than an app.
    #[test]
    #[serial]
    fn a_place_is_drawn_in_its_own_strip() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) = dock_at_with_launchers(DockPosition::Bottom, &["calculator"]);

        let mut state = dock.get_state();
        state.places = vec![Application::test_new(&trash_match_id())];
        dock.update_state(&state);
        settle(&engine);

        let slot = {
            let layers = dock.app_layers.read().unwrap();
            layers
                .get(&trash_match_id())
                .expect("the place should have been laid out")
                .layer
                .clone()
        };
        let places = dock.dock_places_container.render_bounds_transformed();
        let apps = dock.dock_apps_container.render_bounds_transformed();
        let slot = slot.render_bounds_transformed();

        assert!(
            slot.left >= places.left && slot.right <= places.right,
            "the place's slot is outside the places strip: {slot:?} vs {places:?}"
        );
        assert!(
            slot.left >= apps.right,
            "the places strip must come after the applications"
        );
    }

    /// With the place's window open the dock shows one icon, in the places
    /// strip — not a second one appended to the running applications.
    #[test]
    #[serial]
    fn an_open_place_is_not_also_a_running_app() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) = dock_at_with_launchers(DockPosition::Bottom, &["calculator"]);

        let mut state = dock.get_state();
        state.places = vec![Application::test_new(&trash_match_id())];
        state.running_apps = vec![Application::test_new(&trash_match_id())];
        dock.update_state(&state);
        settle(&engine);

        let apps = dock.display_entries(&dock.get_state());
        assert!(
            !apps.iter().any(|(app, _)| app.match_id == trash_match_id()),
            "the open place was appended to the applications as well"
        );
        let places = dock.display_places(&dock.get_state());
        assert_eq!(places.len(), 1);
        assert!(places[0].1, "an open place should show its running dot");
    }

    /// The places strip is exactly as thick as the apps strip, and its slots
    /// sit on the same line: two strips of the same icons that do not line up
    /// read as the icons jiggling as the dock re-renders.
    #[test]
    #[serial]
    fn a_place_sits_on_the_same_line_as_the_applications() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) = dock_at_with_launchers(DockPosition::Bottom, &["calculator"]);

        let mut state = dock.get_state();
        state.places = vec![Application::test_new(&trash_match_id())];
        dock.update_state(&state);
        settle(&engine);

        let apps = dock.dock_apps_container.render_bounds_transformed();
        let places = dock.dock_places_container.render_bounds_transformed();
        assert_eq!(
            (places.top, places.bottom),
            (apps.top, apps.bottom),
            "the places strip is not on the applications' line"
        );

        // And with magnification off, where the strips are sized by the
        // render alone: a second writer there sized the places strip without
        // its slots' dot area, and the icon hopped every time the pointer
        // moved over the dock.
        let (engine, dock) = dock_at_with_magnification(DockPosition::Bottom, false);
        let mut state = dock.get_state();
        state.launchers = vec![Application::test_new("calculator")];
        state.places = vec![Application::test_new(&trash_match_id())];
        dock.update_state(&state);
        settle(&engine);

        let apps = dock.dock_apps_container.render_bounds_transformed();
        let places = dock.dock_places_container.render_bounds_transformed();
        let slot = |id: &str| {
            dock.app_layers
                .read()
                .unwrap()
                .get(id)
                .expect("slot")
                .layer
                .render_bounds_transformed()
        };
        eprintln!("apps {apps:?}\nplaces {places:?}");
        eprintln!(
            "calc {:?}\ntrash {:?}",
            slot("calculator"),
            slot(&trash_match_id())
        );
        assert_eq!(
            (places.top, places.bottom),
            (apps.top, apps.bottom),
            "unmagnified, the places strip is not on the applications' line"
        );
    }

    /// With the pointer nowhere near the dock — where it is when the session
    /// starts — every icon is the same size. The mapping of the pointer onto
    /// the strips used to clamp a pointer that was still left of the dock to
    /// the very start of the first strip, which magnified the first icon as
    /// though the pointer were sitting on it.
    #[test]
    #[serial]
    fn at_rest_no_icon_is_magnified() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) =
            dock_at_with_launchers(DockPosition::Bottom, &["calculator", "files", "term"]);
        let mut state = dock.get_state();
        state.places = vec![Application::test_new(&trash_match_id())];
        dock.update_state(&state);
        dock.magnify_elements();
        settle(&engine);

        let height = |id: &str| {
            dock.app_layers
                .read()
                .unwrap()
                .get(id)
                .expect("slot")
                .layer
                .render_bounds_transformed()
                .height()
        };
        let trash = trash_match_id();
        let sizes: Vec<f32> = ["calculator", "files", "term", trash.as_str()]
            .iter()
            .map(|id| height(id))
            .collect();
        assert!(
            sizes.windows(2).all(|pair| pair[0] == pair[1]),
            "an icon is magnified with the pointer off the dock: {sizes:?}"
        );
    }

    /// While the dock is being resized the places strip stays on the
    /// applications' line, frame by frame — not only once everything settles.
    /// A strip whose own size has stopped changing falls back to its content,
    /// and a slot's content includes the label balloon, which is half again as
    /// tall as the icon: the trash icon dropped to the bottom of a strip as
    /// tall as the whole dock and bobbed back up on every render.
    #[test]
    #[serial]
    fn the_places_strip_keeps_the_line_while_the_dock_resizes() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) =
            dock_at_with_launchers(DockPosition::Bottom, &["calculator", "files", "term"]);
        let mut state = dock.get_state();
        state.places = vec![Application::test_new(&trash_match_id())];
        dock.update_state(&state);
        settle(&engine);

        let slot = |id: &str| {
            dock.app_layers
                .read()
                .unwrap()
                .get(id)
                .expect("slot")
                .layer
                .render_bounds_transformed()
        };

        for step in 1..=8 {
            Config::update(|c| c.dock.size = 1.0 + step as f64 * 0.05);
            dock.render_dock();
            for frame in 0..6 {
                engine.update(0.016);
                let app = slot("term");
                let place = slot(&trash_match_id());
                assert!(
                    (place.bottom - app.bottom).abs() <= 2.0,
                    "step {step} frame {frame}: the place is {:.1}px off the applications' line",
                    place.bottom - app.bottom
                );
                // The minimized-window strip is thickened by the same pass and
                // bobbed the same way; it is checked as a strip because its
                // slots need real windows to exist.
                let apps_thickness = dock.dock_apps_container.render_size().y;
                let windows_thickness = dock.dock_windows_container.render_size().y;
                assert!(
                    (windows_thickness - apps_thickness).abs() <= 2.0,
                    "step {step} frame {frame}: the minimized-window strip is {:.1}px thicker than the applications'",
                    windows_thickness - apps_thickness
                );
            }
        }
    }

    /// The wastebasket is a place like any other, named by `[dock]
    /// trash_desktop_id`: point that at another file manager's desktop entry
    /// and the icon that follows the can is that entry's, along with the
    /// command a click runs and the actions in its menu.
    #[test]
    #[serial]
    fn which_place_is_the_wastebasket_is_configurable() {
        assert_eq!(trash_match_id(), "otto-trash");

        Config::update(|c| c.dock.trash_desktop_id = "org.gnome.Nautilus.desktop".to_string());
        assert_eq!(trash_match_id(), "org.gnome.Nautilus");

        // Written without the suffix too, since that is how a `match_id` reads.
        Config::update(|c| c.dock.trash_desktop_id = "thunar".to_string());
        assert_eq!(trash_match_id(), "thunar");

        Config::update(|c| c.dock.trash_desktop_id = "otto-trash.desktop".to_string());
    }

    /// An empty places strip takes no room: a stub past the divider would read
    /// as a gap in a dock that has no places at all.
    #[test]
    #[serial]
    fn an_empty_places_strip_takes_no_room() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) = dock_at_with_launchers(DockPosition::Bottom, &["calculator"]);
        settle(&engine);

        let places = dock.dock_places_container.render_bounds_transformed();
        assert_eq!(places.width(), 0.0, "an empty places strip reserved width");
    }

    fn launcher_order(dock: &DockView) -> Vec<String> {
        dock.get_state()
            .launchers
            .iter()
            .map(|app| app.match_id.clone())
            .collect()
    }

    /// Where the pointer has to be, in logical coordinates, to have grabbed the
    /// icon at `index` and dragged it by `slots` places.
    fn drag_to(dock: &DockView, slots: f32) -> (f64, f64) {
        let scale = Config::with(|c| c.screen_scale);
        ((dock.slot_pitch() * slots) as f64 / scale, 0.0)
    }

    /// The centre of the dragged icon, and the centre of the pointer that is
    /// dragging it, in scene coordinates.
    fn ghost_and_pointer(dock: &DockView, drag: &IconDrag, pointer: (f64, f64)) -> (Point, Point) {
        let ghost = drag.ghost.as_ref().unwrap().render_bounds_transformed();
        let icons = dock.dock_apps_container.render_bounds_transformed();
        let scale = Config::with(|c| c.screen_scale) as f32;
        let pitch = drag.pitch;
        // The pointer only carries the icon along the dock; across it the icon
        // stays in the row, which is a half pitch in from the far edge.
        let expected = match dock.position() {
            DockPosition::Bottom => Point::new(pointer.0 as f32 * scale, icons.top + pitch / 2.0),
            DockPosition::Left => Point::new(icons.right - pitch / 2.0, pointer.1 as f32 * scale),
            DockPosition::Right => Point::new(icons.left + pitch / 2.0, pointer.1 as f32 * scale),
        };
        (
            Point::new(
                (ghost.left + ghost.right) / 2.0,
                (ghost.top + ghost.bottom) / 2.0,
            ),
            expected,
        )
    }

    #[test]
    #[serial]
    fn the_dragged_icon_stays_under_the_pointer_through_the_magnification() {
        let rt = runtime();
        let _guard = rt.enter();
        for position in [
            DockPosition::Bottom,
            DockPosition::Left,
            DockPosition::Right,
        ] {
            let (engine, dock) =
                dock_at_with_launchers(position, &["calculator", "editor", "files"]);

            // Magnified under the pointer, which is how the dock looks when a
            // press lands on it — and not how it looks a moment later, once the
            // drag has flattened it.
            let scaler = {
                let app_layers = dock.app_layers.read().unwrap();
                app_layers.get("calculator").unwrap().icon_scaler.clone()
            };
            let icon = scaler.render_bounds_transformed();
            let vertical = position.is_vertical();
            let grab_along = if vertical {
                (icon.top + icon.bottom) / 2.0
            } else {
                (icon.left + icon.right) / 2.0
            };
            dock.update_magnification_position(grab_along);
            settle(&engine);

            let pointer = |along: f32| -> (f64, f64) {
                if vertical {
                    (0.0, along as f64)
                } else {
                    (along as f64, 0.0)
                }
            };

            dock.begin_icon_drag("calculator", pointer(grab_along));
            let moved = grab_along + dock.slot_pitch();
            assert!(dock.icon_drag_update(pointer(moved)));

            // The frame the drag starts on, with the dock still magnified...
            engine.update(0.0);
            let drag = dock.icon_drag.read().unwrap().clone().unwrap();
            let (ghost, expected) = ghost_and_pointer(&dock, &drag, pointer(moved));
            assert!(
                (ghost.x - expected.x).abs() < 2.0 && (ghost.y - expected.y).abs() < 2.0,
                "{position:?}: the icon must ride the pointer from the first frame, \
                 not hang off the magnified dock — {ghost:?} against {expected:?}"
            );

            // ...and once the dock has settled flat underneath it.
            settle(&engine);
            assert!(dock.icon_drag_update(pointer(moved)));
            engine.update(0.0);
            let drag = dock.icon_drag.read().unwrap().clone().unwrap();
            let (ghost, expected) = ghost_and_pointer(&dock, &drag, pointer(moved));
            assert!(
                (ghost.x - expected.x).abs() < 2.0 && (ghost.y - expected.y).abs() < 2.0,
                "{position:?}: the icon must stay on the pointer once the dock is \
                 flat — {ghost:?} against {expected:?}"
            );
        }
    }

    #[test]
    #[serial]
    fn a_short_press_is_a_click_not_a_drag() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) = dock_with_launchers(&["calculator", "editor", "files"]);

        dock.begin_icon_drag("calculator", (0.0, 0.0));
        // Well inside the threshold: a hand that wobbles is still clicking.
        assert!(!dock.icon_drag_update((2.0, 0.0)), "a wobble is not a drag");
        assert!(!dock.is_icon_dragging());
        assert!(!dock.end_icon_drag(), "the click must not be swallowed");
        settle(&engine);
        assert_eq!(launcher_order(&dock), ["calculator", "editor", "files"]);
    }

    #[test]
    #[serial]
    fn dragging_an_icon_past_its_neighbour_swaps_them() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) = dock_with_launchers(&["calculator", "editor", "files"]);

        dock.begin_icon_drag("calculator", (0.0, 0.0));
        assert!(dock.icon_drag_update(drag_to(&dock, 1.0)));
        assert!(dock.is_icon_dragging());

        assert_eq!(launcher_order(&dock), ["editor", "calculator", "files"]);
        assert_eq!(
            slot_order(&engine, &dock),
            ["editor", "calculator", "files"],
            "the slots have to be laid out in the order the model now has"
        );
    }

    #[test]
    #[serial]
    fn half_a_slot_is_not_enough_to_swap() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) = dock_with_launchers(&["calculator", "editor", "files"]);

        dock.begin_icon_drag("calculator", (0.0, 0.0));
        // Just short of the half-way point between two slots.
        assert!(dock.icon_drag_update(drag_to(&dock, 0.45)));
        assert_eq!(launcher_order(&dock), ["calculator", "editor", "files"]);

        assert!(dock.icon_drag_update(drag_to(&dock, 0.55)));
        assert_eq!(launcher_order(&dock), ["editor", "calculator", "files"]);
        settle(&engine);
    }

    #[test]
    #[serial]
    fn a_drag_carries_the_icon_all_the_way_across() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) = dock_with_launchers(&["calculator", "editor", "files"]);

        dock.begin_icon_drag("calculator", (0.0, 0.0));
        assert!(dock.icon_drag_update(drag_to(&dock, 2.0)));
        assert_eq!(launcher_order(&dock), ["editor", "files", "calculator"]);

        // And back again, in one move: the icons in between shift the other way.
        assert!(dock.icon_drag_update(drag_to(&dock, 0.0)));
        assert_eq!(launcher_order(&dock), ["calculator", "editor", "files"]);
        assert_eq!(
            slot_order(&engine, &dock),
            ["calculator", "editor", "files"]
        );
    }

    #[test]
    #[serial]
    fn a_drag_cannot_push_an_icon_off_the_end() {
        let rt = runtime();
        let _guard = rt.enter();
        let (_engine, dock) = dock_with_launchers(&["calculator", "editor"]);

        dock.begin_icon_drag("editor", (0.0, 0.0));
        assert!(dock.icon_drag_update(drag_to(&dock, 12.0)));
        assert_eq!(
            launcher_order(&dock),
            ["calculator", "editor"],
            "the last icon has nowhere further to go"
        );
    }

    #[test]
    #[serial]
    fn the_dragged_icon_is_lifted_out_of_its_slot() {
        let rt = runtime();
        let _guard = rt.enter();
        let (engine, dock) = dock_with_launchers(&["calculator", "editor", "files"]);

        let slot = {
            let app_layers = dock.app_layers.read().unwrap();
            app_layers.get("calculator").unwrap().layer.clone()
        };

        dock.begin_icon_drag("calculator", (0.0, 0.0));
        assert!(dock.icon_drag_update(drag_to(&dock, 1.0)));
        settle(&engine);
        assert_eq!(
            slot.opacity(),
            0.0,
            "the slot stands empty while the ghost has the icon"
        );
        assert!(
            dock.icon_drag
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .ghost
                .is_some(),
            "the icon has to be somewhere while its slot is empty"
        );
    }

    #[test]
    #[serial]
    fn tooltips_follow_the_edge_the_dock_starts_on() {
        let rt = runtime();
        let _guard = rt.enter();
        for (position, expected) in [
            (DockPosition::Bottom, Side::Above),
            (DockPosition::Left, Side::Right),
            (DockPosition::Right, Side::Left),
        ] {
            let (_engine, dock) = dock_at(position);
            let (slot, balloon) = first_label_geometry(&dock);
            assert_eq!(
                side_of(slot, balloon),
                expected,
                "{position:?} dock: balloon {balloon:?} against slot {slot:?}"
            );
        }
    }

    #[test]
    #[serial]
    fn tooltips_follow_the_dock_when_it_moves() {
        let rt = runtime();
        let _guard = rt.enter();
        for (from, to, expected) in [
            (DockPosition::Bottom, DockPosition::Right, Side::Left),
            (DockPosition::Bottom, DockPosition::Left, Side::Right),
            (DockPosition::Right, DockPosition::Bottom, Side::Above),
            (DockPosition::Left, DockPosition::Bottom, Side::Above),
            (DockPosition::Left, DockPosition::Right, Side::Left),
        ] {
            let (engine, dock) = dock_at(from);
            Config::update(|c| c.dock.position = to);
            dock.apply_dock_position();
            settle(&engine);

            let (slot, balloon) = first_label_geometry(&dock);
            assert_eq!(
                side_of(slot, balloon),
                expected,
                "{from:?} -> {to:?}: balloon {balloon:?} against slot {slot:?}"
            );

            // The layer that carries the balloon has to be rebuilt too, not
            // just the path drawn in it: its box is what the background blur
            // and the material fill are clipped to.
            let (_engine2, fresh) = dock_at(to);
            let moved_box = first_entry(&dock).1.render_bounds_transformed();
            let fresh_box = first_entry(&fresh).1.render_bounds_transformed();
            assert!(
                (moved_box.width() - fresh_box.width()).abs() < 1.0
                    && (moved_box.height() - fresh_box.height()).abs() < 1.0,
                "{from:?} -> {to:?}: moved label layer is {}x{}, a fresh one is {}x{}",
                moved_box.width(),
                moved_box.height(),
                fresh_box.width(),
                fresh_box.height()
            );
        }
    }

    /// The dock resolves the running applications on a task of its own, half a
    /// second behind the workspace change that caused them, so anything that
    /// has to agree with what the dock draws — the shell's accessible tree —
    /// has to hear about it when the dock applies it and not before. Without
    /// this the tree says an application that has just started is not running,
    /// and goes on saying so until something else changes the workspace.
    #[test]
    #[serial]
    fn applying_a_model_tells_the_dock_s_own_watchers() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Counter(Arc<AtomicUsize>);
        impl Observer<DockModel> for Counter {
            fn notify(&self, _event: &DockModel) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let rt = runtime();
        let _guard = rt.enter();
        let (_engine, dock) = dock_at(DockPosition::Bottom);

        let seen = Arc::new(AtomicUsize::new(0));
        let observer: Arc<dyn Observer<DockModel>> = Arc::new(Counter(seen.clone()));
        dock.add_model_listener(observer.clone());

        let before = seen.load(Ordering::Relaxed);
        let mut state = dock.get_state();
        state.running_apps = vec![Application::test_new("calculator")];
        dock.update_state(&state);
        assert_eq!(seen.load(Ordering::Relaxed), before + 1);

        // Weakly held: a watcher that has gone away must not keep the dock
        // calling into it.
        drop(observer);
        let after = seen.load(Ordering::Relaxed);
        dock.update_state(&state);
        assert_eq!(seen.load(Ordering::Relaxed), after);
    }

    /// An icon slot is a square icon plus the sliver the running dot lives in,
    /// laid along the dock: on a side dock it is wider than it is tall. This
    /// holds with magnification off too, where nothing re-sizes the slots after
    /// they are created.
    #[test]
    #[serial]
    fn icon_slots_are_laid_out_along_the_dock() {
        let rt = runtime();
        let _guard = rt.enter();
        for magnification in [true, false] {
            for position in [
                DockPosition::Bottom,
                DockPosition::Left,
                DockPosition::Right,
            ] {
                let (_engine, dock) = dock_at_with_magnification(position, magnification);
                let slot = first_entry(&dock).0.render_bounds_transformed();
                let along_the_dock = if position.is_vertical() {
                    slot.width() > slot.height()
                } else {
                    slot.height() > slot.width()
                };
                assert!(
                    along_the_dock,
                    "{position:?} dock (magnification {magnification}): slot is {}x{}, \
                     the dot sliver should thicken it across the dock, not along it",
                    slot.width(),
                    slot.height()
                );
            }
        }
    }

    /// Paint the scene into a raster surface: the tooltip is only wrong once it
    /// has been *drawn*, so the layer tree agreeing with itself is not enough.
    fn paint(engine: &Arc<Engine>) -> skia::Image {
        let mut surface = layers::skia::surfaces::raster_n32_premul((1000, 1000)).unwrap();
        let canvas = surface.canvas();
        canvas.clear(layers::skia::Color::from_argb(255, 40, 40, 40));
        if let Some(root) = engine.scene_root() {
            layers::drawing::draw_scene(canvas, engine.scene(), root);
        }
        surface.image_snapshot()
    }

    /// The pixels of `image` inside `area`, as RGBA rows.
    fn pixels_in(image: &skia::Image, area: skia::Rect) -> Vec<u8> {
        use layers::skia::RoundOut;
        let area: skia::Rect = {
            let r: skia::IRect = area.round_out();
            skia::Rect::from(r)
        };
        let (w, h) = (area.width() as i32, area.height() as i32);
        let info = layers::skia::ImageInfo::new(
            (w, h),
            layers::skia::ColorType::RGBA8888,
            layers::skia::AlphaType::Premul,
            None,
        );
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        assert!(
            image.read_pixels(
                &info,
                &mut pixels,
                (w * 4) as usize,
                (area.x() as i32, area.y() as i32),
                layers::skia::image::CachingHint::Allow,
            ),
            "could not read the painted pixels back"
        );
        pixels
    }

    /// How many of the two pixel buffers differ noticeably.
    fn differing_pixels(a: &[u8], b: &[u8]) -> usize {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(p, q)| {
                p.iter()
                    .zip(q.iter())
                    .any(|(x, y)| (*x as i16 - *y as i16).abs() > 8)
            })
            .count()
    }

    /// Hovering an icon shows its tooltip; this is what the dock does on hover.
    fn show_first_label(dock: &DockView, engine: &Arc<Engine>) {
        let label = first_entry(dock).1;
        dock.set_active_label(Some(label));
        settle(engine);
    }

    /// A tooltip that has already been painted at one edge must not be re-used
    /// with its old shape after the dock moves: the balloon painted after a
    /// move has to be pixel-for-pixel the one a dock that started at that edge
    /// paints. This is what a stale draw cache breaks — the layer tree is
    /// right, the placement is right, and the arrow still points the old way.
    #[test]
    #[serial]
    fn a_moved_tooltip_paints_the_same_as_one_that_never_moved() {
        let rt = runtime();
        let _guard = rt.enter();
        for (from, to) in [
            (DockPosition::Bottom, DockPosition::Right),
            (DockPosition::Bottom, DockPosition::Left),
            (DockPosition::Right, DockPosition::Bottom),
            (DockPosition::Left, DockPosition::Bottom),
            (DockPosition::Left, DockPosition::Right),
        ] {
            let (engine, dock) = dock_at(from);
            // Show it once at the old edge, so it gets painted (and cached) there.
            show_first_label(&dock, &engine);
            let _ = paint(&engine);
            dock.set_active_label(None);
            settle(&engine);

            Config::update(|c| c.dock.position = to);
            dock.apply_dock_position();
            settle(&engine);
            show_first_label(&dock, &engine);
            let moved = paint(&engine);

            let (engine2, dock2) = dock_at(to);
            show_first_label(&dock2, &engine2);
            let fresh = paint(&engine2);

            // Compare the band the tooltip lives in. Both docks hold the same
            // apps on the same screen, so everything else in it — the bar, the
            // icon — is laid out identically.
            let mut area = first_entry(&dock).1.render_layer().global_shape_bounds;
            area.join(first_entry(&dock2).1.render_layer().global_shape_bounds);
            area.outset((20.0, 20.0));
            let differing = differing_pixels(&pixels_in(&moved, area), &pixels_in(&fresh, area));
            let total = (area.width().ceil() as usize + 2) * (area.height().ceil() as usize + 2);
            // Identical in practice; the allowance is only for antialiasing
            // noise, and a stale arrow is an order of magnitude more than this.
            assert!(
                differing * 1000 < total,
                "{from:?} -> {to:?}: {differing} of {total} pixels around the tooltip differ \
                 from the dock that started at {to:?}"
            );
        }
    }

    /// The balloon body comes from the palette's tooltip material, so it
    /// follows the scheme. Both arms of the tooltip draw used to hold the same
    /// hardcoded grey — the light material, copied verbatim into the dark one.
    #[test]
    #[serial]
    fn the_tooltip_body_follows_the_theme() {
        let rt = runtime();
        let _guard = rt.enter();
        let body_color = |scheme: crate::theme::ThemeScheme| {
            Config::update(|c| c.theme_scheme = scheme.clone());
            let (engine, dock) = dock_at(DockPosition::Bottom);
            show_first_label(&dock, &engine);
            let painted = first_entry(&dock).1.render_layer().background_color;
            let expected = layers::prelude::PaintColor::Solid {
                color: crate::theme::theme_colors().materials_controls_tooltip,
            };
            assert_eq!(painted, expected, "{scheme:?} tooltip is off the palette");
            painted
        };
        let light = body_color(crate::theme::ThemeScheme::Light);
        let dark = body_color(crate::theme::ThemeScheme::Dark);
        Config::update(|c| c.theme_scheme = crate::theme::ThemeScheme::Light);
        assert_ne!(light, dark, "both schemes painted the same tooltip");
    }

    /// The running indicator hugs the screen edge the dock sits on, whether the
    /// dock started there or was moved there.
    #[test]
    #[serial]
    fn the_running_dot_hugs_the_screen_edge() {
        let rt = runtime();
        let _guard = rt.enter();
        for (from, to) in [
            (DockPosition::Bottom, DockPosition::Bottom),
            (DockPosition::Bottom, DockPosition::Left),
            (DockPosition::Bottom, DockPosition::Right),
            (DockPosition::Right, DockPosition::Bottom),
            (DockPosition::Left, DockPosition::Right),
        ] {
            let (engine, dock) = dock_at(from);
            if to != from {
                Config::update(|c| c.dock.position = to);
                dock.apply_dock_position();
                settle(&engine);
            }
            let (slot_layer, _, dot_layer) = first_entry(&dock);
            let slot = slot_layer.render_bounds_transformed();
            let dot = dot_layer.render_bounds_transformed();
            let hugs = match to {
                // The dot sits under the icon on a bottom dock, and beside it
                // on a side dock — always on the side facing the screen edge.
                DockPosition::Bottom => {
                    dot.bottom >= slot.bottom - 1.0 && dot.width() >= slot.width() - 1.0
                }
                DockPosition::Left => {
                    dot.x() <= slot.x() + 1.0 && dot.height() >= slot.height() - 1.0
                }
                DockPosition::Right => {
                    dot.right >= slot.right - 1.0 && dot.height() >= slot.height() - 1.0
                }
            };
            assert!(
                hugs,
                "{from:?} -> {to:?}: dot {dot:?} does not hug the edge of slot {slot:?}"
            );
        }
    }
    /// An icon magnified under the pointer jumps proportionately. The hop is a
    /// distance in pixels, fixed when the launch began: a grown icon used to
    /// travel exactly as far as a small one, which next to a dock its own
    /// growth had made taller read as the jump shrinking.
    #[test]
    #[serial]
    fn a_magnified_icon_jumps_higher() {
        let rt = runtime();
        let _guard = rt.enter();

        Config::update(|c| c.dock.size = 1.0);
        // How far the first launcher's slot rises above its resting place over
        // one hop, with the pointer where `pointer` says.
        let amplitude = |pointer: Option<f32>| -> f32 {
            let (engine, dock) =
                dock_at_with_launchers(DockPosition::Bottom, &["calculator", "files", "term"]);
            let slot = || {
                dock.app_layers
                    .read()
                    .unwrap()
                    .get("calculator")
                    .expect("slot")
                    .layer
                    .render_bounds_transformed()
            };
            if let Some(pointer) = pointer {
                dock.update_magnification_position(pointer);
                dock.magnify_elements();
                settle(&engine);
            }
            let rest = slot().top;
            dock.start_bounce("calculator");
            let mut highest = rest;
            for _ in 0..60 {
                engine.update(0.016);
                highest = highest.min(slot().top);
            }
            rest - highest
        };

        let (_, dock) =
            dock_at_with_launchers(DockPosition::Bottom, &["calculator", "files", "term"]);
        let over_the_icon = {
            let layers = dock.app_layers.read().unwrap();
            layers
                .get("calculator")
                .expect("slot")
                .layer
                .render_bounds_transformed()
                .center_x()
        };
        drop(dock);

        let at_rest = amplitude(None);
        let magnified = amplitude(Some(over_the_icon));
        assert!(at_rest > 1.0, "the icon did not jump at all: {at_rest}");
        assert!(
            magnified > at_rest * 1.05,
            "the magnified icon jumped {magnified}, no further than the {at_rest} of an icon at rest"
        );
        // And not a great deal further: only a fraction of the magnification
        // reaches the jump, or the icon reads as being flung out of the dock.
        assert!(
            magnified < at_rest * 1.35,
            "the magnified icon jumped {magnified}, far past the {at_rest} of an icon at rest"
        );
    }

    /// With nothing magnified, the jump is exactly the height it always was.
    ///
    /// Scaling the hop by the icon's size is only ever allowed to *add* to it:
    /// a dock with magnification switched off, or a pointer nowhere near the
    /// one that is bouncing, jumps the plain two-thirds of an icon.
    #[test]
    #[serial]
    fn without_magnification_the_jump_is_the_plain_one() {
        let rt = runtime();
        let _guard = rt.enter();
        Config::update(|c| c.dock.size = 1.0);
        let (engine, dock) = dock_at_with_magnification(DockPosition::Bottom, false);
        let mut state = dock.get_state();
        state.launchers = vec![Application::test_new("calculator")];
        dock.update_state(&state);
        settle(&engine);

        let slot = || {
            dock.app_layers
                .read()
                .unwrap()
                .get("calculator")
                .expect("slot")
                .layer
                .render_bounds_transformed()
        };
        let rest = slot().top;
        dock.start_bounce("calculator");
        let mut highest = rest;
        for _ in 0..60 {
            engine.update(0.016);
            highest = highest.min(slot().top);
        }

        // The distance `start_bounce` asks for: two-thirds of an icon.
        let expected = dock.available_icon_size().0 * 0.7;
        let jumped = rest - highest;
        assert!(
            (jumped - expected).abs() < expected * 0.02,
            "an unmagnified icon jumped {jumped}, not the {expected} the dock asked for"
        );
    }

    /// The plane strip the dock renders into is sized from the dock's reach,
    /// so a big dock's launch bounce — a magnified icon, lifted, with its
    /// label open — stays inside the strip instead of being cropped mid-air.
    #[test]
    #[serial]
    fn plane_strip_covers_a_magnified_bouncing_icon() {
        let rt = runtime();
        let _guard = rt.enter();
        let (_engine, dock) = dock_at(DockPosition::Bottom);

        let small = {
            Config::update(|c| c.dock.size = 1.0);
            dock.plane_strip_thickness_px()
        };
        Config::update(|c| c.dock.size = 2.0);
        let big = dock.plane_strip_thickness_px();
        let (_, icon) = dock.available_icon_size();
        let genie = Config::with(|c| c.dock.genie_scale) as f32;

        // Fully magnified icon at the top of its hop, plus the label above it.
        let least = icon * (1.0 + genie) + icon * BOUNCE_HOP * BOUNCE_HOP_CEILING;
        assert!(
            big as f32 > least,
            "strip of {big}px cannot hold a {least}px magnified, bouncing icon"
        );
        assert!(
            big > small,
            "the strip must grow with the dock ({small} -> {big})"
        );
    }

    /// The divider stays between the strips it divides while the icons
    /// magnify under the pointer. The engine only re-reads a node's layout
    /// position when the node itself has changed, and nothing about the handle
    /// does: it stood still as the strips slid past it, overlapping the places
    /// icons by a dozen pixels before snapping back on some later render.
    #[test]
    #[serial]
    fn the_divider_keeps_up_with_the_magnifying_icons() {
        let rt = runtime();
        let _guard = rt.enter();
        // The icon size decides how much rounding there is between three
        // independently laid out rects, and it is global state another test
        // may have left somewhere else.
        Config::update(|c| c.dock.size = 1.0);
        let (engine, dock) =
            dock_at_with_launchers(DockPosition::Bottom, &["calculator", "files", "term"]);
        let mut state = dock.get_state();
        state.places = vec![Application::test_new(&trash_match_id())];
        dock.update_state(&state);
        settle(&engine);

        // A pointer sweeping along the dock, a frame at a time — the handle
        // was only ever caught out mid-sweep, which is why this cannot settle
        // between steps.
        for step in 0..40 {
            dock.update_magnification_position(380.0 + step as f32 * 8.0);
            dock.magnify_elements();
            engine.update(0.016);

            let apps = dock.dock_apps_container.render_bounds_transformed();
            let handle = dock.resize_handle.render_bounds_transformed();
            let places = dock.dock_places_container.render_bounds_transformed();
            // A pixel of slack for the rounding of three independently laid
            // out rects; the bug was worth more than ten.
            assert!(
                (handle.left - apps.right).abs() <= 1.0,
                "the handle left the applications behind: apps {apps:?} handle {handle:?}"
            );
            assert!(
                (places.left - handle.right).abs() <= 1.0,
                "the handle is out of step with the places strip: handle {handle:?} places {places:?}"
            );
        }

        // And as the icons shrink back after the pointer leaves the dock:
        // that animation is slow and has no pointer event of its own, so it is
        // the half the nudge at motion time cannot cover.
        dock.demagnify_elements();
        for _ in 0..30 {
            engine.update(0.016);
            let apps = dock.dock_apps_container.render_bounds_transformed();
            let handle = dock.resize_handle.render_bounds_transformed();
            let places = dock.dock_places_container.render_bounds_transformed();
            assert!(
                (handle.left - apps.right).abs() <= 1.0
                    && (places.left - handle.right).abs() <= 1.0,
                "the handle fell behind as the magnification settled: \
                 apps {apps:?} handle {handle:?} places {places:?}"
            );
        }
    }
}
