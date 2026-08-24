//! Dock badges — the unread notification count on each app's dock icon.
//!
//! otto-islands is the session's notification daemon, so it is the only thing
//! that knows how many notifications an app has outstanding. It publishes that
//! count through `otto_dock_v1`: one dock item per app, its badge set to the
//! number of notifications still waiting to be read. The count drops as the
//! user dismisses them and the badge disappears with the last one.

use std::collections::HashMap;

use otto_kit::protocols::otto_dock_item_v1::OttoDockItemV1;
use otto_kit::AppContext;

use crate::activity::{Activity, ActivitySource};

/// Counts above this are shown as "99+" — a dock badge is a glance, not a
/// readout, and three digits do not fit the circle.
const MAX_COUNT: usize = 99;

#[derive(Default)]
pub struct DockBadges {
    /// One dock item per app, kept for the lifetime of the process: the
    /// protocol has no destroy request, and the same app badges repeatedly.
    items: HashMap<String, OttoDockItemV1>,
    /// Badge text currently applied per app — the diff that keeps a redraw
    /// storm off the compositor when nothing about the counts changed.
    applied: HashMap<String, String>,
}

impl DockBadges {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconcile every dock badge with `activities`.
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

        if changed {
            tracing::debug!(badges = ?self.applied, "dock badges updated");
            AppContext::flush();
        }
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
