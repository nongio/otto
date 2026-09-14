//! A near-empty window that repaints on every frame the compositor presents,
//! for measuring what a commit costs the compositor rather than the client.
//!
//! The client work is kept trivial — a ground, a static side panel and a band of
//! moving stripes — and identical in every mode. Only what is *reported* as
//! damaged changes, so two runs differ in the compositor's work alone.
//!
//! ```sh
//! cargo run --release -p otto-kit --example damage_probe -- --damage full
//! cargo run --release -p otto-kit --example damage_probe -- --damage strip --opaque
//! ```
//!
//! - `--damage full`   report the whole buffer (what otto-kit does by default)
//! - `--damage strip`  report the band that changed — a list's viewport
//! - `--damage small`  animate and report a 120×40 pt patch only
//! - `--opaque`        opaque ground plus an opaque region
//! - `--seconds N`     run time (default 12), then print frame intervals and exit
//! - `--size WxH`      window size in points (default 1100x700)

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use otto_kit::prelude::*;
use skia_safe::{Color, Paint, Rect};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Damage {
    Full,
    Strip,
    Small,
}

#[derive(Clone, Copy)]
struct Args {
    damage: Damage,
    opaque: bool,
    seconds: f32,
    width: f32,
    height: f32,
}

fn args() -> Args {
    let mut args = Args {
        damage: Damage::Full,
        opaque: false,
        seconds: 12.0,
        width: 1100.0,
        height: 700.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--damage" => {
                args.damage = match it.next().as_deref() {
                    Some("full") => Damage::Full,
                    Some("strip") => Damage::Strip,
                    Some("small") => Damage::Small,
                    other => panic!("--damage full|strip|small, got {other:?}"),
                }
            }
            "--opaque" => args.opaque = true,
            "--seconds" => args.seconds = it.next().unwrap().parse().unwrap(),
            "--size" => {
                let size = it.next().unwrap();
                let (w, h) = size.split_once('x').expect("--size WxH");
                args.width = w.parse().unwrap();
                args.height = h.parse().unwrap();
            }
            other => panic!("unknown flag {other}"),
        }
    }
    args
}

/// The part of the window whose pixels change, in points.
fn animated_rect(args: &Args) -> Rect {
    match args.damage {
        // Where a file list's rows sit: right of a sidebar, under a header,
        // above a path bar.
        Damage::Full | Damage::Strip => {
            Rect::from_ltrb(260.0, 90.0, args.width, args.height - 30.0)
        }
        Damage::Small => Rect::from_xywh(300.0, 200.0, 120.0, 40.0),
    }
}

#[derive(Default)]
struct Stats {
    started: Option<Instant>,
    last: Option<Instant>,
    intervals_ms: Vec<f64>,
}

struct Probe {
    args: Args,
    window: Option<Window>,
    phase: Arc<Mutex<f32>>,
    stats: Arc<Mutex<Stats>>,
}

impl App for Probe {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let args = self.args;
        let mut window = Window::new("damage probe", args.width as i32, args.height as i32)?;

        let phase = self.phase.clone();
        window.on_draw(move |canvas| {
            let offset = *phase.lock().unwrap();
            let ground = if args.opaque {
                Color::from_rgb(0xF4, 0xF4, 0xF6)
            } else {
                Color::from_argb(0xE0, 0xF4, 0xF4, 0xF6)
            };
            canvas.clear(ground);

            let mut panel = Paint::default();
            panel.set_color(Color::from_rgb(0xE4, 0xE4, 0xE8));
            canvas.draw_rect(Rect::from_ltrb(0.0, 0.0, 260.0, args.height), &panel);

            // Rows scrolling past: the same pixels change in every mode.
            let area = animated_rect(&args);
            canvas.save();
            canvas.clip_rect(area, None, None);
            let mut row = Paint::default();
            let pitch = 32.0;
            let first = ((area.top - offset) / pitch).floor() as i32;
            let last = ((area.bottom - offset) / pitch).ceil() as i32;
            for i in first..=last {
                let shade = if i.rem_euclid(2) == 0 { 0xFF } else { 0xEE };
                row.set_color(Color::from_rgb(shade, shade, shade));
                let top = offset + i as f32 * pitch;
                canvas.draw_rect(
                    Rect::from_ltrb(area.left, top, area.right, top + pitch),
                    &row,
                );
            }
            canvas.restore();
        });

        let surface = window.wl_surface().ok_or("window has no surface yet")?;

        if args.opaque {
            use otto_kit::app_runner::AppContext as Ctx;
            let region = Ctx::compositor_state()
                .wl_compositor()
                .create_region(Ctx::queue_handle(), ());
            region.add(0, 0, args.width as i32, args.height as i32);
            surface.set_opaque_region(Some(&region));
            region.destroy();
        }

        // One step per presented frame: advance the rows, say what changed,
        // and ask to be drawn.
        let driver_window = window.clone();
        let phase = self.phase.clone();
        let stats = self.stats.clone();
        let damage = match args.damage {
            Damage::Full => None,
            Damage::Strip | Damage::Small => Some(vec![animated_rect(&args)]),
        };
        {
            use wayland_client::Proxy;
            AppContext::register_frame_callback(surface.id(), move || {
                let now = Instant::now();
                {
                    let mut stats = stats.lock().unwrap();
                    stats.started.get_or_insert(now);
                    if let Some(last) = stats.last {
                        stats
                            .intervals_ms
                            .push(now.duration_since(last).as_secs_f64() * 1000.0);
                    }
                    stats.last = Some(now);
                }
                *phase.lock().unwrap() += 6.0;
                match &damage {
                    Some(rects) => driver_window.request_frame_damaged(rects),
                    None => driver_window.request_frame(),
                }
            });
        }
        AppContext::register_frame_loop(&surface);

        self.window = Some(window);
        Ok(())
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let stats = self.stats.lock().unwrap();
        let Some(started) = stats.started else {
            return;
        };
        if started.elapsed() < Duration::from_secs_f32(self.args.seconds) {
            return;
        }
        let mut intervals = stats.intervals_ms.clone();
        intervals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct =
            |p: f64| intervals[((intervals.len() as f64 * p) as usize).min(intervals.len() - 1)];
        println!(
            "damage_probe damage={:?} opaque={} frames={} interval p50 {:.1} p90 {:.1} p99 {:.1} ms",
            self.args.damage,
            self.args.opaque,
            intervals.len() + 1,
            pct(0.5),
            pct(0.9),
            pct(0.99)
        );
        std::process::exit(0);
    }

    fn idle_timeout(&self) -> Option<Duration> {
        Some(Duration::from_millis(50))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    AppRunner::new(Probe {
        args: args(),
        window: None,
        phase: Arc::new(Mutex::new(0.0)),
        stats: Arc::new(Mutex::new(Stats::default())),
    })
    .run()?;
    Ok(())
}
