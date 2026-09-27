//! What the browser window opens on, read from its command line.
//!
//! ```sh
//! otto-files [FOLDER | FILE...]
//! otto-files --select PATH...
//! otto-files --search QUERY [--in DIR] [--select PATH...]
//! ```
//!
//! Split in two so both halves test without a window or a disk:
//! [`parse`] only reads the words, and [`resolve`] decides what they mean
//! against the filesystem it is handed.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::model::SearchScope;

/// The browser's arguments, as written.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Args {
    /// Paths given without a flag: a folder to open, or files to select.
    pub paths: Vec<PathBuf>,
    /// `--select`: paths to open the folder of, selected.
    pub select: Vec<PathBuf>,
    /// `--search`: a query to open the results of.
    pub search: Option<String>,
    /// `--in`: the folder the search is scoped to.
    pub search_in: Option<PathBuf>,
    /// Anything not understood, to warn about.
    pub unknown: Vec<String>,
}

/// Read the arguments after the program name.
///
/// `--select` takes every path after it up to the next option; `--search`
/// and `--in` take one value each, also spelled `--search=QUERY`.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Args {
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    let mut selecting = false;
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        let (flag, inline) = match text.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(OsString::from(value))),
            _ => (text.as_ref(), None),
        };
        match flag {
            "--search" | "--in" => {
                selecting = false;
                let Some(value) = inline.or_else(|| args.next()) else {
                    parsed.unknown.push(format!("{flag} needs a value"));
                    continue;
                };
                if flag == "--search" {
                    parsed.search = Some(value.to_string_lossy().into_owned());
                } else {
                    parsed.search_in = Some(PathBuf::from(value));
                }
            }
            "--select" => {
                selecting = true;
                if let Some(value) = inline {
                    parsed.select.push(PathBuf::from(value));
                }
            }
            other if other.starts_with("--") => {
                selecting = false;
                parsed.unknown.push(format!("unknown option {other}"));
            }
            _ if selecting => parsed.select.push(PathBuf::from(&arg)),
            _ => parsed.paths.push(PathBuf::from(&arg)),
        }
    }
    parsed
}

/// What the window opens on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// The folder shown, and the one Back and Escape return to from a search.
    pub folder: PathBuf,
    /// Paths to select once they are listed, absolute.
    pub select: Vec<PathBuf>,
    /// A search to run at once, with its scope.
    pub search: Option<(String, SearchScope)>,
    /// Arguments that could not be honoured, to say so on stderr.
    pub warnings: Vec<String>,
}

/// What a path is on disk, for [`resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Folder,
    File,
    Missing,
}

impl Probe {
    /// Ask the filesystem.
    pub fn disk(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => Self::Folder,
            Ok(_) => Self::File,
            // A broken symlink is still something to select.
            Err(_) if path.symlink_metadata().is_ok() => Self::File,
            Err(_) => Self::Missing,
        }
    }
}

/// Decide what `args` open, given the home and current folders and a way to
/// ask what a path is.
///
/// - A folder among the plain paths is opened; a file there counts as
///   `--select`.
/// - `--select` opens the first path's folder with it and every other given
///   path in that folder selected. It wins over a plain folder.
/// - `--search` opens the results: scoped to `--in` (or a plain folder) as
///   *This folder*, else *Everywhere* from home. A `--select` then waits for
///   its path to arrive in the results.
/// - A path that is not there is left out with a warning; with nothing left,
///   the window opens at home.
pub fn resolve(args: &Args, home: &Path, cwd: &Path, probe: impl Fn(&Path) -> Probe) -> Start {
    let absolute = |path: &Path| absolute(path, home, cwd);
    let mut warnings = args.unknown.clone();
    let mut folder = None;
    let mut select = Vec::new();

    let wanted = args
        .paths
        .iter()
        .map(|p| (p, false))
        .chain(args.select.iter().map(|p| (p, true)));
    for (typed, explicit) in wanted {
        let path = absolute(typed);
        match probe(&path) {
            Probe::Folder if !explicit => {
                folder.get_or_insert(path);
            }
            Probe::Folder | Probe::File => select.push(path),
            Probe::Missing => warnings.push(format!("{} does not exist", typed.display())),
        }
    }

    if let Some(query) = &args.search {
        let scoped = match &args.search_in {
            Some(typed) => {
                let dir = absolute(typed);
                if probe(&dir) == Probe::Folder {
                    Some(dir)
                } else {
                    warnings.push(format!(
                        "{} is not a folder; searching everywhere",
                        typed.display()
                    ));
                    None
                }
            }
            None => folder,
        };
        let (folder, scope) = match scoped {
            Some(dir) => (dir, SearchScope::Folder),
            None => (home.to_path_buf(), SearchScope::Everywhere),
        };
        return Start {
            folder,
            select,
            search: Some((query.clone(), scope)),
            warnings,
        };
    }

    // Selecting: the first path's folder, and whatever else is in it.
    if let Some(parent) = select
        .first()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
    {
        let (here, elsewhere): (Vec<_>, Vec<_>) = select
            .into_iter()
            .partition(|p| p.parent() == Some(parent.as_path()));
        for path in elsewhere {
            warnings.push(format!(
                "{} is not in {}; not selected",
                path.display(),
                parent.display()
            ));
        }
        return Start {
            folder: parent,
            select: here,
            search: None,
            warnings,
        };
    }

    Start {
        folder: folder.unwrap_or_else(|| home.to_path_buf()),
        select: Vec::new(),
        search: None,
        warnings,
    }
}

/// `path` made absolute and tidied without touching the disk: `~` is home,
/// relative is under `cwd`, and `.` and `..` are folded away.
///
/// Symlinks are kept as written rather than resolved, so the folder opened
/// is the one named, and a selected path matches the listing's own spelling.
fn absolute(path: &Path, home: &Path, cwd: &Path) -> PathBuf {
    let joined = if let Ok(rest) = path.strip_prefix("~") {
        home.join(rest)
    } else {
        cwd.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        parse(list.iter().map(OsString::from))
    }

    /// A pretend disk: folders end in `/` in `tree`, everything else is a file.
    fn start(list: &[&str], tree: &[&str]) -> Start {
        let probe = |path: &Path| {
            let text = path.to_string_lossy();
            if tree.iter().any(|t| t.trim_end_matches('/') == text) {
                if tree.contains(&format!("{text}/").as_str()) {
                    Probe::Folder
                } else {
                    Probe::File
                }
            } else {
                Probe::Missing
            }
        };
        resolve(
            &args(list),
            Path::new("/home/u"),
            Path::new("/home/u/work"),
            probe,
        )
    }

    const TREE: &[&str] = &[
        "/home/u/",
        "/home/u/work/",
        "/home/u/Documents/",
        "/home/u/Documents/a.pdf",
        "/home/u/Documents/b.pdf",
        "/home/u/Pictures/",
        "/home/u/Pictures/c.png",
    ];

    #[test]
    fn parse_reads_every_flag_in_both_spellings() {
        let parsed = args(&[
            "--search",
            "invoice kind:pdf",
            "--in=~/Documents",
            "--select",
            "a",
            "b",
            "--bogus",
            "c",
        ]);
        assert_eq!(parsed.search.as_deref(), Some("invoice kind:pdf"));
        assert_eq!(parsed.search_in, Some(PathBuf::from("~/Documents")));
        assert_eq!(parsed.select, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(parsed.paths, [PathBuf::from("c")]);
        assert_eq!(parsed.unknown, ["unknown option --bogus"]);
        assert_eq!(args(&["--search"]).unknown, ["--search needs a value"]);
    }

    #[test]
    fn nothing_opens_home_and_a_folder_opens_itself() {
        let home = start(&[], TREE);
        assert_eq!(home.folder, PathBuf::from("/home/u"));
        assert!(home.select.is_empty() && home.search.is_none());

        let docs = start(&["~/Documents"], TREE);
        assert_eq!(docs.folder, PathBuf::from("/home/u/Documents"));
    }

    #[test]
    fn a_file_opens_its_folder_with_it_selected() {
        for list in [
            &["../Documents/a.pdf"][..],
            &["--select", "~/Documents/a.pdf"],
        ] {
            let s = start(list, TREE);
            assert_eq!(s.folder, PathBuf::from("/home/u/Documents"), "{list:?}");
            assert_eq!(s.select, [PathBuf::from("/home/u/Documents/a.pdf")]);
            assert!(s.warnings.is_empty());
        }
    }

    #[test]
    fn several_selections_keep_to_the_first_ones_folder() {
        let s = start(
            &[
                "--select",
                "/home/u/Documents/a.pdf",
                "/home/u/Pictures/c.png",
                "/home/u/Documents/b.pdf",
            ],
            TREE,
        );
        assert_eq!(s.folder, PathBuf::from("/home/u/Documents"));
        assert_eq!(
            s.select,
            [
                PathBuf::from("/home/u/Documents/a.pdf"),
                PathBuf::from("/home/u/Documents/b.pdf")
            ]
        );
        assert_eq!(
            s.warnings.len(),
            1,
            "the picture is not selected, and says so"
        );
    }

    #[test]
    fn a_missing_path_opens_home_with_a_warning() {
        for list in [&["/nowhere"][..], &["--select", "/nowhere/x"]] {
            let s = start(list, TREE);
            assert_eq!(s.folder, PathBuf::from("/home/u"), "{list:?}");
            assert!(s.select.is_empty());
            assert_eq!(s.warnings.len(), 1);
        }
    }

    #[test]
    fn a_search_runs_everywhere_or_in_a_folder() {
        let s = start(&["--search", "invoice"], TREE);
        assert_eq!(s.folder, PathBuf::from("/home/u"));
        assert_eq!(
            s.search,
            Some(("invoice".to_owned(), SearchScope::Everywhere))
        );

        let s = start(&["--search", "invoice", "--in", "../Documents"], TREE);
        assert_eq!(s.folder, PathBuf::from("/home/u/Documents"));
        assert_eq!(s.search, Some(("invoice".to_owned(), SearchScope::Folder)));

        let s = start(&["--search", "invoice", "--in", "/nowhere"], TREE);
        assert_eq!(s.folder, PathBuf::from("/home/u"));
        assert_eq!(
            s.search.map(|(_, scope)| scope),
            Some(SearchScope::Everywhere)
        );
        assert_eq!(s.warnings.len(), 1);
    }

    #[test]
    fn a_search_keeps_its_selection_for_the_results() {
        let s = start(
            &["--search", "pdf", "--select", "/home/u/Pictures/c.png"],
            TREE,
        );
        assert_eq!(
            s.folder,
            PathBuf::from("/home/u"),
            "the search decides the folder"
        );
        assert_eq!(s.select, [PathBuf::from("/home/u/Pictures/c.png")]);
    }
}
