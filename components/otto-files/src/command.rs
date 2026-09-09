//! What the file browser can be asked to do, as data.
//!
//! See `specs/file-command-palette.md`. The palette does not know what any
//! command *is*: it asks providers what is available given the situation the
//! window is in, ranks the answers against what is being typed, and hands back
//! a [`Request`] — an id and, where the command asked for one, a line of text.
//!
//! Everything crossing that seam is plain data. No closure over the browser,
//! no borrow, nothing that could not be written down and sent somewhere else.
//! That is deliberate and load-bearing: the extension system this is meant to
//! grow into will put a provider in another process, and the difference
//! between reusing this seam and rebuilding it is whether anything in it is a
//! Rust type in disguise.
//!
//! The built-in commands are one provider, [`Builtin`]. It has no privileges
//! the seam does not give everybody — it describes commands and nothing more.
//! Carrying one out is the host's job, because the host is the only thing
//! holding the window.

use std::path::PathBuf;

use otto_kit::matching;

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

/// Which part of the window a command belongs to. Fixed, and ordered the way
/// the resting list reads: where you are, then what you do to files, then the
/// clipboard, then how it all looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Go,
    File,
    Edit,
    View,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::Go, Group::File, Group::Edit, Group::View];

    /// The heading the resting list puts this group under, and the badge a
    /// ranked row carries.
    pub fn label(self) -> String {
        match self {
            Group::Go => otto_kit::t_owned!("files-command-group-go"),
            Group::File => otto_kit::t_owned!("files-command-group-file"),
            Group::Edit => otto_kit::t_owned!("files-command-group-edit"),
            Group::View => otto_kit::t_owned!("files-command-group-view"),
        }
    }
}

/// One of a fixed set of answers to an argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// What the command receives.
    pub value: String,
    /// What the user reads.
    pub title: String,
    /// The dimmer second line — a place's path, a sort key's direction.
    pub subtitle: Option<String>,
}

impl Choice {
    pub fn new(value: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            title: title.into(),
            subtitle: None,
        }
    }

    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

/// What kind of answer an argument wants, which is what decides where its
/// completions come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgKind {
    /// A filesystem path, completed against the disk by the host — the palette
    /// itself never does I/O.
    Path { dirs_only: bool },
    /// Free text, with no completions.
    Text,
    /// One of a fixed set, completed by the palette itself.
    Choice(Vec<Choice>),
}

/// The something-more a command needs said about it before it can run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgSpec {
    /// The non-editable prefix the field wears while the argument is being
    /// typed, without its colon: "Go to path".
    pub prompt: String,
    /// The one-word name of what is being asked for, shown dimmed after the
    /// command's title in the list: `Go to Path  ›  path`.
    pub label: String,
    /// Dim text standing in for an empty argument.
    pub placeholder: Option<String>,
    /// What the field starts out holding — the current name, for a rename.
    pub initial: Option<String>,
    /// Whether a half-typed argument can be applied as you type.
    ///
    /// Only for a command whose effect is *shown* rather than done — a
    /// selection, a filter, a highlight — and which the window can therefore
    /// take back when the palette is abandoned. A command that touches files
    /// must never set this: there is no half-typed rename.
    pub preview: bool,
    pub kind: ArgKind,
}

impl ArgSpec {
    pub fn new(prompt: impl Into<String>, label: impl Into<String>, kind: ArgKind) -> Self {
        Self {
            prompt: prompt.into(),
            label: label.into(),
            placeholder: None,
            initial: None,
            preview: false,
            kind,
        }
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn with_initial(mut self, initial: impl Into<String>) -> Self {
        self.initial = Some(initial.into());
        self
    }

    /// Mark the argument as one that can be applied while it is being typed.
    /// See [`Self::preview`] for what may claim this.
    pub fn previewed(mut self) -> Self {
        self.preview = true;
        self
    }

    /// The choices this argument offers, if it is a choice at all.
    pub fn choices(&self) -> Option<&[Choice]> {
        match &self.kind {
            ArgKind::Choice(choices) => Some(choices),
            _ => None,
        }
    }
}

/// One thing the window can be asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// Stable across releases and across locales — this is what a [`Request`]
    /// carries, so it is the one part of a command nothing may translate.
    /// Namespaced `provider:id` for everything but the built-ins.
    pub id: String,
    /// The localised line someone reads and types against.
    pub title: String,
    pub group: Group,
    /// Extra words that match but are never shown — the other name for the
    /// same thing, the term another file manager uses.
    pub keywords: Vec<String>,
    /// The chord this is also bound to, written the way the menus write it.
    /// Shown right-aligned; purely informational.
    pub shortcut: Option<String>,
    /// What the command needs said about it, if anything.
    pub arg: Option<ArgSpec>,
}

impl Command {
    pub fn new(id: impl Into<String>, title: impl Into<String>, group: Group) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            group,
            keywords: Vec::new(),
            shortcut: None,
            arg: None,
        }
    }

    pub fn with_keywords<S: Into<String>>(mut self, keywords: impl IntoIterator<Item = S>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn with_arg(mut self, arg: ArgSpec) -> Self {
        self.arg = Some(arg);
        self
    }

    /// Which provider this came from. The built-ins have no namespace, and the
    /// host answers for those itself.
    pub fn namespace(&self) -> Option<&str> {
        self.id.split_once(':').map(|(namespace, _)| namespace)
    }
}

/// A command, said in full: what to run and what to run it on.
///
/// The whole of what crosses back over the seam. An argument is text because
/// text is what was typed and text is what survives a trip out of the process;
/// making sense of it is the runner's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub arg: Option<String>,
}

// ---------------------------------------------------------------------------
// The situation
// ---------------------------------------------------------------------------

/// A place the window can be sent, as a provider sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceRef {
    pub label: String,
    pub path: PathBuf,
}

/// Everything a provider is allowed to know about the window when it is asked
/// what it can offer.
///
/// Deliberately a description rather than a handle. A provider that does not
/// live in this process must be able to answer the same question from the same
/// facts, so there is nothing here it could not be sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Situation {
    /// The directory the active pane is showing.
    pub path: PathBuf,
    /// What is selected, deepest pane only.
    pub selection: Vec<PathBuf>,
    /// The name under the cursor, selected or not.
    pub cursor_name: Option<String>,
    /// Whether the cursor is on a directory.
    pub cursor_is_dir: bool,
    /// This window is the Trash — a shell where most commands mean nothing.
    pub trash: bool,
    /// This window is showing Recent: a listing with no directory behind it,
    /// so nothing that needs a location applies.
    pub recent: bool,
    /// Something has been cut or copied and could be pasted.
    pub can_paste: bool,
    pub can_undo: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub can_go_up: bool,
    /// The listing has at least one entry.
    pub has_entries: bool,
    pub show_hidden: bool,
    /// The current view, as one of the ids [`view_choices`] offers.
    pub view: String,
    /// The current sort key, as one of the ids [`sort_choices`] offers.
    pub sort: String,
    /// The sidebar's places, in sidebar order.
    pub places: Vec<PlaceRef>,
}

impl Situation {
    /// How many files a command would act on: the selection, or the cursor
    /// entry when nothing is selected.
    pub fn target_count(&self) -> usize {
        if self.selection.is_empty() {
            usize::from(self.cursor_name.is_some())
        } else {
            self.selection.len()
        }
    }

    fn has_target(&self) -> bool {
        self.target_count() > 0
    }
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// Somewhere commands come from.
///
/// `Send` because the browser's state is shared with the threads that read
/// directories and decode previews; a provider rides along in it.
pub trait CommandProvider: Send {
    /// The prefix on this provider's command ids, and the name it is known by.
    /// Empty for the built-ins, which own the unprefixed namespace.
    fn namespace(&self) -> &'static str;

    /// What this provider can offer, here and now.
    ///
    /// Called on the keystroke that opens the palette, so it must not touch
    /// the disk or the network: a provider offers what it already knows.
    fn commands(&self, situation: &Situation) -> Vec<Command>;

    /// Carry out one of this provider's commands.
    ///
    /// The host never calls this for its own namespace — it is the only thing
    /// holding the window, so it routes its own commands internally. A
    /// provider that lives elsewhere implements this; the default refuses,
    /// which is the right answer for one that only describes.
    fn run(&mut self, request: &Request) -> Result<(), String> {
        Err(format!("{} cannot be run here", request.id))
    }
}

/// Every provider, asked together.
#[derive(Default)]
pub struct Registry {
    providers: Vec<Box<dyn CommandProvider>>,
}

impl Registry {
    /// The registry the browser runs with: the built-ins, and nothing else
    /// yet.
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        registry.add(Box::new(Builtin));
        registry
    }

    pub fn add(&mut self, provider: Box<dyn CommandProvider>) {
        self.providers.push(provider);
    }

    /// Everything on offer, in provider order — the built-ins first, so a
    /// later provider can never push a window command down the resting list.
    pub fn commands(&self, situation: &Situation) -> Vec<Command> {
        self.providers
            .iter()
            .flat_map(|provider| provider.commands(situation))
            .collect()
    }

    /// Hand a request to the provider whose namespace it names. `None` when
    /// the request belongs to the host's own namespace, which the host runs
    /// itself.
    pub fn run(&mut self, request: &Request) -> Option<Result<(), String>> {
        let namespace = request.namespace()?;
        let provider = self
            .providers
            .iter_mut()
            .find(|provider| provider.namespace() == namespace)?;
        Some(provider.run(request))
    }
}

impl Request {
    pub fn new(id: impl Into<String>, arg: Option<String>) -> Self {
        Self { id: id.into(), arg }
    }

    /// The provider this is addressed to, or `None` for the host's own.
    pub fn namespace(&self) -> Option<&str> {
        self.id
            .split_once(':')
            .map(|(namespace, _)| namespace)
            .filter(|namespace| !namespace.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

/// A matched command, with the score that ordered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub index: usize,
    pub score: i32,
}

/// Rank `commands` against `query`, best first.
///
/// An empty query keeps every command in the order the providers gave it,
/// which is the order the resting list groups.
pub fn rank(commands: &[Command], query: &str) -> Vec<Match> {
    let query = query.trim();
    if query.is_empty() {
        return commands
            .iter()
            .enumerate()
            .map(|(index, _)| Match { index, score: 0 })
            .collect();
    }

    let mut matches: Vec<Match> = commands
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            // The title is what someone is aiming at. A keyword or a group
            // name is a way of still finding the command when they aimed at
            // something adjacent, so it scores lower and can never outrank a
            // title hit.
            let title = matching::score(&command.title, query);
            let group = command.group.label();
            let secondary = command
                .keywords
                .iter()
                .map(String::as_str)
                .chain(std::iter::once(group.as_str()))
                .filter_map(|text| matching::score(text, query))
                .max()
                .map(|score| score / 2 - 20);

            let best = match (title, secondary) {
                (Some(a), Some(b)) => a.max(b),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => return None,
            };
            Some(Match { index, score: best })
        })
        .collect();

    // Ties keep provider order — a stable sort, so a fully tied query looks
    // like the resting list.
    matches.sort_by_key(|m| std::cmp::Reverse(m.score));
    matches
}

// ---------------------------------------------------------------------------
// The built-in commands
// ---------------------------------------------------------------------------

/// The ids the host answers for. Public so the host's routing and these
/// descriptions cannot drift apart.
pub mod id {
    pub const GO_BACK: &str = "go_back";
    pub const GO_FORWARD: &str = "go_forward";
    pub const GO_UP: &str = "go_up";
    pub const GO_HOME: &str = "go_home";
    pub const GO_TO_PATH: &str = "go_to_path";
    pub const GO_TO_PLACE: &str = "go_to_place";
    pub const OPEN: &str = "open";
    pub const GET_INFO: &str = "get_info";
    pub const RENAME: &str = "rename";
    pub const NEW_FOLDER: &str = "new_folder";
    pub const TRASH: &str = "trash";
    pub const PUT_BACK: &str = "put_back";
    pub const DELETE_FOREVER: &str = "delete_forever";
    pub const EMPTY_TRASH: &str = "empty_trash";
    pub const CUT: &str = "cut";
    pub const COPY: &str = "copy";
    pub const PASTE: &str = "paste";
    pub const SELECT_ALL: &str = "select_all";
    pub const SELECT_MATCHING: &str = "select_matching";
    pub const MOVE_TO: &str = "move_to";
    pub const NEW_FOLDER_WITH_SELECTION: &str = "new_folder_with_selection";
    pub const UNDO: &str = "undo";
    /// The three views, by name. Their ids double as the values
    /// [`CHANGE_VIEW`] takes, so there is one spelling of "grid".
    pub const VIEW_LIST: &str = "list";
    pub const VIEW_GRID: &str = "grid";
    pub const VIEW_COLUMNS: &str = "columns";
    pub const CHANGE_VIEW: &str = "change_view";
    pub const SORT_BY: &str = "sort_by";
    pub const TOGGLE_HIDDEN: &str = "toggle_hidden";
    pub const QUICK_LOOK: &str = "quick_look";
    pub const SEARCH: &str = "search";
    pub const RECENT: &str = "recent";
}

/// The views [`id::CHANGE_VIEW`] can be given.
pub fn view_choices() -> Vec<Choice> {
    vec![
        Choice::new("list", otto_kit::t_owned!("files-view-list")),
        Choice::new("grid", otto_kit::t_owned!("files-view-grid")),
        Choice::new("columns", otto_kit::t_owned!("files-view-columns")),
    ]
}

/// The keys [`id::SORT_BY`] can be given.
pub fn sort_choices() -> Vec<Choice> {
    vec![
        Choice::new("name", otto_kit::t_owned!("files-column-name")),
        Choice::new("size", otto_kit::t_owned!("files-column-size")),
        Choice::new("kind", otto_kit::t_owned!("files-column-kind")),
        Choice::new("modified", otto_kit::t_owned!("files-column-date-modified")),
    ]
}

/// The window's own commands.
pub struct Builtin;

impl CommandProvider for Builtin {
    fn namespace(&self) -> &'static str {
        ""
    }

    fn commands(&self, s: &Situation) -> Vec<Command> {
        let mut out = Vec::new();

        // --- Go -------------------------------------------------------------
        if s.can_go_back {
            out.push(
                Command::new(
                    id::GO_BACK,
                    otto_kit::t_owned!("files-command-back"),
                    Group::Go,
                )
                .with_keywords(["previous", "history"])
                .with_shortcut("Alt+←"),
            );
        }
        if s.can_go_forward {
            out.push(
                Command::new(
                    id::GO_FORWARD,
                    otto_kit::t_owned!("files-command-forward"),
                    Group::Go,
                )
                .with_keywords(["next", "history"])
                .with_shortcut("Alt+→"),
            );
        }
        if s.can_go_up {
            out.push(
                Command::new(id::GO_UP, otto_kit::t_owned!("files-command-up"), Group::Go)
                    .with_keywords(["parent", "enclosing folder"])
                    .with_shortcut("Alt+↑"),
            );
        }
        if !s.trash {
            out.push(
                Command::new(id::GO_HOME, otto_kit::t_owned!("files-home"), Group::Go)
                    .with_keywords(["home folder"])
                    .with_shortcut("Alt+Home"),
            );
            out.push(
                Command::new(
                    id::GO_TO_PATH,
                    otto_kit::t_owned!("files-command-go-to-path"),
                    Group::Go,
                )
                .with_keywords(["location", "directory", "cd"])
                .with_shortcut("Ctrl+L")
                .with_arg(
                    ArgSpec::new(
                        otto_kit::t_owned!("files-command-go-to-path-prompt"),
                        otto_kit::t_owned!("files-command-arg-path"),
                        ArgKind::Path { dirs_only: false },
                    )
                    .with_placeholder("~/Documents"),
                ),
            );
            if !s.places.is_empty() {
                out.push(
                    Command::new(
                        id::GO_TO_PLACE,
                        otto_kit::t_owned!("files-command-go-to-place"),
                        Group::Go,
                    )
                    .with_keywords(["sidebar", "shortcut", "bookmark"])
                    .with_arg(ArgSpec::new(
                        otto_kit::t_owned!("files-command-go-to-place-prompt"),
                        otto_kit::t_owned!("files-command-arg-place"),
                        ArgKind::Choice(
                            s.places
                                .iter()
                                .map(|place| {
                                    Choice::new(
                                        place.path.to_string_lossy().into_owned(),
                                        place.label.clone(),
                                    )
                                    .with_subtitle(crate::model::abbreviate_home(&place.path))
                                })
                                .collect(),
                        ),
                    )),
                );
            }
            out.push(
                Command::new(
                    id::SEARCH,
                    otto_kit::t_owned!("files-command-search"),
                    Group::Go,
                )
                .with_keywords(["find", "filter", "look for"])
                .with_shortcut("Ctrl+F")
                .with_arg(ArgSpec::new(
                    otto_kit::t_owned!("files-command-search-prompt"),
                    otto_kit::t_owned!("files-command-arg-query"),
                    ArgKind::Text,
                )),
            );
            if !s.recent {
                out.push(
                    Command::new(id::RECENT, otto_kit::t_owned!("files-recent"), Group::Go)
                        .with_keywords(["latest", "newest", "lately"]),
                );
            }
            if s.cursor_is_dir || s.cursor_name.is_some() {
                out.push(
                    Command::new(id::OPEN, otto_kit::t_owned!("common-open"), Group::Go)
                        .with_keywords(["launch", "enter"])
                        .with_shortcut("Ctrl+O"),
                );
            }
        }

        // --- File -----------------------------------------------------------
        if s.has_target() {
            out.push(
                Command::new(
                    id::GET_INFO,
                    otto_kit::t_owned!("files-get-info"),
                    Group::File,
                )
                .with_keywords(["properties", "permissions", "size"])
                .with_shortcut("Ctrl+I"),
            );
        }
        if !s.trash {
            if s.target_count() == 1 {
                out.push(
                    Command::new(id::RENAME, otto_kit::t_owned!("common-rename"), Group::File)
                        .with_keywords(["name"])
                        .with_shortcut("F2")
                        .with_arg(
                            ArgSpec::new(
                                otto_kit::t_owned!("files-command-rename-prompt"),
                                otto_kit::t_owned!("files-command-arg-name"),
                                ArgKind::Text,
                            )
                            .with_initial(s.cursor_name.clone().unwrap_or_default()),
                        ),
                );
            }
            out.push(
                Command::new(
                    id::NEW_FOLDER,
                    otto_kit::t_owned!("files-new-folder"),
                    Group::File,
                )
                .with_keywords(["create", "directory", "mkdir"])
                .with_arg(
                    ArgSpec::new(
                        otto_kit::t_owned!("files-command-new-folder-prompt"),
                        otto_kit::t_owned!("files-command-arg-name"),
                        ArgKind::Text,
                    )
                    .with_placeholder("untitled folder"),
                ),
            );
            if s.has_target() {
                let title = if s.target_count() == 1 {
                    otto_kit::t_owned!("files-move-to-trash")
                } else {
                    otto_kit::t_owned!("files-move-count-to-trash", count = s.target_count() as f64)
                };
                let folder_title = if s.target_count() == 1 {
                    otto_kit::t_owned!("files-new-folder-with-selection")
                } else {
                    otto_kit::t_owned!(
                        "files-new-folder-with-count",
                        count = s.target_count() as f64
                    )
                };
                out.push(
                    Command::new(id::NEW_FOLDER_WITH_SELECTION, folder_title, Group::File)
                        .with_keywords(["group", "gather", "collect", "into folder"])
                        .with_arg(
                            ArgSpec::new(
                                otto_kit::t_owned!("files-command-new-folder-prompt"),
                                otto_kit::t_owned!("files-command-arg-name"),
                                ArgKind::Text,
                            )
                            .with_placeholder("untitled folder"),
                        ),
                );
                out.push(
                    Command::new(
                        id::MOVE_TO,
                        otto_kit::t_owned!("files-command-move-to"),
                        Group::File,
                    )
                    .with_keywords(["move", "put", "file away", "relocate"])
                    .with_arg(
                        ArgSpec::new(
                            otto_kit::t_owned!("files-command-move-to-prompt"),
                            otto_kit::t_owned!("files-command-arg-path"),
                            // Only a folder can be moved into, so only folders
                            // are offered.
                            ArgKind::Path { dirs_only: true },
                        )
                        .with_placeholder("~/Documents"),
                    ),
                );
                out.push(
                    Command::new(id::TRASH, title, Group::File)
                        .with_keywords(["delete", "remove", "bin"])
                        .with_shortcut("Delete"),
                );
            }
        }
        if s.trash {
            if s.has_target() {
                out.push(
                    Command::new(
                        id::PUT_BACK,
                        otto_kit::t_owned!("files-put-back"),
                        Group::File,
                    )
                    .with_keywords(["restore", "undelete"]),
                );
                let title = if s.target_count() == 1 {
                    otto_kit::t_owned!("files-delete-immediately")
                } else {
                    otto_kit::t_owned!(
                        "files-delete-count-immediately",
                        count = s.target_count() as f64
                    )
                };
                out.push(
                    Command::new(id::DELETE_FOREVER, title, Group::File).with_keywords([
                        "destroy",
                        "erase",
                        "permanently",
                    ]),
                );
            }
            if s.has_entries {
                out.push(
                    Command::new(
                        id::EMPTY_TRASH,
                        otto_kit::t_owned!("files-empty-trash"),
                        Group::File,
                    )
                    .with_keywords(["destroy", "erase all"]),
                );
            }
        }

        // --- Edit -----------------------------------------------------------
        if !s.trash {
            if s.has_target() {
                out.push(
                    Command::new(id::CUT, otto_kit::t_owned!("common-cut"), Group::Edit)
                        .with_keywords(["move"])
                        .with_shortcut("Ctrl+X"),
                );
                out.push(
                    Command::new(id::COPY, otto_kit::t_owned!("common-copy"), Group::Edit)
                        .with_keywords(["duplicate", "clipboard"])
                        .with_shortcut("Ctrl+C"),
                );
            }
            if s.can_paste {
                out.push(
                    Command::new(id::PASTE, otto_kit::t_owned!("common-paste"), Group::Edit)
                        .with_keywords(["clipboard"])
                        .with_shortcut("Ctrl+V"),
                );
            }
            if s.can_undo {
                out.push(
                    Command::new(
                        id::UNDO,
                        otto_kit::t_owned!("files-command-undo"),
                        Group::Edit,
                    )
                    .with_keywords(["take back", "revert"])
                    .with_shortcut("Ctrl+Z"),
                );
            }
        }
        if s.has_entries {
            out.push(
                Command::new(
                    id::SELECT_MATCHING,
                    otto_kit::t_owned!("files-command-select-matching"),
                    Group::Edit,
                )
                .with_keywords(["glob", "wildcard", "pattern", "extension"])
                .with_arg(
                    ArgSpec::new(
                        otto_kit::t_owned!("files-command-select-matching-prompt"),
                        otto_kit::t_owned!("files-command-arg-pattern"),
                        ArgKind::Text,
                    )
                    .with_placeholder("*.png")
                    // The selection is shown, not done: every keystroke can
                    // narrow it in place, and abandoning the palette puts back
                    // whatever was selected before.
                    .previewed(),
                ),
            );
            out.push(
                Command::new(
                    id::SELECT_ALL,
                    otto_kit::t_owned!("files-command-select-all"),
                    Group::Edit,
                )
                .with_shortcut("Ctrl+A"),
            );
        }

        // --- View -----------------------------------------------------------
        if !s.trash {
            // The three views by name as well as behind Change View: typing
            // "grid" should land on the grid, not on a question about which
            // view you meant. The one that is already on is not offered —
            // switching to where you are is not a command.
            for (view, title, shortcut) in [
                (
                    id::VIEW_LIST,
                    otto_kit::t_owned!("files-view-list"),
                    "Ctrl+1",
                ),
                (
                    id::VIEW_GRID,
                    otto_kit::t_owned!("files-view-grid"),
                    "Ctrl+2",
                ),
                (
                    id::VIEW_COLUMNS,
                    otto_kit::t_owned!("files-view-columns"),
                    "Ctrl+3",
                ),
            ] {
                if s.view == view {
                    continue;
                }
                out.push(
                    Command::new(view, title, Group::View)
                        .with_keywords(["view", "layout"])
                        .with_shortcut(shortcut),
                );
            }
            out.push(
                Command::new(
                    id::CHANGE_VIEW,
                    otto_kit::t_owned!("files-command-change-view"),
                    Group::View,
                )
                .with_keywords(["list", "grid", "icons", "columns", "layout"])
                .with_arg(
                    ArgSpec::new(
                        otto_kit::t_owned!("files-command-change-view-prompt"),
                        otto_kit::t_owned!("files-command-arg-view"),
                        ArgKind::Choice(view_choices()),
                    )
                    // The view that is already on, so the list opens on it.
                    .with_initial(s.view.clone()),
                ),
            );
        }
        out.push(
            Command::new(
                id::SORT_BY,
                otto_kit::t_owned!("files-command-sort-by"),
                Group::View,
            )
            .with_keywords(["order", "arrange"])
            .with_arg(
                ArgSpec::new(
                    otto_kit::t_owned!("files-command-sort-by-prompt"),
                    otto_kit::t_owned!("files-command-arg-sort"),
                    ArgKind::Choice(sort_choices()),
                )
                .with_initial(s.sort.clone()),
            ),
        );
        let hidden = if s.show_hidden {
            otto_kit::t_owned!("files-command-hide-hidden")
        } else {
            otto_kit::t_owned!("files-command-show-hidden")
        };
        out.push(
            Command::new(id::TOGGLE_HIDDEN, hidden, Group::View)
                .with_keywords(["dotfiles", "invisible"])
                .with_shortcut("Ctrl+H"),
        );
        if s.has_target() {
            out.push(
                Command::new(
                    id::QUICK_LOOK,
                    otto_kit::t_owned!("files-command-quick-look"),
                    Group::View,
                )
                .with_keywords(["preview", "peek"])
                .with_shortcut("Space"),
            );
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browsing() -> Situation {
        Situation {
            path: PathBuf::from("/home/someone"),
            cursor_name: Some("notes.txt".into()),
            has_entries: true,
            can_go_up: true,
            view: "list".into(),
            sort: "name".into(),
            ..Situation::default()
        }
    }

    fn ids(situation: &Situation) -> Vec<String> {
        Builtin
            .commands(situation)
            .into_iter()
            .map(|command| command.id)
            .collect()
    }

    #[test]
    fn a_command_whose_preconditions_are_unmet_is_not_offered() {
        let quiet = Situation {
            can_paste: false,
            can_undo: false,
            ..browsing()
        };
        let offered = ids(&quiet);
        assert!(!offered.contains(&id::PASTE.to_string()));
        assert!(!offered.contains(&id::UNDO.to_string()));

        let ready = Situation {
            can_paste: true,
            can_undo: true,
            ..browsing()
        };
        let offered = ids(&ready);
        assert!(offered.contains(&id::PASTE.to_string()));
        assert!(offered.contains(&id::UNDO.to_string()));
    }

    #[test]
    fn the_trash_offers_its_own_commands_and_not_the_browsers() {
        let trash = Situation {
            trash: true,
            ..browsing()
        };
        let offered = ids(&trash);
        assert!(offered.contains(&id::PUT_BACK.to_string()));
        assert!(offered.contains(&id::EMPTY_TRASH.to_string()));
        assert!(!offered.contains(&id::TRASH.to_string()));
        assert!(!offered.contains(&id::RENAME.to_string()));
        assert!(!offered.contains(&id::PASTE.to_string()));
    }

    #[test]
    fn trashing_names_the_number_of_files_it_would_take() {
        let many = Situation {
            selection: vec!["/a".into(), "/b".into(), "/c".into()],
            ..browsing()
        };
        let trash = Builtin
            .commands(&many)
            .into_iter()
            .find(|command| command.id == id::TRASH)
            .expect("trash is offered");
        assert!(trash.title.contains('3'), "{}", trash.title);
    }

    #[test]
    fn renaming_is_offered_for_one_file_and_not_for_several() {
        assert!(ids(&browsing()).contains(&id::RENAME.to_string()));
        let many = Situation {
            selection: vec!["/a".into(), "/b".into()],
            ..browsing()
        };
        assert!(!ids(&many).contains(&id::RENAME.to_string()));
    }

    #[test]
    fn a_rename_starts_from_the_name_the_file_already_has() {
        let rename = Builtin
            .commands(&browsing())
            .into_iter()
            .find(|command| command.id == id::RENAME)
            .expect("rename is offered");
        let arg = rename.arg.expect("rename takes a name");
        assert_eq!(arg.initial.as_deref(), Some("notes.txt"));
    }

    #[test]
    fn the_places_argument_is_built_from_the_sidebar() {
        let with_places = Situation {
            places: vec![PlaceRef {
                label: "Downloads".into(),
                path: PathBuf::from("/home/someone/Downloads"),
            }],
            ..browsing()
        };
        let go = Builtin
            .commands(&with_places)
            .into_iter()
            .find(|command| command.id == id::GO_TO_PLACE)
            .expect("go to place is offered");
        let choices = go.arg.as_ref().and_then(ArgSpec::choices).expect("choices");
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].value, "/home/someone/Downloads");
    }

    #[test]
    fn the_view_already_on_is_not_offered_as_a_command() {
        let offered = ids(&browsing());
        assert!(!offered.contains(&id::VIEW_LIST.to_string()));
        assert!(offered.contains(&id::VIEW_GRID.to_string()));
        assert!(offered.contains(&id::CHANGE_VIEW.to_string()));
    }

    #[test]
    fn an_empty_query_keeps_every_command_in_provider_order() {
        let commands = Builtin.commands(&browsing());
        let ranked = rank(&commands, "   ");
        assert_eq!(ranked.len(), commands.len());
        assert_eq!(ranked[0].index, 0);
    }

    #[test]
    fn a_gapped_query_finds_the_command_it_meant() {
        let commands = Builtin.commands(&Situation {
            selection: vec!["/a".into()],
            ..browsing()
        });
        let ranked = rank(&commands, "mo tr");
        let best = &commands[ranked[0].index];
        assert_eq!(best.id, id::TRASH);
    }

    #[test]
    fn a_title_hit_outranks_a_keyword_hit() {
        let commands = vec![
            Command::new("a", "Duplicate", Group::File),
            Command::new("b", "Copy", Group::Edit).with_keywords(["duplicate"]),
        ];
        let ranked = rank(&commands, "duplicate");
        assert_eq!(commands[ranked[0].index].id, "a");
    }

    #[test]
    fn a_request_names_the_provider_it_belongs_to() {
        assert_eq!(Request::new("go_up", None).namespace(), None);
        assert_eq!(
            Request::new("scripts:reveal", None).namespace(),
            Some("scripts")
        );
    }

    #[test]
    fn the_registry_leaves_the_hosts_own_requests_to_the_host() {
        let mut registry = Registry::builtin();
        assert!(registry.run(&Request::new(id::GO_UP, None)).is_none());
    }
}
