//! Wallpaper settings: the global `background_*` keys, overridden per output
//! and per workspace.
//!
//! An override names only what it changes. What a workspace shows is the
//! global settings, then its output's override, then its own, field by field
//! — so a workspace can change the image and keep the output's mode, or an
//! output the mode and keep the global image.

use serde::{Deserialize, Serialize};
use tracing::warn;

use super::{file, writable_config_path, Config};

/// Wallpaper settings for one output or one workspace, over the ones below
/// it. Every field is optional; an absent one is inherited.
///
/// Written as `[background_outputs."DP-1"]` for an output and
/// `[workspaces.entries."DP-1:0".background]` for a workspace, with the same
/// names as the global keys without their `background_` prefix.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperOverride {
    /// The image. An empty string is an explicit "no image": the colour's
    /// gradient, whatever is set below.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f64>,
}

impl WallpaperOverride {
    /// Nothing overridden: the record can be dropped.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The override as the table it is written as, unset fields left out.
    pub(super) fn to_table(&self) -> toml_edit::Table {
        let mut table = toml_edit::Table::new();
        if let Some(image) = &self.image {
            table.insert("image", toml_edit::value(image.as_str()));
        }
        if let Some(color) = &self.color {
            table.insert("color", toml_edit::value(color.as_str()));
        }
        if let Some(mode) = &self.mode {
            table.insert("mode", toml_edit::value(mode.as_str()));
        }
        if let Some(x) = self.position_x {
            table.insert("position_x", toml_edit::value(x));
        }
        if let Some(y) = self.position_y {
            table.insert("position_y", toml_edit::value(y));
        }
        if let Some(zoom) = self.zoom {
            table.insert("zoom", toml_edit::value(zoom));
        }
        table
    }

    /// Read an override off a table in the file's own syntax tree.
    pub(super) fn from_table(table: &dyn toml_edit::TableLike) -> Self {
        let string = |key: &str| table.get(key).and_then(|v| v.as_str()).map(str::to_string);
        let number = |key: &str| {
            table
                .get(key)
                .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64)))
        };
        Self {
            image: string("image"),
            color: string("color"),
            mode: string("mode"),
            position_x: number("position_x"),
            position_y: number("position_y"),
            zoom: number("zoom"),
        }
    }
}

/// What one workspace's background shows, every override applied.
#[derive(Debug, Clone, PartialEq)]
pub struct WallpaperSettings {
    /// Empty for none.
    pub image: String,
    pub color: layers::skia::Color4f,
    pub layout: otto_kit::wallpaper::Layout,
}

impl Config {
    /// The wallpaper of the workspace whose own override is `workspace`, on
    /// output `output`: the global settings, then the output's override,
    /// then the workspace's.
    pub fn wallpaper_for(
        &self,
        output: &str,
        workspace: Option<&WallpaperOverride>,
    ) -> WallpaperSettings {
        let mut image = self.background_image.clone();
        let mut color = self.background_color.clone();
        let mut mode = self.background_mode.clone();
        let mut position = (self.background_position_x, self.background_position_y);
        let mut zoom = self.background_zoom;
        for layer in [self.background_outputs.get(output), workspace]
            .into_iter()
            .flatten()
        {
            if let Some(value) = &layer.image {
                image = value.clone();
            }
            if let Some(value) = &layer.color {
                color = value.clone();
            }
            if let Some(value) = &layer.mode {
                mode = value.clone();
            }
            if let Some(value) = layer.position_x {
                position.0 = value;
            }
            if let Some(value) = layer.position_y {
                position.1 = value;
            }
            if let Some(value) = layer.zoom {
                zoom = value;
            }
        }
        WallpaperSettings {
            image: expand_home(&image),
            color: colour_from(&color),
            layout: layout_from(&mode, position, zoom),
        }
    }

    /// The global image, as [`Self::wallpaper_for`] resolves it before any
    /// override.
    pub fn global_wallpaper_image(&self) -> String {
        expand_home(&self.background_image)
    }

    /// What is wrong with the wallpaper settings as written: an unknown mode
    /// or an unreadable colour, globally or in an output's override. Each
    /// still works — `fill`, and Otto's default colour — so these are
    /// warnings, said once when the configuration is read.
    pub fn wallpaper_problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut check = |place: &str, mode: Option<&str>, color: Option<&str>| {
            if let Some(mode) = mode.filter(|m| otto_kit::wallpaper::Mode::parse(m).is_none()) {
                problems.push(format!(
                    "{place}: unknown background mode `{mode}`, using fill"
                ));
            }
            if let Some(color) = color.filter(|c| otto_kit::color::parse_hex_lenient(c).is_none()) {
                problems.push(format!(
                    "{place}: `{color}` is not a colour, using the default"
                ));
            }
        };
        check(
            "background_mode/background_color",
            Some(&self.background_mode),
            Some(&self.background_color),
        );
        for (output, record) in &self.background_outputs {
            check(
                &format!("[background_outputs.\"{output}\"]"),
                record.mode.as_deref(),
                record.color.as_deref(),
            );
        }
        problems
    }

    /// How the global wallpaper is laid out, from the `background_*`
    /// settings. An unknown mode is `fill` and out-of-range numbers are
    /// pulled into range, rather than leaving the desktop blank over a typo.
    pub fn wallpaper_layout(&self) -> otto_kit::wallpaper::Layout {
        layout_from(
            &self.background_mode,
            (self.background_position_x, self.background_position_y),
            self.background_zoom,
        )
    }
}

/// The layout the settings describe. An unknown mode is `fill`; it is
/// reported once, when the configuration is read
/// ([`Config::wallpaper_problems`]), not on every layout pass.
fn layout_from(mode: &str, position: (f64, f64), zoom: f64) -> otto_kit::wallpaper::Layout {
    otto_kit::wallpaper::Layout {
        mode: otto_kit::wallpaper::Mode::parse(mode).unwrap_or_default(),
        position: (position.0 as f32, position.1 as f32),
        zoom: zoom as f32,
    }
    .sanitised()
}

/// The colour a `background_color` names, or Otto's default for one that
/// names none — reported, like a bad mode, when the configuration is read.
fn colour_from(text: &str) -> layers::skia::Color4f {
    otto_kit::color::parse_hex_lenient(text)
        .map(layers::skia::Color4f::from)
        .unwrap_or(layers::skia::Color4f::new(0.1, 0.1, 0.18, 1.0))
}

/// `path` with a leading `~/` made the home folder, as a hand-written
/// configuration may spell it.
fn expand_home(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => otto_kit::xdg::home()
            .map(|home| home.join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        None => path.to_string(),
    }
}

/// Change output `output`'s wallpaper override, in the running configuration
/// and in the writable config file. `edit` is handed the override as it
/// stands; one it leaves empty is removed.
///
/// The file is edited through `toml_edit`, so the rest of a hand-maintained
/// config survives byte for byte.
pub fn save_output_wallpaper(output: &str, edit: impl FnOnce(&mut WallpaperOverride)) {
    let mut record =
        Config::with(|c| c.background_outputs.get(output).cloned()).unwrap_or_default();
    edit(&mut record);
    let stored = record.clone();
    Config::update(|config| {
        if stored.is_empty() {
            config.background_outputs.remove(output);
        } else {
            config
                .background_outputs
                .insert(output.to_string(), stored.clone());
        }
    });

    let path = writable_config_path();
    let mut doc = match file::load_document(&path) {
        Ok(doc) => doc,
        Err(err) => {
            warn!("Failed to save the output's background: {err}");
            return;
        }
    };
    if let Err(err) = edit_output_wallpaper(&mut doc, output, &record) {
        warn!(
            "Failed to save the output's background to {}: {err}",
            path.display()
        );
        return;
    }
    if let Err(err) = file::store_document(&path, &doc) {
        warn!("Failed to save the output's background: {err}");
    }
}

/// [`save_output_wallpaper`]'s edit of the document, so it can be tested
/// without a file. Fails rather than overwrite a `background_outputs` key
/// that is not a table: the user wrote that, whatever it is.
fn edit_output_wallpaper(
    doc: &mut toml_edit::DocumentMut,
    output: &str,
    record: &WallpaperOverride,
) -> Result<(), String> {
    let outputs = doc
        .as_table_mut()
        .entry("background_outputs")
        .or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            toml_edit::Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| "`background_outputs` is not a table".to_string())?;
    if record.is_empty() {
        outputs.remove(output);
    } else {
        outputs.insert(output, toml_edit::Item::Table(record.to_table()));
    }
    if outputs.is_empty() {
        doc.as_table_mut().remove("background_outputs");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_kit::wallpaper::Mode;

    fn config(text: &str) -> Config {
        toml::from_str(text).unwrap()
    }

    /// The most specific setting wins, field by field.
    #[test]
    fn overrides_apply_from_global_to_output_to_workspace() {
        let config = config(
            r##"
            background_image = "/global.png"
            background_color = "#000000"
            background_mode = "fill"

            [background_outputs."DP-1"]
            mode = "fit"
            color = "#ff0000"
            "##,
        );
        let global = config.wallpaper_for("eDP-1", None);
        assert_eq!(global.image, "/global.png");
        assert_eq!(global.layout.mode, Mode::Fill);

        let output = config.wallpaper_for("DP-1", None);
        assert_eq!(output.image, "/global.png", "the image is inherited");
        assert_eq!(output.layout.mode, Mode::Fit);
        assert_eq!(output.color.r, 1.0);

        let workspace = WallpaperOverride {
            image: Some("/mine.jpg".into()),
            position_y: Some(0.0),
            ..Default::default()
        };
        let mine = config.wallpaper_for("DP-1", Some(&workspace));
        assert_eq!(mine.image, "/mine.jpg");
        assert_eq!(mine.layout.mode, Mode::Fit, "the output's mode is kept");
        assert_eq!(mine.layout.position, (0.5, 0.0));
    }

    /// A typo in a mode or a colour is reported once, at load, and still
    /// works: fill, and the default colour.
    #[test]
    fn bad_values_are_reported_and_fall_back() {
        let config = config(
            r##"
            background_mode = "zoomed"
            background_color = "blue-ish"
            [background_outputs."DP-1"]
            mode = "fit"
            color = "#12"
            "##,
        );
        let problems = config.wallpaper_problems();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert_eq!(config.wallpaper_for("eDP-1", None).layout.mode, Mode::Fill);
        assert!(Config::default().wallpaper_problems().is_empty());
    }

    /// A hand-written `~/` is the home folder.
    #[test]
    fn a_home_relative_image_is_expanded() {
        let config = config("background_image = \"~/Pictures/a.png\"\n");
        let image = config.wallpaper_for("eDP-1", None).image;
        assert!(!image.starts_with('~'), "{image}");
        assert!(image.ends_with("/Pictures/a.png"), "{image}");
    }

    /// An empty image is an explicit "none", not "inherit".
    #[test]
    fn an_empty_image_overrides_to_none() {
        let config = config(
            r#"
            background_image = "/global.png"
            [background_outputs."DP-1"]
            image = ""
            "#,
        );
        assert_eq!(config.wallpaper_for("DP-1", None).image, "");
    }

    /// The writer adds, rewrites and removes one output's table, leaving
    /// the rest of the file alone.
    #[test]
    fn the_output_table_is_edited_in_place() {
        let mut doc: toml_edit::DocumentMut = "# my config\nbackground_image = \"/a.png\"\n"
            .parse()
            .unwrap();
        let record = WallpaperOverride {
            image: Some("/b.png".into()),
            mode: Some("tile".into()),
            ..Default::default()
        };
        edit_output_wallpaper(&mut doc, "DP-1", &record).unwrap();
        let text = doc.to_string();
        assert!(text.starts_with("# my config\nbackground_image = \"/a.png\"\n"));
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.background_outputs.get("DP-1"), Some(&record));

        edit_output_wallpaper(&mut doc, "DP-1", &WallpaperOverride::default()).unwrap();
        assert!(!doc.to_string().contains("background_outputs"));
    }

    #[test]
    fn an_override_round_trips_through_its_table() {
        let record = WallpaperOverride {
            image: Some("/x.svg".into()),
            color: Some("#123456".into()),
            mode: Some("center".into()),
            position_x: Some(0.25),
            position_y: Some(1.0),
            zoom: Some(2.5),
        };
        let table = record.to_table();
        assert_eq!(WallpaperOverride::from_table(&table), record);
    }
}
