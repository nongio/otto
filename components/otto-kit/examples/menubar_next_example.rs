//! Example demonstrating the MenuBarNext component
//!
//! This shows:
//! - Creating a menu bar with items
//! - Custom styling
//! - Simple rendering without full interaction
//!
//! Note: This is a basic rendering demo. For full interactivity (keyboard/mouse),
//! the component should be integrated with proper state management.

use otto_kit::components::menu_bar::MenuBarStyle;
use otto_kit::components::{
    label::Label,
    menu_bar::{MenuBarRenderer, MenuBarState},
};
use otto_kit::prelude::*;
use otto_kit::typography::styles;
use skia_safe::Color;

struct MenuBarExample {
    window: Option<Window>,
}

impl MenuBarExample {
    fn new() -> Self {
        Self { window: None }
    }
}

impl App for MenuBarExample {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("MenuBarNext Component Demo");
        println!("===========================");
        println!("This demo shows the menu bar rendering.");
        println!("The component supports keyboard navigation and click interaction");
        println!("when properly integrated with state management.");
        println!();

        // Create window
        let mut window = Window::new("MenuBarNext Component Demo", 900, 200)?;
        window.set_background(Color::from_rgb(245, 245, 245));

        window.on_draw(|canvas| {
            let mut y = 20.0;

            // Title
            Label::new("MenuBarNext Component")
                .at(20.0, y)
                .with_style(styles::TITLE_1)
                .with_color(Color::from_rgb(30, 30, 30))
                .render(canvas);

            y += 50.0;

            // Example 1: Default Style
            Label::new("Default Style:")
                .at(20.0, y)
                .with_style(styles::BODY)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 30.0;

            // Create menu bar with default style
            let mut menu_bar1 = MenuBarState::new();
            menu_bar1.add_item("File");
            menu_bar1.add_item("Edit");
            menu_bar1.add_item("View");
            menu_bar1.add_item("Window");
            menu_bar1.add_item("Help");

            // Render it
            canvas.save();
            canvas.translate((20.0, y));
            let style = MenuBarStyle::default();
            MenuBarRenderer::render(canvas, &menu_bar1, &style, 860.0);
            canvas.restore();

            y += 60.0;

            // Example 2: Custom Style with Active State
            Label::new("Custom Style (with active item):")
                .at(20.0, y)
                .with_style(styles::BODY)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 30.0;

            let custom_style = MenuBarStyle {
                height: 36.0,
                item_padding_horizontal: 18.0,
                background_color: Color::from_rgb(255, 255, 255),
                text_color: Color::from_rgb(20, 20, 20),
                hover_color: Color::from_argb(25, 0, 120, 215),
                active_color: Color::from_argb(50, 0, 120, 215),
                font_size: 15.0,
                item_corner_radius: 6.0,
                ..MenuBarStyle::default()
            };

            let mut menu_bar2 = MenuBarState::new();
            menu_bar2.add_item("File");
            menu_bar2.add_item("Edit");
            menu_bar2.add_item("View");
            menu_bar2.add_item("Window");
            menu_bar2.add_item("Help");

            // Set "Edit" as active
            menu_bar2.set_active(Some(1));

            canvas.save();
            canvas.translate((20.0, y));
            let style = custom_style;
            MenuBarRenderer::render(canvas, &menu_bar2, &style, 860.0);
            canvas.restore();

            y += 60.0;

            // Info
            Label::new(
                "Features: Keyboard navigation (←/→), Click interaction, Hover highlighting",
            )
            .at(20.0, y)
            .with_style(styles::CAPTION_1)
            .with_color(Color::from_rgb(150, 150, 150))
            .render(canvas);
        });

        self.window = Some(window);

        Ok(())
    }

    fn on_keyboard_event(
        &mut self,
        _key: u32,
        _state: wayland_client::protocol::wl_keyboard::KeyState,
        _serial: u32,
    ) {
        // In a real app, you would:
        // 1. Keep the MenuBarNext in app state
        // 2. Call menu_bar.handle_key(key, state)
        // 3. Request a redraw
    }

    fn on_close(&mut self) -> bool {
        println!("\nClosing MenuBarNext demo");
        false
    }
}

fn main() {
    let app = MenuBarExample::new();
    let runner = AppRunnerWithType::new(app);

    if let Err(e) = runner.run() {
        eprintln!("Error running app: {}", e);
        std::process::exit(1);
    }
}
