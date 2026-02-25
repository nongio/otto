use skia_safe::{Canvas, Color, Matrix, Paint, Path};
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::common::Renderable;

/// Cache for parsed SVG path data (stroke icons) - stores (path_data, viewbox_size)
static PATH_CACHE: OnceLock<HashMap<String, (String, f32)>> = OnceLock::new();

/// Cache for parsed SVG path data (filled icons) - stores (path_data, viewbox_size)
static FILLED_PATH_CACHE: OnceLock<HashMap<String, (String, f32)>> = OnceLock::new();

/// Cache for Tabler icons - stores (path_data, viewbox_size)
static TABLER_PATH_CACHE: OnceLock<HashMap<String, (String, f32)>> = OnceLock::new();

/// Get or initialize the path cache for stroke icons
fn get_path_cache() -> &'static HashMap<String, (String, f32)> {
    PATH_CACHE.get_or_init(|| {
        let mut cache = HashMap::new();

        // Load all SVG files from resources/icons directory
        let icons_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/icons");

        if let Ok(entries) = std::fs::read_dir(icons_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        if let Some(file_name) = entry.file_name().to_str() {
                            if file_name.ends_with(".svg") {
                                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                                    // Extract path data and viewbox from SVG
                                    if let Some((path_data, viewbox_size)) =
                                        extract_path_and_viewbox(&content)
                                    {
                                        let icon_name =
                                            file_name.trim_end_matches(".svg").to_string();
                                        cache.insert(icon_name, (path_data, viewbox_size));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        cache
    })
}

/// Get or initialize the path cache for filled icons
fn get_filled_path_cache() -> &'static HashMap<String, (String, f32)> {
    FILLED_PATH_CACHE.get_or_init(|| {
        let mut cache = HashMap::new();

        // Load all SVG files from resources/icons-filled directory
        let icons_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/icons-filled");

        if let Ok(entries) = std::fs::read_dir(icons_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        if let Some(file_name) = entry.file_name().to_str() {
                            if file_name.ends_with(".svg") {
                                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                                    // Extract path data and viewbox from SVG
                                    if let Some((path_data, viewbox_size)) =
                                        extract_path_and_viewbox(&content)
                                    {
                                        let icon_name =
                                            file_name.trim_end_matches(".svg").to_string();
                                        cache.insert(icon_name, (path_data, viewbox_size));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        cache
    })
}

/// Get or initialize the path cache for Tabler icons
fn get_tabler_path_cache() -> &'static HashMap<String, (String, f32)> {
    TABLER_PATH_CACHE.get_or_init(|| {
        let mut cache = HashMap::new();

        // Load all SVG files from resources/icons-tabler directory
        let icons_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/icons-tabler");

        if let Ok(entries) = std::fs::read_dir(icons_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        if let Some(file_name) = entry.file_name().to_str() {
                            if file_name.ends_with(".svg") {
                                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                                    // Extract path data and viewbox from SVG
                                    if let Some((path_data, viewbox_size)) =
                                        extract_path_and_viewbox(&content)
                                    {
                                        let icon_name =
                                            file_name.trim_end_matches(".svg").to_string();
                                        cache.insert(icon_name, (path_data, viewbox_size));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        cache
    })
}

/// Extract the 'd' attribute from SVG path elements and viewBox size
fn extract_path_and_viewbox(svg_content: &str) -> Option<(String, f32)> {
    let mut path_data = String::new();
    let mut viewbox_size = 24.0; // Default to 24x24

    // Extract viewBox to determine icon size
    if let Some(viewbox_start) = svg_content.find("viewBox=\"") {
        let viewbox_start = viewbox_start + 9;
        if let Some(viewbox_end) = svg_content[viewbox_start..].find('"') {
            let viewbox = &svg_content[viewbox_start..viewbox_start + viewbox_end];
            // viewBox format: "0 0 256 256" - we want the width (3rd value)
            let parts: Vec<&str> = viewbox.split_whitespace().collect();
            if parts.len() >= 3 {
                if let Ok(width) = parts[2].parse::<f32>() {
                    viewbox_size = width;
                }
            }
        }
    }

    // Find all path elements
    for line in svg_content.lines() {
        if let Some(d_start) = line.find(r#"d=""#) {
            let d_start = d_start + 3; // Skip 'd="'
            if let Some(d_end) = line[d_start..].find('"') {
                let d = &line[d_start..d_start + d_end];
                if !path_data.is_empty() {
                    path_data.push(' ');
                }
                path_data.push_str(d);
            }
        }
        // Handle simple circle elements and convert to path
        if let Some(_circle_start) = line.find("<circle") {
            if let Some(path) = parse_circle_to_path(line) {
                if !path_data.is_empty() {
                    path_data.push(' ');
                }
                path_data.push_str(&path);
            }
        }
    }

    if path_data.is_empty() {
        None
    } else {
        Some((path_data, viewbox_size))
    }
}

/// Convert <circle cx="..." cy="..." r="..." /> to SVG path data
fn parse_circle_to_path(line: &str) -> Option<String> {
    let cx = extract_attr(line, "cx")?;
    let cy = extract_attr(line, "cy")?;
    let r = extract_attr(line, "r")?;

    // Circle as path: M cx,cy m -r,0 a r,r 0 1,0 (r*2),0 a r,r 0 1,0 -(r*2),0
    Some(format!(
        "M {} {} m -{},0 a {},{} 0 1,0 {},0 a {},{} 0 1,0 -{},0",
        cx,
        cy,
        r,
        r,
        r,
        r * 2.0,
        r,
        r,
        r * 2.0
    ))
}

fn extract_attr(line: &str, attr: &str) -> Option<f32> {
    let pattern = format!(r#"{}=""#, attr);
    let start = line.find(&pattern)? + pattern.len();
    let end = line[start..].find('"')?;
    line[start..start + end].parse().ok()
}

/// Icon component that renders Lucide SVG icons (stroke) or Iconoir icons (filled)
pub struct Icon {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub color: Color,
    pub stroke_width: Option<f32>,
    pub filled: bool,
    icon_name: String,
}

impl Icon {
    /// Create a new icon with the given name (e.g., "heart", "circle", "check")
    pub fn new(icon_name: impl Into<String>) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            size: 24.0,
            color: Color::BLACK,
            stroke_width: None,
            filled: false,
            icon_name: icon_name.into(),
        }
    }

    /// Create a filled icon (uses Iconoir solid icons)
    pub fn filled(icon_name: impl Into<String>) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            size: 24.0,
            color: Color::BLACK,
            stroke_width: None,
            filled: true,
            icon_name: icon_name.into(),
        }
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_stroke_width(mut self, width: f32) -> Self {
        self.stroke_width = Some(width);
        self
    }

    pub fn build(self) -> Self {
        self
    }
}

impl Renderable for Icon {
    fn render(&self, canvas: &Canvas) {
        // Determine which cache to use based on prefix or filled flag
        let (cache_type, icon_name) = if let Some(stripped) = self.icon_name.strip_prefix("tabler:")
        {
            ("tabler", stripped)
        } else if self.filled {
            ("filled", self.icon_name.as_str())
        } else {
            ("lucide", self.icon_name.as_str())
        };

        // Get SVG path data and viewbox size from the appropriate cache
        let (path_data, icon_size) = match cache_type {
            "tabler" => {
                let cache = get_tabler_path_cache();
                match cache.get(icon_name) {
                    Some((data, size)) => (data, *size),
                    None => {
                        eprintln!("Tabler icon '{}' not found", icon_name);
                        return;
                    }
                }
            }
            "filled" => {
                let cache = get_filled_path_cache();
                match cache.get(icon_name) {
                    Some((data, size)) => (data, *size),
                    None => {
                        eprintln!("Filled icon '{}' not found", icon_name);
                        return;
                    }
                }
            }
            _ => {
                // Default to Lucide
                let cache = get_path_cache();
                match cache.get(icon_name) {
                    Some((data, size)) => (data, *size),
                    None => {
                        eprintln!("Icon '{}' not found in Lucide set", icon_name);
                        return;
                    }
                }
            }
        };

        // Parse SVG path data using Skia's built-in parser
        let mut path = match Path::from_svg(path_data) {
            Some(p) => p,
            None => {
                eprintln!("Failed to parse SVG path for icon '{}'", self.icon_name);
                return;
            }
        };

        // Scale from icon's native viewBox size to desired size
        let scale = self.size / icon_size;
        let mut matrix = Matrix::new_identity();
        matrix.pre_translate((self.x, self.y));
        matrix.pre_scale((scale, scale), None);
        path.transform(&matrix);

        // Draw the path
        let mut paint = Paint::default();
        paint.set_color(self.color);
        paint.set_anti_alias(true);

        if self.filled {
            paint.set_style(skia_safe::PaintStyle::Fill);
        } else {
            paint.set_style(skia_safe::PaintStyle::Stroke);
            let stroke_width = self.stroke_width.unwrap_or(1.8 * scale);
            paint.set_stroke_width(stroke_width);
            paint.set_stroke_cap(skia_safe::PaintCap::Round);
            paint.set_stroke_join(skia_safe::PaintJoin::Round);
        }

        canvas.draw_path(&path, &paint);
    }
}

/// List all available icon names
pub fn list_icons() -> Vec<String> {
    let cache = get_path_cache();
    let mut names: Vec<_> = cache.keys().cloned().collect();
    names.sort();
    names
}
