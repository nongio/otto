//! Isolated micro-benchmarks for the scroll hot path.
//!
//! Not a correctness suite: each of these times one thing the browser does per
//! frame (or per wheel event) against the alternative it could do instead, so
//! "is this worth changing" is a number rather than an argument.
//!
//! ```sh
//! cargo test --bin otto-files -- --ignored --nocapture bench
//! ```

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::model::{self, Entry, SortKey};
use otto_kit::filetype::Kind;

/// Rows a 1100pt-tall pane shows at ROW_H — what one frame actually draws.
const VISIBLE: usize = 40;

fn bench<F: FnMut()>(label: &str, iters: u32, mut f: F) -> Duration {
    // One untimed pass so caches and lazy statics are not charged to the mean.
    f();
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let total = start.elapsed();
    let each = total / iters;
    println!("  {label:<44} {:>9.1} µs", each.as_secs_f64() * 1e6);
    each
}

fn ratio(label: &str, before: Duration, after: Duration) {
    let b = before.as_secs_f64();
    let a = after.as_secs_f64().max(1e-12);
    println!("  → {label}: {:.1}× faster\n", b / a);
}

fn entries(count: usize) -> Vec<Entry> {
    (0..count)
        .map(|i| {
            let name = match i % 5 {
                0 => format!("Screenshot 2026-08-{:02} at {i}.png", i % 28 + 1),
                1 => format!("report-{i}.pdf"),
                2 => format!("src-{i}"),
                3 => format!("a rather long file name that will not fit {i}.txt"),
                _ => format!("IMG_{i:04}.jpeg"),
            };
            Entry {
                path: PathBuf::from(format!("/home/u/files/{name}")),
                is_dir: i % 5 == 2,
                is_symlink: false,
                hidden: i % 11 == 0,
                kind: Kind::Other,
                size: Some((i as u64 * 7919) % 4_000_000),
                modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(i as u64 * 3600)),
                name,
            }
        })
        .collect()
}

/// `Browser::visible_uncounted`, lifted verbatim so the bench measures the
/// real comparator and the real allocation rather than a stand-in.
fn visible_uncounted(
    all: &[Entry],
    show_hidden: bool,
    sort: SortKey,
    ascending: bool,
) -> Vec<&Entry> {
    let mut entries: Vec<&Entry> = all.iter().filter(|e| show_hidden || !e.hidden).collect();
    entries.sort_by(|a, b| {
        let dirs_first = b.is_dir.cmp(&a.is_dir);
        if dirs_first != std::cmp::Ordering::Equal {
            return dirs_first;
        }
        let ord = match sort {
            SortKey::Name => model::natural_cmp(&a.name, &b.name),
            SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
            SortKey::Kind => a
                .kind_label()
                .cmp(b.kind_label())
                .then_with(|| model::natural_cmp(&a.name, &b.name)),
            SortKey::Modified => a.modified.cmp(&b.modified),
        };
        if ascending {
            ord
        } else {
            ord.reverse()
        }
    });
    entries
}

#[test]
#[ignore = "benchmark"]
fn bench_visible_sort() {
    println!("\n=== 1. visible(): sort every frame vs. cached order ===");
    for count in [200usize, 1_000, 5_000, 20_000] {
        let all = entries(count);
        println!("  -- {count} entries --");
        let sorted = bench("sort_by (what happens today)", 200, || {
            std::hint::black_box(visible_uncounted(&all, false, SortKey::Name, true));
        });
        let cached_vec: Vec<&Entry> = visible_uncounted(&all, false, SortKey::Name, true);
        let cached = bench("clone a cached Vec<&Entry>", 200, || {
            std::hint::black_box(cached_vec.clone());
        });
        let borrowed = bench("borrow a cached slice", 200, || {
            std::hint::black_box(&cached_vec[..]);
        });
        ratio("cache + clone", sorted, cached);
        ratio("cache + borrow", sorted, borrowed);
    }
}

#[test]
#[ignore = "benchmark"]
fn bench_selection_vec() {
    println!("\n=== 2. PaneData.selected: whole listing vs. visible rows ===");
    for count in [1_000usize, 20_000] {
        let all = entries(count);
        let sorted = visible_uncounted(&all, false, SortKey::Name, true);
        // A realistic multi-select: a run of 20 rows.
        let selection: BTreeSet<String> = sorted
            .iter()
            .skip(300)
            .take(20)
            .map(|e| e.name.clone())
            .collect();
        println!("  -- {count} entries, 20 selected --");
        let full = bench("Vec<bool> over all entries (today)", 500, || {
            let v: Vec<bool> = sorted.iter().map(|e| selection.contains(&e.name)).collect();
            std::hint::black_box(v);
        });
        let windowed = bench("Vec<bool> over the visible rows", 500, || {
            let v: Vec<bool> = sorted[..VISIBLE.min(sorted.len())]
                .iter()
                .map(|e| selection.contains(&e.name))
                .collect();
            std::hint::black_box(v);
        });
        let lazy = bench("lookup per drawn row, no Vec", 500, || {
            for e in &sorted[..VISIBLE.min(sorted.len())] {
                std::hint::black_box(selection.contains(&e.name));
            }
        });
        ratio("visible-range Vec", full, windowed);
        ratio("no Vec at all", full, lazy);
    }
}

#[test]
#[ignore = "benchmark"]
fn bench_row_build() {
    println!("\n=== 3. build_rows(): the cost of one re-record ===");
    let all = entries(5_000);
    let sorted = visible_uncounted(&all, false, SortKey::Name, true);
    let band = &sorted[..30];

    let chains = bench("icon_chain() × 30 rows (mime lookup + Vec)", 500, || {
        for e in band {
            std::hint::black_box(e.icon_chain());
        }
    });
    println!(
        "  a band re-records on every row boundary crossed.\n\
         \x20 at 1500 pt/s over a 24 pt row that is ~62 re-records/s:\n\
         \x20   no overscan : {:>7.2} ms/s of scroll\n\
         \x20   ±8 overscan : {:>7.2} ms/s of scroll (one per 8 rows)\n",
        chains.as_secs_f64() * 62.0 * 1e3,
        chains.as_secs_f64() * 62.0 / 8.0 * 1e3,
    );
}

#[test]
#[ignore = "benchmark"]
fn bench_text_shaping() {
    use crate::view;
    use otto_kit::typography::styles;
    use skia_safe::{surfaces, Color, Paint, PictureRecorder, Rect};

    println!("\n=== 4. list rows: immediate re-shape vs. cached picture ===");
    println!("  both sides rasterise the same 40 rows into the same surface;");
    println!("  only the measuring/shaping/formatting differs.");
    let all = entries(5_000);
    let sorted = visible_uncounted(&all, false, SortKey::Name, true);
    let band: Vec<&Entry> = sorted[..VISIBLE].to_vec();
    let font = styles::BODY_MEDIUM.font();
    let mut surface = surfaces::raster_n32_premul((900, 1000)).unwrap();

    let draw_row = |canvas: &skia_safe::Canvas, i: usize, e: &Entry, font: &skia_safe::Font| {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::BLACK);
        let y = 20.0 + i as f32 * 24.0;
        let name = view::ellipsize(font, &e.name, 320.0);
        canvas.draw_str(&name, (14.0, y), font, &paint);
        let size_text = if e.is_dir {
            "--".to_string()
        } else {
            e.size.map(model::format_size).unwrap_or_default()
        };
        canvas.draw_str(&size_text, (400.0, y), font, &paint);
        canvas.draw_str(e.kind_label(), (520.0, y), font, &paint);
        let when = e.modified.map(model::format_time).unwrap_or_default();
        canvas.draw_str(&when, (680.0, y), font, &paint);
    };

    let immediate = bench("shape + format + rasterise 40 rows", 200, || {
        let canvas = surface.canvas();
        canvas.clear(Color::WHITE);
        for (i, e) in band.iter().enumerate() {
            draw_row(canvas, i, e, &font);
        }
    });

    let mut recorder = PictureRecorder::new();
    let rec = recorder.begin_recording(Rect::from_wh(900.0, 1000.0), true);
    for (i, e) in band.iter().enumerate() {
        draw_row(rec, i, e, &font);
    }
    let picture = recorder.finish_recording_as_picture(None).unwrap();

    let replay = bench("replay the cached picture (same pixels)", 200, || {
        let canvas = surface.canvas();
        canvas.clear(Color::WHITE);
        canvas.draw_picture(&picture, None, None);
    });
    ratio("layerise list/grid rows", immediate, replay);
}

/// One frame of the real thing: what `Browser::frame()` costs before anything
/// is drawn, for a Miller stack of three columns over a directory of `count`.
#[test]
#[ignore = "benchmark"]
fn bench_frame_build() {
    println!("\n=== 5. one frame's frame(): today vs. cached order ===");
    for count in [500usize, 5_000, 20_000] {
        let all = entries(count);
        let sorted = visible_uncounted(&all, false, SortKey::Name, true);
        let selection: BTreeSet<String> = sorted
            .iter()
            .skip(3)
            .take(2)
            .map(|e| e.name.clone())
            .collect();
        println!("  -- {count} entries, 3 Miller columns --");

        // frame() calls visible() once per column; subtitle() calls it twice
        // more; sync_scroll_metrics()/counts() once per column again — and
        // sync_scroll_metrics also runs on every wheel event.
        let today = bench(
            "3×visible + 2×subtitle + 3×counts + selected",
            20,
            || {
                for _ in 0..3 {
                    let v = visible_uncounted(&all, false, SortKey::Name, true);
                    let sel: Vec<bool> = v.iter().map(|e| selection.contains(&e.name)).collect();
                    std::hint::black_box((v, sel));
                }
                for _ in 0..2 {
                    std::hint::black_box(visible_uncounted(&all, false, SortKey::Name, true).len());
                }
                for _ in 0..3 {
                    std::hint::black_box(visible_uncounted(&all, false, SortKey::Name, true).len());
                }
            },
        );
        let cached = bench("same frame off a cached sorted order", 20, || {
            for _ in 0..3 {
                let v = &sorted[..];
                let sel: Vec<bool> = v[..VISIBLE.min(v.len())]
                    .iter()
                    .map(|e| selection.contains(&e.name))
                    .collect();
                std::hint::black_box((v, sel));
            }
            for _ in 0..5 {
                std::hint::black_box(sorted.len());
            }
        });
        println!(
            "    frame budget at 120 Hz is 8333 µs — today uses {:.0}% of it, cached {:.2}%",
            today.as_secs_f64() * 1e6 / 8333.0 * 100.0,
            cached.as_secs_f64() * 1e6 / 8333.0 * 100.0
        );
        ratio("cache the sorted order", today, cached);
    }
}

/// A touchpad reports ~150 deltas a second, and each one runs
/// `sync_scroll_metrics()` before it touches a scroll view.
#[test]
#[ignore = "benchmark"]
fn bench_wheel_event() {
    println!("\n=== 6. sync_scroll_metrics() on every wheel event ===");
    for count in [500usize, 5_000, 20_000] {
        let all = entries(count);
        println!("  -- {count} entries, 3 Miller columns --");
        let per_event = bench("counts() = 3 × visible()", 20, || {
            for _ in 0..3 {
                std::hint::black_box(visible_uncounted(&all, false, SortKey::Name, true).len());
            }
        });
        println!(
            "    at 150 events/s that is {:.0} ms of CPU per second of scrolling\n",
            per_event.as_secs_f64() * 150.0 * 1e3
        );
    }
}

// ---------------------------------------------------------------------------
// 7. The architectural question: a scroll driven through lay-rs layers vs. a
//    scroll repainted immediately, over the same pixels.
// ---------------------------------------------------------------------------

/// Row height both arms lay out at — [`crate::view::ROW_H`].
const ROW_H: f32 = 24.0;
const PANE_W: f32 = 900.0;
const PANE_H: f32 = 1000.0;

/// One row with everything already resolved: the shape of `scene::Row`, which
/// is exactly the work the layer arm does once per re-record and the immediate
/// arm does once per frame.
struct BenchRow {
    top: f32,
    name: String,
    size: String,
    kind: String,
    when: String,
    selected: bool,
}

fn resolve_rows(
    band: &[&Entry],
    first: usize,
    font: &skia_safe::Font,
    band_relative: bool,
) -> Vec<BenchRow> {
    band.iter()
        .enumerate()
        .map(|(i, e)| BenchRow {
            top: if band_relative {
                i as f32 * ROW_H
            } else {
                (first + i) as f32 * ROW_H
            },
            name: crate::view::ellipsize(font, &e.name, 320.0),
            size: if e.is_dir {
                "--".to_string()
            } else {
                e.size.map(model::format_size).unwrap_or_default()
            },
            kind: e.kind_label().to_string(),
            when: e.modified.map(model::format_time).unwrap_or_default(),
            selected: false,
        })
        .collect()
}

fn paint_rows(
    canvas: &skia_safe::Canvas,
    rows: &[BenchRow],
    font: &skia_safe::Font,
    icon: &skia_safe::Image,
) {
    use skia_safe::{Color, Paint, RRect, Rect};
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    for row in rows {
        let rect = Rect::from_xywh(0.0, row.top, PANE_W, ROW_H);
        if row.selected {
            paint.set_color(Color::from_argb(255, 60, 120, 220));
            canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0, 6.0), &paint);
        }
        canvas.draw_image_rect(
            icon,
            None,
            Rect::from_xywh(14.0, rect.center_y() - 8.0, 16.0, 16.0),
            &paint,
        );
        paint.set_color(Color::BLACK);
        let y = rect.center_y() + 5.0;
        canvas.draw_str(&row.name, (40.0, y), font, &paint);
        canvas.draw_str(&row.size, (400.0, y), font, &paint);
        canvas.draw_str(&row.kind, (520.0, y), font, &paint);
        canvas.draw_str(&row.when, (680.0, y), font, &paint);
    }
}

fn bench_icon() -> skia_safe::Image {
    use skia_safe::{surfaces, Color, Paint, Rect};
    let mut s = surfaces::raster_n32_premul((16, 16)).unwrap();
    let mut p = Paint::default();
    p.set_color(Color::from_argb(255, 90, 150, 240));
    s.canvas().draw_rect(Rect::from_wh(16.0, 16.0), &p);
    s.image_snapshot()
}

const FRAMES: usize = 240;
/// 1500 pt/s at 120 Hz — a brisk fling.
const PER_FRAME: f32 = 1500.0 / 120.0;

fn rows_visible() -> usize {
    (PANE_H / ROW_H).ceil() as usize + 1
}

/// Immediate mode: re-resolve and re-draw every visible row, every frame.
fn scroll_immediate(
    surface: &mut skia_safe::Surface,
    sorted: &[&Entry],
    font: &skia_safe::Font,
    icon: &skia_safe::Image,
    present: &mut dyn FnMut(&mut skia_safe::Surface),
) -> Duration {
    use skia_safe::Color;
    let rows_visible = rows_visible();
    let mut offset = 0.0f32;
    let start = Instant::now();
    for _ in 0..FRAMES {
        let first = (offset / ROW_H).floor() as usize;
        let last = (first + rows_visible).min(sorted.len());
        let rows = resolve_rows(&sorted[first..last], first, font, false);
        {
            let canvas = surface.canvas();
            canvas.clear(Color::WHITE);
            canvas.save();
            canvas.translate((0.0, -offset));
            paint_rows(canvas, &rows, font, icon);
            canvas.restore();
        }
        present(surface);
        offset += PER_FRAME;
    }
    start.elapsed() / FRAMES as u32
}

/// Layer mode: a `picture_cached` strip the engine moves, re-recorded only
/// when the scroll leaves the recorded band.
#[allow(clippy::too_many_arguments)]
fn scroll_layers(
    surface: &mut skia_safe::Surface,
    sorted: &[&Entry],
    font: &skia_safe::Font,
    icon: &skia_safe::Image,
    overscan: usize,
    image_cached: bool,
    frozen: bool,
    present: &mut dyn FnMut(&mut skia_safe::Surface),
) -> (Duration, usize) {
    use layers::prelude::*;
    use layers::types::{Point as LayerPoint, Size as LayerSize};
    use skia_safe::Color;

    let rows_visible = rows_visible();
    let engine = Engine::create(PANE_W, PANE_H);
    let root = engine.new_layer();
    root.set_layout_style(taffy::Style {
        position: taffy::style::Position::Absolute,
        ..Default::default()
    });
    root.set_size(LayerSize::points(PANE_W, PANE_H), None);
    root.set_clip_content(true, None);
    root.set_clip_children(true, None);
    engine.scene_set_root(root.clone());

    let strip = engine.new_layer();
    strip.set_layout_style(taffy::Style {
        position: taffy::style::Position::Absolute,
        ..Default::default()
    });
    strip.set_picture_cached(true);
    strip.set_image_cached(image_cached);
    let _ = root.add_sublayer(&strip);

    let mut recorded: Option<(usize, usize)> = None;
    let mut rerecords = 0usize;
    let mut offset = 0.0f32;
    let start = Instant::now();
    for _ in 0..FRAMES {
        let first_visible = (offset / ROW_H).floor() as usize;
        let need = (
            first_visible,
            (first_visible + rows_visible).min(sorted.len()),
        );
        let covered = recorded.is_some_and(|(lo, hi)| need.0 >= lo && need.1 <= hi);
        if !covered {
            let lo = first_visible.saturating_sub(overscan);
            let hi = (first_visible + rows_visible + overscan).min(sorted.len());
            let rows = resolve_rows(&sorted[lo..hi], lo, font, true);
            let icon = icon.clone();
            let font = font.clone();
            strip.set_size(LayerSize::points(PANE_W, (hi - lo) as f32 * ROW_H), None);
            strip.set_draw_content(move |canvas: &skia_safe::Canvas, w: f32, h: f32| {
                paint_rows(canvas, &rows, &font, &icon);
                skia_safe::Rect::from_wh(w, h)
            });
            recorded = Some((lo, hi));
            rerecords += 1;
        }
        let (lo, _) = recorded.unwrap();
        if !frozen {
            strip.set_position(LayerPoint::new(0.0, lo as f32 * ROW_H - offset), None);
        }
        engine.update(0.0);
        {
            let canvas = surface.canvas();
            canvas.clear(Color::WHITE);
            draw_scene(canvas, engine.scene(), root.id());
        }
        present(surface);
        if !frozen {
            offset += PER_FRAME;
        }
    }
    (start.elapsed() / FRAMES as u32, rerecords)
}

/// Run every arm against one surface and print the table.
fn compare_arms(
    surface: &mut skia_safe::Surface,
    sorted: &[&Entry],
    font: &skia_safe::Font,
    icon: &skia_safe::Image,
    floor: Duration,
    present: &mut dyn FnMut(&mut skia_safe::Surface),
) {
    let rows_visible = rows_visible();
    println!(
        "  {FRAMES} frames, {PER_FRAME:.1} pt/frame (1500 pt/s), {rows_visible} rows on screen\n"
    );
    let net = |d: Duration| (d.saturating_sub(floor)).as_secs_f64() * 1e6;
    // Best of N, not the mean of N: this machine runs a compositor that
    // contends for the same GPU, so a slow repetition measures the contention
    // and the fastest one measures the work.
    const REPS: usize = 5;
    let immediate = (0..REPS)
        .map(|_| scroll_immediate(surface, sorted, font, icon, present))
        .min()
        .unwrap();
    println!(
        "  immediate: resolve + paint every row every frame  {:>9.1} µs   (net {:>8.1} µs)",
        immediate.as_secs_f64() * 1e6,
        net(immediate)
    );
    let arms: [(usize, bool, bool, &str); 8] = [
        (0, false, false, "layers, picture cache: no overscan"),
        (8, false, false, "layers, picture cache: ±8 rows"),
        (
            rows_visible,
            false,
            false,
            "layers, picture cache: ±1 screen",
        ),
        (0, true, false, "layers, image cache: no overscan"),
        (8, true, false, "layers, image cache: ±8 rows"),
        (rows_visible, true, false, "layers, image cache: ±1 screen"),
        (0, false, true, "CONTROL frozen, picture cache"),
        (0, true, true, "CONTROL frozen, image cache"),
    ];
    let mut results = Vec::new();
    for (overscan, image_cached, frozen, label) in arms {
        let mut best = Duration::MAX;
        let mut rerecords = 0;
        for _ in 0..REPS {
            let (each, n) = scroll_layers(
                surface,
                sorted,
                font,
                icon,
                overscan,
                image_cached,
                frozen,
                present,
            );
            best = best.min(each);
            rerecords = n;
        }
        let each = best;
        println!(
            "  {label:<48} {:>9.1} µs   (net {:>8.1} µs, {rerecords} re-records)",
            each.as_secs_f64() * 1e6,
            net(each)
        );
        results.push((label, each));
    }
    println!();
    for (label, each) in results {
        // Ratios on the net figures: the frame-boundary floor is paid by every
        // arm alike and including it would flatter whichever is slower.
        ratio(
            label,
            Duration::from_secs_f64(net(immediate) / 1e6),
            Duration::from_secs_f64(net(each).max(1.0) / 1e6),
        );
    }
}

/// Scroll a pane 240 frames at a realistic fling speed, both ways, and compare
/// Scroll a pane 240 frames at a realistic fling speed, both ways, and compare
/// what one frame of it costs.
///
/// Both arms end up with the same pixels in the same raster surface. The only
/// difference is *who* holds the recorded content between frames: the immediate
/// arm re-resolves and re-draws every visible row every frame; the layer arm
/// resolves a band once, hands it to a `picture_cached` layer, and a frame of
/// scrolling is a `set_position` plus an engine update plus a picture replay —
/// with a re-record only when the scroll crosses out of the recorded band.
#[test]
#[ignore = "benchmark"]
fn bench_layers_vs_immediate() {
    use otto_kit::typography::styles;
    use skia_safe::surfaces;

    let all = entries(5_000);
    let sorted = visible_uncounted(&all, false, SortKey::Name, true);
    let font = styles::BODY_MEDIUM.font();
    let icon = bench_icon();
    let mut surface = surfaces::raster_n32_premul((PANE_W as i32, PANE_H as i32)).unwrap();

    println!("\n=== 7. layers vs. immediate — CPU raster surface ===");
    compare_arms(
        &mut surface,
        &sorted,
        &font,
        &icon,
        Duration::ZERO,
        &mut |_| {},
    );
}

// ---------------------------------------------------------------------------
// 8. The same comparison on the substrate the app actually runs on: a real
//    Wayland surface with a real EGL window surface and Skia's GL backend.
// ---------------------------------------------------------------------------

/// A live EGL window, built the same way `otto_kit::rendering` builds one: a
/// `wl_surface`, a `wl_egl_window` over it, a GLES2 context, and a Skia
/// surface wrapping the window's own framebuffer.
///
/// The surface is never given an `xdg_surface`, so nothing is mapped and
/// nothing is presented to a screen — but every drawing call goes through the
/// same driver, the same glyph atlas and the same window-backed framebuffer a
/// mapped otto-files window would use, which is what the measurement is about.
/// Field order matters: the EGL surface must die before the `WlEglSurface`.
#[cfg(test)]
struct EglWindow {
    skia_surface: skia_safe::Surface,
    context: skia_safe::gpu::DirectContext,
    egl: khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
    display: khronos_egl::Display,
    egl_surface: khronos_egl::Surface,
    _wl_egl: wayland_egl::WlEglSurface,
    _wl_surface: wayland_client::protocol::wl_surface::WlSurface,
    _conn: wayland_client::Connection,
}

#[cfg(test)]
struct EglWindowState;

#[cfg(test)]
mod egl_dispatch {
    use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
    impl
        wayland_client::Dispatch<
            wl_registry::WlRegistry,
            wayland_client::globals::GlobalListContents,
        > for super::EglWindowState
    {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &wayland_client::globals::GlobalListContents,
            _: &wayland_client::Connection,
            _: &wayland_client::QueueHandle<Self>,
        ) {
        }
    }
    wayland_client::delegate_noop!(super::EglWindowState: ignore wl_compositor::WlCompositor);
    wayland_client::delegate_noop!(super::EglWindowState: ignore wl_surface::WlSurface);
}

#[cfg(test)]
impl EglWindow {
    fn new(width: i32, height: i32) -> Result<Self, String> {
        use wayland_client::protocol::wl_compositor::WlCompositor;
        use wayland_client::{globals::registry_queue_init, Connection, Proxy};

        let conn = Connection::connect_to_env().map_err(|e| format!("no wayland: {e}"))?;
        let (globals, mut queue) =
            registry_queue_init::<EglWindowState>(&conn).map_err(|e| format!("registry: {e}"))?;
        let qh = queue.handle();
        let compositor: WlCompositor = globals
            .bind(&qh, 1..=6, ())
            .map_err(|e| format!("wl_compositor: {e}"))?;
        let wl_surface = compositor.create_surface(&qh, ());
        queue.roundtrip(&mut EglWindowState).ok();

        unsafe {
            let egl = khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required()
                .map_err(|e| format!("load egl: {e}"))?;
            let display = egl
                .get_display(conn.backend().display_ptr() as *mut std::ffi::c_void)
                .ok_or("no egl display")?;
            egl.initialize(display).map_err(|e| format!("init: {e}"))?;

            let config_attribs = [
                khronos_egl::SURFACE_TYPE,
                khronos_egl::WINDOW_BIT,
                khronos_egl::RED_SIZE,
                8,
                khronos_egl::GREEN_SIZE,
                8,
                khronos_egl::BLUE_SIZE,
                8,
                khronos_egl::ALPHA_SIZE,
                8,
                khronos_egl::RENDERABLE_TYPE,
                khronos_egl::OPENGL_ES2_BIT,
                khronos_egl::NONE,
            ];
            let config = egl
                .choose_first_config(display, &config_attribs)
                .map_err(|e| format!("choose config: {e}"))?
                .ok_or("no config")?;
            egl.bind_api(khronos_egl::OPENGL_ES_API)
                .map_err(|e| format!("bind api: {e}"))?;
            let context_attribs = [khronos_egl::CONTEXT_CLIENT_VERSION, 2, khronos_egl::NONE];
            let context = egl
                .create_context(display, config, None, &context_attribs)
                .map_err(|e| format!("context: {e}"))?;

            let wl_egl = wayland_egl::WlEglSurface::new(wl_surface.id(), width, height)
                .map_err(|e| format!("wl_egl: {e:?}"))?;
            let egl_surface = egl
                .create_window_surface(
                    display,
                    config,
                    wl_egl.ptr() as khronos_egl::NativeWindowType,
                    None,
                )
                .map_err(|e| format!("window surface: {e}"))?;
            egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(context))
                .map_err(|e| format!("make current: {e}"))?;
            // No vsync: the measurement is of work done, not of waiting for a
            // vblank that an unmapped surface would never get anyway.
            egl.swap_interval(display, 0).ok();

            let interface = skia_safe::gpu::gl::Interface::new_load_with(|name| {
                egl.get_proc_address(name)
                    .map(|p| p as *const std::ffi::c_void)
                    .unwrap_or(std::ptr::null())
            })
            .ok_or("skia gl interface")?;
            let mut skia_context = skia_safe::gpu::direct_contexts::make_gl(interface, None)
                .ok_or("direct context")?;

            let fb_info = skia_safe::gpu::gl::FramebufferInfo {
                // A window-backed EGL surface renders to framebuffer 0.
                fboid: 0,
                format: skia_safe::gpu::gl::Format::RGBA8.into(),
                protected: skia_safe::gpu::Protected::No,
            };
            let target =
                skia_safe::gpu::backend_render_targets::make_gl((width, height), 0, 8, fb_info);
            let skia_surface = skia_safe::gpu::surfaces::wrap_backend_render_target(
                &mut skia_context,
                &target,
                skia_safe::gpu::SurfaceOrigin::BottomLeft,
                skia_safe::ColorType::RGBA8888,
                None,
                None,
            )
            .ok_or("wrap render target")?;

            Ok(Self {
                skia_surface,
                context: skia_context,
                egl,
                display,
                egl_surface,
                _wl_egl: wl_egl,
                _wl_surface: wl_surface,
                _conn: conn,
            })
        }
    }

    /// End a frame the way the app does — flush the recorded work to the GPU
    /// and wait for it, so a frame's cost is the work and not the queueing.
    fn present(&mut self, surface: &mut skia_safe::Surface) {
        self.context
            .flush_and_submit_surface(surface, skia_safe::gpu::SyncCpu::Yes);
        self.egl.swap_buffers(self.display, self.egl_surface).ok();
    }
}

#[test]
#[ignore = "benchmark"]
fn bench_layers_vs_immediate_egl() {
    use otto_kit::typography::styles;

    println!("\n=== 8. layers vs. immediate — real EGL window, Skia GL backend ===");
    let mut window = match EglWindow::new(PANE_W as i32, PANE_H as i32) {
        Ok(w) => w,
        Err(e) => {
            println!("  skipped: {e}");
            return;
        }
    };
    println!("  EGL window up: {}×{} GLES2 RGBA8\n", PANE_W, PANE_H);

    let floor;
    // Floor: what a frame costs with no content at all. If this is close to
    // the arms below, the frame boundary — not the drawing — is what is being
    // measured, and the comparison means nothing.
    {
        let mut s = window.skia_surface.clone();
        let start = Instant::now();
        for _ in 0..FRAMES {
            s.canvas().clear(skia_safe::Color::WHITE);
            window
                .context
                .flush_and_submit_surface(&mut s, skia_safe::gpu::SyncCpu::Yes);
        }
        let flush_only = start.elapsed() / FRAMES as u32;
        let start = Instant::now();
        for _ in 0..FRAMES {
            s.canvas().clear(skia_safe::Color::WHITE);
            window.present(&mut s);
        }
        let with_swap = start.elapsed() / FRAMES as u32;
        println!(
            "  FLOOR empty frame, clear + flush + gpu sync       {:>9.1} µs/frame",
            flush_only.as_secs_f64() * 1e6
        );
        println!(
            "  FLOOR empty frame, + eglSwapBuffers               {:>9.1} µs/frame\n",
            with_swap.as_secs_f64() * 1e6
        );
        floor = with_swap;
    }

    let all = entries(5_000);
    let sorted = visible_uncounted(&all, false, SortKey::Name, true);
    let font = styles::BODY_MEDIUM.font();
    let icon = bench_icon();

    // The surface is borrowed by the arms, so hand them a clone of the handle
    // and keep the window for presenting.
    let mut surface = window.skia_surface.clone();
    let window_ptr: *mut EglWindow = &mut window;
    let mut present = move |s: &mut skia_safe::Surface| {
        // Safe in practice: `present` is only called from the arms below, on
        // this thread, and nothing else touches the window while they run.
        unsafe { (*window_ptr).present(s) };
    };
    compare_arms(&mut surface, &sorted, &font, &icon, floor, &mut present);
}
