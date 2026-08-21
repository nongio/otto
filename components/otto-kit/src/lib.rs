#[cfg(feature = "testing")]
pub mod testing;

pub mod accent;
pub mod app_runner;
pub mod clipboard;
pub mod color_scheme;
pub mod common;
pub mod components;
pub mod desktop_entry;
pub mod filetype;
pub mod icon_theme;
pub mod icons;
pub mod input;
pub mod lottie;
mod portal_runtime;
pub mod preview;
pub mod protocols;
pub mod rendering;
pub mod surfaces;
pub mod theme;
pub mod typography;
pub mod utils;

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

// Re-export cursor shape type for apps
pub use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape as CursorShape;

/// Convenience prelude for application development
pub mod prelude {
    pub use crate::accent::current_accent;
    pub use crate::app_runner::{App, AppContext, AppRunner, AppRunnerWithType};
    pub use crate::color_scheme::current_color_scheme;
    pub use crate::common::Renderable;
    pub use crate::components::color_picker::{
        ColorPickerPopup, HexField, Mode as ColorPickerMode, Swatch as ColorSwatch, WellInteraction,
    };
    pub use crate::components::container::stack::StackAlignment;
    pub use crate::components::container::{
        Border, BoxShadow, Container, CornerRadius, EdgeInsets, Frame, FrameBuilder,
        LayoutConstraints, Stack, StackDirection,
    };
    pub use crate::components::context_menu::ContextMenuStyle;
    pub use crate::components::dropdown::{DropdownInteraction, DropdownMenu};
    pub use crate::components::label::{Label, LabelBuilder, TextAlign};
    pub use crate::components::list::{ListLayout, ListRow};
    pub use crate::components::menu_item::{
        MenuItem, MenuItemGroup, MenuItemIcon, MenuItemKind, MenuItemState,
    };
    pub use crate::components::scroll::{ScrollRenderer, ScrollState, ScrollView};
    pub use crate::components::slider::{
        SliderDrag, SliderInteraction, SliderResponse, KNOB_RADIUS as SLIDER_KNOB_RADIUS,
        TRACK_THICKNESS as SLIDER_TRACK_THICKNESS,
    };
    pub use crate::components::source_list::{SourceListItem, SourceListLayout};
    pub use crate::components::text_input::{
        KeyMods, TextInput, TextInputKey, TextInputRenderer, TextInputResponse, TextInputState,
        TextInputStyle,
    };
    // `slider` and `toggle` also export free `draw`/`hit_test*` functions with
    // deliberately plain names — call them namespaced, e.g. `toggle::draw(..)`,
    // rather than importing them into scope, where "draw" would be ambiguous.
    // `dropdown::field` follows the same precedent one level deeper, so the
    // draw/client split stays visible: `dropdown::field::draw(..)`.
    pub use crate::components::toggle::{
        ToggleInteraction, HEIGHT as TOGGLE_HEIGHT, WIDTH as TOGGLE_WIDTH,
    };
    pub use crate::components::window::Window;
    pub use crate::components::{color_picker, dropdown, slider, toggle};
    pub use crate::icon_theme::current_icon_theme;
    pub use crate::icons::{named_icon, named_icon_sized};
    pub use crate::theme::ColorScheme;
    pub use crate::theme::Theme;
    pub use crate::typography::{get_font, get_font_with_fallback, styles, TextStyle};
    pub use skia_safe::{Canvas, Color, Font, Paint, Rect};
    // Add more common types as needed
}
