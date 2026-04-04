use std::time::Instant;

pub type ActivityId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

impl Priority {
    pub fn rank(&self) -> u8 {
        match self {
            Priority::Low => 0,
            Priority::Normal => 1,
            Priority::High => 2,
            Priority::Critical => 3,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationMode {
    Idle,
    Compact,
    Minimal,
    Expanded,
    Banner,
}

#[derive(Debug, Clone)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivitySource {
    DBus,
    Notification,
    Portal,
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
    pub live: bool,
    pub created_at: Instant,
    pub expired: bool,
    pub actions: Vec<NotificationAction>,
    pub default_action: Option<String>,
    pub category: Option<String>,
    pub image_path: Option<String>,
    pub transient: bool,
    pub resident: bool,
    pub notification_id: Option<u32>,
    pub source: ActivitySource,
}

/// Trait for rendering activity content at different presentation sizes.
///
/// Each activity type (generic, media, timer, etc.) implements this to
/// draw itself appropriately for the given mode.
pub trait ActivityRenderer {
    /// Preferred surface size (width, height) for this mode.
    fn size(&self, mode: PresentationMode) -> (f32, f32);

    /// Draw content into the canvas at the given dimensions.
    fn draw(&self, canvas: &skia_safe::Canvas, mode: PresentationMode, w: f32, h: f32);
}
