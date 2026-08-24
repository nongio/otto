//! Scripted 3-finger swipe driver for testing (`/tmp/otto-gesture`).
//!
//! A touchpad gesture cannot be synthesized through uinput the way a key press
//! can, which leaves the expose and workspace-switch transitions untestable
//! without a human finger. This drives the same backend-independent entry
//! points libinput's handler calls (`gesture_swipe_begin_3finger` /
//! `gesture_swipe_update` / `gesture_swipe_end`), so a scripted swipe runs the
//! real gesture state machine, animations and render paths — only libinput's
//! event decoding is skipped.
//!
//! One gesture per line in the script file:
//!
//! ```text
//! <dx> <dy> <steps> [settle]
//! ```
//!
//! `steps` updates of `(dx, dy)` between a begin and an end, then `settle` idle
//! ticks before the next line. Overshoot by making `steps * dy` far exceed the
//! distance that opens expose fully; repeat a swipe by repeating the line.

use std::collections::VecDeque;
use std::sync::Mutex;

/// Default script path. `OTTO_GESTURE_FILE` overrides it, so a nested test
/// compositor can be driven without the udev session on the same machine
/// picking the script up and swiping the user's real desktop.
const DEFAULT_SCRIPT_PATH: &str = "/tmp/otto-gesture";

fn script_path() -> String {
    std::env::var("OTTO_GESTURE_FILE").unwrap_or_else(|_| DEFAULT_SCRIPT_PATH.to_string())
}

enum Step {
    Begin,
    Update(f64, f64),
    End,
    /// Idle tick — lets animations and the render loop advance between
    /// gestures, which is where the settle bugs live.
    Settle,
}

static QUEUE: Mutex<VecDeque<Step>> = Mutex::new(VecDeque::new());

/// Parse the script file into `queue` and delete it, so one write runs once.
fn load_script(queue: &mut VecDeque<Step>) {
    let path = script_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let _ = std::fs::remove_file(&path);

    let mut gestures = 0usize;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 {
            continue;
        }
        let (Ok(dx), Ok(dy), Ok(steps)) = (
            f[0].parse::<f64>(),
            f[1].parse::<f64>(),
            f[2].parse::<usize>(),
        ) else {
            continue;
        };
        let settle = f.get(3).and_then(|s| s.parse::<usize>().ok()).unwrap_or(60);

        queue.push_back(Step::Begin);
        for _ in 0..steps {
            queue.push_back(Step::Update(dx, dy));
        }
        queue.push_back(Step::End);
        for _ in 0..settle {
            queue.push_back(Step::Settle);
        }
        gestures += 1;
    }

    if gestures > 0 {
        tracing::info!(
            target: "otto::gesture",
            "synthetic gestures loaded: {gestures} ({} steps)",
            queue.len()
        );
    }
}

/// Advance one step per call. Drive from a timer at roughly frame rate.
pub fn tick<B: crate::state::Backend>(state: &mut crate::Otto<B>) {
    let step = {
        let mut queue = QUEUE.lock().unwrap();
        if queue.is_empty() {
            load_script(&mut queue);
        }
        queue.pop_front()
    };

    match step {
        Some(Step::Begin) => state.gesture_swipe_begin_3finger(),
        Some(Step::Update(dx, dy)) => state.gesture_swipe_update(dx, dy),
        Some(Step::End) => state.gesture_swipe_end(false),
        Some(Step::Settle) | None => {}
    }
}
