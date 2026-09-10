//! The command palette's state: what is typed, what matches, what is
//! highlighted, and what an argument is being answered with.
//!
//! See `specs/file-command-palette.md`. Everything here is decided without a
//! window, a font or a disk: the palette is a state machine the host feeds
//! keys to and reads rows off. The one thing it cannot work out for itself is
//! what a half-typed path could be completed to — that is I/O, so the host
//! does it and hands the answers back through [`Palette::set_completions`].

use std::collections::HashSet;

use otto_kit::prelude::{KeyMods, TextInput, TextInputKey, TextInputResponse, TextInputStyle};

use crate::command::{rank, ArgKind, Choice, Command, Group, Request};

/// A key press, as the palette understands it.
///
/// Split rather than passed through whole because the same physical key means
/// different things here than in a plain field: Return runs a command, Tab
/// commits to one, and Escape unwinds a layer.
#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Up,
    Down,
    Home,
    End,
    Tab,
    Enter,
    Escape,
    /// Everything the text field itself handles — characters, Backspace,
    /// caret movement, clipboard.
    Edit(TextInputKey),
}

/// What the host should do about a key the palette has just taken.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Nothing happened; the key was not the palette's.
    Ignored,
    /// Something on screen changed.
    Changed,
    /// Close the palette without running anything.
    Close,
    /// Put this on the clipboard — a copy or cut inside the field.
    Clipboard(String),
    /// Run this, then close.
    Run(Request),
}

/// One line of the list, as the view draws it.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// A group's name, in the resting list only. Never highlighted.
    Heading(Group),
    /// A command, by its index into [`Palette::commands`].
    Command(usize),
    /// An answer to the argument being typed, by its index into the session's
    /// completions.
    Completion(usize),
    /// One line of a dry run — what the argument as typed would do to one
    /// thing — by its index into the session's preview. Never *chosen*:
    /// Return runs the command, not the line. But a line can be highlighted
    /// and toggled, which leaves its file out of the run.
    Preview(usize),
}

/// One line of the dry run on show: what a thing is, what it would become,
/// and whether it has been left out of the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewLine {
    pub from: String,
    pub to: String,
    pub conflict: bool,
    /// Toggled off: the file is named so it can be toggled back, but the
    /// command will not touch it and the dry run was made without it.
    pub excluded: bool,
}

impl PreviewLine {
    /// Lay a provider's dry run over the full list of targets, so the ones
    /// left out still have a line to be brought back by.
    ///
    /// A provider answers for the targets it was given — the included ones —
    /// and, when it answers one line per target, its lines are threaded back
    /// among the excluded names in the targets' order. A provider that says
    /// something else (one line for the whole batch, say) has its lines shown
    /// as they are, with the excluded names after them.
    pub fn merge(
        targets: &[String],
        excluded: &HashSet<String>,
        rows: Vec<crate::command::PreviewRow>,
    ) -> Vec<PreviewLine> {
        let left_out = |name: &String| PreviewLine {
            from: name.clone(),
            to: String::new(),
            conflict: false,
            excluded: true,
        };
        let included = targets
            .iter()
            .filter(|name| !excluded.contains(*name))
            .count();
        let mut rows = rows.into_iter().map(|row| PreviewLine {
            from: row.from,
            to: row.to,
            conflict: row.conflict,
            excluded: false,
        });
        if rows.len() == included {
            return targets
                .iter()
                .map(|name| {
                    if excluded.contains(name) {
                        left_out(name)
                    } else {
                        rows.next().expect("one row per included target")
                    }
                })
                .collect();
        }
        rows.chain(
            targets
                .iter()
                .filter(|name| excluded.contains(*name))
                .map(left_out),
        )
        .collect()
    }
}

/// Something an argument could be completed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// What the argument becomes if this is taken.
    pub value: String,
    /// What the user reads.
    pub title: String,
    pub subtitle: Option<String>,
}

impl Completion {
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

impl From<&Choice> for Completion {
    fn from(choice: &Choice) -> Self {
        Self {
            value: choice.value.clone(),
            title: choice.title.clone(),
            subtitle: choice.subtitle.clone(),
        }
    }
}

/// An argument being answered.
struct ArgSession {
    /// Which command asked, by its index into [`Palette::commands`].
    command: usize,
    input: TextInput,
    completions: Vec<Completion>,
    /// Which completion is highlighted, if any. `None` means the typed text
    /// stands on its own — which is the usual state for a path being typed.
    highlight: Option<usize>,
    /// The query the search was left on, so backing out restores it whole.
    /// The prefix is never edited a character at a time, so this is the only
    /// way back.
    saved_query: String,
    /// What the argument as typed would do, from the host, when the command
    /// can say. Shown as the rows while it is not empty.
    preview: Vec<PreviewLine>,
    /// The names toggled out of the run from the dry run's lines. The host
    /// leaves these out of what it asks the provider to preview and to do.
    excluded: HashSet<String>,
}

/// The palette.
pub struct Palette {
    /// Everything on offer, gathered once when the palette opened. Nothing
    /// re-gathers while it is up: the window is not changing underneath it.
    commands: Vec<Command>,
    query: TextInput,
    /// The ranked commands, best first. Empty query means all of them, in
    /// provider order.
    order: Vec<usize>,
    rows: Vec<Row>,
    /// Index into `rows`, always pointing at a highlightable row when there is
    /// one.
    highlight: usize,
    arg: Option<ArgSession>,
    /// Why the last attempt did not work, shown in place of the list.
    error: Option<String>,
    /// What the argument being typed is doing right now — how many items a
    /// pattern has picked, say. Set by the host after each preview and shown
    /// where an error would be; cleared by the next key like an error is,
    /// since it was about text that is now being changed.
    note: Option<String>,
    /// The first row on screen. The list is longer than the card whenever the
    /// query is short, and this is what keeps the highlight in view.
    /// How the field is drawn. Held rather than looked up, because argument
    /// mode builds a second field and the two must match — and because a test
    /// has no theme to look one up from.
    style: TextInputStyle,
}

impl Palette {
    /// Open onto `commands`, resting: no query, everything grouped.
    pub fn open(commands: Vec<Command>, style: TextInputStyle) -> Self {
        let mut palette = Self {
            commands,
            query: field("", &style),
            order: Vec::new(),
            rows: Vec::new(),
            highlight: 0,
            arg: None,
            error: None,
            note: None,
            style,
        };
        palette.refilter();
        palette
    }

    // === What the view reads ===

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Which row is highlighted, or `None` when there is nothing to highlight.
    ///
    /// Argument mode answers from the argument's own highlight rather than
    /// from the command list's. The two are separate — backing out of an
    /// argument has to put the highlight back on the command it came from —
    /// and reporting the wrong one here draws a bar that Return does not act
    /// on, which is worse than drawing none. `None` in argument mode means the
    /// typed text stands on its own, which is the usual state for a path.
    pub fn highlighted(&self) -> Option<usize> {
        if let Some(session) = self.arg.as_ref() {
            // Rows are one-to-one with completions here, so the completion's
            // index is the row's.
            return session.highlight;
        }
        self.rows.get(self.highlight).map(|_| self.highlight)
    }

    /// The field the caret is in — the query, or the argument.
    pub fn input(&self) -> &TextInput {
        match &self.arg {
            Some(session) => &session.input,
            None => &self.query,
        }
    }

    pub fn input_mut(&mut self) -> &mut TextInput {
        match &mut self.arg {
            Some(session) => &mut session.input,
            None => &mut self.query,
        }
    }

    /// The non-editable prefix in front of the field, with its colon, while an
    /// argument is being answered. `None` in search mode.
    pub fn prompt(&self) -> Option<String> {
        let session = self.arg.as_ref()?;
        let spec = self.commands[session.command].arg.as_ref()?;
        Some(format!("{}:", spec.prompt))
    }

    /// Dim text standing in for an empty field.
    pub fn placeholder(&self) -> String {
        match &self.arg {
            Some(session) => self.commands[session.command]
                .arg
                .as_ref()
                .and_then(|spec| spec.placeholder.clone())
                .unwrap_or_default(),
            None => otto_kit::t_owned!("files-palette-placeholder"),
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The live result of the argument being typed, if the host has one.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Put the highlight on `row`, if it is one that can be chosen. What the
    /// pointer does when it moves over the list, and the first half of what a
    /// click on a row does.
    pub fn highlight_row(&mut self, row: usize) -> bool {
        match self.rows.get(row) {
            Some(Row::Command(_)) | Some(Row::Completion(_)) | Some(Row::Preview(_)) => {
                self.highlight = row;
                if let (Some(session), Some(Row::Completion(index) | Row::Preview(index))) =
                    (self.arg.as_mut(), self.rows.get(row))
                {
                    session.highlight = Some(*index);
                }
                true
            }
            _ => false,
        }
    }

    /// Toggle the dry-run line at `row` in or out of the run — what a click
    /// on one does. `false` when the row is not a dry-run line.
    pub fn toggle_row(&mut self, row: usize) -> bool {
        match self.rows.get(row) {
            Some(Row::Preview(index)) => {
                let index = *index;
                self.highlight_row(row);
                self.toggle_line(index)
            }
            _ => false,
        }
    }

    fn toggle_line(&mut self, index: usize) -> bool {
        let Some(session) = self.arg.as_mut() else {
            return false;
        };
        let Some(line) = session.preview.get_mut(index) else {
            return false;
        };
        line.excluded = !line.excluded;
        if line.excluded {
            session.excluded.insert(line.from.clone());
        } else {
            session.excluded.remove(&line.from);
        }
        true
    }

    /// Whether a dry-run line has the highlight, which is when Space toggles
    /// rather than types.
    fn on_preview_line(&self) -> bool {
        self.arg.as_ref().is_some_and(|session| {
            !session.preview.is_empty()
                && session
                    .highlight
                    .is_some_and(|index| index < session.preview.len())
        })
    }

    /// The names toggled out of the run. Empty outside argument mode.
    pub fn excluded(&self) -> HashSet<String> {
        self.arg
            .as_ref()
            .map(|session| session.excluded.clone())
            .unwrap_or_default()
    }

    /// Whether the list is the grouped resting one rather than a ranked list.
    pub fn resting(&self) -> bool {
        self.arg.is_none() && self.query.value().trim().is_empty()
    }

    pub fn completions(&self) -> &[Completion] {
        match &self.arg {
            Some(session) => &session.completions,
            None => &[],
        }
    }

    /// The command an argument is being typed for.
    pub fn arg_command(&self) -> Option<&Command> {
        self.arg
            .as_ref()
            .map(|session| &self.commands[session.command])
    }

    /// The argument being typed, when its command asked to be shown as it is
    /// written. The host applies it after each key and takes it back if the
    /// palette is abandoned.
    pub fn previewed_argument(&self) -> Option<(&str, &str)> {
        let session = self.arg.as_ref()?;
        let command = &self.commands[session.command];
        command
            .arg
            .as_ref()
            .filter(|spec| spec.preview)
            .map(|_| (command.id.as_str(), session.input.value()))
    }

    /// The half-typed path the host should complete against, and whether only
    /// directories count. `None` unless a path argument is open — which is the
    /// palette's whole involvement with the disk.
    pub fn path_argument(&self) -> Option<(&str, bool)> {
        let session = self.arg.as_ref()?;
        match self.commands[session.command].arg.as_ref()?.kind {
            ArgKind::Path { dirs_only } => Some((session.input.value(), dirs_only)),
            _ => None,
        }
    }

    /// Hand back what a path could be completed to.
    ///
    /// Answering with the list that is already there changes nothing — not
    /// even the highlight. The host re-completes after every key, arrow keys
    /// included, and an arrow key that moved the highlight must not have it
    /// taken away again by the refresh that follows.
    pub fn set_completions(&mut self, completions: Vec<Completion>) {
        if let Some(session) = self.arg.as_mut() {
            if session.completions == completions {
                return;
            }
            session.completions = completions;
            // A different list: the old position in it means nothing.
            session.highlight = None;
        }
        self.rebuild_rows();
    }

    /// Say why the last attempt did not work. The palette stays open with the
    /// text still there to fix.
    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    /// Say what the argument as typed is doing — the answer a previewed
    /// argument produced. Shown until the next key changes the question.
    pub fn set_note(&mut self, note: Option<String>) {
        self.note = note;
    }

    /// Show what the argument as typed would do, line by line, in place of
    /// the completions. Nothing outside argument mode.
    pub fn set_preview(&mut self, lines: Vec<PreviewLine>) {
        let Some(session) = self.arg.as_mut() else {
            return;
        };
        if session.preview == lines {
            return;
        }
        session.preview = lines;
        // A highlight past the end of a shorter list means nothing.
        if session
            .highlight
            .is_some_and(|index| index >= session.preview.len())
        {
            session.highlight = None;
        }
        self.rebuild_rows();
    }

    /// The dry run on show, if any.
    pub fn preview(&self) -> &[PreviewLine] {
        match &self.arg {
            Some(session) => &session.preview,
            None => &[],
        }
    }

    // === Keys ===

    pub fn on_key(&mut self, key: Key, mods: KeyMods) -> Outcome {
        // Any key at all clears the last complaint: it was about text that is
        // now being changed.
        let had_error = self.error.take().is_some();
        self.note = None;
        let outcome = if self.arg.is_some() {
            self.arg_key(key, mods)
        } else {
            self.search_key(key, mods)
        };
        match (outcome, had_error) {
            (Outcome::Ignored, true) => Outcome::Changed,
            (outcome, _) => outcome,
        }
    }

    fn search_key(&mut self, key: Key, mods: KeyMods) -> Outcome {
        match key {
            Key::Escape => Outcome::Close,
            Key::Up => {
                self.step_highlight(-1);
                Outcome::Changed
            }
            Key::Down => {
                self.step_highlight(1);
                Outcome::Changed
            }
            // In search mode the list is what Home and End are about — the
            // query is short and its ends are one Left or Right away.
            Key::Home => {
                self.highlight = 0;
                self.settle_highlight(1);
                Outcome::Changed
            }
            Key::End => {
                self.highlight = self.rows.len().saturating_sub(1);
                self.settle_highlight(-1);
                Outcome::Changed
            }
            // Both commit to the highlighted command. They agree deliberately:
            // a user who only ever presses Return still lands in the field the
            // command was going to need.
            Key::Enter | Key::Tab => match self.highlighted_command() {
                Some(index) if self.commands[index].arg.is_some() => {
                    self.enter_arg(index);
                    Outcome::Changed
                }
                Some(index) if key == Key::Enter => {
                    Outcome::Run(Request::new(self.commands[index].id.clone(), None))
                }
                _ => Outcome::Ignored,
            },
            Key::Edit(edit) => {
                let response = self.query.on_key(edit, mods);
                if let TextInputResponse::Clipboard(text) = response {
                    return Outcome::Clipboard(text);
                }
                self.refilter();
                Outcome::Changed
            }
        }
    }

    fn arg_key(&mut self, key: Key, mods: KeyMods) -> Outcome {
        match key {
            // One layer at a time: out of the argument, then out of the
            // palette. Escape never runs anything.
            Key::Escape => {
                self.leave_arg();
                Outcome::Changed
            }
            Key::Up => {
                self.step_completion(-1);
                Outcome::Changed
            }
            Key::Down => {
                self.step_completion(1);
                Outcome::Changed
            }
            Key::Enter => {
                let session = self.arg.as_ref().expect("in argument mode");
                let command = &self.commands[session.command];
                let value = match session.highlight.and_then(|i| session.completions.get(i)) {
                    Some(completion) => completion.value.clone(),
                    None => session.input.value().to_string(),
                };
                Outcome::Run(Request::new(command.id.clone(), Some(value)))
            }
            Key::Tab => {
                self.complete();
                Outcome::Changed
            }
            // Down into the dry run, Space to leave a line out or bring it
            // back. Only there: in the field, a space is a space.
            Key::Edit(TextInputKey::Char(' ')) if self.on_preview_line() => {
                if let Some(index) = self.arg.as_ref().and_then(|session| session.highlight) {
                    self.toggle_line(index);
                }
                Outcome::Changed
            }
            Key::Edit(TextInputKey::Backspace) if self.arg_is_empty() => {
                // The prefix is not text and is never eaten a character at a
                // time: Backspace at the start of an empty argument is the way
                // back to the query that was typed.
                self.leave_arg();
                Outcome::Changed
            }
            // Here the field is the point, so Home and End are the caret's.
            Key::Home => self.edit_arg(TextInputKey::Home, mods),
            Key::End => self.edit_arg(TextInputKey::End, mods),
            Key::Edit(edit) => self.edit_arg(edit, mods),
        }
    }

    fn edit_arg(&mut self, edit: TextInputKey, mods: KeyMods) -> Outcome {
        let Some(session) = self.arg.as_mut() else {
            return Outcome::Ignored;
        };
        // Editing is being back in the field: a highlight left on a dry-run
        // line would make the next Space a toggle instead of a space.
        if !session.preview.is_empty() {
            session.highlight = None;
        }
        let response = session.input.on_key(edit, mods);
        if let TextInputResponse::Clipboard(text) = response {
            return Outcome::Clipboard(text);
        }
        self.refilter_choices();
        Outcome::Changed
    }

    // === Moving about ===

    fn highlighted_command(&self) -> Option<usize> {
        match self.rows.get(self.highlight) {
            Some(Row::Command(index)) => Some(*index),
            _ => None,
        }
    }

    fn step_highlight(&mut self, direction: isize) {
        if self.rows.is_empty() {
            return;
        }
        let mut cursor = self.highlight as isize;
        // Headings are read, not chosen: the arrow keys walk past them.
        loop {
            cursor += direction;
            if cursor < 0 || cursor as usize >= self.rows.len() {
                return;
            }
            if matches!(self.rows[cursor as usize], Row::Command(_)) {
                self.highlight = cursor as usize;
                return;
            }
        }
    }

    /// Nudge the highlight onto a choosable row, looking in `direction`.
    fn settle_highlight(&mut self, direction: isize) {
        if matches!(self.rows.get(self.highlight), Some(Row::Command(_))) {
            return;
        }
        self.step_highlight(direction);
    }

    fn step_completion(&mut self, direction: isize) {
        let Some(session) = self.arg.as_mut() else {
            return;
        };
        // The list is the dry run while there is one, the completions
        // otherwise — whichever the rows are showing.
        let len = if session.preview.is_empty() {
            session.completions.len()
        } else {
            session.preview.len()
        };
        if len == 0 {
            return;
        }
        let last = len as isize - 1;
        session.highlight = Some(match session.highlight {
            // Down out of the field lands on the first answer; up out of it
            // lands on the last, so both ends are one key away.
            None if direction > 0 => 0,
            None => last as usize,
            Some(current) => (current as isize + direction).clamp(0, last) as usize,
        });
        self.rebuild_rows();
    }

    // === Modes ===

    fn enter_arg(&mut self, command: usize) {
        let Some(spec) = self.commands[command].arg.clone() else {
            return;
        };
        let (initial, highlight) = match &spec.kind {
            // A choice's `initial` names the answer that is already true, so
            // it is highlighted rather than typed: one arrow key and Return
            // is the whole interaction.
            ArgKind::Choice(choices) => (
                String::new(),
                spec.initial
                    .as_ref()
                    .and_then(|value| choices.iter().position(|c| &c.value == value)),
            ),
            _ => (spec.initial.clone().unwrap_or_default(), None),
        };
        let mut input = field(&initial, &self.style);
        if !initial.is_empty() {
            // A name is retyped up to its extension — "Archive" in
            // "Archive.zip" — so the kind of file stays put. Anything else
            // is replaced whole.
            let keep_extension = matches!(spec.kind, ArgKind::Text);
            let stem = initial
                .rfind('.')
                .filter(|&dot| keep_extension && dot > 0 && dot + 1 < initial.len())
                .unwrap_or(initial.len());
            input.state.select_range(0..stem);
        }
        self.arg = Some(ArgSession {
            command,
            input,
            completions: match &spec.kind {
                ArgKind::Choice(choices) => choices.iter().map(Completion::from).collect(),
                // A path's answers arrive from the host; free text has none.
                _ => Vec::new(),
            },
            highlight,
            saved_query: self.query.value().to_string(),
            preview: Vec::new(),
            excluded: HashSet::new(),
        });
        self.rebuild_rows();
    }

    fn leave_arg(&mut self) {
        let Some(session) = self.arg.take() else {
            return;
        };
        let command = session.command;
        self.query = field(&session.saved_query, &self.style);
        self.query
            .state
            .set_caret(session.saved_query.chars().count(), false);
        self.refilter();
        // The command that was being answered stays highlighted: backing out
        // of the argument is not backing out of the choice.
        if let Some(row) = self
            .rows
            .iter()
            .position(|row| matches!(row, Row::Command(index) if *index == command))
        {
            self.highlight = row;
        }
    }

    fn arg_is_empty(&self) -> bool {
        self.arg
            .as_ref()
            .is_some_and(|session| session.input.value().is_empty())
    }

    /// Tab: take the highlighted answer, or extend as far as every answer
    /// agrees.
    fn complete(&mut self) {
        let Some(session) = self.arg.as_mut() else {
            return;
        };
        if let Some(completion) = session.highlight.and_then(|i| session.completions.get(i)) {
            let value = completion.value.clone();
            session.input.set_value(value);
            session.input.state.set_caret(usize::MAX, false);
            session.highlight = None;
        } else if let Some(prefix) = common_prefix(&session.completions) {
            if prefix.chars().count() > session.input.value().chars().count() {
                session.input.set_value(prefix);
                session.input.state.set_caret(usize::MAX, false);
            }
        }
        self.refilter_choices();
    }

    // === Lists ===

    fn refilter(&mut self) {
        let query = self.query.value().to_string();
        self.order = rank(&self.commands, &query)
            .into_iter()
            .map(|m| m.index)
            .collect();
        self.rebuild_rows();
        // A fresh query is a fresh list: start at the top of it.
        self.highlight = 0;
        self.settle_highlight(1);
    }

    /// A choice argument filters itself — its answers are already in hand, so
    /// typing at them is ranking, not I/O.
    fn refilter_choices(&mut self) {
        let Some(session) = self.arg.as_ref() else {
            return;
        };
        let Some(spec) = self.commands[session.command].arg.as_ref() else {
            return;
        };
        let ArgKind::Choice(choices) = &spec.kind else {
            return;
        };
        let typed = session.input.value().to_string();
        let filtered: Vec<Completion> = if typed.trim().is_empty() {
            choices.iter().map(Completion::from).collect()
        } else {
            let mut scored: Vec<(i32, Completion)> = choices
                .iter()
                .filter_map(|choice| {
                    otto_kit::matching::score(&choice.title, &typed)
                        .map(|score| (score, Completion::from(choice)))
                })
                .collect();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            scored
                .into_iter()
                .map(|(_, completion)| completion)
                .collect()
        };
        let session = self.arg.as_mut().expect("checked above");
        session.completions = filtered;
        session.highlight =
            session
                .highlight
                .filter(|_| false)
                .or(if session.completions.is_empty() {
                    None
                } else {
                    Some(0)
                });
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        if let Some(session) = self.arg.as_ref() {
            // A dry run stands in for the completions while there is one: the
            // two are never both worth showing, and a command that previews
            // is one whose argument is free text with nothing to complete.
            if !session.preview.is_empty() {
                self.rows
                    .extend((0..session.preview.len()).map(Row::Preview));
            } else {
                self.rows
                    .extend((0..session.completions.len()).map(Row::Completion));
            }
            return;
        }
        if self.resting() {
            // Grouped, in the fixed order the groups are declared in. A group
            // nothing is offered from is not named.
            for group in Group::ALL {
                let mut named = false;
                for &index in &self.order {
                    if self.commands[index].group != group {
                        continue;
                    }
                    if !named {
                        self.rows.push(Row::Heading(group));
                        named = true;
                    }
                    self.rows.push(Row::Command(index));
                }
            }
        } else {
            self.rows
                .extend(self.order.iter().copied().map(Row::Command));
        }
    }
}

fn field(value: &str, style: &TextInputStyle) -> TextInput {
    let mut input = TextInput::new(value, style.clone());
    input.state.set_focused(true);
    input
}

/// The longest start every completion agrees on, or `None` when there is
/// nothing to agree about.
fn common_prefix(completions: &[Completion]) -> Option<String> {
    let first = completions.first()?;
    let mut prefix: Vec<char> = first.value.chars().collect();
    for completion in &completions[1..] {
        let shared = completion
            .value
            .chars()
            .zip(prefix.iter())
            .take_while(|(a, b)| a == *b)
            .count();
        prefix.truncate(shared);
        if prefix.is_empty() {
            return None;
        }
    }
    Some(prefix.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{id, ArgSpec, Group};

    fn mods() -> KeyMods {
        KeyMods {
            shift: false,
            ctrl: false,
        }
    }

    fn type_text(palette: &mut Palette, text: &str) {
        for ch in text.chars() {
            palette.on_key(Key::Edit(TextInputKey::Char(ch)), mods());
        }
    }

    fn open(commands: Vec<Command>) -> Palette {
        Palette::open(commands, TextInputStyle::new())
    }

    fn commands() -> Vec<Command> {
        vec![
            Command::new(id::GO_UP, "Up", Group::Go),
            Command::new(id::GO_TO_PATH, "Go to Path", Group::Go).with_arg(ArgSpec::new(
                "Go to path",
                "path",
                ArgKind::Path { dirs_only: false },
            )),
            Command::new(id::TRASH, "Move to Trash", Group::File),
            Command::new(id::CHANGE_VIEW, "Change View", Group::View).with_arg(
                ArgSpec::new(
                    "View",
                    "view",
                    ArgKind::Choice(vec![
                        Choice::new("list", "List"),
                        Choice::new("grid", "Grid"),
                        Choice::new("columns", "Columns"),
                    ]),
                )
                .with_initial("grid"),
            ),
        ]
    }

    fn highlighted_id(palette: &Palette) -> Option<&str> {
        match palette.rows().get(palette.highlighted()?)? {
            Row::Command(index) => Some(palette.commands()[*index].id.as_str()),
            _ => None,
        }
    }

    #[test]
    fn the_resting_list_is_grouped_and_headings_are_not_chosen() {
        let palette = open(commands());
        assert!(palette.resting());
        assert!(matches!(palette.rows()[0], Row::Heading(Group::Go)));
        // The highlight starts on the first *command*, never on the heading.
        assert_eq!(highlighted_id(&palette), Some(id::GO_UP));
    }

    #[test]
    fn arrowing_down_walks_past_the_headings() {
        let mut palette = open(commands());
        let mut seen = vec![highlighted_id(&palette).unwrap().to_string()];
        for _ in 0..3 {
            palette.on_key(Key::Down, mods());
            seen.push(highlighted_id(&palette).unwrap().to_string());
        }
        assert_eq!(
            seen,
            vec![id::GO_UP, id::GO_TO_PATH, id::TRASH, id::CHANGE_VIEW]
        );
    }

    #[test]
    fn typing_flattens_the_list_and_ranks_it() {
        let mut palette = open(commands());
        type_text(&mut palette, "mo tr");
        assert!(!palette.resting());
        assert!(palette
            .rows()
            .iter()
            .all(|row| matches!(row, Row::Command(_))));
        assert_eq!(highlighted_id(&palette), Some(id::TRASH));
    }

    #[test]
    fn return_runs_a_command_that_asks_for_nothing() {
        let mut palette = open(commands());
        type_text(&mut palette, "up");
        let outcome = palette.on_key(Key::Enter, mods());
        assert_eq!(outcome, Outcome::Run(Request::new(id::GO_UP, None)));
    }

    #[test]
    fn return_on_a_command_that_wants_an_argument_asks_for_it_instead() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        assert_eq!(palette.on_key(Key::Enter, mods()), Outcome::Changed);
        assert_eq!(palette.prompt().as_deref(), Some("Go to path:"));
        assert_eq!(
            palette.arg_command().map(|c| c.id.as_str()),
            Some(id::GO_TO_PATH)
        );
    }

    #[test]
    fn tab_asks_for_the_argument_the_same_way_return_does() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        assert_eq!(palette.on_key(Key::Tab, mods()), Outcome::Changed);
        assert_eq!(palette.prompt().as_deref(), Some("Go to path:"));
    }

    #[test]
    fn tab_on_a_command_with_nothing_to_ask_does_nothing() {
        let mut palette = open(commands());
        type_text(&mut palette, "up");
        assert_eq!(palette.on_key(Key::Tab, mods()), Outcome::Ignored);
        assert!(palette.prompt().is_none());
    }

    #[test]
    fn what_is_typed_after_the_prompt_is_the_argument_not_a_query() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        type_text(&mut palette, "/etc");
        assert_eq!(palette.input().value(), "/etc");
        assert_eq!(
            palette.on_key(Key::Enter, mods()),
            Outcome::Run(Request::new(id::GO_TO_PATH, Some("/etc".into())))
        );
    }

    #[test]
    fn backspacing_out_of_an_empty_argument_restores_the_query() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        type_text(&mut palette, "/e");
        palette.on_key(Key::Edit(TextInputKey::Backspace), mods());
        palette.on_key(Key::Edit(TextInputKey::Backspace), mods());
        // Still in the argument: it only just became empty.
        assert!(palette.prompt().is_some());
        palette.on_key(Key::Edit(TextInputKey::Backspace), mods());
        assert!(palette.prompt().is_none());
        assert_eq!(palette.input().value(), "go to path");
        assert_eq!(highlighted_id(&palette), Some(id::GO_TO_PATH));
    }

    #[test]
    fn escape_leaves_the_argument_before_it_closes_the_palette() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        assert_eq!(palette.on_key(Key::Escape, mods()), Outcome::Changed);
        assert!(palette.prompt().is_none());
        assert_eq!(palette.on_key(Key::Escape, mods()), Outcome::Close);
    }

    #[test]
    fn a_choice_argument_opens_on_the_answer_that_is_already_true() {
        let mut palette = open(commands());
        type_text(&mut palette, "change view");
        palette.on_key(Key::Tab, mods());
        assert_eq!(palette.completions().len(), 3);
        // "grid" was the initial, so it is the one Return would take.
        assert_eq!(
            palette.on_key(Key::Enter, mods()),
            Outcome::Run(Request::new(id::CHANGE_VIEW, Some("grid".into())))
        );
    }

    #[test]
    fn one_arrow_key_and_return_answers_a_choice() {
        let mut palette = open(commands());
        type_text(&mut palette, "change view");
        palette.on_key(Key::Tab, mods());
        palette.on_key(Key::Down, mods());
        assert_eq!(
            palette.on_key(Key::Enter, mods()),
            Outcome::Run(Request::new(id::CHANGE_VIEW, Some("columns".into())))
        );
    }

    #[test]
    fn typing_at_a_choice_narrows_it_without_touching_the_disk() {
        let mut palette = open(commands());
        type_text(&mut palette, "change view");
        palette.on_key(Key::Tab, mods());
        type_text(&mut palette, "col");
        assert_eq!(palette.completions().len(), 1);
        assert_eq!(
            palette.on_key(Key::Enter, mods()),
            Outcome::Run(Request::new(id::CHANGE_VIEW, Some("columns".into())))
        );
    }

    #[test]
    fn the_host_is_asked_to_complete_a_path_and_nothing_else() {
        let mut palette = open(commands());
        type_text(&mut palette, "change view");
        palette.on_key(Key::Tab, mods());
        assert!(palette.path_argument().is_none());

        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        type_text(&mut palette, "/us");
        assert_eq!(palette.path_argument(), Some(("/us", false)));
    }

    #[test]
    fn tab_extends_a_path_as_far_as_every_answer_agrees() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        type_text(&mut palette, "/us");
        palette.set_completions(vec![
            Completion::new("/usr/share", "share"),
            Completion::new("/usr/src", "src"),
        ]);
        palette.on_key(Key::Tab, mods());
        assert_eq!(palette.input().value(), "/usr/s");
    }

    #[test]
    fn tab_on_a_highlighted_answer_takes_it_whole() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        palette.set_completions(vec![
            Completion::new("/usr/share", "share"),
            Completion::new("/usr/src", "src"),
        ]);
        palette.on_key(Key::Down, mods());
        palette.on_key(Key::Tab, mods());
        assert_eq!(palette.input().value(), "/usr/share");
    }

    /// The bug this pins: the palette holds a command highlight and an
    /// argument highlight, and argument mode has to report the second. It once
    /// reported the first, so the bar sat on whatever row the command had
    /// occupied and no arrow key moved it.
    #[test]
    fn the_arrows_move_the_highlight_through_the_answers() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        palette.set_completions(vec![
            Completion::new("/usr/bin", "bin"),
            Completion::new("/usr/lib", "lib"),
            Completion::new("/usr/src", "src"),
        ]);
        // Nothing is highlighted until an arrow key says so: what was typed
        // stands on its own.
        assert_eq!(palette.highlighted(), None);
        palette.on_key(Key::Down, mods());
        assert_eq!(palette.highlighted(), Some(0));
        palette.on_key(Key::Down, mods());
        assert_eq!(palette.highlighted(), Some(1));
        palette.on_key(Key::Up, mods());
        assert_eq!(palette.highlighted(), Some(0));
    }

    /// And what is highlighted is what Return acts on.
    #[test]
    fn the_highlight_the_arrows_moved_is_the_one_return_takes() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        palette.set_completions(vec![
            Completion::new("/usr/bin", "bin"),
            Completion::new("/usr/lib", "lib"),
        ]);
        palette.on_key(Key::Down, mods());
        palette.on_key(Key::Down, mods());
        assert_eq!(palette.highlighted(), Some(1));
        assert_eq!(
            palette.on_key(Key::Enter, mods()),
            Outcome::Run(Request::new(id::GO_TO_PATH, Some("/usr/lib".into())))
        );
    }

    #[test]
    fn return_on_a_highlighted_answer_runs_with_it() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        palette.set_completions(vec![Completion::new("/usr/share", "share")]);
        palette.on_key(Key::Down, mods());
        assert_eq!(
            palette.on_key(Key::Enter, mods()),
            Outcome::Run(Request::new(id::GO_TO_PATH, Some("/usr/share".into())))
        );
    }

    #[test]
    fn a_query_matching_nothing_has_nothing_to_run() {
        let mut palette = open(commands());
        type_text(&mut palette, "zzzz");
        assert!(palette.rows().is_empty());
        assert_eq!(palette.on_key(Key::Enter, mods()), Outcome::Ignored);
    }

    #[test]
    fn a_rejected_argument_leaves_the_text_there_to_fix() {
        let mut palette = open(commands());
        type_text(&mut palette, "go to path");
        palette.on_key(Key::Tab, mods());
        type_text(&mut palette, "/nowhere");
        palette.set_error("No such folder");
        assert_eq!(palette.error(), Some("No such folder"));
        palette.set_note(Some("3 of 9 selected".into()));
        assert_eq!(palette.note(), Some("3 of 9 selected"));
        // The next keystroke is about the text, so the complaint goes.
        palette.on_key(Key::Edit(TextInputKey::Backspace), mods());
        assert!(palette.error().is_none());
        assert!(
            palette.note().is_none(),
            "a note is about text that has now changed"
        );
        assert_eq!(palette.input().value(), "/nowher");
    }
}
