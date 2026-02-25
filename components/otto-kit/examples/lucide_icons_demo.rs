use otto_kit::components::icon::Icon;
use otto_kit::prelude::*;
use skia_safe::Color;

struct LucideIconsApp {
    window: Option<Window>,
}

impl App for LucideIconsApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Lucide Icons Demo", 1000, 700)?;
        window.set_background(Color::from_rgb(245, 245, 245));

        window.on_draw(|canvas| {
            // Title
            Label::new("Lucide Icons Demo")
                .at(50.0, 50.0)
                .with_style(styles::LARGE_TITLE_EMPHASIZED)
                .with_color(Color::from_rgb(30, 30, 30))
                .render(canvas);

            // Popular icons showcase
            let icons = [
                ("heart", "Heart"),
                ("star", "Star"),
                ("home", "Home"),
                ("search", "Search"),
                ("settings", "Settings"),
                ("user", "User"),
                ("mail", "Mail"),
                ("bell", "Bell"),
                ("check", "Check"),
                ("x", "Close"),
                ("menu", "Menu"),
                ("chevron-right", "Chevron"),
                ("circle", "Circle"),
                ("square", "Square"),
                ("triangle", "Triangle"),
                ("plus", "Plus"),
                ("minus", "Minus"),
                ("download", "Download"),
                ("upload", "Upload"),
                ("trash", "Trash"),
            ];

            let mut x = 50.0;
            let mut y = 120.0;
            let icon_spacing = 90.0;
            let icons_per_row = 10;

            for (i, (icon_name, label)) in icons.iter().enumerate() {
                if i > 0 && i % icons_per_row == 0 {
                    y += 100.0;
                    x = 50.0;
                }

                // Draw icon
                Icon::new(*icon_name)
                    .at(x + 8.0, y)
                    .with_size(32.0)
                    .with_color(Color::from_rgb(59, 130, 246))
                    .render(canvas);

                // Draw label
                Label::new(*label)
                    .at(x - 10.0, y + 50.0)
                    .with_style(styles::CAPTION_1)
                    .with_color(Color::from_rgb(100, 100, 100))
                    .render(canvas);

                x += icon_spacing;
            }

            y += 120.0;

            // Different sizes
            Label::new("Different Sizes:")
                .at(50.0, y)
                .with_style(styles::TITLE_2)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 40.0;
            x = 50.0;

            let sizes = [16.0, 24.0, 32.0, 48.0, 64.0];
            for size in &sizes {
                Icon::new("heart")
                    .at(x, y)
                    .with_size(*size)
                    .with_color(Color::from_rgb(239, 68, 68))
                    .render(canvas);

                let size_text = format!("{}px", size);
                Label::new(&size_text)
                    .at(x - 5.0, y + size + 20.0)
                    .with_style(styles::CAPTION_1)
                    .with_color(Color::from_rgb(100, 100, 100))
                    .render(canvas);

                x += size + 30.0;
            }

            y += 110.0;

            // Different stroke widths
            Label::new("Different Stroke Widths:")
                .at(50.0, y)
                .with_style(styles::TITLE_2)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 40.0;
            x = 50.0;

            let strokes = [1.0, 1.5, 2.0, 2.5, 3.0];
            for stroke in &strokes {
                Icon::new("star")
                    .at(x, y)
                    .with_size(32.0)
                    .with_stroke_width(*stroke)
                    .with_color(Color::from_rgb(16, 185, 129))
                    .render(canvas);

                let stroke_text = format!("{}px", stroke);
                Label::new(&stroke_text)
                    .at(x - 5.0, y + 50.0)
                    .with_style(styles::CAPTION_1)
                    .with_color(Color::from_rgb(100, 100, 100))
                    .render(canvas);

                x += 80.0;
            }
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        println!("Closing Lucide icons demo...");
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = LucideIconsApp { window: None };
    AppRunnerWithType::new(app).run()?;
    Ok(())
}
