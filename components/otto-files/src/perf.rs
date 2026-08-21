//! Where a frame's time went, when anyone asks.
//!
//! Off unless `OTTO_FILES_PERF` is set, and cheap enough when off that the
//! call sites can stay in the hot path: one relaxed atomic load, no clock
//! read. The point is to be able to answer "what is spinning the fans" on the
//! installed binary, on the user's real directories, without another build and
//! another fingerprint to install it.
//!
//! Set `OTTO_FILES_PERF=1` and every 120 frames a line lands on stderr with
//! the mean and worst microseconds of each stage:
//!
//! ```text
//! otto-files perf: 120 frames | scene.update 41µs/312µs | scene.render 890µs/2104µs | chrome 233µs/501µs
//! ```
//!
//! `scene.update` is the keying and the engine tick; `scene.render` is the
//! scene replay; `chrome` is everything [`crate::view::draw`] still paints
//! immediately. A stage that dominates while scrolling is the one to chase.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// How many frames to fold into one line.
const WINDOW: u64 = 120;

#[derive(Clone, Copy)]
pub enum Stage {
    Prep = 0,
    FrameBuild = 1,
    Visible = 2,
    SceneUpdate = 3,
    SceneRender = 4,
    Chrome = 5,
    Total = 6,
}

const STAGES: usize = 7;
const NAMES: [&str; STAGES] = [
    "prep",
    "frame",
    "  visible",
    "scene.update",
    "scene.render",
    "chrome",
    "TOTAL",
];

static ENABLED: OnceLock<bool> = OnceLock::new();
static FRAMES: AtomicU64 = AtomicU64::new(0);
#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
static TOTAL_US: [AtomicU64; STAGES] = [ZERO; STAGES];
static WORST_US: [AtomicU64; STAGES] = [ZERO; STAGES];
/// Set once the window's last stage has been folded in, so the report is
/// emitted by exactly one stage and reads a complete set of totals.
static REPORTING: AtomicBool = AtomicBool::new(false);

fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("OTTO_FILES_PERF").is_some())
}

/// A clock read, or `None` when the probe is off — which is what keeps the
/// call sites free in the normal case.
pub fn now() -> Option<Instant> {
    enabled().then(Instant::now)
}

/// Fold the time since `start` into `stage`, and report once a window is full.
pub fn mark(stage: Stage, start: Option<Instant>) {
    let Some(start) = start else { return };
    let index = stage as usize;
    let micros = start.elapsed().as_micros() as u64;

    TOTAL_US[index].fetch_add(micros, Ordering::Relaxed);
    WORST_US[index].fetch_max(micros, Ordering::Relaxed);

    // The last stage of a frame closes it, so a window is a whole number of
    // frames and the means below divide by something real.
    if !matches!(stage, Stage::Total) {
        return;
    }
    if FRAMES.fetch_add(1, Ordering::Relaxed) + 1 < WINDOW {
        return;
    }
    if REPORTING.swap(true, Ordering::Relaxed) {
        return;
    }

    let frames = FRAMES.swap(0, Ordering::Relaxed);
    let report: Vec<String> = (0..STAGES)
        .map(|i| {
            let total = TOTAL_US[i].swap(0, Ordering::Relaxed);
            let worst = WORST_US[i].swap(0, Ordering::Relaxed);
            format!("{} {}µs/{}µs", NAMES[i], total / frames.max(1), worst)
        })
        .collect();
    eprintln!(
        "otto-files perf: {} frames | {}",
        frames,
        report.join(" | ")
    );
    REPORTING.store(false, Ordering::Relaxed);
}
