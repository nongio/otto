use std::hash::{Hash, Hasher};

use smithay::reexports::wayland_server::backend::ObjectId;

use crate::workspaces::Application;

#[derive(Debug, Clone, Default)]
pub struct DockModel {
    pub launchers: Vec<Application>,
    pub running_apps: Vec<Application>,
    pub minimized_windows: Vec<(ObjectId, String)>,
    pub width: i32,
    pub focus: f32,
}

impl Hash for DockModel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.launchers.hash(state);
        self.running_apps.hash(state);
        self.minimized_windows.hash(state);
        self.width.hash(state);
    }
}

impl DockModel {
    pub fn new() -> Self {
        Self {
            focus: -500.0,
            ..Default::default()
        }
    }

    /// Merge launchers with running apps into a display list.
    /// Each entry is `(app, is_running)`. Launchers matched by `match_id`
    /// to a running app get `is_running = true`. Running apps not in
    /// launchers are appended at the end.
    pub fn display_entries(&self) -> Vec<(Application, bool)> {
        let mut entries: Vec<(Application, bool)> = self
            .launchers
            .iter()
            .map(|launcher| (launcher.clone(), false))
            .collect();

        for running in self.running_apps.iter() {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app(id: &str) -> Application {
        Application::test_new(id)
    }

    #[test]
    fn no_running_apps_all_launchers_not_running() {
        let model = DockModel {
            launchers: vec![make_app("firefox"), make_app("terminal")],
            running_apps: vec![],
            ..DockModel::new()
        };
        let entries = model.display_entries();
        assert_eq!(entries.len(), 2);
        assert!(!entries[0].1, "firefox should not be running");
        assert!(!entries[1].1, "terminal should not be running");
    }

    #[test]
    fn running_app_matches_launcher() {
        let model = DockModel {
            launchers: vec![make_app("firefox"), make_app("terminal")],
            running_apps: vec![make_app("firefox")],
            ..DockModel::new()
        };
        let entries = model.display_entries();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].1, "firefox should be running");
        assert!(!entries[1].1, "terminal should not be running");
    }

    #[test]
    fn running_app_not_in_launchers_appended() {
        let model = DockModel {
            launchers: vec![make_app("firefox")],
            running_apps: vec![make_app("spotify")],
            ..DockModel::new()
        };
        let entries = model.display_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0.match_id, "firefox");
        assert!(!entries[0].1);
        assert_eq!(entries[1].0.match_id, "spotify");
        assert!(entries[1].1, "spotify should be running");
    }

    #[test]
    fn multiple_running_apps_mixed() {
        let model = DockModel {
            launchers: vec![make_app("firefox"), make_app("terminal"), make_app("files")],
            running_apps: vec![make_app("terminal"), make_app("chromium")],
            ..DockModel::new()
        };
        let entries = model.display_entries();
        assert_eq!(entries.len(), 4);
        assert!(!entries[0].1, "firefox not running");
        assert!(entries[1].1, "terminal running");
        assert!(!entries[2].1, "files not running");
        assert_eq!(entries[3].0.match_id, "chromium");
        assert!(entries[3].1, "chromium running");
    }

    #[test]
    fn override_name_preserved_from_launcher() {
        let mut launcher = make_app("firefox");
        launcher.override_name = Some("My Browser".to_string());
        let model = DockModel {
            launchers: vec![launcher],
            running_apps: vec![make_app("firefox")],
            ..DockModel::new()
        };
        let entries = model.display_entries();
        assert_eq!(
            entries[0].0.override_name,
            Some("My Browser".to_string()),
            "override_name from launcher must be preserved"
        );
        assert!(entries[0].1);
    }
}
