use layers::view::{LayerTree, LayerTreeBuilder, RenderLayerTree, View};
use otto_kit::{
    components::context_menu::{ContextMenuRenderer, ContextMenuState},
    input::keycodes,
    prelude::*,
    protocols::otto_surface_style_v1,
};
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::protocol::wl_keyboard;

/// Your application struct - define your app state here
struct MyApp {
    window: Option<Window>,
    layer: Option<layers::prelude::Layer>,
    view: layers::prelude::View<ContextMenuState>,
}

/// Implement the App trait to make your struct runnable with AppRunner
///
/// Required methods:
///   - fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>>
///   - fn on_close(&mut self) -> bool
impl App for MyApp {
    /// Called when the app is ready and the window has been created
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("App is ready!");
        AppContext::enable_layer_engine(1000.0, 1000.0);

        let mut window = Window::new("Simple Window Example", 800, 600)?;

        // Customize the window
        window.set_background(skia_safe::Color::from_argb(50, 180, 180, 180));

        // Customize the window appearance
        if let Some(surface_style) = window.surface_style() {
            surface_style.set_opacity(1.0);
            surface_style.set_background_color(0.9, 0.9, 0.95, 1.0);
            surface_style.set_corner_radius(12.0); // Larger radius than default
            surface_style.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
            surface_style.set_border(1.0, 0.9, 0.9, 0.9, 0.9);
        }

        let layer = AppContext::layers_renderer(|renderer| {
            let l = renderer.engine().new_layer();
            renderer.engine().add_layer(&l);
            l
        });
        self.layer = layer;

        if let Some(layer) = &self.layer {
            // println!("Configuring the layer...");
            layer.set_layout_style(layers::taffy::Style {
                position: layers::taffy::style::Position::Absolute,
                ..Default::default()
            });
            self.view.mount_layer(layer.clone());
        }

        self.window = Some(window);

        Ok(())
    }
    fn on_keyboard_event(&mut self, key: u32, key_state: wl_keyboard::KeyState, _serial: u32) {
        // Only handle key press events
        if key_state != wl_keyboard::KeyState::Pressed {
            return;
        }

        let mut state = self.view.get_state();

        match key {
            keycodes::DOWN => {
                println!("DOWN key pressed at depth {}", state.depth());
                state.select_next_at_depth(None);
                self.view.update_state(&state);
            }
            keycodes::UP => {
                println!("UP key pressed at depth {}", state.depth());
                state.select_previous_at_depth(None);
                self.view.update_state(&state);
                self.redraw();
            }
            keycodes::RIGHT => {
                let current_depth = state.depth();
                let (has_submenu, selected_idx) =
                    { (state.selected_has_submenu(None), state.selected_index(None)) };

                if has_submenu {
                    if let Some(idx) = selected_idx {
                        println!(
                            "Opening submenu at depth {} for item {}",
                            current_depth, idx
                        );
                        state.open_submenu(current_depth, idx);
                        state.select_at_depth(current_depth + 1, Some(0));
                        self.view.update_state(&state);
                        self.redraw();
                    }
                }
            }
            keycodes::LEFT => {
                let current_depth = state.depth();
                if current_depth > 0 {
                    println!("Closing submenu, going back to depth {}", current_depth - 1);
                    let target_depth = current_depth - 1;
                    state.close_submenus_from(target_depth);
                    self.view.update_state(&state);
                    self.redraw();
                }
            }
            keycodes::ENTER => {
                // let current_depth = state.depth();
                let label = state.selected_label(None).map(|s| s.to_string());

                if let Some(label) = label {
                    println!("Selected item: {}", label);
                    // You can add item click handling here
                }
            }
            keycodes::ESC => {
                println!("ESC pressed - resetting menu");
                let mut state = self.view.get_state();
                state.reset();
                self.view.update_state(&state);
                self.redraw();
            }
            _ => {}
        }
    }
    fn on_pointer_event(
        &mut self,
        _events: &[smithay_client_toolkit::seat::pointer::PointerEvent],
    ) {
        // println!("Pointer event received: {:?}", _events);
        if let Some(window) = self.window.as_ref() {
            let toplevel = window.surface().unwrap();

            AppContext::layers_renderer(|renderer| {
                toplevel.draw(|canvas| {
                    if let Some(root) = renderer.engine().scene_root() {
                        layers::prelude::draw_scene(canvas, renderer.engine().scene(), root);
                    }
                });
                toplevel.window().commit();
                // toplevel.draw(draw_fn);
            });
        }
    }
    /// Called when the user requests to close the window
    fn on_close(&mut self) -> bool {
        println!("App is closing...");
        true // Return false to prevent closing
    }
}

impl MyApp {
    /// Helper function to redraw the window
    fn redraw(&mut self) {
        if let Some(window) = self.window.as_ref() {
            if let Some(layer) = self.layer.as_ref() {
                let toplevel = window.surface().unwrap();
                AppContext::layers_renderer(|renderer| {
                    toplevel.draw(|canvas| {
                        layers::prelude::draw_scene(canvas, renderer.engine().scene(), layer.id);
                    });
                    toplevel.window().commit();
                });
            }
        }
    }
}
fn render_menu(state: &ContextMenuState, _view: &View<ContextMenuState>) -> LayerTree {
    let style = state.style.clone();

    // Create a container layer for all menu depths
    let mut children = Vec::new();
    let mut x_offset = 0.0;

    // Render each depth level (depth 0 = root, depth 1+ = submenus)
    for depth in 0..=state.depth() {
        let items_vec = state.items_at_depth(depth).to_vec();

        // Skip empty depths (happens when submenus are closed)
        if items_vec.is_empty() {
            continue;
        }

        let selected = state.selected_at_depth(depth);
        let (width, height) = ContextMenuRenderer::measure_items(&items_vec, &style);

        // Clone data for the closure
        let items_for_closure = items_vec.clone();
        let style_for_closure = style.clone();

        let draw_depth = move |canvas: &skia_safe::Canvas, w: f32, h: f32| {
            ContextMenuRenderer::render_depth(
                canvas,
                &items_for_closure,
                selected,
                &style_for_closure,
                w,
                h,
            );
            skia_safe::Rect::from_xywh(0.0, 0.0, w, h)
        };

        let depth_layer = LayerTreeBuilder::default()
            .key(format!("menu-depth-{}", depth))
            .position(layers::types::Point::new(x_offset, 0.0))
            .size(layers::types::Size::points(width, height))
            .opacity((
                1.0,
                Some(layers::prelude::Transition {
                    delay: 0.2,
                    timing: layers::prelude::TimingFunction::ease_out_quad(0.8),
                }),
            ))
            .border_corner_radius(layers::prelude::BorderRadius::new_single(24.0))
            .content(Some(draw_depth))
            .background_color(layers::prelude::Color::new_rgba(0.7, 0.7, 0.7, 1.0))
            .pointer_events(false)
            .build()
            .unwrap();

        children.push(depth_layer);

        // Offset next submenu to the right with a small gap
        x_offset += width + 8.0;
    }

    // Return container with all depth layers
    LayerTreeBuilder::default()
        .key("menu-container")
        .children(children)
        .build()
        .unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let menu = ContextMenuState::new(vec![
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
    ]);
    let view = View::new("key", menu, Box::new(render_menu));
    // Create your app instance
    let app = MyApp {
        window: None,
        layer: None,
        view,
    };

    // Run the app - this handles all the Wayland/window setup
    AppRunner::new(app).run()?;

    Ok(())
}
