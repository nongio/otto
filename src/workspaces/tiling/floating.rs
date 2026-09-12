//! The floating layer of a tiling workspace.
//!
//! A window on a tiling workspace that is not in the tree floats above the
//! tiles (`specs/tiling.md`, *Floating within a tiling workspace*). Nothing
//! here touches the compositor: this is the arithmetic of where a floated
//! window lands and which layer focus is on. The hooks that move windows and
//! restack them live in `src/shell/tiling.rs`.

use super::layout::Rect;

/// Which of a tiling workspace's two layers something is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// In the tree.
    Tiled,
    /// Above the tree.
    Floating,
}

impl Layer {
    /// The layer `focus mode_toggle` moves to from this one.
    pub fn other(self) -> Layer {
        match self {
            Layer::Tiled => Layer::Floating,
            Layer::Floating => Layer::Tiled,
        }
    }
}

/// What fraction of the usable area a window gets when it is floated by hand
/// and has no remembered floating rectangle — it was opened straight into the
/// tree, so there is nothing to restore it to.
const DEFAULT_SHARE: f32 = 0.6;

/// Where a window goes when it is floated and never had a floating rect: a
/// sensible fraction of the workspace, centred on the cell it is leaving
/// (`specs/tiling.md`, *By hand*).
///
/// The result is always inside `area` — a cell at the edge pulls the rect
/// back rather than letting it hang off the screen — and never larger than
/// it.
pub fn default_float_rect(area: Rect, cell: Rect) -> Rect {
    let w = ((area.w as f32 * DEFAULT_SHARE) as i32)
        .max(1)
        .min(area.w.max(1));
    let h = ((area.h as f32 * DEFAULT_SHARE) as i32)
        .max(1)
        .min(area.h.max(1));
    // The cell's centre, not its corner: a floated window keeps the place on
    // screen it had, so the eye does not have to find it again.
    let cx = cell.x + cell.w / 2;
    let cy = cell.y + cell.h / 2;
    let x = (cx - w / 2).clamp(area.x, (area.x + area.w - w).max(area.x));
    let y = (cy - h / 2).clamp(area.y, (area.y + area.h - h).max(area.y));
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 30,
        w: 1000,
        h: 700,
    };

    #[test]
    fn a_floated_window_keeps_the_centre_of_its_cell() {
        // A cell in the middle of the area: the rect is centred on it.
        let cell = Rect::new(400, 280, 200, 200);
        let rect = default_float_rect(AREA, cell);
        assert_eq!(rect.w, 600);
        assert_eq!(rect.h, 420);
        assert_eq!(rect.x + rect.w / 2, cell.x + cell.w / 2);
        assert_eq!(rect.y + rect.h / 2, cell.y + cell.h / 2);
    }

    #[test]
    fn a_cell_at_the_edge_pulls_the_rect_back_inside() {
        let cell = Rect::new(0, 30, 100, 700);
        let rect = default_float_rect(AREA, cell);
        assert_eq!(rect.x, AREA.x, "never off the left edge");
        assert!(rect.y >= AREA.y);
        assert!(rect.x + rect.w <= AREA.x + AREA.w);
        assert!(rect.y + rect.h <= AREA.y + AREA.h);
    }

    #[test]
    fn the_rect_never_outgrows_a_tiny_workspace() {
        let area = Rect::new(0, 0, 10, 10);
        let rect = default_float_rect(area, area);
        assert!(rect.w <= area.w && rect.h <= area.h);
        assert!(rect.w >= 1 && rect.h >= 1);
    }

    #[test]
    fn the_layers_are_each_other_s_other() {
        assert_eq!(Layer::Tiled.other(), Layer::Floating);
        assert_eq!(Layer::Floating.other(), Layer::Tiled);
    }
}
