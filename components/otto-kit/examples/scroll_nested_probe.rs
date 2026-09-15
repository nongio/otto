//! Surface-backed scrolling on both axes, and panes nested inside a pane.
//!
//! The pane is driven by time, not by input, so a script can measure what the
//! compositor does with it: every presented frame moves a band, and a band is
//! only repainted when the scroll nears its edge. Laid out the way an
//! application would: the window paints the pane's ground once, the panes are
//! transparent, and the bands carry rows.
//!
//! ```sh
//! cargo run --release -p otto-kit --example scroll_nested_probe -- --mode nested
//! ```
//!
//! - `--mode vertical`    one vertical pane of rows
//! - `--mode horizontal`  one horizontal pane of tiles
//! - `--mode nested`      vertical panes inside a horizontal container, all moving
//! - `--seconds N`        run time (default 12), then print a summary and exit
//! - `--speed PT`         sweep speed in points per second (default 1500)

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use otto_kit::components::scroll::{Axis, ScrollState, ScrollSurfaces};
use otto_kit::prelude::*;
use skia_safe::{Canvas, Color, Font, FontStyle, Paint, Rect};

const WINDOW_W: f32 = 1100.0;
const WINDOW_H: f32 = 700.0;
const PANE: Rect = Rect {
    left: 40.0,
    top: 60.0,
    right: 1060.0,
    bottom: 640.0,
};
const ROW_H: f32 = 44.0;
const ROWS: usize = 600;
const TILE_W: f32 = 160.0;
const TILES: usize = 400;
const COLUMN_W: f32 = 300.0;
const COLUMNS: usize = 24;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Vertical,
    Horizontal,
    Nested,
}

#[derive(Clone, Copy)]
struct Args {
    mode: Mode,
    seconds: f32,
    speed: f32,
}

fn args() -> Args {
    let mut args = Args {
        mode: Mode::Nested,
        seconds: 12.0,
        speed: 1500.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--mode" => {
                args.mode = match it.next().as_deref() {
                    Some("vertical") => Mode::Vertical,
                    Some("horizontal") => Mode::Horizontal,
                    Some("nested") => Mode::Nested,
                    other => panic!("--mode vertical|horizontal|nested, got {other:?}"),
                }
            }
            "--seconds" => args.seconds = it.next().unwrap().parse().unwrap(),
            "--speed" => args.speed = it.next().unwrap().parse().unwrap(),
            other => panic!("unknown flag {other}"),
        }
    }
    args
}

/// A back-and-forth sweep over `0..=max` at `speed`, `t` seconds in.
fn sweep(t: f32, speed: f32, max: f32) -> f32 {
    if max <= 0.0 {
        return 0.0;
    }
    let travel = (t * speed) % (2.0 * max);
    if travel <= max {
        travel
    } else {
        2.0 * max - travel
    }
}

thread_local! {
    static FONT: Font = {
        let mgr = skia_safe::FontMgr::new();
        let typeface = mgr
            .match_family_style("Inter", FontStyle::normal())
            .or_else(|| mgr.match_family_style("sans-serif", FontStyle::normal()))
            .expect("no font available");
        Font::new(typeface, 15.0)
    };
}

fn text_paint() -> Paint {
    let mut text = Paint::default();
    text.set_anti_alias(true);
    text.set_color(Color::from_rgb(0x20, 0x22, 0x26));
    text
}

/// Rows of a vertical pane that intersect `band`, in content coordinates, on a
/// transparent canvas: every other row is shaded, the ground is the window's.
fn paint_rows(canvas: &Canvas, band: Rect, width: f32, tint: u8) {
    let mut shade = Paint::default();
    shade.set_color(Color::from_argb(0x18, 0x40, 0x40, tint));
    let text = text_paint();
    let first = (band.top / ROW_H).floor().max(0.0) as usize;
    let last = ((band.bottom / ROW_H).ceil() as usize).min(ROWS);
    FONT.with(|font| {
        for row in first..last {
            let y = row as f32 * ROW_H;
            if row % 2 == 0 {
                canvas.draw_rect(Rect::from_xywh(0.0, y, width, ROW_H), &shade);
            }
            canvas.draw_str(format!("row {row}"), (14.0, y + 28.0), font, &text);
        }
    });
}

/// Tiles of a horizontal pane that intersect `band`, in content coordinates.
fn paint_tiles(canvas: &Canvas, band: Rect, height: f32) {
    let mut shade = Paint::default();
    shade.set_color(Color::from_argb(0x18, 0x40, 0x20, 0x60));
    let text = text_paint();
    let first = (band.left / TILE_W).floor().max(0.0) as usize;
    let last = ((band.right / TILE_W).ceil() as usize).min(TILES);
    FONT.with(|font| {
        for tile in first..last {
            let x = tile as f32 * TILE_W;
            if tile % 2 == 0 {
                canvas.draw_rect(Rect::from_xywh(x, 0.0, TILE_W, height), &shade);
            }
            canvas.draw_str(format!("tile {tile}"), (x + 14.0, 40.0), font, &text);
        }
    });
}

struct Pane {
    surfaces: ScrollSurfaces,
    state: ScrollState,
    /// Seconds of phase, so the columns do not all move in step.
    phase: f32,
}

#[derive(Default)]
struct Counters {
    started: Option<Instant>,
    steps: u32,
    step_gaps_ms: Vec<f64>,
    last_step: Option<Instant>,
    repaints: Rc<RefCell<u32>>,
}

struct Probe {
    args: Args,
    window: Option<Window>,
    outer: Option<Pane>,
    inner: Vec<Pane>,
    counters: Counters,
}

impl App for Probe {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("scroll nested probe", WINDOW_W as i32, WINDOW_H as i32)?;
        // The window's own paint: chrome around the pane and the pane's ground,
        // drawn once. Nothing here moves while the panes scroll.
        window.on_draw(move |canvas| {
            canvas.clear(Color::from_rgb(0x1C, 0x1E, 0x22));
            let mut ground = Paint::default();
            ground.set_color(Color::from_rgb(0xF6, 0xF6, 0xF8));
            canvas.draw_rect(PANE, &ground);
        });
        let parent = window
            .surface()
            .map(|s| s.wl_surface().clone())
            .ok_or("window has no surface yet")?;
        let viewport_local = Rect::from_wh(PANE.width(), PANE.height());

        match self.args.mode {
            Mode::Vertical => {
                let mut surfaces = ScrollSurfaces::new(&parent, PANE, Color::TRANSPARENT)?;
                surfaces.set_input_passthrough();
                let mut state = ScrollState::new(viewport_local);
                state.set_content_length(ROWS as f32 * ROW_H);
                self.outer = Some(Pane {
                    surfaces,
                    state,
                    phase: 0.0,
                });
            }
            Mode::Horizontal => {
                let mut surfaces =
                    ScrollSurfaces::on_axis(&parent, PANE, Color::TRANSPARENT, Axis::Horizontal)?;
                surfaces.set_input_passthrough();
                let mut state = ScrollState::on_axis(Axis::Horizontal, viewport_local);
                state.set_content_length(TILES as f32 * TILE_W);
                self.outer = Some(Pane {
                    surfaces,
                    state,
                    phase: 0.0,
                });
            }
            Mode::Nested => {
                let mut outer = ScrollSurfaces::container(&parent, PANE, Axis::Horizontal)?;
                outer.set_input_passthrough();
                let mut state = ScrollState::on_axis(Axis::Horizontal, viewport_local);
                state.set_content_length(COLUMNS as f32 * COLUMN_W);
                for column in 0..COLUMNS {
                    // Content coordinates of the container: the column stays
                    // here however the stack is panned.
                    let rect = Rect::from_xywh(
                        column as f32 * COLUMN_W,
                        0.0,
                        COLUMN_W - 1.0,
                        PANE.height(),
                    );
                    let mut surfaces =
                        ScrollSurfaces::new(outer.band_surface(), rect, Color::TRANSPARENT)?;
                    surfaces.set_input_passthrough();
                    let mut inner = ScrollState::new(Rect::from_wh(rect.width(), rect.height()));
                    inner.set_content_length(ROWS as f32 * ROW_H);
                    self.inner.push(Pane {
                        surfaces,
                        state: inner,
                        phase: column as f32 * 0.37,
                    });
                }
                self.outer = Some(Pane {
                    surfaces: outer,
                    state,
                    phase: 0.0,
                });
            }
        }

        self.window = Some(window);
        Ok(())
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let now = Instant::now();
        let started = *self.counters.started.get_or_insert(now);
        let t = now.duration_since(started).as_secs_f32();
        let theme = AppContext::current_theme();
        let speed = self.args.speed;
        let repaints = self.counters.repaints.clone();

        let mut stepped = false;
        let mut visible = 0.0..f32::MAX;
        if let Some(outer) = self.outer.as_mut() {
            let max = outer.state.max_offset();
            outer.state.set_offset(sweep(t, speed, max));
            outer.state.set_scrollbar_opacity(1.0);
            let cross = outer.state.viewport();
            let mode = self.args.mode;
            stepped |= outer
                .surfaces
                .sync_state(&outer.state, speed, &theme, |canvas, band| {
                    *repaints.borrow_mut() += 1;
                    match mode {
                        Mode::Vertical => paint_rows(canvas, band, cross.width(), 0x80),
                        Mode::Horizontal => paint_tiles(canvas, band, cross.height()),
                        Mode::Nested => {}
                    }
                });
            let offset = outer.state.offset();
            visible = offset..offset + outer.state.viewport_length();
        }
        for (index, pane) in self.inner.iter_mut().enumerate() {
            // A column panned out of the container's view is left where it is,
            // hidden: nothing of it is on screen to keep up to date.
            let left = index as f32 * COLUMN_W;
            let shown = left + COLUMN_W > visible.start && left < visible.end;
            pane.surfaces.set_hidden(!shown);
            if !shown {
                continue;
            }
            let max = pane.state.max_offset();
            pane.state
                .set_offset(sweep(t + pane.phase, speed * 0.6, max));
            pane.state.set_scrollbar_opacity(1.0);
            let width = pane.state.viewport().width();
            let repaints = repaints.clone();
            let tint = (0x40 + index * 9 % 0x80) as u8;
            stepped |=
                pane.surfaces
                    .sync_state(&pane.state, speed * 0.6, &theme, |canvas, band| {
                        *repaints.borrow_mut() += 1;
                        paint_rows(canvas, band, width, tint);
                    });
        }

        if stepped {
            self.counters.steps += 1;
            if let Some(last) = self.counters.last_step {
                self.counters
                    .step_gaps_ms
                    .push(now.duration_since(last).as_secs_f64() * 1000.0);
            }
            self.counters.last_step = Some(now);
        }

        if t >= self.args.seconds {
            let mut gaps = self.counters.step_gaps_ms.clone();
            gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let pct = |p: f64| {
                gaps.get(((gaps.len() as f64 * p) as usize).min(gaps.len().saturating_sub(1)))
                    .copied()
                    .unwrap_or(0.0)
            };
            println!(
                "scroll_nested_probe mode={} steps={} ({:.1}/s) band_repaints={} step_gap p50 {:.1} p90 {:.1} p99 {:.1} ms",
                match self.args.mode {
                    Mode::Vertical => "vertical",
                    Mode::Horizontal => "horizontal",
                    Mode::Nested => "nested",
                },
                self.counters.steps,
                self.counters.steps as f32 / t,
                self.counters.repaints.borrow(),
                pct(0.5),
                pct(0.9),
                pct(0.99)
            );
            std::process::exit(0);
        }
    }

    fn idle_timeout(&self) -> Option<Duration> {
        Some(Duration::from_millis(4))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    AppRunner::new(Probe {
        args: args(),
        window: None,
        outer: None,
        inner: Vec::new(),
        counters: Counters::default(),
    })
    .run()?;
    Ok(())
}
