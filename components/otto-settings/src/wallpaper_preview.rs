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
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

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

/// How long an answer about the primary screen is trusted: displays come
/// and go while the window is open.
const SCREEN_TTL: Duration = Duration::from_secs(30);

/// The last answer, and when the question was last asked.
static SCREEN: Mutex<(Option<Screen>, Option<Instant>)> = Mutex::new((None, None));

/// The primary screen, as the compositor last said; a laptop panel until it
/// has. Asked on a thread, at most every [`SCREEN_TTL`], waking the window
/// when the answer is in.
pub fn screen() -> Screen {
    let mut state = SCREEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let stale = state.1.is_none_or(|asked| asked.elapsed() > SCREEN_TTL);
    if stale {
        state.1 = Some(Instant::now());
        let spawned = std::thread::Builder::new()
            .name("wallpaper-screen".into())
            .spawn(|| {
                let Some(found) = ask_primary_screen() else {
                    return;
                };
                let mut state = SCREEN
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if state.0 != Some(found) {
                    state.0 = Some(found);
                    drop(state);
                    settings_client::request_redraw();
                }
            });
        if let Err(err) = spawned {
            eprintln!("wallpaper preview: could not ask for the screen ({err})");
        }
    }
    state.0.unwrap_or(FALLBACK_SCREEN)
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

/// What the preview has to show for the wallpaper at a path.
pub enum Preview {
    Ready(skia::Image),
    /// Being prepared; there is nothing of this image to show yet.
    Loading,
    /// The file cannot be shown.
    Unavailable,
}

/// One preview to render.
struct Request {
    key: String,
    thumbnail_key: String,
    path: String,
    size_px: (i32, i32),
    layout: Layout,
    fill: skia::Color4f,
}

/// The preview's work, shared between the window and the thread that does
/// it: reading and reducing a photo, and rendering the layout, are far too
/// slow for the draw path, which runs every frame.
#[derive(Default)]
struct Previews {
    /// Reduced copies by `path@modified`; `None` for a file that cannot be
    /// shown, so it is not read again every frame.
    thumbnails: HashMap<String, Option<Thumbnail>>,
    /// The last preview rendered, by its request key.
    frame: Option<(String, String, skia::Image)>,
    /// The newest request not rendered yet. A burst of them — a slider being
    /// dragged — renders the last, not every one.
    latest: Option<Request>,
    running: bool,
}

static PREVIEWS: LazyLock<Mutex<Previews>> = LazyLock::new(Default::default);

fn previews() -> std::sync::MutexGuard<'static, Previews> {
    PREVIEWS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn thumbnail_key(path: &str) -> String {
    // Keyed by when the file last changed as well as where it is: a new
    // wallpaper written over the old one is a different picture.
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    format!("{path}@{modified:?}")
}

/// The preview of the wallpaper at `path`, `size_px` pixels, laid out as
/// set now. Never blocks: what is not ready is prepared on a thread, which
/// wakes the window when it is; meanwhile the last preview of the same image
/// is shown.
pub fn frame(path: &str, size_px: (i32, i32)) -> Preview {
    let thumbnail_key = thumbnail_key(path);
    let (layout, fill) = current_style();
    let key = format!(
        "{thumbnail_key}|{size_px:?}|{layout:?}|{fill:?}|{:?}",
        screen()
    );
    let mut previews = previews();
    if let Some(None) = previews.thumbnails.get(&thumbnail_key) {
        return Preview::Unavailable;
    }
    if let Some((done, _, image)) = &previews.frame {
        if *done == key {
            return Preview::Ready(image.clone());
        }
    }
    previews.latest = Some(Request {
        key,
        thumbnail_key: thumbnail_key.clone(),
        path: path.to_string(),
        size_px,
        layout,
        fill,
    });
    if !previews.running {
        previews.running = true;
        let spawned = std::thread::Builder::new()
            .name("wallpaper-preview".into())
            .spawn(render_requests);
        if spawned.is_err() {
            previews.running = false;
        }
    }
    match &previews.frame {
        Some((_, image_key, image)) if *image_key == thumbnail_key => Preview::Ready(image.clone()),
        _ => Preview::Loading,
    }
}

/// Render requests until none is waiting.
fn render_requests() {
    loop {
        let (request, cached) = {
            let mut previews = previews();
            let Some(request) = previews.latest.take() else {
                previews.running = false;
                return;
            };
            let cached = previews.thumbnails.get(&request.thumbnail_key).cloned();
            (request, cached)
        };
        let thumbnail = cached.unwrap_or_else(|| make_thumbnail(&request.path));
        let image = thumbnail.as_ref().and_then(|thumbnail| {
            let style = preview_style(thumbnail, request.size_px, request.layout, request.fill);
            wallpaper::render_frame(&thumbnail.source, &style, request.size_px)
        });
        {
            let mut previews = previews();
            previews
                .thumbnails
                .insert(request.thumbnail_key.clone(), thumbnail);
            if let Some(image) = image {
                previews.frame = Some((request.key, request.thumbnail_key, image));
            }
        }
        settings_client::request_redraw();
    }
}

/// The position that keeps the image point under `at` in view: `at` is a
/// fraction across and down the preview. `None` where the mode has no
/// position, or the point is off the image.
pub fn position_at(path: &str, at: (f32, f32)) -> Option<(f32, f32)> {
    let (layout, _) = current_style();
    if !layout.mode.uses_position() {
        return None;
    }
    // Only once the preview has the image: a click on a picture still
    // loading has nothing to land on.
    let thumbnail = previews()
        .thumbnails
        .get(&thumbnail_key(path))
        .cloned()
        .flatten()?;
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

    /// The preview is prepared off the draw path: asking for it returns at
    /// once, and the picture is there once the thread has made it.
    #[test]
    fn the_preview_is_rendered_off_the_draw_path() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/wallpaper.jpg");
        let deadline = Instant::now() + Duration::from_secs(60);
        let image = loop {
            let asked = Instant::now();
            let preview = frame(path, (320, 200));
            assert!(
                asked.elapsed() < Duration::from_millis(250),
                "the draw path waited"
            );
            match preview {
                Preview::Ready(image) => break image,
                Preview::Loading => {}
                Preview::Unavailable => panic!("the shipped wallpaper is unavailable"),
            }
            assert!(Instant::now() < deadline, "the preview never arrived");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!((image.width(), image.height()), (320, 200));
        assert!(matches!(
            frame("/nonexistent/otto.png", (32, 20)),
            Preview::Loading | Preview::Unavailable
        ));
    }

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
