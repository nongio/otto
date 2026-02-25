use std::sync::Arc;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
};

use freedesktop_desktop_entry::DesktopEntry;

use layers::skia;

use crate::{
    config::Config,
    utils::{find_icon_with_theme, image_from_path},
};

#[derive(Clone)]
pub struct Application {
    pub identifier: String,
    pub match_id: String,
    pub icon_path: Option<String>,
    pub icon: Option<skia::Image>,
    pub picture: Option<skia::Picture>,
    pub override_name: Option<String>,
    pub desktop_file_id: Option<String>,
    desktop_entry: Option<DesktopEntry>,
}

impl Application {
    pub fn desktop_name(&self) -> Option<String> {
        if let Some(name) = &self.override_name {
            return Some(name.clone());
        }
        Config::with(|c| {
            self.desktop_entry
                .as_ref()
                .and_then(|entry| entry.name(&c.locales))
                .map(|name| name.to_string())
        })
    }
    pub fn command(&self, extra_args: &[String]) -> Option<(String, Vec<String>)> {
        let exec = self.desktop_entry.as_ref()?.exec()?;
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

    async fn get_desktop_entry(app_id: &str) -> Option<DesktopEntry> {
        // Normalize the app_id - remove .desktop suffix if present
        let normalized_id = if let Some(stripped) = app_id.strip_suffix(".desktop") {
            stripped
        } else {
            app_id
        };

        tracing::trace!(normalized_id = %normalized_id, "[get_desktop_entry] searching");

        // Exact filename match (case-insensitive)
        let entry_path = freedesktop_desktop_entry::Iter::new(
            freedesktop_desktop_entry::default_paths(),
        )
        .find(|path| {
            if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
                file_stem.eq_ignore_ascii_case(normalized_id)
            } else {
                false
            }
        });

        if let Some(entry_path) = entry_path {
            tracing::trace!(path = ?entry_path, "[get_desktop_entry] found");
            let locales = &["en"];
            return DesktopEntry::from_path(entry_path, Some(locales)).ok();
        }

        tracing::trace!(normalized_id = %normalized_id, "[get_desktop_entry] not found");
        None
    }

    async fn load_app_info(app_id: &str) -> Option<Application> {
        tracing::trace!(app_id = %app_id, "[load_app_info] start");
        let desktop_entry = ApplicationsInfo::get_desktop_entry(app_id).await;

        if let Some(desktop_entry) = desktop_entry {
            let match_id = desktop_entry.id().to_string();
            let identifier = if app_id.ends_with(".desktop") {
                match_id.clone()
            } else {
                app_id.to_string()
            };
            let icon_name = desktop_entry.icon().map(|icon| icon.to_string());

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
            // let picture = icon_path
            //     .as_ref()
            //     .and_then(|icon_path| {
            //         let path = std::path::Path::new(icon_path);
            //         if path.extension().and_then(std::ffi::OsStr::to_str) == Some("svg") {
            //             if let Ok(svg) = svg_dom(icon_path, (100, 100)) {
            //                 let mut rec = skia::PictureRecorder::new();
            //                 let canvas = rec.begin_recording(skia::Rect::from_iwh(512, 512), None);
            //                 svg.render(&canvas);
            //                 // let paint = skia::Paint::new(skia::Color4f::new(1.0, 0.0, 0.0, 1.0), None);
            //                 // canvas.draw_circle((50.0, 50.0), 50.0, &paint);
            //                 return rec.finish_recording_as_picture(None)
            //             }
            //         }
            //         None
            //     });

            let desktop_file_id = desktop_entry
                .path
                .file_stem()
                .and_then(|os| os.to_str())
                .map(|s| s.to_string());

            let app = Application {
                identifier,
                match_id,
                icon_path,
                icon,
                picture: None,
                override_name: None,
                desktop_file_id,
                desktop_entry: Some(desktop_entry),
            };

            return Some(app);
        }

        // No desktop entry found - create minimal Application with fallback icon
        tracing::debug!("[load_app_info] no desktop entry; creating fallback application");

        let fallback_icon_path = find_icon_with_theme("application-default-icon", 512, 1)
            .or_else(|| {
                tracing::trace!("[load_app_info] application-default-icon not found; trying application-x-executable");
                find_icon_with_theme("application-x-executable", 512, 1)
            });

        let fallback_icon = fallback_icon_path.as_ref().and_then(|path| {
            let result = image_from_path(path, (512, 512));
            tracing::trace!(
                loaded = result.is_some(),
                "[load_app_info] fallback application icon loaded"
            );
            result
        });

        // Format the app_id (executable name) as a nice display name
        let display_name = app_id
            .split('/')
            .next_back()
            .unwrap_or(app_id)
            .split('-')
            .map(|word| {
                let mut c = word.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        if fallback_icon.is_some() {
            tracing::debug!(display_name = %display_name, "[load_app_info] fallback application created");
        } else {
            tracing::warn!(display_name = %display_name, "[load_app_info] fallback application created without icon");
        }

        Some(Application {
            identifier: app_id.to_string(),
            match_id: app_id.to_string(),
            icon_path: fallback_icon_path,
            icon: fallback_icon,
            picture: None,
            override_name: Some(display_name),
            desktop_file_id: None,
            desktop_entry: None,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_desktop_entry_matching_logic() {
        // Test exact filename matching (case-insensitive)
        let test_cases = vec![
            // (app_id, desktop_file_stem, should_match)
            ("thunar", "thunar", true),
            ("thunar", "thunar-bulk-rename", false),
            ("thunar", "thunar-settings", false),
            ("firefox", "firefox", true),
            ("firefox", "firefox-esr", false),
            ("code", "code", true),
            ("code", "code-url-handler", false),
            ("Thunar", "thunar", true), // Case insensitive
            ("THUNAR", "thunar", true),
            ("org.kde.dolphin", "org.kde.dolphin", true),
            ("org.gnome.gedit", "org.gnome.gedit", true),
            ("io.elementary.files", "io.elementary.files", true),
            ("com.mitchellh.ghostty", "com.mitchellh.ghostty", true),
        ];
        #[allow(clippy::manual_strip)]
        for (app_id, file_stem, expected_match) in test_cases {
            let normalized_id = if app_id.ends_with(".desktop") {
                &app_id[..app_id.len() - 8]
            } else {
                app_id
            };

            let exact_match = file_stem.eq_ignore_ascii_case(normalized_id);

            assert_eq!(
                exact_match, expected_match,
                "Match failed for app_id='{}' vs file_stem='{}' (expected: {})",
                app_id, file_stem, expected_match
            );
        }
    }
}
