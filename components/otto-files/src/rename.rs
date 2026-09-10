//! Renaming several files at once from one pattern.
//!
//! The pattern is the new name with holes in it, and the holes are filled from
//! each file in turn: its number in the selection, its original name, or parts
//! of that name picked out by position. Nothing here touches the disk — it
//! turns names into names, so the palette can show the outcome as it is typed
//! and the host can carry it out afterwards from the same answer.
//!
//! ## The pattern
//!
//! Plain text stands for itself. Between braces:
//!
//! - `{name}` — the original name without its extension; `{ext}` — the
//!   extension without its dot.
//! - `{n}` — the file's number in the selection, from 1. `{n:3}` pads it to
//!   three digits, `{n@10}` starts counting at ten; both may be combined.
//! - `{1}`, `{2}` … — the words of the original name, counted from 1, split
//!   wherever it has a space, an underscore, a dash or a dot; `{-1}` is the
//!   last. `{2..}`, `{1..3}` and `{..-2}` take a run of them, joined by a
//!   space — so `IMG_2024_Paris` gives `{3}` = `Paris` and `{2..}` =
//!   `2024 Paris`.
//! - `{name:1..4}` — the characters of the original name by position, the
//!   same way: `{name:5..}` from the fifth on, `{name:-3..}` the last three.
//!
//! Positions are counted from one, ranges include both ends, and a negative
//! position counts from the back — one rule for words and characters alike.
//!
//! A brace that does not spell one of these is left exactly as typed, so a
//! half-written hole reads as what it is rather than vanishing. If the pattern
//! names no extension — no `{ext}` and no dot anywhere in it — the original
//! extension is kept, which is what "`Holiday {n}`" means nine times in ten.

/// One file's outcome under a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub old: String,
    pub new: String,
    pub state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The pattern leaves this name as it is.
    Same,
    /// A new name, and it is free.
    Changed,
    /// The new name is taken — by another file in the same batch, or by
    /// something already in the directory that is not being renamed.
    Conflict,
    /// The pattern made no usable name at all: empty, or with a slash in it.
    Invalid,
}

/// Work out what every name becomes.
///
/// `names` are the files in the order they are numbered; `existing` is
/// everything else in the directory, so a new name that would land on a file
/// outside the batch is caught before anything moves.
pub fn plan(names: &[String], pattern: &str, existing: &[String]) -> Vec<Rename> {
    let mut out: Vec<Rename> = names
        .iter()
        .enumerate()
        .map(|(index, old)| {
            let new = expand(pattern, old, index + 1);
            let state = if new == *old {
                State::Same
            } else if new.is_empty() || new.contains('/') || new == "." || new == ".." {
                State::Invalid
            } else {
                State::Changed
            };
            Rename {
                old: old.clone(),
                new,
                state,
            }
        })
        .collect();

    // A name is taken if another file in the batch ends up with it too, or if
    // something that is staying put already has it. A file keeping its own
    // name takes nothing from anyone.
    for i in 0..out.len() {
        if out[i].state != State::Changed {
            continue;
        }
        let taken_in_batch = out
            .iter()
            .enumerate()
            .any(|(j, other)| j != i && other.new == out[i].new && other.state != State::Invalid);
        let taken_outside = existing
            .iter()
            .any(|name| *name == out[i].new && !names.contains(name));
        if taken_in_batch || taken_outside {
            out[i].state = State::Conflict;
        }
    }
    out
}

/// Fill one name's holes.
pub fn expand(pattern: &str, original: &str, number: usize) -> String {
    let (stem, ext) = split_ext(original);
    let mut out = String::new();
    let mut rest = pattern;
    let mut names_ext = false;
    // Only a dot in the pattern's *own* text names an extension — the dots
    // inside `{2..}` or `{name:1..3}` are part of the hole.
    let mut literal_dot = false;
    while let Some(open) = rest.find('{') {
        literal_dot |= rest[..open].contains('.');
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let hole = &after[..close];
                match fill(hole, stem, ext, number) {
                    Some(text) => {
                        names_ext |= hole.trim() == "ext";
                        out.push_str(&text);
                    }
                    // Not a hole this module knows: left as typed.
                    None => {
                        out.push('{');
                        out.push_str(hole);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                literal_dot |= rest[open..].contains('.');
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    literal_dot |= rest.contains('.');
    out.push_str(rest);

    if !names_ext && !literal_dot && !ext.is_empty() && !out.is_empty() {
        out.push('.');
        out.push_str(ext);
    }
    out
}

/// What one hole stands for, or `None` for something that is not a hole.
fn fill(hole: &str, stem: &str, ext: &str, number: usize) -> Option<String> {
    let hole = hole.trim();
    if hole == "name" {
        return Some(stem.to_string());
    }
    if hole == "ext" {
        return Some(ext.to_string());
    }
    if let Some(spec) = hole.strip_prefix("name:") {
        let chars: Vec<char> = stem.chars().collect();
        let (from, to) = parse_range(spec, chars.len())?;
        return Some(chars[from..to].iter().collect());
    }
    if let Some(spec) = hole.strip_prefix('n') {
        return fill_number(spec, number);
    }
    // Words: `{2}`, `{-1}`, `{2..}`, `{1..3}`, `{..-2}`.
    let words = words(stem);
    let (from, to) = parse_range(hole, words.len())?;
    Some(words[from..to].join(" "))
}

/// `{n}`, with `:W` for zero-padding to `W` digits and `@S` to start at `S`,
/// in either order.
fn fill_number(spec: &str, number: usize) -> Option<String> {
    let mut width = 0usize;
    let mut start = 1i64;
    let mut rest = spec;
    while !rest.is_empty() {
        let (key, tail) = rest.split_at(1);
        let digits: String = tail
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        if digits.is_empty() {
            return None;
        }
        match key {
            ":" => width = digits.parse().ok()?,
            "@" => start = digits.parse().ok()?,
            _ => return None,
        }
        rest = &tail[digits.len()..];
    }
    let value = start + number as i64 - 1;
    Some(format!("{value:0width$}"))
}

/// A one-based, inclusive range over `len` things — a single position, a run,
/// or an open end — with negatives counted from the back. `None` for a spec
/// that is not one, or one that lies entirely outside; a run clipped by the
/// end is simply shorter.
fn parse_range(spec: &str, len: usize) -> Option<(usize, usize)> {
    let resolve = |text: &str, default: i64| -> Option<i64> {
        if text.is_empty() {
            return Some(default);
        }
        let value: i64 = text.parse().ok()?;
        if value == 0 {
            return None;
        }
        Some(if value < 0 {
            len as i64 + 1 + value
        } else {
            value
        })
    };
    let (from, to) = match spec.split_once("..") {
        Some((a, b)) => (resolve(a, 1)?, resolve(b, len as i64)?),
        None => {
            let one = resolve(spec, 0)?;
            (one, one)
        }
    };
    let from = from.max(1);
    let to = to.min(len as i64);
    if from > to || len == 0 {
        // A position past the end names nothing — the hole fills with
        // nothing rather than failing, so `{5}` on a three-word name is
        // empty, not an error.
        return Some((0, 0));
    }
    Some((from as usize - 1, to as usize))
}

/// The name split where a person would: at spaces, underscores, dashes and
/// dots. Empty pieces — two underscores in a row — do not count.
fn words(stem: &str) -> Vec<&str> {
    stem.split([' ', '_', '-', '.'])
        .filter(|w| !w.is_empty())
        .collect()
}

/// Stem and extension, the way the browser reads them: a leading dot is part
/// of the name, not an extension.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot + 1..]),
        _ => (name, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_number_counts_from_one_and_the_extension_is_kept() {
        assert_eq!(expand("Holiday {n}", "IMG_001.jpg", 1), "Holiday 1.jpg");
        assert_eq!(expand("Holiday {n}", "IMG_002.jpg", 2), "Holiday 2.jpg");
        assert_eq!(expand("Holiday {n}", "notes", 3), "Holiday 3");
    }

    #[test]
    fn padding_and_a_start_can_be_asked_for_in_either_order() {
        assert_eq!(expand("{n:3}", "a.txt", 7), "007.txt");
        assert_eq!(expand("{n@10}", "a.txt", 1), "10.txt");
        assert_eq!(expand("{n:3@10}", "a.txt", 2), "011.txt");
        assert_eq!(expand("{n@10:3}", "a.txt", 2), "011.txt");
    }

    #[test]
    fn the_original_name_and_extension_are_holes_too() {
        assert_eq!(expand("{name}-small", "photo.jpg", 1), "photo-small.jpg");
        assert_eq!(expand("{name}.png", "photo.jpg", 1), "photo.png");
        assert_eq!(expand("{name}.{ext}.bak", "photo.jpg", 1), "photo.jpg.bak");
        assert_eq!(expand("{ext}-{name}", "photo.jpg", 1), "jpg-photo");
    }

    #[test]
    fn words_are_picked_by_number() {
        let original = "IMG_2024_Paris.jpg";
        assert_eq!(expand("{3} {n}", original, 4), "Paris 4.jpg");
        assert_eq!(expand("{2..}", original, 1), "2024 Paris.jpg");
        assert_eq!(expand("{1..2}", original, 1), "IMG 2024.jpg");
        assert_eq!(expand("{-1}", original, 1), "Paris.jpg");
        assert_eq!(expand("{..-2}", original, 1), "IMG 2024.jpg");
        // A word past the end is nothing — and a name that is nothing gets no
        // extension bolted onto it either.
        assert_eq!(expand("{5}", original, 1), "");
    }

    #[test]
    fn characters_are_picked_by_position() {
        assert_eq!(expand("{name:1..3}", "photograph.jpg", 1), "pho.jpg");
        assert_eq!(expand("{name:5..}", "IMG_2024.jpg", 1), "2024.jpg");
        assert_eq!(expand("{name:..-5}", "photo_old.jpg", 1), "photo.jpg");
        assert_eq!(expand("{name:..-4}", "photo_old.jpg", 1), "photo_.jpg");
        assert_eq!(expand("{name:-3..}", "photo_old.jpg", 1), "old.jpg");
    }

    #[test]
    fn what_is_not_a_hole_is_left_as_typed() {
        assert_eq!(expand("{na", "a.txt", 1), "{na.txt");
        assert_eq!(expand("{what}", "a.txt", 1), "{what}.txt");
        assert_eq!(expand("a { b", "a.txt", 1), "a { b.txt");
        assert_eq!(expand("", "a.txt", 1), "");
    }

    #[test]
    fn a_leading_dot_is_a_name_not_an_extension() {
        assert_eq!(expand("{name}", ".bashrc", 1), ".bashrc");
        assert_eq!(expand("{n}", ".bashrc", 1), "1");
    }

    #[test]
    fn the_plan_tells_same_changed_and_taken_apart() {
        let out = plan(
            &names(&["a.jpg", "b.jpg", "c.jpg"]),
            "x{n}",
            &names(&["x2.jpg", "other.txt"]),
        );
        assert_eq!(out[0].new, "x1.jpg");
        assert_eq!(out[0].state, State::Changed);
        assert_eq!(out[1].state, State::Conflict, "x2.jpg is already there");
        assert_eq!(out[2].state, State::Changed);

        let out = plan(&names(&["a.jpg", "b.jpg"]), "same", &[]);
        assert!(out.iter().all(|r| r.state == State::Conflict));

        let out = plan(&names(&["a.jpg"]), "{name}", &names(&["a.jpg"]));
        assert_eq!(out[0].state, State::Same, "keeping a name takes nothing");

        let out = plan(&names(&["a.jpg", "b.jpg"]), "b", &[]);
        assert_eq!(out[0].state, State::Conflict, "b.jpg belongs to the batch");
        assert_eq!(out[1].state, State::Same);
    }

    #[test]
    fn a_chain_is_fine_because_the_host_renames_through_temporary_names() {
        // 1→2 and 2→3: a plain rename in that order would overwrite 2 before
        // it moved, so the host goes through temporary names. Here that means
        // a name another file is *leaving* is not taken.
        let out = plan(&names(&["1.txt", "2.txt"]), "{n@2}", &[]);
        assert_eq!(out[0].new, "2.txt");
        assert_eq!(out[0].state, State::Changed);
        assert_eq!(out[1].new, "3.txt");
        assert_eq!(out[1].state, State::Changed);
    }

    #[test]
    fn an_empty_or_slashed_name_is_invalid() {
        let out = plan(&names(&["a.jpg"]), "{5}", &[]);
        assert_eq!(out[0].new, "");
        assert_eq!(out[0].state, State::Invalid);
        let out = plan(&names(&["a"]), "x/y", &[]);
        assert_eq!(out[0].state, State::Invalid);
    }
}

// ---------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------

use std::path::PathBuf;

use crate::command::{
    ArgKind, ArgSpec, Command, CommandProvider, Effect, Group, Preview, PreviewRow, Request,
    Situation,
};
use crate::model::Change;

/// The provider's namespace, and so the prefix on its one command's id.
pub const NAMESPACE: &str = "rename";
/// The command's id in full.
pub const RENAME_MANY: &str = "rename:many";

/// "Rename N Items", as a provider of its own.
///
/// Built through the command seam rather than into the browser, on purpose:
/// this is the shape a script would take — the selection in, one line of text
/// as the argument, a dry run to show while it is typed, and what changed
/// handed back for undo — and having a built-in take exactly that shape is
/// what keeps the seam honest before anything lives on the far side of it.
/// Nothing here reaches into the window; it only ever sees a [`Situation`].
pub struct RenameProvider;

impl RenameProvider {
    /// The files the command works on, in the order they are numbered — the
    /// order they were selected in the listing, which is the order the person
    /// can see.
    fn names(situation: &Situation) -> Vec<String> {
        situation
            .selection
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect()
    }

    fn plan_for(request: &Request, situation: &Situation) -> Vec<Rename> {
        let pattern = request.arg.as_deref().unwrap_or_default();
        plan(&Self::names(situation), pattern, &situation.siblings)
    }

    /// The summary line: what the plan comes to, and the first thing wrong
    /// with it if anything is.
    fn summary(plan: &[Rename]) -> String {
        let conflicts = plan.iter().filter(|r| r.state == State::Conflict).count();
        let invalid = plan.iter().filter(|r| r.state == State::Invalid).count();
        let changed = plan.iter().filter(|r| r.state == State::Changed).count();
        if invalid > 0 {
            otto_kit::t_owned!("files-rename-invalid", count = invalid as i64)
        } else if conflicts > 0 {
            otto_kit::t_owned!("files-rename-conflicts", count = conflicts as i64)
        } else if changed == 0 {
            otto_kit::t_owned!("files-rename-preview-unchanged")
        } else {
            otto_kit::t_owned!(
                "files-rename-preview",
                count = changed as i64,
                total = plan.len() as i64
            )
        }
    }
}

impl CommandProvider for RenameProvider {
    fn namespace(&self) -> &'static str {
        NAMESPACE
    }

    fn commands(&self, situation: &Situation) -> Vec<Command> {
        if situation.trash || situation.selection.is_empty() {
            return Vec::new();
        }
        let count = situation.selection.len();
        let title = if count > 1 {
            otto_kit::t_owned!("files-rename-many", count = count as i64)
        } else {
            otto_kit::t_owned!("files-rename-pattern")
        };
        vec![Command::new(RENAME_MANY, title, Group::File)
            .with_keywords(["batch", "pattern", "number", "sequence", "rename"])
            .with_arg(
                ArgSpec::new(
                    otto_kit::t_owned!("files-command-rename-many-prompt"),
                    otto_kit::t_owned!("files-command-arg-pattern"),
                    ArgKind::Text,
                )
                .with_initial("{name}")
                .with_placeholder("Holiday {n}")
                // A dry run, not a rename: shown, and nothing moves until
                // Return.
                .previewed(),
            )]
    }

    fn preview(&self, request: &Request, situation: &Situation) -> Option<Preview> {
        if request.id != RENAME_MANY {
            return None;
        }
        let plan = Self::plan_for(request, situation);
        if plan.is_empty() {
            return None;
        }
        Some(Preview {
            note: Some(Self::summary(&plan)),
            rows: plan
                .into_iter()
                .map(|rename| PreviewRow {
                    conflict: matches!(rename.state, State::Conflict | State::Invalid),
                    from: rename.old,
                    to: rename.new,
                })
                .collect(),
        })
    }

    fn run(&mut self, request: &Request, situation: &Situation) -> Result<Effect, String> {
        if request.id != RENAME_MANY {
            return Err(format!("{} is not mine", request.id));
        }
        let plan = Self::plan_for(request, situation);
        if plan
            .iter()
            .any(|r| matches!(r.state, State::Conflict | State::Invalid))
        {
            return Err(Self::summary(&plan));
        }
        let moves: Vec<(PathBuf, PathBuf)> = situation
            .selection
            .iter()
            .zip(&plan)
            .filter(|(_, rename)| rename.state == State::Changed)
            .map(|(path, rename)| (path.clone(), path.with_file_name(&rename.new)))
            .collect();
        if moves.is_empty() {
            return Err(otto_kit::t_owned!("files-rename-preview-unchanged"));
        }
        let changes = rename_all(&moves)?;
        Ok(Effect {
            status: Some(otto_kit::t_owned!(
                "files-renamed-count",
                count = changes.len() as i64
            )),
            undo_label: Some(otto_kit::t!("files-undo-rename")),
            changes,
            reload: true,
        })
    }
}

/// Carry the moves out, through temporary names.
///
/// Two passes rather than one, because a batch may hand one file the name
/// another is about to give up — `1 → 2, 2 → 3` — and renaming in place, in
/// either order, would overwrite. Everything is first moved aside under a
/// name nobody has, then to where it is going; a failure in the second pass
/// puts back what has moved so far, so the directory is never left half way.
fn rename_all(moves: &[(PathBuf, PathBuf)]) -> Result<Vec<Change>, String> {
    let tag = std::process::id();
    let aside: Vec<PathBuf> = moves
        .iter()
        .enumerate()
        .map(|(i, (from, _))| from.with_file_name(format!(".otto-rename-{tag}-{i}")))
        .collect();

    let mut done = Vec::new();
    for ((from, _), temp) in moves.iter().zip(&aside) {
        if let Err(err) = std::fs::rename(from, temp) {
            undo_moves(&done);
            return Err(otto_kit::t_owned!(
                "files-rename-failed",
                error = err.to_string()
            ));
        }
        done.push((from.clone(), temp.clone()));
    }
    let mut landed = Vec::new();
    for ((from, to), temp) in moves.iter().zip(&aside) {
        if let Err(err) = std::fs::rename(temp, to) {
            undo_moves(&landed);
            undo_moves(&done);
            return Err(otto_kit::t_owned!(
                "files-rename-failed",
                error = err.to_string()
            ));
        }
        landed.push((temp.clone(), to.clone()));
        let _ = from;
    }
    Ok(moves
        .iter()
        .map(|(from, to)| Change::Moved {
            from: from.clone(),
            to: to.clone(),
        })
        .collect())
}

/// Best effort: put a run of `(from, to)` moves back, last first.
fn undo_moves(moves: &[(PathBuf, PathBuf)]) {
    for (from, to) in moves.iter().rev() {
        let _ = std::fs::rename(to, from);
    }
}

#[cfg(test)]
mod provider_tests {
    use std::path::Path;

    use super::*;

    /// A directory of our own under the system temp dir, removed on drop.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "otto-files-rename-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn situation(dir: &Path, names: &[&str], siblings: &[&str]) -> Situation {
        Situation {
            path: dir.to_path_buf(),
            selection: names.iter().map(|n| dir.join(n)).collect(),
            siblings: siblings.iter().map(|s| s.to_string()).collect(),
            has_entries: true,
            ..Situation::default()
        }
    }

    #[test]
    fn it_is_offered_for_a_selection_and_never_in_the_trash() {
        let dir = Path::new("/tmp/x");
        assert!(RenameProvider
            .commands(&situation(dir, &[], &[]))
            .is_empty());
        let one = RenameProvider.commands(&situation(dir, &["a.txt"], &["a.txt"]));
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, RENAME_MANY);
        assert!(one[0].arg.as_ref().is_some_and(|arg| arg.preview));
        let mut trash = situation(dir, &["a.txt"], &[]);
        trash.trash = true;
        assert!(RenameProvider.commands(&trash).is_empty());
    }

    #[test]
    fn the_preview_is_the_plan_with_a_summary() {
        let dir = Path::new("/tmp/x");
        let s = situation(
            dir,
            &["IMG_1.jpg", "IMG_2.jpg"],
            &["IMG_1.jpg", "IMG_2.jpg", "x2.jpg"],
        );
        let request = Request::new(RENAME_MANY, Some("x{n}".into()));
        let preview = RenameProvider.preview(&request, &s).unwrap();
        assert_eq!(preview.rows[0].to, "x1.jpg");
        assert!(!preview.rows[0].conflict);
        assert!(preview.rows[1].conflict, "x2.jpg is already there");
        assert_eq!(
            preview.note.as_deref(),
            Some(otto_kit::t_owned!("files-rename-conflicts", count = 1).as_str())
        );
    }

    #[test]
    fn running_renames_through_temporary_names_and_reports_the_moves() {
        let dir = TempDir::new();
        for name in ["1.txt", "2.txt"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        let s = situation(dir.path(), &["1.txt", "2.txt"], &["1.txt", "2.txt"]);
        // 1 → 2 and 2 → 3: the chain a naive loop would break.
        let request = Request::new(RENAME_MANY, Some("{n@2}".into()));
        let effect = RenameProvider.run(&request, &s).unwrap();
        assert_eq!(effect.changes.len(), 2);
        assert!(effect.reload);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("2.txt")).unwrap(),
            "1.txt"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("3.txt")).unwrap(),
            "2.txt"
        );
        assert!(!dir.path().join("1.txt").exists());
        assert!(std::fs::read_dir(dir.path()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".otto-rename")));
    }

    #[test]
    fn a_conflict_refuses_and_touches_nothing() {
        let dir = TempDir::new();
        for name in ["a.txt", "b.txt", "x1.txt"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        let s = situation(
            dir.path(),
            &["a.txt", "b.txt"],
            &["a.txt", "b.txt", "x1.txt"],
        );
        let request = Request::new(RENAME_MANY, Some("x{n}".into()));
        assert!(RenameProvider.run(&request, &s).is_err());
        assert!(dir.path().join("a.txt").exists());
        assert!(dir.path().join("b.txt").exists());
    }
}
