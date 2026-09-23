//! Open With: the chooser's state, apart from any window.
//!
//! The chooser lists the applications that can open the selection — the
//! default first — and, folded under a heading, every other application
//! installed. A query narrows both. The selection is opened either just this
//! once or, with the box ticked, with the choice remembered as the type's
//! default. See `specs/open-with.md`.
//!
//! Everything here is plain data, so the list, the keyboard and the answer
//! can be tested without a compositor; the window that draws it lives in
//! `app`.

use std::path::PathBuf;

use otto_kit::filetype;
use otto_kit::mime_apps::{App, Associations};

/// The type of a file nothing identifies. Every file is one, so no choice
/// is remembered for it.
const UNKNOWN_TYPE: &str = "application/octet-stream";

/// One line of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row<'a> {
    /// An application, and whether it is the one the type opens with now.
    App { app: &'a App, is_default: bool },
    /// The heading every other application is folded under. Picking it
    /// unfolds or folds them.
    Others { open: bool },
}

/// An Open With chooser for a set of files.
#[derive(Debug)]
pub struct Chooser {
    /// What is being opened.
    pub paths: Vec<PathBuf>,
    /// The file's name, or how many files there are.
    pub title: String,
    /// The type every file shares, when they share one. Only then can a
    /// choice be remembered: one "always" cannot answer for two types.
    pub mime: Option<String>,
    /// What the shared type is called, for the header and the checkbox.
    pub type_name: Option<String>,
    /// Icon names for the header, most specific first.
    pub icon_names: Vec<String>,
    /// The application the shared type opens with now.
    pub default_id: Option<String>,
    /// The applications that can open every file, the default first.
    pub suggested: Vec<App>,
    /// Everything else installed, by name.
    pub others: Vec<App>,
    /// What has been typed into the search field.
    pub query: String,
    /// Whether the other applications are unfolded.
    pub others_open: bool,
    /// The highlighted row, as an index into [`Chooser::rows`].
    pub highlight: Option<usize>,
    /// Whether the choice is to be remembered.
    pub always: bool,
    /// Why the last attempt to open failed, shown in place.
    pub error: Option<String>,
}

impl Chooser {
    /// A chooser for `paths`, drawing its applications from `associations`.
    ///
    /// # Panics
    ///
    /// If `paths` is empty: there is nothing to choose an application for,
    /// and a caller asking for a chooser anyway is a bug.
    pub fn new(paths: Vec<PathBuf>, associations: &Associations) -> Self {
        assert!(!paths.is_empty(), "an Open With chooser needs a file");

        let mut mimes: Vec<String> = Vec::new();
        for path in &paths {
            // The same question a plain Open asks, so the default shown is
            // the one it uses.
            let mime = filetype::for_file(path).to_string();
            if !mimes.contains(&mime) {
                mimes.push(mime);
            }
        }
        let chains: Vec<Vec<String>> = mimes.iter().map(|m| filetype::ancestors(m)).collect();

        // Only an application that can open every file is suggested for all
        // of them. The first type's order stands.
        let suggested: Vec<App> = associations
            .apps_for(&chains[0])
            .into_iter()
            .filter(|app| {
                chains[1..]
                    .iter()
                    .all(|chain| associations.apps_for(chain).iter().any(|a| a.id == app.id))
            })
            .cloned()
            .collect();
        let others: Vec<App> = associations
            .all()
            .into_iter()
            .filter(|app| !suggested.iter().any(|s| s.id == app.id))
            .cloned()
            .collect();

        let shared = (mimes.len() == 1).then(|| mimes[0].clone());
        let default_id = shared
            .as_ref()
            .and_then(|_| associations.default_for(&chains[0]))
            .map(|app| app.id.clone());
        let type_name = shared
            .as_deref()
            .map(|mime| filetype::description(mime).unwrap_or_else(|| mime.to_string()));
        let icon_names = match &shared {
            Some(mime) => filetype::icon_names(mime),
            None => vec!["text-x-generic".to_string()],
        };
        let title = match paths.as_slice() {
            [only] => only
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            _ => otto_kit::t_owned!("files-open-with-count", count = paths.len() as f64),
        };

        let mut chooser = Self {
            paths,
            title,
            mime: shared,
            type_name,
            icon_names,
            default_id,
            suggested,
            others,
            query: String::new(),
            // With nothing to suggest, the full list is the only answer, so
            // it is not folded away behind a heading.
            others_open: false,
            highlight: None,
            always: false,
            error: None,
        };
        chooser.others_open = chooser.suggested.is_empty();
        chooser.highlight = chooser.first_app_row();
        chooser
    }

    /// The lines on show, top to bottom.
    ///
    /// With a query, only the applications whose name matches it, best
    /// first within each group, and the other applications unfolded: a
    /// search that only looked at the suggestions would miss what it was
    /// typed to find.
    pub fn rows(&self) -> Vec<Row<'_>> {
        let row = |app| Row::App {
            app,
            is_default: self.default_id.as_deref() == Some(app.id.as_str()),
        };
        let query = self.query.trim();
        if query.is_empty() {
            let mut rows: Vec<Row<'_>> = self.suggested.iter().map(row).collect();
            if !self.others.is_empty() {
                rows.push(Row::Others {
                    open: self.others_open,
                });
                if self.others_open {
                    rows.extend(self.others.iter().map(row));
                }
            }
            return rows;
        }
        let mut rows: Vec<Row<'_>> = matching(&self.suggested, query)
            .into_iter()
            .map(row)
            .collect();
        let others = matching(&self.others, query);
        if !others.is_empty() {
            rows.push(Row::Others { open: true });
            rows.extend(others.into_iter().map(row));
        }
        rows
    }

    /// The highlighted application, if the highlight is on one.
    pub fn selected(&self) -> Option<&App> {
        match self.rows().get(self.highlight?)? {
            Row::App { app, .. } => Some(app),
            Row::Others { .. } => None,
        }
    }

    /// Whether the box that remembers the choice is on offer at all.
    pub fn can_remember(&self) -> bool {
        self.mime
            .as_deref()
            .is_some_and(|mime| mime != UNKNOWN_TYPE)
    }

    /// Replace the query, and put the highlight on the best match.
    pub fn set_query(&mut self, query: &str) {
        if self.query == query {
            return;
        }
        self.query = query.to_string();
        self.highlight = self.first_app_row();
    }

    /// Move the highlight by `delta` rows, stopping at the ends.
    pub fn move_highlight(&mut self, delta: isize) {
        let count = self.rows().len();
        if count == 0 {
            self.highlight = None;
            return;
        }
        let next = match self.highlight {
            Some(current) => current.saturating_add_signed(delta).min(count - 1),
            None if delta < 0 => count - 1,
            None => 0,
        };
        self.highlight = Some(next);
    }

    /// Activate the highlighted row: an application is the answer, and the
    /// heading folds or unfolds what is under it.
    ///
    /// Returns the application to open with, if the row was one.
    pub fn activate(&mut self, index: usize) -> Option<App> {
        let app = match *self.rows().get(index)? {
            Row::App { app, .. } => Some(app.clone()),
            Row::Others { .. } => None,
        };
        self.highlight = Some(index);
        // A search always shows everything that matches, so there is nothing
        // to fold while one is typed.
        if app.is_none() && self.query.trim().is_empty() {
            self.others_open = !self.others_open;
        }
        app
    }

    /// Toggle whether the choice is remembered, where it can be.
    pub fn toggle_always(&mut self) {
        if self.can_remember() {
            self.always = !self.always;
        }
    }

    fn first_app_row(&self) -> Option<usize> {
        self.rows()
            .iter()
            .position(|row| matches!(row, Row::App { .. }))
    }
}

/// The applications whose name matches `query`, best first.
fn matching<'a>(apps: &'a [App], query: &str) -> Vec<&'a App> {
    let mut scored: Vec<(i32, &App)> = apps
        .iter()
        .filter_map(|app| otto_kit::matching::score(&app.name, query).map(|s| (s, app)))
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, app)| app).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str) -> App {
        App {
            id: id.to_string(),
            name: name.to_string(),
            icon_name: None,
            exec: Some("app %f".to_string()),
            terminal: false,
            working_dir: None,
            mime_types: Vec::new(),
            no_display: false,
            entry_path: PathBuf::new(),
        }
    }

    fn chooser(suggested: &[(&str, &str)], others: &[(&str, &str)]) -> Chooser {
        let suggested: Vec<App> = suggested.iter().map(|(id, name)| app(id, name)).collect();
        let default_id = suggested.first().map(|a| a.id.clone());
        let mut chooser = Chooser {
            paths: vec![PathBuf::from("/tmp/report.pdf")],
            title: "report.pdf".to_string(),
            mime: Some("application/pdf".to_string()),
            type_name: Some("PDF document".to_string()),
            icon_names: Vec::new(),
            default_id,
            suggested,
            others: others.iter().map(|(id, name)| app(id, name)).collect(),
            query: String::new(),
            others_open: false,
            highlight: None,
            always: false,
            error: None,
        };
        chooser.others_open = chooser.suggested.is_empty();
        chooser.highlight = chooser.first_app_row();
        chooser
    }

    fn names(chooser: &Chooser) -> Vec<String> {
        chooser
            .rows()
            .iter()
            .map(|row| match row {
                Row::App {
                    app,
                    is_default: true,
                } => format!("{}*", app.name),
                Row::App { app, .. } => app.name.clone(),
                Row::Others { open: true } => "v others".to_string(),
                Row::Others { open: false } => "> others".to_string(),
            })
            .collect()
    }

    #[test]
    fn the_default_is_first_and_highlighted() {
        let c = chooser(
            &[("evince", "Evince"), ("okular", "Okular")],
            &[("gimp", "GIMP")],
        );
        assert_eq!(names(&c), ["Evince*", "Okular", "> others"]);
        assert_eq!(c.selected().map(|a| a.id.as_str()), Some("evince"));
    }

    #[test]
    fn the_heading_folds_the_other_apps() {
        let mut c = chooser(&[("evince", "Evince")], &[("gimp", "GIMP")]);
        assert_eq!(c.activate(1), None);
        assert_eq!(names(&c), ["Evince*", "v others", "GIMP"]);
        assert_eq!(c.activate(2).map(|a| a.id), Some("gimp".to_string()));
        c.activate(1);
        assert_eq!(names(&c), ["Evince*", "> others"]);
    }

    #[test]
    fn with_nothing_suggested_everything_is_unfolded() {
        let c = chooser(&[], &[("gimp", "GIMP"), ("zed", "Zed")]);
        assert_eq!(names(&c), ["v others", "GIMP", "Zed"]);
        assert_eq!(c.selected().map(|a| a.id.as_str()), Some("gimp"));
    }

    #[test]
    fn a_query_searches_both_groups() {
        let mut c = chooser(
            &[("evince", "Evince"), ("okular", "Okular")],
            &[("gimp", "GIMP"), ("gedit", "gedit")],
        );
        c.set_query("ge");
        assert_eq!(names(&c), ["v others", "gedit"]);
        assert_eq!(c.selected().map(|a| a.id.as_str()), Some("gedit"));
        // The heading does not fold a search's results away.
        c.activate(0);
        assert_eq!(names(&c), ["v others", "gedit"]);
        c.set_query("nothing like it");
        assert!(c.rows().is_empty());
        assert_eq!(c.selected(), None);
    }

    #[test]
    fn the_highlight_stops_at_the_ends() {
        let mut c = chooser(&[("evince", "Evince"), ("okular", "Okular")], &[]);
        c.move_highlight(-1);
        assert_eq!(c.highlight, Some(0));
        c.move_highlight(5);
        assert_eq!(c.highlight, Some(1));
    }

    #[test]
    fn only_one_type_can_be_remembered() {
        let mut c = chooser(&[("evince", "Evince")], &[]);
        c.toggle_always();
        assert!(c.always);
        c.mime = None;
        c.always = false;
        c.toggle_always();
        assert!(!c.always, "mixed types have no single default to set");
        c.mime = Some(UNKNOWN_TYPE.to_string());
        c.toggle_always();
        assert!(!c.always, "every file is octet-stream underneath");
    }
}
