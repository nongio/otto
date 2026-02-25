use skia_safe::{Canvas, Rect};

use super::traits::{Container, EdgeInsets};

/// Direction for stack layout
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackDirection {
    /// Stack children vertically (top to bottom)
    Vertical,
    /// Stack children horizontally (left to right)
    Horizontal,
}

/// Alignment for children in a stack
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackAlignment {
    /// Align to start (left for horizontal, top for vertical)
    Start,
    /// Center children
    Center,
    /// Align to end (right for horizontal, bottom for vertical)
    End,
    /// Stretch children to fill available space
    Stretch,
}

/// A container that stacks children in a single direction
///
/// Stack is useful for creating lists, toolbars, or any linear layout.
/// Children can be spaced with gaps and aligned within the container.
///
/// # Examples
///
/// ```no_run
/// use otto_kit::components::container::{Stack, StackDirection, StackAlignment};
///
/// let mut stack = Stack::new(StackDirection::Vertical)
///     .with_gap(8.0)
///     .with_alignment(StackAlignment::Center);
///
/// // Add children...
/// ```
pub struct Stack {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    direction: StackDirection,
    alignment: StackAlignment,
    gap: f32,
    padding: EdgeInsets,
    children: Vec<Box<dyn Container>>,
    auto_size: bool,
}

impl Stack {
    /// Create a new stack with the specified direction
    pub fn new(direction: StackDirection) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            direction,
            alignment: StackAlignment::Start,
            gap: 0.0,
            padding: EdgeInsets::zero(),
            children: Vec::new(),
            auto_size: true,
        }
    }

    /// Set the gap between children
    pub fn with_gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Set the alignment of children
    pub fn with_alignment(mut self, alignment: StackAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Set padding around children
    pub fn with_padding(mut self, padding: EdgeInsets) -> Self {
        self.padding = padding;
        self
    }

    /// Set a fixed size (disables auto-sizing)
    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self.auto_size = false;
        self
    }

    /// Add a child to the stack
    pub fn add(&mut self, child: Box<dyn Container>) {
        self.children.push(child);
        if self.auto_size {
            self.relayout();
        }
    }

    /// Recalculate layout for all children
    pub fn relayout(&mut self) {
        if self.children.is_empty() {
            return;
        }

        let content_x = self.x + self.padding.left;
        let content_y = self.y + self.padding.top;
        let available_width = if self.auto_size {
            f32::INFINITY
        } else {
            self.width - self.padding.horizontal()
        };
        let available_height = if self.auto_size {
            f32::INFINITY
        } else {
            self.height - self.padding.vertical()
        };

        match self.direction {
            StackDirection::Vertical => {
                self.layout_vertical(content_x, content_y, available_width, available_height);
            }
            StackDirection::Horizontal => {
                self.layout_horizontal(content_x, content_y, available_width, available_height);
            }
        }
    }

    fn layout_vertical(
        &mut self,
        x: f32,
        mut y: f32,
        available_width: f32,
        _available_height: f32,
    ) {
        let mut total_height = 0.0;
        let mut max_width: f32 = 0.0;

        for (i, child) in self.children.iter_mut().enumerate() {
            if i > 0 {
                y += self.gap;
                total_height += self.gap;
            }

            let child_bounds = child.bounds();
            let child_width = child_bounds.width();
            let child_height = child_bounds.height();

            // Position based on alignment
            let child_x = match self.alignment {
                StackAlignment::Start => x,
                StackAlignment::Center => x + (available_width - child_width) / 2.0,
                StackAlignment::End => x + available_width - child_width,
                StackAlignment::Stretch => {
                    child.set_size(available_width, child_height);
                    x
                }
            };

            child.set_position(child_x, y);

            y += child_height;
            total_height += child_height;
            max_width = max_width.max(child_width);
        }

        // Update stack size if auto-sizing
        if self.auto_size {
            self.width = max_width + self.padding.horizontal();
            self.height = total_height + self.padding.vertical();
        }
    }

    fn layout_horizontal(
        &mut self,
        mut x: f32,
        y: f32,
        _available_width: f32,
        available_height: f32,
    ) {
        let mut total_width = 0.0;
        let mut max_height: f32 = 0.0;

        for (i, child) in self.children.iter_mut().enumerate() {
            if i > 0 {
                x += self.gap;
                total_width += self.gap;
            }

            let child_bounds = child.bounds();
            let child_width = child_bounds.width();
            let child_height = child_bounds.height();

            // Position based on alignment
            let child_y = match self.alignment {
                StackAlignment::Start => y,
                StackAlignment::Center => y + (available_height - child_height) / 2.0,
                StackAlignment::End => y + available_height - child_height,
                StackAlignment::Stretch => {
                    child.set_size(child_width, available_height);
                    y
                }
            };

            child.set_position(x, child_y);

            x += child_width;
            total_width += child_width;
            max_height = max_height.max(child_height);
        }

        // Update stack size if auto-sizing
        if self.auto_size {
            self.width = total_width + self.padding.horizontal();
            self.height = max_height + self.padding.vertical();
        }
    }

    /// Get the stack direction
    pub fn direction(&self) -> StackDirection {
        self.direction
    }

    /// Get the gap between children
    pub fn gap(&self) -> f32 {
        self.gap
    }

    /// Get the alignment
    pub fn alignment(&self) -> StackAlignment {
        self.alignment
    }
}

impl Container for Stack {
    fn bounds(&self) -> Rect {
        Rect::from_xywh(self.x, self.y, self.width, self.height)
    }

    fn set_position(&mut self, x: f32, y: f32) {
        let dx = x - self.x;
        let dy = y - self.y;

        self.x = x;
        self.y = y;

        // Move all children by the same offset
        for child in &mut self.children {
            let child_bounds = child.bounds();
            child.set_position(child_bounds.left + dx, child_bounds.top + dy);
        }
    }

    fn set_size(&mut self, width: f32, height: f32) {
        self.width = width;
        self.height = height;
        self.auto_size = false;
        self.relayout();
    }

    fn render(&mut self, canvas: &Canvas) {
        // Stacks don't draw anything themselves, just render children
        for child in &mut self.children {
            child.render(canvas);
        }
    }

    fn handle_pointer(&mut self, x: f32, y: f32) -> bool {
        // Check children in reverse order (top-to-bottom)
        for child in self.children.iter_mut().rev() {
            if child.handle_pointer(x, y) {
                return true;
            }
        }

        // Stack itself doesn't handle events
        false
    }

    fn children_mut(&mut self) -> Vec<&mut dyn Container> {
        self.children
            .iter_mut()
            .map(|child| child.as_mut() as &mut dyn Container)
            .collect()
    }

    fn add_child(&mut self, child: Box<dyn Container>) {
        self.add(child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::container::frame::FrameBuilder;
    use skia_safe::Color;

    #[test]
    fn test_vertical_stack() {
        let mut stack = Stack::new(StackDirection::Vertical).with_gap(10.0);

        // Add two 100x50 frames
        stack.add(Box::new(
            FrameBuilder::new(100.0, 50.0)
                .with_background(Color::RED)
                .build(),
        ));
        stack.add(Box::new(
            FrameBuilder::new(100.0, 50.0)
                .with_background(Color::BLUE)
                .build(),
        ));

        // Total height: 50 + 10 (gap) + 50 = 110
        assert_eq!(stack.bounds().height(), 110.0);
        // Width should be 100 (max of children)
        assert_eq!(stack.bounds().width(), 100.0);
    }

    #[test]
    fn test_horizontal_stack() {
        let mut stack = Stack::new(StackDirection::Horizontal).with_gap(8.0);

        stack.add(Box::new(
            FrameBuilder::new(50.0, 100.0)
                .with_background(Color::RED)
                .build(),
        ));
        stack.add(Box::new(
            FrameBuilder::new(50.0, 100.0)
                .with_background(Color::BLUE)
                .build(),
        ));

        // Total width: 50 + 8 (gap) + 50 = 108
        assert_eq!(stack.bounds().width(), 108.0);
        // Height should be 100 (max of children)
        assert_eq!(stack.bounds().height(), 100.0);
    }

    #[test]
    fn test_stack_with_padding() {
        let mut stack =
            Stack::new(StackDirection::Vertical).with_padding(EdgeInsets::uniform(10.0));

        stack.add(Box::new(
            FrameBuilder::new(100.0, 50.0)
                .with_background(Color::RED)
                .build(),
        ));

        // Width: 100 + 20 (padding)
        assert_eq!(stack.bounds().width(), 120.0);
        // Height: 50 + 20 (padding)
        assert_eq!(stack.bounds().height(), 70.0);
    }
}
