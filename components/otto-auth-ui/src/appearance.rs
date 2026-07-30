//! Colours and imagery the panel draws with.
//!
//! These come from Otto's own configuration, so the login screen and the lock
//! screen look like the session they lead into. The config is read directly
//! rather than through the compositor's `Config` type: that lives in the
//! compositor crate, and neither client can depend on it.

use std::path::PathBuf;

use serde::Deserialize;
use skia_safe::Color;

/// The subset of Otto's config that affects how the panel looks.
#[derive(Debug, Clone)]
pub struct Appearance {
    /// Wallpaper to fill the screen with. `None` draws a gradient built from
    /// [`Appearance::background`] instead.
    pub wallpaper: Option<PathBuf>,
    pub background: Color,
    pub accent: Color,
    pub font_family: String,
    pub dark: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            wallpaper: None,
            background: Color::from_argb(255, 26, 26, 46),
            accent: Color::from_argb(255, 10, 132, 255),
            font_family: "Inter".to_string(),
            dark: true,
        }
    }
}

impl Appearance {
    /// Read the system config, then the current user's if there is one.
    ///
    /// A greeter runs as an unprivileged user with no config of its own, so in
    /// practice only `/etc/otto/config.toml` is found there; a lock screen runs
    /// as the user and picks up both.
    pub fn load() -> Self {
        let mut appearance = Self::default();

        for path in Self::config_paths() {
            match std::fs::read_to_string(&path) {
                Ok(content) => match toml::from_str::<ConfigFile>(&content) {
                    Ok(config) => {
                        appearance.apply(config);
                        tracing::debug!(path = %path.display(), "read appearance from config");
                    }
                    Err(err) => {
                        tracing::warn!(path = %path.display(), %err, "ignoring unreadable config")
                    }
                },
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => tracing::warn!(path = %path.display(), %err, "cannot read config"),
            }
        }

        appearance
    }

    /// System config first, so the user's overrides it.
    fn config_paths() -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from("/etc/otto/config.toml")];

        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
        if let Some(dir) = config_home {
            paths.push(dir.join("otto").join("config.toml"));
        }

        paths
    }

    fn apply(&mut self, config: ConfigFile) {
        // An empty string is how Otto's config spells "no wallpaper", so it
        // must not become a path that fails to load later.
        if let Some(image) = config
            .background_image
            .filter(|image| !image.trim().is_empty())
        {
            self.wallpaper = Some(PathBuf::from(image));
        }
        if let Some(color) = config.background_color.as_deref().and_then(parse_hex) {
            self.background = color;
        }
        if let Some(accent) = config.accent_color.as_deref().map(accent_color) {
            self.accent = accent;
        }
        if let Some(family) = config.font_family.filter(|f| !f.trim().is_empty()) {
            self.font_family = family;
        }
        if let Some(scheme) = config.theme_scheme.as_deref() {
            self.dark = !scheme.eq_ignore_ascii_case("light");
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    background_image: Option<String>,
    background_color: Option<String>,
    accent_color: Option<String>,
    font_family: Option<String>,
    theme_scheme: Option<String>,
}

/// Otto's named accents. Kept in step with `src/theme/colors_light.rs` in the
/// compositor, which is where these values come from.
fn accent_color(name: &str) -> Color {
    let hex = match name.trim().to_ascii_lowercase().as_str() {
        "red" => "#FF453A",
        "orange" => "#FF9500",
        "yellow" => "#FFCC00",
        "green" => "#28CD41",
        "mint" => "#00C7BE",
        "teal" => "#59ADC4",
        "cyan" => "#55BEF0",
        "indigo" => "#5856D6",
        "purple" => "#AF52DE",
        "pink" => "#FF2D55",
        "gray" => "#8E8E93",
        "brown" => "#A2845E",
        // An unknown name falls back to blue, as the compositor's theme does;
        // a literal colour is also accepted so a greeter can be themed alone.
        other => return parse_hex(other).unwrap_or(Color::from_argb(255, 10, 132, 255)),
        // "blue" lands here through the fallback, which resolves to the same
        // value, so it needs no arm of its own.
    };
    parse_hex(hex).unwrap_or(Color::from_argb(255, 10, 132, 255))
}

/// Parse `#RGB`, `#RRGGBB` or `#RRGGBBAA`.
fn parse_hex(value: &str) -> Option<Color> {
    let hex = value.trim().strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();

    match hex.len() {
        3 => {
            let nibble = |i: usize| {
                u8::from_str_radix(&hex[i..i + 1], 16)
                    .ok()
                    .map(|v| v << 4 | v)
            };
            Some(Color::from_argb(255, nibble(0)?, nibble(1)?, nibble(2)?))
        }
        6 => Some(Color::from_argb(255, byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color::from_argb(byte(6)?, byte(0)?, byte(2)?, byte(4)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colours() {
        assert_eq!(
            parse_hex("#0A84FF"),
            Some(Color::from_argb(255, 10, 132, 255))
        );
        assert_eq!(
            parse_hex("  #fff  "),
            Some(Color::from_argb(255, 255, 255, 255))
        );
        assert_eq!(
            parse_hex("#FF000080"),
            Some(Color::from_argb(128, 255, 0, 0))
        );
        assert_eq!(parse_hex("0A84FF"), None, "a leading # is required");
        assert_eq!(parse_hex("#GGGGGG"), None);
        assert_eq!(parse_hex("#12345"), None);
    }

    #[test]
    fn named_accents_resolve_and_unknown_names_fall_back() {
        assert_eq!(accent_color("pink"), Color::from_argb(255, 255, 45, 85));
        assert_eq!(accent_color("BLUE"), Color::from_argb(255, 10, 132, 255));
        assert_eq!(
            accent_color("nonsense"),
            Color::from_argb(255, 10, 132, 255)
        );
        // A literal colour is accepted where a name is expected.
        assert_eq!(accent_color("#123456"), Color::from_argb(255, 18, 52, 86));
    }

    /// An empty `background_image` means "no wallpaper", not a path to ""..
    #[test]
    fn empty_config_strings_are_ignored() {
        let mut appearance = Appearance::default();
        appearance.apply(ConfigFile {
            background_image: Some("  ".into()),
            background_color: None,
            accent_color: None,
            font_family: Some("".into()),
            theme_scheme: Some("Light".into()),
        });
        assert!(appearance.wallpaper.is_none());
        assert_eq!(appearance.font_family, "Inter");
        assert!(!appearance.dark);
    }
}
