pub mod handlers;
pub mod protocol;

// Re-export key types for convenience
pub use handlers::manager::{DockItemRole};
pub use handlers::OttoDockState;
pub use protocol::{DockItem, DockItemType, OttoDockItemV1, OttoDockManagerV1};
