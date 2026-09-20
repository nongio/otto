//! Dock overlays — what each app's dock icon says about itself.
//!
//! otto-islands is the session's notification daemon and the home of its
//! activities, so it is the only thing that knows both how many notifications
//! an app has outstanding and what it is busy doing. It publishes both through
//! `otto_dock_v1`: one dock item per app, its badge set to the number of
//! notifications still waiting to be read, and its progress set to how far
//! along that app's running tasks are.
//!
//! The island is the glance and the dock is the ambient reading: an island
//! shrinks to a dot or goes away entirely, while the dock icon carries a task
//! for as long as it runs, from wherever the user happens to be looking.

use std::collections::HashMap;

use otto_kit::protocols::otto_dock_item_v1::OttoDockItemV1;
use otto_kit::AppContext;

use crate::activity::{Activity, ActivitySource};

/// Counts above this are shown as "99+" — a dock badge is a glance, not a
/// readout, and three digits do not fit the circle.
const MAX_COUNT: usize = 99;

#[derive(Default)]
pub struct DockOverlays {
    /// One dock item per app, kept for the lifetime of the process: the
    /// protocol has no destroy request, and the same app badges repeatedly.
    items: HashMap<String, OttoDockItemV1>,
    /// Badge text currently applied per app — the diff that keeps a redraw
    /// storm off the compositor when nothing about the counts changed.
    applied: HashMap<String, String>,
    /// Progress currently applied per app, diffed the same way. A task that
    /// reports every file of a thousand must not become a thousand dock
    /// redraws, so a move smaller than [`PROGRESS_STEP`] is not worth sending.
    applied_progress: HashMap<String, f64>,
}

/// The smallest progress change worth a dock redraw. A bar this wide on a
/// dock icon is a pixel or two, so finer updates would cost a redraw and show
/// nothing.
const PROGRESS_STEP: f64 = 0.01;

impl DockOverlays {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconcile every dock badge and progress bar with `activities`.
    ///
    /// Cheap to call on every state change: only apps whose count actually
    /// moved produce a request.
    pub fn sync(&mut self, activities: &[Activity]) {
        let wanted = badge_texts(activities);
        let mut changed = false;

        for (app_id, text) in &wanted {
            if self.applied.get(app_id) == Some(text) {
                continue;
            }
            let Some(item) = self.item(app_id) else {
                continue;
            };
            item.set_badge(Some(text.clone()));
            self.applied.insert(app_id.clone(), text.clone());
            changed = true;
        }

        // Apps whose last notification just went away.
        let cleared: Vec<String> = self
            .applied
            .keys()
            .filter(|app_id| !wanted.contains_key(app_id.as_str()))
            .cloned()
            .collect();
        for app_id in cleared {
            if let Some(item) = self.items.get(&app_id) {
                item.set_badge(None);
            }
            self.applied.remove(&app_id);
            changed = true;
        }

        changed |= self.sync_progress(activities);

        if changed {
            tracing::debug!(badges = ?self.applied, progress = ?self.applied_progress, "dock overlays updated");
            AppContext::flush();
        }
    }

    /// Reconcile the progress bar on every app's dock icon.
    fn sync_progress(&mut self, activities: &[Activity]) -> bool {
        let wanted = progress_values(activities);
        let mut changed = false;

        for (app_id, value) in &wanted {
            let settled = self
                .applied_progress
                .get(app_id)
                .is_some_and(|applied| (applied - value).abs() < PROGRESS_STEP);
            // The end of a task is always worth sending, however small the
            // step into it: a bar that stops just short of full looks stuck.
            if settled && *value < 1.0 {
                continue;
            }
            let Some(item) = self.item(app_id) else {
                continue;
            };
            item.set_progress(*value);
            self.applied_progress.insert(app_id.clone(), *value);
            changed = true;
        }

        // Apps whose last task just finished.
        let cleared: Vec<String> = self
            .applied_progress
            .keys()
            .filter(|app_id| !wanted.contains_key(app_id.as_str()))
            .cloned()
            .collect();
        for app_id in cleared {
            if let Some(item) = self.items.get(&app_id) {
                item.set_progress(-1.0);
            }
            self.applied_progress.remove(&app_id);
            changed = true;
        }

        changed
    }

    /// The dock item for `app_id`, created on first use.
    ///
    /// `None` on a compositor without `otto_dock_v1` — badges are then simply
    /// not shown, which is the right outcome for an optional decoration.
    fn item(&mut self, app_id: &str) -> Option<&OttoDockItemV1> {
        if !self.items.contains_key(app_id) {
            let manager = AppContext::otto_dock_manager()?;
            // `AppRunner` wraps the app in `DefaultApp`, so this is the queue
            // handle the dispatch state is actually typed for.
            let qh = AppContext::queue_handle();
            let item = manager.get_dock_item(app_id.to_string(), qh, ());
            self.items.insert(app_id.to_string(), item);
        }
        self.items.get(app_id)
    }
}

/// The badge text each app should be showing for `activities`.
fn badge_texts(activities: &[Activity]) -> HashMap<String, String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for activity in activities.iter().filter(|a| badgeable(a)) {
        *counts.entry(activity.app_id.as_str()).or_insert(0) += 1;
    }

    counts
        .into_iter()
        .map(|(app_id, count)| {
            let text = if count > MAX_COUNT {
                format!("{MAX_COUNT}+")
            } else {
                count.to_string()
            };
            (app_id.to_string(), text)
        })
        .collect()
}

/// The progress each app's dock icon should be showing for `activities`.
///
/// Only a live activity with a progress value counts: a notification is
/// something that happened, not something still going on. An app running
/// several tasks at once gets the mean of them, so its icon fills as its work
/// does and empties when the last one ends.
fn progress_values(activities: &[Activity]) -> HashMap<String, f64> {
    let mut totals: HashMap<&str, (f64, usize)> = HashMap::new();
    for activity in activities.iter().filter(|a| !a.app_id.is_empty()) {
        let (Some(progress), true) = (activity.progress, activity.live) else {
            continue;
        };
        let entry = totals.entry(activity.app_id.as_str()).or_insert((0.0, 0));
        entry.0 += progress.clamp(0.0, 1.0);
        entry.1 += 1;
    }

    totals
        .into_iter()
        .map(|(app_id, (sum, count))| (app_id.to_string(), sum / count as f64))
        .collect()
}

/// Whether an activity counts towards its app's badge.
///
/// Only real notifications do: an internal activity (a media island, a
/// progress readout) is not something the user has to come back to. Transient
/// notifications opt out of persistence by definition, so they never badge.
fn badgeable(activity: &Activity) -> bool {
    activity.source == ActivitySource::Notification
        && !activity.transient
        && !activity.app_id.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Priority;
    use std::time::Instant;

    fn notification(app_id: &str) -> Activity {
        Activity {
            id: 0,
            app_id: app_id.to_string(),
            title: String::new(),
            body: String::new(),
            icon: String::new(),
            progress: None,
            timeout_ms: 0,
            priority: Priority::Normal,
            live: false,
            quiet: false,
            created_at: Instant::now(),
            expired: false,
            actions: Vec::new(),
            default_action: None,
            category: None,
            image_path: None,
            transient: false,
            resident: false,
            notification_id: Some(1),
            source: ActivitySource::Notification,
        }
    }

    fn task(app_id: &str, progress: f64) -> Activity {
        Activity {
            live: true,
            progress: Some(progress),
            notification_id: None,
            source: ActivitySource::DBus,
            ..notification(app_id)
        }
    }

    #[test]
    fn a_running_task_fills_its_app_s_dock_icon() {
        assert_eq!(
            progress_values(&[task("otto-files", 0.25)]).get("otto-files"),
            Some(&0.25)
        );
    }

    #[test]
    fn two_tasks_of_one_app_share_one_bar() {
        let values = progress_values(&[task("otto-files", 0.2), task("otto-files", 0.8)]);
        assert_eq!(values.get("otto-files"), Some(&0.5));
    }

    #[test]
    fn a_quiet_task_still_fills_the_dock() {
        // Quiet is about the island. The dock is the ambient reading, and an
        // app the user is looking at is exactly the one whose icon should
        // show what it is busy with.
        let mut quiet = task("otto-files", 0.4);
        quiet.quiet = true;
        assert_eq!(progress_values(&[quiet]).get("otto-files"), Some(&0.4));
    }

    #[test]
    fn a_notification_is_not_a_task() {
        // Something that happened has nothing left to run, so it must not put
        // a bar on the icon — and a task with no progress yet has nothing to
        // draw either.
        let mut indeterminate = task("otto-files", 0.0);
        indeterminate.progress = None;
        assert!(progress_values(&[notification("ghostty"), indeterminate]).is_empty());
    }

    #[test]
    fn counts_notifications_per_app() {
        let activities = vec![
            notification("ghostty"),
            notification("ghostty"),
            notification("thunderbird"),
        ];
        let texts = badge_texts(&activities);
        assert_eq!(texts.get("ghostty"), Some(&"2".to_string()));
        assert_eq!(texts.get("thunderbird"), Some(&"1".to_string()));
    }

    #[test]
    fn transient_and_internal_activities_do_not_badge() {
        let mut transient = notification("ghostty");
        transient.transient = true;
        let mut internal = notification("music");
        internal.source = ActivitySource::Internal;

        assert!(badge_texts(&[transient, internal]).is_empty());
    }

    #[test]
    fn a_hundred_notifications_read_as_99_plus() {
        let activities: Vec<Activity> = (0..100).map(|_| notification("ghostty")).collect();
        assert_eq!(
            badge_texts(&activities).get("ghostty"),
            Some(&"99+".to_string())
        );
    }

    #[test]
    fn an_expired_notification_still_badges() {
        // Expiring only takes the island off the row; the notification has
        // still not been read, which is exactly what the badge counts.
        let mut expired = notification("ghostty");
        expired.expired = true;
        assert_eq!(
            badge_texts(&[expired]).get("ghostty"),
            Some(&"1".to_string())
        );
    }
}
