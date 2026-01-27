pub mod traits;
pub mod frame;
pub mod stack;

pub use traits::{Container, ContainerBackend, DrawingBackend, SurfaceBackend};
pub use frame::{Frame, FrameBuilder};
pub use stack::{Stack, StackDirection};

// Re-export common styling types
pub use crate::components::container::traits::{
    BoxShadow, Border, CornerRadius, EdgeInsets, LayoutConstraints,
};
