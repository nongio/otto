use otto_kit::prelude::*;

struct TypographyDemo {
    window: Option<Window>,
}

impl App for TypographyDemo {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Typography Demo", 600, 700)?;

        window.set_background(Color::from_rgb(250, 250, 250));

        window.on_draw(|canvas| {
            let mut y = 60.0;
            let x = 50.0;
            let line_height_multiplier = 1.5;

            // Display
            let font = styles::HEADLINE.font();
            let paint = Paint::new(skia_safe::Color4f::new(0.1, 0.1, 0.1, 1.0), None);
            canvas.draw_str("Display", (x, y), &font, &paint);
            y += styles::HEADLINE.size * line_height_multiplier;

            // H1
            let font = styles::TITLE_1_EMPHASIZED.font();
            canvas.draw_str("Headline 1", (x, y), &font, &paint);
            y += styles::TITLE_1_EMPHASIZED.size * line_height_multiplier;

            // H2
            let font = styles::TITLE_2.font();
            canvas.draw_str("Headline 2", (x, y), &font, &paint);
            y += styles::TITLE_2.size * line_height_multiplier;

            // H3
            let font = styles::TITLE_3.font();
            canvas.draw_str("Headline 3", (x, y), &font, &paint);
            y += styles::TITLE_3.size * line_height_multiplier;

            // Title
            let font = styles::LARGE_TITLE.font();
            canvas.draw_str("Title Text", (x, y), &font, &paint);
            y += styles::LARGE_TITLE.size * line_height_multiplier;

            // Body
            let font = styles::BODY.font();
            canvas.draw_str("Body Text - Default paragraph text", (x, y), &font, &paint);
            y += styles::BODY.size * line_height_multiplier;

            // Body Small
            let font = styles::CALLOUT.font();
            canvas.draw_str("Body Small - Secondary text", (x, y), &font, &paint);
            y += styles::CALLOUT.size * line_height_multiplier;

            // Label
            let font = styles::CAPTION_1.font();
            canvas.draw_str("Label - Form labels and buttons", (x, y), &font, &paint);
            y += styles::CAPTION_1.size * line_height_multiplier;

            // Caption
            let font = styles::CAPTION_1.font();
            let caption_paint = Paint::new(skia_safe::Color4f::new(0.4, 0.4, 0.4, 1.0), None);
            canvas.draw_str(
                "Caption - Helper text and metadata",
                (x, y),
                &font,
                &caption_paint,
            );

            // Footer text
            y += 80.0;
            let footer_font = styles::CAPTION_2.font();
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
