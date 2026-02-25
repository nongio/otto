use crate::components::layers::LayerFrame;
/// Common traits and types shared across components
use skia_safe::{Canvas, Rect};

/// Core trait for components that can render themselves on a canvas
///
/// This trait provides a simple, unified interface for rendering components
/// directly on a Skia canvas. Unlike the `Container` trait which includes
/// layout and positioning, this trait focuses solely on rendering.
///
/// # Examples
///
/// ```no_run
/// use otto_kit::common::Renderable;
/// use skia_safe::Canvas;
///
/// struct MyComponent {
///     text: String,
/// }
///
/// impl Renderable for MyComponent {
///     fn render(&self, canvas: &Canvas) {
///         // Draw your component
///     }
/// }
///
/// // Use it directly
/// let component = MyComponent { text: "Hello".to_string() };
/// component.render(&canvas);
///
/// // Or as a trait object
/// let renderable: Box<dyn Renderable> = Box::new(component);
/// renderable.render(&canvas);
///
/// // Convert to a layer for scene graph rendering
/// let layer = component.to_layer(100.0, 50.0);
/// ```
pub trait Renderable {
    /// Render the component on the provided canvas
    ///
    /// The component should draw itself at its current position/state.
    /// The canvas may already have transformations applied.
    fn render(&self, canvas: &Canvas);

    /// Get the intrinsic size of the component
    /// Returns (width, height) or None if the component doesn't have an intrinsic size
    fn intrinsic_size(&self) -> Option<(f32, f32)> {
        None
    }

    /// Convert this renderable into a LayerFrame for scene graph rendering
    ///
    /// This method creates a LayerFrame with the specified dimensions and
    /// sets up a draw function that calls this component's `render()` method.
    ///
    /// # Arguments
    /// * `width` - The width of the layer
    /// * `height` - The height of the layer
    ///
    /// # Returns
    /// A LayerFrame configured to render this component
    fn to_layer(&self, width: f32, height: f32) -> LayerFrame
    where
        Self: Clone + Send + Sync + 'static,
    {
        let frame = LayerFrame::new();
        frame.set_size(width, height);

        let component = self.clone();
        frame.set_draw(move |canvas: &Canvas, _alpha, _context| {
            component.render(canvas);
            Rect::default()
        });

        frame
    }
}
