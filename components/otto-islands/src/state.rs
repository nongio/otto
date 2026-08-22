use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::activity::{Activity, ActivityId, ActivitySource, NotificationAction, Priority};
use crate::dialog::{DialogId, DialogRequest, DialogResponse, DialogView};

pub type SharedState = Arc<Mutex<IslandState>>;

/// Counter for generating unique org.freedesktop.Notifications IDs.
static NEXT_NOTIFICATION_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

pub struct IslandState {
    next_id: ActivityId,
    pub activities: Vec<Activity>,
    pub dirty: bool,
    next_dialog_id: DialogId,
    /// Pending Access-style dialogs, presented one at a time in arrival order.
    pub dialogs: Vec<DialogRequest>,
}

impl IslandState {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            activities: Vec::new(),
            dirty: false,
            next_dialog_id: 1,
            dialogs: Vec::new(),
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

    // -----------------------------------------------------------------------
    // Access-style dialogs
    // -----------------------------------------------------------------------

    /// Queue a new dialog request. Returns its assigned id.
    pub fn add_dialog(&mut self, mut req: DialogRequest) -> DialogId {
        let id = self.next_dialog_id;
        self.next_dialog_id += 1;
        req.id = id;
        self.dialogs.push(req);
        self.dirty = true;
        id
    }

    /// A display snapshot of the front dialog.
    pub fn front_dialog_view(&self) -> Option<DialogView> {
        self.dialogs.first().map(|d| d.view())
    }

    /// Deliver a decision for dialog `id` and remove it from the queue.
    /// Returns true if a dialog with that id was found.
    pub fn resolve_dialog(&mut self, id: DialogId, response: DialogResponse) -> bool {
        if let Some(pos) = self.dialogs.iter().position(|d| d.id == id) {
            let mut req = self.dialogs.remove(pos);
            if let Some(tx) = req.response_tx.take() {
                let _ = tx.send(response);
            }
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Resolve every dialog whose caller has abandoned it (receiver dropped)
    /// with an "ended" response. Returns true if any were pruned.
    pub fn prune_withdrawn_dialogs(&mut self) -> bool {
        let before = self.dialogs.len();
        let mut i = 0;
        while i < self.dialogs.len() {
            if self.dialogs[i].is_withdrawn() {
                let mut req = self.dialogs.remove(i);
                if let Some(tx) = req.response_tx.take() {
                    let _ = tx.send(DialogResponse::ended());
                }
            } else {
                i += 1;
            }
        }
        if self.dialogs.len() != before {
            self.dirty = true;
            true
        } else {
            false
        }
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
