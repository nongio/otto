//! Example demonstrating the MenuBarNext component
//!
//! This shows:
//! - Creating a menu bar with items
//! - Custom styling
//! - Click-to-activate interaction via pointer events
//! - Opening context menus from menu bar items

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, RwLock};

use otto_kit::components::context_menu::ContextMenu;
use otto_kit::components::label::Label;
use otto_kit::components::menu_bar::{MenuBarIcon, MenuBarRenderer, MenuBarState, MenuBarStyle};
use otto_kit::components::menu_item::MenuItem;
use otto_kit::prelude::*;
use otto_kit::typography::styles;
use skia_safe::Color;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::XdgPositioner;
use smithay_client_toolkit::shell::xdg::XdgSurface as XdgSurfaceTrait;
use wayland_protocols::xdg::shell::client::xdg_positioner;

// Layout constants
const BAR_X: f32 = 20.0;
const BAR_Y: f32 = 30.0;
const BAR_WIDTH: f32 = 860.0;

fn build_menu_bar() -> MenuBarState {
    let mut bar = MenuBarState::new();
    // Icon-only item (like an app logo / system icon)
    bar.add_icon_item(MenuBarIcon::Named("nm-signal-75".into()));
    // Text-only items
    bar.add_item("File");
    bar.add_item("Edit");
    bar.add_item("View");
    bar.add_item("Window");
    bar.add_item("Help");
    // Icon + label item
    bar.add_icon_label_item(MenuBarIcon::Named("audio-volume-medium".into()), "Sound");
    bar
}

/// Number of items that have context menus (the text items: File..Help)
const MENU_ITEM_COUNT: usize = 5;
/// Indices of items that have context menus (skip the leading icon-only item at 0)
const MENU_INDICES: [usize; MENU_ITEM_COUNT] = [1, 2, 3, 4, 5];

fn file_menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action("New Window").with_shortcut("⌘N"),
        MenuItem::action("New Tab").with_shortcut("⌘T"),
        MenuItem::separator(),
        MenuItem::submenu(
            "Open Recent",
            vec![
                MenuItem::action("document1.txt"),
                MenuItem::action("document2.txt"),
                MenuItem::action("project.rs"),
            ],
        ),
        MenuItem::separator(),
        MenuItem::action("Save").with_shortcut("⌘S"),
        MenuItem::action("Save As...").with_shortcut("⇧⌘S"),
        MenuItem::separator(),
        MenuItem::action("Quit").with_shortcut("⌘Q"),
    ]
}

fn edit_menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action("Undo").with_shortcut("⌘Z"),
        MenuItem::action("Redo").with_shortcut("⇧⌘Z"),
        MenuItem::separator(),
        MenuItem::action("Cut").with_shortcut("⌘X"),
        MenuItem::action("Copy").with_shortcut("⌘C"),
        MenuItem::action("Paste").with_shortcut("⌘V"),
        MenuItem::separator(),
        MenuItem::action("Select All").with_shortcut("⌘A"),
    ]
}

fn view_menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action("Zoom In").with_shortcut("⌘+"),
        MenuItem::action("Zoom Out").with_shortcut("⌘-"),
        MenuItem::action("Actual Size").with_shortcut("⌘0"),
        MenuItem::separator(),
        MenuItem::action("Toggle Fullscreen").with_shortcut("F11"),
    ]
}

fn window_menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action("Minimize").with_shortcut("⌘M"),
        MenuItem::action("Maximize"),
        MenuItem::separator(),
        MenuItem::action("Show All Windows"),
    ]
}

fn help_menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action("Documentation"),
        MenuItem::action("Release Notes"),
        MenuItem::separator(),
        MenuItem::action("About"),
    ]
}

struct MenuBarExample {
    window: Option<Window>,
}

impl MenuBarExample {
    fn new() -> Self {
        Self { window: None }
    }
}

impl App for MenuBarExample {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        println!("MenuBarNext Component Demo — click on menu items to open menus");

        let bar_state = Arc::new(RwLock::new(build_menu_bar()));

        let mut window = Window::new("MenuBarNext Component Demo", 900, 400)?;
        window.set_background(Color::from_rgb(245, 245, 245));

        let style = MenuBarStyle::default();

        // --- drawing ---
        let draw_state = bar_state.clone();
        let draw_style = style.clone();
        window.on_draw(move |canvas| {
            let state = draw_state.read().unwrap();

            // Menu bar at top
            canvas.save();
            canvas.translate((BAR_X, BAR_Y));
            MenuBarRenderer::render(canvas, &state, &draw_style, BAR_WIDTH);
            canvas.restore();

            // Content area below
            Label::new("Click a menu bar item to open its dropdown menu")
                .at(24.0, BAR_Y + draw_style.height + 40.0)
                .with_style(styles::BODY)
                .with_color(Color::from_rgb(120, 120, 120))
                .render(canvas);
        });

        // --- context menus for the text items (indices 1..5) ---
        let menu_labels = ["File", "Edit", "View", "Window", "Help"];
        let menus: Vec<ContextMenu> = vec![
            ContextMenu::new(file_menu_items()),
            ContextMenu::new(edit_menu_items()),
            ContextMenu::new(view_menu_items()),
            ContextMenu::new(window_menu_items()),
            ContextMenu::new(help_menu_items()),
        ];

        let menus: Vec<ContextMenu> = menus
            .into_iter()
            .enumerate()
            .map(|(i, menu)| {
                let label = menu_labels[i];
                let cb_state = bar_state.clone();
                let cb_window = window.clone();
                menu.on_item_click(move |item| {
                    println!("{label} > {item}");
                    cb_state.write().unwrap().set_active(None);
                    cb_window.request_frame();
                })
            })
            .collect();

        let menus = Rc::new(RefCell::new(menus));

        // --- pointer click handling (registered directly, no Send needed) ---
        let click_state = bar_state.clone();
        let click_style = style.clone();
        let click_window = window.clone();
        let click_menus = menus.clone();

        AppContext::register_pointer_callback(move |events| {
            // Only handle events for our window surface
            let our_surface = match click_window.wl_surface() {
                Some(s) => s,
                None => return,
            };

            use wayland_client::Proxy;
            for event in events {
                if event.surface.id() != our_surface.id() {
                    continue;
                }
                if let PointerEventKind::Press { serial, .. } = event.kind {
                    let x = event.position.0 as f32;
                    let y = event.position.1 as f32;

                    let mut state = click_state.write().unwrap();
                    if let Some(idx) = hit_test_bar(x, y, &state, &click_style) {
                        let menu_idx = MENU_INDICES.iter().position(|&mi| mi == idx);
                        let menus = click_menus.borrow();

                        // If clicking the already-active item, toggle off
                        if state.active_index() == Some(idx) {
                            state.set_active(None);
                            if let Some(mi) = menu_idx {
                                menus[mi].hide();
                            }
                            click_window.request_frame();
                            return;
                        }

                        // Hide any previously open menu
                        if let Some(prev) = state.active_index() {
                            if let Some(prev_mi) = MENU_INDICES.iter().position(|&mi| mi == prev) {
                                menus[prev_mi].hide();
                            }
                        }

                        state.set_active(Some(idx));
                        let item_x = item_x_offset(idx, &state, &click_style);
                        drop(state);

                        click_window.request_frame();

                        // Show context menu if this item has one
                        if let Some(mi) = menu_idx {
                            if let Some(surface) = click_window.surface() {
                                let xdg_surface = surface.xdg_window().xdg_surface();

                                if let Ok(positioner) =
                                    XdgPositioner::new(AppContext::xdg_shell_state())
                                {
                                    let (menu_w, menu_h) = menus[mi].get_size_at_depth(0);

                                    positioner.set_size(menu_w as i32, menu_h as i32);
                                    positioner.set_anchor_rect(
                                        (BAR_X + item_x) as i32,
                                        (BAR_Y + click_style.height) as i32,
                                        1,
                                        1,
                                    );
                                    positioner.set_anchor(xdg_positioner::Anchor::BottomLeft);
                                    positioner.set_gravity(xdg_positioner::Gravity::BottomRight);

                                    menus[mi].show(xdg_surface, &positioner, serial);
                                }
                            }
                        }
                    }
                }
            }
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        println!("\nClosing MenuBarNext demo");
        false
    }
}

/// Check if (x, y) hits a menu item in the bar. Returns the item index.
fn hit_test_bar(x: f32, y: f32, state: &MenuBarState, style: &MenuBarStyle) -> Option<usize> {
    if y < BAR_Y || y > BAR_Y + style.height {
        return None;
    }

    let local_x = x - BAR_X;
    if local_x < 0.0 {
        return None;
    }

    let font =
        otto_kit::typography::get_font_with_fallback("Inter", style.font_style(), style.font_size);
    let mut x_offset = style.bar_padding_horizontal;

    for (i, item) in state.items().iter().enumerate() {
        let content_width = style.item_content_width(item, &font);
        let item_width = style.item_width(content_width);

        if local_x >= x_offset && local_x <= x_offset + item_width {
            return Some(i);
        }

        x_offset += item_width + style.item_spacing;
    }

    None
}

/// Get the x offset for a menu item (for positioning the popup).
fn item_x_offset(index: usize, state: &MenuBarState, style: &MenuBarStyle) -> f32 {
    let font =
        otto_kit::typography::get_font_with_fallback("Inter", style.font_style(), style.font_size);
    let mut x_offset = style.bar_padding_horizontal;

    for (i, item) in state.items().iter().enumerate() {
        if i == index {
            return x_offset;
        }
        let content_width = style.item_content_width(item, &font);
        x_offset += style.item_width(content_width) + style.item_spacing;
    }

    x_offset
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = MenuBarExample::new();
    AppRunner::new(app).run()?;
    Ok(())
}
