mod interactions;
mod model;
mod render;
mod view;
pub(crate) use render::{
    draw_app_icon, draw_badge, draw_progress, setup_badge_layer, setup_progress_layer,
};
pub use view::DockView;
pub use view::BASE_ICON_SIZE;
