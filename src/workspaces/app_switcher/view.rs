use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

use layers::{
    engine::{
        animation::{TimingFunction, Transition},
        Engine, TransactionRef,
    },
    prelude::taffy,
    taffy::{prelude::FromLength, style::Style},
    types::{BlendMode, BorderRadius, PaintColor, Size},
};
use smithay::utils::IsAlive;
use tokio::sync::mpsc;

use crate::{
    interactive_view::ViewInteractions,
    theme::theme_colors,
    utils::Observer,
    workspaces::{Application, DockView, WorkspacesModel, apps_info::ApplicationsInfo, dock::BASE_ICON_SIZE},
};

use super::{
    model::AppSwitcherModel,
    render::{draw_appswitcher_overlay, layout_metrics},
};

#[derive(Debug, Clone)]
pub struct AppSwitcherView {
    pub wrap_layer: layers::prelude::Layer,
    panel_layer: layers::prelude::Layer,
    apps_container: layers::prelude::Layer,
    layers_engine: Arc<Engine>,
    dock: Arc<DockView>,
    mirror_layers: Arc<RwLock<HashMap<String, layers::prelude::Layer>>>,
    state: Arc<RwLock<AppSwitcherModel>>,
    active: Arc<AtomicBool>,
    notify_tx: mpsc::Sender<WorkspacesModel>,
    latest_event: Arc<tokio::sync::RwLock<Option<WorkspacesModel>>>,
}
impl PartialEq for AppSwitcherView {
    fn eq(&self, other: &Self) -> bool {
        self.wrap_layer == other.wrap_layer
    }
}
impl IsAlive for AppSwitcherView {
    fn alive(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
}

impl AppSwitcherView {
    pub fn new(layers_engine: Arc<Engine>, dock: Arc<DockView>) -> Self {
        let wrap = layers_engine.new_layer();
        wrap.set_key("app_switcher");
        wrap.set_size(Size::percent(1.0, 1.0), None);
        wrap.set_layout_style(Style {
            position: taffy::Position::Absolute,
            display: taffy::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            ..Default::default()
        });
        wrap.set_opacity(0.0, None);
        wrap.set_pointer_events(false);
        wrap.set_hidden(true);

        let panel = layers_engine.new_layer();
        panel.set_key("app_switcher_panel");
        panel.set_blend_mode(BlendMode::BackgroundBlur);
        panel.set_background_color(
            PaintColor::Solid {
                color: theme_colors().materials_thin,
            },
            None,
        );
        panel.set_layout_style(Style {
            display: taffy::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            ..Default::default()
        });
        panel.set_pointer_events(false);
        panel.set_hidden(true);

        let apps_container = layers_engine.new_layer();
        apps_container.set_key("app_switcher_apps_container");
        apps_container.set_layout_style(Style {
            position: taffy::Position::Absolute,
            display: taffy::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            ..Default::default()
        });
        apps_container.set_pointer_events(false);

        layers_engine.add_layer(&wrap);
        wrap.add_sublayer(&panel);
        panel.add_sublayer(&apps_container);

        let (notify_tx, notify_rx) = mpsc::channel(5);

        let view = Self {
            wrap_layer: wrap,
            panel_layer: panel,
            apps_container,
            layers_engine,
            dock,
            mirror_layers: Arc::new(RwLock::new(HashMap::new())),
            state: Arc::new(RwLock::new(AppSwitcherModel::new())),
            active: Arc::new(AtomicBool::new(false)),
            notify_tx,
            latest_event: Arc::new(tokio::sync::RwLock::new(None)),
        };
        view.init_notification_handler(notify_rx);
        view
    }

    fn update_state(&self, new_state: AppSwitcherModel) {
        *self.state.write().unwrap() = new_state;
        self.rebuild_mirrors();
        self.update_panel();
    }

    /// Tear down all existing mirror layers and rebuild them in app-list order.
    /// Each mirror is a live clone of the dock's `icon_stack` so badge/progress
    /// overlays show through automatically.
    fn rebuild_mirrors(&self) {
        let state = self.state.read().unwrap().clone();
        let (_, _, available_icon_size, icon_padding, gap, _, _) = layout_metrics(&state);
        let slot_size = available_icon_size + icon_padding * 2.0;

        {
            let mut mirrors = self.mirror_layers.write().unwrap();
            for (_, m) in mirrors.drain() {
                m.remove();
            }
        }

        self.apps_container.set_layout_style(Style {
            position: taffy::Position::Absolute,
            display: taffy::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            gap: taffy::Size::<taffy::LengthPercentage>::from_length(gap),
            ..Default::default()
        });

        let mut mirrors = self.mirror_layers.write().unwrap();
        for app in &state.apps {
            if let Some(icon_stack) = self.dock.get_icon_stack_for_app(&app.identifier) {
                let wrap = self.layers_engine.new_layer();
                wrap.set_key(format!("switcher_wrap_{}", app.identifier));
                wrap.set_layout_style(Style {
                    size: taffy::Size {
                        width: taffy::Dimension::Length(slot_size),
                        height: taffy::Dimension::Length(slot_size),
                    },
                    ..Default::default()
                });
                let mirror = self.layers_engine.new_layer();
                mirror.set_draw_content(icon_stack.as_content());
                mirror.set_picture_cached(false);
                mirror.set_key(format!("switcher_mirror_{}", app.identifier));
                mirror.set_position(layers::prelude::Point {x: slot_size / 2.0, y: slot_size / 2.0}, None);
                mirror.set_anchor_point(layers::prelude::Point {x: 0.5, y: 0.5}, None);
                // Keep the mirror at the original icon_stack size and use scale to
                // match the desired slot_size visually.
                let original_size = layers::types::Point::new(BASE_ICON_SIZE, BASE_ICON_SIZE * 1.08);
                let scale = if original_size.x > 0.0 {
                    (slot_size * 0.8) / original_size.x
                } else {
                    1.0
                };
                mirror.set_layout_style(Style {
                    size: taffy::Size {
                        width: taffy::Dimension::Length(original_size.x),
                        height: taffy::Dimension::Length(original_size.y),
                    },
                    ..Default::default()
                });
                mirror.set_scale(layers::types::Point::new(scale, scale), None);
                icon_stack.add_follower_node(&mirror);
                wrap.add_sublayer(&mirror);
                self.apps_container.add_sublayer(&wrap);
                mirrors.insert(app.identifier.clone(), wrap);
            }
        }
    }

    fn update_panel(&self) {
        let state = self.state.read().unwrap().clone();
        let (w, h, ..) = layout_metrics(&state);
        self.panel_layer
            .set_size(Size::points(w, h), Some(Transition::ease_out_quad(0.35)));
        self.panel_layer
            .set_border_corner_radius(BorderRadius::new_single(h / 8.0), None);
        self.panel_layer
            .set_draw_content(draw_appswitcher_overlay(&state));
    }

    pub fn next(&self) {
        let mut state = self.state.read().unwrap().clone();
        if !self.active.load(Ordering::Relaxed) {
            state.current_app = 0;
        }
        if !state.apps.is_empty() {
            state.current_app = (state.current_app + 1) % state.apps.len();
        } else {
            state.current_app = 0;
        }
        *self.state.write().unwrap() = state;
        self.update_panel();
        self.active.store(true, Ordering::Relaxed);
        self.wrap_layer.set_hidden(false);
        self.panel_layer.set_hidden(false);
        self.wrap_layer.set_opacity(
            1.0,
            Some(Transition {
                delay: 0.1,
                timing: TimingFunction::ease_out_quad(0.150),
            }),
        );
    }

    pub fn previous(&self) {
        let mut state = self.state.read().unwrap().clone();
        if !state.apps.is_empty() {
            state.current_app =
                (state.current_app + state.apps.len() - 1) % state.apps.len();
        } else {
            state.current_app = 0;
        }
        *self.state.write().unwrap() = state;
        self.update_panel();
        self.active.store(true, Ordering::Relaxed);
        self.wrap_layer.set_hidden(false);
        self.panel_layer.set_hidden(false);
        self.wrap_layer.set_opacity(
            1.0,
            Some(Transition {
                delay: 0.05,
                timing: TimingFunction::linear(0.01),
            }),
        );
    }

    pub fn hide(&self) -> TransactionRef {
        self.active.store(false, Ordering::Relaxed);
        let tr = self
            .wrap_layer
            .set_opacity(0.0, Some(Transition::ease_in_quad(0.01)));
        let p = self.panel_layer.clone();
        tr.on_finish(
            move |l: &layers::prelude::Layer, _p: f32| {
                l.set_hidden(true);
                p.set_hidden(true);
            },
            true,
        );
        tr
    }

    pub fn reset(&self) {
        self.state.write().unwrap().current_app = 0;
        self.update_panel();
    }

    pub fn get_current_app_id(&self) -> Option<String> {
        let state = self.state.read().unwrap();
        state
            .apps
            .get(state.current_app)
            .map(|app| app.identifier.clone())
    }

    fn init_notification_handler(&self, mut rx: mpsc::Receiver<WorkspacesModel>) {
        let latest_event = self.latest_event.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                *latest_event.write().await = Some(event);
            }
        });
        let latest_event = self.latest_event.clone();
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs_f32(0.4)).await;
                let event = {
                    let mut guard = latest_event.write().await;
                    guard.take()
                };
                if let Some(workspace) = event {
                    let mut app_set = HashSet::new();
                    let mut apps: Vec<Application> = Vec::new();
                    for app_id in workspace.zindex_application_list.iter().rev() {
                        if app_set.insert(app_id.clone()) {
                            if let Some(app) =
                                ApplicationsInfo::get_app_info_by_id(app_id).await
                            {
                                apps.push(app);
                            }
                        }
                    }
                    let current_app = {
                        let s = this.state.read().unwrap();
                        if apps.is_empty() {
                            0
                        } else if s.current_app >= apps.len() {
                            apps.len() - 1
                        } else {
                            s.current_app
                        }
                    };
                    this.update_state(AppSwitcherModel {
                        current_app,
                        apps,
                        width: workspace.width,
                    });
                }
            }
        });
    }
}

impl Observer<WorkspacesModel> for AppSwitcherView {
    fn notify(&self, event: &WorkspacesModel) {
        let _ = self.notify_tx.try_send(event.clone());
    }
}

impl<Backend: crate::state::Backend> ViewInteractions<Backend> for AppSwitcherView {
    fn id(&self) -> Option<usize> {
        Some(self.wrap_layer.id.0.into())
    }
    fn is_alive(&self) -> bool {
        self.alive()
    }
}
