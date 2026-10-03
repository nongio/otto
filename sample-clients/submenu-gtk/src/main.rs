use anyhow::Result;
use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, EventControllerMotion, Label, ListBox,
    ListBoxRow, Orientation, Popover, PositionType, ScrolledWindow,
};
use std::cell::RefCell;
use std::rc::Rc;

/// State for tracking which row currently has a submenu open
struct SubmenuState {
    current_row: Option<glib::WeakRef<ListBoxRow>>,
    submenu: Popover,
}

impl SubmenuState {
    fn new() -> Self {
        let submenu = Popover::builder()
            .has_arrow(false)
            .position(PositionType::Right)
            .autohide(false)
            .build();
        Self {
            current_row: None,
            submenu,
        }
    }

    fn show_for_row(&mut self, row: &ListBoxRow, item_index: usize) {
        // Skip if already showing for this row
        if self
            .current_row
            .as_ref()
            .and_then(|w| w.upgrade())
            .as_ref()
            .is_some_and(|current| current == row)
        {
            return;
        }

        self.hide();

        // Build submenu content for this item
        let content = GtkBox::new(Orientation::Vertical, 4);
        content.set_margin_start(8);
        content.set_margin_end(8);
        content.set_margin_top(6);
        content.set_margin_bottom(6);

        let header = Label::new(Some(&format!("Submenu for Item {}", item_index + 1)));
        header.add_css_class("heading");
        content.append(&header);

        // Add submenu items
        for j in 1..=5 {
            let sub_item = Label::new(Some(&format!("  Sub-action {}.{}", item_index + 1, j)));
            sub_item.set_halign(gtk4::Align::Start);
            content.append(&sub_item);
        }

        self.submenu.set_child(Some(&content));

        // Position submenu aligned to the row
        let alloc = row.allocation();
        self.submenu.set_parent(row);
        self.submenu.set_pointing_to(Some(&gdk::Rectangle::new(
            alloc.width(),
            0,
            1,
            alloc.height().max(1),
        )));

        self.submenu.popup();
        self.current_row = Some(row.downgrade());
    }

    fn hide(&mut self) {
        self.submenu.popdown();
        if self.submenu.parent().is_some() {
            self.submenu.unparent();
        }
        self.current_row = None;
    }
}

fn main() -> Result<()> {
    let app = Application::builder()
        .application_id("com.screencomposer.submenu_gtk4")
        .build();

    app.connect_activate(|app| {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Submenu GTK4 Demo")
            .default_width(300)
            .default_height(200)
            .build();

        // Main menu popover
        let main_menu = Popover::builder()
            .has_arrow(true)
            .position(PositionType::Bottom)
            .build();

        // Submenu state
        let submenu_state = Rc::new(RefCell::new(SubmenuState::new()));

        // Build list with 20 items
        let list = ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::None);

        for i in 0..20 {
            let row = ListBoxRow::new();
            let label = Label::new(Some(&format!("Menu Item {}", i + 1)));
            label.set_halign(gtk4::Align::Start);
            label.set_margin_start(12);
            label.set_margin_end(12);
            label.set_margin_top(8);
            label.set_margin_bottom(8);
            row.set_child(Some(&label));

            // Show submenu on hover
            let motion = EventControllerMotion::new();
            let state = submenu_state.clone();
            motion.connect_enter(move |controller, _, _| {
                if let Some(Ok(row)) = controller.widget().map(|w| w.downcast::<ListBoxRow>()) {
                    state.borrow_mut().show_for_row(&row, i);
                }
            });
            row.add_controller(motion);

            list.append(&row);
        }

        // Scrollable container for the menu
        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(300)
            .max_content_height(400)
            .child(&list)
            .build();

        main_menu.set_child(Some(&scroll));

        // Close submenu when main menu closes
        let state_for_close = submenu_state.clone();
        main_menu.connect_closed(move |_| {
            state_for_close.borrow_mut().hide();
        });

        // Button to open the menu
        let button = Button::with_label("Open Menu");
        let menu_for_button = main_menu.clone();
        button.connect_clicked(move |btn| {
            menu_for_button.set_parent(btn);
            menu_for_button.popup();
        });

        // Layout
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_halign(gtk4::Align::Center);
        container.set_valign(gtk4::Align::Center);
        container.append(&button);

        window.set_child(Some(&container));
        window.present();
    });

    app.run();
    Ok(())
}
