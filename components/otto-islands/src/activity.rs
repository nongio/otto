use std::time::Instant;

pub type ActivityId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

impl TryFrom<&str> for Priority {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "low" => Ok(Priority::Low),
            "normal" => Ok(Priority::Normal),
            "high" => Ok(Priority::High),
            "critical" | "urgent" => Ok(Priority::Critical),
            other => Err(format!("unknown priority: {other}")),
        }
    }
}

impl From<u8> for Priority {
    fn from(urgency: u8) -> Self {
        match urgency {
            0 => Priority::Low,
            2 => Priority::Critical,
            _ => Priority::Normal,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivitySource {
    DBus,
    Notification,
    /// Published by otto-islands itself, like the music island.
    Internal,
}

#[derive(Debug, Clone)]
pub struct Activity {
    pub id: ActivityId,
    pub app_id: String,
    pub title: String,
    pub body: String,
    pub icon: String,
    pub progress: Option<f64>,
    pub timeout_ms: u32,
    pub priority: Priority,
    /// A running task rather than something that happened: it holds its place
    /// with its progress on show until the app says it is over, where a
    /// notification would announce itself and settle back into the stack.
    pub live: bool,
    /// Reported, but not put on screen as an island. The dock icon still
    /// fills: an app whose own window the user is looking at has already said
    /// what it is doing, and does not need a bubble to say it again. Set from
    /// the app, which is the only thing that knows whether it is being
    /// watched.
    pub quiet: bool,
    pub created_at: Instant,
    pub expired: bool,
    pub actions: Vec<NotificationAction>,
    pub default_action: Option<String>,
    pub category: Option<String>,
    pub image_path: Option<String>,
    pub transient: bool,
    // Set by callers but not yet read by the renderer/state update loop.
    #[allow(dead_code)]
    pub resident: bool,
    pub notification_id: Option<u32>,
    pub source: ActivitySource,
}
