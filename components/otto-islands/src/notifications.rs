//! org.freedesktop.Notifications daemon implementation.
//!
//! Implements the Desktop Notifications Specification (1.2) so that
//! native apps (notify-send, Firefox, Thunderbird, etc.) can send
//! notifications that appear as island activities.

use otto_kit::AppContext;
use std::collections::HashMap;
use zbus::interface;
use zbus::zvariant::Value;

use crate::activity::{NotificationAction, Priority};
use crate::state::SharedState;

pub const NOTIFICATIONS_DBUS_NAME: &str = "org.freedesktop.Notifications";
pub const NOTIFICATIONS_DBUS_PATH: &str = "/org/freedesktop/Notifications";

const DEFAULT_TIMEOUT_MS: u32 = 5000;

pub struct NotificationDaemon {
    state: SharedState,
}

impl NotificationDaemon {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationDaemon {
    /// Returns the capabilities of this notification server.
    async fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".into(),
            "body-markup".into(),
            "actions".into(),
            "persistence".into(),
            "icon-static".into(),
            "action-icons".into(),
        ]
    }

    /// Send a notification. Returns a unique notification ID.
    async fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        // Parse hints
        let priority = parse_urgency(&hints);
        let category = parse_string_hint(&hints, "category");
        let image_path = parse_string_hint(&hints, "image-path");
        let desktop_entry = parse_string_hint(&hints, "desktop-entry");
        let transient = parse_bool_hint(&hints, "transient");
        let resident = parse_bool_hint(&hints, "resident");

        // Determine app_id: prefer desktop-entry hint, fall back to app_name
        let app_id = desktop_entry.unwrap_or_else(|| app_name.to_string());

        // Resolve icon: prefer app_icon param, then image-path hint, then app_id
        let icon = if !app_icon.is_empty() {
            app_icon.to_string()
        } else if let Some(ref path) = image_path {
            path.clone()
        } else {
            // Fall back to app_id as icon name (works for desktop entries like com.mitchellh.ghostty)
            app_id.clone()
        };

        // Parse timeout
        let timeout_ms = match expire_timeout {
            -1 => DEFAULT_TIMEOUT_MS,
            0 => 0, // persistent
            t if t > 0 => t as u32,
            _ => DEFAULT_TIMEOUT_MS,
        };

        // Parse actions: alternating [id, label, id, label, ...]
        let mut parsed_actions = Vec::new();
        let mut default_action = None;
        let mut iter = actions.iter();
        while let Some(id) = iter.next() {
            if let Some(label) = iter.next() {
                if id == "default" {
                    default_action = Some(id.clone());
                } else {
                    parsed_actions.push(NotificationAction {
                        id: id.clone(),
                        label: label.clone(),
                    });
                }
            }
        }

        let mut state = self.state.lock().unwrap();
        let (activity_id, notification_id) = state.create_notification(
            app_id,
            replaces_id,
            icon,
            summary.to_string(),
            body.to_string(),
            parsed_actions,
            priority,
            timeout_ms,
            category,
            image_path,
            transient,
            resident,
            default_action,
        );
        drop(state);

        AppContext::request_wakeup();
        tracing::info!(
            notification_id,
            activity_id,
            app_name,
            app_icon,
            summary,
            body,
            "notification created"
        );

        Ok(notification_id)
    }

    /// Close a notification by ID.
    async fn close_notification(&self, id: u32) -> zbus::fdo::Result<()> {
        let mut state = self.state.lock().unwrap();
        let dismissed = state.dismiss_notification(id);
        drop(state);

        if dismissed {
            AppContext::request_wakeup();
            tracing::info!(id, "notification closed");
        }
        Ok(())
    }

    /// Return server information.
    async fn get_server_information(&self) -> (String, String, String, String) {
        (
            "Otto Islands".into(),
            "otto-compositor".into(),
            "0.1.0".into(),
            "1.2".into(),
        )
    }

    // TODO: Signals — these require zbus SignalContext
    // NotificationClosed(id: u32, reason: u32)
    // ActionInvoked(id: u32, action_key: String)
}

fn parse_urgency(hints: &HashMap<String, Value>) -> Priority {
    if let Some(Value::U8(u)) = hints.get("urgency") {
        Priority::from(*u)
    } else {
        Priority::Normal
    }
}

fn parse_string_hint(hints: &HashMap<String, Value>, key: &str) -> Option<String> {
    match hints.get(key) {
        Some(Value::Str(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn parse_bool_hint(hints: &HashMap<String, Value>, key: &str) -> bool {
    match hints.get(key) {
        Some(Value::Bool(b)) => *b,
        _ => false,
    }
}
