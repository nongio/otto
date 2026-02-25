pub mod frame;
pub mod stack;
pub mod traits;

pub use frame::{Frame, FrameBuilder};
pub use stack::{Stack, StackDirection};
pub use traits::{Container, ContainerBackend, DrawingBackend, SurfaceBackend};

// Re-export common styling types
pub use crate::components::container::traits::{
    Border, BoxShadow, CornerRadius, EdgeInsets, LayoutConstraints,
};
