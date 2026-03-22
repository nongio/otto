/// Topbar configuration constants.
/// These will later be loaded from otto_config.toml.

/// Bar height in logical points.
pub const BAR_HEIGHT: u32 = 28;

/// Horizontal padding inside the bar (left and right edges).
pub const BAR_PADDING_H: f32 = 12.0;

/// Spacing between tray icons.
pub const TRAY_ICON_SPACING: f32 = 8.0;

/// Tray icon size in logical points.
pub const TRAY_ICON_SIZE: f32 = 18.0;

/// Clock format string (chrono strftime).
pub const CLOCK_FORMAT: &str = "%H:%M";

/// Corner radius for the bar's bottom corners.
pub const BAR_CORNER_RADIUS: f32 = 10.0;
