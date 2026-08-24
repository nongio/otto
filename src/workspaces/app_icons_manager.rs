use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use layers::{
    engine::{animation::Transition, Engine},
    prelude::{taffy, Layer},
    taffy::style::Style,
    types::Size,
    view::{BuildLayerTree, LayerTreeBuilder},
};

use crate::workspaces::{
    dock::{
        draw_app_icon, draw_badge, draw_progress, setup_badge_layer, setup_progress_layer,
        BASE_ICON_SIZE,
    },
    Application,
};

struct AppIconEntry {
    pub stack: Layer,
    pub icon_layer: Layer,
    pub badge_layer: Layer,
    pub progress_layer: Layer,
    pub icon_id: Option<u32>,
}

/// Owns a persistent, hidden icon stack (icon + badge + progress) for every known app.
///
/// Both the dock and the app switcher hold mirror layers that replicate from these stacks.
/// Stacks are append-only — they are never freed, so `NodeRef`s pointing at them remain
/// valid for the lifetime of the compositor session.
pub struct AppIconsManager {
    engine: Arc<Engine>,
    /// Container for all icon stacks. Pointer events are disabled so it doesn't
    /// interfere with interaction, but it participates in layout so that
    /// `render_node_tree` can produce output for mirror followers.
    pub container: Layer,
    entries: RwLock<HashMap<String, AppIconEntry>>,
    /// Badge text per canonical app key, kept even when no icon stack exists
    /// yet: a notification can badge an app before the dock has built its
    /// icon, and the badge is applied as soon as the stack appears.
    badges: RwLock<HashMap<String, String>>,
    /// Memoized `app_id` → canonical key resolution (see `canonical_key`).
    key_cache: RwLock<HashMap<String, String>>,
}

impl std::fmt::Debug for AppIconsManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppIconsManager")
            .field("container", &self.container)
            .finish()
    }
}

impl AppIconsManager {
    pub fn new(engine: Arc<Engine>) -> Self {
        let container = engine.new_layer();
        container.set_key("app_icons_manager");
        container.set_pointer_events(false);
        container.set_layout_style(Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        Self {
            engine,
            container,
            entries: RwLock::new(HashMap::new()),
            badges: RwLock::new(HashMap::new()),
            key_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Return the icon stack for `app_id`, creating it if it does not yet exist.
    /// Stacks are never removed, so the returned `Layer` (and its `NodeRef`) stays valid forever.
    pub fn get_or_create_stack(&self, app_id: &str, app: &Application) -> Layer {
        {
            let entries = self.entries.read().unwrap();
            if let Some(entry) = entries.get(app_id) {
                return entry.stack.clone();
            }
        }

        let stack = self.engine.new_layer();
        let icon_layer = self.engine.new_layer();
        let badge_layer = self.engine.new_layer();
        let progress_layer = self.engine.new_layer();

        let stack_tree = LayerTreeBuilder::default()
            .key(format!("icon_stack_{}", app_id))
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
        stack.build_layer_tree(&stack_tree);

        let icon_tree = LayerTreeBuilder::default()
            .key("icon")
            .layout_style(taffy::Style {
                display: taffy::Display::Block,
                position: taffy::Position::Relative,
                ..Default::default()
            })
            .size((
                Size {
                    width: taffy::Dimension::Percent(1.0),
                    height: taffy::Dimension::Percent(1.0),
                },
                None,
            ))
            .pointer_events(false)
            .picture_cached(false)
            .image_cache(false)
            .content(Some(draw_app_icon(app)))
            .build()
            .unwrap();
        icon_layer.build_layer_tree(&icon_tree);
        icon_layer.set_image_cached(true);

        setup_badge_layer(&badge_layer, BASE_ICON_SIZE);
        setup_progress_layer(&progress_layer, BASE_ICON_SIZE);

        let _ = self.container.add_sublayer(&stack);
        let _ = stack.add_sublayer(&icon_layer);
        let _ = stack.add_sublayer(&badge_layer);
        let _ = stack.add_sublayer(&progress_layer);

        let icon_id = app.icon.as_ref().map(|i| i.unique_id());
        self.entries.write().unwrap().insert(
            app_id.to_string(),
            AppIconEntry {
                stack: stack.clone(),
                icon_layer,
                badge_layer,
                progress_layer,
                icon_id,
            },
        );

        // A badge may have been set before this stack existed (a notification
        // arriving for an app the dock had not drawn yet) — apply it now. It
        // may also be filed under a looser id than this key, e.g. "Ghostty"
        // for `com.mitchellh.ghostty`, since there was nothing to resolve
        // against at the time; re-file it under the key that now exists.
        let pending = {
            let mut badges = self.badges.write().unwrap();
            let loose_key = badges
                .keys()
                .find(|key| key.as_str() != app_id && matches_app_key(app_id, key))
                .cloned();
            if let Some(loose_key) = loose_key {
                if let Some(text) = badges.remove(&loose_key) {
                    badges.insert(app_id.to_string(), text);
                }
            }
            badges.get(app_id).cloned()
        };
        if let Some(text) = pending {
            let entries = self.entries.read().unwrap();
            if let Some(entry) = entries.get(app_id) {
                entry.badge_layer.set_draw_content(draw_badge(text));
                entry.badge_layer.set_opacity(1.0_f32, None);
            }
        }

        stack
    }

    /// Return the icon stack for `app_id` if it has been created, or `None`.
    pub fn get_stack(&self, app_id: &str) -> Option<Layer> {
        self.entries
            .read()
            .unwrap()
            .get(app_id)
            .map(|e| e.stack.clone())
    }

    /// Redraw the icon if `app`'s icon has changed since the last call.
    pub fn update_app(&self, app_id: &str, app: &Application) {
        let current_icon_id = app.icon.as_ref().map(|i| i.unique_id());
        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.get_mut(app_id) {
            if entry.icon_id != current_icon_id {
                entry.icon_layer.set_draw_content(draw_app_icon(app));
                entry.icon_id = current_icon_id;
            }
        }
    }

    /// Resolve an arbitrary `app_id` to the key the icon stacks are filed
    /// under (`Application::match_id`, i.e. the desktop file stem).
    ///
    /// A notification carries whatever the sending app put in its
    /// `desktop-entry` hint — `com.mitchellh.ghostty`, `Ghostty`,
    /// `ghostty.desktop` — while the dock files its icon under the desktop
    /// file stem. Without this, badges from a notification daemon would land
    /// on a key nothing draws.
    pub fn canonical_key(&self, app_id: &str) -> String {
        if let Some(cached) = self.key_cache.read().unwrap().get(app_id) {
            return cached.clone();
        }

        let resolved = self.resolve_key(app_id);
        // Only memoize a resolution that actually landed somewhere. Falling
        // back to the raw id can happen simply because the app's icon stack
        // does not exist yet, and that answer must not be cached forever.
        if resolved != app_id {
            self.key_cache
                .write()
                .unwrap()
                .insert(app_id.to_string(), resolved.clone());
        }
        resolved
    }

    fn resolve_key(&self, app_id: &str) -> String {
        {
            let entries = self.entries.read().unwrap();
            if entries.contains_key(app_id) {
                return app_id.to_string();
            }
        }

        // The dock's own resolution path: desktop entry → file stem.
        if let Some(stem) =
            otto_kit::desktop_entry::lookup_app(app_id).and_then(|info| info.desktop_file_id)
        {
            return stem;
        }

        // No desktop entry. An app_name like "Ghostty" can still name a known
        // icon stack whose key ends in that segment (`com.mitchellh.ghostty`).
        let entries = self.entries.read().unwrap();
        if let Some(key) = entries.keys().find(|key| matches_app_key(key, app_id)) {
            return key.clone();
        }

        app_id.to_string()
    }

    /// Show or hide the badge on the dock/switcher icon for `app_id`.
    ///
    /// The text is remembered per app, so an icon stack built later still
    /// shows it.
    pub fn update_badge(&self, app_id: &str, text: Option<String>) {
        let app_id = &self.canonical_key(app_id);
        match text.as_deref() {
            Some(t) if !t.is_empty() => {
                self.badges
                    .write()
                    .unwrap()
                    .insert(app_id.clone(), t.to_string());
            }
            _ => {
                self.badges.write().unwrap().remove(app_id);
            }
        }

        let entries = self.entries.read().unwrap();
        if let Some(entry) = entries.get(app_id) {
            match text {
                Some(t) if !t.is_empty() => {
                    entry.badge_layer.set_draw_content(draw_badge(t));
                    entry
                        .badge_layer
                        .set_opacity(1.0_f32, Some(Transition::ease_in_quad(0.15)));
                }
                _ => {
                    entry
                        .badge_layer
                        .set_opacity(0.0_f32, Some(Transition::ease_in_quad(0.15)));
                }
            }
        }
    }

    /// Show or hide the progress bar on the dock/switcher icon for `app_id`.
    pub fn update_progress(&self, app_id: &str, value: Option<f64>) {
        let app_id = &self.canonical_key(app_id);
        let entries = self.entries.read().unwrap();
        if let Some(entry) = entries.get(app_id) {
            match value {
                Some(v) if v >= 0.0 => {
                    entry
                        .progress_layer
                        .set_draw_content(draw_progress(v.clamp(0.0, 1.0)));
                    entry
                        .progress_layer
                        .set_opacity(1.0_f32, Some(Transition::ease_in_quad(0.15)));
                }
                _ => {
                    entry
                        .progress_layer
                        .set_opacity(0.0_f32, Some(Transition::ease_in_quad(0.15)));
                }
            }
        }
    }
}

/// Whether `loose` names the app filed under `key` — the same string, or the
/// last dotted segment of a reverse-DNS key ("Ghostty" for
/// `com.mitchellh.ghostty`). Both comparisons ignore case and a trailing
/// `.desktop`.
fn matches_app_key(key: &str, loose: &str) -> bool {
    let key = key.strip_suffix(".desktop").unwrap_or(key);
    let loose = loose.strip_suffix(".desktop").unwrap_or(loose);
    if key.eq_ignore_ascii_case(loose) {
        return true;
    }
    key.rsplit('.')
        .next()
        .map(|seg| seg.eq_ignore_ascii_case(loose))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspaces::apps_info::Application;

    fn manager() -> (Arc<Engine>, AppIconsManager) {
        let engine = Engine::create(1000.0, 1000.0);
        let icons = AppIconsManager::new(engine.clone());
        (engine, icons)
    }

    fn settle(engine: &Arc<Engine>) {
        for _ in 0..60 {
            engine.update(0.016);
        }
    }

    fn badge_opacity(icons: &AppIconsManager, app_id: &str) -> f32 {
        icons
            .entries
            .read()
            .unwrap()
            .get(app_id)
            .expect("icon stack")
            .badge_layer
            .opacity()
    }

    #[test]
    fn badge_survives_an_icon_stack_that_does_not_exist_yet() {
        let (engine, icons) = manager();

        // The notification lands before the dock has drawn the app.
        icons.update_badge("com.example.mail", Some("3".into()));
        icons.get_or_create_stack(
            "com.example.mail",
            &Application::test_new("com.example.mail"),
        );
        settle(&engine);

        assert_eq!(badge_opacity(&icons, "com.example.mail"), 1.0);
    }

    #[test]
    fn clearing_a_badge_hides_it() {
        let (engine, icons) = manager();
        icons.get_or_create_stack(
            "com.example.mail",
            &Application::test_new("com.example.mail"),
        );
        icons.update_badge("com.example.mail", Some("3".into()));
        settle(&engine);
        assert_eq!(badge_opacity(&icons, "com.example.mail"), 1.0);

        icons.update_badge("com.example.mail", None);
        settle(&engine);
        assert_eq!(badge_opacity(&icons, "com.example.mail"), 0.0);
        assert!(icons.badges.read().unwrap().is_empty());
    }

    #[test]
    fn a_loose_app_name_badges_the_stack_it_names() {
        let (engine, icons) = manager();

        // No desktop entry to resolve against, and no stack yet: the badge is
        // parked under the name the notification used.
        icons.update_badge("Mail", Some("1".into()));
        icons.get_or_create_stack(
            "com.example.mail",
            &Application::test_new("com.example.mail"),
        );
        settle(&engine);
        assert_eq!(badge_opacity(&icons, "com.example.mail"), 1.0);

        // …and the next update for the same loose name finds it.
        icons.update_badge("Mail", Some("2".into()));
        settle(&engine);
        assert_eq!(
            icons.badges.read().unwrap().get("com.example.mail"),
            Some(&"2".to_string())
        );
    }

    #[test]
    fn app_key_matching_ignores_case_and_desktop_suffix() {
        assert!(matches_app_key("com.mitchellh.ghostty", "Ghostty"));
        assert!(matches_app_key("com.mitchellh.ghostty", "ghostty.desktop"));
        assert!(matches_app_key("ghostty", "Ghostty"));
        assert!(!matches_app_key("com.mitchellh.ghostty", "mail"));
    }
}
