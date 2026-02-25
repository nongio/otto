use skia_safe::{Canvas, Color, Contains, Paint, Rect};

/// Core trait for all container components
///
/// Containers provide a way to draw content either directly on a canvas
/// or using subsurfaces for performance optimization. The abstraction
/// allows users to build UIs with simple drawing first and optimize
/// later without major refactoring.
pub trait Container {
    /// Get the container's bounds
    fn bounds(&self) -> Rect;

    /// Set the container's position
    fn set_position(&mut self, x: f32, y: f32);

    /// Set the container's size
    fn set_size(&mut self, width: f32, height: f32);

    /// Render the container and its children
    fn render(&mut self, canvas: &Canvas);

    /// Handle pointer events (returns true if handled)
    fn handle_pointer(&mut self, x: f32, y: f32) -> bool {
        self.bounds().contains(skia_safe::Point::new(x, y))
    }

    /// Get mutable access to children (if any)
    fn children_mut(&mut self) -> Vec<&mut dyn Container> {
        Vec::new()
    }

    /// Add a child container
    fn add_child(&mut self, _child: Box<dyn Container>) {
        // Default: no-op for leaf containers
    }
}

/// Backend marker trait - determines how containers render
pub trait ContainerBackend: Send + 'static {
    /// Render the container's background and borders
    fn render_background(&mut self, canvas: &Canvas, bounds: Rect, style: &ContainerStyle);

    /// Render the container's content
    fn render_content(&mut self, canvas: &Canvas, bounds: Rect);

    /// Cleanup resources
    fn cleanup(&mut self) {}
}

/// Drawing-based backend - renders directly on parent canvas
pub struct DrawingBackend {
    /// Custom drawing function
    pub draw_fn: Option<Box<dyn FnMut(&Canvas, Rect) + Send>>,
}

impl DrawingBackend {
    pub fn new() -> Self {
        Self { draw_fn: None }
    }

    pub fn with_draw_fn<F>(mut self, draw_fn: F) -> Self
    where
        F: FnMut(&Canvas, Rect) + Send + 'static,
    {
        self.draw_fn = Some(Box::new(draw_fn));
        self
    }
}

impl Default for DrawingBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ContainerBackend for DrawingBackend {
    fn render_background(&mut self, canvas: &Canvas, bounds: Rect, style: &ContainerStyle) {
        canvas.save();

        // Clip to rounded corners if specified
        if let Some(radius) = &style.corner_radius {
            let rrect = radius.to_rrect(bounds);
            canvas.clip_rrect(rrect, None, Some(true));
        }

        // Draw shadow
        if let Some(shadow) = &style.shadow {
            shadow.draw(canvas, bounds);
        }

        // Draw background
        if let Some(bg_color) = style.background_color {
            let mut paint = Paint::default();
            paint.set_color(bg_color);
            paint.set_anti_alias(true);

            if let Some(radius) = &style.corner_radius {
                let rrect = radius.to_rrect(bounds);
                canvas.draw_rrect(rrect, &paint);
            } else {
                canvas.draw_rect(bounds, &paint);
            }
        }

        // Draw border
        if let Some(border) = &style.border {
            border.draw(canvas, bounds, &style.corner_radius);
        }

        canvas.restore();
    }

    fn render_content(&mut self, canvas: &Canvas, bounds: Rect) {
        if let Some(ref mut draw_fn) = self.draw_fn {
            draw_fn(canvas, bounds);
        }
    }
}

/// Surface-backed backend - uses Wayland subsurfaces for optimization
///
/// This allows expensive rendering to be cached and only updated when needed,
/// improving performance for complex UIs.
pub struct SurfaceBackend {
    /// Reference to subsurface (managed externally)
    subsurface_id: Option<String>,
    /// Whether the surface needs to be redrawn
    needs_redraw: bool,
    /// Custom drawing function
    draw_fn: Option<Box<dyn FnMut(&Canvas, Rect) + Send>>,
}

impl SurfaceBackend {
    pub fn new() -> Self {
        Self {
            subsurface_id: None,
            needs_redraw: true,
            draw_fn: None,
        }
    }

    pub fn with_subsurface(mut self, id: String) -> Self {
        self.subsurface_id = Some(id);
        self
    }

    pub fn with_draw_fn<F>(mut self, draw_fn: F) -> Self
    where
        F: FnMut(&Canvas, Rect) + Send + 'static,
    {
        self.draw_fn = Some(Box::new(draw_fn));
        self
    }

    pub fn mark_dirty(&mut self) {
        self.needs_redraw = true;
    }

    pub fn subsurface_id(&self) -> Option<&str> {
        self.subsurface_id.as_deref()
    }
}

impl Default for SurfaceBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ContainerBackend for SurfaceBackend {
    fn render_background(&mut self, canvas: &Canvas, bounds: Rect, style: &ContainerStyle) {
        // Only render if dirty or no subsurface is available
        if !self.needs_redraw && self.subsurface_id.is_some() {
            return;
        }

        // Same rendering as DrawingBackend
        canvas.save();

        if let Some(radius) = &style.corner_radius {
            let rrect = radius.to_rrect(bounds);
            canvas.clip_rrect(rrect, None, Some(true));
        }

        if let Some(shadow) = &style.shadow {
            shadow.draw(canvas, bounds);
        }

        if let Some(bg_color) = style.background_color {
            let mut paint = Paint::default();
            paint.set_color(bg_color);
            paint.set_anti_alias(true);

            if let Some(radius) = &style.corner_radius {
                let rrect = radius.to_rrect(bounds);
                canvas.draw_rrect(rrect, &paint);
            } else {
                canvas.draw_rect(bounds, &paint);
            }
        }

        if let Some(border) = &style.border {
            border.draw(canvas, bounds, &style.corner_radius);
        }

        canvas.restore();

        self.needs_redraw = false;
    }

    fn render_content(&mut self, canvas: &Canvas, bounds: Rect) {
        if let Some(ref mut draw_fn) = self.draw_fn {
            draw_fn(canvas, bounds);
        }
    }

    fn cleanup(&mut self) {
        // Could destroy subsurface here
        self.subsurface_id = None;
    }
}

/// Container styling configuration
#[derive(Clone, Debug)]
pub struct ContainerStyle {
    pub background_color: Option<Color>,
    pub corner_radius: Option<CornerRadius>,
    pub border: Option<Border>,
    pub shadow: Option<BoxShadow>,
    pub padding: EdgeInsets,
}

impl Default for ContainerStyle {
    fn default() -> Self {
        Self {
            background_color: None,
            corner_radius: None,
            border: None,
            shadow: None,
            padding: EdgeInsets::zero(),
        }
    }
}

/// Defines corner radius for containers
#[derive(Clone, Debug)]
pub struct CornerRadius {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl CornerRadius {
    pub fn uniform(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    pub fn top(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: 0.0,
            bottom_left: 0.0,
        }
    }

    pub fn bottom(radius: f32) -> Self {
        Self {
            top_left: 0.0,
            top_right: 0.0,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    pub fn to_rrect(&self, rect: Rect) -> skia_safe::RRect {
        let radii = [
            (self.top_left, self.top_left).into(),
            (self.top_right, self.top_right).into(),
            (self.bottom_right, self.bottom_right).into(),
            (self.bottom_left, self.bottom_left).into(),
        ];
        skia_safe::RRect::new_rect_radii(rect, &radii)
    }
}

/// Defines edge insets for padding/margin
#[derive(Clone, Debug, Copy)]
pub struct EdgeInsets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl EdgeInsets {
    pub fn zero() -> Self {
        Self {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }
    }

    pub fn uniform(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    pub fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    pub fn only(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }

    pub fn apply_to_rect(&self, rect: Rect) -> Rect {
        Rect::new(
            rect.left + self.left,
            rect.top + self.top,
            rect.right - self.right,
            rect.bottom - self.bottom,
        )
    }
}

/// Border configuration
#[derive(Clone, Debug)]
pub struct Border {
    pub color: Color,
    pub width: f32,
}

impl Border {
    pub fn new(color: Color, width: f32) -> Self {
        Self { color, width }
    }

    pub fn draw(&self, canvas: &Canvas, bounds: Rect, corner_radius: &Option<CornerRadius>) {
        let mut paint = Paint::default();
        paint.set_color(self.color);
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::PaintStyle::Stroke);
        paint.set_stroke_width(self.width);

        // Inset by half the border width for proper alignment
        let inset = self.width / 2.0;
        let border_rect = bounds.with_outset((-inset, -inset));

        if let Some(radius) = corner_radius {
            let rrect = radius.to_rrect(border_rect);
            canvas.draw_rrect(rrect, &paint);
        } else {
            canvas.draw_rect(border_rect, &paint);
        }
    }
}

/// Box shadow configuration
#[derive(Clone, Debug)]
pub struct BoxShadow {
    pub color: Color,
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread: f32,
}

impl BoxShadow {
    pub fn new(color: Color, offset_x: f32, offset_y: f32, blur_radius: f32) -> Self {
        Self {
            color,
            offset_x,
            offset_y,
            blur_radius,
            spread: 0.0,
        }
    }

    pub fn with_spread(mut self, spread: f32) -> Self {
        self.spread = spread;
        self
    }

    pub fn draw(&self, canvas: &Canvas, bounds: Rect) {
        let mut paint = Paint::default();
        paint.set_color(self.color);
        paint.set_anti_alias(true);

        // Apply blur if specified
        if self.blur_radius > 0.0 {
            let blur =
                skia_safe::MaskFilter::blur(skia_safe::BlurStyle::Normal, self.blur_radius, None);
            if let Some(blur_filter) = blur {
                paint.set_mask_filter(blur_filter);
            }
        }

        // Calculate shadow rect with offset and spread
        let shadow_rect = Rect::new(
            bounds.left + self.offset_x - self.spread,
            bounds.top + self.offset_y - self.spread,
            bounds.right + self.offset_x + self.spread,
            bounds.bottom + self.offset_y + self.spread,
        );

        canvas.draw_rect(shadow_rect, &paint);
    }
}

/// Layout constraints for containers
#[derive(Clone, Copy, Debug)]
pub struct LayoutConstraints {
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
}

impl LayoutConstraints {
    pub fn new() -> Self {
        Self {
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
        }
    }

    pub fn constrain_width(&self, width: f32) -> f32 {
        let mut result = width;
        if let Some(min) = self.min_width {
            result = result.max(min);
        }
        if let Some(max) = self.max_width {
            result = result.min(max);
        }
        result
    }

    pub fn constrain_height(&self, height: f32) -> f32 {
        let mut result = height;
        if let Some(min) = self.min_height {
            result = result.max(min);
        }
        if let Some(max) = self.max_height {
            result = result.min(max);
        }
        result
    }

    pub fn constrain_size(&self, width: f32, height: f32) -> (f32, f32) {
        (self.constrain_width(width), self.constrain_height(height))
    }
}

impl Default for LayoutConstraints {
    fn default() -> Self {
        Self::new()
    }
}
