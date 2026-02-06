use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub mod default_apps;
pub mod shortcuts;

use shortcuts::{build_bindings, ShortcutBinding, ShortcutMap};
use toml::map::Entry;
use tracing::warn;

use crate::theme::ThemeScheme;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub screen_scale: f64,
    #[serde(default)]
    pub displays: DisplaysConfig,
    pub cursor_theme: String,
    pub icon_theme: Option<String>,
    pub cursor_size: u32,
    #[serde(default)]
    pub input: InputConfig,
    #[serde(default)]
    pub dock: DockConfig,
    #[serde(default)]
    pub layer_shell: LayerShellConfig,
    pub font_family: String,
    pub keyboard_repeat_delay: i32,
    pub keyboard_repeat_rate: i32,
    pub theme_scheme: ThemeScheme,
    pub gtk_theme: Option<String>,
    pub background_image: String,
    pub background_color: String,
    pub locales: Vec<String>,
    pub use_10bit_color: bool,
    #[serde(default = "shortcuts::default_shortcut_map")]
    pub keyboard_shortcuts: ShortcutMap,
    #[serde(skip)]
    #[serde(default)]
    shortcut_bindings: Vec<ShortcutBinding>,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

impl Default for Config {
    fn default() -> Self {
        let mut config = Self {
            screen_scale: 2.0,
            displays: DisplaysConfig::default(),
            cursor_theme: "Notwaita-Black".to_string(),
            icon_theme: None,
            cursor_size: 24,
            input: InputConfig::default(),
            dock: DockConfig::default(),
            layer_shell: LayerShellConfig::default(),
            font_family: "Inter".to_string(),
            keyboard_repeat_delay: 300,
            keyboard_repeat_rate: 30,
            theme_scheme: ThemeScheme::Light,
            gtk_theme: None,
            background_image: "".to_string(),
            background_color: "#1a1a2e".to_string(),
            locales: vec!["en".to_string()],
            use_10bit_color: false,
            keyboard_shortcuts: shortcuts::default_shortcut_map(),
            shortcut_bindings: Vec::new(),
        };
        config.rebuild_shortcut_bindings();
        config
    }
}
pub const WINIT_DISPLAY_ID: &str = "winit";

impl Config {
    pub fn with<R>(f: impl FnOnce(&Config) -> R) -> R {
        let config = CONFIG.get_or_init(Config::init);
        f(config)
    }
    fn init() -> Self {
        let mut merged =
            toml::Value::try_from(Self::default()).expect("default config is always valid toml");

        let mut found_any_config = false;

        // Load config files in order of priority (lowest to highest)
        // 1. System config
        if let Some(system_config) = get_system_config_path() {
            if let Ok(content) = std::fs::read_to_string(&system_config) {
                match content.parse::<toml::Value>() {
                    Ok(value) => {
                        merge_value(&mut merged, value);
                        found_any_config = true;
                        tracing::info!("Loaded system config from {}", system_config.display());
                    }
                    Err(err) => warn!("Failed to parse {}: {err}", system_config.display()),
                }
            }
        }

        // 2. User config (XDG)
        if let Some(user_config) = get_user_config_path() {
            if let Ok(content) = std::fs::read_to_string(&user_config) {
                match content.parse::<toml::Value>() {
                    Ok(value) => {
                        merge_value(&mut merged, value);
                        found_any_config = true;
                        tracing::info!("Loaded user config from {}", user_config.display());
                    }
                    Err(err) => warn!("Failed to parse {}: {err}", user_config.display()),
                }
            }
        }

        // 3. Current directory (dev override)
        if let Ok(content) = std::fs::read_to_string("otto_config.toml") {
            match content.parse::<toml::Value>() {
                Ok(value) => {
                    merge_value(&mut merged, value);
                    found_any_config = true;
                    tracing::info!("Loaded local config from ./otto_config.toml");
                }
                Err(err) => warn!("Failed to parse otto_config.toml: {err}"),
            }
        }

        // 4. Backend overrides (highest priority)
        if let Ok(backend) = std::env::var("SCREEN_COMPOSER_BACKEND") {
            for candidate in backend_override_candidates(&backend) {
                tracing::debug!("Trying to load backend override config: {}", &candidate);
                if let Ok(content) = std::fs::read_to_string(&candidate) {
                    match content.parse::<toml::Value>() {
                        Ok(value) => {
                            merge_value(&mut merged, value);
                            found_any_config = true;
                            tracing::info!("Loaded backend override config from {}", &candidate);
                            break;
                        }
                        Err(err) => {
                            warn!("Failed to parse {candidate}: {err}");
                        }
                    }
                }
            }
        }

        // If no config was found, copy example config to current directory
        if !found_any_config {
            warn!("No configuration file found, using default config");
        }

        let mut config: Config = merged.try_into().unwrap_or_else(|err| {
            warn!("Falling back to default config due to invalid overrides: {err}");
            Self::default()
        });

        config.rebuild_shortcut_bindings();

        // Environment variables for Wayland session
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        std::env::set_var("XDG_CURRENT_DESKTOP", "otto");

        tracing::info!("Config initialized: {:#?}", config.theme_scheme);
        config
    }

    fn rebuild_shortcut_bindings(&mut self) {
        self.shortcut_bindings = build_bindings(&self.keyboard_shortcuts);
    }

    pub fn shortcut_bindings(&self) -> &[ShortcutBinding] {
        &self.shortcut_bindings
    }

    pub fn resolve_display_profile(
        &self,
        name: &str,
        descriptor: &DisplayDescriptor<'_>,
    ) -> Option<DisplayProfile> {
        self.displays.resolve(name, descriptor)
    }
}

fn merge_value(base: &mut toml::Value, overrides: toml::Value) {
    match (base, overrides) {
        (toml::Value::Table(base_map), toml::Value::Table(override_map)) => {
            for (key, override_value) in override_map {
                match base_map.entry(key) {
                    Entry::Occupied(mut entry) => merge_value(entry.get_mut(), override_value),
                    Entry::Vacant(entry) => {
                        entry.insert(override_value);
                    }
                }
            }
        }
        (base_value, override_value) => {
            *base_value = override_value;
        }
    }
}

fn get_system_config_path() -> Option<PathBuf> {
    let path = PathBuf::from("/etc/otto/config.toml");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn get_user_config_path() -> Option<PathBuf> {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join(".config"))
        })?;

    let path = config_dir.join("otto").join("config.toml");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn backend_override_candidates(backend: &str) -> Vec<String> {
    match backend {
        "winit" => vec!["otto_config.winit.toml".into()],
        "tty-udev" => vec![
            "otto_config.tty-udev.toml".into(),
            "otto_config.udev.toml".into(),
        ],
        "x11" => vec![
            "otto_config.x11.toml".into(),
            "otto_config.udev.toml".into(),
        ],
        other => vec![format!("otto_config.{other}.toml")],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DockConfig {
    #[serde(default = "default_dock_size")]
    pub size: f64,
    #[serde(default = "default_genie_scale")]
    pub genie_scale: f64,
    #[serde(default = "default_genie_span")]
    pub genie_span: f64,
    #[serde(default)]
    pub bookmarks: Vec<DockBookmark>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerShellConfig {
    /// Maximum exclusive zone allowed for top edge in logical points (0 = unlimited)
    #[serde(default = "default_max_top")]
    pub max_top: i32,
    /// Maximum exclusive zone allowed for bottom edge in logical points (0 = unlimited)
    #[serde(default = "default_max_bottom")]
    pub max_bottom: i32,
    /// Maximum exclusive zone allowed for left edge in logical points (0 = unlimited)
    #[serde(default = "default_max_left")]
    pub max_left: i32,
    /// Maximum exclusive zone allowed for right edge in logical points (0 = unlimited)
    #[serde(default = "default_max_right")]
    pub max_right: i32,
}

impl Default for LayerShellConfig {
    fn default() -> Self {
        Self {
            max_top: default_max_top(),
            max_bottom: default_max_bottom(),
            max_left: default_max_left(),
            max_right: default_max_right(),
        }
    }
}

fn default_max_top() -> i32 {
    100 // Max 100 logical points for top panels
}

fn default_max_bottom() -> i32 {
    100 // Max 100 logical points for bottom panels/docks
}

fn default_max_left() -> i32 {
    50 // Max 50 logical points for side panels
}

fn default_max_right() -> i32 {
    50 // Max 50 logical points for side panels
}

fn default_dock_size() -> f64 {
    1.0
}

fn default_genie_scale() -> f64 {
    0.5
}

fn default_genie_span() -> f64 {
    10.0
}

/// Input device configuration
///
/// Note: These settings map directly to libinput configuration options.
/// Names reflect libinput's terminology for compatibility and documentation purposes.
///
/// TODO: Consider providing more user-friendly option names/descriptions while
/// maintaining backward compatibility with libinput terminology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    #[serde(default = "default_tap_enabled")]
    pub tap_enabled: bool,
    #[serde(default = "default_tap_drag_enabled")]
    pub tap_drag_enabled: bool,
    #[serde(default = "default_tap_drag_lock_enabled")]
    pub tap_drag_lock_enabled: bool,
    #[serde(default = "default_touchpad_click_method")]
    pub touchpad_click_method: TouchpadClickMethod,
    #[serde(default = "default_touchpad_dwt_enabled")]
    pub touchpad_dwt_enabled: bool,
    #[serde(default = "default_touchpad_natural_scroll_enabled")]
    pub touchpad_natural_scroll_enabled: bool,
    #[serde(default = "default_touchpad_left_handed")]
    pub touchpad_left_handed: bool,
    #[serde(default = "default_touchpad_middle_emulation_enabled")]
    pub touchpad_middle_emulation_enabled: bool,
    #[serde(default)]
    pub xkb_layout: Option<String>,
    #[serde(default)]
    pub xkb_variant: Option<String>,
    #[serde(default)]
    pub xkb_options: Vec<String>,
}

/// Touchpad click method configuration
///
/// Maps to libinput's LIBINPUT_CONFIG_CLICK_METHOD_* enum values.
/// See: https://wayland.freedesktop.org/libinput/doc/latest/clickpad_softbuttons.html
///
/// TODO: Consider more intuitive naming like "finger_count" vs "button_areas"
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TouchpadClickMethod {
    /// Click behavior depends on number of fingers (1=left, 2=right, 3=middle)
    /// Corresponds to LIBINPUT_CONFIG_CLICK_METHOD_CLICKFINGER
    Clickfinger,
    /// Traditional button areas (top-right corner = right click)
    /// Corresponds to LIBINPUT_CONFIG_CLICK_METHOD_BUTTON_AREAS
    ButtonAreas,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            tap_enabled: default_tap_enabled(),
            tap_drag_enabled: default_tap_drag_enabled(),
            tap_drag_lock_enabled: default_tap_drag_lock_enabled(),
            touchpad_click_method: default_touchpad_click_method(),
            touchpad_dwt_enabled: default_touchpad_dwt_enabled(),
            touchpad_natural_scroll_enabled: default_touchpad_natural_scroll_enabled(),
            touchpad_left_handed: default_touchpad_left_handed(),
            touchpad_middle_emulation_enabled: default_touchpad_middle_emulation_enabled(),
            xkb_layout: None,
            xkb_variant: None,
            xkb_options: Vec::new(),
        }
    }
}

fn default_tap_enabled() -> bool {
    true
}

fn default_tap_drag_enabled() -> bool {
    true
}

fn default_tap_drag_lock_enabled() -> bool {
    false
}

fn default_touchpad_click_method() -> TouchpadClickMethod {
    TouchpadClickMethod::Clickfinger
}

fn default_touchpad_dwt_enabled() -> bool {
    true
}

fn default_touchpad_natural_scroll_enabled() -> bool {
    true
}

fn default_touchpad_left_handed() -> bool {
    false
}

fn default_touchpad_middle_emulation_enabled() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockBookmark {
    pub desktop_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub exec_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplaysConfig {
    #[serde(default)]
    pub named: BTreeMap<String, DisplayProfile>,
    #[serde(default)]
    pub generic: Vec<DisplayProfileMatch>,
}

impl DisplaysConfig {
    pub fn resolve(
        &self,
        name: &str,
        descriptor: &DisplayDescriptor<'_>,
    ) -> Option<DisplayProfile> {
        if let Some(profile) = self.named.get(name) {
            return Some(profile.clone());
        }

        self.generic
            .iter()
            .find(|entry| entry.matcher.matches(name, descriptor))
            .map(|entry| entry.profile.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayProfile {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub resolution: Option<DisplayResolution>,
    #[serde(default)]
    pub refresh_hz: Option<f64>,
    #[serde(default)]
    pub position: Option<DisplayPosition>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisplayResolution {
    pub width: u32,
    pub height: u32,
}

impl DisplayResolution {
    #[allow(dead_code)]
    pub fn as_f64(self) -> (f64, f64) {
        (self.width as f64, self.height as f64)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DisplayPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayProfileMatch {
    #[serde(default, rename = "match")]
    pub matcher: DisplayMatcher,
    #[serde(flatten)]
    pub profile: DisplayProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayMatcher {
    #[serde(default)]
    pub connector: Option<String>,
    #[serde(default)]
    pub connector_prefix: Option<String>,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub kind: Option<DisplayKind>,
}

impl DisplayMatcher {
    fn matches(&self, connector: &str, descriptor: &DisplayDescriptor<'_>) -> bool {
        if let Some(expected) = &self.connector {
            if expected != connector && descriptor.connector != expected {
                return false;
            }
        }

        if let Some(prefix) = &self.connector_prefix {
            let matches_actual = connector.starts_with(prefix);
            let matches_descriptor = descriptor.connector.starts_with(prefix);
            if !matches_actual && !matches_descriptor {
                return false;
            }
        }

        if let Some(expected_vendor) = &self.vendor {
            match descriptor.vendor {
                Some(vendor) if equals_ignore_case(vendor, expected_vendor) => {}
                _ => return false,
            }
        }

        if let Some(expected_model) = &self.model {
            match descriptor.model {
                Some(model) if equals_ignore_case(model, expected_model) => {}
                _ => return false,
            }
        }

        if let Some(expected_kind) = self.kind {
            if descriptor.kind.unwrap_or(DisplayKind::Unknown) != expected_kind {
                return false;
            }
        }

        true
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DisplayKind {
    Internal,
    External,
    Virtual,
    #[default]
    Unknown,
}

#[derive(Debug, Clone)]
pub struct DisplayDescriptor<'a> {
    pub connector: &'a str,
    pub vendor: Option<&'a str>,
    pub model: Option<&'a str>,
    pub kind: Option<DisplayKind>,
}

impl<'a> DisplayDescriptor<'a> {
    #[allow(dead_code)]
    pub fn new(connector: &'a str) -> Self {
        Self {
            connector,
            vendor: None,
            model: None,
            kind: None,
        }
    }
}

fn equals_ignore_case(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    use std::fs;

    #[test]
    fn theme_scheme_defaults_to_light() {
        let config = Config::default();
        assert!(matches!(config.theme_scheme, ThemeScheme::Light));
    }

    #[test]
    fn theme_scheme_overrides_to_dark_in_toml() {
        let overrides = r#"
            theme_scheme = "Dark"
        "#;

        let config: Config = toml::from_str(overrides).expect("Config should deserialize");
        assert!(matches!(config.theme_scheme, ThemeScheme::Dark));
    }

    #[test]
    #[serial]
    fn test_get_user_config_path_with_xdg_config_home() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Set XDG_CONFIG_HOME temporarily
        let old_xdg = env::var("XDG_CONFIG_HOME").ok();
        env::set_var("XDG_CONFIG_HOME", temp_dir.path());

        // Create the config file
        let config_dir = temp_dir.path().join("otto");
        fs::create_dir_all(&config_dir).unwrap();
        let config_file = config_dir.join("config.toml");
        fs::write(&config_file, "# test config").unwrap();

        let path = get_user_config_path();
        assert!(path.is_some());
        assert_eq!(path.unwrap(), config_file);

        // Cleanup
        if let Some(old) = old_xdg {
            env::set_var("XDG_CONFIG_HOME", old);
        } else {
            env::remove_var("XDG_CONFIG_HOME");
        }
        // temp_dir automatically cleaned up when dropped
    }

    #[test]
    #[serial]
    fn test_get_user_config_path_without_file() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Set XDG_CONFIG_HOME to a dir without config
        let old_xdg = env::var("XDG_CONFIG_HOME").ok();
        env::set_var("XDG_CONFIG_HOME", temp_dir.path());

        let path = get_user_config_path();
        assert!(path.is_none());

        // Cleanup
        if let Some(old) = old_xdg {
            env::set_var("XDG_CONFIG_HOME", old);
        } else {
            env::remove_var("XDG_CONFIG_HOME");
        }
        // temp_dir automatically cleaned up when dropped
    }

    #[test]
    fn test_get_system_config_path() {
        // System config path is fixed
        let path = get_system_config_path();

        // Only returns Some if the file exists
        if let Some(p) = path {
            assert_eq!(p, PathBuf::from("/etc/otto/config.toml"));
        }
    }

    #[test]
    fn test_config_merge_priority() {
        // Test that config values merge correctly with priority
        let mut base =
            toml::Value::try_from(Config::default()).expect("default config is valid toml");

        // Override with custom values
        let override_toml = r#"
            screen_scale = 3.0
            font_family = "Custom Font"
        "#;
        let override_value: toml::Value = override_toml.parse().unwrap();

        merge_value(&mut base, override_value);

        let config: Config = base.try_into().unwrap();
        assert_eq!(config.screen_scale, 3.0);
        assert_eq!(config.font_family, "Custom Font");
    }

    #[test]
    fn test_config_partial_override() {
        // Test that partial overrides work correctly
        let mut base =
            toml::Value::try_from(Config::default()).expect("default config is valid toml");

        // Override only screen_scale, leave other values
        let override_toml = r#"
            screen_scale = 1.5
        "#;
        let override_value: toml::Value = override_toml.parse().unwrap();

        merge_value(&mut base, override_value);

        let config: Config = base.try_into().unwrap();
        assert_eq!(config.screen_scale, 1.5);
        // Other defaults should remain
        assert_eq!(config.cursor_theme, "Notwaita-Black");
    }

    #[test]
    #[serial]
    #[test]
    fn test_backend_override_candidates() {
        let winit = backend_override_candidates("winit");
        assert_eq!(winit, vec!["otto_config.winit.toml"]);

        let udev = backend_override_candidates("tty-udev");
        assert_eq!(
            udev,
            vec!["otto_config.tty-udev.toml", "otto_config.udev.toml"]
        );

        let x11 = backend_override_candidates("x11");
        assert_eq!(x11, vec!["otto_config.x11.toml", "otto_config.udev.toml"]);

        let custom = backend_override_candidates("custom");
        assert_eq!(custom, vec!["otto_config.custom.toml"]);
    }
}
