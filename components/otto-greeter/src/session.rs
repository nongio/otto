//! Session discovery.
//!
//! greetd needs an argv to exec once authentication succeeds. Desktop sessions
//! advertise themselves as `.desktop` files under `wayland-sessions`, the same
//! way GDM and SDDM discover them.

use std::path::Path;

/// A session the greeter can launch.
#[derive(Debug, Clone)]
pub struct Session {
    /// Human-readable name, shown in the greeter.
    pub name: String,
    /// argv handed to greetd's `start_session`.
    pub command: Vec<String>,
}

impl Session {
    /// Last-resort session used when nothing is installed — keeps a
    /// freshly-built Otto testable before it is installed system-wide.
    fn fallback() -> Self {
        Self {
            name: "Otto".to_string(),
            command: vec!["otto".to_string()],
        }
    }
}

const SESSION_DIRS: &[&str] = &[
    "/usr/share/wayland-sessions",
    "/usr/local/share/wayland-sessions",
];

/// Discover installed Wayland sessions, sorted by name.
///
/// `$OTTO_GREETER_SESSION` overrides discovery entirely and is parsed as a
/// shell-style argv — useful for testing an uninstalled build.
pub fn discover() -> Vec<Session> {
    if let Ok(override_cmd) = std::env::var("OTTO_GREETER_SESSION") {
        let command: Vec<String> = override_cmd
            .split_whitespace()
            .map(str::to_string)
            .collect();
        if !command.is_empty() {
            return vec![Session {
                name: command[0].clone(),
                command,
            }];
        }
    }

    let mut sessions: Vec<Session> = SESSION_DIRS
        .iter()
        .filter_map(|dir| std::fs::read_dir(Path::new(dir)).ok())
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("desktop"))
        .filter_map(|entry| parse_desktop_entry(&entry.path()))
        .collect();

    sessions.sort_by(|a, b| a.name.cmp(&b.name));
    sessions.dedup_by(|a, b| a.name == b.name);

    if sessions.is_empty() {
        tracing::warn!("No sessions found in {SESSION_DIRS:?}, falling back to `otto`");
        sessions.push(Session::fallback());
    }
    sessions
}

/// Which session should be selected when the greeter starts.
///
/// `$OTTO_GREETER_DEFAULT_SESSION` names it — set it in greetd's config rather
/// than hardcoding a preference here, since which session should be offered
/// first is a deployment decision. Falls back to the first session so the
/// greeter is always usable.
pub fn default_index(sessions: &[Session]) -> usize {
    let Ok(preferred) = std::env::var("OTTO_GREETER_DEFAULT_SESSION") else {
        return 0;
    };
    let preferred = preferred.trim();
    if preferred.is_empty() {
        return 0;
    }

    // Exact name first, then a case-insensitive contains, so a short hint like
    // "current" picks "Otto (current build)" without naming it in full.
    let exact = sessions
        .iter()
        .position(|s| s.name.eq_ignore_ascii_case(preferred));
    let fuzzy = || {
        let needle = preferred.to_lowercase();
        sessions
            .iter()
            .position(|s| s.name.to_lowercase().contains(&needle))
    };

    match exact.or_else(fuzzy) {
        Some(index) => index,
        None => {
            tracing::warn!(
                preferred,
                available = ?sessions.iter().map(|s| &s.name).collect::<Vec<_>>(),
                "Preferred session not found, using the first"
            );
            0
        }
    }
}

/// Minimal `.desktop` parse: `Name` and `Exec` from the `[Desktop Entry]`
/// group, ignoring `Hidden` entries. Field codes (`%f`, `%U`, …) are stripped
/// since a session takes no arguments.
fn parse_desktop_entry(path: &Path) -> Option<Session> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut in_entry = false;
    let mut name = None;
    let mut exec = None;
    let mut hidden = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        // Only the unlocalised keys — the greeter has no locale of its own.
        match key.trim() {
            "Name" => name = Some(value.trim().to_string()),
            "Exec" => exec = Some(value.trim().to_string()),
            "Hidden" | "NoDisplay" => hidden |= value.trim() == "true",
            _ => {}
        }
    }

    if hidden {
        return None;
    }

    let exec = exec?;
    let command: Vec<String> = exec
        .split_whitespace()
        .filter(|token| !(token.len() == 2 && token.starts_with('%')))
        .map(str::to_string)
        .collect();
    if command.is_empty() {
        return None;
    }

    Some(Session {
        name: name.unwrap_or_else(|| command[0].clone()),
        command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Tests run in parallel, so each needs its own file.
    static NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn entry(content: &str) -> Option<Session> {
        let dir = std::env::temp_dir().join(format!("otto-greeter-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("session-{id}.desktop"));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let parsed = parse_desktop_entry(&path);
        let _ = std::fs::remove_file(&path);
        parsed
    }

    #[test]
    fn reads_name_and_exec() {
        let session = entry(
            "[Desktop Entry]\n\
             Name=Otto\n\
             Comment=A Wayland compositor\n\
             Exec=/usr/bin/otto\n\
             Type=Application\n",
        )
        .expect("entry should parse");
        assert_eq!(session.name, "Otto");
        assert_eq!(session.command, vec!["/usr/bin/otto"]);
    }

    #[test]
    fn strips_field_codes_but_keeps_real_arguments() {
        let session = entry(
            "[Desktop Entry]\n\
             Name=Sway\n\
             Exec=sway --unsupported-gpu %U\n",
        )
        .expect("entry should parse");
        assert_eq!(session.command, vec!["sway", "--unsupported-gpu"]);
    }

    #[test]
    fn ignores_keys_outside_the_desktop_entry_group() {
        // A trailing action group must not overwrite the session's own Exec.
        let session = entry(
            "[Desktop Entry]\n\
             Name=Otto\n\
             Exec=otto\n\
             \n\
             [Desktop Action Debug]\n\
             Name=Otto (debug)\n\
             Exec=otto --debug\n",
        )
        .expect("entry should parse");
        assert_eq!(session.name, "Otto");
        assert_eq!(session.command, vec!["otto"]);
    }

    #[test]
    fn skips_hidden_entries() {
        assert!(entry(
            "[Desktop Entry]\n\
             Name=Hidden\n\
             Exec=hidden\n\
             Hidden=true\n"
        )
        .is_none());

        assert!(entry(
            "[Desktop Entry]\n\
             Name=NoDisplay\n\
             Exec=nodisplay\n\
             NoDisplay=true\n"
        )
        .is_none());
    }

    #[test]
    fn rejects_entries_without_exec() {
        assert!(entry("[Desktop Entry]\nName=Broken\n").is_none());
    }

    fn sessions(names: &[&str]) -> Vec<Session> {
        names
            .iter()
            .map(|n| Session {
                name: n.to_string(),
                command: vec![n.to_lowercase()],
            })
            .collect()
    }

    // These mutate a process-wide env var, so they run as one test rather than
    // racing each other under the parallel harness.
    #[test]
    fn default_session_selection() {
        let list = sessions(&["GNOME", "Otto", "Otto (current build)", "Plasma"]);

        // SAFETY: single-threaded within this test; no other test reads this var.
        unsafe { std::env::remove_var("OTTO_GREETER_DEFAULT_SESSION") };
        assert_eq!(default_index(&list), 0, "unset -> first session");

        unsafe { std::env::set_var("OTTO_GREETER_DEFAULT_SESSION", "Otto (current build)") };
        assert_eq!(default_index(&list), 2, "exact name wins");

        // An exact match must beat a substring: "Otto" is contained in
        // "Otto (current build)" too, but names the plain entry.
        unsafe { std::env::set_var("OTTO_GREETER_DEFAULT_SESSION", "Otto") };
        assert_eq!(default_index(&list), 1, "exact match beats substring");

        unsafe { std::env::set_var("OTTO_GREETER_DEFAULT_SESSION", "current") };
        assert_eq!(default_index(&list), 2, "substring match");

        unsafe { std::env::set_var("OTTO_GREETER_DEFAULT_SESSION", "gnome") };
        assert_eq!(default_index(&list), 0, "match is case-insensitive");

        unsafe { std::env::set_var("OTTO_GREETER_DEFAULT_SESSION", "Nonexistent") };
        assert_eq!(
            default_index(&list),
            0,
            "unknown name -> first, not a panic"
        );

        unsafe { std::env::set_var("OTTO_GREETER_DEFAULT_SESSION", "   ") };
        assert_eq!(default_index(&list), 0, "blank -> first");

        unsafe { std::env::remove_var("OTTO_GREETER_DEFAULT_SESSION") };
    }

    #[test]
    fn falls_back_to_the_command_when_name_is_missing() {
        let session = entry("[Desktop Entry]\nExec=otto\n").expect("entry should parse");
        assert_eq!(session.name, "otto");
    }
}
