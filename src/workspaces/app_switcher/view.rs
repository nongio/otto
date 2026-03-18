use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use layers::{
    engine::{
        animation::{TimingFunction, Transition},
        Engine,
    },
    prelude::{taffy, View},
    taffy::style::Style,
    types::Size,
};
use smithay::utils::IsAlive;
use tokio::sync::mpsc;

use crate::{
    interactive_view::ViewInteractions,
    utils::Observer,
    workspaces::{
        app_icons_manager::AppIconsManager, apps_info::ApplicationsInfo, Application,
        WorkspacesModel,
    },
};

use super::{model::AppSwitcherModel, render_app::render_appswitcher_panel};

#[derive(Debug, Clone)]
pub struct AppSwitcherView {
    pub wrap_layer: layers::prelude::Layer,
    pub view: View<AppSwitcherModel>,
    app_icons_manager: Arc<AppIconsManager>,
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
    pub fn new(layers_engine: Arc<Engine>, app_icons_manager: Arc<AppIconsManager>) -> Self {
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

        let view_layer = layers_engine.new_layer();
        let _ = wrap.add_sublayer(&view_layer);

        let view = View::new(
            "app_switcher_view",
            AppSwitcherModel::default(),
            render_appswitcher_panel,
        );
        view.set_layer(view_layer);

        let (notify_tx, notify_rx) = mpsc::channel(5);

        let switcher = Self {
            wrap_layer: wrap,
            view,
            app_icons_manager,
            active: Arc::new(AtomicBool::new(false)),
            notify_tx,
            latest_event: Arc::new(tokio::sync::RwLock::new(None)),
        };
        switcher.init_notification_handler(notify_rx);
        switcher
    }

    fn update_state(&self, new_state: AppSwitcherModel) {
        self.view.update_state(&new_state);
    }

    pub fn next(&self) {
        let mut state = self.view.get_state();
        if !self.active.load(Ordering::Relaxed) {
            state.current_app = 0;
        }
        if !state.apps.is_empty() {
            state.current_app = (state.current_app + 1) % state.apps.len();
        } else {
            state.current_app = 0;
        }
        // Re-fetch icon stacks so we never hold stale NodeRefs from removed dock layers.
        let fresh = self.build_model_with_stacks(state.apps, state.current_app, state.width);
        self.view.update_state(&fresh);
        self.active.store(true, Ordering::Relaxed);
        self.wrap_layer.set_hidden(false);
        self.wrap_layer.set_opacity(
            1.0,
            Some(Transition {
                delay: 0.1,
                timing: TimingFunction::ease_out_quad(0.3),
            }),
        );
    }

    pub fn previous(&self) {
        let mut state = self.view.get_state();
        if !state.apps.is_empty() {
            state.current_app = (state.current_app + state.apps.len() - 1) % state.apps.len();
        } else {
            state.current_app = 0;
        }
        // Re-fetch icon stacks so we never hold stale NodeRefs from removed dock layers.
        let fresh = self.build_model_with_stacks(state.apps, state.current_app, state.width);
        self.view.update_state(&fresh);
        self.active.store(true, Ordering::Relaxed);
        self.wrap_layer.set_hidden(false);
        self.wrap_layer.set_opacity(
            1.0,
            Some(Transition {
                delay: 0.05,
                timing: TimingFunction::linear(0.3),
            }),
        );
    }

    pub fn hide(&self) -> layers::engine::TransactionRef {
        self.active.store(false, Ordering::Relaxed);
        let tr = self
            .wrap_layer
            .set_opacity(0.0, Some(Transition::ease_in_quad(0.05)));
        tr.on_finish(
            move |l: &layers::prelude::Layer, _p: f32| {
                l.set_hidden(true);
            },
            true,
        );
        tr
    }

    pub fn reset(&self) {
        let mut state = self.view.get_state();
        state.current_app = 0;
        self.view.update_state(&state);
    }

    pub fn get_current_app_id(&self) -> Option<String> {
        let state = self.view.get_state();
        state
            .apps
            .get(state.current_app)
            .map(|app| app.identifier.clone())
    }

    fn build_model_with_stacks(
        &self,
        apps: Vec<Application>,
        current_app: usize,
        width: i32,
    ) -> AppSwitcherModel {
        let icon_stacks = apps
            .iter()
            .map(|app| {
                self.app_icons_manager
                    .get_stack(&app.match_id)
                    .map(|layer| layer.id())
            })
            .collect();
        AppSwitcherModel {
            apps,
            current_app,
            width,
            icon_stacks,
        }
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
                            if let Some(app) = ApplicationsInfo::get_app_info_by_id(app_id).await {
                                apps.push(app);
                            }
                        }
                    }
                    let current_app = {
                        let s = this.view.get_state();
                        if apps.is_empty() {
                            0
                        } else if s.current_app >= apps.len() {
                            apps.len() - 1
                        } else {
                            s.current_app
                        }
                    };
                    let new_state =
                        this.build_model_with_stacks(apps, current_app, workspace.width);
                    this.update_state(new_state);
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
