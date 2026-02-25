mod icon;
mod svg_icon;

// Export the SVG-based icon as the default
pub use svg_icon::list_icons;
pub use svg_icon::Icon;

// Keep the old geometric icons available for backwards compatibility
pub use icon::Icon as GeometricIcon;
pub use icon::{IconShape, IconStyle};
