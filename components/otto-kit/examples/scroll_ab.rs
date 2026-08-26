//! The same scroll, two mechanisms, one number that separates them.
//!
//! Both modes show identical content, run the identical [`ScrollView`]
//! physics and are driven by the identical scripted gesture. The only
//! difference is where the scrolling happens:
//!
//! - `canvas`   — the pane is painted into the window's own buffer every
//!   frame, the way an ordinary immediate-mode widget is. Each frame is a
//!   full window repaint, and otto-kit damages the whole buffer on commit
//!   (`rendering/surface.rs`), so the compositor is told the entire window
//!   changed.
//! - `surfaces` — the pane is a [`ScrollSurfaces`] band in its own
//!   subsurface, moved with `otto_surface_style_v1` and clipped by the parent.
//!   A frame of scrolling is a position request: no paint, no buffer, no
//!   damage. The client only draws when the scroll nears the edge of the
//!   painted band.
//!
//! The headline metric is **window repaints per scroll**, printed at the end.
//! It is a client-side count, but it maps directly onto compositor work:
//! every window repaint is one `damage_buffer(0, 0, w, h)` and one full-window
//! recomposite. What this example cannot measure is what that costs Otto —
//! for that, watch the compositor with the plane dump while this runs.
//!
//! Motion is advanced on the compositor's **frame callback** by default, so
//! one tick of physics lands on one presented frame. Set `SCROLL_AB_TIMER=1`
//! to fall back to the old free-running 8 ms timer and feel the difference —
//! that path advances the scroll on a clock that beats against the refresh,
//! which reads as judder even though the integration is correct in time.
//!
//! ```sh
//! cargo run --release -p otto-kit --example scroll_ab -- canvas
//! cargo run --release -p otto-kit --example scroll_ab -- surfaces
//!
//! # scripted, for a comparison that does not depend on a hand:
//! SCROLL_AB_AUTO=1 cargo run --release -p otto-kit --example scroll_ab -- canvas
//! SCROLL_AB_AUTO=1 cargo run --release -p otto-kit --example scroll_ab -- surfaces
//! ```

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use otto_kit::components::scroll::{ScrollSurfaces, ScrollView};
use otto_kit::prelude::*;
use skia_safe::{Canvas, Color, Font, FontStyle, Paint, Rect};
use smithay_client_toolkit::seat::pointer::PointerEventKind;

const WINDOW_W: f32 = 460.0;
const WINDOW_H: f32 = 380.0;
const PANE_X: f32 = 60.0;
const PANE_Y: f32 = 70.0;
const PANE_W: f32 = 320.0;
const PANE_H: f32 = 240.0;
const ROW_H: f32 = 40.0;
const ROWS: usize = 4000;

/// Frames of scripted gesture, then release, then time to settle.
const AUTO_GESTURE: u32 = 120;
const AUTO_END: u32 = 420;

fn pane() -> Rect {
    Rect::from_xywh(PANE_X, PANE_Y, PANE_W, PANE_H)
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Canvas,
    /// The pane is a subsurface, but the client still does everything inside
    /// it: it repaints the band every frame, translating and clipping in its
    /// own buffer. Isolates one variable — the damage is now the pane's, not
    /// the window's, while the per-frame repaint is unchanged.
    ClientSub,
    Surfaces,
}

impl Mode {
    fn from_args() -> Self {
        match std::env::args().nth(1).as_deref() {
            Some("surfaces") => Mode::Surfaces,
            Some("client-sub") => Mode::ClientSub,
            Some("canvas") | None => Mode::Canvas,
            Some(other) => {
                eprintln!("unknown mode {other:?}; expected `canvas`, `client-sub` or `surfaces`");
                std::process::exit(2);
            }
        }
    }

    fn label(self) -> &'static str {
        match self {
            Mode::Canvas => "canvas (painted into the window every frame)",
            Mode::ClientSub => "client-sub (subsurface, client repaints/translates/clips it)",
            Mode::Surfaces => "surfaces (subsurface band moved by the compositor)",
        }
    }
}

/// Counters shared with the draw closures, which the runner owns.
#[derive(Default)]
struct Stats {
    /// `on_draw` invocations — one full-window repaint and one full-buffer
    /// damage each.
    window_repaints: AtomicU32,
    /// Band repaints, in `surfaces` mode.
    band_repaints: AtomicU32,
    /// Pane-subsurface repaints, in `client-sub` mode. Each is an attach plus
    /// a damage of the pane, not of the window.
    pane_repaints: AtomicU32,
    /// Nanoseconds spent inside painting, either kind.
    paint_ns: AtomicU64,
    /// Rows drawn, so the two modes can be shown to be doing the same work
    /// when they do paint.
    rows_drawn: AtomicU64,
    /// Offset deltas between consecutive *presented* frames, in milli-points.
    /// Smooth motion means these are near-constant; judder is variance here,
    /// however even the physics looks against a wall clock.
    step_um: Mutex<Vec<i64>>,
    last_offset: Mutex<Option<f32>>,
    /// Microseconds between consecutive presented frames.
    frame_dt_us: Mutex<Vec<i64>>,
    last_frame_at: Mutex<Option<Instant>>,
    reported: AtomicBool,
}

thread_local! {
    /// Matched once. A `FontMgr` lookup costs tens of milliseconds — leaving
    /// it inside the draw would swamp the very thing being measured.
    static FONT: Font = {
        let typeface = skia_safe::FontMgr::new()
            .match_family_style("Inter", FontStyle::normal())
            .or_else(|| {
                skia_safe::FontMgr::new().match_family_style("sans-serif", FontStyle::normal())
            })
            .expect("no font available");
        Font::new(typeface, 15.0)
    };
}

/// Draw the rows intersecting `band`, in content coordinates. Identical in
/// both modes — the mechanism differs, the pixels do not.
fn draw_rows(canvas: &Canvas, band: Rect, stats: &Stats) {
    let start = Instant::now();
    let font = FONT.with(|f| f.clone());
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
        canvas.draw_str(format!("row {row}"), (16.0, y + 26.0), &font, &text);
    }
    stats
        .rows_drawn
        .fetch_add(last.saturating_sub(first) as u64, Ordering::Relaxed);
    stats
        .paint_ns
        .fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

struct Ab {
    mode: Mode,
    window: Option<Window>,
    surfaces: Arc<Mutex<Option<ScrollSurfaces>>>,
    /// The plain subsurface used by `client-sub`.
    pane_sub: Arc<Mutex<Option<otto_kit::surfaces::SubsurfaceSurface>>>,
    /// Shared with the frame-callback driver.
    window_handle: Arc<Mutex<Option<Window>>>,
    last_tick: Arc<Mutex<Option<Instant>>>,
    auto_frames: Arc<Mutex<u32>>,
    /// The band subsurface's id. Pointer events over the pane arrive on
    /// *that* surface, not on the toplevel — `Window::on_pointer_event`
    /// filters to the toplevel alone, so a surface-backed pane has to watch
    /// the raw stream and accept both.
    band_id: Arc<Mutex<Option<wayland_client::backend::ObjectId>>>,
    scroll: Arc<Mutex<ScrollView>>,
    stats: Arc<Stats>,
    frames: u32,
}

impl Ab {
    fn report(&self) {
        let s = &self.stats;
        if s.reported.swap(true, Ordering::Relaxed) {
            return;
        }
        let repaints = s.window_repaints.load(Ordering::Relaxed);
        let bands = s.band_repaints.load(Ordering::Relaxed);
        let paint_us = s.paint_ns.load(Ordering::Relaxed) as f64 / 1000.0;
        println!("\n=== scroll_ab: {} ===", self.mode.label());
        println!("  frames ticked          {}", self.frames);
        println!("  WINDOW REPAINTS        {repaints}   <- one full-buffer damage each");
        println!("  band repaints          {bands}");
        println!(
            "  pane repaints          {}",
            s.pane_repaints.load(Ordering::Relaxed)
        );
        println!(
            "  rows drawn             {}",
            s.rows_drawn.load(Ordering::Relaxed)
        );
        println!("  time spent painting    {paint_us:.0} µs");
        println!(
            "  final offset           {:.0}",
            self.scroll.lock().unwrap().offset()
        );
        let steps = s.step_um.lock().unwrap();
        let moving: Vec<f64> = steps
            .iter()
            .map(|&u| u as f64 / 1000.0)
            .filter(|d| d.abs() > 0.01)
            .collect();
        if moving.len() > 2 {
            let n = moving.len() as f64;
            let mean = moving.iter().sum::<f64>() / n;
            let var = moving.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n;
            let sd = var.sqrt();
            let min = moving.iter().cloned().fold(f64::MAX, f64::min);
            let max = moving.iter().cloned().fold(f64::MIN, f64::max);
            println!(
                "  --- per-presented-frame motion ({} frames) ---",
                moving.len()
            );
            println!("  step mean              {mean:.2} pt");
            println!(
                "  step stddev            {sd:.2} pt   ({:.0}% of mean)",
                sd / mean.abs() * 100.0
            );
            println!("  step min/max           {min:.2} / {max:.2} pt");
            let dts = s.frame_dt_us.lock().unwrap();
            let ms: Vec<f64> = dts.iter().map(|&u| u as f64 / 1000.0).collect();
            if ms.len() > 2 {
                let n = ms.len() as f64;
                let mean_dt = ms.iter().sum::<f64>() / n;
                let sd_dt = (ms.iter().map(|d| (d - mean_dt).powi(2)).sum::<f64>() / n).sqrt();
                let min_dt = ms.iter().cloned().fold(f64::MAX, f64::min);
                let max_dt = ms.iter().cloned().fold(f64::MIN, f64::max);
                println!(
                    "  frame interval mean    {mean_dt:.2} ms  ({:.0} Hz)",
                    1000.0 / mean_dt
                );
                println!("  frame interval stddev  {sd_dt:.2} ms");
                println!("  frame interval min/max {min_dt:.2} / {max_dt:.2} ms");
                // How often `ScrollView::advance` truncated its own timestep,
                // and roughly how much travel that cost. A frame longer than
                // the clamp advances less than the elapsed time says it
                // should, so the fling quietly falls behind itself.
                const CLAMP_MS: f64 = 1000.0 / 20.0;
                let over: Vec<(usize, f64)> = ms
                    .iter()
                    .enumerate()
                    .filter(|(_, &d)| d > CLAMP_MS)
                    .map(|(i, &d)| (i, d))
                    .collect();
                let lost_pt: f64 = over
                    .iter()
                    .filter_map(|&(i, d)| {
                        moving.get(i).map(|step| {
                            let v = step / (CLAMP_MS / 1000.0);
                            v * (d - CLAMP_MS) / 1000.0
                        })
                    })
                    .sum();
                println!(
                    "  frames over {CLAMP_MS:.0}ms clamp {}/{}  ({:.0}%)",
                    over.len(),
                    ms.len(),
                    over.len() as f64 / ms.len() as f64 * 100.0
                );
                println!("  travel lost to clamp   {lost_pt:.0} pt");
            }
        }
        println!();
    }
}

/// Everything one frame of scrolling does, independent of what drove it.
///
/// `dt` is the real interval since the last time this ran. Passing it in
/// rather than letting `ScrollView::tick` read the wall clock is the point of
/// frame sync: the physics is advanced by exactly the time the display is
/// about to show, once per presented frame.
struct Driver {
    mode: Mode,
    scroll: Arc<Mutex<ScrollView>>,
    surfaces: Arc<Mutex<Option<ScrollSurfaces>>>,
    pane_sub: Arc<Mutex<Option<otto_kit::surfaces::SubsurfaceSurface>>>,
    stats: Arc<Stats>,
    window: Arc<Mutex<Option<Window>>>,
    last_tick: Arc<Mutex<Option<Instant>>>,
    auto_frames: Arc<Mutex<u32>>,
}

impl Driver {
    fn step(&self) {
        let auto = std::env::var_os("SCROLL_AB_AUTO").is_some();
        let now = Instant::now();
        let dt = {
            let mut last = self.last_tick.lock().unwrap();
            let dt = last.map(|t| now.duration_since(t).as_secs_f32());
            *last = Some(now);
            dt
        };

        let mut scroll = self.scroll.lock().unwrap();
        if auto {
            let mut frames = self.auto_frames.lock().unwrap();
            *frames += 1;
            if *frames <= AUTO_GESTURE {
                scroll.on_wheel(3.0);
            } else if *frames == AUTO_GESTURE + 1 {
                scroll.on_wheel_end();
            }
        }

        if scroll.is_animating() {
            // One presented frame, one advance, by the interval actually
            // elapsed. `tick()` would re-read the clock at a moment that has
            // nothing to do with when this frame reaches the screen.
            match dt {
                Some(dt) => {
                    scroll.advance(dt);
                }
                None => {
                    scroll.tick();
                }
            }
        }
        let animating = scroll.is_animating();

        match self.mode {
            Mode::Surfaces => {
                let stats = self.stats.clone();
                let theme = AppContext::current_theme();
                if let Some(surfaces) = self.surfaces.lock().unwrap().as_mut() {
                    surfaces.sync(&scroll, &theme, |canvas, band| {
                        stats.band_repaints.fetch_add(1, Ordering::Relaxed);
                        draw_rows(canvas, band, &stats);
                    });
                }
            }
            Mode::ClientSub => {
                if animating {
                    let stats = self.stats.clone();
                    let offset = scroll.offset();
                    if let Some(sub) = self.pane_sub.lock().unwrap().as_ref() {
                        stats.pane_repaints.fetch_add(1, Ordering::Relaxed);
                        sub.draw(|canvas| {
                            canvas.clear(Color::from_rgb(0x24, 0x26, 0x2B));
                            canvas.save();
                            canvas.translate((0.0, -offset));
                            draw_rows(canvas, Rect::from_xywh(0.0, offset, PANE_W, PANE_H), &stats);
                            canvas.restore();
                        });
                    }
                }
            }
            Mode::Canvas => {
                if animating {
                    if let Some(window) = self.window.lock().unwrap().as_ref() {
                        window.request_frame();
                    }
                }
            }
        }
    }
}

impl App for Ab {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mode = self.mode;
        println!("scroll_ab mode: {}", mode.label());

        let mut window = Window::new("scroll a/b", WINDOW_W as i32, WINDOW_H as i32)?;

        let stats = self.stats.clone();
        let scroll_for_draw = self.scroll.clone();
        window.on_draw(move |canvas| {
            stats.window_repaints.fetch_add(1, Ordering::Relaxed);
            {
                // Sampled here because `on_draw` is a frame that actually
                // reaches the screen — the tick rate is not what the eye sees.
                let offset = scroll_for_draw.lock().unwrap().offset();
                let mut last = stats.last_offset.lock().unwrap();
                if let Some(prev) = *last {
                    stats
                        .step_um
                        .lock()
                        .unwrap()
                        .push(((offset - prev) * 1000.0) as i64);
                }
                *last = Some(offset);
                let now = Instant::now();
                let mut prev_at = stats.last_frame_at.lock().unwrap();
                if let Some(t) = *prev_at {
                    stats
                        .frame_dt_us
                        .lock()
                        .unwrap()
                        .push(now.duration_since(t).as_micros() as i64);
                }
                *prev_at = Some(now);
            }
            canvas.clear(Color::from_rgb(0x1C, 0x1E, 0x22));

            let mut outline = Paint::default();
            outline.set_anti_alias(true);
            outline.set_style(skia_safe::paint::Style::Stroke);
            outline.set_stroke_width(1.0);
            outline.set_color(Color::from_rgb(0xFF, 0x9F, 0x0A));
            canvas.draw_rect(pane().with_outset((1.0, 1.0)), &outline);

            // In `surfaces` mode the pane is a subsurface the compositor
            // composites over this buffer, so there is deliberately nothing
            // to draw here — that absence is the whole result.
            if mode == Mode::Canvas {
                let scroll = scroll_for_draw.lock().unwrap();
                let theme = AppContext::current_theme();
                let mut ground = Paint::default();
                ground.set_color(Color::from_rgb(0x24, 0x26, 0x2B));
                canvas.draw_rect(pane(), &ground);
                scroll.render(canvas, &theme, |canvas, band| {
                    draw_rows(canvas, band, &stats)
                });
            }
        });

        {
            let mut scroll = self.scroll.lock().unwrap();
            scroll.set_viewport(pane());
            scroll.set_content_length(ROWS as f32 * ROW_H);
        }

        if mode == Mode::ClientSub {
            let parent = window
                .surface()
                .map(|s| s.wl_surface().clone())
                .ok_or("window has no surface yet")?;
            // Pane-sized buffer: the clipping is the buffer's own edges, so
            // the client never draws outside the viewport and the damage it
            // reports can only ever be the pane.
            let sub = otto_kit::surfaces::SubsurfaceSurface::new(
                &parent,
                PANE_X as i32,
                PANE_Y as i32,
                PANE_W as i32,
                PANE_H as i32,
            )?;
            {
                use wayland_client::Proxy;
                *self.band_id.lock().unwrap() = Some(sub.wl_surface().id());
            }
            *self.pane_sub.lock().unwrap() = Some(sub);
            self.scroll
                .lock()
                .unwrap()
                .set_viewport(Rect::from_wh(PANE_W, PANE_H));
        }

        if mode == Mode::Surfaces {
            let parent = window
                .surface()
                .map(|s| s.wl_surface().clone())
                .ok_or("window has no surface yet")?;
            // The surface-backed view measures its own viewport from the
            // pane rect it is given, so its ScrollView works in pane-local
            // coordinates.
            self.scroll
                .lock()
                .unwrap()
                .set_viewport(Rect::from_wh(PANE_W, PANE_H));
            let surfaces = ScrollSurfaces::new(&parent, pane(), Color::from_rgb(0x24, 0x26, 0x2B))?;
            {
                use wayland_client::Proxy;
                *self.band_id.lock().unwrap() = Some(surfaces.content_surface().id());
            }
            *self.surfaces.lock().unwrap() = Some(surfaces);
        }

        let scroll = self.scroll.clone();
        let band_id = self.band_id.clone();
        let window_surface = window.wl_surface();
        AppContext::register_pointer_callback(move |events| {
            use wayland_client::Proxy;
            let ours = |event: &smithay_client_toolkit::seat::pointer::PointerEvent| {
                let id = event.surface.id();
                window_surface.as_ref().is_some_and(|s| s.id() == id)
                    || band_id.lock().unwrap().as_ref() == Some(&id)
            };
            for event in events.iter().filter(|e| ours(e)) {
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

        *self.window_handle.lock().unwrap() = Some(window.clone());

        // Frame sync: advance the scroll once per compositor frame callback.
        // `SCROLL_AB_TIMER=1` keeps the old free-running 8 ms timer instead.
        if std::env::var_os("SCROLL_AB_TIMER").is_none() {
            let driver = Driver {
                mode,
                scroll: self.scroll.clone(),
                surfaces: self.surfaces.clone(),
                pane_sub: self.pane_sub.clone(),
                stats: self.stats.clone(),
                window: self.window_handle.clone(),
                last_tick: self.last_tick.clone(),
                auto_frames: self.auto_frames.clone(),
            };
            // The callback loop only sustains itself on a surface that
            // commits: `wl_surface.frame()` takes effect on the next commit,
            // and in `surfaces` mode the toplevel deliberately never commits
            // again. So drive from whichever surface this mode keeps
            // committing — the band for `surfaces`, the pane for
            // `client-sub`, the toplevel for `canvas`.
            let driver_surface: Option<wayland_client::protocol::wl_surface::WlSurface> = match mode
            {
                Mode::Surfaces => self
                    .surfaces
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|s| s.content_surface().clone()),
                Mode::ClientSub => self
                    .pane_sub
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|s| s.wl_surface().clone()),
                Mode::Canvas => window.wl_surface(),
            };
            if let Some(surface) = driver_surface {
                use wayland_client::Proxy;
                AppContext::register_frame_callback(surface.id(), move || driver.step());
                AppContext::request_initial_frame(&surface);
            }
        }

        self.window = Some(window);
        Ok(())
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        // Only the legacy timer path runs work here; with frame sync on, the
        // compositor's frame callback drives everything instead.
        if std::env::var_os("SCROLL_AB_TIMER").is_some() {
            let driver = Driver {
                mode: self.mode,
                scroll: self.scroll.clone(),
                surfaces: self.surfaces.clone(),
                pane_sub: self.pane_sub.clone(),
                stats: self.stats.clone(),
                window: self.window_handle.clone(),
                last_tick: self.last_tick.clone(),
                auto_frames: self.auto_frames.clone(),
            };
            driver.step();
        }
        self.frames = *self.auto_frames.lock().unwrap();
        if std::env::var_os("SCROLL_AB_AUTO").is_some() && self.frames >= AUTO_END {
            self.report();
            std::process::exit(0);
        }
    }

    fn idle_timeout(&self) -> Option<std::time::Duration> {
        // Still needed as a heartbeat so the auto-run can finish and exit, but
        // with frame sync on it does no scrolling work.
        Some(std::time::Duration::from_millis(8))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    AppRunner::new(Ab {
        mode: Mode::from_args(),
        window: None,
        surfaces: Arc::new(Mutex::new(None)),
        pane_sub: Arc::new(Mutex::new(None)),
        window_handle: Arc::new(Mutex::new(None)),
        last_tick: Arc::new(Mutex::new(None)),
        auto_frames: Arc::new(Mutex::new(0)),
        band_id: Arc::new(Mutex::new(None)),
        scroll: Arc::new(Mutex::new(ScrollView::new(pane()))),
        stats: Arc::new(Stats::default()),
        frames: 0,
    })
    .run()?;
    Ok(())
}
