/// Topbar configuration constants.
/// These will later be loaded from otto_config.toml.

/// Bar height in logical points.
pub const BAR_HEIGHT: u32 = 30;

/// Left panel width (app name + menus).
pub const LEFT_WIDTH: u32 = 260;

/// Right panel width (tray icons + clock).
pub const RIGHT_WIDTH: u32 = 360;

/// Top margin from screen edge.
pub const BAR_MARGIN_TOP: i32 = 3;

/// Side margin from screen edge.
pub const BAR_MARGIN_SIDE: i32 = 6;

/// Horizontal padding inside a panel.
pub const BAR_PADDING_H: f32 = 14.0;

/// Spacing between tray icons.
pub const TRAY_ICON_SPACING: f32 = 8.0;

/// Tray icon size in logical points.
pub const TRAY_ICON_SIZE: f32 = 24.0;

/// Clock format string (chrono strftime).
pub const CLOCK_FORMAT: &str = "%H:%M";

/// Corner radius for the bar's corners.
pub const BAR_CORNER_RADIUS: f32 = 8.0;
