//! Geometry for scrolling content: rows, grids and justified rows.
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
        if self.sections.is_empty() {
            // One plain lattice, which is most grids: no sections to walk.
            if index >= self.count {
                return Rect::new_empty();
            }
            return Rect::from_xywh(
                self.pad + (index % cols) as f32 * self.cell.width,
                self.pad + (index / cols) as f32 * self.cell.height,
                self.cell.width,
                self.cell.height,
            );
        }
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
    ///
    /// A rect with no extent is a click, and catches nothing. One flat in a
    /// single axis is a sweep — a pointer dragged straight across a row rarely
    /// moves a whole pixel down — and catches what it crosses.
    pub fn cells_in(&self, rect: Rect, width: f32) -> Vec<usize> {
        if self.count == 0 || (rect.width() <= 0.0 && rect.height() <= 0.0) {
            return Vec::new();
        }
        // A sliver of extent, big enough to survive being added to a large
        // coordinate, so a flat sweep still intersects what it crosses.
        const SLIVER: f32 = 0.01;
        let probe = Rect::from_ltrb(
            rect.left,
            rect.top,
            rect.right.max(rect.left + SLIVER),
            rect.bottom.max(rect.top + SLIVER),
        );
        self.range(probe.top, probe.bottom, width)
            .filter(|&index| {
                let cell = self.cell_rect(index, width);
                !cell.is_empty() && cell.intersects(probe)
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

/// A direction to step in from one item of a [`JustifiedLayout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// A run of items in a [`JustifiedLayout`], under an optional heading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JustifiedSection<'a> {
    /// Height of the heading band above the section's rows. `0.0` for a
    /// section with no heading.
    pub header: f32,
    pub items: SectionItems<'a>,
}

impl<'a> JustifiedSection<'a> {
    /// Items at their own proportions, packed into justified rows.
    pub fn justified(header: f32, aspects: &'a [f32]) -> Self {
        Self {
            header,
            items: SectionItems::Justified(aspects),
        }
    }

    /// `count` items of one fixed size, in left-aligned rows that wrap.
    pub fn cells(header: f32, cell: Size, count: usize) -> Self {
        Self {
            header,
            items: SectionItems::Cells { cell, count },
        }
    }
}

/// What a [`JustifiedSection`] holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SectionItems<'a> {
    /// Each item's width over its height, in order. A ratio that is not a
    /// positive finite number is laid out as a square.
    Justified(&'a [f32]),
    /// Cards of one size — folders above a wall of photos — laid out as many
    /// to a row as fit, with the layout's gap between them.
    Cells { cell: Size, count: usize },
}

impl SectionItems<'_> {
    fn len(&self) -> usize {
        match self {
            SectionItems::Justified(aspects) => aspects.len(),
            SectionItems::Cells { count, .. } => *count,
        }
    }
}

/// One packed row of a [`JustifiedLayout`].
#[derive(Debug, Clone, Copy, PartialEq)]
struct JustifiedRow {
    /// The row's first item and one past its last.
    first: usize,
    end: usize,
    top: f32,
    height: f32,
}

impl JustifiedRow {
    fn bottom(&self) -> f32 {
        self.top + self.height
    }
}

/// Items of varying aspect ratio packed into rows that each fill the width,
/// the way a photo library lays out pictures without cropping them to one
/// shape.
///
/// Rows are filled greedily: items are added until the height that would make
/// them span the width exactly drops to the target height or below, and that
/// height becomes the row's. Every item in a row shares its height and keeps
/// its own proportions. A section's last row, when it runs out of items before
/// it fills, stays at the target height and left-aligned rather than being
/// stretched into a strip of giants.
///
/// Unlike [`GridLayout`] this is not closed-form: the rows depend on every
/// ratio before them, so it is computed once for a width and queried from the
/// result. Every query is a binary search over the rows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JustifiedLayout {
    items: Vec<Rect>,
    rows: Vec<JustifiedRow>,
    /// One per section, empty for a section without a heading.
    headers: Vec<Rect>,
    length: f32,
}

impl JustifiedLayout {
    /// A layout with nothing in it.
    pub const fn empty() -> Self {
        Self {
            items: Vec::new(),
            rows: Vec::new(),
            headers: Vec::new(),
            length: 0.0,
        }
    }

    /// Pack `sections` into rows across `width`, aiming at rows `target`
    /// tall, with `gap` between items and between rows and `pad` around the
    /// whole layout.
    pub fn new(
        sections: &[JustifiedSection<'_>],
        width: f32,
        target: f32,
        gap: f32,
        pad: f32,
    ) -> Self {
        let avail = (width - pad * 2.0).max(1.0);
        let target = target.max(1.0);
        let count = sections.iter().map(|s| s.items.len()).sum();
        let mut layout = Self {
            items: Vec::with_capacity(count),
            ..Self::empty()
        };
        let mut y = pad;
        let mut last_bottom = pad;
        for section in sections {
            if section.header > 0.0 {
                layout
                    .headers
                    .push(Rect::from_xywh(pad, y, avail, section.header));
                y += section.header;
                last_bottom = y;
            } else {
                layout.headers.push(Rect::new_empty());
            }
            let aspects: Vec<f32> = match section.items {
                SectionItems::Justified(aspects) => aspects
                    .iter()
                    .map(|&a| if a.is_finite() && a > 0.0 { a } else { 1.0 })
                    .collect(),
                SectionItems::Cells { cell, count } => {
                    let bottom = layout.place_cells(cell, count, avail, gap, pad, y);
                    if count > 0 {
                        last_bottom = bottom;
                        y = bottom + gap;
                    }
                    continue;
                }
            };
            let mut start = 0;
            while start < aspects.len() {
                let (end, height, justified) = Self::pack(&aspects[start..], avail, target, gap);
                let first = layout.items.len();
                let mut x = pad;
                for (k, &aspect) in aspects[start..start + end].iter().enumerate() {
                    let last = k + 1 == end;
                    // The last item of a full row takes up the rounding, so
                    // the row's right edge is exactly the layout's.
                    let w = if justified && last {
                        (pad + avail - x).max(1.0)
                    } else {
                        aspect * height
                    };
                    layout.items.push(Rect::from_xywh(x, y, w, height));
                    x += w + gap;
                }
                layout.rows.push(JustifiedRow {
                    first,
                    end: first + end,
                    top: y,
                    height,
                });
                last_bottom = y + height;
                y += height + gap;
                start += end;
            }
        }
        layout.length = last_bottom + pad;
        layout
    }

    /// Lay `count` fixed cells out in rows from `top`, and return the bottom
    /// of the last row.
    fn place_cells(
        &mut self,
        cell: Size,
        count: usize,
        avail: f32,
        gap: f32,
        pad: f32,
        top: f32,
    ) -> f32 {
        let (w, h) = (cell.width.max(1.0), cell.height.max(1.0));
        let per_row = (((avail + gap) / (w + gap)).floor() as usize).max(1);
        let mut y = top;
        let mut placed = 0;
        while placed < count {
            let n = per_row.min(count - placed);
            let first = self.items.len();
            for k in 0..n {
                self.items
                    .push(Rect::from_xywh(pad + k as f32 * (w + gap), y, w, h));
            }
            self.rows.push(JustifiedRow {
                first,
                end: first + n,
                top: y,
                height: h,
            });
            placed += n;
            y += h + gap;
        }
        y - gap
    }

    /// How many of `aspects` go in the next row, the row's height, and
    /// whether it was filled to the width.
    fn pack(aspects: &[f32], avail: f32, target: f32, gap: f32) -> (usize, f32, bool) {
        let mut sum = 0.0;
        for (i, &aspect) in aspects.iter().enumerate() {
            sum += aspect;
            let n = (i + 1) as f32;
            let height = (avail - gap * (n - 1.0)) / sum;
            if height <= target {
                return (i + 1, height.max(1.0), true);
            }
        }
        (aspects.len(), target, false)
    }

    /// How many items were laid out.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Total length of the content: what a scroll view's content length is.
    pub fn length(&self) -> f32 {
        self.length
    }

    /// The rect of item `index`. Empty past the end: asked for while a
    /// listing is replaced underneath, an empty rect is a less surprising
    /// answer than a panic.
    pub fn rect(&self, index: usize) -> Rect {
        self.items
            .get(index)
            .copied()
            .unwrap_or_else(Rect::new_empty)
    }

    /// Every section's heading rect, in section order. A section without a
    /// heading has an empty rect in its place.
    pub fn headers(&self) -> &[Rect] {
        &self.headers
    }

    /// The row holding item `index`.
    fn row_of(&self, index: usize) -> Option<usize> {
        let row = self.rows.partition_point(|row| row.end <= index);
        (row < self.rows.len() && index >= self.rows[row].first).then_some(row)
    }

    /// The item under `point`, if any. Headings, gaps and the space after a
    /// short row are all misses.
    pub fn index_at(&self, point: Point) -> Option<usize> {
        let row = self.rows.partition_point(|row| row.bottom() <= point.y);
        let row = self.rows.get(row)?;
        if point.y < row.top {
            return None;
        }
        (row.first..row.end).find(|&index| {
            let rect = self.items[index];
            point.x >= rect.left && point.x < rect.right
        })
    }

    /// The items whose rows intersect `lo..hi`, widened to whole rows. One
    /// contiguous range, since rows are in item order.
    pub fn range(&self, lo: f32, hi: f32) -> Range<usize> {
        if self.rows.is_empty() || hi <= lo {
            return 0..0;
        }
        let first = self.rows.partition_point(|row| row.bottom() < lo);
        let last = self.rows.partition_point(|row| row.top <= hi);
        if first >= last {
            let at = self.rows.get(first).map_or(self.len(), |row| row.first);
            return at..at;
        }
        self.rows[first].first..self.rows[last - 1].end
    }

    /// The items `rect` touches at all — a marquee's hit test.
    ///
    /// A rect with no extent is a click, and catches nothing. One flat in a
    /// single axis is a sweep, and catches what it crosses.
    pub fn cells_in(&self, rect: Rect) -> Vec<usize> {
        if self.items.is_empty() || (rect.width() <= 0.0 && rect.height() <= 0.0) {
            return Vec::new();
        }
        // Same sliver as `GridLayout::cells_in`, for the same reason.
        const SLIVER: f32 = 0.01;
        let probe = Rect::from_ltrb(
            rect.left,
            rect.top,
            rect.right.max(rect.left + SLIVER),
            rect.bottom.max(rect.top + SLIVER),
        );
        self.range(probe.top, probe.bottom)
            .filter(|&index| self.items[index].intersects(probe))
            .collect()
    }

    /// The item one step from `index` in `direction`, for keyboard movement.
    ///
    /// Left and Right step through the items in order, crossing row ends.
    /// Up and Down go to the adjacent row, including across a heading, and
    /// land on the item there whose horizontal centre is closest to this
    /// one's. `None` at the edges of the layout.
    pub fn neighbor(&self, index: usize, direction: Direction) -> Option<usize> {
        if index >= self.items.len() {
            return None;
        }
        let row = self.row_of(index)?;
        let target = match direction {
            Direction::Left => return index.checked_sub(1),
            Direction::Right => return (index + 1 < self.items.len()).then_some(index + 1),
            Direction::Up => row.checked_sub(1)?,
            Direction::Down => row + 1,
        };
        let target = self.rows.get(target)?;
        let centre = self.items[index].center_x();
        (target.first..target.end).min_by(|&a, &b| {
            let da = (self.items[a].center_x() - centre).abs();
            let db = (self.items[b].center_x() - centre).abs();
            da.total_cmp(&db)
        })
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

    #[test]
    fn a_marquee_flat_in_one_axis_still_catches_what_it_crosses() {
        let grid = grid();
        // A sweep straight across the second row, not a pixel of height: it
        // crosses the first three cells of that row.
        let caught = grid.cells_in(Rect::from_ltrb(20.0, 130.0, 250.0, 130.0), 420.0);
        assert_eq!(caught, vec![4, 5, 6]);
        // And straight down the second column, far enough down that a tiny
        // epsilon would vanish into the coordinate.
        let tall = GridLayout::new(Size::new(100.0, 80.0), 4_000).with_pad(10.0);
        let y = 80.0 * 900.0;
        let caught = tall.cells_in(Rect::from_ltrb(150.0, y + 20.0, 150.0, y + 100.0), 420.0);
        assert_eq!(caught, vec![3601, 3605]);
    }

    fn justified(sections: &[(f32, &[f32])], width: f32) -> JustifiedLayout {
        let sections: Vec<JustifiedSection<'_>> = sections
            .iter()
            .map(|&(header, aspects)| JustifiedSection::justified(header, aspects))
            .collect();
        JustifiedLayout::new(&sections, width, 100.0, 4.0, 0.0)
    }

    #[test]
    fn a_full_row_spans_the_width_at_one_height() {
        // Four 3:2 items at 100 tall are 600 wide plus gaps; in 404 points the
        // first three give (404 - 8) / 4.5 = 88, which is the row's height.
        let layout = justified(&[(0.0, &[1.5, 1.5, 1.5, 1.5])], 404.0);
        let row: Vec<Rect> = (0..3).map(|i| layout.rect(i)).collect();
        for rect in &row {
            assert!((rect.height() - 88.0).abs() < 1e-3, "{rect:?}");
        }
        assert_eq!(row[0].left, 0.0);
        assert!((row[2].right - 404.0).abs() < 1e-3, "{row:?}");
        assert!((row[1].left - (row[0].right + 4.0)).abs() < 1e-3);
    }

    #[test]
    fn a_short_last_row_keeps_the_target_height_and_is_not_stretched() {
        let layout = justified(&[(0.0, &[1.5, 1.5, 1.5, 1.5])], 404.0);
        let last = layout.rect(3);
        assert_eq!(last.height(), 100.0);
        assert_eq!(last.width(), 150.0);
        assert_eq!(last.left, 0.0);
        assert!((last.top - (88.0 + 4.0)).abs() < 1e-3, "{last:?}");
        assert!((layout.length() - (88.0 + 4.0 + 100.0)).abs() < 1e-3);
    }

    #[test]
    fn headings_push_their_sections_down() {
        let layout = justified(&[(30.0, &[1.0, 1.0]), (30.0, &[1.0])], 1000.0);
        assert_eq!(layout.headers()[0], Rect::from_xywh(0.0, 0.0, 1000.0, 30.0));
        assert_eq!(layout.rect(0).top, 30.0);
        // The second heading follows the first section's row and its gap.
        assert_eq!(layout.headers()[1].top, 30.0 + 100.0 + 4.0);
        assert_eq!(layout.rect(2).top, layout.headers()[1].bottom);
        assert_eq!(layout.length(), layout.rect(2).bottom);
    }

    #[test]
    fn a_point_finds_its_item_and_gaps_find_nothing() {
        let layout = justified(&[(20.0, &[1.0, 1.0, 1.0])], 1000.0);
        assert_eq!(layout.index_at(Point::new(50.0, 60.0)), Some(0));
        assert_eq!(layout.index_at(Point::new(150.0, 60.0)), Some(1));
        // The gap between the two, the heading, and past the short row.
        assert_eq!(layout.index_at(Point::new(102.0, 60.0)), None);
        assert_eq!(layout.index_at(Point::new(50.0, 10.0)), None);
        assert_eq!(layout.index_at(Point::new(900.0, 60.0)), None);
        assert_eq!(layout.index_at(Point::new(50.0, 500.0)), None);
    }

    #[test]
    fn the_range_covers_whole_rows() {
        let aspects = [1.0; 40];
        // Ten squares of 100 fit in 1036: rows of ten, 100 tall.
        let layout = justified(&[(0.0, &aspects)], 1036.0);
        assert_eq!(layout.rect(10).top, 104.0);
        assert_eq!(layout.range(0.0, 50.0), 0..10);
        assert_eq!(layout.range(120.0, 210.0), 10..30);
        assert_eq!(layout.range(5000.0, 6000.0), 40..40);
        assert!(layout
            .cells_in(Rect::from_ltrb(0.0, 0.0, 0.0, 0.0))
            .is_empty());
        assert_eq!(
            layout.cells_in(Rect::from_ltrb(50.0, 50.0, 160.0, 50.0)),
            vec![0, 1]
        );
    }

    #[test]
    fn up_and_down_land_on_the_nearest_centre() {
        // Row one: a wide item then two squares. Row two: squares.
        let layout = justified(&[(0.0, &[2.0, 1.0, 1.0, 1.0, 1.0, 1.0])], 400.0);
        assert_eq!(layout.rect(3).top, layout.rect(4).top);
        assert!(layout.rect(3).top > layout.rect(0).top);
        // Down from the wide item's centre lands under it; up from the second
        // square of row two lands on the wide item, the nearest centre.
        let down = layout.neighbor(0, Direction::Down).unwrap();
        assert!(
            layout.rect(down).center_x() < layout.rect(0).right,
            "{down}"
        );
        assert_eq!(layout.neighbor(2, Direction::Down), Some(5));
        assert_eq!(layout.neighbor(4, Direction::Up), Some(0));
        assert_eq!(layout.neighbor(0, Direction::Up), None);
        assert_eq!(layout.neighbor(5, Direction::Down), None);
        assert_eq!(layout.neighbor(2, Direction::Right), Some(3));
        assert_eq!(layout.neighbor(3, Direction::Left), Some(2));
        assert_eq!(layout.neighbor(0, Direction::Left), None);
        assert_eq!(layout.neighbor(5, Direction::Right), None);
    }

    #[test]
    fn up_and_down_cross_headings() {
        let layout = justified(&[(30.0, &[1.0]), (30.0, &[1.0, 1.0])], 1000.0);
        assert_eq!(layout.neighbor(0, Direction::Down), Some(1));
        assert_eq!(layout.neighbor(2, Direction::Up), Some(0));
    }

    #[test]
    fn cells_wrap_at_their_own_size_and_navigate_into_the_rows_below() {
        let sections = [
            JustifiedSection::cells(30.0, Size::new(240.0, 150.0), 5),
            JustifiedSection::justified(30.0, &[1.0, 1.0]),
        ];
        // Two cards fit in 500 (240 + 4 + 240), a third does not.
        let layout = JustifiedLayout::new(&sections, 500.0, 100.0, 4.0, 0.0);
        assert_eq!(layout.rect(0), Rect::from_xywh(0.0, 30.0, 240.0, 150.0));
        assert_eq!(layout.rect(1).left, 244.0);
        assert_eq!(layout.rect(2), Rect::from_xywh(0.0, 184.0, 240.0, 150.0));
        assert_eq!(layout.rect(4).top, 338.0);
        // The photos' heading follows the last row of cards and its gap.
        assert_eq!(layout.headers()[1].top, 338.0 + 150.0 + 4.0);
        assert_eq!(layout.neighbor(1, Direction::Down), Some(3));
        assert_eq!(layout.neighbor(4, Direction::Down), Some(6));
        assert_eq!(layout.neighbor(5, Direction::Up), Some(4));
        assert_eq!(layout.index_at(Point::new(300.0, 200.0)), Some(3));
    }

    #[test]
    fn a_nonsense_ratio_is_laid_out_square() {
        let layout = justified(&[(0.0, &[f32::NAN, 0.0, -2.0])], 1000.0);
        for i in 0..3 {
            assert_eq!(layout.rect(i).width(), 100.0);
        }
        assert!(layout.rect(3).is_empty());
    }
}
