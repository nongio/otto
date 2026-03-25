use skia_safe::{Canvas, Color, Contains, Rect};

use super::traits::{
    Border, BoxShadow, Container, ContainerBackend, ContainerStyle, CornerRadius, DrawingBackend,
    EdgeInsets, LayoutConstraints, SurfaceBackend,
};

/// A flexible container component that can use either drawing-based or surface-based rendering
///
/// Frame provides a simple rectangular container with styling support (background, borders,
/// shadows, padding). It can render in two modes:
///
/// - **Drawing mode**: Renders directly on the parent canvas (default, simple, good for prototyping)
/// - **Surface mode**: Uses a Wayland subsurface for optimized rendering (better for complex/static content)
///
/// # Examples
///
/// ```no_run
/// use otto_kit::components::container::{Frame, FrameBuilder};
/// use skia_safe::Color;
///
/// // Simple drawing-based frame
/// let frame = FrameBuilder::new(100.0, 100.0)
///     .with_background(Color::WHITE)
///     .with_corner_radius(8.0)
///     .with_padding(16.0)
///     .build();
///
/// // Surface-backed frame for optimization
/// let optimized_frame = FrameBuilder::new(200.0, 200.0)
///     .with_background(Color::from_rgb(240, 240, 240))
///     .with_border(Color::from_rgb(200, 200, 200), 1.0)
///     .use_surface()
///     .build();
/// ```
pub struct Frame<B: ContainerBackend = DrawingBackend> {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    style: ContainerStyle,
    backend: B,
    children: Vec<Box<dyn Container>>,
    constraints: LayoutConstraints,
}

impl<B: ContainerBackend> Frame<B> {
    /// Get the content area after applying padding
    pub fn content_bounds(&self) -> Rect {
        let bounds = self.bounds();
        self.style.padding.apply_to_rect(bounds)
    }

    /// Get the style configuration
    pub fn style(&self) -> &ContainerStyle {
        &self.style
    }

    /// Get mutable style configuration
    pub fn style_mut(&mut self) -> &mut ContainerStyle {
        &mut self.style
    }

    /// Set the background color
    pub fn set_background(&mut self, color: Color) {
        self.style.background_color = Some(color);
    }

    /// Set corner radius
    pub fn set_corner_radius(&mut self, radius: CornerRadius) {
        self.style.corner_radius = Some(radius);
    }

    /// Set border
    pub fn set_border(&mut self, border: Border) {
        self.style.border = Some(border);
    }

    /// Set shadow
    pub fn set_shadow(&mut self, shadow: BoxShadow) {
        self.style.shadow = Some(shadow);
    }

    /// Set padding
    pub fn set_padding(&mut self, padding: EdgeInsets) {
        self.style.padding = padding;
    }
}

impl<B: ContainerBackend> Container for Frame<B> {
    fn bounds(&self) -> Rect {
        Rect::from_xywh(self.x, self.y, self.width, self.height)
    }

    fn set_position(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
    }

    fn set_size(&mut self, width: f32, height: f32) {
        let (constrained_width, constrained_height) =
            self.constraints.constrain_size(width, height);
        self.width = constrained_width;
        self.height = constrained_height;
    }

    fn render(&mut self, canvas: &Canvas) {
        let bounds = self.bounds();

        // Render background and borders
        self.backend.render_background(canvas, bounds, &self.style);

        // Get content area
        let content_bounds = self.content_bounds();

        // Render custom content
        canvas.save();
        canvas.clip_rect(content_bounds, None, Some(true));
        self.backend.render_content(canvas, content_bounds);
        canvas.restore();

        // Render children
        for child in &mut self.children {
            child.render(canvas);
        }
    }

    fn handle_pointer(&mut self, x: f32, y: f32) -> bool {
        // Check children first (top-to-bottom)
        for child in self.children.iter_mut().rev() {
            if child.handle_pointer(x, y) {
                return true;
            }
        }

        // Check self
        self.bounds().contains(skia_safe::Point::new(x, y))
    }

    fn children_mut(&mut self) -> Vec<&mut dyn Container> {
        self.children
            .iter_mut()
            .map(|child| child.as_mut() as &mut dyn Container)
            .collect()
    }

    fn add_child(&mut self, child: Box<dyn Container>) {
        self.children.push(child);
    }
}

/// Builder for creating Frame instances
///
/// Provides a fluent API for configuring frames before creation.
pub struct FrameBuilder {
    width: f32,
    height: f32,
    x: f32,
    y: f32,
    style: ContainerStyle,
    constraints: LayoutConstraints,
    use_surface: bool,
    subsurface_id: Option<String>,
    draw_fn: Option<Box<dyn FnMut(&Canvas, Rect) + Send>>,
}

impl FrameBuilder {
    /// Create a new frame builder with the specified size
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            x: 0.0,
            y: 0.0,
            style: ContainerStyle::default(),
            constraints: LayoutConstraints::new(),
            use_surface: false,
            subsurface_id: None,
            draw_fn: None,
        }
    }

    /// Set the initial position
    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    /// Set the background color
    pub fn with_background(mut self, color: Color) -> Self {
        self.style.background_color = Some(color);
        self
    }

    /// Set uniform corner radius
    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.style.corner_radius = Some(CornerRadius::uniform(radius));
        self
    }

    /// Set custom corner radius
    pub fn with_corner_radius_custom(mut self, radius: CornerRadius) -> Self {
        self.style.corner_radius = Some(radius);
        self
    }

    /// Set border
    pub fn with_border(mut self, color: Color, width: f32) -> Self {
        self.style.border = Some(Border::new(color, width));
        self
    }

    /// Set box shadow
    pub fn with_shadow(mut self, shadow: BoxShadow) -> Self {
        self.style.shadow = Some(shadow);
        self
    }

    /// Set uniform padding
    pub fn with_padding(mut self, padding: f32) -> Self {
        self.style.padding = EdgeInsets::uniform(padding);
        self
    }

    /// Set custom padding
    pub fn with_padding_custom(mut self, padding: EdgeInsets) -> Self {
        self.style.padding = padding;
        self
    }

    /// Set layout constraints
    pub fn with_constraints(mut self, constraints: LayoutConstraints) -> Self {
        self.constraints = constraints;
        self
    }

    /// Set minimum width
    pub fn with_min_width(mut self, min_width: f32) -> Self {
        self.constraints.min_width = Some(min_width);
        self
    }

    /// Set maximum width
    pub fn with_max_width(mut self, max_width: f32) -> Self {
        self.constraints.max_width = Some(max_width);
        self
    }

    /// Set minimum height
    pub fn with_min_height(mut self, min_height: f32) -> Self {
        self.constraints.min_height = Some(min_height);
        self
    }

    /// Set maximum height
    pub fn with_max_height(mut self, max_height: f32) -> Self {
        self.constraints.max_height = Some(max_height);
        self
    }

    /// Set a custom drawing function
    pub fn with_draw_fn<F>(mut self, draw_fn: F) -> Self
    where
        F: FnMut(&Canvas, Rect) + Send + 'static,
    {
        self.draw_fn = Some(Box::new(draw_fn));
        self
    }

    /// Use surface-backed rendering instead of direct drawing
    ///
    /// This creates a subsurface for the frame, which can improve performance
    /// for complex or static content that doesn't need to be redrawn frequently.
    pub fn use_surface(mut self) -> Self {
        self.use_surface = true;
        self
    }

    /// Specify a subsurface ID when using surface-backed rendering
    pub fn with_subsurface_id(mut self, id: String) -> Self {
        self.subsurface_id = Some(id);
        self
    }

    /// Build a frame with drawing backend (default)
    pub fn build(self) -> Frame<DrawingBackend> {
        let backend = if let Some(draw_fn) = self.draw_fn {
            DrawingBackend::new().with_draw_fn(draw_fn)
        } else {
            DrawingBackend::new()
        };

        Frame {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            style: self.style,
            backend,
            children: Vec::new(),
            constraints: self.constraints,
        }
    }

    /// Build a frame with surface backend
    pub fn build_with_surface(self) -> Frame<SurfaceBackend> {
        let mut backend = SurfaceBackend::new();
        if let Some(id) = self.subsurface_id {
            backend = backend.with_subsurface(id);
        }
        if let Some(draw_fn) = self.draw_fn {
            backend = backend.with_draw_fn(draw_fn);
        }

        Frame {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            style: self.style,
            backend,
            children: Vec::new(),
            constraints: self.constraints,
        }
    }

    /// Build a frame, automatically choosing backend based on use_surface flag
    pub fn build_auto(self) -> Box<dyn Container> {
        if self.use_surface {
            Box::new(self.build_with_surface())
        } else {
            Box::new(self.build())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_builder() {
        let frame = FrameBuilder::new(100.0, 100.0)
            .at(10.0, 20.0)
            .with_background(Color::WHITE)
            .with_padding(8.0)
            .build();

        assert_eq!(frame.bounds().left, 10.0);
        assert_eq!(frame.bounds().top, 20.0);
        assert_eq!(frame.bounds().width(), 100.0);
        assert_eq!(frame.bounds().height(), 100.0);
    }

    #[test]
    fn test_content_bounds_with_padding() {
        let frame = FrameBuilder::new(100.0, 100.0).with_padding(10.0).build();

        let content = frame.content_bounds();
        assert_eq!(content.left, 10.0);
        assert_eq!(content.top, 10.0);
        assert_eq!(content.width(), 80.0);
        assert_eq!(content.height(), 80.0);
    }

    #[test]
    fn test_constraints() {
        let mut frame = FrameBuilder::new(100.0, 100.0)
            .with_min_width(150.0)
            .with_max_height(80.0)
            .build();

        frame.set_size(100.0, 100.0);

        // Width should be constrained to min
        assert_eq!(frame.bounds().width(), 150.0);
        // Height should be constrained to max
        assert_eq!(frame.bounds().height(), 80.0);
    }
}
