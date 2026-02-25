//! Interactive example demonstrating the container system
//!
//! This example shows how to build UIs using Frame and Stack containers,
//! demonstrating different visual states and styling options.

use otto_kit::{
    components::container::stack::StackAlignment,
    components::container::{BoxShadow, Container, FrameBuilder, Stack, StackDirection},
    prelude::*,
};
use skia_safe::{Color, Paint, PaintStyle};

struct ContainerDemo {
    window: Option<Window>,
}

impl ContainerDemo {
    fn new() -> Self {
        Self { window: None }
    }

    fn render_simple_frames(canvas: &Canvas, x: f32, y: f32) {
        let mut stack = Stack::new(StackDirection::Horizontal).with_gap(12.0);

        // Create three colored frames with different states
        let frames = [
            (Color::from_rgb(255, 100, 100), "Normal"),
            (Color::from_rgb(100, 255, 150), "Lighter"),
            (Color::from_rgb(80, 80, 255), "Darker"),
        ];

        for (color, _label) in &frames {
            stack.add(Box::new(
                FrameBuilder::new(100.0, 80.0)
                    .with_background(*color)
                    .with_corner_radius(8.0)
                    .build(),
            ));
        }

        stack.set_position(x, y);
        stack.render(canvas);

        // Draw instruction text
        let font = styles::CAPTION_1.font();
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgb(100, 100, 100));
        paint.set_anti_alias(true);
        canvas.draw_str(
            "Three different colored frames",
            (x, y - 5.0),
            &font,
            &paint,
        );
    }

    fn render_styled_frames(canvas: &Canvas, x: f32, y: f32) {
        let mut stack = Stack::new(StackDirection::Horizontal).with_gap(12.0);

        // Frame with border
        stack.add(Box::new(
            FrameBuilder::new(120.0, 80.0)
                .with_background(Color::WHITE)
                .with_corner_radius(8.0)
                .with_border(Color::from_rgb(200, 200, 200), 2.0)
                .build(),
        ));

        // Frame with shadow
        stack.add(Box::new(
            FrameBuilder::new(120.0, 80.0)
                .with_background(Color::WHITE)
                .with_corner_radius(8.0)
                .with_shadow(BoxShadow::new(Color::from_argb(50, 0, 0, 0), 0.0, 4.0, 8.0))
                .build(),
        ));

        // Frame with both
        stack.add(Box::new(
            FrameBuilder::new(120.0, 80.0)
                .with_background(Color::from_rgb(250, 250, 250))
                .with_corner_radius(8.0)
                .with_border(Color::from_rgb(100, 100, 255), 2.0)
                .with_shadow(BoxShadow::new(
                    Color::from_argb(30, 100, 100, 255),
                    0.0,
                    2.0,
                    6.0,
                ))
                .build(),
        ));

        stack.set_position(x, y);
        stack.render(canvas);
    }

    fn render_nested(canvas: &Canvas, x: f32, y: f32) {
        // Outer container with padding
        let mut outer = FrameBuilder::new(400.0, 120.0)
            .with_background(Color::from_rgb(240, 240, 240))
            .with_corner_radius(12.0)
            .with_padding(16.0)
            .with_border(Color::from_rgb(220, 220, 220), 1.0)
            .build();

        // Inner horizontal stack
        let mut inner_stack = Stack::new(StackDirection::Horizontal)
            .with_gap(8.0)
            .with_alignment(StackAlignment::Center);

        // Add some small colored boxes
        for color in [
            Color::RED,
            Color::GREEN,
            Color::BLUE,
            Color::YELLOW,
            Color::CYAN,
            Color::MAGENTA,
        ] {
            inner_stack.add(Box::new(
                FrameBuilder::new(40.0, 40.0)
                    .with_background(color)
                    .with_corner_radius(4.0)
                    .build(),
            ));
        }

        outer.set_position(x, y);
        outer.add_child(Box::new(inner_stack));
        outer.render(canvas);
    }

    fn render_custom(canvas: &Canvas, x: f32, y: f32) {
        // Frame with custom drawing function
        let mut frame = FrameBuilder::new(300.0, 100.0)
            .with_background(Color::WHITE)
            .with_corner_radius(8.0)
            .with_border(Color::from_rgb(200, 200, 200), 1.0)
            .with_padding(16.0)
            .with_draw_fn(|canvas, bounds| {
                // Draw a simple pattern
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgb(100, 150, 255));
                paint.set_anti_alias(true);
                paint.set_style(PaintStyle::Stroke);
                paint.set_stroke_width(2.0);

                let center_x = bounds.center_x();
                let center_y = bounds.center_y();

                // Draw concentric circles
                for radius in [10.0, 20.0, 30.0, 40.0] {
                    canvas.draw_circle((center_x, center_y), radius, &paint);
                }
            })
            .build();

        frame.set_position(x, y);
        frame.render(canvas);
    }
}

impl App for ContainerDemo {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Container Demo", 600, 550)?;
        window.set_background(Color::from_rgb(250, 250, 250));

        window.on_draw(|canvas| {
            let mut y_offset = 35.0;

            // Example 1: Simple colored frames
            Self::render_simple_frames(canvas, 20.0, y_offset);
            y_offset += 96.0; // height + gap

            // Example 2: Styled frames
            Self::render_styled_frames(canvas, 20.0, y_offset);
            y_offset += 96.0;

            // Example 3: Nested containers
            Self::render_nested(canvas, 20.0, y_offset);
            y_offset += 136.0;

            // Example 4: Custom drawing
            Self::render_custom(canvas, 20.0, y_offset);
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        println!("Container demo closing...");
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = ContainerDemo::new();
    AppRunnerWithType::new(app).run()?;
    Ok(())
}
