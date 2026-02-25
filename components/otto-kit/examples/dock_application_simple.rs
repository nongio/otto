// use otto_kit::{components::context_menu_next::MenuParent, prelude::*};
use otto_kit::components::menu_item::MenuItem;
use otto_kit::{
    components::context_menu::{ContextMenu as Menu, ContextMenuStyle},
    App, AppContext, AppRunner, Window,
};
use skia_safe::Color;
use smithay_client_toolkit::{
    seat::pointer::PointerEventKind,
    shell::xdg::{window::WindowConfigure, XdgPositioner, XdgSurface},
};
use wayland_client::protocol::{wl_keyboard, wl_surface};
use wayland_protocols::xdg::shell::client::xdg_positioner;
struct DockApp {
    window: Option<Window>,
    menu: Option<Menu>,
    is_activated: bool,     // Track window activation state
    last_input_serial: u32, // Track last input serial for popup grabs
}

// Track pointer state for drag detection
#[derive(Default)]
struct PointerState {
    press_position: Option<(f64, f64)>,
    press_serial: Option<u32>,
    move_started: bool,
}

const DRAG_THRESHOLD: f64 = 5.0; // pixels

impl App for DockApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // let theme = Theme::light();
        let mut window = Window::new("Ghostty", 160, 160)?;
        window.set_background(Color::TRANSPARENT);

        if let Some(layer) = window.surface_style() {
            layer.set_corner_radius(32.0);
        }
        self.window = Some(window.clone());
        AppContext::register_configure_handler(|| {
            println!("Global configure event received");
        });
        // Create unified menu with nested submenus
        let menu = Menu::new(vec![
            MenuItem::action("New Window").with_shortcut("⌘N"),
            MenuItem::separator(),
            MenuItem::submenu(
                "Recent Files",
                vec![
                    MenuItem::action("document1.txt"),
                    MenuItem::action("document2.txt"),
                    MenuItem::action("document3.txt"),
                    MenuItem::separator(),
                    MenuItem::submenu(
                        "More Files",
                        vec![
                            MenuItem::action("archived_doc1.txt"),
                            MenuItem::action("archived_doc2.txt"),
                        ],
                    ),
                ],
            ),
            MenuItem::separator(),
            MenuItem::submenu(
                "Export",
                vec![
                    MenuItem::action("Export as PDF"),
                    MenuItem::action("Export as HTML"),
                    MenuItem::separator(),
                    MenuItem::submenu(
                        "Images",
                        vec![
                            MenuItem::action("PNG"),
                            MenuItem::action("JPEG"),
                            MenuItem::action("SVG"),
                        ],
                    ),
                ],
            ),
            MenuItem::separator(),
            MenuItem::action("Show All Windows"),
            MenuItem::action("Hide"),
            MenuItem::separator(),
            MenuItem::action("Options..."),
            MenuItem::separator(),
            MenuItem::action("Quit").with_shortcut("⌘Q"),
        ])
        // .with_popup(window.wl_surface())
        .with_style(ContextMenuStyle {
            width: Some(200.0),
            ..ContextMenuStyle::default()
        })
        .on_item_click(|label| {
            println!("Menu item clicked: {}", label);
            if label == "Quit" {
                std::process::exit(0);
            }
        });

        // Handle pointer events for drag detection
        let window_clone = window.clone();
        let pointer_state = std::rc::Rc::new(std::cell::RefCell::new(PointerState::default()));

        window.on_pointer_event(move |events| {
            for event in events {
                let mut state = pointer_state.borrow_mut();

                match event.kind {
                    PointerEventKind::Press {
                        button: 0x110, // Left mouse button
                        serial,
                        ..
                    } => {
                        state.press_position = Some(event.position);
                        state.press_serial = Some(serial);
                        state.move_started = false;
                    }
                    PointerEventKind::Motion { .. } => {
                        if let (Some(press_pos), Some(serial)) =
                            (state.press_position, state.press_serial)
                        {
                            if !state.move_started {
                                let dx = event.position.0 - press_pos.0;
                                let dy = event.position.1 - press_pos.1;
                                let distance = (dx * dx + dy * dy).sqrt();

                                if distance > DRAG_THRESHOLD {
                                    // Start window drag
                                    if let Some(seat) = AppContext::seat_state().seats().next() {
                                        window_clone.start_move(&seat, serial);
                                        state.move_started = true;
                                    }
                                }
                            }
                        }
                    }
                    PointerEventKind::Release { button: 0x110, .. } => {
                        // Reset state
                        state.press_position = None;
                        state.press_serial = None;
                        state.move_started = false;
                    }
                    _ => {}
                }
            }
        });

        window.on_draw(move |canvas| {
            // Load and draw Ghostty PNG icon
            let icon_path =
                std::path::Path::new("components/hello-design/resources/icons/ghostty.png");
            if let Ok(image_data) = std::fs::read(icon_path) {
                if let Some(image) =
                    skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&image_data))
                {
                    let icon_size = 96.0;
                    let x = (160.0 - icon_size) / 2.0;
                    let y = (160.0 - icon_size) / 2.0;

                    let src_rect = skia_safe::Rect::from_xywh(
                        0.0,
                        0.0,
                        image.width() as f32,
                        image.height() as f32,
                    );
                    let dst_rect = skia_safe::Rect::from_xywh(x, y, icon_size, icon_size);

                    canvas.draw_image_rect(
                        image,
                        Some((&src_rect, skia_safe::canvas::SrcRectConstraint::Fast)),
                        dst_rect,
                        &skia_safe::Paint::default(),
                    );
                }
            }
        });

        // self.window = Some(MenuParent::Window(window));
        self.menu = Some(menu);
        Ok(())
    }

    fn on_configure(&mut self, configure: WindowConfigure, _serial: u32) {
        // Check if window activation state changed
        let was_activated = self.is_activated;
        let is_activated = configure.is_activated();

        self.is_activated = is_activated;

        println!("Window configured activated: {}", configure.is_activated());
        // If window lost activation (focus), close menu
        if was_activated && !is_activated {
            println!("[DOCK] Window lost focus, hiding menu");
            if let Some(ref menu) = self.menu {
                menu.hide();
            }
        }
    }

    fn on_close(&mut self) -> bool {
        // if let Some(ref mut menu) = self.menu {
        //     // menu.hide();
        // }
        true
    }

    fn on_keyboard_event(&mut self, key: u32, key_state: wl_keyboard::KeyState, serial: u32) {
        // Save the serial for popup grabs
        self.last_input_serial = serial;

        if key_state != wl_keyboard::KeyState::Pressed {
            return;
        }

        // Press 'm' to toggle menu, or forward to menu for navigation
        if let (Some(ref window), Some(ref mut menu)) = (&self.window, &mut self.menu) {
            if key == 50 {
                // 'm' key
                println!("'m' pressed, toggling menu with serial {}", serial);
                // Toggle menu visibility
                let x = 80; // Center of icon
                let y = 160; // Bottom of window
                let top_level = window.surface().unwrap();
                let xdg_surface = top_level.xdg_window().xdg_surface();

                // Create positioner for popup placement
                if let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state()) {
                    positioner.set_size(200, 150); // Approximate menu size
                    positioner.set_anchor_rect(x, y, 1, 1);
                    positioner.set_anchor(xdg_positioner::Anchor::BottomLeft);
                    positioner.set_gravity(xdg_positioner::Gravity::BottomRight);

                    // Add constraint adjustment - match popup_test exactly
                    positioner.set_constraint_adjustment(
                        xdg_positioner::ConstraintAdjustment::SlideX
                            | xdg_positioner::ConstraintAdjustment::SlideY
                            | xdg_positioner::ConstraintAdjustment::FlipX
                            | xdg_positioner::ConstraintAdjustment::FlipY,
                    );

                    // Pass the serial to show() for popup grab
                    menu.show(&xdg_surface, &positioner, serial);
                }
            } else {
                // menu.hide();
                // Forward to menu for keyboard navigation
                menu.handle_key(key, key_state);
            }
        }
    }

    fn on_keyboard_leave(&mut self, _surface: &wl_surface::WlSurface) {
        // Don't close menu on keyboard leave - the menu popup itself
        // causes keyboard focus to shift, which would close it immediately.
        // Instead, rely on Escape key and popup done event (clicking outside).
        //
        // If we needed to distinguish between surfaces, we could check:
        // if let Some(ref window) = self.window {
        //     if surface == window.surface().unwrap().wl_surface() {
        //         // Main window lost focus
        //     }
        // }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Dock Application with Menu");
    println!("- Press 'm' to toggle menu");
    println!("- Arrow keys to navigate");
    println!("- Enter to select");
    println!("- Escape to close menu");
    println!("- Click and drag icon to move window");

    let app = DockApp {
        window: None,
        menu: None,
        is_activated: false,
        last_input_serial: 0,
    };

    AppRunner::new(app).run()?;
    Ok(())
}
