//! A [`ScrollView`]'s physics driving a [`ScrollSurfaces`] band, with the
//! compositor doing the cropping.
//!
//! Where `clip_children_probe` proves the bare mechanism — a tall subsurface
//! moved behind a clipping parent — this one exercises the real component: the
//! band is repainted only when the scroll nears its edge, and every frame in
//! between is a position request and nothing else.
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-2 cargo run --release -p otto-kit --example scroll_surfaces_probe
//! ```
//!
//! Scroll over the inset box. The rows move, the thumb tracks and fades, and
//! the terminal prints one line per band repaint — nothing at all for the
//! frames in between, which is the entire point.

use std::sync::{Arc, Mutex};

use otto_kit::components::scroll::{ScrollSurfaces, ScrollView};
use otto_kit::prelude::*;
use skia_safe::{Canvas, Color, Font, FontStyle, Paint, Rect};
use smithay_client_toolkit::seat::pointer::PointerEventKind;

const WINDOW_W: f32 = 500.0;
const WINDOW_H: f32 = 360.0;
/// The fixed window onto the content.
const PANE_X: f32 = 60.0;
const PANE_Y: f32 = 60.0;
const PANE_W: f32 = 300.0;
const PANE_H: f32 = 200.0;
const ROW_H: f32 = 60.0;
const ROWS: usize = 400;

fn pane() -> Rect {
    Rect::from_xywh(PANE_X, PANE_Y, PANE_W, PANE_H)
}

struct Probe {
    window: Option<Window>,
    surfaces: Arc<Mutex<Option<ScrollSurfaces>>>,
    scroll: Arc<Mutex<ScrollView>>,
    repaints: Arc<Mutex<u32>>,
    frames: Arc<Mutex<u32>>,
}

/// Draw the rows that intersect `band`, in content coordinates.
fn draw_rows(canvas: &Canvas, band: Rect, repaints: &Arc<Mutex<u32>>) {
    let mut count = repaints.lock().unwrap();
    *count += 1;
    println!(
        "band repaint #{count}: content {:.0}..{:.0}",
        band.top, band.bottom
    );

    let typeface = skia_safe::FontMgr::new()
        .match_family_style("Inter", FontStyle::normal())
        .or_else(|| skia_safe::FontMgr::new().match_family_style("sans-serif", FontStyle::normal()))
        .expect("no font available");
    let font = Font::new(typeface, 16.0);
    let mut stripe = Paint::default();
    stripe.set_anti_alias(true);
    let mut text = Paint::default();
    text.set_anti_alias(true);
    text.set_color(Color::WHITE);

    let first = (band.top / ROW_H).floor().max(0.0) as usize;
    let last = ((band.bottom / ROW_H).ceil() as usize).min(ROWS);
    for row in first..last {
        let y = row as f32 * ROW_H;
        let shade = if row % 2 == 0 { 0x3A } else { 0x30 };
        stripe.set_color(Color::from_rgb(shade, shade, shade + 8));
        canvas.draw_rect(Rect::from_xywh(0.0, y, PANE_W, ROW_H), &stripe);
        canvas.draw_str(format!("row {row}"), (16.0, y + 36.0), &font, &text);
    }
}

impl App for Probe {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("scroll surfaces probe", WINDOW_W as i32, WINDOW_H as i32)?;
        window.on_draw(move |canvas| {
            canvas.clear(Color::from_rgb(0x1C, 0x1E, 0x22));
            // A frame where the clip box sits, so a spill would be obvious.
            let mut outline = Paint::default();
            outline.set_anti_alias(true);
            outline.set_style(skia_safe::paint::Style::Stroke);
            outline.set_stroke_width(1.0);
            outline.set_color(Color::from_rgb(0xFF, 0x9F, 0x0A));
            canvas.draw_rect(pane().with_outset((1.0, 1.0)), &outline);
        });

        let parent = window
            .surface()
            .map(|s| s.wl_surface().clone())
            .ok_or("window has no surface yet")?;

        {
            let mut scroll = self.scroll.lock().unwrap();
            scroll.set_viewport(Rect::from_wh(PANE_W, PANE_H));
            scroll.set_content_length(ROWS as f32 * ROW_H);
        }

        let surfaces = ScrollSurfaces::new(
            &parent,
            pane(),
            AppContext::scale_factor() as f32,
            Color::from_rgb(0x24, 0x26, 0x2B),
        )?;
        *self.surfaces.lock().unwrap() = Some(surfaces);

        let scroll = self.scroll.clone();
        window.on_pointer_event(move |events| {
            for event in events {
                if let PointerEventKind::Axis { vertical, .. } = &event.kind {
                    let mut scroll = scroll.lock().unwrap();
                    if vertical.stop {
                        scroll.on_wheel_end();
                    } else if vertical.discrete != 0 {
                        scroll.on_wheel_discrete(vertical.absolute as f32);
                    } else {
                        scroll.on_wheel(vertical.absolute as f32);
                    }
                }
            }
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let mut scroll = self.scroll.lock().unwrap();
        // `OTTO_PROBE_AUTOSCROLL=1` flicks the view without a touchpad, so the
        // repaint count can be checked from a script rather than by hand.
        if std::env::var_os("OTTO_PROBE_AUTOSCROLL").is_some() {
            let mut frames = self.frames.lock().unwrap();
            *frames += 1;
            if *frames < 120 {
                scroll.on_wheel(3.0);
            } else if *frames == 120 {
                scroll.on_wheel_end();
                println!("gesture released at offset {:.0}", scroll.offset());
            } else if *frames == 400 {
                println!(
                    "settled at offset {:.0} after {} frames, {} band repaints",
                    scroll.offset(),
                    *frames,
                    self.repaints.lock().unwrap()
                );
            }
        }
        if scroll.is_animating() {
            scroll.tick();
        }
        let repaints = self.repaints.clone();
        let theme = AppContext::current_theme();
        if let Some(surfaces) = self.surfaces.lock().unwrap().as_mut() {
            surfaces.sync(&scroll, &theme, |canvas, band| {
                draw_rows(canvas, band, &repaints)
            });
        }
    }

    fn idle_timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_millis(8))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    AppRunner::new(Probe {
        window: None,
        surfaces: Arc::new(Mutex::new(None)),
        scroll: Arc::new(Mutex::new(ScrollView::new(Rect::from_wh(PANE_W, PANE_H)))),
        repaints: Arc::new(Mutex::new(0)),
        frames: Arc::new(Mutex::new(0)),
    })
    .run()?;
    Ok(())
}
