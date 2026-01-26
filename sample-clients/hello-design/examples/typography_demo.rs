use hello_design::prelude::*;

struct TypographyDemo {
    window: Option<Window>,
}

impl App for TypographyDemo {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new::<TypographyDemo>("Typography Demo", 600, 700)?;

        window.set_background(Color::from_rgb(250, 250, 250));

        window.on_draw(|canvas| {
            let mut y = 60.0;
            let x = 50.0;
            let line_height_multiplier = 1.5;

            // Display
            let font = styles::DISPLAY.font();
            let paint = Paint::new(skia_safe::Color4f::new(0.1, 0.1, 0.1, 1.0), None);
            canvas.draw_str("Display", (x, y), &font, &paint);
            y += styles::DISPLAY.size * line_height_multiplier;

            // H1
            let font = styles::H1.font();
            canvas.draw_str("Headline 1", (x, y), &font, &paint);
            y += styles::H1.size * line_height_multiplier;

            // H2
            let font = styles::H2.font();
            canvas.draw_str("Headline 2", (x, y), &font, &paint);
            y += styles::H2.size * line_height_multiplier;

            // H3
            let font = styles::H3.font();
            canvas.draw_str("Headline 3", (x, y), &font, &paint);
            y += styles::H3.size * line_height_multiplier;

            // Title
            let font = styles::TITLE.font();
            canvas.draw_str("Title Text", (x, y), &font, &paint);
            y += styles::TITLE.size * line_height_multiplier;

            // Body
            let font = styles::BODY.font();
            canvas.draw_str("Body Text - Default paragraph text", (x, y), &font, &paint);
            y += styles::BODY.size * line_height_multiplier;

            // Body Small
            let font = styles::BODY_SMALL.font();
            canvas.draw_str("Body Small - Secondary text", (x, y), &font, &paint);
            y += styles::BODY_SMALL.size * line_height_multiplier;

            // Label
            let font = styles::LABEL.font();
            canvas.draw_str("Label - Form labels and buttons", (x, y), &font, &paint);
            y += styles::LABEL.size * line_height_multiplier;

            // Caption
            let font = styles::CAPTION.font();
            let caption_paint = Paint::new(skia_safe::Color4f::new(0.4, 0.4, 0.4, 1.0), None);
            canvas.draw_str(
                "Caption - Helper text and metadata",
                (x, y),
                &font,
                &caption_paint,
            );

            // Footer text
            y += 80.0;
            let footer_font = styles::CAPTION.font();
            canvas.draw_str(
                "All text uses Inter font with font caching",
                (x, y),
                &footer_font,
                &caption_paint,
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
    let app = TypographyDemo { window: None };
    AppRunner::new(app).run()?;
    Ok(())
}
