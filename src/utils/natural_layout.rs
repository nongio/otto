use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
};

use rand::Rng;
use smithay::reexports::wayland_server::backend::ObjectId;

use crate::shell::WindowElement;

#[allow(unused)]
trait LayoutBoundingBox {
    fn bounding_box(&self) -> LayoutRect;
}

impl LayoutBoundingBox for WindowElement {
    fn bounding_box(&self) -> LayoutRect {
        // Return the bounding box of the window
        let bbox = self.bbox();
        LayoutRect::new(
            bbox.loc.x as f32,
            bbox.loc.y as f32,
            bbox.size.w as f32,
            bbox.size.h as f32,
        )
    }
}

const WINDOW_PLACEMENT_NATURAL_ACCURACY: f32 = 10.0;
const WINDOW_PLACEMENT_NATURAL_GAPS: f32 = 20.0;
const WINDOW_PLACEMENT_NATURAL_MAX_TRANSLATIONS: usize = 5000;
const WINDOW_PLACEMENT_JITTER_RADIUS_PX: f32 = 1.0;
const WINDOW_PLACEMENT_JITTER_STEP_DEG: f32 = 90.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Hash for LayoutRect {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.x.to_bits().hash(state);
        self.y.to_bits().hash(state);
        self.width.to_bits().hash(state);
        self.height.to_bits().hash(state);
    }
}

impl LayoutRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        LayoutRect {
            x,
            y,
            width,
            height,
        }
    }

    fn copy(&self) -> Self {
        *self
    }

    fn union(&self, rect2: &LayoutRect) -> Self {
        let mut dest = self.copy();
        if rect2.x < dest.x {
            dest.width += dest.x - rect2.x;
            dest.x = rect2.x;
        }
        if rect2.y < dest.y {
            dest.height += dest.y - rect2.y;
            dest.y = rect2.y;
        }
        if rect2.x + rect2.width > dest.x + dest.width {
            dest.width = rect2.x + rect2.width - dest.x;
        }
        if rect2.y + rect2.height > dest.y + dest.height {
            dest.height = rect2.y + rect2.height - dest.y;
        }
        dest
    }

    fn adjusted(&self, dx: f32, dy: f32, dx2: f32, dy2: f32) -> Self {
        let mut dest = self.copy();
        dest.x += dx;
        dest.y += dy;
        dest.width += -dx + dx2;
        dest.height += -dy + dy2;
        dest
    }

    fn overlap(&self, rect2: &LayoutRect) -> bool {
        !(self.x + self.width <= rect2.x
            || rect2.x + rect2.width <= self.x
            || self.y + self.height <= rect2.y
            || rect2.y + rect2.height <= self.y)
    }

    fn center(&self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    fn translate(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
    }
}

#[allow(clippy::mutable_key_type)]
pub fn natural_layout(
    slots: &mut HashMap<ObjectId, LayoutRect>,
    windows: impl IntoIterator<Item = (ObjectId, LayoutRect)>,
    area: &LayoutRect,
    use_more_screen: bool,
) {
    let padding = 20.0;
    let mut area_rect = area.copy();
    area_rect.x += padding;
    area_rect.y += padding;
    area_rect.width -= padding * 2.0;
    area_rect.height -= padding * 2.0;
    let mut bounds = area_rect.copy();

    let mut direction = 0;
    let mut directions = vec![];
    let mut rects = vec![];
    let mut rng = rand::thread_rng();

    for (index, (window_id, rect)) in windows.into_iter().enumerate() {
        // Add tiny deterministic jitter to break symmetry when windows are at identical positions.
        // Uses a configurable step angle (degrees). 90° matches the old 4-direction cycle.
        let step_rad = WINDOW_PLACEMENT_JITTER_STEP_DEG.to_radians();
        let angle = index as f32 * step_rad;
        let jitter_x = WINDOW_PLACEMENT_JITTER_RADIUS_PX * angle.cos();
        let jitter_y = WINDOW_PLACEMENT_JITTER_RADIUS_PX * angle.sin();

        let layout_rect = LayoutRect::new(
            rect.x + jitter_x,
            rect.y + jitter_y,
            rect.width,
            rect.height,
        );
        bounds = bounds.union(&layout_rect);

        rects.push((window_id, layout_rect));

        directions.push(direction);
        direction = (direction + 1) % 4;
    }

    let mut loop_counter = 0;
    let mut overlap;
    loop {
        overlap = false;
        for i in 0..rects.len() {
            for j in 0..rects.len() {
                if i != j {
                    let adjustments =
                        [-1.0, -1.0, 1.0, 1.0].map(|v| v * WINDOW_PLACEMENT_NATURAL_GAPS);
                    let i_adjusted = rects[i].1.adjusted(
                        adjustments[0],
                        adjustments[1],
                        adjustments[2],
                        adjustments[3],
                    );
                    let j_adjusted = rects[j].1.adjusted(
                        adjustments[0],
                        adjustments[1],
                        adjustments[2],
                        adjustments[3],
                    );
                    if i_adjusted.overlap(&j_adjusted) {
                        loop_counter += 1;
                        overlap = true;

                        let i_center = rects[i].1.center();
                        let j_center = rects[j].1.center();
                        let mut diff = (j_center.0 - i_center.0, j_center.1 - i_center.1);

                        if diff.0 == 0.0 && diff.1 == 0.0 {
                            diff.0 = 1.0;
                        }
                        if bounds.height / bounds.width > area_rect.height / area_rect.width {
                            diff.0 *= 2.0;
                        } else {
                            diff.1 *= 2.0;
                        }

                        let length = (diff.0 * diff.0 + diff.1 * diff.1).sqrt();
                        diff.0 = diff.0 * WINDOW_PLACEMENT_NATURAL_ACCURACY / length;
                        diff.1 = diff.1 * WINDOW_PLACEMENT_NATURAL_ACCURACY / length;

                        rects[i].1.translate(-diff.0, -diff.1);
                        rects[j].1.translate(diff.0, diff.1);

                        if use_more_screen {
                            let mut x_section =
                                ((rects[i].1.x - bounds.x) / (bounds.width / 3.0)).round() as i32;
                            let mut y_section =
                                ((rects[i].1.y - bounds.y) / (bounds.height / 3.0)).round() as i32;

                            let mut diff = (0.0, 0.0);
                            if x_section != 1 || y_section != 1 {
                                if x_section == 1 {
                                    x_section = if directions[i] / 2 == 0 { 0 } else { 2 };
                                }
                                if y_section == 1 {
                                    y_section = if directions[i] % 2 == 0 { 0 } else { 2 };
                                }
                            }
                            if x_section == 0 && y_section == 0 {
                                diff.0 = bounds.x - rects[i].1.center().0;
                                diff.1 = bounds.y - rects[i].1.center().1;
                            }
                            if x_section == 2 && y_section == 0 {
                                diff.0 = bounds.x + bounds.width - rects[i].1.center().0;
                                diff.1 = bounds.y - rects[i].1.center().1;
                            }
                            if x_section == 2 && y_section == 2 {
                                diff.0 = bounds.x + bounds.width - rects[i].1.center().0;
                                diff.1 = bounds.y + bounds.height - rects[i].1.center().1;
                            }
                            if x_section == 0 && y_section == 2 {
                                diff.0 = bounds.x - rects[i].1.center().0;
                                diff.1 = bounds.y + bounds.height - rects[i].1.center().1;
                            }
                            if diff.0 != 0.0 || diff.1 != 0.0 {
                                let length = (diff.0 * diff.0 + diff.1 * diff.1).sqrt();
                                diff.0 *= WINDOW_PLACEMENT_NATURAL_ACCURACY / length / 2.0;
                                diff.1 *= WINDOW_PLACEMENT_NATURAL_ACCURACY / length / 2.0;
                                rects[i].1.translate(diff.0, diff.1);
                            }
                        }

                        bounds = bounds.union(&rects[i].1);
                        bounds = bounds.union(&rects[j].1);
                    }
                }
            }
        }
        if !overlap || loop_counter >= WINDOW_PLACEMENT_NATURAL_MAX_TRANSLATIONS {
            break;
        }
    }

    let scale = (area_rect.width / bounds.width)
        .min(area_rect.height / bounds.height)
        .min(1.0);

    bounds.x -= (area_rect.width - bounds.width * scale) / 2.0;
    bounds.y -= (area_rect.height - bounds.height * scale) / 2.0;
    bounds.width = area_rect.width / scale;
    bounds.height = area_rect.height / scale;

    for (_, rect) in &mut rects {
        rect.translate(-bounds.x, -bounds.y);
    }

    for (id, rect) in rects.iter_mut() {
        rect.x = rect.x * scale + area_rect.x;
        rect.y = rect.y * scale + area_rect.y;
        rect.width *= scale;
        rect.height *= scale;
        slots.insert(id.clone(), *rect);
    }
}
