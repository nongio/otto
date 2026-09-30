//! The desk's pile: what `overflow = "stack"` does with icons the panel has
//! no cell for, and the fan the pile opens into.
//!
//! The pile is the grid's last cell, holding that cell's own item and every
//! item after it (see [`crate::desk::pile`]). A click on it, or Return or
//! Space with the keyboard on it, opens the fan: the pile's items as a small
//! grid of their own near it, where they are selected, opened, renamed and
//! dragged like any other icon, since [`Browser::entry_at`] and
//! [`Browser::entry_rect`] answer for them there. Escape, a click away, or
//! the desk losing the keyboard closes it.

// Rust guideline compliant 2026-02-21

use smithay_client_toolkit::seat::pointer::PointerEvent;

use super::listing_pointer::{After, PointerAt};
use super::*;
use crate::desk::{Overflow, Pile};
use skia_safe::Point;

impl Browser {
    /// The grid the desk lays its icons in.
    fn desk_grid_area(&self) -> Rect {
        view::content_viewport(self.size.0, self.content_h(), ViewMode::Grid)
    }

    /// The desk's pile and, while it is open, its fan: `None` unless this is
    /// a desk that stacks and has more icons than cells.
    pub(super) fn desk_pile(&self) -> Option<view::DeskPile> {
        if !self.desk || self.mode != ViewMode::Grid {
            return None;
        }
        if self.desk_config.as_ref()?.overflow != Overflow::Stack {
            return None;
        }
        let depth = self.columns.len() - 1;
        view::DeskPile::new(
            self.desk_grid_area(),
            self.visible_len(depth),
            self.desk_fan,
        )
    }

    /// The entries a desk with a pile shows, from the first: the cells up to
    /// the pile and the few icons drawn on it, or, with the fan open, up to
    /// the last item showing in the fan.
    pub(super) fn desk_pile_shown(&self) -> Option<std::ops::Range<usize>> {
        let pile = self.desk_pile()?;
        let end = match pile.fan {
            Some((fan, scroll)) => pile.pile.first + fan.visible(pile.pile.count, scroll).end,
            None => pile.pile.first + pile.pile.count.min(view::PILE_LAYERS),
        };
        Some(0..end)
    }

    /// Where entry `index` of the desk is when the pile holds it.
    pub(super) fn desk_pile_entry_rect(&self, index: usize) -> Option<Rect> {
        self.desk_pile()?.entry_rect(self.desk_grid_area(), index)
    }

    /// The entry under (`x`, `y`) on a desk with a pile: a cell before the
    /// pile, or an item in the open fan. The pile's own cell is no entry,
    /// so a press there opens the fan and a drop there lands on the desk.
    /// `None` when there is no pile, for the ordinary hit test to answer.
    pub(super) fn desk_pile_entry_at(&self, x: f32, y: f32) -> Option<Option<usize>> {
        let pile = self.desk_pile()?;
        if let Some((fan, scroll)) = pile.fan {
            if fan.rect.contains(Point::new(x, y)) {
                return Some(
                    fan.index_at(x, y, pile.pile.count, scroll)
                        .map(|k| pile.pile.first + k),
                );
            }
        }
        let area = self.desk_grid_area();
        Some(view::grid_cell_at_in(
            area,
            view::GridSections::FLAT,
            x,
            y,
            pile.pile.first,
            0.0,
        ))
    }

    /// Open the fan, scrolled to the top.
    pub(super) fn open_desk_fan(&mut self) {
        if self.desk_pile().is_some() {
            self.desk_fan = Some(0.0);
            self.dirty = true;
        }
    }

    /// Close the fan. Returns whether it was open.
    pub(super) fn close_desk_fan(&mut self) -> bool {
        let open = self.desk_fan.take().is_some();
        if open {
            self.dirty = true;
        }
        open
    }

    /// Forget a fan whose pile has gone: the folder emptied out, the panel
    /// grew, or the config stopped stacking. It must not come back open when
    /// a pile forms again.
    pub(super) fn settle_desk_fan(&mut self) {
        if self.desk_fan.is_some() && self.desk_pile().is_none() {
            self.desk_fan = None;
        }
    }

    /// The keyboard's range on a desk with a pile: the fan's items while it
    /// is open, the cells up to and including the pile while it is not.
    pub(super) fn desk_cursor_range(&self) -> Option<std::ops::RangeInclusive<usize>> {
        let pile = self.desk_pile()?;
        let range = pile.pile.range();
        Some(match pile.fan {
            Some(_) => range.start..=range.end - 1,
            None => 0..=pile.pile.first,
        })
    }

    /// How far Up and Down move the keyboard in the open fan: one of its
    /// rows.
    pub(super) fn desk_fan_row_step(&self) -> Option<i32> {
        let (fan, _) = self.desk_pile()?.fan?;
        Some(fan.columns as i32)
    }

    /// Scroll the open fan so the keyboard's item shows. Returns whether the
    /// desk has a pile, which is also when the grid itself never scrolls.
    pub(super) fn reveal_in_desk_pile(&mut self, index: usize) -> bool {
        let Some(pile) = self.desk_pile() else {
            return false;
        };
        if let Some((fan, scroll)) = pile.fan.filter(|_| pile.pile.contains(index)) {
            let next = fan.reveal(index - pile.pile.first, scroll);
            if next != scroll {
                self.desk_fan = Some(next);
                self.dirty = true;
            }
        }
        true
    }

    /// Whether the keyboard is on the closed pile, where Return and Space
    /// open the fan rather than acting on the pile's top item.
    pub(super) fn cursor_on_closed_pile(&self) -> bool {
        let Some(pile) = self.desk_pile().filter(|pile| pile.fan.is_none()) else {
            return false;
        };
        let depth = self.columns.len() - 1;
        self.columns[depth].cursor == Some(pile.pile.first)
    }

    /// The pile, for a screen reader's pick of its node: `Some` when entry
    /// `index` is the closed pile.
    pub(super) fn closed_pile_at(&self, index: usize) -> Option<Pile> {
        self.desk_pile()
            .filter(|pile| pile.fan.is_none() && pile.pile.first == index)
            .map(|pile| pile.pile)
    }

    /// The pointer on a desk with a pile, before the listing sees it.
    ///
    /// A press on the pile opens or closes the fan. While the fan is open, a
    /// press outside it closes it and goes no further, and a press on its
    /// ground between items clears the selection rather than starting a
    /// rubber band over the grid beneath; the wheel over it scrolls it.
    /// Everything else is the listing's, which finds the fan's items through
    /// [`Browser::entry_at`].
    pub(super) fn desk_pile_pointer(
        &mut self,
        event: &PointerEvent,
        at: PointerAt,
    ) -> Option<After> {
        let pile = self.desk_pile()?;
        let (x, y) = (at.x, at.y);
        let on_pile = pile.cell(self.desk_grid_area()).contains(Point::new(x, y));
        match event.kind {
            PointerEventKind::Press { button, .. } if button != BTN_RIGHT => {
                if let Some((fan, scroll)) = pile.fan {
                    if fan.rect.contains(Point::new(x, y)) {
                        if fan.index_at(x, y, pile.pile.count, scroll).is_some() {
                            return None;
                        }
                        let depth = self.columns.len() - 1;
                        self.clear_pane_selection(depth);
                        return Some(After::Stop);
                    }
                    self.close_desk_fan();
                    return Some(After::Stop);
                }
                if on_pile {
                    self.open_desk_fan();
                    return Some(After::Stop);
                }
                None
            }
            PointerEventKind::Press { .. } if pile.fan.is_some() => {
                // A right click away from the fan closes it too; one on it
                // is the item's menu.
                let (fan, _) = pile.fan?;
                if fan.rect.contains(Point::new(x, y)) {
                    return None;
                }
                self.close_desk_fan();
                Some(After::Stop)
            }
            PointerEventKind::Axis { vertical, .. } => {
                let (fan, scroll) = pile.fan?;
                if !fan.rect.contains(Point::new(x, y)) {
                    return None;
                }
                let next = (scroll + vertical.absolute as f32).clamp(0.0, fan.max_scroll());
                if next != scroll {
                    self.desk_fan = Some(next);
                    self.dirty = true;
                }
                Some(After::Next)
            }
            _ => None,
        }
    }
}
