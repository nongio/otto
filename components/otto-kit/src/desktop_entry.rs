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
    CACHE.write().unwrap().insert(app_id.to_string(), result.clone());

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
    let entry = find_desktop_entry(app_id)?;
    let locales = sys_locales();

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

fn find_desktop_entry(app_id: &str) -> Option<DesktopEntry> {
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

    let locales: Vec<&str> = vec!["en"];
    DesktopEntry::from_path(entry_path, Some(&locales)).ok()
}

fn sys_locales() -> Vec<String> {
    // Simple locale detection from env
    let mut locales = Vec::new();
    for var in ["LC_MESSAGES", "LC_ALL", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() && val != "C" && val != "POSIX" {
                // Extract language code (e.g. "en_US.UTF-8" → "en_US", "en")
                let base = val.split('.').next().unwrap_or(&val);
                if !locales.contains(&base.to_string()) {
                    locales.push(base.to_string());
                }
                let lang = base.split('_').next().unwrap_or(base);
                if !locales.contains(&lang.to_string()) {
                    locales.push(lang.to_string());
                }
            }
        }
    }
    if locales.is_empty() {
        locales.push("en".to_string());
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
