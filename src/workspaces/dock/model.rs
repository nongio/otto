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

/// Reorder `bookmarks` to follow `order`, a list of match ids.
///
/// A bookmark the dock never loaded — one whose desktop entry is missing, say —
/// is not in `order`; it keeps its relative place at the end rather than being
/// dropped, so a reorder never costs the user an entry they cannot see.
pub fn sort_bookmarks_to(bookmarks: &mut [crate::config::DockBookmark], order: &[String]) {
    bookmarks.sort_by_key(|bookmark| {
        let id = bookmark
            .desktop_id
            .strip_suffix(".desktop")
            .unwrap_or(&bookmark.desktop_id);
        order
            .iter()
            .position(|known| known == id)
            .unwrap_or(usize::MAX)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bookmark(id: &str) -> crate::config::DockBookmark {
        crate::config::DockBookmark {
            desktop_id: id.to_string(),
            label: None,
            exec_args: vec![],
        }
    }

    fn ids(bookmarks: &[crate::config::DockBookmark]) -> Vec<String> {
        bookmarks.iter().map(|b| b.desktop_id.clone()).collect()
    }

    fn order(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|e| e.to_string()).collect()
    }

    #[test]
    fn bookmarks_follow_the_launcher_order() {
        let mut bookmarks = vec![bookmark("firefox"), bookmark("terminal"), bookmark("files")];
        sort_bookmarks_to(&mut bookmarks, &order(&["files", "firefox", "terminal"]));
        assert_eq!(ids(&bookmarks), vec!["files", "firefox", "terminal"]);
    }

    #[test]
    fn desktop_suffix_still_matches() {
        let mut bookmarks = vec![bookmark("firefox.desktop"), bookmark("terminal")];
        sort_bookmarks_to(&mut bookmarks, &order(&["terminal", "firefox"]));
        assert_eq!(ids(&bookmarks), vec!["terminal", "firefox.desktop"]);
    }

    #[test]
    fn unknown_bookmarks_keep_their_order_at_the_end() {
        let mut bookmarks = vec![
            bookmark("ghost-one"),
            bookmark("firefox"),
            bookmark("ghost-two"),
            bookmark("terminal"),
        ];
        sort_bookmarks_to(&mut bookmarks, &order(&["terminal", "firefox"]));
        assert_eq!(
            ids(&bookmarks),
            vec!["terminal", "firefox", "ghost-one", "ghost-two"],
            "bookmarks the dock could not load must survive a reorder"
        );
    }

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
