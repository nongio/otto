//! The desk's overflow: what `overflow = "stack"` does with icons the panel
//! has no cell for, and the overflow panel the overflow tile opens.
//!
//! The overflow tile is the grid's last cell, holding that cell's own item
//! and every item after it (see [`crate::desk::overflow_tile`]). A click on
//! it, or Return or Space with the keyboard on it, opens the overflow panel:
//! the tile's items as a small grid of their own near it, on a surface of
//! its own above the windows (see `overflow_surface`). There they are
//! selected, opened, renamed and dragged like any other icon, since
//! [`Browser::entry_at`] and [`Browser::entry_rect`] answer for them in the
//! desk's own coordinates. Escape, a click away, or the keyboard leaving
//! the desk closes it.
//!
//! Everything here is in the desk's coordinates. The panel's surface is the
//! same size as the desk's and sits at the same place, so a point on it
//! means the same thing on both.

// Rust guideline compliant 2026-02-21

use std::time::{Duration, Instant};

use smithay_client_toolkit::seat::pointer::PointerEvent;

use super::listing_pointer::{After, PointerAt};
use super::*;
use crate::desk::{Overflow, OverflowTile};
use skia_safe::Point;

/// How long the overflow panel takes to grow out of its tile.
///
/// Long enough to see where the panel came from, short enough that a
/// click on the tile still feels like it opened something at once.
const OPEN_DURATION: Duration = Duration::from_millis(240);

/// How long it takes to go back into the tile. Quicker than the way in:
/// there is nothing to settle into on the way out.
const CLOSE_DURATION: Duration = Duration::from_millis(190);

/// The overflow panel while it is up: its scroll and when it opened or
/// started closing.
pub(super) struct OverflowSession {
    /// The panel's scroll: momentum, the elastic ends and the overlay bar,
    /// as every other scrolling list in Files has them.
    pub(super) scroll: ScrollView,
    /// When it opened, or when it started closing.
    since: Instant,
    /// What it was showing when it started closing, and the icon rect it is
    /// going back into. `None` while it is open, when both are worked out
    /// afresh from the desk's geometry.
    frozen: Option<(view::DeskOverflow, Rect)>,
}

/// The overflow panel as its surface shows it on this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct OverflowShown {
    /// The tile and the panel at rest, with the panel's scroll.
    pub(super) overflow: view::DeskOverflow,
    /// The tile's icon, which the panel grows out of and goes back into.
    pub(super) anchor: Rect,
    /// How far through its entrance or exit the panel is, 0 to 1.
    pub(super) t: f32,
    /// Whether it is on its way back into the tile.
    pub(super) closing: bool,
}

impl OverflowShown {
    /// The panel at rest, in the desk's coordinates.
    pub(super) fn resting(&self) -> Rect {
        self.overflow
            .panel
            .map_or_else(Rect::new_empty, |(panel, _)| panel.rect)
    }

    /// Where the panel is on this frame: part way out of the tile, at rest,
    /// or part way back into it. Always the resting rect scaled evenly, so
    /// what is drawn in it is the panel at rest, scaled.
    pub(super) fn rect(&self) -> Rect {
        let resting = self.resting();
        let to = |rect: Rect| {
            otto_peek::opening::Rect::new(rect.left, rect.top, rect.width(), rect.height())
        };
        let rect = if self.closing {
            otto_peek::opening::sample_out(to(self.anchor), to(resting), self.t)
        } else {
            otto_peek::opening::sample(to(self.anchor), to(resting), self.t)
        };
        Rect::from_xywh(rect.x, rect.y, rect.width.max(1.0), rect.height.max(1.0))
    }

    /// How opaque the panel is on this frame: it fades in over the first part
    /// of its entrance and out over its exit.
    pub(super) fn opacity(&self) -> f32 {
        if self.closing {
            (1.0 - self.t).clamp(0.0, 1.0)
        } else {
            (self.t * 2.5).clamp(0.0, 1.0)
        }
    }

    /// Whether there are frames still to run.
    pub(super) fn animating(&self) -> bool {
        self.t < 1.0
    }
}

impl OverflowSession {
    fn new() -> Self {
        Self {
            scroll: ScrollView::new(Rect::new_empty()),
            since: Instant::now(),
            frozen: None,
        }
    }

    /// How far through its entrance or exit the panel is, 0 to 1.
    fn progress(&self) -> f32 {
        let duration = if self.frozen.is_some() {
            CLOSE_DURATION
        } else {
            OPEN_DURATION
        };
        (self.since.elapsed().as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
    }
}

impl Browser {
    /// The grid the desk lays its icons in.
    fn desk_grid_area(&self) -> Rect {
        view::content_viewport(self.size.0, self.content_h(), ViewMode::Grid)
    }

    /// The desk's overflow tile and, while it is open, its panel: `None`
    /// unless this is a desk that stacks and has more icons than cells.
    pub(super) fn desk_overflow(&self) -> Option<view::DeskOverflow> {
        if !self.desk || self.mode != ViewMode::Grid {
            return None;
        }
        if self.desk_config.as_ref()?.overflow != Overflow::Stack {
            return None;
        }
        let depth = self.columns.len() - 1;
        view::DeskOverflow::new(
            self.desk_grid_area(),
            Rect::from_wh(self.size.0, self.size.1),
            self.visible_len(depth),
            self.overflow_panel
                .as_ref()
                .map(|session| session.scroll.offset()),
        )
    }

    /// The entries a desk with an overflow tile shows, from the first: the
    /// cells up to the tile and the few icons drawn on it, or, with the
    /// panel open, up to the last item showing in the panel.
    pub(super) fn desk_overflow_shown(&self) -> Option<std::ops::Range<usize>> {
        let overflow = self.desk_overflow()?;
        let tile = overflow.tile;
        let end = match overflow.panel {
            Some((panel, scroll)) => tile.first + panel.visible(tile.count, scroll).end,
            None => tile.first + tile.count.min(view::OVERFLOW_TILE_LAYERS),
        };
        Some(0..end)
    }

    /// Where entry `index` of the desk is when the tile holds it.
    pub(super) fn desk_overflow_entry_rect(&self, index: usize) -> Option<Rect> {
        self.desk_overflow()?
            .entry_rect(self.desk_grid_area(), index)
    }

    /// Whether entry `index` is in the open overflow panel, and so drawn on
    /// the panel's surface rather than the desk's.
    pub(super) fn in_overflow_panel(&self, index: usize) -> bool {
        self.desk_overflow()
            .is_some_and(|overflow| overflow.panel.is_some() && overflow.tile.contains(index))
    }

    /// The entry under (`x`, `y`) on a desk with an overflow tile: a cell
    /// before the tile, or an item in the open panel. The tile's own cell is
    /// no entry, so a press there opens the panel and a drop there lands on
    /// the desk. `None` when there is no tile, for the ordinary hit test to
    /// answer.
    pub(super) fn desk_overflow_entry_at(&self, x: f32, y: f32) -> Option<Option<usize>> {
        let overflow = self.desk_overflow()?;
        if let Some((panel, scroll)) = overflow.panel {
            if panel.rect.contains(Point::new(x, y)) {
                return Some(
                    panel
                        .index_at(x, y, overflow.tile.count, scroll)
                        .map(|k| overflow.tile.first + k),
                );
            }
        }
        let area = self.desk_grid_area();
        Some(view::grid_cell_at_in(
            area,
            view::GridSections::FLAT,
            x,
            y,
            overflow.tile.first,
            0.0,
        ))
    }

    /// Open the overflow panel, scrolled to the top.
    pub(super) fn open_overflow_panel(&mut self) {
        if self.overflow_panel.is_some() || self.desk_overflow().is_none() {
            return;
        }
        // A panel still on its way back in is replaced: the new one grows
        // out of the tile from the start.
        self.overflow_panel_closing = None;
        self.overflow_panel = Some(OverflowSession::new());
        self.settle_overflow_panel();
        self.dirty = true;
    }

    /// Close the overflow panel, which goes back into its tile. Returns
    /// whether it was open.
    ///
    /// A rename under way in the panel is committed first, the way a click
    /// away from the field commits it anywhere else.
    pub(super) fn close_overflow_panel(&mut self) -> bool {
        let Some(overflow) = self.desk_overflow().filter(|o| o.panel.is_some()) else {
            // A session whose tile has gone has nothing to animate back to.
            return self.overflow_panel.take().is_some();
        };
        if self
            .rename
            .as_ref()
            .is_some_and(|session| overflow.tile.contains(session.index))
        {
            self.commit_rename();
        }
        let Some(mut session) = self.overflow_panel.take() else {
            return false;
        };
        let anchor = view::entry_icon_rect(overflow.cell(self.desk_grid_area()), ViewMode::Grid);
        // Back from wherever it has got to: a panel closed part way in goes
        // home from there rather than jumping to rest first.
        let elapsed = CLOSE_DURATION.mul_f32(1.0 - session.progress());
        session.since = Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(Instant::now);
        session.scroll.stop();
        session.frozen = Some((overflow, anchor));
        self.overflow_panel_closing = Some(session);
        self.dirty = true;
        true
    }

    /// Keep the open panel's scroll fitted to it, and forget a panel whose
    /// tile has gone: the folder emptied out, the panel grew, or the config
    /// stopped stacking. It must not come back open when a tile forms again.
    pub(super) fn settle_overflow_panel(&mut self) {
        if self.overflow_panel.is_none() {
            return;
        }
        let area = self.desk_grid_area();
        let depth = self.columns.len() - 1;
        let fitted = (self.desk
            && self.mode == ViewMode::Grid
            && self
                .desk_config
                .as_ref()
                .is_some_and(|config| config.overflow == Overflow::Stack))
        .then(|| {
            view::DeskOverflow::new(
                area,
                Rect::from_wh(self.size.0, self.size.1),
                self.visible_len(depth),
                Some(0.0),
            )
        })
        .flatten()
        .and_then(|overflow| overflow.panel);
        match fitted {
            Some((panel, _)) => {
                if let Some(session) = self.overflow_panel.as_mut() {
                    session.scroll.state.set_viewport(panel.rect);
                    session.scroll.set_content_length(panel.content_h);
                }
            }
            None => self.overflow_panel = None,
        }
    }

    /// The panel as its surface shows it on this frame, open or closing.
    /// `None` when there is nothing on screen.
    pub(super) fn overflow_shown(&self) -> Option<OverflowShown> {
        if let Some(session) = self.overflow_panel.as_ref() {
            let overflow = self.desk_overflow()?;
            overflow.panel?;
            let anchor =
                view::entry_icon_rect(overflow.cell(self.desk_grid_area()), ViewMode::Grid);
            return Some(OverflowShown {
                overflow,
                anchor,
                t: session.progress(),
                closing: false,
            });
        }
        let session = self.overflow_panel_closing.as_ref()?;
        let (overflow, anchor) = session.frozen?;
        Some(OverflowShown {
            overflow,
            anchor,
            t: session.progress(),
            closing: true,
        })
    }

    /// The scroll of the panel on screen, open or closing.
    pub(super) fn overflow_scroll(&self) -> Option<&otto_kit::components::scroll::ScrollState> {
        self.overflow_panel
            .as_ref()
            .or(self.overflow_panel_closing.as_ref())
            .map(|session| &session.scroll.state)
    }

    /// Advance the panel's scroll and retire a finished exit. Returns whether
    /// anything moved.
    pub(super) fn tick_overflow_panel(&mut self) -> bool {
        let mut moved = false;
        if let Some(session) = self.overflow_panel.as_mut() {
            if session.scroll.is_animating() {
                moved |= session.scroll.tick();
            }
        }
        if self
            .overflow_panel_closing
            .as_ref()
            .is_some_and(|session| session.progress() >= 1.0)
        {
            self.overflow_panel_closing = None;
            moved = true;
        }
        moved
    }

    /// Whether the panel has frames still to run: growing, shrinking, or
    /// its scroll gliding.
    pub(super) fn overflow_animating(&self) -> bool {
        self.overflow_shown().is_some_and(|shown| shown.animating())
            || self
                .overflow_panel
                .as_ref()
                .is_some_and(|session| session.scroll.is_animating())
    }

    /// Stop the panel's scroll where it is: a hand laid on the touchpad.
    pub(super) fn stop_overflow_scroll(&mut self) {
        if let Some(session) = self.overflow_panel.as_mut() {
            session.scroll.stop();
        }
    }

    /// The keyboard's range on a desk with an overflow tile: the panel's
    /// items while it is open, the cells up to and including the tile while
    /// it is not.
    pub(super) fn desk_cursor_range(&self) -> Option<std::ops::RangeInclusive<usize>> {
        let overflow = self.desk_overflow()?;
        let range = overflow.tile.range();
        Some(match overflow.panel {
            Some(_) => range.start..=range.end - 1,
            None => 0..=overflow.tile.first,
        })
    }

    /// How far Up and Down move the keyboard in the open panel: one of its
    /// rows.
    pub(super) fn overflow_panel_row_step(&self) -> Option<i32> {
        let (panel, _) = self.desk_overflow()?.panel?;
        Some(panel.columns as i32)
    }

    /// Scroll the open panel so the keyboard's item shows. Returns whether
    /// the desk has an overflow tile, which is also when the grid itself
    /// never scrolls.
    pub(super) fn reveal_in_desk_overflow(&mut self, index: usize) -> bool {
        let Some(overflow) = self.desk_overflow() else {
            return false;
        };
        let tile = overflow.tile;
        if let Some((panel, scroll)) = overflow.panel.filter(|_| tile.contains(index)) {
            let next = panel.reveal(index - tile.first, scroll);
            if next != scroll {
                if let Some(session) = self.overflow_panel.as_mut() {
                    session.scroll.scroll_to(next);
                }
            }
        }
        true
    }

    /// Whether the keyboard is on the closed tile, where Return and Space
    /// open the panel rather than acting on the tile's top item.
    pub(super) fn cursor_on_closed_tile(&self) -> bool {
        let Some(overflow) = self.desk_overflow().filter(|o| o.panel.is_none()) else {
            return false;
        };
        let depth = self.columns.len() - 1;
        self.columns[depth].cursor == Some(overflow.tile.first)
    }

    /// The tile, for a screen reader's pick of its node: `Some` when entry
    /// `index` is the closed tile.
    pub(super) fn closed_tile_at(&self, index: usize) -> Option<OverflowTile> {
        self.desk_overflow()
            .filter(|overflow| overflow.panel.is_none() && overflow.tile.first == index)
            .map(|overflow| overflow.tile)
    }

    /// The pointer on a desk with an overflow tile, before the listing sees
    /// it. Events over the panel arrive here already moved into the desk's
    /// coordinates, from the panel's own surface.
    ///
    /// A press on the tile opens the panel. While the panel is open, a press
    /// outside it closes it and goes no further, and a press on its ground
    /// between items clears the selection rather than starting a rubber band
    /// over the grid beneath; the wheel over it scrolls it. Everything else
    /// is the listing's, which finds the panel's items through
    /// [`Browser::entry_at`].
    pub(super) fn desk_overflow_pointer(
        &mut self,
        event: &PointerEvent,
        at: PointerAt,
    ) -> Option<After> {
        let overflow = self.desk_overflow()?;
        let (x, y) = (at.x, at.y);
        let on_tile = overflow
            .cell(self.desk_grid_area())
            .contains(Point::new(x, y));
        match event.kind {
            PointerEventKind::Press { button, .. } if button != BTN_RIGHT => {
                if let Some((panel, scroll)) = overflow.panel {
                    if panel.rect.contains(Point::new(x, y)) {
                        if panel.index_at(x, y, overflow.tile.count, scroll).is_some() {
                            return None;
                        }
                        let depth = self.columns.len() - 1;
                        self.clear_pane_selection(depth);
                        return Some(After::Stop);
                    }
                    self.close_overflow_panel();
                    return Some(After::Stop);
                }
                if on_tile {
                    self.open_overflow_panel();
                    return Some(After::Stop);
                }
                None
            }
            PointerEventKind::Press { .. } if overflow.panel.is_some() => {
                // A right click away from the panel closes it too; one on it
                // is the item's menu.
                let (panel, _) = overflow.panel?;
                if panel.rect.contains(Point::new(x, y)) {
                    return None;
                }
                self.close_overflow_panel();
                Some(After::Stop)
            }
            PointerEventKind::Axis { vertical, .. } => {
                let (panel, _) = overflow.panel?;
                if !panel.rect.contains(Point::new(x, y)) {
                    return None;
                }
                let session = self.overflow_panel.as_mut()?;
                let scroll = &mut session.scroll;
                // A touchpad's stream flings and stretches past the ends; a
                // notched wheel steps.
                if vertical.stop {
                    scroll.on_wheel_end();
                } else if vertical.discrete != 0 {
                    scroll.on_wheel_discrete(vertical.absolute as f32);
                } else {
                    scroll.on_wheel(vertical.absolute as f32);
                }
                Some(After::Next)
            }
            _ => None,
        }
    }
}
