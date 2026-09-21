// Topbar layout/style constants (not user-configurable).

/// Bar height in logical points.
pub const BAR_HEIGHT: u32 = 30;

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

/// Default clock format.
///
/// Taken from the locale rather than hardcoded: the ordering of day and month
/// and the choice of a 12- or 24-hour clock are conventions, not preferences,
/// and they differ between en-GB and en-US before any other language is
/// involved. An explicit `clock_format` in the config file still wins.
fn default_clock_format() -> String {
    otto_kit::t!("bar-clock-format").to_string()
}

/// When the battery indicator is drawn at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatteryVisibility {
    /// Shown on a machine that has a battery, hidden on one that does not.
    #[default]
    Auto,
    Always,
    Never,
}

/// Where the percentage is written, if anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PercentagePosition {
    /// Inside the glyph, over the fill.
    #[default]
    Inside,
    /// To the left of the glyph, as text.
    Beside,
    Off,
}

/// One `[[battery.profiles]]` entry: a command the menu can run, and
/// optionally how to tell that it is the one currently in effect.
#[derive(Debug, Clone)]
pub struct ProfileEntry {
    pub label: String,
    /// argv — the first element is the program.
    pub command: Vec<String>,
    /// Check-marked when `scaling_governor` reads this.
    pub governor: Option<String>,
    /// Check-marked when `energy_performance_preference` reads this.
    pub epp: Option<String>,
}

/// Everything about the battery indicator.
#[derive(Debug, Clone)]
pub struct BatteryConfig {
    pub visibility: BatteryVisibility,
    /// Tint the fill by level. False draws it in the bar's text colour, which
    /// is what a monochrome bar wants.
    pub colored: bool,
    pub percentage: PercentagePosition,
    /// Below this, the fill turns to `color_low`.
    pub low_level: f64,
    /// Below this, to `color_critical`.
    pub critical_level: f64,
    /// Glyph size in logical points, excluding the cap.
    pub width: f32,
    pub height: f32,
    pub color_normal: skia_safe::Color,
    pub color_low: skia_safe::Color,
    pub color_critical: skia_safe::Color,
    pub color_charging: skia_safe::Color,
    /// Seconds between battery readings.
    pub update_interval: u64,
    /// Whether clicking opens the power menu.
    pub menu: bool,
    /// Show the percentage and time remaining at the top of the menu.
    pub show_battery_info: bool,
    /// Show the live CPU frequency and policy in the menu.
    pub show_cpu_info: bool,
    /// "auto", "power-profiles" or "commands".
    pub profile_backend: String,
    pub profiles: Vec<ProfileEntry>,
    /// Command behind the menu's last entry. Empty hides the entry.
    pub settings_command: Vec<String>,
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            visibility: BatteryVisibility::Auto,
            colored: true,
            percentage: PercentagePosition::Inside,
            low_level: 20.0,
            critical_level: 10.0,
            // Wide enough that "100%" sits inside the glyph with air around
            // it — three digits and a sign is the widest the label ever gets.
            width: 28.0,
            height: 13.0,
            // Not taken from the theme: a battery's colours mean charge, not
            // brand, and an accent-coloured battery says nothing about level.
            color_normal: skia_safe::Color::from_rgb(0x34, 0xC7, 0x59),
            color_low: skia_safe::Color::from_rgb(0xFF, 0x9F, 0x0A),
            color_critical: skia_safe::Color::from_rgb(0xFF, 0x3B, 0x30),
            color_charging: skia_safe::Color::from_rgb(0x34, 0xC7, 0x59),
            update_interval: 10,
            menu: true,
            show_battery_info: true,
            show_cpu_info: true,
            profile_backend: "auto".to_string(),
            profiles: Vec::new(),
            settings_command: vec!["otto-settings".to_string()],
        }
    }
}

/// User-configurable topbar settings.
#[derive(Debug, Clone)]
pub struct TopbarConfig {
    /// chrono strftime format string for the clock.
    pub clock_format: String,
    pub battery: BatteryConfig,
}

impl Default for TopbarConfig {
    fn default() -> Self {
        Self {
            clock_format: default_clock_format(),
            battery: BatteryConfig::default(),
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

/// Settings for the battery indicator.
pub fn battery_config() -> &'static BatteryConfig {
    &CONFIG.battery
}

/// Parse `#RGB`, `#RRGGBB` or `#AARRGGBB`. Returns None for anything else,
/// so a typo leaves the default colour rather than painting the glyph black.
fn parse_color(raw: &str) -> Option<skia_safe::Color> {
    let hex = raw.trim().strip_prefix('#')?;
    let digits = |s: &str| u32::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let v = digits(hex)?;
            // #abc means #aabbcc.
            let (r, g, b) = ((v >> 8) & 0xF, (v >> 4) & 0xF, v & 0xF);
            Some(skia_safe::Color::from_rgb(
                (r * 17) as u8,
                (g * 17) as u8,
                (b * 17) as u8,
            ))
        }
        6 => {
            let v = digits(hex)?;
            Some(skia_safe::Color::from_rgb(
                ((v >> 16) & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                (v & 0xFF) as u8,
            ))
        }
        8 => {
            let v = digits(hex)?;
            Some(skia_safe::Color::from_argb(
                ((v >> 24) & 0xFF) as u8,
                ((v >> 16) & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                (v & 0xFF) as u8,
            ))
        }
        _ => None,
    }
}

/// Read a command as either a bare string or an argv array, so both
/// `settings_command = "otto-settings"` and the array form work.
fn parse_command(value: &toml::Value) -> Option<Vec<String>> {
    match value {
        toml::Value::String(s) if s.trim().is_empty() => Some(Vec::new()),
        toml::Value::String(s) => Some(s.split_whitespace().map(str::to_string).collect()),
        toml::Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
        ),
        _ => None,
    }
}

fn parse_battery(table: &toml::Value, cfg: &mut BatteryConfig) {
    let Some(battery) = table.get("battery") else {
        return;
    };

    // `show` is a tri-state spelled as either a bool or "auto", because
    // "hide this" and "I have no battery" are different intentions.
    match battery.get("show") {
        Some(toml::Value::Boolean(true)) => cfg.visibility = BatteryVisibility::Always,
        Some(toml::Value::Boolean(false)) => cfg.visibility = BatteryVisibility::Never,
        Some(toml::Value::String(s)) => {
            cfg.visibility = match s.as_str() {
                "always" => BatteryVisibility::Always,
                "never" => BatteryVisibility::Never,
                _ => BatteryVisibility::Auto,
            }
        }
        _ => {}
    }

    if let Some(v) = battery.get("colored").and_then(|v| v.as_bool()) {
        cfg.colored = v;
    }
    // Also a tri-state: false is the same as "off", which is how someone who
    // has not read the documentation will write it.
    match battery.get("percentage") {
        Some(toml::Value::Boolean(true)) => cfg.percentage = PercentagePosition::Inside,
        Some(toml::Value::Boolean(false)) => cfg.percentage = PercentagePosition::Off,
        Some(toml::Value::String(s)) => {
            cfg.percentage = match s.as_str() {
                "beside" => PercentagePosition::Beside,
                "off" | "none" | "hidden" => PercentagePosition::Off,
                _ => PercentagePosition::Inside,
            }
        }
        _ => {}
    }

    if let Some(v) = battery.get("low_level").and_then(|v| v.as_float()) {
        cfg.low_level = v;
    } else if let Some(v) = battery.get("low_level").and_then(|v| v.as_integer()) {
        cfg.low_level = v as f64;
    }
    if let Some(v) = battery.get("critical_level").and_then(|v| v.as_float()) {
        cfg.critical_level = v;
    } else if let Some(v) = battery.get("critical_level").and_then(|v| v.as_integer()) {
        cfg.critical_level = v as f64;
    }
    if let Some(v) = battery.get("width").and_then(number) {
        cfg.width = v as f32;
    }
    if let Some(v) = battery.get("height").and_then(number) {
        cfg.height = v as f32;
    }
    if let Some(v) = battery.get("update_interval").and_then(|v| v.as_integer()) {
        cfg.update_interval = v.max(1) as u64;
    }
    if let Some(v) = battery.get("menu").and_then(|v| v.as_bool()) {
        cfg.menu = v;
    }
    if let Some(v) = battery.get("show_battery_info").and_then(|v| v.as_bool()) {
        cfg.show_battery_info = v;
    }
    if let Some(v) = battery.get("show_cpu_info").and_then(|v| v.as_bool()) {
        cfg.show_cpu_info = v;
    }
    if let Some(v) = battery.get("profile_backend").and_then(|v| v.as_str()) {
        cfg.profile_backend = v.to_string();
    }
    if let Some(cmd) = battery.get("settings_command").and_then(parse_command) {
        cfg.settings_command = cmd;
    }

    for (key, slot) in [
        ("color_normal", &mut cfg.color_normal),
        ("color_low", &mut cfg.color_low),
        ("color_critical", &mut cfg.color_critical),
        ("color_charging", &mut cfg.color_charging),
    ] {
        if let Some(raw) = battery.get(key).and_then(|v| v.as_str()) {
            match parse_color(raw) {
                Some(color) => *slot = color,
                None => tracing::warn!("battery.{key}: {raw:?} is not a colour like \"#34C759\""),
            }
        }
    }

    if let Some(entries) = battery.get("profiles").and_then(|v| v.as_array()) {
        cfg.profiles = entries
            .iter()
            .filter_map(|entry| {
                let label = entry.get("label").and_then(|v| v.as_str())?.to_string();
                let command = entry.get("command").and_then(parse_command)?;
                if command.is_empty() {
                    tracing::warn!("battery profile {label:?} has no command");
                    return None;
                }
                Some(ProfileEntry {
                    label,
                    command,
                    governor: entry
                        .get("governor")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    epp: entry
                        .get("epp")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                })
            })
            .collect();
    }
}

/// TOML tells 1 and 1.0 apart; a size does not care which was written.
fn number(value: &toml::Value) -> Option<f64> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|i| i as f64))
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
                    parse_battery(&table, &mut cfg.battery);
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
