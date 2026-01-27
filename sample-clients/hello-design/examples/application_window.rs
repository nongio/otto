/// Example demonstrating the ApplicationWindow component with titlebar
use hello_design::prelude::*;
use hello_design::components::window::{ApplicationWindow, WindowLayout};

struct AppWindowDemo {
    window: Option<ApplicationWindow>,
}

impl App for AppWindowDemo {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Create a layout with titlebar and sidebar
        let layout = WindowLayout {
            titlebar_height: 60,
            sidebar_width: 200,
            sidebar_left: true,
        };

        let mut window = ApplicationWindow::new::<AppWindowDemo>(
            "Application Window Demo",
            800,
            600,
            layout,
        )?;

        window.set_background(Color::from_rgb(200, 200, 200));

        // Draw titlebar
        window.on_titlebar_draw(|canvas| {
            // Draw title text
            canvas.clear(skia_safe::Color::TRANSPARENT);
            let font = styles::TITLE_3_EMPHASIZED.font();
            let paint = Paint::new(skia_safe::Color4f::new(0.2, 0.2, 0.2, 1.0), None);
            canvas.draw_str("Application Window Title", (230.0, 25.0), &font, &paint);
        });
        window.on_sidebar_draw(|canvas| {
            // Draw sidebar title
            canvas.clear(skia_safe::Color::TRANSPARENT);
            let title_font = styles::TITLE_3_EMPHASIZED.font();
            let paint = Paint::new(skia_safe::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
            let margin_top = 100.0;
            canvas.draw_str("Sidebar", (16.0, margin_top), &title_font, &paint);
            
            // Draw some menu items
            let body_font = styles::BODY.font();
            canvas.draw_str("• Home", (16.0, margin_top + 60.0), &body_font, &paint);
            canvas.draw_str("• Files", (16.0, margin_top + 85.0), &body_font, &paint);
            canvas.draw_str("• Settings", (16.0, margin_top + 110.0), &body_font, &paint);
            canvas.draw_str("• About", (16.0, margin_top + 135.0), &body_font, &paint);
        });
        // Draw main content
        window.on_content_draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            let title_font = styles::TITLE_1.font();
            let paint = Paint::new(skia_safe::Color4f::new(0.9, 0.2, 0.2, 1.0), None);
            // canvas.draw_str("Main Content Area", (50.0, 100.0), &title_font, &paint);
            
            let body_font = styles::BODY.font();
            canvas.draw_str("Window has a titlebar subsurface at the top", (50.0, 140.0), &body_font, &paint);
        });

        self.window = Some(window);

        Ok(())
    }

    fn on_close(&mut self) -> bool {
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = AppWindowDemo { window: None };
    AppRunner::new(app).run()?;
    Ok(())
}
