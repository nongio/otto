//! Lock the session, hold it for a moment, then unlock — with no PAM in the
//! way. What it exercises is the protocol lifecycle: acquire, surface,
//! `locked`, unlock, exit.
//!
//! ```sh
//! cargo run -p otto-lock --example lock_cycle           # 3 seconds
//! HOLD=10 cargo run -p otto-lock --example lock_cycle   # longer
//! ```
//!
//! Run it against a nested compositor (`otto --winit`) rather than the session
//! you are sitting in: it locks whatever `$WAYLAND_DISPLAY` points at, and a
//! lock that fails to unlock is a lock you can only leave by switching VT.

use otto_kit::surfaces::{SessionLock, SessionLockSurface};
use otto_kit::{App, AppContext, AppRunner};
use wayland_client::protocol::wl_output::WlOutput;

struct LockCycle {
    lock: Option<SessionLock>,
    surfaces: Vec<(WlOutput, SessionLockSurface)>,
    /// When the session was hidden. The unlock is timed from here, so the hold
    /// is a hold on a locked screen rather than on a request in flight.
    locked_at: Option<std::time::Instant>,
    hold: std::time::Duration,
}

impl App for LockCycle {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        AppContext::enable_layer_engine(1920.0, 1080.0);
        self.lock = Some(SessionLock::acquire()?);
        Ok(())
    }

    fn on_session_locked(&mut self, _ctx: &AppContext) {
        tracing::info!("locked; holding for {:?}", self.hold);
        self.locked_at = Some(std::time::Instant::now());
    }

    fn on_session_lock_finished(&mut self, _ctx: &AppContext) {
        tracing::error!("the compositor refused to lock");
        if let Some(lock) = self.lock.as_ref() {
            lock.abandon();
        }
        AppContext::request_exit();
    }

    fn on_update(&mut self, ctx: &AppContext) {
        let Some(lock) = self.lock.as_ref() else {
            return;
        };
        if lock.is_released() {
            return;
        }

        for output in ctx.output_state_ref().outputs() {
            if self.surfaces.iter().any(|(known, _)| *known == output) {
                continue;
            }
            match lock.surface_for(&output) {
                // Nothing is painted into it: an unpainted lock surface leaves
                // the compositor's own blank up, which is the state this is
                // here to check the way out of.
                Ok(surface) => {
                    tracing::info!("lock surface created");
                    self.surfaces.push((output, surface));
                }
                Err(err) => tracing::error!(%err, "could not create a lock surface"),
            }
        }

        if self.locked_at.is_some_and(|at| at.elapsed() >= self.hold) {
            tracing::info!("unlocking");
            lock.unlock();
            AppContext::request_exit();
        }
    }

    fn idle_timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_millis(200))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let hold = std::env::var("HOLD")
        .ok()
        .and_then(|secs| secs.parse().ok())
        .unwrap_or(3);

    AppRunner::new(LockCycle {
        lock: None,
        surfaces: Vec::new(),
        locked_at: None,
        hold: std::time::Duration::from_secs(hold),
    })
    .run()?;
    Ok(())
}
