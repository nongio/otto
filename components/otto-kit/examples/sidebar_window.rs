//! A window with the file browser's chrome and nothing else in it.
//!
//! Same shape as `otto-files`: a full-height frosted sidebar with the window
//! controls at its top, a tall header beside it carrying a large title and a
//! subtitle, and an opaque content sheet under both. The content area here is
//! a placeholder — this example is about the chrome.
//!
//! Geometry lives in the `*_rect` / `*_at` helpers so drawing and hit-testing
//! read the same numbers, which is the otto-kit convention.
//!
//! ```sh
//! cargo run -p otto-kit --example sidebar_window
//! ```

use std::sync::{Arc, Mutex};

use otto_kit::components::source_list::{self, SourceListItem, SourceListLayout};
use otto_kit::components::titlebar::{WindowControl, WindowControls};
use otto_kit::components::window::resize;
use otto_kit::prelude::*;
use skia_safe::{Contains, Paint, Point};
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;

const WINDOW_W: f32 = 900.0;
const WINDOW_H: f32 = 600.0;
const MIN_W: f32 = 560.0;
const MIN_H: f32 = 360.0;
const CORNER: f32 = 12.0;

/// Full-height sidebar, like Finder's — the header sits beside it, not above.
const SIDEBAR_W: f32 = 232.0;
/// Tall enough for a large title with a subtitle under it, which is what makes
/// the window read as a document rather than a dialog.
const HEADER_H: f32 = 92.0;
const CONTENT_PAD: f32 = 20.0;
const CONTROLS_INSET: f32 = 18.0;
/// Optical centres of the header's two text lines, within `HEADER_H`.
const TITLE_CY: f32 = 40.0;
const SUBTITLE_CY: f32 = 66.0;
/// Top of the first sidebar row, under the "Places" caption.
const LIST_TOP: f32 = 68.0;

struct Demo {
    size: (f32, f32),
    items: Vec<SourceListItem>,
    selected: usize,
}

impl Demo {
    fn new() -> Self {
        Self {
            size: (WINDOW_W, WINDOW_H),
            items: [
                "Overview",
                "Documents",
                "Downloads",
                "Pictures",
                "Music",
                "Projects",
            ]
            .into_iter()
            .map(SourceListItem::new)
            .collect(),
            selected: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Rows of the sidebar, laid out once and shared by drawing and hit-testing.
fn list_layout(count: usize) -> SourceListLayout {
    SourceListLayout::compute(count, 0.0, LIST_TOP, SIDEBAR_W)
}

/// The area right of the sidebar and below the header.
fn content_viewport(width: f32, height: f32) -> Rect {
    Rect::from_ltrb(SIDEBAR_W, HEADER_H, width, height)
}

/// Anywhere in the header band that is not a window control drags the window.
/// The sidebar's rows start below the band, so they never compete with it.
fn is_drag_area(x: f32, y: f32, width: f32) -> bool {
    if y > HEADER_H || x > width {
        return false;
    }
    !Rect::from_xywh(CONTROLS_INSET - 4.0, CONTROLS_INSET - 4.0, 70.0, 20.0)
        .contains(Point::new(x, y))
}

fn control_at(x: f32, y: f32) -> Option<WindowControl> {
    const STEP: f32 = 20.0;
    const R: f32 = 6.0;
    [
        WindowControl::Close,
        WindowControl::Minimize,
        WindowControl::Zoom,
    ]
    .into_iter()
    .enumerate()
    .find(|(i, _)| {
        let cx = CONTROLS_INSET + R + *i as f32 * STEP;
        let cy = CONTROLS_INSET + R;
        (x - cx).powi(2) + (y - cy).powi(2) <= (R + 3.0).powi(2)
    })
    .map(|(_, control)| control)
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw(canvas: &Canvas, demo: &Demo, theme: &Theme) {
    let height = demo.size.1;

    // Nothing paints the ground here. The frost is the compositor's layer,
    // sitting under this buffer with the material colour on it — see
    // `apply_material`; the buffer carries the content alone. It is cleared,
    // not filled: each frame arrives in a fresh, uninitialised buffer, and
    // clearing to anything but transparent would cover the blur.
    canvas.clear(Color::TRANSPARENT);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    draw_sidebar(canvas, demo, theme);
    draw_header(canvas, demo, theme);
    draw_content(canvas, demo, theme);

    // Hairline between the sidebar and the sheet, full height.
    paint.set_color(theme.fill_tertiary);
    paint.set_stroke_width(1.0);
    canvas.draw_line(
        Point::new(SIDEBAR_W, 0.0),
        Point::new(SIDEBAR_W, height),
        &paint,
    );
}

fn draw_sidebar(canvas: &Canvas, demo: &Demo, theme: &Theme) {
    // A thin tint over the frost, not a ground: enough to set the sidebar apart
    // from the content side while the blur still reads through it.
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::from_argb(0x1A, 0x00, 0x00, 0x00));
    canvas.draw_rect(Rect::from_ltrb(0.0, 0.0, SIDEBAR_W, demo.size.1), &paint);

    WindowControls::new()
        .at(CONTROLS_INSET, CONTROLS_INSET)
        .render(canvas);

    Label::new("Places")
        .with_style(styles::CAPTION_1_EMPHASIZED)
        .with_color(theme.text_tertiary)
        .centered_on(20.0, 54.0)
        .render(canvas);

    let layout = list_layout(demo.items.len());
    source_list::draw(
        canvas,
        &layout,
        &demo.items,
        Some(demo.selected),
        theme,
        |canvas, _index, rect, tint| {
            // A folder-ish glyph, drawn rather than themed so the example needs
            // no icon theme installed.
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(tint);
            let body = Rect::from_ltrb(
                rect.left + 1.0,
                rect.top + 5.0,
                rect.right - 1.0,
                rect.bottom - 3.0,
            );
            canvas.draw_rrect(skia_safe::RRect::new_rect_xy(body, 3.0, 3.0), &paint);
            let tab = Rect::from_xywh(rect.left + 1.0, rect.top + 2.0, 7.0, 4.0);
            canvas.draw_rrect(skia_safe::RRect::new_rect_xy(tab, 1.5, 1.5), &paint);
        },
    );
}

fn draw_header(canvas: &Canvas, demo: &Demo, theme: &Theme) {
    let width = demo.size.0;
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    // The header shares the content's ground rather than tinting itself: the
    // hairline under it is what separates the two.
    Label::new(&demo.items[demo.selected].label)
        .with_style(styles::TITLE_1_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_on(SIDEBAR_W + CONTENT_PAD, TITLE_CY)
        .render(canvas);

    Label::new("A window with the file browser's header and sidebar")
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_secondary)
        .centered_on(SIDEBAR_W + CONTENT_PAD, SUBTITLE_CY)
        .render(canvas);

    paint.set_color(theme.fill_tertiary);
    paint.set_stroke_width(1.0);
    canvas.draw_line(
        Point::new(SIDEBAR_W, HEADER_H),
        Point::new(width, HEADER_H),
        &paint,
    );
}

/// Placeholder: whatever an app puts here is its own business.
fn draw_content(canvas: &Canvas, demo: &Demo, theme: &Theme) {
    let area = content_viewport(demo.size.0, demo.size.1);
    Label::new("Content goes here")
        .with_style(styles::TITLE_3)
        .with_color(theme.text_tertiary)
        .centered_on(area.center_x(), area.center_y())
        .with_align(TextAlign::Center)
        .render(canvas);
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct SidebarWindow {
    window: Option<Window>,
    state: Arc<Mutex<Demo>>,
}

impl App for SidebarWindow {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Sidebar Window", WINDOW_W as i32, WINDOW_H as i32)?;
        window.set_min_size(MIN_W as u32, MIN_H as u32);
        // The window paints its own translucent body; an opaque backing colour
        // would defeat the blur before the first draw.
        window.set_background(Color::TRANSPARENT);

        apply_material(&window);

        let state = Arc::clone(&self.state);
        window.on_draw(move |canvas| {
            let demo = state.lock().unwrap();
            let theme = AppContext::current_theme();
            draw(canvas, &demo, &theme);
        });

        self.install_pointer(&window);
        AppContext::register_window(window.clone());
        self.window = Some(window);
        Ok(())
    }

    fn on_configure(&mut self, _ctx: &AppContext, configure: WindowConfigure, _serial: u32) {
        if let (Some(w), Some(h)) = (configure.new_size.0, configure.new_size.1) {
            self.state.lock().unwrap().size = (w.get() as f32, h.get() as f32);
        }
        if let Some(window) = &self.window {
            window.request_frame();
        }
    }
}

/// Ask the compositor for the window's material: the frost, its colour, and the
/// shape it is cut to.
///
/// A blur needs the pixels *behind* the surface, and the only process that has
/// them is the compositor — so none of this can be drawn client-side. The
/// colour goes on the compositor's own layer rather than into the buffer:
/// `BackgroundBlur` blurs what is behind that layer and tints the result with
/// this colour, and a ground painted into the buffer would simply cover it.
/// That is why [`draw`] clears to transparent.
fn apply_material(window: &Window) {
    use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode};

    let Some(style) = window.surface_style() else {
        eprintln!("sidebar_window: no otto-surface-style — the window will not be frosted");
        return;
    };

    let colour = skia_safe::Color4f::from(AppContext::current_theme().material_medium);
    style.set_background_color(
        colour.r as f64,
        colour.g as f64,
        colour.b as f64,
        colour.a as f64,
    );
    style.set_blend_mode(BlendMode::BackgroundBlur);
    style.set_corner_radius(CORNER as f64);
    style.set_masks_to_bounds(ClipMode::Enabled);
}

impl SidebarWindow {
    fn install_pointer(&self, window: &Window) {
        let state = Arc::clone(&self.state);
        let window_for_events = window.clone();

        window.on_pointer_event(move |events| {
            for event in events {
                let (x, y) = (event.position.0 as f32, event.position.1 as f32);
                let PointerEventKind::Press { serial, .. } = event.kind else {
                    continue;
                };
                let mut demo = state.lock().unwrap();
                let (width, height) = demo.size;

                if let Some(edge) = resize::edge_at(Rect::from_wh(width, height), x, y) {
                    if let Some(seat) = AppContext::seat_state().seats().next() {
                        window_for_events.start_resize(&seat, serial, edge);
                    }
                    return;
                }

                if let Some(control) = control_at(x, y) {
                    match control {
                        WindowControl::Close => std::process::exit(0),
                        WindowControl::Minimize | WindowControl::Zoom => {}
                    }
                    return;
                }

                if is_drag_area(x, y, width) {
                    if let Some(seat) = AppContext::seat_state().seats().next() {
                        window_for_events.start_move(&seat, serial);
                    }
                    return;
                }

                if let Some(index) = list_layout(demo.items.len()).item_at(x, y) {
                    demo.selected = index;
                }
            }
            window_for_events.request_frame();
        });
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Runtime::new()?.block_on(async {
        AppRunner::new(SidebarWindow {
            window: None,
            state: Arc::new(Mutex::new(Demo::new())),
        })
        .run()
    })
}
