pub mod app_runner;
pub mod components;
pub mod rendering;
pub mod surfaces;
pub mod typography;

// Re-export commonly used items
pub use components::layer::{surface::LayerSurface, Layer};
pub use components::menu_bar::{surface::MenuBarSurface, MenuBar, MenuBarItem};
pub use components::window::Window;

// Re-export new surface types
pub use surfaces::{
    PopupSurface, ScLayerAugment, SubsurfaceSurface, Surface, SurfaceError, ToplevelSurface,
};

// Re-export app framework
pub use app_runner::{App, AppContext, AppRunner};

/// Convenience prelude for application development
pub mod prelude {
    pub use crate::app_runner::{App, AppContext, AppRunner};
    pub use crate::components::window::Window;
    pub use crate::typography::{get_font, get_font_with_fallback, styles, TextStyle};
    pub use skia_safe::{Canvas, Color, Font, Paint, Rect};
    // Add more common types as needed
}
