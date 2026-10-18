//! What the user has said about the sidebar, in `~/.config/otto/files.toml`.
//!
//! The file is entirely optional and the defaults stand without it: Recent,
//! Home, and whichever XDG user directories actually exist. What it adds is
//! the two things a defaults-only sidebar cannot express — folders that are
//! nobody's idea of standard but are where *this* person works, and built-in
//! rows they never use taking up the list.
//!
//! ```toml
//! [sidebar]
//! # Built-in rows to leave out, by name.
//! hide = ["music", "videos"]
//!
//! # Folders to add, after the built-in ones.
//! [[sidebar.places]]
//! path = "~/dev/otto"
//! label = "Otto"          # optional: the folder's own name otherwise
//! icon = "folder-code"    # optional: a generic folder otherwise
//! ```
//!
//! The browser edits the same file when someone drags a folder onto the
//! sidebar or removes a row from it — see [`with_places_added`] and its
//! siblings — through `toml_edit`, so whatever else the file holds, comments
//! included, is left as it was written.
//!
//! A file that cannot be read or cannot be parsed is a warning in the log and
//! nothing more. A sidebar is how you get anywhere in this window, and losing
//! it over a stray bracket would be a bad trade for a strictness nobody asked
//! for.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The built-in rows, by the name the config file calls them.
///
/// Stable identifiers rather than labels: the label is translated, and a
/// config written on an English desktop must not stop working when the
/// language changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Recent,
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
}

impl Builtin {
    /// Every built-in row, in sidebar order.
    pub const ALL: [Builtin; 8] = [
        Builtin::Recent,
        Builtin::Home,
        Builtin::Desktop,
        Builtin::Documents,
        Builtin::Downloads,
        Builtin::Music,
        Builtin::Pictures,
        Builtin::Videos,
    ];

    /// The row `id` names in `hide`, matched the way `hide` is read.
    pub fn from_id(id: &str) -> Option<Self> {
        let id = id.trim().to_lowercase();
        Self::ALL.into_iter().find(|builtin| builtin.id() == id)
    }

    /// The name this row answers to in `hide`.
    pub fn id(self) -> &'static str {
        match self {
            Builtin::Recent => "recent",
            Builtin::Home => "home",
            Builtin::Desktop => "desktop",
            Builtin::Documents => "documents",
            Builtin::Downloads => "downloads",
            Builtin::Music => "music",
            Builtin::Pictures => "pictures",
            Builtin::Videos => "videos",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct File {
    #[serde(default)]
    sidebar: Sidebar,
}

#[derive(Debug, Default, Deserialize)]
struct Sidebar {
    #[serde(default)]
    hide: Vec<String>,
    #[serde(default)]
    places: Vec<Entry>,
    #[serde(default)]
    order: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    path: String,
    label: Option<String>,
    icon: Option<String>,
}

/// One folder the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Custom {
    pub label: String,
    pub path: PathBuf,
    pub icon: String,
}

/// A sidebar row as `order` names it: a built-in row by its id, or one of
/// the user's folders by its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceKey {
    Builtin(Builtin),
    Path(PathBuf),
}

impl PlaceKey {
    /// Read one entry of `order`. A built-in row's id wins over a relative
    /// path that happens to spell it.
    fn parse(raw: &str, home: Option<&Path>) -> Option<Self> {
        Builtin::from_id(raw)
            .map(Self::Builtin)
            .or_else(|| expand_home(raw, home).map(Self::Path))
    }

    /// This key the way `order` writes it.
    fn written(&self, home: Option<&Path>) -> String {
        match self {
            Self::Builtin(builtin) => builtin.id().to_string(),
            Self::Path(path) => abbreviate(path, home),
        }
    }
}

/// The sidebar as configured, or an empty one where nothing was said.
#[derive(Debug, Default, Clone)]
pub struct SidebarConfig {
    hidden: Vec<String>,
    pub extra: Vec<Custom>,
    /// The order the user dragged the rows into, if they have. Empty means
    /// the standard order.
    pub order: Vec<PlaceKey>,
}

impl SidebarConfig {
    /// Read `~/.config/otto/files.toml`, if it is there.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // Absent is the ordinary case and says nothing worth logging;
            // unreadable is worth a line, because the person meant it to be
            // read.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "cannot read the sidebar config");
                return Self::default();
            }
        };
        match toml::from_str::<File>(&text) {
            Ok(file) => Self::from_file(file, home_dir().as_deref()),
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "ignoring the sidebar config");
                Self::default()
            }
        }
    }

    fn from_file(file: File, home: Option<&Path>) -> Self {
        Self {
            order: file
                .sidebar
                .order
                .iter()
                .filter_map(|raw| PlaceKey::parse(raw, home))
                .collect(),
            hidden: file
                .sidebar
                .hide
                .into_iter()
                .map(|name| name.trim().to_lowercase())
                .collect(),
            extra: file
                .sidebar
                .places
                .into_iter()
                .filter_map(|entry| {
                    let path = expand(&entry.path, home)?;
                    Some(Custom {
                        // A folder's own name is nearly always what you would
                        // have typed anyway, so the label is worth asking for
                        // only when it is not.
                        label: entry.label.unwrap_or_else(|| {
                            path.file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.to_string_lossy().into_owned())
                        }),
                        icon: entry.icon.unwrap_or_else(|| "folder".to_string()),
                        path,
                    })
                })
                .collect(),
        }
    }

    /// A config that hides these rows and says nothing else. For tests, and
    /// for anywhere the answer is known without reading a file.
    pub fn hiding(builtins: &[Builtin]) -> Self {
        Self {
            hidden: builtins.iter().map(|b| b.id().to_string()).collect(),
            extra: Vec::new(),
            order: Vec::new(),
        }
    }

    /// Whether a built-in row was asked to stay out of the list.
    pub fn hides(&self, builtin: Builtin) -> bool {
        self.hidden.iter().any(|name| name == builtin.id())
    }

    /// The same config with every built-in row shown: what the sidebar would
    /// hold if nothing had been hidden.
    pub fn hiding_nothing(&self) -> Self {
        Self {
            hidden: Vec::new(),
            extra: self.extra.clone(),
            order: self.order.clone(),
        }
    }
}

/// Write `order` into `text`'s `[sidebar]`, one row per line.
///
/// # Errors
///
/// As [`with_places_added`].
pub fn with_order(
    text: &str,
    order: &[PlaceKey],
    home: Option<&Path>,
) -> Result<String, toml_edit::TomlError> {
    let mut doc = text.parse::<toml_edit::DocumentMut>()?;
    let mut array = toml_edit::Array::new();
    for key in order {
        let mut value = toml_edit::Value::from(key.written(home));
        value.decor_mut().set_prefix("\n    ");
        array.push_formatted(value);
    }
    array.set_trailing_comma(true);
    array.set_trailing("\n");
    sidebar_table(&mut doc)["order"] = toml_edit::value(array);
    Ok(doc.to_string())
}

/// Add `paths` to the sidebar's own folders in `text`, before the entry for
/// `before`, or after the last one when `before` is `None` or not listed.
///
/// A folder already listed moves rather than appearing twice, which is what
/// makes dragging one of the user's folders to another gap a reorder. Paths
/// are written under `~` where they can be, the way a person would type them.
///
/// # Errors
///
/// When `text` is not TOML. The file is left alone then: rewriting it would
/// lose whatever the person was halfway through typing.
pub fn with_places_added(
    text: &str,
    paths: &[PathBuf],
    before: Option<&Path>,
    home: Option<&Path>,
) -> Result<String, toml_edit::TomlError> {
    let mut doc = text.parse::<toml_edit::DocumentMut>()?;
    let places = places_array(&mut doc);
    for path in paths {
        // Dropped on the gap beside itself: it is already there.
        if before == Some(path.as_path()) {
            continue;
        }
        if let Some(index) = entry_index(places, path, home) {
            places.remove(index);
        }
        let at = before
            .and_then(|before| entry_index(places, before, home))
            .unwrap_or(places.len());
        let mut entry = toml_edit::Table::new();
        entry["path"] = toml_edit::value(abbreviate(path, home));
        places.insert(at, entry);
    }
    Ok(doc.to_string())
}

/// Take `path` out of the sidebar's own folders in `text`. Every entry naming
/// it goes, so a folder listed twice by hand does not survive its removal.
///
/// # Errors
///
/// As [`with_places_added`].
pub fn with_place_removed(
    text: &str,
    path: &Path,
    home: Option<&Path>,
) -> Result<String, toml_edit::TomlError> {
    let mut doc = text.parse::<toml_edit::DocumentMut>()?;
    let places = places_array(&mut doc);
    while let Some(index) = entry_index(places, path, home) {
        places.remove(index);
    }
    // Its place in the order goes with it.
    if let Some(order) = sidebar_table(&mut doc)
        .get_mut("order")
        .and_then(|item| item.as_array_mut())
    {
        order.retain(|value| {
            value.as_str().and_then(|raw| PlaceKey::parse(raw, home))
                != Some(PlaceKey::Path(path.to_path_buf()))
        });
    }
    Ok(doc.to_string())
}

/// Hide `builtin` in `text`, or show it again.
///
/// # Errors
///
/// As [`with_places_added`].
pub fn with_builtin_hidden(
    text: &str,
    builtin: Builtin,
    hidden: bool,
) -> Result<String, toml_edit::TomlError> {
    let mut doc = text.parse::<toml_edit::DocumentMut>()?;
    let sidebar = sidebar_table(&mut doc);
    if !sidebar.contains_key("hide") {
        sidebar["hide"] = toml_edit::value(toml_edit::Array::new());
    }
    let Some(hide) = sidebar["hide"].as_array_mut() else {
        // Something that is not a list sits where the list goes. Better left
        // for the person to see than overwritten.
        return Ok(doc.to_string());
    };
    let names_it = |value: &toml_edit::Value| {
        value
            .as_str()
            .is_some_and(|name| Builtin::from_id(name) == Some(builtin))
    };
    if hidden {
        if !hide.iter().any(names_it) {
            hide.push(builtin.id());
        }
    } else {
        hide.retain(|value| !names_it(value));
    }
    Ok(doc.to_string())
}

/// `[sidebar]`, made if it is not there yet.
fn sidebar_table(doc: &mut toml_edit::DocumentMut) -> &mut toml_edit::Table {
    if !doc.contains_table("sidebar") {
        doc["sidebar"] = toml_edit::table();
    }
    doc["sidebar"]
        .as_table_mut()
        .expect("`sidebar` was just made a table")
}

/// `[[sidebar.places]]`, made if it is not there yet.
///
/// An inline array a person wrote by hand is turned into the array-of-tables
/// form, keeping its entries; anything else under the key is not a list of
/// places and is replaced.
fn places_array(doc: &mut toml_edit::DocumentMut) -> &mut toml_edit::ArrayOfTables {
    let item = sidebar_table(doc)
        .entry("places")
        .or_insert_with(|| toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
    if !item.is_array_of_tables() {
        let tables = std::mem::take(item)
            .into_array_of_tables()
            .unwrap_or_else(|_| toml_edit::ArrayOfTables::new());
        *item = toml_edit::Item::ArrayOfTables(tables);
    }
    item.as_array_of_tables_mut()
        .expect("`places` was just made an array of tables")
}

/// Where the entry for `path` is in `places`, comparing paths as the reader
/// expands them rather than as they were written.
fn entry_index(
    places: &toml_edit::ArrayOfTables,
    path: &Path,
    home: Option<&Path>,
) -> Option<usize> {
    places.iter().position(|entry| {
        entry
            .get("path")
            .and_then(|item| item.as_str())
            .and_then(|raw| expand_home(raw, home))
            .is_some_and(|written| written == path)
    })
}

/// `path` the way a person would write it: under `~` when it is in home.
fn abbreviate(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Rewrite `~/.config/otto/files.toml` with `edit`, a missing file reading
/// as an empty one.
///
/// # Errors
///
/// When there is no home folder, the file cannot be read or written, or
/// `edit` refuses the text.
pub(crate) fn edit_file(
    edit: impl FnOnce(&str) -> Result<String, toml_edit::TomlError>,
) -> std::io::Result<()> {
    let path = config_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home folder"))?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let next =
        edit(&text).map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written beside and renamed over, so the windows watching the file
    // never read a half-written one.
    let temp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    std::fs::write(&temp, next)?;
    std::fs::rename(&temp, &path)
}

/// `~/.config/otto/files.toml`. otto-settings' desk pane edits the same
/// file.
pub(crate) fn config_path() -> Option<PathBuf> {
    otto_kit::xdg::otto_config_file("files.toml")
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    otto_kit::xdg::home()
}

/// `~/dev` and `$HOME/dev` both mean the same folder to the person writing
/// them, and neither is a path any shell has already expanded by the time it
/// reaches a config file.
fn expand(raw: &str, home: Option<&Path>) -> Option<PathBuf> {
    let path = expand_home(raw, home)?;
    // A row leading nowhere is worse than its absence — the same rule the
    // built-in XDG folders follow.
    path.is_dir().then_some(path)
}

/// [`expand`] without asking whether the folder is there — for a path that
/// is still meaningful before it exists, such as the desk's folder.
pub(crate) fn expand_home(raw: &str, home: Option<&Path>) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let path = if let Some(rest) = raw
        .strip_prefix("~/")
        .or_else(|| raw.strip_prefix("$HOME/"))
    {
        home?.join(rest)
    } else if raw == "~" || raw == "$HOME" {
        home?.to_path_buf()
    } else {
        PathBuf::from(raw)
    };
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str, home: &Path) -> SidebarConfig {
        SidebarConfig::from_file(
            toml::from_str::<File>(text).expect("valid toml"),
            Some(home),
        )
    }

    /// The shape the documentation promises, read back.
    #[test]
    fn a_configured_sidebar_hides_and_adds() {
        let dir = std::env::temp_dir().join(format!("otto-files-cfg-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("dev/otto")).unwrap();

        let config = parse(
            r#"
            [sidebar]
            hide = ["music", "Videos"]

            [[sidebar.places]]
            path = "~/dev/otto"
            label = "Otto"
            icon = "folder-code"

            [[sidebar.places]]
            path = "~/dev"
            "#,
            &dir,
        );

        assert!(config.hides(Builtin::Music));
        // Names are matched case-insensitively: a config is prose, not code.
        assert!(config.hides(Builtin::Videos));
        assert!(!config.hides(Builtin::Home));

        assert_eq!(config.extra.len(), 2);
        assert_eq!(config.extra[0].label, "Otto");
        assert_eq!(config.extra[0].icon, "folder-code");
        assert_eq!(config.extra[0].path, dir.join("dev/otto"));
        // No label given, so the folder's own name; no icon, so a folder.
        assert_eq!(config.extra[1].label, "dev");
        assert_eq!(config.extra[1].icon, "folder");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A row that leads nowhere is dropped rather than drawn, the same as a
    /// missing XDG folder. A typo in a path should cost that row and no more.
    #[test]
    fn a_folder_that_is_not_there_is_not_listed() {
        let home = std::env::temp_dir();
        let config = parse(
            r#"
            [[sidebar.places]]
            path = "~/definitely-not-a-real-folder-9f3a"
            "#,
            &home,
        );
        assert!(config.extra.is_empty());
    }

    /// The paths written in `text`'s `[[sidebar.places]]`, in order.
    fn written(text: &str) -> Vec<String> {
        let file: File = toml::from_str(text).expect("valid toml");
        file.sidebar.places.into_iter().map(|e| e.path).collect()
    }

    /// A folder dropped on a gap lands in that gap, written the way a person
    /// would have typed it, and the rest of the file is left as it was.
    #[test]
    fn adding_a_folder_puts_it_where_it_was_dropped() {
        let home = Path::new("/home/me");
        let text = "# my sidebar\n[sidebar]\nhide = [\"music\"]\n\n\
                    [[sidebar.places]]\npath = \"~/a\"\nlabel = \"A\" # mine\n\n\
                    [[sidebar.places]]\npath = \"~/b\"\n\n\
                    [desk]\narrange = \"free\"\n";

        let between = with_places_added(
            text,
            &[PathBuf::from("/home/me/new")],
            Some(Path::new("/home/me/b")),
            Some(home),
        )
        .unwrap();
        assert_eq!(written(&between), ["~/a", "~/new", "~/b"]);
        assert!(between.contains("# my sidebar"));
        assert!(between.contains("label = \"A\" # mine"));
        assert!(between.contains("hide = [\"music\"]"));
        assert!(between.contains("[desk]\narrange = \"free\""));

        let appended =
            with_places_added(text, &[PathBuf::from("/srv/data")], None, Some(home)).unwrap();
        assert_eq!(written(&appended), ["~/a", "~/b", "/srv/data"]);
    }

    /// Dropping a folder that is already listed moves it: that is reordering.
    #[test]
    fn adding_a_listed_folder_moves_it() {
        let home = Path::new("/home/me");
        let text = "[[sidebar.places]]\npath = \"~/a\"\nlabel = \"A\"\n\n\
                    [[sidebar.places]]\npath = \"~/b\"\n\n\
                    [[sidebar.places]]\npath = \"~/c\"\n";

        let to_end =
            with_places_added(text, &[PathBuf::from("/home/me/a")], None, Some(home)).unwrap();
        assert_eq!(written(&to_end), ["~/b", "~/c", "~/a"]);

        let up = with_places_added(
            text,
            &[PathBuf::from("/home/me/c")],
            Some(Path::new("/home/me/a")),
            Some(home),
        )
        .unwrap();
        assert_eq!(written(&up), ["~/c", "~/a", "~/b"]);

        // Beside itself: nothing moves, and nothing is lost either.
        let stay = with_places_added(
            text,
            &[PathBuf::from("/home/me/b")],
            Some(Path::new("/home/me/b")),
            Some(home),
        )
        .unwrap();
        assert_eq!(written(&stay), ["~/a", "~/b", "~/c"]);
    }

    /// The first edit to a file that has none of this yet makes the section.
    #[test]
    fn adding_to_an_empty_file_makes_the_section() {
        let home = Path::new("/home/me");
        let text =
            with_places_added("", &[PathBuf::from("/home/me/dev")], None, Some(home)).unwrap();
        assert_eq!(written(&text), ["~/dev"]);

        let inline = "[sidebar]\nplaces = [{ path = \"~/a\" }]\n";
        let text =
            with_places_added(inline, &[PathBuf::from("/home/me/b")], None, Some(home)).unwrap();
        assert_eq!(written(&text), ["~/a", "~/b"]);
    }

    /// Removing matches on the expanded path, however it was written.
    #[test]
    fn removing_a_folder_takes_every_entry_for_it() {
        let home = Path::new("/home/me");
        let text = "[[sidebar.places]]\npath = \"~/a\"\n\n\
                    [[sidebar.places]]\npath = \"$HOME/b\"\n\n\
                    [[sidebar.places]]\npath = \"/home/me/a/\"\n";
        let text = with_place_removed(text, Path::new("/home/me/a"), Some(home)).unwrap();
        assert_eq!(written(&text), ["$HOME/b"]);
    }

    /// A built-in row hides once and comes back however its name was cased.
    #[test]
    fn hiding_and_showing_a_built_in_row() {
        let text = with_builtin_hidden("", Builtin::Music, true).unwrap();
        let text = with_builtin_hidden(&text, Builtin::Music, true).unwrap();
        let config = SidebarConfig::from_file(toml::from_str(&text).unwrap(), None);
        assert!(config.hides(Builtin::Music));
        assert_eq!(text.matches("music").count(), 1);

        let text = with_builtin_hidden(
            "[sidebar]\nhide = [\"Music\", \"videos\"]\n",
            Builtin::Music,
            false,
        )
        .unwrap();
        let config = SidebarConfig::from_file(toml::from_str(&text).unwrap(), None);
        assert!(!config.hides(Builtin::Music));
        assert!(config.hides(Builtin::Videos));
    }

    /// The order a sort writes reads back as the same rows, one per line,
    /// paths under `~`; removing a folder takes it out of the order too.
    #[test]
    fn the_order_is_written_and_read_back() {
        let home = Path::new("/home/me");
        let order = [
            PlaceKey::Path(PathBuf::from("/home/me/Pictures/Screenshots")),
            PlaceKey::Builtin(Builtin::Home),
            PlaceKey::Builtin(Builtin::Downloads),
        ];
        let text = with_order("# kept\n[desk]\narrange = \"free\"\n", &order, Some(home)).unwrap();
        assert!(text.contains("# kept"));
        assert!(text.contains("\n    \"~/Pictures/Screenshots\",\n"));
        let config = SidebarConfig::from_file(toml::from_str(&text).unwrap(), Some(home));
        assert_eq!(config.order, order);

        let text = with_place_removed(
            &text,
            Path::new("/home/me/Pictures/Screenshots"),
            Some(home),
        )
        .unwrap();
        let config = SidebarConfig::from_file(toml::from_str(&text).unwrap(), Some(home));
        assert_eq!(config.order, order[1..]);
    }

    /// Saying nothing is the ordinary case and has to mean "the defaults".
    #[test]
    fn an_empty_config_changes_nothing() {
        let config = parse("", &std::env::temp_dir());
        assert!(config.extra.is_empty());
        for builtin in [
            Builtin::Recent,
            Builtin::Home,
            Builtin::Desktop,
            Builtin::Documents,
            Builtin::Downloads,
            Builtin::Music,
            Builtin::Pictures,
            Builtin::Videos,
        ] {
            assert!(!config.hides(builtin), "{}", builtin.id());
        }
    }
}
