//! Resolving `open_default_app` shortcut roles to a command.
//!
//! The association lookup is `otto_kit::mime_apps`, the same one otto-files
//! and the launcher use, so a shortcut opens what a double-click would: it
//! honours `otto-mimeapps.list`, `[Added Associations]` and
//! `[Removed Associations]`, and falls back to installed entries that declare
//! the type.

use std::path::Path;

use freedesktop_desktop_entry::{self as desktop_entry, DesktopEntry, ExecError};
use once_cell::sync::Lazy;
use otto_kit::mime_apps::Associations;

use super::Config;

/// Read once, on the first shortcut that needs it: loading walks every
/// application directory, which is too slow to repeat on each key press.
static ASSOCIATIONS: Lazy<Associations> = Lazy::new(Associations::load);

pub fn resolve(
    role: &str,
    fallback: Option<&str>,
    config: &Config,
) -> Option<(String, Vec<String>)> {
    if let Some(result) = resolve_role(role, config) {
        return Some(result);
    }

    if let Some(fallback) = fallback {
        if let Some(result) = resolve_spec(fallback, config) {
            return Some(result);
        }
    }

    None
}

/// The MIME types (or scheme handlers) a role stands for, most specific first.
fn role_mime_types(role: &str) -> Vec<String> {
    match role {
        "browser" => vec![
            "x-scheme-handler/https".to_string(),
            "x-scheme-handler/http".to_string(),
            "text/html".to_string(),
        ],
        "file_manager" | "files" => vec!["inode/directory".to_string()],
        "terminal" | "shell" => vec![
            "x-scheme-handler/terminal".to_string(),
            "application/x-terminal".to_string(),
        ],
        other if other.contains('/') => vec![other.to_string()],
        other => vec![format!("x-scheme-handler/{other}")],
    }
}

fn resolve_role(role: &str, config: &Config) -> Option<(String, Vec<String>)> {
    if role.ends_with(".desktop") {
        return desktop_id_to_command(role, &config.locales);
    }

    let app = ASSOCIATIONS.default_for(&role_mime_types(role))?;
    entry_to_command(&app.entry_path, &config.locales)
}

fn resolve_spec(spec: &str, config: &Config) -> Option<(String, Vec<String>)> {
    if spec.ends_with(".desktop") {
        if let Some(command) = desktop_id_to_command(spec, &config.locales) {
            return Some(command);
        }
    }

    let mut parts = spec
        .split_whitespace()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return None;
    }
    let cmd = parts.remove(0);
    Some((cmd, parts))
}

fn desktop_id_to_command(desktop_id: &str, locales: &[String]) -> Option<(String, Vec<String>)> {
    let normalized = if desktop_id.ends_with(".desktop") {
        desktop_id.to_string()
    } else {
        format!("{desktop_id}.desktop")
    };

    let path = desktop_entry::Iter::new(desktop_entry::default_paths()).find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(&normalized))
            .unwrap_or(false)
    })?;

    entry_to_command(&path, locales)
}

/// The command line a desktop entry's `Exec=` runs, field codes dropped.
fn entry_to_command(path: &Path, locales: &[String]) -> Option<(String, Vec<String>)> {
    let locale_refs: Vec<&str> = locales.iter().map(|s| s.as_str()).collect();
    let entry = DesktopEntry::from_path(path, Some(&locale_refs)).ok()?;
    match entry.parse_exec() {
        Ok(mut args) => {
            if args.is_empty() {
                return None;
            }
            let cmd = args.remove(0);
            Some((cmd, args))
        }
        Err(ExecError::ExecFieldNotFound) | Err(ExecError::ExecFieldIsEmpty) => None,
        Err(ExecError::WrongFormat(_)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_spec_command() {
        let config = Config::default();
        let (cmd, args) = resolve_spec("echo hello world", &config).expect("resolve spec");
        assert_eq!(cmd, "echo");
        assert_eq!(args, vec!["hello".to_string(), "world".to_string()]);
    }
}
