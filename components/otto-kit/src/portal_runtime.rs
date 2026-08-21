//! Where the portal watchers run.
//!
//! The watchers are async (zbus signal streams), but an otto-kit app is not
//! required to be: `otto-settings` has a plain `fn main`, and gating the
//! watchers on a tokio runtime being present is what left it ignoring the
//! user's colour scheme and accent. Each watcher asks for a home instead —
//! the app's runtime when there is one, a dedicated thread with a minimal
//! runtime when there is not.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

/// Bumped whenever a watcher changes something the theme is built from.
///
/// The watchers run off the main thread and cannot call into the app, so the
/// run loop reads this instead and turns a change into `App::on_theme_changed`.
static THEME_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Note that the theme's inputs changed, and wake the run loop to notice.
pub(crate) fn theme_changed() {
    THEME_GENERATION.fetch_add(1, Ordering::Relaxed);
    crate::app_runner::AppContext::request_wakeup();
}

/// The current generation, for a caller keeping track of what it last saw.
pub(crate) fn theme_generation() -> u64 {
    THEME_GENERATION.load(Ordering::Relaxed)
}

/// Run `task` in the background, wherever it can be run.
///
/// The thread is named after the watcher, so a stuck one is identifiable in a
/// backtrace rather than being one of several anonymous tokio threads.
pub(crate) fn spawn<F>(name: &'static str, task: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(task);
        return;
    }

    let spawned = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(task),
                Err(err) => tracing::warn!("{name}: no runtime to watch on ({err})"),
            }
        });

    if let Err(err) = spawned {
        tracing::warn!("{name}: could not start ({err})");
    }
}
