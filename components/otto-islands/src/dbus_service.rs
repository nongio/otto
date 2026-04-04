use otto_kit::AppContext;
use zbus::interface;

use crate::activity::Priority;
use crate::state::SharedState;

pub const DBUS_NAME: &str = "org.otto.Island";
pub const DBUS_PATH: &str = "/org/otto/Island";

pub struct IslandService {
    state: SharedState,
}

impl IslandService {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[interface(name = "org.otto.Island1")]
impl IslandService {
    /// Create a new activity in the island.
    ///
    /// Returns the activity ID on success.
    /// `progress`: 0.0–1.0 for a progress bar, negative for no progress.
    /// `priority`: "low", "normal", "high", or "critical".
    async fn create_activity(
        &self,
        app_id: &str,
        title: &str,
        icon: &str,
        progress: f64,
        timeout_ms: u32,
        priority: &str,
        live: bool,
    ) -> zbus::fdo::Result<u64> {
        let priority =
            Priority::try_from(priority).map_err(|e| zbus::fdo::Error::InvalidArgs(e))?;

        let progress = if progress < 0.0 {
            None
        } else {
            Some(progress.clamp(0.0, 1.0))
        };

        let mut state = self.state.lock().unwrap();
        let id = state.create_activity(
            app_id.to_string(),
            title.to_string(),
            icon.to_string(),
            progress,
            timeout_ms,
            priority,
            live,
        );
        drop(state);

        AppContext::request_wakeup();
        tracing::info!(id, app_id, title, "activity created");
        Ok(id)
    }

    /// Update an existing activity's title and/or progress.
    ///
    /// Pass an empty string for title to leave it unchanged.
    /// Pass a negative value for progress to clear it.
    async fn update_activity(
        &self,
        id: u64,
        title: &str,
        progress: f64,
    ) -> zbus::fdo::Result<bool> {
        let mut state = self.state.lock().unwrap();
        let ok = state.update_activity(id, title, progress);
        drop(state);

        if ok {
            AppContext::request_wakeup();
        }
        Ok(ok)
    }

    /// Dismiss an activity by ID.
    async fn dismiss_activity(&self, id: u64) -> zbus::fdo::Result<bool> {
        let mut state = self.state.lock().unwrap();
        let ok = state.dismiss_activity(id);
        drop(state);

        if ok {
            AppContext::request_wakeup();
            tracing::info!(id, "activity dismissed");
        }
        Ok(ok)
    }
}
