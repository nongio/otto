use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::activity::{Activity, ActivityId, ActivitySource, NotificationAction, Priority};

pub type SharedState = Arc<Mutex<IslandState>>;

/// Counter for generating unique org.freedesktop.Notifications IDs.
static NEXT_NOTIFICATION_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

pub struct IslandState {
    next_id: ActivityId,
    pub activities: Vec<Activity>,
    pub dirty: bool,
}

impl IslandState {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            activities: Vec::new(),
            dirty: false,
        }
    }

    fn next_activity_id(&mut self) -> ActivityId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn create_activity(
        &mut self,
        app_id: String,
        title: String,
        icon: String,
        progress: Option<f64>,
        timeout_ms: u32,
        priority: Priority,
        live: bool,
    ) -> ActivityId {
        let id = self.next_activity_id();
        self.activities.push(Activity {
            id,
            app_id,
            title,
            body: String::new(),
            icon,
            progress,
            timeout_ms,
            priority,
            live,
            created_at: Instant::now(),
            expired: false,
            actions: Vec::new(),
            default_action: None,
            category: None,
            image_path: None,
            transient: false,
            resident: false,
            notification_id: None,
            source: ActivitySource::DBus,
        });
        self.dirty = true;
        id
    }

    /// Create an activity from an org.freedesktop.Notifications Notify call.
    /// Returns (activity_id, notification_id).
    pub fn create_notification(
        &mut self,
        app_id: String,
        replaces_id: u32,
        icon: String,
        title: String,
        body: String,
        actions: Vec<NotificationAction>,
        priority: Priority,
        timeout_ms: u32,
        category: Option<String>,
        image_path: Option<String>,
        transient: bool,
        resident: bool,
        default_action: Option<String>,
    ) -> (ActivityId, u32) {
        // If replaces_id > 0, update existing notification
        if replaces_id > 0 {
            if let Some(activity) = self
                .activities
                .iter_mut()
                .find(|a| a.notification_id == Some(replaces_id))
            {
                activity.title = title;
                activity.body = body;
                activity.icon = icon;
                activity.actions = actions;
                activity.priority = priority;
                activity.category = category;
                activity.image_path = image_path;
                activity.default_action = default_action;
                if timeout_ms > 0 {
                    activity.timeout_ms = timeout_ms;
                    activity.created_at = Instant::now();
                    activity.expired = false;
                }
                self.dirty = true;
                return (activity.id, replaces_id);
            }
        }

        let notification_id =
            NEXT_NOTIFICATION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = self.next_activity_id();
        self.activities.push(Activity {
            id,
            app_id,
            title,
            body,
            icon,
            progress: None,
            timeout_ms,
            priority,
            live: false,
            created_at: Instant::now(),
            expired: false,
            actions,
            default_action,
            category,
            image_path,
            transient,
            resident,
            notification_id: Some(notification_id),
            source: ActivitySource::Notification,
        });
        self.dirty = true;
        (id, notification_id)
    }

    /// Dismiss a notification by its org.freedesktop.Notifications ID.
    pub fn dismiss_notification(&mut self, notification_id: u32) -> bool {
        let len_before = self.activities.len();
        self.activities
            .retain(|a| a.notification_id != Some(notification_id));
        if self.activities.len() != len_before {
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn update_activity(&mut self, id: ActivityId, title: &str, progress: f64) -> bool {
        if let Some(activity) = self.activities.iter_mut().find(|a| a.id == id) {
            let mut changed = false;
            if !title.is_empty() && activity.title != title {
                activity.title = title.to_string();
                changed = true;
            }
            let new_progress = if progress < 0.0 {
                None
            } else {
                Some(progress.clamp(0.0, 1.0))
            };
            if activity.progress != new_progress {
                activity.progress = new_progress;
                changed = true;
            }
            if changed {
                self.dirty = true;
            }
            true
        } else {
            false
        }
    }

    pub fn dismiss_activity(&mut self, id: ActivityId) -> bool {
        let len_before = self.activities.len();
        self.activities.retain(|a| a.id != id);
        if self.activities.len() != len_before {
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Mark expired activities instead of removing them.
    /// This triggers a refocus: the expired activity shrinks and the
    /// previous one expands.
    pub fn check_expired_refocus(&mut self) {
        let now = Instant::now();
        for activity in &mut self.activities {
            if activity.timeout_ms > 0
                && !activity.expired
                && now.duration_since(activity.created_at).as_millis()
                    >= activity.timeout_ms as u128
            {
                activity.expired = true;
                self.dirty = true;
            }
        }
    }

    /// Returns activities grouped for layout purposes.
    /// Notifications from the same app_id are grouped — only the most recent
    /// is returned as the representative, with a count of how many are in the group.
    /// Non-notification activities (music, D-Bus, internal) are returned as-is.
    pub fn grouped_activities(&self) -> Vec<(Activity, usize)> {
        use std::collections::HashMap;

        let mut notification_groups: HashMap<&str, Vec<&Activity>> = HashMap::new();
        let mut result: Vec<(Activity, usize)> = Vec::new();

        // Separate notifications from other activities
        for activity in &self.activities {
            if activity.source == ActivitySource::Notification {
                notification_groups
                    .entry(&activity.app_id)
                    .or_default()
                    .push(activity);
            } else {
                result.push((activity.clone(), 1));
            }
        }

        // For each notification group, take the most recent as representative
        for (_app_id, mut group) in notification_groups {
            group.sort_by_key(|a| a.created_at);
            let count = group.len();
            if let Some(latest) = group.last() {
                result.push(((*latest).clone(), count));
            }
        }

        // Sort by creation time so layout order is stable
        result.sort_by_key(|(a, _)| a.created_at);
        result
    }

    /// Get all notifications for a given app_id, ordered by creation time (newest first).
    pub fn notifications_for_app(&self, app_id: &str) -> Vec<&Activity> {
        let mut result: Vec<_> = self
            .activities
            .iter()
            .filter(|a| a.source == ActivitySource::Notification && a.app_id == app_id)
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        result
    }

    /// Remove all activities whose timeout has expired.
    pub fn expire_timeouts(&mut self) {
        let now = Instant::now();
        let len_before = self.activities.len();
        self.activities.retain(|a| {
            a.timeout_ms == 0 || now.duration_since(a.created_at).as_millis() < a.timeout_ms as u128
        });
        if self.activities.len() != len_before {
            self.dirty = true;
        }
    }

    /// The highest-priority, most-recent activity (for the left "O" surface).
    pub fn top_activity(&self) -> Option<&Activity> {
        self.activities
            .iter()
            .max_by_key(|a| (a.priority.rank(), a.created_at))
    }

    /// The second activity (for the right "o" surface).
    pub fn second_activity(&self) -> Option<&Activity> {
        if self.activities.len() < 2 {
            return None;
        }
        let top_id = self.top_activity().map(|a| a.id);
        self.activities
            .iter()
            .filter(|a| Some(a.id) != top_id)
            .max_by_key(|a| (a.priority.rank(), a.created_at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_state() -> IslandState {
        IslandState::new()
    }

    #[test]
    fn create_activity_sets_dirty() {
        let mut s = new_state();
        assert!(!s.dirty);
        let id = s.create_activity(
            "com.test".into(),
            "Hello".into(),
            "icon".into(),
            None,
            0,
            Priority::Normal,
            false,
        );
        assert!(s.dirty);
        assert_eq!(id, 1);
        assert_eq!(s.activities.len(), 1);
    }

    #[test]
    fn dismiss_activity_removes_and_marks_dirty() {
        let mut s = new_state();
        let id = s.create_activity(
            "com.test".into(),
            "Hello".into(),
            "icon".into(),
            None,
            0,
            Priority::Normal,
            false,
        );
        s.dirty = false;
        assert!(s.dismiss_activity(id));
        assert!(s.dirty);
        assert!(s.activities.is_empty());
    }

    #[test]
    fn dismiss_nonexistent_returns_false() {
        let mut s = new_state();
        assert!(!s.dismiss_activity(999));
        assert!(!s.dirty);
    }

    #[test]
    fn update_activity_changes_title_and_progress() {
        let mut s = new_state();
        let id = s.create_activity(
            "com.test".into(),
            "Old".into(),
            "icon".into(),
            None,
            0,
            Priority::Normal,
            false,
        );
        s.dirty = false;

        assert!(s.update_activity(id, "New", 0.5));
        assert!(s.dirty);
        let a = &s.activities[0];
        assert_eq!(a.title, "New");
        assert_eq!(a.progress, Some(0.5));
    }

    #[test]
    fn update_activity_negative_progress_clears() {
        let mut s = new_state();
        let id = s.create_activity(
            "com.test".into(),
            "T".into(),
            "i".into(),
            Some(0.5),
            0,
            Priority::Normal,
            false,
        );
        s.dirty = false;
        s.update_activity(id, "", -1.0);
        assert!(s.dirty);
        assert_eq!(s.activities[0].progress, None);
    }

    #[test]
    fn notification_grouping() {
        let mut s = new_state();
        // Two notifications from same app
        s.create_notification(
            "firefox".into(),
            0,
            "ff".into(),
            "Tab 1".into(),
            "body1".into(),
            vec![],
            Priority::Normal,
            0,
            None,
            None,
            false,
            false,
            None,
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        s.create_notification(
            "firefox".into(),
            0,
            "ff".into(),
            "Tab 2".into(),
            "body2".into(),
            vec![],
            Priority::Normal,
            0,
            None,
            None,
            false,
            false,
            None,
        );
        // One from a different app
        s.create_notification(
            "slack".into(),
            0,
            "sl".into(),
            "Message".into(),
            "".into(),
            vec![],
            Priority::Normal,
            0,
            None,
            None,
            false,
            false,
            None,
        );

        let grouped = s.grouped_activities();
        assert_eq!(grouped.len(), 2); // firefox grouped, slack separate

        // Firefox group should show most recent (Tab 2) with count 2
        let ff = grouped.iter().find(|(a, _)| a.app_id == "firefox").unwrap();
        assert_eq!(ff.0.title, "Tab 2");
        assert_eq!(ff.1, 2);

        let sl = grouped.iter().find(|(a, _)| a.app_id == "slack").unwrap();
        assert_eq!(sl.0.title, "Message");
        assert_eq!(sl.1, 1);
    }

    #[test]
    fn notifications_for_app_newest_first() {
        let mut s = new_state();
        s.create_notification(
            "app".into(), 0, "i".into(), "First".into(), "".into(),
            vec![], Priority::Normal, 0, None, None, false, false, None,
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        s.create_notification(
            "app".into(), 0, "i".into(), "Second".into(), "".into(),
            vec![], Priority::Normal, 0, None, None, false, false, None,
        );

        let notifs = s.notifications_for_app("app");
        assert_eq!(notifs.len(), 2);
        assert_eq!(notifs[0].title, "Second"); // newest first
        assert_eq!(notifs[1].title, "First");
    }

    #[test]
    fn replaces_id_updates_existing_notification() {
        let mut s = new_state();
        let (_, nid) = s.create_notification(
            "app".into(), 0, "i".into(), "Original".into(), "".into(),
            vec![], Priority::Normal, 0, None, None, false, false, None,
        );
        assert_eq!(s.activities.len(), 1);

        // Replace with same notification_id
        let (_, nid2) = s.create_notification(
            "app".into(), nid, "i".into(), "Updated".into(), "new body".into(),
            vec![], Priority::Normal, 0, None, None, false, false, None,
        );
        assert_eq!(nid, nid2);
        assert_eq!(s.activities.len(), 1);
        assert_eq!(s.activities[0].title, "Updated");
        assert_eq!(s.activities[0].body, "new body");
    }

    #[test]
    fn dismiss_notification_by_notification_id() {
        let mut s = new_state();
        let (_, nid) = s.create_notification(
            "app".into(), 0, "i".into(), "T".into(), "".into(),
            vec![], Priority::Normal, 0, None, None, false, false, None,
        );
        s.dirty = false;
        assert!(s.dismiss_notification(nid));
        assert!(s.dirty);
        assert!(s.activities.is_empty());
    }

    #[test]
    fn check_expired_refocus_marks_timed_out() {
        let mut s = new_state();
        s.create_activity(
            "app".into(),
            "T".into(),
            "i".into(),
            None,
            1, // 1ms timeout
            Priority::Normal,
            false,
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        s.dirty = false;
        s.check_expired_refocus();
        assert!(s.dirty);
        assert!(s.activities[0].expired);
    }

    #[test]
    fn non_notification_activities_not_grouped() {
        let mut s = new_state();
        // Two D-Bus activities from same app_id are NOT grouped
        s.create_activity(
            "app".into(), "A".into(), "i".into(), None, 0, Priority::Normal, false,
        );
        s.create_activity(
            "app".into(), "B".into(), "i".into(), None, 0, Priority::Normal, false,
        );
        let grouped = s.grouped_activities();
        assert_eq!(grouped.len(), 2); // Not grouped — only notifications group
    }

    #[test]
    fn top_activity_picks_highest_priority() {
        let mut s = new_state();
        s.create_activity(
            "low".into(), "L".into(), "i".into(), None, 0, Priority::Low, false,
        );
        s.create_activity(
            "crit".into(), "C".into(), "i".into(), None, 0, Priority::Critical, false,
        );
        let top = s.top_activity().unwrap();
        assert_eq!(top.app_id, "crit");
    }
}
