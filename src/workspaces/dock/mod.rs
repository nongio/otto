mod interactions;
mod model;
mod render;
mod view;
pub use model::DockModel;
pub(crate) use render::{
    badge_size, draw_app_icon, draw_badge, draw_progress, icon_color_filter, setup_badge_layer,
    setup_progress_layer,
};
pub use view::DockView;
pub use view::BASE_ICON_SIZE;
