pub mod app_runner;
pub mod common;
pub mod components;
pub mod input;
pub mod protocols;
pub mod rendering;
pub mod surfaces;
pub mod theme;
pub mod typography;

// Re-export commonly used items
pub use common::Renderable;
pub use components::container::{
    frame::{Frame, FrameBuilder},
    stack::{Stack, StackDirection},
    traits::{Border, BoxShadow, Container, CornerRadius, EdgeInsets, LayoutConstraints},
};
pub use components::label::{Label, LabelBuilder, TextAlign};
pub use components::layer::{surface::LayerSurface, Layer};
// pub use components::menu_bar::{surface::MenuBarSurface, MenuBar, MenuBarItem};
pub use components::window::Window;

// Re-export new surface types
pub use surfaces::{
    BaseWaylandSurface, PopupSurface, SubsurfaceSurface, SurfaceError, ToplevelSurface,
};

// Re-export app framework
pub use app_runner::{App, AppContext, AppRunner, AppRunnerWithType};

/// Convenience prelude for application development
pub mod prelude {
    pub use crate::app_runner::{App, AppContext, AppRunner, AppRunnerWithType};
    pub use crate::common::Renderable;
    pub use crate::components::container::stack::StackAlignment;
    pub use crate::components::container::{
        Border, BoxShadow, Container, CornerRadius, EdgeInsets, Frame, FrameBuilder,
        LayoutConstraints, Stack, StackDirection,
    };
    pub use crate::components::context_menu::ContextMenuStyle;
    pub use crate::components::label::{Label, LabelBuilder, TextAlign};
    pub use crate::components::menu_item::{MenuItem, MenuItemGroup, MenuItemKind, MenuItemState};
    pub use crate::components::window::Window;
    pub use crate::theme::Theme;
    pub use crate::typography::{get_font, get_font_with_fallback, styles, TextStyle};
    pub use skia_safe::{Canvas, Color, Font, Paint, Rect};
    // Add more common types as needed
}
