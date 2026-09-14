//! Closed-form geometry for scrolling content: rows and grids.
//!
//! Everything here is in *content* coordinates — `0` is the start of the
//! content, before any scroll, with no window, pane or sidebar offset mixed
//! in. A scroll pane maps content to the screen; these layouts only say where
//! things are in the content. The same layout answers the paint walk (what
//! intersects the band being painted), the hit test (what is under a point),
//! the keyboard (where the cursor's row is, to reveal it), accessibility and
//! any prefetching (what is visible) — so what is drawn and what is clickable
//! cannot drift apart.
//!
//! Fixed pitch is what makes them closed-form: finding the rows in a band of a
//! ten-thousand-entry listing costs a division, not a walk over the entries.

use std::ops::Range;

use skia_safe::{Point, Rect, Size};

/// Rows of a uniform pitch along a vertical pane, with space before the first
/// and after the last.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowLayout {
    /// Height of one row, in points.
    pub pitch: f32,
    pub count: usize,
    /// Space before the first row.
    pub leading: f32,
    /// Space after the last row.
    pub trailing: f32,
}

impl RowLayout {
    pub fn new(pitch: f32, count: usize) -> Self {
        Self {
            pitch,
            count,
            leading: 0.0,
            trailing: 0.0,
        }
    }

    pub fn with_insets(mut self, leading: f32, trailing: f32) -> Self {
        self.leading = leading;
        self.trailing = trailing;
        self
    }

    /// Total length of the content: what a scroll view's content length is.
    pub fn length(&self) -> f32 {
        self.leading + self.count as f32 * self.pitch + self.trailing
    }

    /// Row `index`, spanning `width` across.
    pub fn rect(&self, index: usize, width: f32) -> Rect {
        Rect::from_xywh(
            0.0,
            self.leading + index as f32 * self.pitch,
            width,
            self.pitch,
        )
    }

    /// The row `y` falls on, if any. The insets and anything past the last row
    /// are misses.
    pub fn index_at(&self, y: f32) -> Option<usize> {
        let local = y - self.leading;
        if local < 0.0 || self.pitch <= 0.0 {
            return None;
        }
        let index = (local / self.pitch) as usize;
        (index < self.count).then_some(index)
    }

    /// The rows that intersect `lo..hi` along the axis — a painted band, the
    /// visible part of the pane.
    ///
    /// Inclusive at both ends, so a row resting exactly on an edge survives
    /// with the hairline it contributes there. An empty span yields nothing.
    pub fn range(&self, lo: f32, hi: f32) -> Range<usize> {
        if self.count == 0 || hi <= lo || self.pitch <= 0.0 {
            return 0..0;
        }
        let first = (((lo - self.leading) / self.pitch).floor().max(0.0) as usize).min(self.count);
        // Clamped before the cast: a span far past the end of a short list
        // would otherwise turn into an index no `usize` can hold.
        let last = ((hi - self.leading) / self.pitch)
            .floor()
            .min(self.count as f32);
        if last < 0.0 {
            return 0..0;
        }
        let end = (last as usize + 1).min(self.count);
        first..end.max(first)
    }

    /// [`Self::range`] over a rect's vertical extent.
    pub fn visible(&self, rect: Rect) -> Range<usize> {
        self.range(rect.top, rect.bottom)
    }
}

/// A run of cells in a [`GridLayout`], optionally under a heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSection {
    /// Index of the section's first cell.
    pub first: usize,
    pub count: usize,
    /// Whether the section starts with a heading band.
    pub headed: bool,
}

/// Uniform cells laid out in rows that fill the width, in sections that may
/// each start with a heading.
#[derive(Debug, Clone, PartialEq)]
pub struct GridLayout {
    pub cell: Size,
    /// Padding around the whole grid.
    pub pad: f32,
    /// Height of a section heading.
    pub header: f32,
    pub count: usize,
    /// Empty means one unheaded section holding every cell.
    pub sections: Vec<GridSection>,
}

impl GridLayout {
    pub fn new(cell: Size, count: usize) -> Self {
        Self {
            cell,
            pad: 0.0,
            header: 0.0,
            count,
            sections: Vec::new(),
        }
    }

    pub fn with_pad(mut self, pad: f32) -> Self {
        self.pad = pad;
        self
    }

    /// Group the cells into sections, each with a heading `header` tall when
    /// it asks for one.
    pub fn with_sections(mut self, header: f32, sections: Vec<GridSection>) -> Self {
        self.header = header;
        self.sections = sections;
        self
    }

    /// How many cells fit across `width`. Never zero, so a very narrow pane
    /// degrades to one column rather than dividing by it.
    pub fn columns(&self, width: f32) -> usize {
        (((width - self.pad * 2.0) / self.cell.width).floor() as usize).max(1)
    }

    fn runs(&self) -> Vec<GridSection> {
        if self.sections.is_empty() {
            vec![GridSection {
                first: 0,
                count: self.count,
                headed: false,
            }]
        } else {
            self.sections.clone()
        }
    }

    /// Each section with the content y of its top — its heading when it has
    /// one, its first row of cells otherwise.
    fn walk(&self, cols: usize) -> Vec<(GridSection, f32)> {
        let mut y = self.pad;
        self.runs()
            .into_iter()
            .map(|section| {
                let top = y;
                y += self.section_length(&section, cols);
                (section, top)
            })
            .collect()
    }

    fn section_length(&self, section: &GridSection, cols: usize) -> f32 {
        section.headed as u8 as f32 * self.header
            + section.count.div_ceil(cols) as f32 * self.cell.height
    }

    /// Total length of the content at `width`.
    pub fn length(&self, width: f32) -> f32 {
        let cols = self.columns(width);
        let cells: f32 = self
            .runs()
            .iter()
            .map(|section| self.section_length(section, cols))
            .sum();
        cells + self.pad * 2.0
    }

    /// The rect of cell `index` at `width`. Empty for an index past every
    /// section: asked for while a listing is replaced underneath, an empty
    /// rect is a less surprising answer than a panic.
    pub fn cell_rect(&self, index: usize, width: f32) -> Rect {
        let cols = self.columns(width);
        for (section, top) in self.walk(cols) {
            if index < section.first || index >= section.first + section.count {
                continue;
            }
            let local = index - section.first;
            let y = top
                + section.headed as u8 as f32 * self.header
                + (local / cols) as f32 * self.cell.height;
            let x = self.pad + (local % cols) as f32 * self.cell.width;
            return Rect::from_xywh(x, y, self.cell.width, self.cell.height);
        }
        Rect::new_empty()
    }

    /// The cell under `point`, if any. A heading, the padding, and the space
    /// after a section's last cell are all misses.
    pub fn index_at(&self, point: Point, width: f32) -> Option<usize> {
        let cols = self.columns(width);
        let local_x = point.x - self.pad;
        if local_x < 0.0 {
            return None;
        }
        let col = (local_x / self.cell.width) as usize;
        if col >= cols {
            return None;
        }
        for (section, top) in self.walk(cols) {
            let cells_top = top + section.headed as u8 as f32 * self.header;
            let cells_bottom = cells_top + section.count.div_ceil(cols) as f32 * self.cell.height;
            if point.y < cells_top {
                return None;
            }
            if point.y >= cells_bottom {
                continue;
            }
            let row = ((point.y - cells_top) / self.cell.height) as usize;
            let index = section.first + row * cols + col;
            return (index < section.first + section.count && index < self.count).then_some(index);
        }
        None
    }

    /// The cells whose rows intersect `lo..hi`, widened to whole rows. One
    /// contiguous range: sections partition the cells in order, so everything
    /// between the first visible cell and the last is visible too.
    pub fn range(&self, lo: f32, hi: f32, width: f32) -> Range<usize> {
        if self.count == 0 || hi <= lo {
            return 0..0;
        }
        let cols = self.columns(width);
        let mut first: Option<usize> = None;
        let mut end = 0;
        for (section, top) in self.walk(cols) {
            let cells_top = top + section.headed as u8 as f32 * self.header;
            let rows = section.count.div_ceil(cols);
            if cells_top > hi {
                break;
            }
            if cells_top + rows as f32 * self.cell.height < lo {
                continue;
            }
            let first_row =
                (((lo - cells_top) / self.cell.height).floor().max(0.0) as usize).min(rows);
            let last_row =
                (((hi - cells_top) / self.cell.height).floor().max(0.0) as usize + 1).min(rows);
            let start = (section.first + first_row * cols).min(self.count);
            first = Some(first.map_or(start, |f| f.min(start)));
            end = end.max((section.first + last_row * cols).min(section.first + section.count));
        }
        let first = first.unwrap_or(0);
        first..end.min(self.count).max(first)
    }

    /// The cells `rect` touches at all — a marquee's hit test.
    pub fn cells_in(&self, rect: Rect, width: f32) -> Vec<usize> {
        if self.count == 0 || (rect.width() <= 0.0 && rect.height() <= 0.0) {
            return Vec::new();
        }
        self.range(rect.top, rect.bottom.max(rect.top + f32::EPSILON), width)
            .filter(|&index| {
                let cell = self.cell_rect(index, width);
                !cell.is_empty() && cell.intersects(rect)
            })
            .collect()
    }

    /// Every heading, as the index of its section and its rect at `width`.
    pub fn headers(&self, width: f32) -> Vec<(usize, Rect)> {
        let cols = self.columns(width);
        self.walk(cols)
            .into_iter()
            .enumerate()
            .filter(|(_, (section, _))| section.headed)
            .map(|(i, (_, top))| {
                (
                    i,
                    Rect::from_xywh(self.pad, top, width - self.pad * 2.0, self.header),
                )
            })
            .collect()
    }

    /// The heading to pin at the top of a pane scrolled to `offset`: the last
    /// heading at or above the pane's top edge, and where to draw it relative
    /// to that edge. It sits at `0` until the next section's heading arrives
    /// and pushes it up and out.
    pub fn pinned_header(&self, offset: f32, width: f32) -> Option<(usize, f32)> {
        let headers = self.headers(width);
        let current = headers.iter().rposition(|(_, rect)| rect.top <= offset)?;
        let (section, _) = headers[current];
        let push = headers
            .get(current + 1)
            .map(|(_, next)| (next.top - offset - self.header).min(0.0))
            .unwrap_or(0.0);
        Some((section, push))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_placed_after_the_leading_inset() {
        let rows = RowLayout::new(24.0, 10).with_insets(6.0, 4.0);
        assert_eq!(rows.rect(0, 200.0), Rect::from_xywh(0.0, 6.0, 200.0, 24.0));
        assert_eq!(rows.rect(3, 200.0).top, 6.0 + 72.0);
        assert_eq!(rows.length(), 6.0 + 240.0 + 4.0);
    }

    #[test]
    fn a_point_in_an_inset_or_past_the_end_is_no_row() {
        let rows = RowLayout::new(24.0, 10).with_insets(6.0, 4.0);
        assert_eq!(rows.index_at(3.0), None);
        assert_eq!(rows.index_at(6.0), Some(0));
        assert_eq!(rows.index_at(6.0 + 24.0 * 9.5), Some(9));
        assert_eq!(rows.index_at(6.0 + 24.0 * 10.0 + 1.0), None);
    }

    #[test]
    fn the_range_is_inclusive_at_both_edges() {
        let rows = RowLayout::new(10.0, 100);
        // A span ending exactly on row 5's top still includes row 5.
        assert_eq!(rows.range(20.0, 50.0), 2..6);
        assert_eq!(rows.range(25.0, 49.0), 2..5);
    }

    #[test]
    fn a_range_past_either_end_is_clamped_or_empty() {
        let rows = RowLayout::new(10.0, 5);
        assert_eq!(rows.range(-100.0, -50.0), 0..0);
        assert_eq!(rows.range(30.0, 1.0e9), 3..5);
        assert_eq!(rows.range(1.0e9, 2.0e9), 5..5);
        assert_eq!(RowLayout::new(10.0, 0).range(0.0, 100.0), 0..0);
        assert_eq!(rows.range(20.0, 20.0), 0..0);
    }

    fn grid() -> GridLayout {
        GridLayout::new(Size::new(100.0, 80.0), 10).with_pad(10.0)
    }

    #[test]
    fn cells_fill_rows_across_the_width() {
        // 420 wide less 20 of padding fits four 100-point cells.
        let grid = grid();
        assert_eq!(grid.columns(420.0), 4);
        assert_eq!(
            grid.cell_rect(5, 420.0),
            Rect::from_xywh(110.0, 90.0, 100.0, 80.0)
        );
        assert_eq!(grid.length(420.0), 3.0 * 80.0 + 20.0);
    }

    #[test]
    fn a_narrow_grid_still_has_one_column() {
        assert_eq!(grid().columns(30.0), 1);
    }

    #[test]
    fn hit_testing_a_grid_finds_the_cell_and_misses_the_gaps() {
        let grid = grid();
        assert_eq!(grid.index_at(Point::new(115.0, 95.0), 420.0), Some(5));
        assert_eq!(grid.index_at(Point::new(5.0, 95.0), 420.0), None);
        // Past the last cell of the last, short row.
        assert_eq!(grid.index_at(Point::new(315.0, 175.0), 420.0), None);
    }

    #[test]
    fn a_grid_range_is_whole_rows() {
        let grid = grid();
        // Rows start at 10, 90, 170: a span inside the second row is that row.
        assert_eq!(grid.range(100.0, 120.0, 420.0), 4..8);
        assert_eq!(grid.range(0.0, 1000.0, 420.0), 0..10);
    }

    fn sectioned() -> GridLayout {
        GridLayout::new(Size::new(100.0, 80.0), 7)
            .with_pad(10.0)
            .with_sections(
                30.0,
                vec![
                    GridSection {
                        first: 0,
                        count: 3,
                        headed: true,
                    },
                    GridSection {
                        first: 3,
                        count: 4,
                        headed: true,
                    },
                ],
            )
    }

    #[test]
    fn sections_start_on_a_new_row_under_their_heading() {
        let grid = sectioned();
        // Section one: heading at 10, cells from 40, one row. Section two:
        // heading at 120, cells from 150.
        assert_eq!(grid.cell_rect(0, 420.0).top, 40.0);
        assert_eq!(
            grid.cell_rect(3, 420.0),
            Rect::from_xywh(10.0, 150.0, 100.0, 80.0)
        );
        assert_eq!(grid.length(420.0), 30.0 + 80.0 + 30.0 + 80.0 + 20.0);
        assert_eq!(
            grid.headers(420.0),
            vec![
                (0, Rect::from_xywh(10.0, 10.0, 400.0, 30.0)),
                (1, Rect::from_xywh(10.0, 120.0, 400.0, 30.0)),
            ]
        );
    }

    #[test]
    fn a_click_on_a_heading_is_a_click_on_nothing() {
        let grid = sectioned();
        assert_eq!(grid.index_at(Point::new(50.0, 130.0), 420.0), None);
        assert_eq!(grid.index_at(Point::new(50.0, 160.0), 420.0), Some(3));
    }

    #[test]
    fn the_pinned_heading_is_pushed_out_by_the_next() {
        let grid = sectioned();
        // Scrolled a little: section one's heading pinned at the top.
        assert_eq!(grid.pinned_header(20.0, 420.0), Some((0, 0.0)));
        // The next heading (at 120) is 10 points from reaching the bottom of
        // the pinned one: pushed up by that much.
        assert_eq!(grid.pinned_header(100.0, 420.0), Some((0, -10.0)));
        // Past it: section two's heading takes over.
        assert_eq!(grid.pinned_header(130.0, 420.0), Some((1, 0.0)));
        // Above the first heading nothing is pinned.
        assert_eq!(grid.pinned_header(5.0, 420.0), None);
    }

    #[test]
    fn a_marquee_catches_the_cells_it_touches() {
        let grid = grid();
        // Across the boundary between the first two rows (at 90) and the
        // first three columns (edges at 110 and 210): six cells.
        let caught = grid.cells_in(Rect::from_ltrb(105.0, 85.0, 215.0, 95.0), 420.0);
        assert_eq!(caught, vec![0, 1, 2, 4, 5, 6]);
        // Inside one cell: that cell.
        let caught = grid.cells_in(Rect::from_ltrb(120.0, 100.0, 130.0, 110.0), 420.0);
        assert_eq!(caught, vec![5]);
        assert!(grid
            .cells_in(Rect::from_ltrb(50.0, 50.0, 50.0, 50.0), 420.0)
            .is_empty());
    }
}
