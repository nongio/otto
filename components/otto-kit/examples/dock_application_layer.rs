use otto_kit::{
    components::{
        context_menu::{ContextMenu, ContextMenuStyle},
        menu_item::MenuItem,
    },
    protocols::otto_dock_item_v1,
    surfaces::{BaseWaylandSurface, LayerShellSurface},
    App, AppRunner,
};

use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use wayland_client::protocol::wl_keyboard;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::Anchor,
};

struct DockApp {
    // dock_item: Option<DockItem>,
    dock_item: Option<otto_dock_item_v1::OttoDockItemV1>,
    layer_surface: Option<LayerShellSurface>,
    menu: Option<ContextMenu>,
    last_input_serial: u32,
}
// const SIZE: u32 = 60;
impl DockApp {
    fn draw_icon(&self, surface: &BaseWaylandSurface) {
        // if let Some(ref surface) = self.layer_surface {
        println!("Drawing icon on dock surface...");
        let w = surface.dimensions().0 as f32;
        let h = surface.dimensions().1 as f32;
        // let w = self.layer_surface.as_ref().map(|ls| ls.base_surface().dimensions().0 as f32).unwrap_or(120.0);
        // let h = self.layer_surface.as_ref().map(|ls| ls.base_surface().dimensions().1 as f32).unwrap_or(120.0);
        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::RED);

            let icon_path =
                std::path::Path::new("components/hello-design/resources/icons/ghostty.png");
            if let Ok(image_data) = std::fs::read(icon_path) {
                if let Some(image) =
                    skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&image_data))
                {
                    let icon_size = 60.0;
                    let x = (w - icon_size) / 2.0;
                    let y = (h - icon_size) / 2.0;

                    let src_rect = skia_safe::Rect::from_xywh(
                        0.0,
                        0.0,
                        image.width() as f32,
                        image.height() as f32,
                    );
                    let dst_rect = skia_safe::Rect::from_xywh(x, y, icon_size, icon_size);
                    let white = skia_safe::Color4f::new(1.0, 1.0, 1.0, 1.0);
                    canvas.draw_image_rect(
                        image,
                        Some((&src_rect, skia_safe::canvas::SrcRectConstraint::Fast)),
                        // dst_rect,
                        dst_rect,
                        &skia_safe::Paint::new(white, None),
                    );
                }
            }
        });
        // }
    }
}

impl App for DockApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("Creating dock icon layer surface...");
        // Create a layer shell surface for the dock icon

        // AppContext::enable_layer_engine(500.0, 120.0);

        // let surface = DockItemSurface::new("org.gnome.gedit".to_string(), 100, 100)?;

        let layer_surface = LayerShellSurface::new(Layer::Overlay, "dock-icon", 20, 20)?;
        layer_surface.set_anchor(Anchor::Bottom | Anchor::Left);
        layer_surface.set_margin(0, 0, 200, 200);
        layer_surface.set_keyboard_interactivity(wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand);
        println!("Layer surface created");

        if let Some(surface_style) = layer_surface.base_surface().surface_style() {
            surface_style
                .set_masks_to_bounds(otto_kit::protocols::otto_surface_style_v1::ClipMode::Enabled);
            surface_style.set_corner_radius(20.0);
        }

        self.draw_icon(layer_surface.base_surface());
        // Get the otto_dock_manager global and assign this layer surface to the dock
        // let wl_surface = layer_surface.wl_surface();
        // if let Some(dock_manager) = AppContext::otto_dock_manager() {
        //     println!("Got otto_dock_manager, assigning layer to dock");
        //     // let dock_item = dock_manager.(
        //     //     "org.gnome.gedit".to_string(),
        //     //     wl_surface,
        //     //     AppContext::queue_handle_default(),
        //     //     (),
        //     // );

        //     // // dock_item.set_badge(Some("3".to_string()));

        //     // // println!("Dock item configured with badge");
        //     // self.dock_item = Some(dock_item);
        // } else {
        //     println!("Warning: otto_dock_manager global not available");
        // }

        // Create popup menu
        let menu = ContextMenu::new(vec![
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

        self.menu = Some(menu);
        // self.dock_item_surface = Some(surface);
        self.layer_surface = Some(layer_surface);

        println!("Dock application ready, waiting for configure event");
        Ok(())
    }

    fn on_configure_layer(&mut self, width: i32, height: i32, serial: u32) {
        println!(
            "App on_configure_layer: {}x{}, serial: {}",
            width, height, serial
        );

        if let Some(ref surface) = self.layer_surface {
            self.draw_icon(surface.base_surface());
        }
    }

    fn on_configure(&mut self, _configure: WindowConfigure, _serial: u32) {
        // Not used for layer shell
    }

    fn on_close(&mut self) -> bool {
        true
    }

    fn on_keyboard_event(&mut self, key: u32, key_state: wl_keyboard::KeyState, serial: u32) {
        println!(
            "Keyboard event: key={}, state={:?}, serial={}",
            key, key_state, serial
        );
        // Save the serial for popup grabs
        self.last_input_serial = serial;

        if key_state != wl_keyboard::KeyState::Pressed {
            return;
        }

        // Press 'q' to quit
        if key == 16 {
            println!("Quitting...");
            std::process::exit(0);
        }

        // Press 'b' to toggle badge
        if key == 48 {
            if let Some(ref dock_item) = self.dock_item {
                dock_item.set_badge(Some("5".to_string()));
                println!("Badge updated to 5");
            }
        }

        // Press 'r' to redraw
        if key == 19 {
            println!("Redrawing...");
            if let Some(ref surface) = self.layer_surface {
                self.draw_icon(surface.base_surface());
            }
        }

        // Press 'm' to toggle menu
        if key == 50 {
            // 'm' key
            println!("'m' pressed, showing layer shell menu");
            self.on_dock_menu_requested(0, 0);
        } else if let Some(ref mut menu) = self.menu {
            // Forward other keys to menu for keyboard navigation
            menu.handle_key(key, key_state);
        }
    }

    fn on_keyboard_leave(&mut self, _surface: &wayland_client::protocol::wl_surface::WlSurface) {
        // No-op
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Dock Application with Layer Shell + Menu");
    println!("- Press 'q' to quit");
    println!("- Press 'b' to update badge");
    println!("- Press 'r' to redraw");
    println!("- Press 'm' to show menu (experimental)");

    let app = DockApp {
        dock_item: None,
        layer_surface: None,
        menu: None,
        last_input_serial: 0,
    };

    AppRunner::new(app).run()?;
    Ok(())
}
