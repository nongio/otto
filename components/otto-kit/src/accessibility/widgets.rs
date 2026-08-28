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
use crate::preview::Preview;

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

/// The previewer, described.
///
/// A preview is drawn as one picture — an image, a page of text, a listing, a
/// card of facts — and none of that reaches an assistive technology by itself.
/// This lives in the toolkit rather than in a host because the previewer does:
/// every host that embeds [`crate::preview`] describes it the same way, and a
/// host that adds a preview does not have to work out how.
impl A11yTree {
    /// Describes an open preview of `name`.
    ///
    /// What a screen reader gets depends on what there is to give: text and
    /// listings are read, a picture can only be named and measured, and a
    /// preview that failed says why rather than staying silent.
    pub fn preview(&mut self, id: FocusId, bounds: SkRect, name: &str, preview: &Preview) {
        match preview {
            Preview::Pixels { pages, page, .. } => {
                self.control(id, bounds, Role::Image, true, |node| {
                    node.set_label(name.to_owned());
                    // A page number is the one thing about a rendered page that
                    // can be said without seeing it, and it is what tells the
                    // user they are part-way through a document.
                    node.set_description(if *pages > 1 {
                        format!("Preview, page {page} of {pages}")
                    } else {
                        "Preview".to_owned()
                    });
                });
            }
            Preview::Text {
                lines, truncated, ..
            } => {
                // The text itself, which is the whole point of a text preview:
                // a screen reader can read it, where a sighted user reads the
                // panel.
                //
                // It has to be *runs*, not a value: AT-SPI's Text interface
                // exists only for a node with text-run children, and a
                // document carrying its text in `value` alone reads as an
                // empty document. One run per line, so line-by-line review
                // lands where the lines are.
                let lines: Vec<String> = lines.clone();
                let truncated = *truncated;
                let name = name.to_owned();

                self.group_with(
                    id,
                    Role::Document,
                    |node| {
                        node.set_bounds(bounds_of(bounds));
                        node.set_label(name);
                        if truncated {
                            node.set_description("Preview, shortened".to_owned());
                        }
                    },
                    |tree| {
                        for (index, line) in lines.into_iter().enumerate() {
                            tree.node(
                                FocusId::new(format!("preview-line-{index}")),
                                Role::TextRun,
                                |node| {
                                    // The newline is part of the run: the Text
                                    // interface hands out one string, and
                                    // without it every line runs into the next
                                    // — both read aloud and for line-by-line
                                    // review.
                                    let line = format!("{line}\n");
                                    // `character_lengths` is what maps a
                                    // character offset onto the string; without
                                    // it a range request walks off the end.
                                    let lengths: Vec<u8> =
                                        line.chars().map(|c| c.len_utf8() as u8).collect();
                                    node.set_value(line);
                                    node.set_character_lengths(lengths);
                                },
                            );
                        }
                    },
                );
            }
            Preview::Rows {
                rows,
                truncated,
                summary,
            } => {
                let summary = summary.clone();
                let truncated = *truncated;
                let entries: Vec<(String, String)> = rows
                    .iter()
                    .map(|row| {
                        let kind = if row.is_dir { "Folder" } else { "File" };
                        (row.name.clone(), format!("{kind}, {}", bytes(row.size)))
                    })
                    .collect();

                self.group_with(
                    id,
                    Role::List,
                    |node| {
                        node.set_bounds(bounds_of(bounds));
                        node.set_label(name.to_owned());
                        node.set_description(if truncated {
                            format!("{summary}, shortened")
                        } else {
                            summary
                        });
                    },
                    |tree| {
                        for (index, (label, description)) in entries.into_iter().enumerate() {
                            // The rows are drawn into one layer, so there is no
                            // rectangle to give each of them; they can be read
                            // in order, not pointed at.
                            tree.node(
                                FocusId::new(format!("preview-row-{index}")),
                                Role::ListItem,
                                |node| {
                                    node.set_label(label);
                                    node.set_description(description);
                                },
                            );
                        }
                    },
                );
            }
            Preview::Card {
                title,
                subtitle,
                facts,
                ..
            } => {
                let title = title.clone();
                let subtitle = subtitle.clone();
                let facts: Vec<(String, String)> = facts
                    .iter()
                    .map(|fact| (fact.key.clone(), fact.value.clone()))
                    .collect();

                self.group_with(
                    id,
                    Role::Group,
                    |node| {
                        node.set_bounds(bounds_of(bounds));
                        node.set_label(title);
                        node.set_description(subtitle);
                    },
                    |tree| {
                        // Each fact as its own labelled value: read as "Size,
                        // 4.2 MB" rather than as one run-together paragraph.
                        for (index, (key, value)) in facts.into_iter().enumerate() {
                            tree.node(
                                FocusId::new(format!("preview-fact-{index}")),
                                Role::Label,
                                |node| {
                                    node.set_label(key);
                                    node.set_value(value);
                                },
                            );
                        }
                    },
                );
            }
            Preview::Unavailable { reason } => {
                // Announced rather than left out: "no preview, and here is why"
                // is information, and silence reads as a preview still loading.
                self.status(id, bounds, format!("{name}: {reason}"));
            }
        }
    }
}

/// A size, as a preview row says it.
fn bytes(size: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "kB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use crate::accessibility::tree::node_id;
    use crate::preview::{Fact, Pixels, Preview, Row};

    fn find(update: &accesskit::TreeUpdate, id: FocusId) -> &Node {
        &update
            .nodes
            .iter()
            .find(|(node_id_, _)| *node_id_ == node_id(id))
            .expect("node missing")
            .1
    }

    fn rect() -> SkRect {
        SkRect::from_wh(400.0, 300.0)
    }

    const PREVIEW: FocusId = FocusId::from_raw(0x9001);

    /// A text preview is the one kind a screen reader can actually read, so
    /// the text has to be in the tree rather than described from outside.
    #[test]
    fn a_text_preview_carries_its_text() {
        let mut tree = A11yTree::new("Files");
        tree.preview(
            PREVIEW,
            rect(),
            "notes.txt",
            &Preview::Text {
                lines: vec!["first".into(), "second".into()],
                truncated: true,
                language: String::new(),
            },
        );

        let update = tree.finish();
        let node = find(&update, PREVIEW);
        assert_eq!(node.description().as_deref(), Some("Preview, shortened"));

        // The text lives in runs under the document: that is the only shape
        // AT-SPI's Text interface is offered for.
        let runs: Vec<&str> = node
            .children()
            .iter()
            .filter_map(|id| {
                let run = &update.nodes.iter().find(|(node_id, _)| node_id == id)?.1;
                (run.role() == Role::TextRun).then(|| run.value()).flatten()
            })
            .collect();
        assert_eq!(runs, vec!["first\n", "second\n"]);
    }

    /// A picture cannot be read, only named — and the page number is the one
    /// thing about a rendered page worth saying.
    #[test]
    fn a_paged_preview_says_where_it_is() {
        let mut tree = A11yTree::new("Files");
        tree.preview(
            PREVIEW,
            rect(),
            "report.pdf",
            &Preview::Pixels {
                pixels: Pixels {
                    width: 800,
                    height: 600,
                    intrinsic_width: 800,
                    intrinsic_height: 600,
                    data: Vec::new(),
                },
                pages: 12,
                page: 3,
            },
        );

        let update = tree.finish();
        let node = find(&update, PREVIEW);
        assert_eq!(node.role(), Role::Image);
        assert_eq!(node.description().as_deref(), Some("Preview, page 3 of 12"));
    }

    #[test]
    fn an_archive_listing_is_a_list_of_its_entries() {
        let mut tree = A11yTree::new("Files");
        tree.preview(
            PREVIEW,
            rect(),
            "photos.zip",
            &Preview::Rows {
                rows: vec![
                    Row {
                        name: "cover.png".into(),
                        size: 2_400_000,
                        mtime: 0,
                        icon: String::new(),
                        is_dir: false,
                    },
                    Row {
                        name: "raw".into(),
                        size: 0,
                        mtime: 0,
                        icon: String::new(),
                        is_dir: true,
                    },
                ],
                truncated: false,
                summary: "2 items".into(),
            },
        );

        let update = tree.finish();
        let list = find(&update, PREVIEW);
        assert_eq!(list.children().len(), 2);

        let first = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == list.children()[0])
            .unwrap()
            .1;
        assert_eq!(first.label().as_deref(), Some("cover.png"));
        assert_eq!(first.description().as_deref(), Some("File, 2.4 MB"));
    }

    /// A preview that failed says why. Silence reads as one still loading.
    #[test]
    fn a_preview_that_failed_says_so() {
        let mut tree = A11yTree::new("Files");
        tree.preview(
            PREVIEW,
            rect(),
            "clip.mov",
            &Preview::Unavailable {
                reason: "no decoder".into(),
            },
        );

        let update = tree.finish();
        let node = find(&update, PREVIEW);
        assert_eq!(node.value().as_deref(), Some("clip.mov: no decoder"));
    }

    #[test]
    fn a_metadata_card_reads_each_fact_as_its_own() {
        let mut tree = A11yTree::new("Files");
        tree.preview(
            PREVIEW,
            rect(),
            "song.flac",
            &Preview::Card {
                title: "Song".into(),
                subtitle: "Artist".into(),
                facts: vec![Fact {
                    key: "Duration".into(),
                    value: "3:41".into(),
                }],
                hero: None,
            },
        );

        let update = tree.finish();
        let card = find(&update, PREVIEW);
        assert_eq!(card.label().as_deref(), Some("Song"));
        assert_eq!(card.children().len(), 1);

        let fact = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == card.children()[0])
            .unwrap()
            .1;
        assert_eq!(fact.label().as_deref(), Some("Duration"));
        assert_eq!(fact.value().as_deref(), Some("3:41"));
    }
}
