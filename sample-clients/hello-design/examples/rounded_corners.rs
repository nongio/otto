use hello_design::prelude::*;

/// Demonstrates the default rounded corners on windows
struct RoundedWindowDemo {
    window: Option<Window>,
}

impl App for RoundedWindowDemo {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Create a window - it will have rounded corners by default (12px radius)
        let mut window = Window::new::<RoundedWindowDemo>("Rounded Window Demo", 500, 400)?;

        window.set_background(Color::from_rgb(255, 255, 255));

        window.on_draw(|canvas| {
            // Draw some content to show the rounded corners effect
            let title_font = styles::H1.font();
            let body_font = styles::BODY.font();
            let paint = Paint::new(skia_safe::Color4f::new(0.1, 0.1, 0.1, 1.0), None);

            canvas.draw_str("Rounded Corners", (50.0, 80.0), &title_font, &paint);
            canvas.draw_str(
                "This window has 12px rounded corners by default",
                (50.0, 120.0),
                &body_font,
                &paint,
            );

            // Draw a colored box near the corners to make the rounding visible
            let corner_paint = Paint::new(skia_safe::Color4f::new(0.2, 0.6, 0.8, 0.3), None);
            canvas.draw_rect(
                skia_safe::Rect::from_xywh(0.0, 0.0, 100.0, 100.0),
                &corner_paint,
            );
            canvas.draw_rect(
                skia_safe::Rect::from_xywh(400.0, 0.0, 100.0, 100.0),
                &corner_paint,
            );
            canvas.draw_rect(
                skia_safe::Rect::from_xywh(0.0, 300.0, 100.0, 100.0),
                &corner_paint,
            );
            canvas.draw_rect(
                skia_safe::Rect::from_xywh(400.0, 300.0, 100.0, 100.0),
                &corner_paint,
            );

            // Info text
            let caption_font = styles::CAPTION.font();
            let gray_paint = Paint::new(skia_safe::Color4f::new(0.4, 0.4, 0.4, 1.0), None);
            canvas.draw_str(
                "Blue boxes show the corners are properly rounded",
                (50.0, 200.0),
                &caption_font,
                &gray_paint,
            );
        });

        self.window = Some(window);

        Ok(())
    }

    fn on_close(&mut self) -> bool {
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = RoundedWindowDemo { window: None };
    AppRunner::new(app).run()?;
    Ok(())
}
