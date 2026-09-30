//! The desk's edit mode: the panel moved and resized with the pointer.
//!
//! Settings asks for it over `org.otto.Desk1` (see [`crate::desk_service`]).
//! While it is on, the desk sits above the windows with an accent outline and
//! handles round its panel; a drag inside moves the panel, a drag on an edge
//! or a corner resizes it, and the grid reflows as it goes. Done, Return or
//! Escape keeps the result and writes it into `files.toml`; Cancel puts the
//! panel back where it was. The geometry itself is [`crate::desk`]'s; this is
//! the state of one edit and the events that drive it.

// Rust guideline compliant 2026-02-21

use smithay_client_toolkit::seat::pointer::PointerEvent;

use super::listing_pointer::{After, PointerAt};
use super::*;
use crate::desk::{self, Grip};
use crate::view::{DeskEdit, DeskEditButton};

/// One edit in progress.
#[derive(Debug, Clone, Copy)]
pub(super) struct Editing {
    /// What a press took hold of, the panel as it was then, and where the
    /// press landed, while a drag is under way.
    drag: Option<(Grip, Rect, f32, f32)>,
    /// The cursor last set, so a move only asks for a new one on a change.
    cursor: CursorShape,
}

/// The cursor over what a press would take hold of: a resize arrow along the
/// edge or corner, a hand over the panel, a pointer over a button.
fn cursor_for(grip: Option<Grip>, over_button: bool, dragging: bool) -> CursorShape {
    if over_button {
        return CursorShape::Pointer;
    }
    match grip {
        Some(Grip::Move) if dragging => CursorShape::Grabbing,
        Some(Grip::Move) => CursorShape::Grab,
        Some(Grip::Left | Grip::Right) => CursorShape::EwResize,
        Some(Grip::Top | Grip::Bottom) => CursorShape::NsResize,
        Some(Grip::TopLeft | Grip::BottomRight) => CursorShape::NwseResize,
        Some(Grip::TopRight | Grip::BottomLeft) => CursorShape::NeswResize,
        None => CursorShape::Default,
    }
}

impl Browser {
    /// Enter edit mode, starting from the panel as it is now. Nothing
    /// happens outside the desk, or when an edit is already under way.
    pub(super) fn begin_desk_edit(&mut self) {
        if !self.desk || self.desk_editing.is_some() {
            return;
        }
        self.close_desk_fan();
        let panel = view::desk_panel_rect(self.size.0, self.size.1);
        view::set_desk_edit(Some(DeskEdit {
            panel,
            pressed: None,
        }));
        self.desk_editing = Some(Editing {
            drag: None,
            cursor: CursorShape::Default,
        });
        self.dirty = true;
    }

    /// Leave edit mode. With `keep`, the edited panel becomes the desk's
    /// geometry and is written into `files.toml`; without, the panel goes
    /// back to what the config says.
    pub(super) fn finish_desk_edit(&mut self, keep: bool) {
        if self.desk_editing.take().is_none() {
            return;
        }
        let edited = view::desk_edit().map(|edit| edit.panel);
        view::set_desk_edit(None);
        AppContext::set_cursor_shape(CursorShape::Default);
        self.dirty = true;
        let Some(panel) = edited.filter(|_| keep) else {
            return;
        };
        let (anchor, size, position) = desk::stored_geometry(panel, self.size);
        let mut layout = view::desk_layout();
        layout.anchor = anchor;
        layout.size = size;
        layout.position = position;
        view::set_desk_layout(layout);
        // Recorded as the config in force, so the desk's own write, seen by
        // its watch a moment later, reads as no change at all.
        if let Some(config) = self.desk_config.as_mut() {
            config.anchor = anchor;
            config.size = size;
            config.position = position;
        }
        if let Err(error) = desk::save_geometry(anchor, size, position) {
            tracing::warn!(%error, "cannot save the desk's size and position");
        }
    }

    /// The pointer while edit mode is up: it takes every event, so nothing
    /// reaches the icons under the outline. `None` when edit mode is off.
    pub(super) fn desk_edit_pointer(
        &mut self,
        event: &PointerEvent,
        at: PointerAt,
    ) -> Option<After> {
        let editing = self.desk_editing?;
        let edit = view::desk_edit()?;
        let (x, y) = (at.x, at.y);
        match event.kind {
            PointerEventKind::Press { button, .. } if button != BTN_RIGHT => {
                if let Some(pressed) = view::desk_edit_button_at(edit.panel, x, y) {
                    view::set_desk_edit(Some(DeskEdit {
                        pressed: Some(pressed),
                        ..edit
                    }));
                } else if let Some(grip) = desk::grip_at(edit.panel, x, y) {
                    self.desk_editing = Some(Editing {
                        drag: Some((grip, edit.panel, x, y)),
                        ..editing
                    });
                }
                self.dirty = true;
            }
            PointerEventKind::Motion { .. } => {
                if let Some((grip, start, from_x, from_y)) = editing.drag {
                    let panel = desk::dragged(start, grip, x - from_x, y - from_y, self.size);
                    if panel != edit.panel {
                        view::set_desk_edit(Some(DeskEdit { panel, ..edit }));
                        self.dirty = true;
                    }
                }
            }
            PointerEventKind::Release { .. } => {
                self.desk_editing = Some(Editing {
                    drag: None,
                    ..editing
                });
                if let Some(pressed) = edit.pressed {
                    view::set_desk_edit(Some(DeskEdit {
                        pressed: None,
                        ..edit
                    }));
                    self.dirty = true;
                    // A button acts where the press is let go over it, so a
                    // press slid off is a change of mind.
                    if view::desk_edit_button_at(edit.panel, x, y) == Some(pressed) {
                        self.finish_desk_edit(pressed == DeskEditButton::Done);
                    }
                }
            }
            _ => {}
        }
        self.update_desk_edit_cursor(x, y);
        Some(After::Next)
    }

    /// Show what a press at (`x`, `y`) would do, or, mid-drag, what the drag
    /// is doing wherever the pointer has got to.
    fn update_desk_edit_cursor(&mut self, x: f32, y: f32) {
        let (Some(editing), Some(edit)) = (self.desk_editing, view::desk_edit()) else {
            return;
        };
        let shape = match editing.drag {
            Some((grip, ..)) => cursor_for(Some(grip), false, true),
            None => cursor_for(
                desk::grip_at(edit.panel, x, y),
                view::desk_edit_button_at(edit.panel, x, y).is_some(),
                false,
            ),
        };
        if shape != editing.cursor {
            AppContext::set_cursor_shape(shape);
            self.desk_editing = Some(Editing {
                cursor: shape,
                ..editing
            });
        }
    }

    /// Follow a change to `files.toml`: re-read the `[desk]` section and take
    /// up whatever moved.
    ///
    /// The folder is shown afresh, the order and the icon size are the
    /// grid's, and the panel's place is the view's. An edit under way keeps
    /// its own panel until it is finished.
    pub(super) fn reload_desk_config(&mut self) {
        let Some(current) = self.desk_config.clone() else {
            return;
        };
        let next = desk::DeskConfig::load();
        if next == current {
            return;
        }
        if next.folder != current.folder {
            self.go_to(&next.folder);
        }
        if next.sort != current.sort {
            self.sort = next.sort;
            self.ascending = next.sort != SortKey::Modified;
            self.dirty = true;
        }
        if next.icon_size != current.icon_size {
            view::set_grid_icon(next.icon_size);
        }
        // The pile's cell moves with the grid; a fan left open would hang
        // over the wrong one.
        if next.overflow != current.overflow || next.icon_size != current.icon_size {
            self.close_desk_fan();
        }
        view::set_desk_layout(view::DeskLayout::from_config(&next));
        self.desk_config = Some(next);
        self.dirty = true;
    }
}
