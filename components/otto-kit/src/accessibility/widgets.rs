//! One description per kit widget.
//!
//! A screen reader is unforgiving about detail — a switch that reports itself
//! as a button, or a slider with no range, is read out wrongly — and there is
//! no way to notice from inside the application that it happened. So the
//! mapping from widget to accessible node is written once, here, rather than
//! left to every application to get right.
//!
//! Each method takes the same bounds the widget was drawn with, in points and
//! window-relative: the same rectangle the application passes to
//! [`crate::focus::FocusRing::add`], so what a screen reader highlights is
//! where the control actually is.

use accesskit::{Action, Live, Node, Rect, Role, Toggled};
use skia_safe::Rect as SkRect;

use super::tree::A11yTree;
use crate::components::menu_item::{MenuItem, MenuItemKind};
use crate::components::text_input::TextInputState;
use crate::focus::FocusId;

fn bounds_of(rect: SkRect) -> Rect {
    Rect::new(
        f64::from(rect.left),
        f64::from(rect.top),
        f64::from(rect.right),
        f64::from(rect.bottom),
    )
}

impl A11yTree {
    /// The shape every control below is built from: placed, labelled, and
    /// focusable unless it is disabled.
    pub fn control(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        role: Role,
        enabled: bool,
        build: impl FnOnce(&mut Node),
    ) {
        self.node(id, role, |node| {
            node.set_bounds(bounds_of(bounds));
            if enabled {
                // Without this a screen reader cannot move the keyboard to the
                // control it is reading.
                node.add_action(Action::Focus);
            } else {
                node.set_disabled();
            }
            build(node);
        });
    }

    /// Text that is not a control — a caption, a field's title.
    ///
    /// The text goes in `value`, not `label`: for `Role::Label` that is where
    /// AT-SPI expects to find it.
    pub fn label(&mut self, id: FocusId, bounds: SkRect, text: impl Into<String>) {
        self.node(id, Role::Label, |node| {
            node.set_bounds(bounds_of(bounds));
            node.set_value(text.into());
        });
    }

    pub fn button(&mut self, id: FocusId, bounds: SkRect, label: impl Into<String>, enabled: bool) {
        self.control(id, bounds, Role::Button, enabled, |node| {
            node.set_label(label.into());
            if enabled {
                node.add_action(Action::Click);
            }
        });
    }

    /// The kit's switch — `toggle::draw`.
    pub fn toggle(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        label: impl Into<String>,
        on: bool,
        enabled: bool,
    ) {
        self.control(id, bounds, Role::Switch, enabled, |node| {
            node.set_label(label.into());
            node.set_toggled(if on { Toggled::True } else { Toggled::False });
            if enabled {
                node.add_action(Action::Click);
            }
        });
    }

    pub fn checkbox(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        label: impl Into<String>,
        checked: bool,
        enabled: bool,
    ) {
        self.control(id, bounds, Role::CheckBox, enabled, |node| {
            node.set_label(label.into());
            node.set_toggled(if checked {
                Toggled::True
            } else {
                Toggled::False
            });
            if enabled {
                node.add_action(Action::Click);
            }
        });
    }

    pub fn radio(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        label: impl Into<String>,
        selected: bool,
        enabled: bool,
    ) {
        self.control(id, bounds, Role::RadioButton, enabled, |node| {
            node.set_label(label.into());
            node.set_selected(selected);
            if enabled {
                node.add_action(Action::Click);
            }
        });
    }

    /// A slider, with the range a screen reader announces the value against.
    #[allow(clippy::too_many_arguments)]
    pub fn slider(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        label: impl Into<String>,
        value: f64,
        range: std::ops::RangeInclusive<f64>,
        step: f64,
        enabled: bool,
    ) {
        self.control(id, bounds, Role::Slider, enabled, |node| {
            node.set_label(label.into());
            node.set_numeric_value(value);
            node.set_min_numeric_value(*range.start());
            node.set_max_numeric_value(*range.end());
            node.set_numeric_value_step(step);
            if enabled {
                node.add_action(Action::SetValue);
                node.add_action(Action::Increment);
                node.add_action(Action::Decrement);
            }
        });
    }

    /// A text field, reported with its current contents and caret.
    ///
    /// A password field reports that it is one and never its value — the value
    /// would otherwise be spoken aloud, or reach anything listening on the
    /// accessibility bus.
    pub fn text_field(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        label: impl Into<String>,
        state: &TextInputState,
        enabled: bool,
    ) {
        let password = state.password;
        self.control(
            id,
            bounds,
            if password {
                Role::PasswordInput
            } else {
                Role::TextInput
            },
            enabled,
            |node| {
                node.set_label(label.into());
                if !password {
                    node.set_value(state.value().to_owned());
                }
                if enabled {
                    node.add_action(Action::SetValue);
                }
            },
        );
    }

    /// A dropdown: the field itself. Its list, when open, is a separate group.
    pub fn combo_box(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        label: impl Into<String>,
        value: impl Into<String>,
        expanded: bool,
        enabled: bool,
    ) {
        self.control(id, bounds, Role::ComboBox, enabled, |node| {
            node.set_label(label.into());
            node.set_value(value.into());
            node.set_expanded(expanded);
            if enabled {
                node.add_action(Action::Click);
                node.add_action(Action::Expand);
                node.add_action(Action::Collapse);
            }
        });
    }

    /// A list, with its rows inside it.
    ///
    /// The rows are nodes even though a kit list draws every one of them into a
    /// single layer: what is drawn and what is announced are different trees.
    pub fn list(&mut self, id: FocusId, bounds: SkRect, build: impl FnOnce(&mut Self)) {
        self.group_with(
            id,
            Role::List,
            |node| node.set_bounds(bounds_of(bounds)),
            build,
        );
    }

    pub fn list_row(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        label: impl Into<String>,
        selected: bool,
    ) {
        self.control(id, bounds, Role::ListItem, true, |node| {
            node.set_label(label.into());
            node.set_selected(selected);
            node.add_action(Action::Click);
        });
    }

    /// A menu, with its items inside it.
    pub fn menu(&mut self, id: FocusId, bounds: SkRect, build: impl FnOnce(&mut Self)) {
        self.group_with(
            id,
            Role::Menu,
            |node| node.set_bounds(bounds_of(bounds)),
            build,
        );
    }

    /// One item of a menu, described from the item itself so its kind, its
    /// shortcut and whether it is enabled cannot be reported differently from
    /// how it is drawn.
    pub fn menu_item(&mut self, id: FocusId, bounds: SkRect, item: &MenuItem) {
        match &item.kind {
            MenuItemKind::Separator => {
                self.node(id, Role::Splitter, |node| {
                    node.set_bounds(bounds_of(bounds));
                });
            }
            MenuItemKind::Action {
                label, shortcut, ..
            } => {
                self.control(id, bounds, Role::MenuItem, item.enabled, |node| {
                    node.set_label(label.clone());
                    if let Some(shortcut) = shortcut {
                        node.set_keyboard_shortcut(shortcut.clone());
                    }
                    if item.enabled {
                        node.add_action(Action::Click);
                    }
                });
            }
            MenuItemKind::Submenu { label, .. } => {
                self.control(id, bounds, Role::MenuItem, item.enabled, |node| {
                    node.set_label(label.clone());
                    node.set_has_popup(accesskit::HasPopup::Menu);
                    if item.enabled {
                        node.add_action(Action::Click);
                        node.add_action(Action::Expand);
                    }
                });
            }
        }
    }

    /// A scrolling viewport, with what it contains inside it.
    ///
    /// `offset` and `extent` are in points: how far the content has been
    /// scrolled, and how much of it there is beyond the viewport.
    pub fn scroll_view(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        offset: (f32, f32),
        extent: (f32, f32),
        build: impl FnOnce(&mut Self),
    ) {
        self.group_with(
            id,
            Role::ScrollView,
            |node| {
                node.set_bounds(bounds_of(bounds));
                node.set_scroll_x(f64::from(offset.0));
                node.set_scroll_x_min(0.0);
                node.set_scroll_x_max(f64::from(extent.0));
                node.set_scroll_y(f64::from(offset.1));
                node.set_scroll_y_min(0.0);
                node.set_scroll_y_max(f64::from(extent.1));
                node.add_action(Action::ScrollIntoView);
            },
            build,
        );
    }

    /// A toolbar, a sidebar, a pane: something that holds controls and is worth
    /// naming, but is not itself operated.
    pub fn region(
        &mut self,
        id: FocusId,
        bounds: SkRect,
        role: Role,
        label: impl Into<String>,
        build: impl FnOnce(&mut Self),
    ) {
        let label = label.into();
        self.group_with(
            id,
            role,
            |node| {
                node.set_bounds(bounds_of(bounds));
                node.set_label(label);
            },
            build,
        );
    }

    /// An icon or picture that carries meaning. One that is purely decorative
    /// should be left out of the tree entirely rather than described.
    pub fn image(&mut self, id: FocusId, bounds: SkRect, description: impl Into<String>) {
        self.node(id, Role::Image, |node| {
            node.set_bounds(bounds_of(bounds));
            node.set_label(description.into());
        });
    }

    /// Text that changes and should be read when it does — a progress message,
    /// a result count. Announced without the user going looking for it.
    pub fn status(&mut self, id: FocusId, bounds: SkRect, text: impl Into<String>) {
        self.node(id, Role::Status, |node| {
            node.set_bounds(bounds_of(bounds));
            node.set_value(text.into());
            node.set_live(Live::Polite);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accessibility::tree::node_id;

    fn rect() -> SkRect {
        SkRect::new(0.0, 0.0, 100.0, 24.0)
    }

    fn find(update: &accesskit::TreeUpdate, id: FocusId) -> &Node {
        &update
            .nodes
            .iter()
            .find(|(node_id_, _)| *node_id_ == node_id(id))
            .expect("node missing")
            .1
    }

    #[test]
    fn a_switch_reports_its_state_and_can_be_clicked() {
        let mut tree = A11yTree::new("Settings");
        tree.toggle(FocusId::new("dark"), rect(), "Dark mode", true, true);

        let update = tree.finish();
        let node = find(&update, FocusId::new("dark"));
        assert_eq!(node.role(), Role::Switch);
        assert_eq!(node.toggled(), Some(Toggled::True));
        assert!(node.supports_action(Action::Click));
        assert!(!node.is_disabled());
    }

    #[test]
    fn a_disabled_control_is_disabled_and_takes_no_actions() {
        let mut tree = A11yTree::new("Settings");
        tree.button(FocusId::new("apply"), rect(), "Apply", false);

        let update = tree.finish();
        let node = find(&update, FocusId::new("apply"));
        assert!(node.is_disabled());
        assert!(!node.supports_action(Action::Click));
        assert!(!node.supports_action(Action::Focus));
    }

    #[test]
    fn a_slider_carries_the_range_its_value_means_anything_against() {
        let mut tree = A11yTree::new("Settings");
        tree.slider(
            FocusId::new("volume"),
            rect(),
            "Volume",
            0.4,
            0.0..=1.0,
            0.05,
            true,
        );

        let update = tree.finish();
        let node = find(&update, FocusId::new("volume"));
        assert_eq!(node.numeric_value(), Some(0.4));
        assert_eq!(node.min_numeric_value(), Some(0.0));
        assert_eq!(node.max_numeric_value(), Some(1.0));
        assert!(node.supports_action(Action::SetValue));
    }

    #[test]
    fn a_password_field_never_reports_its_contents() {
        let mut tree = A11yTree::new("Login");
        let state = TextInputState::new("hunter2").with_password(true);
        tree.text_field(FocusId::new("password"), rect(), "Password", &state, true);

        let update = tree.finish();
        let node = find(&update, FocusId::new("password"));
        assert_eq!(node.role(), Role::PasswordInput);
        assert_eq!(node.value(), None);
    }

    #[test]
    fn a_text_field_reports_what_it_holds() {
        let mut tree = A11yTree::new("Files");
        let state = TextInputState::new("report.pdf");
        tree.text_field(FocusId::new("name"), rect(), "Name", &state, true);

        let update = tree.finish();
        let node = find(&update, FocusId::new("name"));
        assert_eq!(node.role(), Role::TextInput);
        assert_eq!(node.value(), Some("report.pdf"));
    }

    #[test]
    fn rows_are_children_of_their_list() {
        let mut tree = A11yTree::new("Files");
        tree.list(FocusId::new("files"), rect(), |tree| {
            tree.list_row(FocusId::new("row-0"), rect(), "Documents", true);
            tree.list_row(FocusId::new("row-1"), rect(), "Pictures", false);
        });

        let update = tree.finish();
        let list = find(&update, FocusId::new("files"));
        assert_eq!(list.children().len(), 2);
        assert!(find(&update, FocusId::new("row-0")).is_selected() == Some(true));
    }

    #[test]
    fn a_separator_is_not_a_menu_item() {
        let mut tree = A11yTree::new("Menu");
        let separator = MenuItem::separator();
        tree.menu(FocusId::new("menu"), rect(), |tree| {
            tree.menu_item(FocusId::new("sep"), rect(), &separator);
        });

        let update = tree.finish();
        let node = find(&update, FocusId::new("sep"));
        assert_eq!(node.role(), Role::Splitter);
    }
}
