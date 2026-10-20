//! The `[Desktop Entry]` group of a desktop entry file, read and edited as
//! text.
//!
//! A desktop entry is a keyfile: `[Group]` headers, then `Key=Value` lines,
//! with `#` comments and blank lines in between. What Otto reads from one is a
//! handful of keys of its main group (a session's `Exec`, an autostart entry's
//! `Hidden`, an application's `Name`), so this reads that group only. Other
//! groups, `[Desktop Action …]` among them, have keys of the same names that
//! mean something else, and are skipped.
//!
//! Values come back as written: escapes such as `\s` are left in, and `Exec`
//! quoting is the caller's to undo. A key's localised forms (`Name[it]`) are
//! keys of their own, looked up by that full name.
//!
//! Editing keeps every other line as it was, comments and unknown keys
//! included, so a file someone wrote by hand survives Otto changing one key.

// Rust guideline compliant 2026-02-21

/// The main group's header line.
const GROUP: &str = "[Desktop Entry]";

/// `key`'s value in the `[Desktop Entry]` group of `text`, trimmed.
///
/// The spec forbids a key twice in a group; if one is, the first is taken.
pub fn value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    entries(text).find_map(|(name, value)| (name == key).then_some(value))
}

/// Every `key`/`value` of the `[Desktop Entry]` group of `text`, in the
/// file's order, both trimmed. Comments, blank lines and lines without `=`
/// are skipped.
pub fn entries(text: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut in_group = false;
    text.lines().filter_map(move |line| {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == GROUP;
            return None;
        }
        if !in_group || line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        Some((key.trim(), value.trim()))
    })
}

/// `text` with `key` set to `value` in its `[Desktop Entry]` group.
///
/// The key's line is rewritten where it is. A key the group lacks goes after
/// the group's last key, and a file without the group gets one at its end, so
/// `with_value("", …)` starts a new entry. The result ends with a newline.
pub fn with_value(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key}={value}");
    let mut lines: Vec<&str> = text.lines().collect();
    let mut in_group = false;
    let mut group_end = None;
    for (index, existing) in lines.iter_mut().enumerate() {
        let trimmed = existing.trim();
        if trimmed.starts_with('[') {
            in_group = trimmed == GROUP;
            if in_group {
                group_end = Some(index + 1);
            }
            continue;
        }
        if !in_group {
            continue;
        }
        if !trimmed.starts_with('#')
            && trimmed
                .split_once('=')
                .is_some_and(|(name, _)| name.trim() == key)
        {
            *existing = &line;
            return lines.join("\n") + "\n";
        }
        if !trimmed.is_empty() {
            group_end = Some(index + 1);
        }
    }
    match group_end {
        Some(at) => lines.insert(at, &line),
        None => lines.extend([GROUP, &line]),
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "# A session\n\
                         [Desktop Entry]\n\
                         Name = Otto\n\
                         Name[it]=Otto (it)\n\
                         #Exec=commented-out\n\
                         Exec=otto --session\n\
                         \n\
                         [Desktop Action new]\n\
                         Name=New Window\n\
                         Exec=otto --new\n\
                         TryExec=otto\n";

    #[test]
    fn only_the_main_group_is_read() {
        assert_eq!(value(ENTRY, "Name"), Some("Otto"));
        assert_eq!(value(ENTRY, "Name[it]"), Some("Otto (it)"));
        assert_eq!(value(ENTRY, "Exec"), Some("otto --session"));
        // An action's key, and a key in no group at all, are not the entry's.
        assert_eq!(value(ENTRY, "TryExec"), None);
        assert_eq!(value("Exec=loose\n[Other]\nExec=x\n", "Exec"), None);
        assert_eq!(
            entries(ENTRY).collect::<Vec<_>>(),
            [
                ("Name", "Otto"),
                ("Name[it]", "Otto (it)"),
                ("Exec", "otto --session")
            ]
        );
    }

    #[test]
    fn a_value_keeps_its_equals_signs_and_escapes() {
        let text = "[Desktop Entry]\nExec=env A=b prog\\sname %U\nIcon=\n";
        assert_eq!(value(text, "Exec"), Some("env A=b prog\\sname %U"));
        assert_eq!(value(text, "Icon"), Some(""));
    }

    #[test]
    fn setting_a_key_rewrites_it_in_place() {
        let text = "[Desktop Entry]\n# keep me\nHidden=true\nExec=x\n";
        assert_eq!(
            with_value(text, "Hidden", "false"),
            "[Desktop Entry]\n# keep me\nHidden=false\nExec=x\n"
        );
    }

    #[test]
    fn a_missing_key_goes_after_the_groups_last_key() {
        let text = "[Desktop Entry]\nExec=x\n\n[Desktop Action a]\nExec=y\n";
        assert_eq!(
            with_value(text, "Hidden", "true"),
            "[Desktop Entry]\nExec=x\nHidden=true\n\n[Desktop Action a]\nExec=y\n"
        );
        // A key of another group is not the one being set.
        let text = "[Desktop Action a]\nHidden=false\n";
        assert_eq!(
            with_value(text, "Hidden", "true"),
            "[Desktop Action a]\nHidden=false\n[Desktop Entry]\nHidden=true\n"
        );
    }

    #[test]
    fn an_empty_text_starts_an_entry() {
        let text = with_value("", "Type", "Application");
        let text = with_value(&text, "Exec", "otto-dictate");
        assert_eq!(
            text,
            "[Desktop Entry]\nType=Application\nExec=otto-dictate\n"
        );
    }
}
