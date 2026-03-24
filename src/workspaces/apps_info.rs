use std::sync::Arc;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
};

use layers::skia;

use crate::utils::{find_icon_with_theme, image_from_path};

#[derive(Clone)]
pub struct Application {
    pub identifier: String,
    pub match_id: String,
    pub icon_path: Option<String>,
    pub icon: Option<skia::Image>,
    pub picture: Option<skia::Picture>,
    pub override_name: Option<String>,
    pub desktop_file_id: Option<String>,
    app_info: Option<otto_kit::desktop_entry::AppInfo>,
}

impl Application {
    pub fn desktop_name(&self) -> Option<String> {
        if let Some(name) = &self.override_name {
            return Some(name.clone());
        }
        self.app_info.as_ref().map(|info| info.name.clone())
    }
    pub fn command(&self, extra_args: &[String]) -> Option<(String, Vec<String>)> {
        let exec = self.app_info.as_ref()?.exec.as_ref()?;
        let mut parts = shell_words::split(exec).ok()?;
        if parts.is_empty() {
            return None;
        }
        let cmd = parts.remove(0);
        let mut args: Vec<String> = parts
            .into_iter()
            .filter_map(|arg| {
                if arg.starts_with('%') {
                    None
                } else {
                    Some(arg)
                }
            })
            .collect();
        args.extend(extra_args.iter().cloned());
        Some((cmd, args))
    }
}

impl Hash for Application {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.match_id.hash(state);
        self.icon_path.hash(state);
        self.override_name.hash(state);

        if let Some(i) = self.icon.as_ref() {
            i.unique_id().hash(state)
        }
    }
}

impl PartialEq for Application {
    fn eq(&self, other: &Self) -> bool {
        self.match_id == other.match_id
    }
}
impl Eq for Application {}

impl std::fmt::Debug for Application {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Application")
            .field("identifier", &self.identifier)
            .field("match_id", &self.match_id)
            .field("desktop_file_id", &self.desktop_file_id)
            .field("icon_path", &self.icon_path)
            .field("icon", &self.icon.is_some())
            .field("override_name", &self.override_name)
            .finish()
    }
}

type AppsInfoStorage = HashMap<String, Application>;

fn applications_info() -> &'static Arc<tokio::sync::RwLock<AppsInfoStorage>> {
    static INSTANCE: std::sync::OnceLock<Arc<tokio::sync::RwLock<HashMap<String, Application>>>> =
        std::sync::OnceLock::new();

    INSTANCE.get_or_init(|| Arc::new(tokio::sync::RwLock::new(HashMap::new())))
}

pub struct ApplicationsInfo;

impl ApplicationsInfo {
    pub async fn get_app_info_by_id(app_id: impl Into<String>) -> Option<Application> {
        let app_id = app_id.into();
        tracing::trace!(app_id = %app_id, "[ApplicationsInfo] app info requested");
        let mut applications = applications_info().write().await;
        let mut app = { applications.get(&app_id).cloned() };
        if app.is_none() {
            tracing::debug!(app_id = %app_id, "[ApplicationsInfo] cache miss; loading");
            if let Some(new_app) = ApplicationsInfo::load_app_info(&app_id).await {
                tracing::trace!(
                    app_id = %app_id,
                    has_icon = new_app.icon.is_some(),
                    desktop_file_id = ?new_app.desktop_file_id,
                    "[ApplicationsInfo] loaded"
                );
                applications.insert(app_id.clone(), new_app.clone());
                app = Some(new_app);
            } else {
                tracing::warn!(app_id = %app_id, "[ApplicationsInfo] failed to load app info");
            }
        } else {
            tracing::trace!(app_id = %app_id, "[ApplicationsInfo] cache hit");
        }

        app
    }

    async fn load_app_info(app_id: &str) -> Option<Application> {
        tracing::trace!(app_id = %app_id, "[load_app_info] start");

        // Use otto-kit's desktop entry lookup for metadata
        let info = otto_kit::desktop_entry::lookup_app(app_id);

        let icon_name = info.as_ref().and_then(|i| i.icon_name.clone());
        let desktop_file_id = info.as_ref().and_then(|i| i.desktop_file_id.clone());

        let icon_path =
            icon_name.and_then(|icon_name| find_icon_with_theme(&icon_name, 512, 1));

        let mut icon = icon_path
            .as_ref()
            .and_then(|icon_path| image_from_path(icon_path, (512, 512)));

        // If icon loading failed, try to use the fallback icon
        if icon.is_none() {
            let fallback_path = find_icon_with_theme("application-default-icon", 512, 1)
                .or_else(|| find_icon_with_theme("application-x-executable", 512, 1));

            icon = fallback_path.as_ref().and_then(|fallback_path| {
                let result = image_from_path(fallback_path, (512, 512));
                tracing::trace!(
                    loaded = result.is_some(),
                    "[load_app_info] fallback icon loaded"
                );
                result
            });
        }

        if let Some(info) = info {
            let match_id = info.desktop_file_id.clone().unwrap_or_else(|| app_id.to_string());
            let identifier = if app_id.ends_with(".desktop") {
                match_id.clone()
            } else {
                app_id.to_string()
            };

            Some(Application {
                identifier,
                match_id,
                icon_path,
                icon,
                picture: None,
                override_name: None,
                desktop_file_id,
                app_info: Some(info),
            })
        } else {
            // No desktop entry found - create minimal Application with fallback icon
            tracing::debug!("[load_app_info] no desktop entry; creating fallback application");

            let display_name = otto_kit::desktop_entry::display_name_for_app(app_id);

            if icon.is_some() {
                tracing::debug!(display_name = %display_name, "[load_app_info] fallback application created");
            } else {
                tracing::warn!(display_name = %display_name, "[load_app_info] fallback application created without icon");
            }

            Some(Application {
                identifier: app_id.to_string(),
                match_id: app_id.to_string(),
                icon_path,
                icon,
                picture: None,
                override_name: Some(display_name),
                desktop_file_id: None,
                app_info: None,
            })
        }
    }
}

// Tests for desktop entry matching are now in otto_kit::desktop_entry
