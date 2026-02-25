use otto_kit::components::button::Button;
use otto_kit::components::toolbar::{Toolbar, ToolbarGroup};
use otto_kit::prelude::*;
use smithay_client_toolkit::seat::pointer::PointerEventKind;

struct WindowWithToolbarApp {
    window: Option<Window>,
}

impl App for WindowWithToolbarApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let theme = Theme::light();
        let mut window = Window::new("Window with Toolbar", 1000, 700)?;
        window.set_background(Color::WHITE); // White content area

        // Set window corner radius to 20
        if let Some(layer) = window.surface_style() {
            layer.set_corner_radius(20.0);
        }

        // Make the toolbar draggable!
        // We need to get a seat to call start_move - for now we'll handle this in the pointer callback
        let window_clone = window.clone();
        window.on_pointer_event(move |events| {
            eprintln!("Got {} pointer events", events.len());
            for event in events {
                eprintln!(
                    "Event: {:?} at ({}, {})",
                    event.kind, event.position.0, event.position.1
                );
                // Check if this is a press event in the toolbar region (top 52 pixels)
                if let PointerEventKind::Press { serial, button, .. } = event.kind {
                    eprintln!("Button {} pressed at y={}", button, event.position.1);
                    if event.position.1 < 52.0 {
                        // Click in toolbar area - get the seat from SeatState
                        if let Some(seat) = AppContext::seat_state().seats().next() {
                            println!("Starting window drag from toolbar...");
                            window_clone.start_move(&seat, serial);
                        } else {
                            eprintln!("No seat available!");
                        }
                    }
                }
            }
        });

        window.on_draw(move |canvas| {
            let window_width = 1000.0;

            // Create toolbar groups
            let leading = ToolbarGroup::new()
                .add_item(Button::icon("chevron-left").ghost())
                .add_item(Button::icon("chevron-right").ghost())
                .add_space(16.0)
                .add_item(
                    Label::new("Toolbar window")
                        .with_style(styles::TITLE_3_EMPHASIZED)
                        .with_color(theme.text_primary),
                )
                .build();

            let trailing = ToolbarGroup::new()
                // .add(Button::icon("search").ghost())
                // .add(Button::icon("star").ghost())
                // .add(Button::icon("share-2").ghost())
                // .add_separator()
                // .add(Button::icon("settings").ghost())
                .build();

            // Render toolbar at the top with theme colors
            Toolbar::new()
                .at(0.0, 0.0)
                .with_width(window_width)
                .with_background(theme.material_titlebar)
                .with_border_bottom(theme.fill_secondary)
                .with_leading(leading)
                .with_trailing(trailing)
                .render(canvas);
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        println!("Closing window with toolbar...");
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = WindowWithToolbarApp { window: None };
    AppRunner::new(app).run()?;
    Ok(())
}
