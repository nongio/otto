use std::hash::Hash;

/// Icon associated with a menu item
#[derive(Clone, Debug, PartialEq)]
pub enum MenuItemIcon {
    /// Named icon from the XDG icon theme
    Named(String),
    /// The first of these XDG theme icons the theme has, most specific first,
    /// in its full-colour art rather than the outline glyph many themes ship
    /// at menu size — a folder that should read as a folder, not a symbol.
    Themed(Vec<String>),
    /// Raw ARGB32 pixel data (network byte order / big-endian)
    Pixmap {
        data: Vec<u8>,
        width: i32,
        height: i32,
    },
}

/// Type of menu item
#[derive(Clone, PartialEq)]
pub enum MenuItemKind {
    Action {
        label: String,
        action_id: Option<String>,
        shortcut: Option<String>,
    },
    Submenu {
        label: String,
        items: Vec<MenuItem>,
    },
    Separator,
    /// A value on a track, such as a volume, which the menu stays open for.
    Slider(MenuSlider),
    /// A small caption over the rows of one group.
    Header {
        label: String,
    },
}

/// A slider row: an optional round toggle, an optional name, the track and
/// the value in percent.
///
/// The value is changed in place, so a drag answers at once: the menu draws
/// the new value itself and reports it through
/// [`ContextMenu::on_slider`](crate::components::context_menu::ContextMenu::on_slider).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuSlider {
    /// What the change is reported with.
    pub action_id: String,
    /// The slider's name for a screen reader, such as "Output volume".
    pub accessible_name: String,
    /// From 0.0 to 1.0.
    pub value: f32,
    /// A name drawn before the track, such as an app's.
    pub label: Option<String>,
    /// The round button before the track.
    pub toggle: Option<SliderToggle>,
    /// A compact button after the percentage that opens a submenu, such as
    /// the output an app plays to.
    pub picker: Option<SliderPicker>,
}

/// The pop-up button at the end of a slider row: it shows the current
/// choice and opens the choices as a submenu under it.
///
/// The choices are ordinary menu rows. Picking one reports its action id
/// through [`ContextMenu::on_item_click`](crate::components::context_menu::ContextMenu::on_item_click)
/// and closes the menu, as any submenu row does.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderPicker {
    /// The current choice, drawn on the button and cut short to fit it.
    pub label: String,
    /// What the button does, for a screen reader: "Output for Firefox".
    pub accessible_label: String,
    /// The rows of the submenu it opens.
    pub items: Vec<MenuItem>,
}

/// The round button of a slider row, a toggle such as mute.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderToggle {
    /// The glyph drawn inside it.
    pub icon: MenuItemIcon,
    /// On draws it filled with the accent.
    pub on: bool,
    /// What pressing it does, for a screen reader: "Mute output".
    pub label: String,
}

/// What changed on a slider row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderChange {
    /// The value moved to this, from 0.0 to 1.0.
    Value(f32),
    /// The toggle was pressed and is now this.
    Toggled(bool),
}

impl std::fmt::Debug for MenuItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Action {
                label, shortcut, ..
            } => f
                .debug_struct("Action")
                .field("label", label)
                .field("shortcut", shortcut)
                .finish(),
            Self::Submenu { label, items } => f
                .debug_struct("Submenu")
                .field("label", label)
                .field("items", items)
                .finish(),
            Self::Separator => write!(f, "Separator"),
            Self::Slider(slider) => slider.fmt(f),
            Self::Header { label } => f.debug_struct("Header").field("label", label).finish(),
        }
    }
}

/// How tall a slider row is: room for the round toggle and the knob.
pub const SLIDER_HEIGHT: f32 = 32.0;
/// How tall a header is.
pub const HEADER_HEIGHT: f32 = 22.0;

/// Visual interaction state of a menu item
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisualState {
    Normal,
    Hovered,
    Disabled,
}

/// Pure state/data for a MenuItem
///
/// No rendering, no I/O, just data.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    /// The kind of menu item
    pub kind: MenuItemKind,

    /// Visual state
    pub visual_state: VisualState,

    /// Whether this item is enabled
    pub enabled: bool,

    /// Height in logical pixels (computed based on kind)
    pub height: f32,

    /// Optional icon shown before the label
    pub icon: Option<MenuItemIcon>,

    /// `Some` for a toggle: a check before the label when it is on.
    pub checked: Option<bool>,

    /// Room before the label for a check, so the items of a menu with any
    /// toggle in it keep their labels in line.
    pub check_gutter: bool,
}
impl Hash for MenuItem {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash all fields except callback (which can't be hashed)
        match &self.kind {
            MenuItemKind::Action {
                label,
                action_id,
                shortcut,
                ..
            } => {
                0u8.hash(state);
                label.hash(state);
                action_id.hash(state);
                shortcut.hash(state);
            }
            MenuItemKind::Submenu { label, items } => {
                1u8.hash(state);
                label.hash(state);
                items.hash(state);
            }
            MenuItemKind::Separator => {
                2u8.hash(state);
            }
            MenuItemKind::Slider(slider) => {
                3u8.hash(state);
                slider.action_id.hash(state);
                slider.accessible_name.hash(state);
                slider.value.to_bits().hash(state);
                slider.label.hash(state);
                if let Some(toggle) = &slider.toggle {
                    toggle.on.hash(state);
                    toggle.label.hash(state);
                }
                if let Some(picker) = &slider.picker {
                    picker.label.hash(state);
                    picker.accessible_label.hash(state);
                    picker.items.hash(state);
                }
            }
            MenuItemKind::Header { label } => {
                4u8.hash(state);
                label.hash(state);
            }
        }
        self.visual_state.hash(state);
        self.enabled.hash(state);
        self.checked.hash(state);
        self.check_gutter.hash(state);
        self.height.to_bits().hash(state);
    }
}
impl MenuItem {
    /// Create new menu item data
    pub fn new(kind: MenuItemKind) -> Self {
        let height = match kind {
            MenuItemKind::Separator => 11.0, // MenuItemStyle::separator_height
            MenuItemKind::Slider(_) => SLIDER_HEIGHT,
            MenuItemKind::Header { .. } => HEADER_HEIGHT,
            _ => 23.0, // MenuItemStyle::line_height
        };

        Self {
            kind,
            visual_state: VisualState::Normal,
            enabled: true,
            height,
            icon: None,
            checked: None,
            check_gutter: false,
        }
    }

    /// Create an action item
    pub fn action(label: impl Into<String>) -> Self {
        Self::new(MenuItemKind::Action {
            label: label.into(),
            action_id: None,
            shortcut: None,
        })
    }

    /// Create a separator
    pub fn separator() -> Self {
        Self::new(MenuItemKind::Separator)
    }

    /// A slider row reporting changes as `action_id`, named
    /// `accessible_name` for a screen reader, at `value` from 0.0 to 1.0.
    pub fn slider(
        action_id: impl Into<String>,
        accessible_name: impl Into<String>,
        value: f32,
    ) -> Self {
        Self::new(MenuItemKind::Slider(MenuSlider {
            action_id: action_id.into(),
            accessible_name: accessible_name.into(),
            value: value.clamp(0.0, 1.0),
            label: None,
            toggle: None,
            picker: None,
        }))
    }

    /// A caption over a group of rows.
    pub fn header(label: impl Into<String>) -> Self {
        Self::new(MenuItemKind::Header {
            label: label.into(),
        })
    }

    /// Create a submenu item
    pub fn submenu(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self::new(MenuItemKind::Submenu {
            label: label.into(),
            items,
        })
    }

    // === Getters ===

    pub fn kind(&self) -> &MenuItemKind {
        &self.kind
    }

    pub fn visual_state(&self) -> VisualState {
        self.visual_state
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_separator(&self) -> bool {
        matches!(self.kind, MenuItemKind::Separator)
    }

    /// Whether the keyboard can land on this row: anything but a separator
    /// or a header.
    pub fn is_selectable(&self) -> bool {
        !matches!(
            self.kind,
            MenuItemKind::Separator | MenuItemKind::Header { .. }
        )
    }

    /// The slider, for a slider row.
    pub fn slider_data(&self) -> Option<&MenuSlider> {
        match &self.kind {
            MenuItemKind::Slider(slider) => Some(slider),
            _ => None,
        }
    }

    /// The slider, to change in place.
    pub fn slider_data_mut(&mut self) -> Option<&mut MenuSlider> {
        match &mut self.kind {
            MenuItemKind::Slider(slider) => Some(slider),
            _ => None,
        }
    }

    pub fn has_submenu(&self) -> bool {
        matches!(self.kind, MenuItemKind::Submenu { .. })
    }

    /// Whether this row can open a submenu: a submenu row, which opens on
    /// hover, or a slider row with a picker, which opens on a press.
    pub fn opens_submenu(&self) -> bool {
        self.submenu_items().is_some()
    }

    pub fn label(&self) -> Option<&str> {
        match &self.kind {
            MenuItemKind::Action { label, .. } => Some(label),
            MenuItemKind::Submenu { label, .. } => Some(label),
            MenuItemKind::Header { label } => Some(label),
            // Its accessible name, which is what keeps a highlight on it
            // when the menu's items are replaced.
            MenuItemKind::Slider(slider) => Some(&slider.accessible_name),
            MenuItemKind::Separator => None,
        }
    }

    pub fn shortcut(&self) -> Option<&str> {
        match &self.kind {
            MenuItemKind::Action { shortcut, .. } => shortcut.as_deref(),
            _ => None,
        }
    }

    pub fn action_id(&self) -> Option<&str> {
        match &self.kind {
            MenuItemKind::Action { action_id, .. } => action_id.as_deref(),
            _ => None,
        }
    }

    pub fn submenu_items(&self) -> Option<&[MenuItem]> {
        match &self.kind {
            MenuItemKind::Submenu { items, .. } => Some(items),
            MenuItemKind::Slider(MenuSlider {
                picker: Some(picker),
                ..
            }) => Some(&picker.items),
            _ => None,
        }
    }

    // === Builder API ===

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        if let MenuItemKind::Action {
            label, action_id, ..
        } = self.kind
        {
            self.kind = MenuItemKind::Action {
                label,
                action_id,
                shortcut: Some(shortcut.into()),
            };
        }
        self
    }

    pub fn with_action_id(mut self, id: impl Into<String>) -> Self {
        if let MenuItemKind::Action {
            label, shortcut, ..
        } = self.kind
        {
            self.kind = MenuItemKind::Action {
                label,
                action_id: Some(id.into()),
                shortcut,
            };
        }
        self
    }

    /// A name drawn before a slider's track.
    pub fn with_slider_label(mut self, label: impl Into<String>) -> Self {
        if let Some(slider) = self.slider_data_mut() {
            slider.label = Some(label.into());
        }
        self
    }

    /// A round toggle before a slider's track, drawn with `icon`, filled
    /// while `on`, and called `label` for a screen reader.
    pub fn with_slider_toggle(
        mut self,
        icon: MenuItemIcon,
        on: bool,
        label: impl Into<String>,
    ) -> Self {
        if let Some(slider) = self.slider_data_mut() {
            slider.toggle = Some(SliderToggle {
                icon,
                on,
                label: label.into(),
            });
        }
        self
    }

    /// A pop-up button after a slider's percentage, showing `label` and
    /// opening `items` as a submenu under it; called `accessible_label` for
    /// a screen reader.
    pub fn with_slider_picker(
        mut self,
        label: impl Into<String>,
        accessible_label: impl Into<String>,
        items: Vec<MenuItem>,
    ) -> Self {
        if let Some(slider) = self.slider_data_mut() {
            slider.picker = Some(SliderPicker {
                label: label.into(),
                accessible_label: accessible_label.into(),
                items,
            });
        }
        self
    }

    pub fn with_icon(mut self, icon: MenuItemIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_visual_state(mut self, state: VisualState) -> Self {
        self.visual_state = state;
        self
    }

    /// A toggle, checked when `checked`.
    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self.check_gutter = true;
        self
    }

    /// Room for a check before the label, though this item has none.
    pub fn with_check_gutter(mut self) -> Self {
        self.check_gutter = true;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self.visual_state = VisualState::Disabled;
        self
    }

    // === State Mutations ===

    pub fn set_visual_state(&mut self, state: VisualState) {
        self.visual_state = state;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.visual_state = VisualState::Disabled;
        }
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        if self.enabled {
            self.visual_state = if hovered {
                VisualState::Hovered
            } else {
                VisualState::Normal
            };
        }
    }
}

impl Default for MenuItem {
    fn default() -> Self {
        Self::separator()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_item() {
        let item = MenuItem::action("Copy").with_shortcut("Ctrl+C");

        assert_eq!(item.label(), Some("Copy"));
        assert_eq!(item.shortcut(), Some("Ctrl+C"));
        assert!(!item.has_submenu());
        assert!(item.is_enabled());
        assert_eq!(item.height, 23.0);
    }

    #[test]
    fn test_separator() {
        let item = MenuItem::separator();

        assert!(item.is_separator());
        assert_eq!(item.label(), None);
        assert_eq!(item.height, 11.0);
    }

    #[test]
    fn test_submenu() {
        let item = MenuItem::submenu(
            "File",
            vec![MenuItem::action("New"), MenuItem::action("Open")],
        );

        assert!(item.has_submenu());
        assert_eq!(item.label(), Some("File"));
        assert_eq!(item.submenu_items().unwrap().len(), 2);
    }

    #[test]
    fn a_slider_keeps_its_value_in_range_and_a_header_is_not_selectable() {
        let slider = MenuItem::slider("out", "Output volume", 1.4).with_slider_toggle(
            MenuItemIcon::Named("audio-volume-high-symbolic".into()),
            true,
            "Mute output",
        );
        assert_eq!(slider.slider_data().map(|s| s.value), Some(1.0));
        assert!(slider.is_selectable());
        assert_eq!(slider.label(), Some("Output volume"));
        assert_eq!(slider.height, SLIDER_HEIGHT);

        let header = MenuItem::header("Output");
        assert!(!header.is_selectable());
        assert_eq!(header.label(), Some("Output"));
    }

    #[test]
    fn a_slider_picker_opens_its_choices_as_a_submenu() {
        let plain = MenuItem::slider("app:Firefox", "Firefox volume", 0.5);
        assert!(!plain.opens_submenu());

        let picked = plain.with_slider_picker(
            "Speakers",
            "Output for Firefox",
            vec![
                MenuItem::action("Follow the default").with_checked(false),
                MenuItem::action("Speakers").with_checked(true),
            ],
        );
        assert!(picked.opens_submenu());
        // Not a hover submenu: it opens from its button.
        assert!(!picked.has_submenu());
        assert_eq!(picked.submenu_items().map(<[MenuItem]>::len), Some(2));
    }

    #[test]
    fn test_hover_state() {
        let mut item = MenuItem::action("Test");

        assert_eq!(item.visual_state(), VisualState::Normal);

        item.set_hovered(true);
        assert_eq!(item.visual_state(), VisualState::Hovered);

        item.set_hovered(false);
        assert_eq!(item.visual_state(), VisualState::Normal);
    }

    #[test]
    fn test_disabled_state() {
        let mut item = MenuItem::action("Test").disabled();

        assert!(!item.is_enabled());
        assert_eq!(item.visual_state(), VisualState::Disabled);

        // Hovering disabled item should not change visual state
        item.set_hovered(true);
        assert_eq!(item.visual_state(), VisualState::Disabled);
    }
}
