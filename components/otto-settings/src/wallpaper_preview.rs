//! The background image row's preview: the primary screen, showing the
//! wallpaper as the desktop lays it out.
//!
//! The preview is drawn by the compositor's own code, `otto_kit::wallpaper`,
//! from a reduced copy of the image, in the shape of the primary screen and
//! with the mode, position, zoom and colour currently set — so choosing
//! *Fit* shows the bars, and *Centre* shows the image at the size it will
//! really have on that screen. Clicking the picture sets the position to the
//! point clicked, which then stays in view however the screen crops.

use std::collections::HashMap;
use std::sync::{Mutex, Once};

use otto_kit::wallpaper::{self, FrameStyle, Layout, Mode, Source};
use skia_safe as skia;

use crate::settings_client::{self, Value};

/// The setting the preview belongs to.
pub const IMAGE_ID: &str = "background_image";

/// The longest edge the reduced copy of a wallpaper is kept at: enough to
/// zoom into in a preview a few hundred pixels wide.
const THUMBNAIL_EDGE: i32 = 1024;

/// What the preview stands in for when the compositor does not say: a
/// common laptop panel.
const FALLBACK_SCREEN: Screen = Screen {
    size_px: (2560, 1600),
    scale: 1.0,
};

/// The screen the preview shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    /// Its mode, in physical pixels.
    pub size_px: (i32, i32),
    /// Physical pixels per point.
    pub scale: f32,
}

static SCREEN: Mutex<Option<Screen>> = Mutex::new(None);
static ASK_SCREEN: Once = Once::new();

/// The primary screen, once the compositor has said which it is; a laptop
/// panel until then. The first call asks, on a thread, and wakes the window
/// when the answer is in.
pub fn screen() -> Screen {
    ASK_SCREEN.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("wallpaper-screen".into())
            .spawn(|| {
                let Some(found) = ask_primary_screen() else {
                    return;
                };
                if let Ok(mut screen) = SCREEN.lock() {
                    *screen = Some(found);
                }
                settings_client::request_redraw();
            });
        if let Err(err) = spawned {
            eprintln!("wallpaper preview: could not ask for the screen ({err})");
        }
    });
    SCREEN
        .lock()
        .ok()
        .and_then(|screen| *screen)
        .unwrap_or(FALLBACK_SCREEN)
}

fn ask_primary_screen() -> Option<Screen> {
    let connection = zbus::blocking::Connection::session().ok()?;
    let proxy = otto_kit::dbus::shell::ShellProxyBlocking::new(&connection).ok()?;
    primary_screen(&proxy.get_outputs().ok()?)
}

/// The primary output of a `GetOutputs` answer, or the first one.
fn primary_screen(json: &str) -> Option<Screen> {
    let outputs: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let output = outputs
        .iter()
        .find(|o| o["primary"] == serde_json::Value::Bool(true))
        .or_else(|| outputs.first())?;
    let width = output["current_mode"]["width"].as_i64()? as i32;
    let height = output["current_mode"]["height"].as_i64()? as i32;
    let scale = output["scale"].as_f64().unwrap_or(1.0) as f32;
    (width > 0 && height > 0 && scale > 0.0).then_some(Screen {
        size_px: (width, height),
        scale,
    })
}

/// The preview's size in points: the screen's shape, as tall as `max_h`
/// unless that would be wider than `max_w`.
pub fn box_size(max_w: f32, max_h: f32) -> (f32, f32) {
    let (w, h) = screen().size_px;
    let aspect = w as f32 / h as f32;
    if max_h * aspect <= max_w {
        (max_h * aspect, max_h)
    } else {
        (max_w, max_w / aspect)
    }
}

/// The layout and colour set now, as the compositor serves them.
pub fn current_style() -> (Layout, skia::Color4f) {
    let text = |id: &str| match settings_client::value(id) {
        Some(Value::Text(text)) => Some(text),
        _ => None,
    };
    let number = |id: &str, default: f32| {
        settings_client::value(id)
            .and_then(|v| v.as_f32())
            .unwrap_or(default)
    };
    let layout = Layout {
        mode: text("background_mode")
            .and_then(|mode| Mode::parse(&mode))
            .unwrap_or_default(),
        position: (
            number("background_position_x", 0.5),
            number("background_position_y", 0.5),
        ),
        zoom: number("background_zoom", 1.0),
    }
    .sanitised();
    let colour = text("background_color")
        .and_then(|hex| otto_kit::color::parse_hex(&hex))
        .map(skia::Color4f::from)
        .unwrap_or(skia::Color4f::new(0.1, 0.1, 0.18, 1.0));
    (layout, colour)
}

/// A wallpaper as the preview keeps it: a reduced copy, `ratio` the size of
/// the original.
#[derive(Clone)]
struct Thumbnail {
    source: Source,
    ratio: f32,
}

/// The wallpaper at `path`, opened and reduced once per version of the
/// file. Failures are kept too, so a missing file is not read every frame.
fn thumbnail(path: &str) -> Option<Thumbnail> {
    thread_local! {
        static CACHE: std::cell::RefCell<HashMap<String, Option<Thumbnail>>> =
            std::cell::RefCell::new(HashMap::new());
    }
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let key = format!("{path}@{modified:?}");
    CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(&key) {
            return cached.clone();
        }
        let made = make_thumbnail(path);
        cache.borrow_mut().insert(key, made.clone());
        made
    })
}

fn make_thumbnail(path: &str) -> Option<Thumbnail> {
    match wallpaper::open(path).ok()? {
        Source::Raster(image) => {
            let (w, h) = (image.width(), image.height());
            let ratio = (THUMBNAIL_EDGE as f32 / w.max(h) as f32).min(1.0);
            let reduced = if ratio < 1.0 {
                let size = (
                    ((w as f32 * ratio).round() as i32).max(1),
                    ((h as f32 * ratio).round() as i32).max(1),
                );
                otto_kit::utils::resample::resample_to(&image, size)?
            } else {
                image.make_raster_image(None, None)?
            };
            Some(Thumbnail {
                ratio: reduced.width() as f32 / w as f32,
                source: Source::Raster(reduced),
            })
        }
        // Vector art needs no reducing; its natural size is its own.
        svg @ Source::Svg(_) => Some(Thumbnail {
            source: svg,
            ratio: 1.0,
        }),
    }
}

/// The frame style that draws `thumbnail` on a `size_px` preview the way
/// the screen will show the original.
fn preview_style(
    thumbnail: &Thumbnail,
    size_px: (i32, i32),
    layout: Layout,
    fill: skia::Color4f,
) -> FrameStyle {
    let screen = screen();
    // Preview pixels per screen pixel, and per thumbnail pixel of the
    // original: what the natural-size modes need to come out the size they
    // will be.
    let per_screen_px = size_px.0 as f32 / screen.size_px.0 as f32;
    FrameStyle {
        layout,
        fill,
        output_scale: screen.scale * per_screen_px / thumbnail.ratio,
        // A spanned wallpaper is previewed on the one screen.
        span: None,
    }
}

/// The preview of the wallpaper at `path`, `size_px` pixels, laid out as
/// set now. Rendered once per change of anything it shows.
pub fn frame(path: &str, size_px: (i32, i32)) -> Option<skia::Image> {
    thread_local! {
        static LAST: std::cell::RefCell<Option<(String, skia::Image)>> =
            const { std::cell::RefCell::new(None) };
    }
    let thumbnail = thumbnail(path)?;
    let (layout, fill) = current_style();
    let style = preview_style(&thumbnail, size_px, layout, fill);
    let key = format!("{path}|{size_px:?}|{style:?}");
    LAST.with(|last| {
        if let Some((cached, image)) = last.borrow().as_ref() {
            if *cached == key {
                return Some(image.clone());
            }
        }
        let image = wallpaper::render_frame(&thumbnail.source, &style, size_px)?;
        *last.borrow_mut() = Some((key, image.clone()));
        Some(image)
    })
}

/// The position that keeps the image point under `at` in view: `at` is a
/// fraction across and down the preview. `None` where the mode has no
/// position, or the point is off the image.
pub fn position_at(path: &str, at: (f32, f32)) -> Option<(f32, f32)> {
    let (layout, _) = current_style();
    if !layout.mode.uses_position() {
        return None;
    }
    let thumbnail = thumbnail(path)?;
    // Any preview size will do: the answer is a fraction of the image.
    let size_px = (
        1000,
        (1000.0 * screen().size_px.1 as f32 / screen().size_px.0 as f32) as i32,
    );
    let style = preview_style(
        &thumbnail,
        size_px,
        layout,
        skia::Color4f::new(0.0, 0.0, 0.0, 1.0),
    );
    let image = thumbnail.source.size()?;
    let dest = wallpaper::arrange(layout, image, size_px, style.output_scale, None).dest;
    let x = (at.0 * size_px.0 as f32 - dest.left) / dest.width();
    let y = (at.1 * size_px.1 as f32 - dest.top) / dest.height();
    ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)).then_some((x, y))
}

/// Set the position to `position`, both halves.
pub fn set_position(position: (f32, f32)) {
    // Two settings, so two writes; on the bus thread, as every write is.
    let round = |v: f32| (v as f64 * 100.0).round() / 100.0;
    crate::apply("background_position_x", Value::Double(round(position.0)));
    crate::apply("background_position_y", Value::Double(round(position.1)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_primary_output_is_the_screen() {
        let json = r#"[
            {"name": "eDP-1", "primary": false, "scale": 2.0,
             "current_mode": {"width": 2880, "height": 1800, "refresh": 60000}},
            {"name": "DP-1", "primary": true, "scale": 1.0,
             "current_mode": {"width": 3440, "height": 1440, "refresh": 100000}}
        ]"#;
        assert_eq!(
            primary_screen(json),
            Some(Screen {
                size_px: (3440, 1440),
                scale: 1.0
            })
        );
        // No primary: the first.
        let json = r#"[{"name": "eDP-1", "scale": 1.5,
            "current_mode": {"width": 1920, "height": 1200}}]"#;
        assert_eq!(primary_screen(json).unwrap().size_px, (1920, 1200));
        // An older compositor without modes: nothing to go on.
        assert_eq!(primary_screen(r#"[{"name": "eDP-1"}]"#), None);
    }
}
