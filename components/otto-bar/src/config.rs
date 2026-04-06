// Topbar layout/style constants (not user-configurable).

/// Bar height in logical points.
pub const BAR_HEIGHT: u32 = 32;

/// Left panel initial width (will animate to content size).
pub const LEFT_WIDTH: u32 = 80;

/// Right panel initial width (will animate to content size).
pub const RIGHT_WIDTH: u32 = 80;

/// Top margin from screen edge.
pub const BAR_MARGIN_TOP: i32 = 0;

/// Side margin from screen edge.
pub const BAR_MARGIN_SIDE: i32 = 0;

/// Horizontal padding inside a panel.
pub const BAR_PADDING_H: f32 = 14.0;

/// Spacing between tray icons.
#[allow(dead_code)]
pub const TRAY_ICON_SPACING: f32 = 8.0;

/// Tray icon size in logical points.
pub const TRAY_ICON_SIZE: f32 = 22.0;

/// Gap between tray icons and the clock.
pub const TRAY_CLOCK_GAP: f32 = 12.0;

/// Right panel minimum width.
pub const MIN_RIGHT_WIDTH: u32 = 60;

/// Corner radius for the bar's corners.
pub const BAR_CORNER_RADIUS: f32 = 8.0;

// ---------------------------------------------------------------------------
// Runtime config — loaded from topbar.toml on first access
// ---------------------------------------------------------------------------

use std::sync::LazyLock;

/// Default clock format: "March 23, Thursday 21:16"
const DEFAULT_CLOCK_FORMAT: &str = "%B %-d, %A %H:%M";

/// User-configurable topbar settings.
#[derive(Debug, Clone)]
pub struct TopbarConfig {
    /// chrono strftime format string for the clock.
    pub clock_format: String,
}

impl Default for TopbarConfig {
    fn default() -> Self {
        Self {
            clock_format: DEFAULT_CLOCK_FORMAT.to_string(),
        }
    }
}

static CONFIG: LazyLock<TopbarConfig> = LazyLock::new(load_config);

/// Access the current topbar configuration.
#[allow(dead_code)]
pub fn config() -> &'static TopbarConfig {
    &CONFIG
}

/// Return the clock format string.
pub fn clock_format() -> &'static str {
    &CONFIG.clock_format
}

fn load_config() -> TopbarConfig {
    // Search order: /etc/otto/otto-bar.toml → ~/.config/otto/otto-bar.toml → ./otto-bar.toml
    let candidates: Vec<std::path::PathBuf> = {
        let mut v = Vec::new();
        v.push(std::path::PathBuf::from("/etc/otto/otto-bar.toml"));
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
            })
        {
            v.push(xdg.join("otto").join("otto-bar.toml"));
        }
        v.push(std::path::PathBuf::from("otto-bar.toml"));
        v
    };

    let mut cfg = TopbarConfig::default();

    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            match content.parse::<toml::Value>() {
                Ok(table) => {
                    if let Some(fmt) = table.get("clock_format").and_then(|v| v.as_str()) {
                        cfg.clock_format = fmt.to_string();
                    }
                    break;
                }
                Err(e) => {
                    tracing::warn!("Failed to parse {}: {e}", path.display());
                }
            }
        }
    }

    cfg
}
