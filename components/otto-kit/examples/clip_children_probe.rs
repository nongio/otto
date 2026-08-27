//! Probe for compositor-side child clipping.
//!
//! The scrolling design we are building rests on one compositor behaviour: a
//! surface whose style node has `set_clip_children` must clip its *subsurfaces*
//! to its own bounds, so a tall buffer can be moved behind a fixed window
//! without the client repainting anything. This example is the smallest thing
//! that proves it — reading the compositor's source proves it should work; only
//! running it proves it does.
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-1 cargo run -p otto-kit --example clip_children_probe
//! ```
//!
//! What you should see: a window with a dark chrome border and, inset in it, a
//! 300x200 window onto a tall striped band. The band is 900pt tall and is moved
//! upward over time by its style position, so the stripes scroll. If child
//! clipping works the stripes only ever appear inside the inset box; if it does
//! not, they spill across the whole window (and past it) and the design needs
//! `wp_viewport` instead.

// A single-threaded probe: the Arc<Mutex<..>> state mirrors the shape a real
// otto-kit app uses, and the lay-rs values inside it are not Send.
#![allow(clippy::arc_with_non_send_sync)]

use std::sync::{Arc, Mutex};

use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1::ClipMode;
use otto_kit::surfaces::SubsurfaceSurface;
use skia_safe::{Color, Color4f, Font, FontStyle, Paint, Rect};

const WINDOW_W: f32 = 500.0;
const WINDOW_H: f32 = 360.0;
/// The fixed window onto the content — the "viewport".
const CLIP_X: f32 = 60.0;
const CLIP_Y: f32 = 60.0;
const CLIP_W: f32 = 300.0;
const CLIP_H: f32 = 200.0;
/// The band of content behind it, deliberately much taller than the window.
const BAND_H: f32 = 900.0;

struct Probe {
    window: Option<Window>,
    surfaces: Arc<Mutex<Option<(SubsurfaceSurface, SubsurfaceSurface)>>>,
    /// How far the band has been scrolled, in points.
    offset: Arc<Mutex<f32>>,
}

impl App for Probe {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("clip-children probe", WINDOW_W as i32, WINDOW_H as i32)?;
        window.on_draw(move |canvas| {
            canvas.clear(Color::from_rgb(0x1C, 0x1E, 0x22));
            // A frame around where the clip box sits, so a spill is obvious.
            let mut outline = Paint::default();
            outline.set_anti_alias(true);
            outline.set_style(skia_safe::paint::Style::Stroke);
            outline.set_stroke_width(1.0);
            outline.set_color(Color::from_rgb(0xFF, 0x9F, 0x0A));
            canvas.draw_rect(
                Rect::from_xywh(CLIP_X - 1.0, CLIP_Y - 1.0, CLIP_W + 2.0, CLIP_H + 2.0),
                &outline,
            );
        });
        let parent = window
            .surface()
            .map(|s| s.wl_surface().clone())
            .ok_or("window has no surface yet")?;

        // The clip box: a plain surface at the viewport rect that paints the
        // content's background and clips whatever moves inside it.
        let clip = SubsurfaceSurface::new(
            &parent,
            CLIP_X as i32,
            CLIP_Y as i32,
            CLIP_W as i32,
            CLIP_H as i32,
        )?;
        // Style geometry is in buffer (physical) pixels, and claiming the size
        // is what stops the compositor re-deriving position and size from the
        // surface tree on every commit — without it, the position set below is
        // overwritten the moment anything commits.
        let scale = AppContext::scale_factor() as f64;
        if let Some(style) = clip.layer() {
            style.set_size(CLIP_W as f64 * scale, CLIP_H as f64 * scale);
            style.set_position(CLIP_X as f64 * scale, CLIP_Y as f64 * scale);
            style.set_clip_children(ClipMode::Enabled);
        }
        clip.draw(|canvas| {
            canvas.clear(Color::from_rgb(0x2C, 0x2E, 0x34));
        });

        // The band: a tall buffer that will be moved behind the clip box.
        let band = SubsurfaceSurface::new(clip.wl_surface(), 0, 0, CLIP_W as i32, BAND_H as i32)?;
        if let Some(style) = band.layer() {
            style.set_size(CLIP_W as f64 * scale, BAND_H as f64 * scale);
            style.set_position(0.0, 0.0);
        }
        band.draw(|canvas| {
            canvas.clear(Color::from_rgb(0x24, 0x26, 0x2B));
            let typeface = skia_safe::FontMgr::new()
                .match_family_style("Inter", FontStyle::normal())
                .or_else(|| {
                    skia_safe::FontMgr::new().match_family_style("sans-serif", FontStyle::normal())
                })
                .expect("no font available");
            let font = Font::new(typeface, 16.0);
            let mut stripe = Paint::default();
            stripe.set_anti_alias(true);
            let mut text = Paint::default();
            text.set_anti_alias(true);
            text.set_color(Color::WHITE);
            // Numbered stripes, so it is obvious which part of the band is
            // showing and whether it moved.
            for row in 0..(BAND_H as i32 / 60) {
                let y = row as f32 * 60.0;
                let shade = if row % 2 == 0 { 0x3A } else { 0x30 };
                stripe.set_color4f(
                    Color4f::from(Color::from_rgb(shade, shade, shade + 8)),
                    None,
                );
                canvas.draw_rect(Rect::from_xywh(0.0, y, CLIP_W, 60.0), &stripe);
                canvas.draw_str(format!("row {row}"), (16.0, y + 36.0), &font, &text);
            }
        });

        *self.surfaces.lock().unwrap() = Some((clip, band));
        self.window = Some(window);
        Ok(())
    }

    /// Scroll the band by moving it, never by repainting it. Only its style
    /// position changes — no buffer is attached after the first frame.
    fn on_update(&mut self, _ctx: &AppContext) {
        let mut offset = self.offset.lock().unwrap();
        *offset = (*offset + 1.5) % (BAND_H - CLIP_H);
        if let Some((_, band)) = self.surfaces.lock().unwrap().as_ref() {
            if let Some(style) = band.layer() {
                let scale = AppContext::scale_factor() as f64;
                style.set_position(0.0, -*offset as f64 * scale);
            }
            band.wl_surface().commit();
        }
        if let Some(window) = self.window.as_ref() {
            window.request_frame();
        }
    }

    fn idle_timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_millis(16))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    AppRunner::new(Probe {
        window: None,
        surfaces: Arc::new(Mutex::new(None)),
        offset: Arc::new(Mutex::new(0.0)),
    })
    .run()?;
    Ok(())
}
