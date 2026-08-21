//! Baseline for the settings-pane scroll optimisation (skip off-screen rows,
//! then tile the surface). Measures what a settings-pane-shaped draw closure
//! costs today: walking N rows, shaping each row's labels, and recording the
//! toggle/slider draw ops, rasterised into an offscreen Skia surface.
//!
//! This is a plain `fn main()`, not `#[bench]` / criterion — criterion isn't
//! a dependency anywhere in this workspace and CLAUDE.md asks to avoid
//! adding crates for something `std::time::Instant` + a warmup + a few
//! hundred iterations can already answer. The `[[bench]]` entry in
//! `Cargo.toml` sets `harness = false` so cargo just runs this `main`
//! instead of expecting the (nightly-only) `#[bench]` libtest harness. Run
//! it with:
//!
//!     cargo bench -p otto-kit --bench pane_paint
//!
//! `cargo bench` always builds in release mode; `cargo run` has no way to
//! target a `[[bench]]` entry, so that's the only invocation that works.
//!
//! IMPORTANT: this only measures CPU-side recording/rasterisation on a raster
//! (CPU) Skia surface — there is no GPU and no Wayland connection involved,
//! so it says nothing about GPU upload/composite cost. That's deliberate:
//! the optimisation under test (skip off-screen rows, then tile, and/or cache
//! shaped text) is entirely about how much CPU work the draw closure does
//! before a single pixel reaches the GPU.
//!
//! ## Part 2: text-shaping cache experiment
//!
//! The first table below (unchanged) established that a canvas clip already
//! culls most *rasterisation* work, and that what's left is dominated by the
//! per-row loop plus text measurement/shaping — `Label::render` calls
//! `Font::measure_str` then `Canvas::draw_str` for every row, visible or not,
//! since the clip only affects what Skia paints, not what the draw closure
//! does. The second table isolates how much of that residual cost is text
//! shaping specifically, and how much a per-row cache would recover, by
//! comparing four variants at 200 and 2000 rows (all under the same
//! viewport clip):
//!
//! 1. **today** — `Font::measure_str` + `canvas.draw_str` for every row,
//!    exactly what `Label::render` does (see
//!    `components/otto-kit/src/components/label/label.rs`).
//! 2. **skip** — only rows intersecting the viewport band are visited at
//!    all (what the settings pane now does); still measure+draw per visited
//!    row.
//! 3. **cache (warm)** — every row is still visited, but its label/description
//!    text is measured and shaped into a `skia_safe::TextBlob` exactly once,
//!    up front, and the timed loop only replays `Canvas::draw_text_blob`.
//!    This simulates the *steady state* of scrolling with a shaped-text
//!    cache: real scrolling redraws the same rows' text over and over, so
//!    after the first frame every subsequent frame is a cache hit. The
//!    cache-build cost (the one-time "cold" shaping pass) is reported
//!    separately and is *not* included in the timed samples below — the
//!    question this answers is "what does frame N+1 cost", not "what does
//!    frame 1 cost".
//! 4. **skip + cache (warm)** — both together: the realistic end state.
//!
//! What exactly is cached: a `skia_safe::TextBlob` per label (title and
//! description text differ per row) plus the `Font::measure_str` width that
//! was measured while building it — i.e. blob *and* measurement are both
//! precomputed once and reused, since `Label::render` uses `measure_str`'s
//! width for alignment and a cache that only memoised the blob would still
//! re-measure every frame. The toggle/slider control itself is drawn
//! identically in every variant (its geometry is index-derived, not text);
//! the slider's numeric readout goes through `otto_kit::components::slider`,
//! which calls `Label` internally, so — since this bench does not touch
//! library source — that one small label per odd row still re-measures and
//! re-shapes every frame in *all* variants, including "cache". That's a
//! deliberate, small, and constant per-row cost left on the table; it does
//! not change the shape of the comparison.

use otto_kit::components::slider;
use otto_kit::components::toggle;
use otto_kit::prelude::*;
use skia_safe::{surfaces, Font, Paint, Point, Rect, TextBlob};
use std::time::{Duration, Instant};

/// Matches `otto-settings/src/view.rs::ROW_H` — kept as a literal here
/// rather than a dependency, since the bench is only borrowing the shape of
/// a settings row, not the crate.
const ROW_H: f32 = 42.0;
/// Content column width, roughly `otto-settings`' window width minus its
/// sidebar and padding.
const CONTENT_W: f32 = 640.0;
/// The viewport band the real pane clips its draw closure to today.
const VIEWPORT_H: f32 = 600.0;

/// One synthetic settings row: a label, a dimmer description underneath, and
/// a trailing control that alternates between a toggle and a slider — the
/// two controls that dominate a real pane and that differ most in how many
/// draw ops they record.
fn draw_row(canvas: &Canvas, theme: &Theme, index: usize, y: f32) {
    let cy_label = y + 15.0;
    let cy_desc = y + 30.0;

    Label::new(format!("Setting {index}"))
        .with_style(styles::BODY)
        .with_color(theme.text_primary)
        .at(20.0, cy_label - styles::BODY.size * 0.8)
        .render(canvas);

    Label::new("A short description of what this setting changes.")
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_secondary)
        .at(20.0, cy_desc - styles::SUBHEADLINE.size * 0.8)
        .render(canvas);

    draw_control(canvas, theme, index, y);
}

/// The trailing toggle/slider control, factored out so the cached-text
/// variant can reuse it unchanged — the point of this bench is isolating
/// the cost of text shaping, not the control drawing.
fn draw_control(canvas: &Canvas, theme: &Theme, index: usize, y: f32) {
    let control_cy = y + ROW_H / 2.0;
    if index.is_multiple_of(2) {
        let rect = Rect::from_xywh(CONTENT_W - 40.0 - 20.0, control_cy - 12.0, 40.0, 24.0);
        let on = index.is_multiple_of(4);
        toggle::draw(
            canvas,
            rect,
            toggle::knob_fraction_for(on),
            toggle::ToggleInteraction::Normal,
            theme,
        );
    } else {
        let rect = Rect::from_xywh(CONTENT_W - 160.0 - 60.0, control_cy - 2.0, 160.0, 4.0);
        slider::draw(
            canvas,
            rect,
            ((index * 37) % 100) as f32,
            0.0,
            100.0,
            Some("50%"),
            slider::SliderInteraction::Normal,
            theme,
        );
    }
}

/// Draw all `row_count` rows into `canvas`, starting at y = 0. This is the
/// full pane content — the caller decides separately whether to clip.
fn draw_pane(canvas: &Canvas, theme: &Theme, row_count: usize) {
    for i in 0..row_count {
        draw_row(canvas, theme, i, i as f32 * ROW_H);
    }
}

/// Row indices whose row rect intersects the `[0, viewport_h]` band — the
/// only rows a "skip" pane touches at all. The synthetic pane never
/// scrolls (row 0 always starts at y = 0), matching the offscreen-surface
/// setup below; a real pane would offset this by the scroll position, but
/// the row *count* touched is what matters for cost, and that's identical.
fn visible_row_range(row_count: usize, viewport_h: f32) -> std::ops::Range<usize> {
    let last = ((viewport_h / ROW_H).ceil() as usize + 1).min(row_count);
    0..last
}

fn draw_pane_skip(canvas: &Canvas, theme: &Theme, row_count: usize, viewport_h: f32) {
    for i in visible_row_range(row_count, viewport_h) {
        draw_row(canvas, theme, i, i as f32 * ROW_H);
    }
}

/// A single row's shaped text, built once and replayed every frame. Caches
/// both the `TextBlob` (what `draw_text_blob` needs) and the width
/// `measure_str` reported while building it (what `Label`'s alignment math
/// needs) — see the module doc comment for why both matter.
struct CachedText {
    blob: TextBlob,
    #[allow(dead_code)] // measured for parity with Label's real cost; alignment is Left here
    width: f32,
    /// `font.size() * 0.8`, matching the baseline offset `Label::render` applies.
    baseline_offset: f32,
}

impl CachedText {
    fn build(text: &str, font: &Font) -> Self {
        let (width, _) = font.measure_str(text, None);
        let blob = TextBlob::from_str(text, font).expect("text blob build failed");
        CachedText {
            blob,
            width,
            baseline_offset: font.size() * 0.8,
        }
    }

    fn draw(&self, canvas: &Canvas, x: f32, y: f32, paint: &Paint) {
        canvas.draw_text_blob(&self.blob, Point::new(x, y + self.baseline_offset), paint);
    }
}

struct RowCache {
    label: CachedText,
    desc: CachedText,
}

/// Build the shaped-text cache for `row_count` rows. This is the "cold"
/// shaping pass — timed separately from the warm redraw loop below, since
/// in real scrolling it only happens once per row (the first time it
/// scrolls into view), not once per frame.
fn build_row_cache(row_count: usize) -> Vec<RowCache> {
    let label_font = styles::BODY.font();
    let desc_font = styles::SUBHEADLINE.font();
    (0..row_count)
        .map(|i| RowCache {
            label: CachedText::build(&format!("Setting {i}"), &label_font),
            desc: CachedText::build(
                "A short description of what this setting changes.",
                &desc_font,
            ),
        })
        .collect()
}

fn draw_row_cached(canvas: &Canvas, theme: &Theme, index: usize, y: f32, cache: &RowCache) {
    let mut label_paint = Paint::default();
    label_paint.set_color(theme.text_primary);
    label_paint.set_anti_alias(true);
    let mut desc_paint = Paint::default();
    desc_paint.set_color(theme.text_secondary);
    desc_paint.set_anti_alias(true);

    let cy_label = y + 15.0;
    let cy_desc = y + 30.0;
    cache.label.draw(
        canvas,
        20.0,
        cy_label - styles::BODY.size * 0.8,
        &label_paint,
    );
    cache.desc.draw(
        canvas,
        20.0,
        cy_desc - styles::SUBHEADLINE.size * 0.8,
        &desc_paint,
    );

    draw_control(canvas, theme, index, y);
}

fn draw_pane_cached(canvas: &Canvas, theme: &Theme, row_count: usize, cache: &[RowCache]) {
    for (i, row_cache) in cache.iter().enumerate().take(row_count) {
        draw_row_cached(canvas, theme, i, i as f32 * ROW_H, row_cache);
    }
}

fn draw_pane_skip_cached(
    canvas: &Canvas,
    theme: &Theme,
    row_count: usize,
    viewport_h: f32,
    cache: &[RowCache],
) {
    for i in visible_row_range(row_count, viewport_h) {
        draw_row_cached(canvas, theme, i, i as f32 * ROW_H, &cache[i]);
    }
}

#[derive(Clone, Copy)]
struct Stats {
    median: Duration,
    p90: Duration,
}

fn run_iterations(mut f: impl FnMut(), warmup: usize, iterations: usize) -> Stats {
    for _ in 0..warmup {
        f();
    }
    let mut samples: Vec<Duration> = (0..iterations)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    samples.sort();
    let median = samples[samples.len() / 2];
    let p90 = samples[(samples.len() * 9 / 10).min(samples.len() - 1)];
    Stats { median, p90 }
}

fn fmt(d: Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1000.0)
}

fn clipped_surface(row_count: usize) -> skia_safe::Surface {
    let height = (row_count as f32 * ROW_H).ceil() as i32;
    surfaces::raster_n32_premul((CONTENT_W as i32, height.max(1)))
        .expect("failed to create offscreen raster surface")
}

fn with_viewport_clip(canvas: &Canvas, f: impl FnOnce(&Canvas)) {
    canvas.save();
    canvas.clip_rect(Rect::from_xywh(0.0, 0.0, CONTENT_W, VIEWPORT_H), None, None);
    f(canvas);
    canvas.restore();
}

fn main() {
    let theme = Theme::light();
    // A conservative iteration count: the 2000-row case is the slow one, and
    // even that comfortably finishes a few hundred iterations in seconds.
    let warmup = 5;
    let iterations = 100;

    println!(
        "{:<10} {:<10} {:>12} {:>12}",
        "rows", "clip", "median", "p90"
    );

    for &row_count in &[20usize, 200, 2000] {
        // Surface tall enough to hold the whole pane, uncropped — the clip
        // variant below clips the *canvas*, not the surface, matching how
        // the real pane's draw closure works today.
        let mut surface = clipped_surface(row_count);

        let whole = run_iterations(
            || {
                let canvas = surface.canvas();
                draw_pane(canvas, &theme, row_count);
            },
            warmup,
            iterations,
        );
        println!(
            "{row_count:<10} {:<10} {:>12} {:>12}",
            "no",
            fmt(whole.median),
            fmt(whole.p90)
        );

        let clipped = run_iterations(
            || {
                let canvas = surface.canvas();
                with_viewport_clip(canvas, |canvas| draw_pane(canvas, &theme, row_count));
            },
            warmup,
            iterations,
        );
        println!(
            "{row_count:<10} {:<10} {:>12} {:>12}",
            "yes",
            fmt(clipped.median),
            fmt(clipped.p90)
        );
    }

    // --- Part 2: how much of the residual (clipped) cost is text shaping? ---
    //
    // All four variants below run under the same viewport clip as the "yes"
    // rows above — that established clip cost is a constant, so it's held
    // fixed here to isolate the effect of row-skipping and text caching on
    // top of it. All timed samples are WARM steady-state redraws (see the
    // "cache build" line, printed once per row count, for the one-off COLD
    // shaping cost that a cache variant pays before it can start paying off).
    println!();
    println!(
        "{:<10} {:<24} {:>12} {:>12}",
        "rows", "variant", "median", "p90"
    );

    for &row_count in &[200usize, 2000] {
        let mut surface = clipped_surface(row_count);

        // One-off cold cost of populating the cache — not part of any of
        // the timed steady-state samples below.
        let cache_build_start = Instant::now();
        let cache = build_row_cache(row_count);
        let cache_build = cache_build_start.elapsed();
        println!(
            "{row_count:<10} {:<24} {:>12} {:>12}",
            "(cache build, cold, x1)",
            fmt(cache_build),
            ""
        );

        let today = run_iterations(
            || {
                let canvas = surface.canvas();
                with_viewport_clip(canvas, |canvas| draw_pane(canvas, &theme, row_count));
            },
            warmup,
            iterations,
        );
        println!(
            "{row_count:<10} {:<24} {:>12} {:>12}",
            "today (measure+draw all)",
            fmt(today.median),
            fmt(today.p90)
        );

        let skip = run_iterations(
            || {
                let canvas = surface.canvas();
                with_viewport_clip(canvas, |canvas| {
                    draw_pane_skip(canvas, &theme, row_count, VIEWPORT_H)
                });
            },
            warmup,
            iterations,
        );
        println!(
            "{row_count:<10} {:<24} {:>12} {:>12}",
            "skip (viewport rows only)",
            fmt(skip.median),
            fmt(skip.p90)
        );

        let cached = run_iterations(
            || {
                let canvas = surface.canvas();
                with_viewport_clip(canvas, |canvas| {
                    draw_pane_cached(canvas, &theme, row_count, &cache)
                });
            },
            warmup,
            iterations,
        );
        println!(
            "{row_count:<10} {:<24} {:>12} {:>12}",
            "cache, warm (all rows)",
            fmt(cached.median),
            fmt(cached.p90)
        );

        let skip_cached = run_iterations(
            || {
                let canvas = surface.canvas();
                with_viewport_clip(canvas, |canvas| {
                    draw_pane_skip_cached(canvas, &theme, row_count, VIEWPORT_H, &cache)
                });
            },
            warmup,
            iterations,
        );
        println!(
            "{row_count:<10} {:<24} {:>12} {:>12}",
            "skip + cache, warm",
            fmt(skip_cached.median),
            fmt(skip_cached.p90)
        );
    }
}
