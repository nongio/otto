use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    sync::{atomic::AtomicBool, Arc, RwLock},
    time::Duration,
};

use layers::{
    engine::{animation::Transition, Engine, NodeRef, TransactionRef},
    prelude::{taffy, Layer, Point},
    skia,
    taffy::{prelude::FromLength, style::Style},
    types::{BlendMode, Size},
    view::{BuildLayerTree, LayerTreeBuilder},
};
use smithay::{reexports::wayland_server::backend::ObjectId, utils::IsAlive};
use tokio::sync::mpsc;

use crate::{
    config::{Config, DockBookmark},
    shell::WindowElement,
    theme::theme_colors,
    utils::Observer,
    workspaces::{apps_info::ApplicationsInfo, Application, WorkspacesModel},
};

use super::{
    model::DockModel,
    render::{draw_app_icon, draw_running_indicator_only, setup_app_icon, setup_app_container_for_protocol_layer, setup_label, setup_miniwindow_icon},
};

#[derive(Debug, Clone)]
struct AppLayerEntry {
    layer: Layer,
    icon_layer: Layer,
    label_layer: Layer,
    icon_id: Option<u32>,
    running: bool,
    identifier: String,
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

    app_layers: Arc<RwLock<HashMap<String, AppLayerEntry>>>,
    miniwindow_layers: Arc<RwLock<HashMap<ObjectId, MiniWindowLayers>>>,
    /// Protocol-provided layer shell surfaces (app_id -> wlr layer)
    pub protocol_app_layers: Arc<RwLock<HashMap<String, Layer>>>,
    state: Arc<RwLock<DockModel>>,
    active: Arc<AtomicBool>,
    notify_tx: tokio::sync::mpsc::Sender<WorkspacesModel>,
    latest_event: Arc<tokio::sync::RwLock<Option<WorkspacesModel>>>,
    magnification_position: Arc<RwLock<f32>>,
    bookmark_configs: Arc<RwLock<HashMap<String, DockBookmark>>>,

    pub dragging: Arc<AtomicBool>,
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

        layers_engine.add_layer(&wrap_layer);

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
            .key("dock_handle")
            .pointer_events(false)
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
            protocol_app_layers: Arc::new(RwLock::new(HashMap::new())),
            state: Arc::new(RwLock::new(initial_state)),
            active: Arc::new(AtomicBool::new(true)),
            notify_tx,
            latest_event: Arc::new(tokio::sync::RwLock::new(None)),
            magnification_position: Arc::new(RwLock::new(-500.0)),
            bookmark_configs: Arc::new(RwLock::new(HashMap::new())),
            dragging: Arc::new(AtomicBool::new(false)),
        };
        dock.render_dock();
        dock.notification_handler(notify_rx);
        dock.load_configured_bookmarks();

        dock
    }
    fn load_configured_bookmarks(&self) {
        let bookmarks = Config::with(|c| c.dock.bookmarks.clone());
        {
            let mut configs = self.bookmark_configs.write().unwrap();
            configs.clear();
        }
        if bookmarks.is_empty() {
            let mut state = self.get_state();
            state.launchers.clear();
            self.update_state(&state);
            return;
        }

        let dock = self.clone();
        tokio::spawn(async move {
            let mut launchers = Vec::new();
            let mut configs = HashMap::new();

            for bookmark in bookmarks {
                if let Some(mut app) =
                    ApplicationsInfo::get_app_info_by_id(bookmark.desktop_id.clone()).await
                {
                    app.override_name = bookmark.label.clone();
                    configs.insert(app.match_id.clone(), bookmark.clone());
                    launchers.push(app);
                } else {
                    tracing::warn!("dock bookmark not found: {}", bookmark.desktop_id);
                }
            }

            {
                let mut cfg_guard = dock.bookmark_configs.write().unwrap();
                *cfg_guard = configs;
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
    pub fn hide(&self, transition: Option<Transition>) -> TransactionRef {
        self.active
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.view_layer.set_position((0.0, 250.0), transition)
    }
    pub fn show(&self, transition: Option<Transition>) -> TransactionRef {
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
        let dock_size_multiplier = Config::with(|config| config.dock.size.clamp(0.5, 2.0)) as f32;
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
                    let label = entry.label_layer.clone();

                    // Check if this app now has a protocol layer
                    let protocol_layers = self.protocol_app_layers.read().unwrap();
                    let has_protocol_layer = protocol_layers.contains_key(&app.identifier);
                    let protocol_layer = protocol_layers.get(&app.identifier).cloned();
                    drop(protocol_layers);

                    tracing::info!("Occupied: App {} has_protocol_layer={}", app.identifier, has_protocol_layer);

                    if has_protocol_layer {
                        // Replace icon with protocol layer
                        tracing::info!("Replacing existing icon with protocol layer for app {}", app.identifier);
                        
                        // Clear the icon drawing function so only the protocol layer shows
                        icon_layer.set_draw_content(draw_running_indicator_only(*running));
                        
                        // Add protocol layer as sublayer
                        if let Some(proto_layer) = protocol_layer {
                            // Position and size the protocol layer to fill the container
                            proto_layer.set_position(Point::new(0.0, 0.0), None);
                            proto_layer.set_size(Size::percent(1.0, 1.0), None);
                            
                            icon_layer.add_sublayer(&proto_layer);
                            tracing::info!("Added protocol layer to existing icon_layer for app {} (positioned at 0,0, size 100%)", app.identifier);
                        }
                        
                        // Update entry to reflect it now has a protocol layer (no icon_id)
                        entry.icon_id = None;
                    } else {
                        // Normal icon update
                        let current_icon_id = app_copy.icon.as_ref().map(|i| i.unique_id());
                        if entry.icon_id != current_icon_id || entry.running != *running {
                            let draw_picture = draw_app_icon(&app_copy, *running);
                            icon_layer.set_draw_content(draw_picture);
                            entry.icon_id = current_icon_id;
                        }
                    }
                    entry.running = *running;

                    let darken_color = skia::Color::from_argb(100, 100, 100, 100);
                    let add = skia::Color::from_argb(0, 0, 0, 0);
                    let filter = skia::color_filters::lighting(darken_color, add);

                    let icon_ref = icon_layer.clone();
                    layer.remove_all_pointer_handlers();

                    layer.add_on_pointer_press(move |_: &Layer, _, _| {
                        icon_ref.set_color_filter(filter.clone());
                    });

                    let icon_ref = icon_layer.clone();
                    layer.add_on_pointer_release(move |_: &Layer, _, _| {
                        icon_ref.set_color_filter(None);
                    });

                    let label_ref = label.clone();
                    layer.add_on_pointer_in(move |_: &Layer, _, _| {
                        label_ref.set_opacity(1.0, Some(Transition::ease_in_quad(0.1)));
                    });
                    let label_ref = label.clone();
                    let icon_ref = icon_layer.clone();
                    layer.add_on_pointer_out(move |_: &Layer, _, _| {
                        label_ref.set_opacity(0.0, Some(Transition::ease_in_quad(0.1)));
                        icon_ref.set_color_filter(None);
                    });
                    previous_app_layers.retain(|l| l.id() != layer.id());
                }
                Entry::Vacant(vac) => {
                    let new_layer = self.layers_engine.new_layer();
                    
                    // Check if this app has a protocol-provided layer
                    let protocol_layers = self.protocol_app_layers.read().unwrap();
                    let has_protocol_layer = protocol_layers.contains_key(&app.identifier);
                    tracing::info!("App {} has_protocol_layer={}", app.identifier, has_protocol_layer);
                    drop(protocol_layers);
                    
                    if has_protocol_layer {
                        // Use protocol layer instead of icon
                        tracing::info!("Using protocol layer for app {}", app.identifier);
                        let container_layer = self.layers_engine.new_layer();
                        setup_app_container_for_protocol_layer(
                            &new_layer,
                            &container_layer,
                            app_name.to_string(),
                            available_icon_width,
                            *running,
                        );
                        
                        // Add the protocol layer as a sublayer
                        if let Some(protocol_layer) = self.protocol_app_layers.read().unwrap().get(&app.identifier) {
                            // Position and size the protocol layer to fill the container
                            protocol_layer.set_position(Point::new(0.0, 0.0), None);
                            protocol_layer.set_size(Size::percent(1.0, 1.0), None);
                            
                            container_layer.add_sublayer(protocol_layer);
                            tracing::info!("Added protocol layer as sublayer for app {} (positioned at 0,0, size 100%)", app.identifier);
                        } else {
                            tracing::warn!("Protocol layer disappeared for app {}", app.identifier);
                        }
                        
                        self.dock_apps_container.add_sublayer(&new_layer);
                        let label_layer = self.layers_engine.new_layer();
                        
                        setup_label(&label_layer, app_name);
                        new_layer.add_sublayer(&container_layer);
                        new_layer.add_sublayer(&label_layer);
                        
                        vac.insert(AppLayerEntry {
                            layer: new_layer.clone(),
                            icon_layer: container_layer.clone(),
                            label_layer: label_layer.clone(),
                            icon_id: None, // Protocol layers don't have icon IDs
                            running: *running,
                            identifier: app.identifier.clone(),
                        });
                        
                        let label_ref = label_layer.clone();
                        new_layer.add_on_pointer_in(move |_: &Layer, _, _| {
                            label_ref.set_opacity(1.0, Some(Transition::ease_in_quad(0.1)));
                        });
                        let label_ref = label_layer.clone();
                        new_layer.add_on_pointer_out(move |_: &Layer, _, _| {
                            label_ref.set_opacity(0.0, Some(Transition::ease_in_quad(0.1)));
                        });
                    } else {
                        // Traditional icon rendering
                        let icon_layer = self.layers_engine.new_layer();
                        setup_app_icon(
                            &new_layer,
                            &icon_layer,
                            app_copy.clone(),
                            available_icon_width,
                            *running,
                        );
                        icon_layer.set_image_cached(true);

                        self.dock_apps_container.add_sublayer(&new_layer);
                        let label_layer = self.layers_engine.new_layer();

                        setup_label(&label_layer, app_name);
                        new_layer.add_sublayer(&icon_layer);
                        new_layer.add_sublayer(&label_layer);
                        let icon_id = app_copy.icon.as_ref().map(|i| i.unique_id());

                        vac.insert(AppLayerEntry {
                            layer: new_layer.clone(),
                            icon_layer: icon_layer.clone(),
                            label_layer: label_layer.clone(),
                            icon_id,
                            running: *running,
                            identifier: app.identifier.clone(),
                        });

                        let darken_color = skia::Color::from_argb(100, 100, 100, 100);
                        let add = skia::Color::from_argb(0, 0, 0, 0);
                        let filter = skia::color_filters::lighting(darken_color, add);

                        let icon_ref = icon_layer.clone();
                        new_layer.remove_all_pointer_handlers();

                        new_layer.add_on_pointer_press(move |_: &Layer, _, _| {
                            icon_ref.set_color_filter(filter.clone());
                        });

                        let icon_ref = icon_layer.clone();
                        new_layer.add_on_pointer_release(move |_: &Layer, _, _| {
                            icon_ref.set_color_filter(None);
                        });

                        let label_ref = label_layer.clone();
                        new_layer.add_on_pointer_in(move |_: &Layer, _, _| {
                            label_ref.set_opacity(1.0, Some(Transition::ease_in_quad(0.1)));
                        });
                        let label_ref = label_layer.clone();
                        let icon_ref = icon_layer.clone();
                        new_layer.add_on_pointer_out(move |_: &Layer, _, _| {
                            label_ref.set_opacity(0.0, Some(Transition::ease_in_quad(0.1)));
                            icon_ref.set_color_filter(None);
                        });
                    }
                    previous_app_layers.retain(|l| l.id() != new_layer.id());
                }
            }
        }

        // Render protocol-only layers (apps that provided layers but aren't in the app list yet)
        let protocol_layers_map = self.protocol_app_layers.read().unwrap();
        tracing::info!("Checking protocol layers, count: {}", protocol_layers_map.len());
        for (app_id, protocol_layer) in protocol_layers_map.iter() {
            tracing::info!("Processing protocol layer for app_id={}", app_id);
            // Check if this app_id is already rendered (skip if it is)
            let already_rendered = display_apps.iter().any(|(app, _)| &app.identifier == app_id);
            tracing::info!("app_id={} already_rendered={}", app_id, already_rendered);
            if already_rendered {
                tracing::info!("Skipping app_id={}, already rendered in normal apps", app_id);
                continue;
            }

            // Render protocol-only layer
            match apps_layers_map.entry(app_id.clone()) {
                Entry::Occupied(_) => {
                    // Already exists, skip
                    tracing::info!("app_id={} already has layer entry, skipping", app_id);
                }
                Entry::Vacant(vac) => {
                    tracing::info!("Creating new dock entry for protocol layer app_id={}", app_id);
                    let new_layer = self.layers_engine.new_layer();
                    let container_layer = self.layers_engine.new_layer();
                    
                    setup_app_container_for_protocol_layer(
                        &new_layer,
                        &container_layer,
                        app_id.clone(),
                        available_icon_width,
                        true, // Assume running since it provided a layer
                    );
                    
                    // Add the actual protocol layer as a sublayer
                    // Position and size the protocol layer to fill the container
                    protocol_layer.set_position(Point::new(0.0, 0.0), None);
                    protocol_layer.set_size(Size::percent(1.0, 1.0), None);
                    
                    container_layer.add_sublayer(protocol_layer);
                    tracing::info!("Added protocol layer as sublayer to container for app_id={} (positioned at 0,0, size 100%)", app_id);
                    
                    self.dock_apps_container.add_sublayer(&new_layer);
                    tracing::info!("Added new_layer to dock_apps_container for app_id={}", app_id);
                    let label_layer = self.layers_engine.new_layer();
                    
                    setup_label(&label_layer, app_id.to_string());
                    new_layer.add_sublayer(&container_layer);
                    new_layer.add_sublayer(&label_layer);
                    
                    vac.insert(AppLayerEntry {
                        layer: new_layer.clone(),
                        icon_layer: container_layer.clone(),
                        label_layer: label_layer.clone(),
                        icon_id: None,
                        running: true,
                        identifier: app_id.clone(),
                    });
                    
                    let label_ref = label_layer.clone();
                    new_layer.add_on_pointer_in(move |_: &Layer, _, _| {
                        label_ref.set_opacity(1.0, Some(Transition::ease_in_quad(0.1)));
                    });
                    let label_ref = label_layer.clone();
                    new_layer.add_on_pointer_out(move |_: &Layer, _, _| {
                        label_ref.set_opacity(0.0, Some(Transition::ease_in_quad(0.1)));
                    });
                    
                    previous_app_layers.retain(|l| l.id() != new_layer.id());
                    tracing::info!("Completed protocol layer setup for app_id={}", app_id);
                }
            }
        }
        drop(protocol_layers_map);
        tracing::info!("Finished processing protocol layers");

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

                layer.remove_all_pointer_handlers();

                let darken_color = skia::Color::from_argb(100, 100, 100, 100);
                let add = skia::Color::from_argb(0, 0, 0, 0);
                let filter = skia::color_filters::lighting(darken_color, add);

                layer.remove_all_pointer_handlers();

                layer.add_on_pointer_press(move |l: &Layer, _: f32, _: f32| {
                    l.children().iter().for_each(|child| {
                        child.set_color_filter(filter.clone());
                    });
                });
                // let inner_ref = inner.clone();
                layer.add_on_pointer_release(move |l: &Layer, _: f32, _: f32| {
                    l.children().iter().for_each(|child| {
                        child.set_color_filter(None);
                    });
                });

                let label_ref = label.clone();
                layer.add_on_pointer_in(move |_: &Layer, _, _| {
                    label_ref.set_opacity(1.0, Some(Transition::ease_in_quad(0.1)));
                });
                let label_ref = label.clone();

                layer.add_on_pointer_out(move |l: &Layer, _: f32, _: f32| {
                    label_ref.set_opacity(0.0, Some(Transition::ease_in_out_quad(0.1)));
                    l.children().iter().for_each(|child| {
                        child.set_color_filter(None);
                    });
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
    fn available_icon_size(&self) -> f32 {
        let state = self.get_state();
        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        // those are constant like values
        let available_width = state.width as f32 - 20.0 * draw_scale;
        let base_icon_size = 95.0;
        let dock_size_multiplier = Config::with(|config| config.dock.size.clamp(0.5, 2.0)) as f32;
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
    fn render_dock(&self) {
        let available_icon_size = self.available_icon_size();

        self.render_elements_layers(available_icon_size);
        self.magnify_elements();
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
        let pos = *self.magnification_position.read().unwrap();
        let bounds = self.view_layer.render_bounds_transformed();
        let pos = pos - bounds.x();
        let state = self.get_state();
        let display_apps = self.display_entries(&state);

        let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
        let dock_size_multiplier = Config::with(|config| config.dock.size.clamp(0.5, 2.0)) as f32;
        let base_icon_size = 95.0;
        let icon_size: f32 = base_icon_size * dock_size_multiplier * draw_scale;
        let padding = icon_size * 20.0 / 95.0;
        let focus = pos / (bounds.width() - padding);

        let apps_len = display_apps.len() as f32;
        let windows_len = state.minimized_windows.len() as f32;

        let tot_elements = apps_len + windows_len;
        let animation = self
            .layers_engine
            .add_animation_from_transition(&Transition::ease_out_quad(0.08), false);
        let mut changes = Vec::new();
        let genie_scale = Config::with(|c| c.dock.genie_scale);
        {
            let layers_map = self.app_layers.read().unwrap();
            for (index, (app, _running)) in display_apps.iter().enumerate() {
                if let Some(entry) = layers_map.get(&app.match_id) {
                    let layer = entry.layer.clone();
                    let icon_pos = 1.0 / tot_elements * index as f32 + 1.0 / (tot_elements * 2.0);
                    let icon_focus = 1.0 + magnify_function(focus - icon_pos) * genie_scale;
                    let focused_icon_size = icon_size * icon_focus as f32;
                    let height_padding = draw_scale * dock_size_multiplier * 16.0;

                    let change = layer.change_size(Size::points(
                        focused_icon_size,
                        focused_icon_size + height_padding,
                    ));
                    changes.push(change);
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
                let icon_focus = 1.0 + magnify_function(focus - icon_pos) * genie_scale;
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
    pub fn update_magnification_position(&self, pos: f32) {
        *self.magnification_position.write().unwrap() = pos;
        self.magnify_elements();
    }
    pub fn bookmark_config_for(&self, match_id: &str) -> Option<DockBookmark> {
        self.bookmark_configs.read().unwrap().get(match_id).cloned()
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
    
    /// Check if an app has a protocol layer registered
    pub fn has_protocol_layer(&self, app_id: &str) -> bool {
        self.protocol_app_layers.read().unwrap().contains_key(app_id)
    }
    
    /// Register a protocol-provided layer shell surface for a dock app
    pub fn register_protocol_layer(&self, app_id: String, layer: Layer) {
        tracing::info!("DockView::register_protocol_layer called for app_id={}", app_id);
        
        // Create a wrapper layer that we fully control
        let wrapper_layer = self.layers_engine.new_layer();
        wrapper_layer.set_position(Point::new(0.0, 0.0), None);
        wrapper_layer.set_size(Size::percent(1.0, 1.0), None);
        wrapper_layer.set_draw_content(layer.as_content());
        wrapper_layer.set_pointer_events(false);
        layer.add_follower_node(wrapper_layer.clone());
        // Add the original layer as a sublayer
        // The wrapper controls position/size, the original layer just renders
        wrapper_layer.add_sublayer(&layer);
        tracing::info!("Created wrapper layer for protocol layer app_id={}", app_id);
        
        let mut protocol_layers = self.protocol_app_layers.write().unwrap();
        protocol_layers.insert(app_id.clone(), wrapper_layer);
        tracing::info!("Protocol layer registered (wrapped), total protocol layers: {}", protocol_layers.len());
        drop(protocol_layers);
        
        // Trigger a redraw to update the dock
        tracing::info!("Triggering dock redraw");
        self.render_dock();
    }
    
    /// Unregister a protocol-provided layer shell surface
    pub fn unregister_protocol_layer(&self, app_id: &str) {
        let mut protocol_layers = self.protocol_app_layers.write().unwrap();
        protocol_layers.remove(app_id);
        drop(protocol_layers);
        
        // Trigger a redraw to update the dock
        self.render_dock();
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
pub fn magnify_function(x: impl Into<f64>) -> f64 {
    let x = x.into();
    let genie_span = Config::with(|c| c.dock.genie_span);
    let genie_span = -1.0 * genie_span;
    E.powf(genie_span * (x).powi(2))
}
