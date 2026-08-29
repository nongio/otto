//! XDG Desktop Entry lookup for resolving app_id → display name, icon, etc.
//!
//! Scans standard XDG paths for `.desktop` files and caches results.
//! This is the lightweight metadata layer — no image loading, just strings.

use std::collections::HashMap;
use std::sync::RwLock;

use freedesktop_desktop_entry::DesktopEntry;

/// Metadata from a `.desktop` file.
#[derive(Clone, Debug)]
pub struct AppInfo {
    /// Localized display name (from `Name=`).
    pub name: String,
    /// Icon name (from `Icon=`), suitable for theme lookup.
    pub icon_name: Option<String>,
    /// Exec command line (from `Exec=`).
    pub exec: Option<String>,
    /// The desktop file ID (filename without `.desktop`).
    pub desktop_file_id: Option<String>,
    /// The raw app_id used to look this up.
    pub app_id: String,
    /// Categories (from `Categories=`).
    pub categories: Vec<String>,
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

static CACHE: std::sync::LazyLock<RwLock<HashMap<String, Option<AppInfo>>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// Separate cache for [`lookup_app_by_binary`]: its keys are program names,
/// which must not answer a plain `app_id` lookup.
static BINARY_CACHE: std::sync::LazyLock<RwLock<HashMap<String, Option<AppInfo>>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// Look up app info for the given `app_id`.
///
/// Searches XDG desktop entry paths for a `.desktop` file whose filename
/// matches `app_id` (case-insensitive). Results are cached.
///
/// Returns `None` if no matching desktop file is found.
pub fn lookup_app(app_id: &str) -> Option<AppInfo> {
    // Check cache first
    if let Some(cached) = CACHE.read().unwrap().get(app_id) {
        return cached.clone();
    }

    let result = load_app_info(app_id);

    // Cache the result (including None for negative caching)
    CACHE
        .write()
        .unwrap()
        .insert(app_id.to_string(), result.clone());

    result
}

/// Clear the cache (useful after desktop file changes).
pub fn clear_cache() {
    CACHE.write().unwrap().clear();
}

/// Format an app_id as a human-readable display name.
///
/// If a desktop entry is found, returns its localized `Name=`.
/// Otherwise, strips reverse-domain prefixes and capitalizes.
pub fn display_name_for_app(app_id: &str) -> String {
    if app_id.is_empty() {
        return "Otto".to_string();
    }

    if let Some(info) = lookup_app(app_id) {
        return info.name;
    }

    // Fallback: strip reverse-domain prefix, capitalize
    let short = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut chars = short.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
        None => app_id.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

fn load_app_info(app_id: &str) -> Option<AppInfo> {
    let locales = entry_locales();
    let entry = find_desktop_entry(app_id, &locales)?;

    let name = entry
        .name(&locales)
        .map(|n| n.to_string())
        .unwrap_or_else(|| {
            // Fallback to app_id's last segment, capitalized
            let short = app_id.rsplit('.').next().unwrap_or(app_id);
            let mut chars = short.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                None => app_id.to_string(),
            }
        });

    let icon_name = entry.icon().map(|s| s.to_string());
    let exec = entry.exec().map(|s| s.to_string());
    let desktop_file_id = entry
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());

    let categories: Vec<String> = entry
        .categories()
        .map(|cats| cats.into_iter().map(|c| c.to_string()).collect())
        .unwrap_or_default();

    Some(AppInfo {
        name,
        icon_name,
        exec,
        desktop_file_id,
        app_id: app_id.to_string(),
        categories,
    })
}

/// Look up app info for a program name — the basename of an executable, as
/// found in `/proc/<pid>/comm` or an `Exec=` line.
///
/// This is the loose cousin of [`lookup_app`], for when all that is known
/// about an app is which binary it is: a notification that arrives with no
/// `desktop-entry` hint, say, identified only by the process that sent it.
/// `ghostty` finds `com.mitchellh.ghostty.desktop`, which `lookup_app` alone
/// would miss.
///
/// Matching, in order: the desktop file id, its last reverse-DNS segment, then
/// the program its `Exec=` line runs. Results are cached, `None` included.
pub fn lookup_app_by_binary(binary: &str) -> Option<AppInfo> {
    if binary.is_empty() {
        return None;
    }
    if let Some(cached) = BINARY_CACHE.read().unwrap().get(binary) {
        return cached.clone();
    }

    let result = lookup_app(binary).or_else(|| load_app_info_by_binary(binary));

    BINARY_CACHE
        .write()
        .unwrap()
        .insert(binary.to_string(), result.clone());
    result
}

fn load_app_info_by_binary(binary: &str) -> Option<AppInfo> {
    let mut by_exec = None;

    for path in freedesktop_desktop_entry::Iter::new(freedesktop_desktop_entry::default_paths()) {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };

        // `ghostty` naming `com.mitchellh.ghostty` — the common case, and
        // unambiguous enough to take straight away.
        if stem
            .rsplit('.')
            .next()
            .map(|seg| seg.eq_ignore_ascii_case(binary))
            .unwrap_or(false)
        {
            return lookup_app(stem);
        }

        // An Exec match is weaker — several entries can launch the same
        // program with different arguments — so it is only used if no id
        // matches at all.
        if by_exec.is_none() {
            let runs_binary = DesktopEntry::from_path(path.clone(), Some(&["en"]))
                .ok()
                .and_then(|entry| entry.exec().map(|s| s.to_string()))
                .and_then(|exec| exec.split_whitespace().next().map(|s| s.to_string()))
                .map(|program| {
                    std::path::Path::new(&program)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .map(|name| name.eq_ignore_ascii_case(binary))
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if runs_binary {
                by_exec = Some(stem.to_string());
            }
        }
    }

    by_exec.as_deref().and_then(lookup_app)
}

fn find_desktop_entry(app_id: &str, locales: &[String]) -> Option<DesktopEntry> {
    let normalized = app_id.strip_suffix(".desktop").unwrap_or(app_id);

    let entry_path = freedesktop_desktop_entry::Iter::new(
        freedesktop_desktop_entry::default_paths(),
    )
    .find(|path| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|stem| stem.eq_ignore_ascii_case(normalized))
            .unwrap_or(false)
    })?;

    // The parse keeps only the locales it is given, so this has to be the same
    // list `name()` is later asked for: parsing against `en` alone throws
    // `Name[it]` away and leaves every app in the dock labelled in English.
    DesktopEntry::from_path(entry_path, Some(locales)).ok()
}

/// The locale keys to read a desktop entry in, most preferred first.
///
/// Taken from the interface locale rather than straight from the environment,
/// so an app's name is in the same language as the interface around it: the
/// `locales` setting overrides `LANG`, and reading the environment here would
/// disagree with it — a session started with `LANG=en_US` but set to Italian
/// would show Italian menus over English app names.
///
/// Desktop entries key their translations POSIX-style (`Name[pt_BR]`), so the
/// tag is offered in that form as well as bare.
///
/// Public because anything scanning desktop entries for itself — the launcher
/// builds its own index rather than going through [`lookup_app`] — has to ask
/// for the same locales, and has to hand this same list to `DesktopEntry`'s
/// parser: the parse keeps only the locales it is given.
pub fn entry_locales() -> Vec<String> {
    let mut locales: Vec<String> = Vec::new();
    let tag = crate::i18n::current_locale();
    for candidate in [
        crate::i18n::posix_locale(),
        tag.replace('-', "_"),
        tag.split('-').next().unwrap_or(&tag).to_string(),
        "en".to_string(),
    ] {
        if !candidate.is_empty() && !locales.contains(&candidate) {
            locales.push(candidate);
        }
    }
    locales
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_name_fallback() {
        // When no desktop file exists, should capitalize the last segment
        assert_eq!(display_name_for_app(""), "Otto");
        assert_eq!(display_name_for_app("com.example.myapp"), "Myapp");
        assert_eq!(display_name_for_app("ghostty"), "Ghostty");
    }
}
