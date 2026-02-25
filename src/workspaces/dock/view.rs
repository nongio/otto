use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    sync::{atomic::AtomicBool, Arc, RwLock},
    time::Duration,
};

use layers::{
    engine::{animation::Transition, Engine, NodeRef, TransactionRef},
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
    config::{Config, DockBookmark},
    shell::WindowElement,
    theme::theme_colors,
    utils::{parse_hex_color, Observer},
    workspaces::{
        apps_info::ApplicationsInfo, utils::ContextMenuView, Application, WorkspacesModel,
    },
};

use super::{
    model::DockModel,
    render::{
        draw_app_icon, draw_badge, draw_progress, setup_app_icon, setup_badge_layer, setup_label,
        setup_miniwindow_icon, setup_progress_layer,
    },
};

pub const BASE_ICON_SIZE: f32 = 300.0;

#[derive(Debug, Clone)]
pub(super) struct AppLayerEntry {
    pub(super) layer: Layer,
    /// Icon scaler: fixed-size wrapper that applies a uniform scale to fit the magnified slot.
    pub(super) icon_scaler: Layer,
    /// Icon stack: contains the icon, badge, and progress layers
    pub(super) icon_stack: Layer,
    pub(super) icon_layer: Layer,
    /// Overlay layer showing the badge (red circle + number). Hidden when no badge is set.
    pub(super) badge_layer: Layer,
    /// Overlay layer showing the progress bar. Hidden when no progress is set.
    pub(super) progress_layer: Layer,
    pub(super) label_layer: Layer,
    pub(super) icon_id: Option<u32>,
    pub(super) running: bool,
    pub(super) identifier: String,
}

type MiniWindowLayers = (Layer, Layer, Layer, Option<u32>);

#[derive(Debug, Clone)]
pub struct DockView {
    layers_engine: Arc<Engine>,
    // layers
    pub wrap_layer: layers::prelude::Layer,
    pub view_layer: layers::prelude::Layer,
    pub bar_layer: layers::prelude::Layer,
    pub resize_handle: layers::prelude::Layer,
    dock_apps_container: layers::prelude::Layer,
    dock_windows_container: layers::prelude::Layer,

    pub(super) app_layers: Arc<RwLock<HashMap<String, AppLayerEntry>>>,
    miniwindow_layers: Arc<RwLock<HashMap<ObjectId, MiniWindowLayers>>>,
    state: Arc<RwLock<DockModel>>,
    active: Arc<AtomicBool>,
    notify_tx: tokio::sync::mpsc::Sender<WorkspacesModel>,
    latest_event: Arc<tokio::sync::RwLock<Option<WorkspacesModel>>>,
    magnification_position: Arc<RwLock<f32>>,
    pub dragging: Arc<AtomicBool>,

    pub context_menu: Arc<RwLock<Option<ContextMenuView>>>,
    /// The identifier of the app whose icon is currently showing the context-menu pressed state.
    pub(super) context_menu_app_id: Arc<RwLock<Option<String>>>,
    /// Runtime dock configuration — loaded at startup, kept in sync with the file on changes.
    /// This is the single source of truth for all dock settings and bookmarks.
    pub(super) dock_config: Arc<RwLock<crate::config::DockConfig>>,
    /// Runtime magnification toggle (mirrors config but can be changed without restart).
    magnification_enabled: Arc<AtomicBool>,
    /// Physical screen dimensions, kept in sync by the compositor via `set_screen_size`.
    screen_size: Arc<RwLock<(i32, i32)>>,
    /// Pre-computed autohide hot-zone rect, rebuilt by `render_dock` every time the dock
    /// layout changes. `check_dock_hot_zone` reads this without doing any computation.
    pub cached_hot_zone: Arc<RwLock<Option<skia::Rect>>>,
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
///         └── dock_windows_container `dock_windows_container`
///             ├── miniwindow
///             └── miniwindow
/// ```
///
///
impl DockView {
    /// Calculate dock bar height based on icon size
    /// Bar height = app container height + top padding + bottom padding
    fn calculate_bar_height(icon_size: f32, scale: f32) -> f32 {
        let padding_top = 8.0 * scale; // icon + running indicator padding
        let padding_bottom = 8.0 * scale; // top/bottom padding
        icon_size + padding_top + padding_bottom
    }

    pub fn new(layers_engine: Arc<Engine>) -> Self {
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = Config::with(|config| config.dock.size.clamp(0.5, 2.0)) as f32;
        let base_icon_size = 95.0;
        let scaled_icon_size = base_icon_size * dock_size_multiplier * draw_scale;

        let wrap_layer = layers_engine.new_layer();
        wrap_layer.set_key("dock");
        wrap_layer.set_pointer_events(false);
        wrap_layer.set_size(Size::percent(1.0, 1.0), None);
        wrap_layer.set_layout_style(Style {
            position: layers::taffy::style::Position::Absolute,
            display: layers::taffy::style::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center), // horizontal
            align_items: Some(taffy::AlignItems::FlexEnd),        // vertical alignment
            justify_items: Some(taffy::JustifyItems::Center),
            ..Default::default()
        });

        let view_layer = layers_engine.new_layer();

        wrap_layer.add_sublayer(&view_layer);
        // FIXME: initial dock position
        view_layer.set_position((0.0, 1000.0), None);
        let view_tree = LayerTreeBuilder::default()
            .key("dock-view")
            .size(Size::auto())
            .build()
            .unwrap();

        view_layer.build_layer_tree(&view_tree);

        let bar_layer = layers_engine.new_layer();
        view_layer.add_sublayer(&bar_layer);
        let initial_bar_height =
            Self::calculate_bar_height(scaled_icon_size, dock_size_multiplier * draw_scale);
        let bar_tree = LayerTreeBuilder::default()
            .key("dock-bar")
            .pointer_events(false)
            .size(Size {
                width: taffy::percent(1.0),
                height: taffy::Dimension::Length(initial_bar_height),
            })
            .blend_mode(BlendMode::BackgroundBlur)
            .background_color(theme_colors().materials_medium)
            .border_width((1.0 * draw_scale, None))
            .border_color(theme_colors().materials_highlight)
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
        view_layer.add_sublayer(&dock_apps_container);

        let container_tree = LayerTreeBuilder::default()
            .key("dock_app_container")
            .pointer_events(false)
            .size(Size::auto())
            .layout_style(taffy::Style {
                display: taffy::Display::Flex,
                justify_content: Some(taffy::JustifyContent::FlexEnd),
                justify_items: Some(taffy::JustifyItems::FlexEnd),
                align_items: Some(taffy::AlignItems::Baseline),
                gap: taffy::Size::<taffy::LengthPercentage>::from_length(0.0),
                min_size: taffy::Size {
                    width: taffy::Dimension::Length(20.0 * draw_scale),
                    height: taffy::Dimension::Length(0.0),
                },
                ..Default::default()
            })
            .build()
            .unwrap();
        dock_apps_container.build_layer_tree(&container_tree);

        let resize_handle = layers_engine.new_layer();
        view_layer.add_sublayer(&resize_handle);

        let handle_tree = LayerTreeBuilder::default()
            .pointer_events(true)
            .size(Size {
                width: taffy::Dimension::Length(scaled_icon_size * 0.4),
                height: taffy::Dimension::Length(initial_bar_height),
            })
            // .background_color(Color::new_rgba(0.0, 0.0, 0.0, 0.0     ))
            .content(Some(move |canvas: &skia::Canvas, w, h| {
                let paint = layers::skia::Paint::new(theme_colors().text_tertiary.c4f(), None);

                let line_width: f32 = 3.0 * draw_scale;
                let margin_h = (w - line_width) / 2.0;
                let margin_v = 18.0 * draw_scale * dock_size_multiplier;
                let rect = layers::skia::Rect::from_xywh(
                    margin_h,
                    margin_v,
                    w - 2.0 * margin_h,
                    h - 2.0 * margin_v,
                );
                let rrect = layers::skia::RRect::new_rect_xy(rect, 3.0, 3.0);
                canvas.draw_rrect(rrect, &paint);
                skia::Rect::from_xywh(0.0, 0.0, w, h)
            }))
            .build()
            .unwrap();
        resize_handle.build_layer_tree(&handle_tree);

        let dock_windows_container = layers_engine.new_layer();
        view_layer.add_sublayer(&dock_windows_container);

        let container_tree = LayerTreeBuilder::default()
            .key("dock_windows_container")
            .pointer_events(false)
            .position(Point::new(0.0, 0.0))
            .size(Size {
                width: taffy::Dimension::Auto,
                height: taffy::Dimension::Percent(1.0),
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
            dock_windows_container,
            app_layers: Arc::new(RwLock::new(HashMap::new())),
            miniwindow_layers: Arc::new(RwLock::new(HashMap::new())),
            state: Arc::new(RwLock::new(initial_state)),
            active: Arc::new(AtomicBool::new(true)),
            notify_tx,
            latest_event: Arc::new(tokio::sync::RwLock::new(None)),
            magnification_position: Arc::new(RwLock::new(-500.0)),
            dragging: Arc::new(AtomicBool::new(false)),
            context_menu: Arc::new(RwLock::new(None)),
            context_menu_app_id: Arc::new(RwLock::new(None)),
            dock_config: Arc::new(RwLock::new(Config::with(|c| c.dock.clone()))),
            magnification_enabled: Arc::new(AtomicBool::new(Config::with(|c| {
                c.dock.magnification
            }))),
            screen_size: Arc::new(RwLock::new((0, 0))),
            cached_hot_zone: Arc::new(RwLock::new(None)),
        };
        // Sync AtomicBool from dock_config (single source)
        dock.magnification_enabled.store(
            dock.dock_config.read().unwrap().magnification,
            std::sync::atomic::Ordering::SeqCst,
        );
        dock.render_dock();
        dock.notification_handler(notify_rx);
        dock.load_configured_bookmarks();

        dock
    }
    fn load_configured_bookmarks(&self) {
        let bookmarks = self.dock_config.read().unwrap().bookmarks.clone();
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
    }
    pub fn get_state(&self) -> DockModel {
        self.state.read().unwrap().clone()
    }
    pub fn is_hidden(&self) -> bool {
        !self.active.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn is_autohide_enabled(&self) -> bool {
        self.dock_config.read().unwrap().autohide
    }
    pub fn hide(&self, transition: Option<Transition>) -> TransactionRef {
        tracing::debug!("dock: hide");
        self.active
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.view_layer.set_position((0.0, 250.0), transition)
    }
    pub fn show(&self, transition: Option<Transition>) -> TransactionRef {
        if self.dock_config.read().unwrap().autohide {
            // When autohide is on, external show() calls should keep the dock hidden.
            // Mark active=false so is_hidden() returns true and the hot zone can trigger it.
            self.active
                .store(false, std::sync::atomic::Ordering::Relaxed);
            return self.view_layer.set_position((0.0, 250.0), None);
        }
        tracing::debug!("dock: show");
        self.active
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.view_layer.set_position((0.0, 0.0), transition)
    }
    fn display_entries(&self, state: &DockModel) -> Vec<(Application, bool)> {
        let mut entries: Vec<(Application, bool)> = state
            .launchers
            .iter()
            .map(|launcher| (launcher.clone(), false))
            .collect();

        for running in state.running_apps.iter() {
            if let Some(entry) = entries
                .iter_mut()
                .find(|(app, _)| app.match_id == running.match_id)
            {
                let override_name = entry.0.override_name.clone();
                let mut combined = running.clone();
                if override_name.is_some() {
                    combined.override_name = override_name;
                }
                entry.0 = combined;
                entry.1 = true;
            } else {
                entries.push((running.clone(), true));
            }
        }

        entries
    }
    fn render_elements_layers(&self, available_icon_width: f32) {
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = self.dock_config.read().unwrap().size.clamp(0.5, 2.0) as f32;
        let icon_color_filter = {
            let dock_config = self.dock_config.read().unwrap();
            if dock_config.colorize_icons {
                let color = parse_hex_color(&dock_config.colorize_color);
                let intensity = dock_config.colorize_intensity.clamp(0.0, 1.0) as f32;
                let (r, g, b) = (color.r, color.g, color.b);
                let (lr, lg, lb) = (0.2126_f32, 0.7152_f32, 0.0722_f32);
                let inv = 1.0 - intensity;
                let matrix = skia::ColorMatrix::new(
                    inv + intensity * lr * r,
                    intensity * lg * r,
                    intensity * lb * r,
                    0.0,
                    0.0,
                    intensity * lr * g,
                    inv + intensity * lg * g,
                    intensity * lb * g,
                    0.0,
                    0.0,
                    intensity * lr * b,
                    intensity * lg * b,
                    inv + intensity * lb * b,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                    0.0,
                );
                Some(skia::color_filters::matrix(&matrix, None))
            } else {
                None
            }
        };
        let state = self.get_state();
        let display_apps = self.display_entries(&state);
        let app_height = available_icon_width * (1.0 + 20.0 / 95.0);
        let miniwindow_height = available_icon_width * (1.0 + 60.0 / 95.0);

        // Calculate bar height using helper function
        let bar_height =
            Self::calculate_bar_height(available_icon_width, draw_scale * dock_size_multiplier);
        let padding_top = bar_height * 0.05;
        let padding_bottom = bar_height * 0.1;

        // Update view layer padding to match current icon size
        self.view_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Relative,
            display: taffy::Display::Flex,
            flex_direction: taffy::FlexDirection::Row,
            justify_content: Some(taffy::JustifyContent::Center),
            justify_items: Some(taffy::JustifyItems::Center),
            align_items: Some(taffy::AlignItems::FlexEnd),
            gap: taffy::Size::<taffy::LengthPercentage>::from_length(0.0),
            padding: taffy::Rect {
                top: taffy::length(padding_top),
                bottom: taffy::length(padding_bottom),
                right: taffy::length(available_icon_width * 10.0 / 95.0),
                left: taffy::length(available_icon_width * 10.0 / 95.0),
            },
            ..Default::default()
        });

        self.bar_layer
            .set_border_corner_radius(bar_height / 3.5, None);

        self.resize_handle.set_size(
            Size {
                width: taffy::length(25.0 * draw_scale),
                height: taffy::Dimension::Length(bar_height),
            },
            None,
        );

        self.bar_layer.set_size(
            Size {
                width: taffy::percent(1.0),
                height: taffy::Dimension::Length(bar_height),
            },
            None,
        );

        let mut previous_app_layers = self.get_app_layers();
        let mut apps_layers_map = self.app_layers.write().unwrap();
        for (app, running) in display_apps.iter() {
            let match_id = app.match_id.clone();
            let app_copy = app.clone();
            let app_name = app.clone().desktop_name().unwrap_or(app.identifier.clone());

            match apps_layers_map.entry(match_id.clone()) {
                Entry::Occupied(mut occ) => {
                    let entry = occ.get_mut();
                    entry.identifier = app.identifier.clone();

                    let icon_layer = entry.icon_layer.clone();
                    let layer = entry.layer.clone();
                    // let label = entry.label_layer.clone();

                    icon_layer.set_color_filter(icon_color_filter.clone());

                    // Update icon content if the icon changed.
                    // Running state is shown via the running_indicator_layer (separate from icon_stack).
                    let current_icon_id = app_copy.icon.as_ref().map(|i| i.unique_id());
                    if entry.icon_id != current_icon_id {
                        let draw_picture = draw_app_icon(&app_copy);
                        icon_layer.set_draw_content(draw_picture);
                        entry.icon_id = current_icon_id;
                    }
                    entry.running = *running;
                    let entry_is_running = entry.running;

                    // update main layer render function
                    layer.set_draw_content(move |canvas: &skia::Canvas, w: f32, h: f32| {
                        if entry_is_running {
                            let color = theme_colors().text_primary.opacity(0.9).c4f();
                            let mut paint = layers::skia::Paint::new(color, None);
                            paint.set_anti_alias(true);
                            paint.set_style(layers::skia::paint::Style::Fill);
                            let radius = 2.0 * draw_scale;
                            canvas.draw_circle(
                                (w / 2.0, h - radius - 2.0 * draw_scale),
                                radius,
                                &paint,
                            );
                        }
                        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
                    });

                    previous_app_layers.retain(|l| l.id() != layer.id());
                }
                Entry::Vacant(vac) => {
                    let new_layer = self.layers_engine.new_layer();
                    // icon_scaler wraps icon_stack: fixed size, scales to fill the magnified slot
                    let icon_scaler = self.layers_engine.new_layer();
                    // icon_stack holds icon + badge + progress (not label) — mirrored by app switcher
                    let icon_stack = self.layers_engine.new_layer();
                    let icon_layer = self.layers_engine.new_layer();
                    let badge_layer = self.layers_engine.new_layer();
                    let progress_layer = self.layers_engine.new_layer();

                    setup_app_icon(
                        &new_layer,
                        &icon_layer,
                        app_copy.clone(),
                        available_icon_width,
                        *running,
                    );
                    icon_layer.set_image_cached(true);
                    icon_layer.set_color_filter(icon_color_filter.clone());

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
                            .size(Size::points(BASE_ICON_SIZE, BASE_ICON_SIZE * 1.2))
                            .anchor_point(layers::types::Point::new(0.5, 0.5))
                            .picture_cached(true)
                            .image_cache(true)
                            .pointer_events(false)
                            .build()
                            .unwrap();
                        icon_scaler.build_layer_tree(&scaler_tree);
                    }
                    icon_scaler.set_position(
                        Point::new(
                            available_icon_width / 2.0,
                            (available_icon_width * 1.2) / 2.0,
                        ),
                        None,
                    );

                    // Set up icon_stack as a fixed-size square inside icon_scaler.
                    // icon_stack stays at its original size; visual scaling is handled by icon_scaler.
                    {
                        use layers::view::BuildLayerTree;

                        let stack_tree = layers::view::LayerTreeBuilder::default()
                            .key(format!("icon_stack_{}", app.identifier))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .size(Size::points(BASE_ICON_SIZE, BASE_ICON_SIZE))
                            .picture_cached(true)
                            .image_cache(true)
                            .pointer_events(false)
                            .build()
                            .unwrap();
                        icon_stack.build_layer_tree(&stack_tree);
                    }
                    icon_stack.set_position(Point::new(0.0, 0.0), None);
                    // icon_stack.set_anchor_point(layers::types::Point::new(0.5, 0.0), None);
                    setup_badge_layer(&badge_layer, BASE_ICON_SIZE);
                    setup_progress_layer(&progress_layer, BASE_ICON_SIZE);

                    let label_layer = self.layers_engine.new_layer();
                    setup_label(&label_layer, app_name);

                    self.dock_apps_container.add_sublayer(&new_layer);
                    // icon_scaler wraps icon_stack; icon_stack holds icon + badge + progress
                    new_layer.add_sublayer(&icon_scaler);
                    icon_scaler.add_sublayer(&icon_stack);
                    icon_stack.add_sublayer(&icon_layer);
                    icon_stack.add_sublayer(&badge_layer);
                    icon_stack.add_sublayer(&progress_layer);
                    // label is a direct child of new_layer, NOT inside icon_stack
                    new_layer.add_sublayer(&label_layer);

                    let icon_id = app_copy.icon.as_ref().map(|i| i.unique_id());

                    vac.insert(AppLayerEntry {
                        layer: new_layer.clone(),
                        icon_scaler: icon_scaler.clone(),
                        icon_stack: icon_stack.clone(),
                        icon_layer: icon_layer.clone(),
                        label_layer: label_layer.clone(),
                        badge_layer: badge_layer.clone(),
                        progress_layer: progress_layer.clone(),
                        icon_id,
                        running: *running,
                        identifier: app.identifier.clone(),
                    });

                    new_layer.remove_all_pointer_handlers();

                    let label_ref = label_layer.clone();
                    new_layer.add_on_pointer_in(move |_: &Layer, _, _| {
                        label_ref.set_opacity(1.0, Some(Transition::ease_in_quad(0.1)));
                    });
                    let label_ref = label_layer.clone();
                    new_layer.add_on_pointer_out(move |_: &Layer, _, _| {
                        label_ref.set_opacity(0.0, Some(Transition::ease_in_quad(0.1)));
                    });
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

                        self.dock_windows_container.add_sublayer(&new_layer);

                        setup_miniwindow_icon(&new_layer, &inner_layer, available_icon_width);

                        setup_label(&label_layer, title.clone());
                        new_layer.add_sublayer(&label_layer);

                        (new_layer, inner_layer, label_layer, None)
                    });

                let label = label.clone();
                layer.remove_all_pointer_handlers();

                let label_in = label.clone();
                layer.add_on_pointer_in(move |_: &Layer, _, _| {
                    label_in.set_opacity(1.0, Some(Transition::ease_in_quad(0.1)));
                });

                layer.add_on_pointer_out(move |_: &Layer, _: f32, _: f32| {
                    label.set_opacity(0.0, Some(Transition::ease_in_out_quad(0.1)));
                });
                previous_miniwindows.retain(|l| l.id() != layer.id());
            }
        }

        // Cleanup layers

        // App layers
        for layer in previous_app_layers {
            layer.set_opacity(0.0, Transition::ease_out_quad(0.2));
            layer
                .set_size(
                    layers::types::Size::points(0.0, app_height),
                    Transition::ease_out_quad(0.3),
                )
                .on_finish(
                    |l: &Layer, _| {
                        l.remove();
                    },
                    true,
                );
            apps_layers_map.retain(|_, entry| entry.layer.id() != layer.id());
        }

        // Mini window layers
        for layer in previous_miniwindows {
            layer.set_opacity(0.0, Transition::ease_out_quad(0.2));
            layer
                .set_size(
                    layers::types::Size::points(0.0, miniwindow_height),
                    Transition::ease_out_quad(0.3),
                )
                .on_finish(
                    |l: &Layer, _| {
                        l.remove();
                    },
                    true,
                );

            miniwindows_layers_map.retain(|_k, (v, ..)| v.id() != layer.id());
        }
    }
    pub fn available_icon_size(&self) -> f32 {
        let state = self.get_state();
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        // those are constant like values
        let available_width = state.width as f32 - 20.0 * draw_scale;
        let base_icon_size = 95.0;
        let dock_size_multiplier = self.dock_config.read().unwrap().size.clamp(0.5, 2.0) as f32;
        let icon_size: f32 = base_icon_size * dock_size_multiplier * draw_scale;

        let apps_len = self.display_entries(&state).len() as f32;
        let windows_len = state.minimized_windows.len() as f32;

        let mut component_padding_h: f32 = icon_size * 0.09 * draw_scale;
        if component_padding_h > 5.0 * draw_scale {
            component_padding_h = 5.0 * draw_scale;
        }

        let available_icon_size =
            (available_width - component_padding_h * 2.0) / (apps_len + windows_len);
        icon_size.min(available_icon_size)
    }

    /// Render dock elements (app icons and miniwindow icons) based on the current state.
    /// This is called whenever the state changes to update the dock appearance.
    fn render_dock(&self) {
        let available_icon_size = self.available_icon_size();

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
        self.magnify_elements_with_scale(scale_override);

        // Recompute and cache the autohide hot zone from the new dock dimensions.
        let screen_scale = Config::with(|c| c.screen_scale) as f32;
        // let dock_size_multiplier = self.dock_config.read().unwrap().size.clamp(0.5, 2.0) as f32;
        let bar_h = Self::calculate_bar_height(available_icon_size, 1.0) / screen_scale;
        let bar_h = bar_h / 2.0;
        let (screen_w, screen_h) = *self.screen_size.read().unwrap();
        // println!("screen x=0, w={}, h={} scale={}", screen_w, screen_h, screen_scale);

        let screen_h = screen_h as f32 / screen_scale;
        let screen_w = screen_w as f32 / screen_scale;
        *self.cached_hot_zone.write().unwrap() = if screen_w > 0.0 && screen_h > 0.0 {
            // println!("new hot zone: screen x=0, w={}, h={}", screen_w, screen_h);
            // println!("new hot zone: bar x=0, w={}, h={}", screen_w, bar_h);
            Some(
                skia::Rect::from_xywh(0.0, screen_h - bar_h, screen_w, bar_h)
                    .with_outset((20.0, 40.0)),
            )
        } else {
            None
        };
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

                    dock.update_state(&DockModel {
                        running_apps: apps,
                        minimized_windows,
                        ..state
                    });
                }
            }
        });
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
    pub fn add_window_element(&self, window: &WindowElement) -> (Layer, Layer) {
        let state = self.get_state();
        let mut minimized_windows = state.minimized_windows.clone();
        minimized_windows.push((window.id(), window.xdg_title().to_string()));

        self.update_state(&DockModel {
            minimized_windows,
            ..self.get_state()
        });
        let layers_map = self.miniwindow_layers.read().unwrap();
        let (drawer, inner, ..) = layers_map.get(&window.id()).unwrap();

        (drawer.clone(), inner.clone())
    }
    pub fn remove_window_element(&self, wid: &ObjectId) -> Option<Layer> {
        let mut drawer = None;
        let mut miniwindow_layers = self.miniwindow_layers.write().unwrap();
        if let Some((d, _, label, ..)) = miniwindow_layers.get(wid) {
            drawer = Some(d.clone());
            // hide the label
            label.set_opacity(0.0, None);
            miniwindow_layers.remove(wid);
        }
        drawer
    }
    // Magnify elements
    fn magnify_elements(&self) {
        self.magnify_elements_with_scale(None);
    }

    fn magnify_elements_with_scale(&self, scale_override: Option<f64>) {
        let magnification_enabled = self
            .magnification_enabled
            .load(std::sync::atomic::Ordering::SeqCst);
        if scale_override.is_none() && !magnification_enabled {
            return;
        }
        let pos = *self.magnification_position.read().unwrap();
        let bounds = self.view_layer.render_bounds_transformed();
        let pos = pos - bounds.x();
        let state = self.get_state();
        let display_apps = self.display_entries(&state);

        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = self.dock_config.read().unwrap().size.clamp(0.5, 2.0) as f32;
        let base_icon_size = 80.0;
        let icon_size: f32 = base_icon_size * dock_size_multiplier * draw_scale;
        let padding = icon_size * 20.0 / base_icon_size;
        let focus = pos / (bounds.width() - padding);

        let apps_len = display_apps.len() as f32;
        let windows_len = state.minimized_windows.len() as f32;

        let tot_elements = apps_len + windows_len;
        let animation = self
            .layers_engine
            .add_animation_from_transition(&Transition::ease_out_quad(0.08), false);
        let mut changes = Vec::new();
        let genie_scale =
            scale_override.unwrap_or_else(|| self.dock_config.read().unwrap().genie_scale);
        let genie_span = self.dock_config.read().unwrap().genie_span;
        {
            let layers_map = self.app_layers.read().unwrap();
            for (index, (app, _running)) in display_apps.iter().enumerate() {
                if let Some(entry) = layers_map.get(&app.match_id) {
                    let layer = entry.layer.clone();
                    let icon_pos = 1.0 / tot_elements * index as f32 + 1.0 / (tot_elements * 2.0);
                    let icon_focus =
                        1.0 + magnify_function(focus - icon_pos, genie_span) * genie_scale;
                    let focused_icon_size = icon_size * icon_focus as f32;
                    let height_padding = BASE_ICON_SIZE * 0.08;

                    let change = layer.change_size(Size::points(
                        focused_icon_size,
                        focused_icon_size + height_padding,
                    ));
                    changes.push(change);

                    entry.icon_scaler.set_size(
                        Size::points(BASE_ICON_SIZE, BASE_ICON_SIZE + height_padding),
                        None,
                    );
                    // icon_scaler has a fixed size of 100.0; animate its scale to stretch it
                    // to focused_icon_size. badge and progress scale with it as children.
                    let scaler = (focused_icon_size * 0.9) / BASE_ICON_SIZE;

                    let scaler_change_position = entry.icon_scaler.change_position(Point {
                        x: focused_icon_size / 2.0,
                        y: (focused_icon_size * 1.3) / 2.0,
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
        let miniwindow_start_index = display_apps.len();

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
        let bar_height = Self::calculate_bar_height(icon_size, draw_scale * dock_size_multiplier);

        let bar_change = self.bar_layer.change_size(Size {
            width: taffy::percent(1.0),
            height: taffy::Dimension::Length(bar_height),
        });
        self.bar_layer
            .set_border_corner_radius(bar_height / 3.5, None);

        self.resize_handle.set_size(
            Size {
                width: taffy::length(25.0 * draw_scale),
                height: taffy::Dimension::Length(bar_height),
            },
            None,
        );
        changes.push(bar_change);

        self.layers_engine.schedule_changes(&changes, animation);

        self.layers_engine.start_animation(animation, 0.0);
    }
    /// Update the physical screen dimensions so `render_dock` can compute a correct hot zone.
    pub fn set_screen_size(&self, w: i32, h: i32) {
        *self.screen_size.write().unwrap() = (w, h);
    }

    pub(super) fn demagnify_elements(&self) {
        *self.magnification_position.write().unwrap() = -500.0;
        self.magnify_elements_with_scale(Some(0.0));
    }

    pub fn update_magnification_position(&self, pos: f32) {
        *self.magnification_position.write().unwrap() = pos;
        if self.has_menu_open() {
            return;
        }
        self.magnify_elements();
    }
    pub fn bookmark_config_for(&self, match_id: &str) -> Option<DockBookmark> {
        self.dock_config
            .read()
            .unwrap()
            .bookmarks
            .iter()
            .find(|b| {
                b.desktop_id
                    .strip_suffix(".desktop")
                    .unwrap_or(&b.desktop_id)
                    == match_id
            })
            .cloned()
    }
    /// Returns the icon_stack layer for `identifier` so the app switcher can mirror it.
    /// The icon_stack contains icon + badge + progress overlays (not the label tooltip).
    pub fn get_icon_stack_for_app(&self, identifier: &str) -> Option<Layer> {
        self.app_layers
            .read()
            .unwrap()
            .values()
            .find(|e| e.identifier == identifier)
            .map(|e| e.icon_stack.clone())
    }

    pub fn bookmark_application(&self, match_id: &str) -> Option<Application> {
        self.state
            .read()
            .unwrap()
            .launchers
            .iter()
            .find(|app| app.match_id == match_id)
            .cloned()
    }

    /// Update the badge shown on the dock icon for `app_id`.
    /// Pass `None` or an empty string to hide the badge.
    pub fn update_badge_for_app(&self, app_id: &str, text: Option<String>) {
        let app_layers = self.app_layers.read().unwrap();
        if let Some(entry) = app_layers.values().find(|e| e.identifier == app_id) {
            let badge_layer = entry.badge_layer.clone();
            drop(app_layers);
            match text {
                Some(t) if !t.is_empty() => {
                    badge_layer.set_draw_content(draw_badge(t));
                    badge_layer.set_opacity(1.0, Some(Transition::ease_in_quad(0.15)));
                }
                _ => {
                    badge_layer.set_opacity(0.0, Some(Transition::ease_in_quad(0.15)));
                }
            }
        }
    }

    /// Update the progress bar shown on the dock icon for `app_id`.
    /// Pass `None` or a negative value to hide the progress bar.
    pub fn update_progress_for_app(&self, app_id: &str, value: Option<f64>) {
        let app_layers = self.app_layers.read().unwrap();
        if let Some(entry) = app_layers.values().find(|e| e.identifier == app_id) {
            let progress_layer = entry.progress_layer.clone();
            drop(app_layers);
            match value {
                Some(v) if v >= 0.0 => {
                    progress_layer.set_draw_content(draw_progress(v.clamp(0.0, 1.0)));
                    progress_layer.set_opacity(1.0, Some(Transition::ease_in_quad(0.15)));
                }
                _ => {
                    progress_layer.set_opacity(0.0, Some(Transition::ease_in_quad(0.15)));
                }
            }
        }
    }

    /// Open the dock-settings context menu anchored to the handle.
    pub fn open_handle_context_menu(&self) {
        let scale = Config::with(|c| c.screen_scale) as f32;
        let handle_bounds = self.resize_handle.render_bounds_transformed();
        let wrap_bounds = self.wrap_layer.render_bounds_transformed();
        let pos = Point::new(
            (handle_bounds.x() + handle_bounds.width() / 2.0 - wrap_bounds.x()) / scale,
            (handle_bounds.y() - wrap_bounds.y()) / scale,
        );

        let autohide = self.dock_config.read().unwrap().autohide;
        let magnification = self.dock_config.read().unwrap().magnification;

        let items = vec![
            MenuItem::action(if autohide {
                "✓ Auto-hide"
            } else {
                "Auto-hide"
            })
            .with_action_id("toggle_autohide"),
            MenuItem::action(if magnification {
                "✓ Magnification"
            } else {
                "Magnification"
            })
            .with_action_id("toggle_magnification"),
        ];

        let mut context_menu_lock = self.context_menu.write().unwrap();
        if context_menu_lock.is_none() {
            let menu = ContextMenuView::new(&self.wrap_layer, items.clone());
            let s = Config::with(|c| c.screen_scale) as f32;
            menu.set_style(ContextMenuStyle::default_with_scale(s));
            *context_menu_lock = Some(menu);
        }
        if let Some(menu) = context_menu_lock.as_ref() {
            menu.set_items(items);
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
            items.push(MenuItem::action("Open").with_action_id("open"));
            items.push(MenuItem::separator());
        }

        let keep_label = if bookmarked {
            "✓ Keep in Dock"
        } else {
            "Keep in Dock"
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
                MenuItem::action("Quit")
                    .with_action_id("quit")
                    .with_shortcut("⌘Q"),
            );
        }

        items
    }

    pub fn open_context_menu(&self, _pos: Point, app_id: String) {
        // Compute position from the app icon layer to anchor the menu above it
        let scale = Config::with(|c| c.screen_scale) as f32;
        let menu_pos = {
            let app_layers = self.app_layers.read().unwrap();
            let entry = app_layers.values().find(|e| e.identifier == app_id);
            if let Some(e) = entry {
                let icon_bounds = e.layer.render_bounds_transformed();
                let wrap_bounds = self.wrap_layer.render_bounds_transformed();
                // Center the menu horizontally over the icon;
                // anchor point (0.5, 1.0) means the bottom-center lands at (x, y),
                // so y = top-edge of the icon relative to the wrap_layer (in logical px).
                Point::new(
                    (icon_bounds.x() + icon_bounds.width() / 2.0 - wrap_bounds.x()) / scale,
                    (icon_bounds.y() - wrap_bounds.y()) / scale,
                )
            } else {
                _pos
            }
        };

        let mut context_menu_lock = self.context_menu.write().unwrap();
        if context_menu_lock.is_some() {
            // If a context menu is already open, close it
            if let Some(menu) = context_menu_lock.as_ref() {
                menu.hide();
            }
        } else {
            let items = self.build_context_menu_items(&app_id);
            let menu = ContextMenuView::new(&self.wrap_layer, items);
            let scale = Config::with(|c| c.screen_scale) as f32;
            menu.set_style(ContextMenuStyle::default_with_scale(scale));
            *context_menu_lock = Some(menu);
        }

        if let Some(menu) = context_menu_lock.as_ref() {
            // Refresh items in case the menu was reused (app state may have changed)
            let items = self.build_context_menu_items(&app_id);
            menu.set_items(items);
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
                entry.icon_scaler.set_opacity(1.0, None);
                entry
                    .label_layer
                    .set_opacity(0.0, Some(Transition::ease_in_quad(0.05)));
            } else {
                entry.icon_scaler.set_color_filter(None);
                entry.icon_scaler.set_opacity(1.0, None);
                entry
                    .label_layer
                    .set_opacity(1.0, Some(Transition::ease_in_quad(0.05)));
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

    pub(super) fn set_magnification_enabled(&self, enabled: bool) {
        self.dock_config.write().unwrap().magnification = enabled;
        self.magnification_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
        if !enabled {
            // Reset all icons to base size immediately
            self.update_magnification_position(-500.0);
        }
    }

    /// Persist the current in-memory dock config to the writable config file.
    pub(super) fn save_config(&self) {
        crate::config::save_dock_config(&self.dock_config.read().unwrap());
    }

    /// Mutate the in-memory dock config and immediately persist it.
    pub(super) fn update_dock_config(&self, f: impl FnOnce(&mut crate::config::DockConfig)) {
        f(&mut self.dock_config.write().unwrap());
        self.save_config();
    }

    /// Schedule hiding the dock after a short delay (if autohide is enabled).
    /// The delay is handled by the animation itself; a subsequent show() call
    /// overrides the pending animation and cancels the hide naturally.
    pub fn schedule_autohide(&self) {
        if !self.dock_config.read().unwrap().autohide || self.is_hidden() || self.has_menu_open() {
            return;
        }
        self.active
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.view_layer
            .set_position(
                (0.0, 250.0),
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
    pub fn show_autohide(&self) {
        if self.dock_config.read().unwrap().autohide {
            if self.active.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            self.view_layer.set_hidden(false);
            tracing::debug!("dock: show (override pending hide)");
            self.active
                .store(true, std::sync::atomic::Ordering::Relaxed);
            self.view_layer.set_position(
                (0.0, 0.0),
                Some(Transition {
                    timing: TimingFunction::Spring(Spring::with_duration_and_bounce(0.5, 0.2)),
                    delay: 0.0,
                }),
            );
        }
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
    E.powf(-1.0 * genie_span * x.powi(2))
}
