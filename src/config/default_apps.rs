use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use freedesktop_desktop_entry::{self as desktop_entry, DesktopEntry, ExecError};
use once_cell::sync::Lazy;

use super::Config;

static MIMEAPPS_CACHE: Lazy<MimeAppsCache> = Lazy::new(MimeAppsCache::load);

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

fn resolve_role(role: &str, config: &Config) -> Option<(String, Vec<String>)> {
    if role.ends_with(".desktop") {
        return desktop_id_to_command(role, &config.locales);
    }

    let mut attempts = Vec::new();

    match role {
        "browser" => {
            attempts.push("x-scheme-handler/https".to_string());
            attempts.push("x-scheme-handler/http".to_string());
            attempts.push("text/html".to_string());
        }
        "file_manager" | "files" => {
            attempts.push("inode/directory".to_string());
        }
        "terminal" | "shell" => {
            attempts.push("x-scheme-handler/terminal".to_string());
            attempts.push("application/x-terminal".to_string());
        }
        other if other.contains('/') => attempts.push(other.to_string()),
        other => attempts.push(format!("x-scheme-handler/{}", other)),
    }

    for mime in attempts {
        if let Some(desktops) = MIMEAPPS_CACHE.query(&mime) {
            for desktop in desktops {
                if let Some(command) = desktop_id_to_command(&desktop, &config.locales) {
                    return Some(command);
                }
            }
        }
    }

    None
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

struct MimeAppsCache {
    defaults: HashMap<String, Vec<String>>,
    raw: Mutex<HashMap<String, Option<Vec<String>>>>,
}

impl MimeAppsCache {
    fn load() -> Self {
        let defaults = load_mimeapps_defaults();
        Self {
            defaults,
            raw: Mutex::new(HashMap::new()),
        }
    }

    fn query(&self, mime: &str) -> Option<Vec<String>> {
        if let Ok(cache) = self.raw.lock() {
            if let Some(value) = cache.get(mime) {
                return value.clone();
            }
        }

        let result = self.defaults.get(mime).cloned();
        if let Ok(mut cache) = self.raw.lock() {
            cache.insert(mime.to_string(), result.clone());
        }
        result
    }
}

fn load_mimeapps_defaults() -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    for path in mimeapps_paths() {
        parse_mimeapps(&path, &mut map);
    }
    map
}

fn mimeapps_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(config_home) = xdg_config_home() {
        paths.push(config_home.join("mimeapps.list"));
        paths.push(config_home.join("applications/mimeapps.list"));
    }

    if let Some(data_home) = xdg_data_home() {
        paths.push(data_home.join("applications/mimeapps.list"));
        paths.push(data_home.join("applications/defaults.list"));
    }

    for dir in xdg_config_dirs() {
        paths.push(dir.join("mimeapps.list"));
    }

    for dir in xdg_data_dirs() {
        paths.push(dir.join("applications/mimeapps.list"));
        paths.push(dir.join("applications/defaults.list"));
    }

    paths
}

fn parse_mimeapps(path: &Path, map: &mut HashMap<String, Vec<String>>) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };

    let mut in_default_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_default_section = trimmed.eq_ignore_ascii_case("[Default Applications]");
            continue;
        }

        if !in_default_section {
            continue;
        }

        let Some((mime, handlers)) = trimmed.split_once('=') else {
            continue;
        };

        if map.contains_key(mime) {
            continue;
        }

        let handlers = handlers
            .split(';')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| entry.to_string())
            .collect::<Vec<_>>();

        if !handlers.is_empty() {
            map.insert(mime.to_string(), handlers);
        }
    }
}

fn xdg_config_home() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
}

fn xdg_data_home() -> Option<PathBuf> {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")))
}

fn xdg_config_dirs() -> Vec<PathBuf> {
    env::var("XDG_CONFIG_DIRS")
        .map(|dirs| dirs.split(':').map(PathBuf::from).collect())
        .unwrap_or_else(|_| vec![PathBuf::from("/etc/xdg")])
}

fn xdg_data_dirs() -> Vec<PathBuf> {
    env::var("XDG_DATA_DIRS")
        .map(|dirs| dirs.split(':').map(PathBuf::from).collect())
        .unwrap_or_else(|_| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        })
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
