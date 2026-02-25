use layers::{prelude::*, skia};
use smithay::{
    backend::input::ButtonState,
    input::pointer::{CursorIcon, CursorImageStatus},
    reexports::wayland_server::backend::ObjectId,
};
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};

use crate::{
    config::Config,
    interactive_view::ViewInteractions,
    utils::natural_layout::{natural_layout, LayoutRect},
};

use super::{utils::FONT_CACHE, WORKSPACE_SELECTOR_PREVIEW_WIDTH};

// Logical (unscaled) values - will be multiplied by screen scale when used
const WINDOW_SELECTOR_DRAG_THRESHOLD_LOGICAL: f32 = 1.5;
const WORKSPACE_SELECTOR_TARGET_Y_LOGICAL: f32 = 200.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowSelection {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub window_title: String,
    pub index: usize,
    pub window_id: Option<ObjectId>,
}

#[derive(Debug, Clone)]
pub struct WindowSelectorState {
    pub rects: Vec<WindowSelection>,
    pub current_selection: Option<usize>,
}

impl Hash for WindowSelectorState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let current = self
            .current_selection
            .as_ref()
            .map(|x| self.rects.get(*x).unwrap());
        current.hash(state);
        self.rects.hash(state);
    }
}

impl Hash for WindowSelection {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.x.to_bits().hash(state);
        self.y.to_bits().hash(state);
        self.w.to_bits().hash(state);
        self.h.to_bits().hash(state);
        self.window_title.hash(state);
    }
}

#[derive(Clone, Hash)]
pub struct WindowSelectorWindow {
    pub id: ObjectId,
    pub rect: LayoutRect,
    pub title: String,
}

#[derive(Clone)]
pub struct HandlerFunction(pub std::sync::Arc<dyn Fn(usize) + 'static + Send + Sync>);

impl PartialEq for HandlerFunction {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<F: Fn(usize) + Send + Sync + 'static> From<F> for HandlerFunction {
    fn from(f: F) -> Self {
        HandlerFunction(std::sync::Arc::new(f))
    }
}

#[derive(Clone, Debug)]
pub struct DragState {
    pub window_layer: Layer,
    pub window_id: ObjectId,
    pub selection: WindowSelection,
    pub start_location: (f32, f32),
    pub offset: (f32, f32),
    pub original_position: layers::types::Point,
    pub original_scale: layers::types::Point,
    pub original_anchor: layers::types::Point,
    pub original_parent: Layer,
    pub current_drop_target: Option<usize>,
}

#[derive(Clone)]
pub struct WindowSelectorView {
    pub layer: layers::prelude::Layer,
    pub background_layer: layers::prelude::Layer,
    pub windows_layer: layers::prelude::Layer,
    pub overlay_layer: layers::prelude::Layer,
    pub drag_overlay_layer: layers::prelude::Layer,
    pub view: layers::prelude::View<WindowSelectorState>,
    pub windows: std::sync::Arc<RwLock<HashMap<ObjectId, Layer>>>,
    pub cursor_location: Arc<RwLock<Option<(f32, f32)>>>,
    pub press_location: Arc<RwLock<Option<(f32, f32)>>>,
    pub pointer_down: Arc<AtomicBool>,
    pub pressed_selection: Arc<RwLock<Option<WindowSelection>>>,
    pub drag_state: Arc<RwLock<Option<DragState>>>,
    pub expose_bin: Arc<RwLock<HashMap<ObjectId, LayoutRect>>>,
    layout_hash: Arc<RwLock<u64>>,
}

/// # WindowSelectorView Layer Structure
///
/// ```diagram
/// WindowSelectorView
/// ├── layer
/// │   ├── background_layer
/// │   ├── windows_layer
/// │   ├── overlay_layer (view(view_window_selector))
/// │   │   └── window_selector_label
/// ```
///
/// - `layer`: The root layer for the window selector view.
/// - `background_layer`: a replica of the workspace background
/// - `windows_layer`: windows replica container
/// - `overlay_layer`: draw the window selection and text
/// - `window_selector_label`: text layer for the window title
impl WindowSelectorView {
    pub fn new(
        index: usize,
        layers_engine: Arc<Engine>,
        background_layer: Layer,
        drag_overlay_layer: Layer,
    ) -> Self {
        let window_selector_root = layers_engine.new_layer();
        window_selector_root.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });

        window_selector_root.set_key(format!("window_selector_root_{}", index));
        window_selector_root.set_size(layers::types::Size::percent(1.0, 1.0), None);
        // window_selector_root.set_picture_cached(false);
        // window_selector_root.set_image_cached(false);
        layers_engine.add_layer(&window_selector_root);
        window_selector_root.set_size(layers::types::Size::percent(1.0, 1.0), None);

        // window_selector_root.set_background_color(Color::new_hex("#ff0000ff"), None);
        let overlay_layer = layers_engine.new_layer();
        overlay_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        // let mut overlay_color = theme_colors().accents_green;
        // overlay_color.alpha = 0.5;
        // overlay_layer.set_background_color(overlay_color, None);
        overlay_layer.set_size(layers::types::Size::percent(1.0, 1.0), None);
        overlay_layer.set_pointer_events(false);
        // overlay_layer.set_picture_cached(false);
        // overlay_layer.set_image_cached(false);
        overlay_layer.set_hidden(true);

        let state = WindowSelectorState {
            rects: vec![],
            current_selection: None,
        };
        let view = layers::prelude::View::new(
            format!("window_selector_view_{}", index),
            state,
            view_window_selector,
        );
        view.mount_layer(overlay_layer.clone());

        let clone_background_layer = layers_engine.new_layer();
        clone_background_layer.set_key(format!("window_selector_background_{}", index));
        clone_background_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        clone_background_layer.set_size(layers::types::Size::percent(1.0, 1.0), None);
        clone_background_layer.set_draw_content(background_layer.as_content());
        clone_background_layer.set_picture_cached(false);
        background_layer.add_follower_node(&clone_background_layer);
        clone_background_layer.set_opacity(1.0, None);
        let windows_layer = layers_engine.new_layer();
        windows_layer.set_key(format!("window_selector_windows_container_{}", index));
        windows_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        // windows_layer.set_pointer_events(false);
        // windows_layer.set_picture_cached(false);
        // windows_layer.set_image_cached(false);
        windows_layer.set_size(layers::types::Size::percent(1.0, 1.0), None);

        window_selector_root.add_sublayer(&clone_background_layer);

        window_selector_root.add_sublayer(&windows_layer);

        window_selector_root.add_sublayer(&overlay_layer);

        Self {
            view,
            layer: window_selector_root,
            background_layer: clone_background_layer,
            windows_layer,
            overlay_layer,
            drag_overlay_layer,
            windows: std::sync::Arc::new(RwLock::new(HashMap::new())),
            cursor_location: Arc::new(RwLock::new(None)),
            press_location: Arc::new(RwLock::new(None)),
            pointer_down: Arc::new(AtomicBool::new(false)),
            pressed_selection: Arc::new(RwLock::new(None)),
            drag_state: Arc::new(RwLock::new(None)),
            expose_bin: Arc::new(RwLock::new(HashMap::new())),
            layout_hash: Arc::new(RwLock::new(0)),
        }
    }
    pub fn layer_for_window(&self, window: &ObjectId) -> Option<Layer> {
        self.windows.read().unwrap().get(window).cloned()
    }

    /// add a window layer to windows map
    /// and append the window to the windows_layer
    pub fn map_window(&self, window_id: ObjectId, layer: &Layer) {
        self.windows_layer.add_sublayer(layer);
        self.windows
            .write()
            .unwrap()
            .insert(window_id, layer.clone());
    }

    /// Ensure the window preview layers keep the same stacking order
    /// as the current selector state, useful after a drag is cancelled.
    fn restore_layer_order_from_state(&self) {
        let state = self.view.get_state();
        let windows = self.windows.read().unwrap();
        for window_id in state.rects.iter().filter_map(|r| r.window_id.as_ref()) {
            if let Some(layer) = windows.get(window_id) {
                // Re-append in state order; append detaches first, so this
                // effectively reorders the siblings without changing parents.
                self.windows_layer.add_sublayer(layer);
            }
        }
    }
    /// remove the window from the windows map
    /// and remove the layer from windows_layer
    pub fn unmap_window(&self, window_id: &ObjectId) -> Option<Layer> {
        self.windows.write().unwrap().remove(window_id)
    }

    fn record_cursor_location(&self, location: (f32, f32)) {
        let mut cursor = self.cursor_location.write().unwrap();
        *cursor = Some(location);
    }

    fn current_pointer_or_default<Backend: crate::state::Backend>(
        &self,
        composer: &crate::Otto<Backend>,
    ) -> (f32, f32) {
        if let Some(location) = *self.cursor_location.read().unwrap() {
            location
        } else {
            let physical = composer.get_cursor_position();
            (physical.x as f32, physical.y as f32)
        }
    }

    fn preview_scale(&self) -> f32 {
        let workspace_width = self.layer.render_size().x.max(1.0);
        WORKSPACE_SELECTOR_PREVIEW_WIDTH / workspace_width
    }

    fn clear_press_context(&self) {
        *self.pressed_selection.write().unwrap() = None;
        *self.press_location.write().unwrap() = None;
    }

    fn update_drag_position(&self, pointer_location: (f32, f32)) {
        let drag_state = self.drag_state.read().unwrap();
        if let Some(ref state) = *drag_state {
            let position = layers::types::Point {
                x: pointer_location.0 - state.offset.0,
                y: pointer_location.1 - state.offset.1,
            };
            state.window_layer.set_position(position, None);
            drop(drag_state);
            self.update_drag_scale(pointer_location);
        }
    }

    fn stop_dragging(&self) -> Option<DragState> {
        let mut drag_state = self.drag_state.write().unwrap();
        let state = drag_state.take()?;

        state.window_layer.set_position(
            state.original_position,
            Some(Transition::ease_out_quad(0.12)),
        );
        state
            .window_layer
            .set_scale(state.original_scale, Some(Transition::ease_out_quad(0.12)));
        state
            .window_layer
            .set_anchor_point(state.original_anchor, Some(Transition::ease_out_quad(0.12)));
        state.original_parent.add_sublayer(&state.window_layer);

        Some(state)
    }

    fn remove_rect_from_state(&self, target_index: usize) {
        let mut state = self.view.get_state().clone();
        if let Some(position) = state.rects.iter().position(|r| r.index == target_index) {
            state.rects.remove(position);
            if let Some(current) = state.current_selection {
                match current.cmp(&target_index) {
                    std::cmp::Ordering::Equal => state.current_selection = None,
                    std::cmp::Ordering::Greater => state.current_selection = Some(current - 1),
                    std::cmp::Ordering::Less => {}
                }
            }
            for (idx, rect) in state.rects.iter_mut().enumerate() {
                rect.index = idx;
            }
            self.view.update_state(&state);
        }
    }

    fn restore_rect_to_state(&self, rect: WindowSelection) {
        let mut state = self.view.get_state().clone();
        let insert_idx = rect.index.min(state.rects.len());
        state.rects.insert(insert_idx, rect);
        for (idx, rect) in state.rects.iter_mut().enumerate() {
            rect.index = idx;
        }
        self.view.update_state(&state);
    }

    fn update_drag_scale(&self, pointer_location: (f32, f32)) {
        let drag_state = self.drag_state.read().unwrap();
        if let Some(ref state) = *drag_state {
            let screen_scale = Config::with(|config| config.screen_scale) as f32;
            let target_y = WORKSPACE_SELECTOR_TARGET_Y_LOGICAL * screen_scale;

            let preview_scale_value = self.preview_scale();
            let preview_scale_point = layers::types::Point {
                x: preview_scale_value,
                y: preview_scale_value,
            };
            let mut start_y_value = state.start_location.1;
            if start_y_value <= target_y {
                start_y_value = target_y + 0.01;
            }
            let mut progress = (pointer_location.1 - target_y) / (start_y_value - target_y);
            progress = progress.clamp(0.0, 1.0);

            let new_scale = layers::types::Point {
                x: preview_scale_point.x
                    + (state.original_scale.x - preview_scale_point.x) * progress,
                y: preview_scale_point.y
                    + (state.original_scale.y - preview_scale_point.y) * progress,
            };
            state.window_layer.set_scale(new_scale, None);
        }
    }

    fn try_activate_drag<Backend: crate::state::Backend>(
        &self,
        pointer_location: (f32, f32),
        otto: &crate::Otto<Backend>,
    ) -> Option<ObjectId> {
        // If already dragging, return the current window_id
        if self.drag_state.read().unwrap().is_some() {
            return self
                .drag_state
                .read()
                .unwrap()
                .as_ref()
                .map(|s| s.window_id.clone());
        }

        // Do not allow dragging when the current workspace is fullscreen
        if otto
            .workspaces
            .get_current_workspace()
            .map(|w| w.get_fullscreen_mode())
            .unwrap_or(false)
        {
            return None;
        }

        let selection = self.pressed_selection.read().unwrap().clone();
        let selection = selection?;

        if let Some(window_id) = &selection.window_id {
            let window_in_space = otto
                .workspaces
                .space()
                .map(|s| s.elements().any(|w| w.id() == *window_id))
                .unwrap_or(false);

            if !window_in_space {
                return None;
            }

            if let Some(window) = otto.workspaces.windows_map.get(window_id) {
                if window.is_fullscreen() {
                    return None;
                }
            }
        }
        let window_layer = selection
            .window_id
            .as_ref()
            .and_then(|id| self.layer_for_window(id));

        let window_layer = window_layer?;

        let bounds = window_layer.render_layer().global_transformed_bounds;
        let original_position = Point::new(bounds.left(), bounds.top());
        let render_size = bounds.size();

        let anchor_point = layers::types::Point {
            x: ((pointer_location.0 - original_position.x) / render_size.width).clamp(0.0, 1.0),
            y: ((pointer_location.1 - original_position.y) / render_size.height).clamp(0.0, 1.0),
        };

        let original_scale = window_layer.scale();
        let original_anchor = window_layer.anchor_point();

        // Move window to drag overlay
        self.drag_overlay_layer.add_sublayer(&window_layer);

        // Remove from state grid
        self.remove_rect_from_state(selection.index);

        // Set new anchor point and position
        let new_position = window_layer.set_anchor_point_preserving_position(anchor_point);
        window_layer.set_position(new_position, None);

        // Calculate offset for smooth dragging
        let offset = (
            pointer_location.0 - new_position.x,
            pointer_location.1 - new_position.y,
        );

        // Create and store drag state
        let drag_state = DragState {
            window_layer: window_layer.clone(),
            window_id: selection.window_id.clone()?,
            selection: selection.clone(),
            start_location: pointer_location,
            offset,
            original_position,
            original_scale,
            original_anchor,
            original_parent: self.windows_layer.clone(),
            current_drop_target: None,
        };

        *self.drag_state.write().unwrap() = Some(drag_state);

        self.update_drag_position(pointer_location);

        selection.window_id.clone()
    }
}

pub fn get_paragraph_for_text(text: &str, font_size: f32) -> skia::textlayout::Paragraph {
    let mut text_style = skia::textlayout::TextStyle::new();

    text_style.set_font_size(font_size);
    let font_style = skia::FontStyle::new(
        skia::font_style::Weight::SEMI_BOLD,
        skia::font_style::Width::CONDENSED,
        skia::font_style::Slant::Upright,
    );
    text_style.set_font_style(font_style);
    text_style.set_letter_spacing(-1.0);
    let foreground_paint = skia::Paint::new(skia::Color4f::new(0.1, 0.1, 0.1, 0.9), None);
    text_style.set_foreground_paint(&foreground_paint);
    let ff = Config::with(|c| c.font_family.clone());
    text_style.set_font_families(&[ff]);

    let mut paragraph_style = skia::textlayout::ParagraphStyle::new();
    paragraph_style.set_text_style(&text_style);
    paragraph_style.set_max_lines(1);
    paragraph_style.set_text_align(skia::textlayout::TextAlign::Center);
    paragraph_style.set_text_direction(skia::textlayout::TextDirection::LTR);
    paragraph_style.set_ellipsis("…");

    let mut builder = FONT_CACHE.with(|font_cache| {
        skia::textlayout::ParagraphBuilder::new(
            &paragraph_style,
            font_cache.font_collection.clone(),
        )
    });
    let paragraph = builder.add_text(text).build();

    paragraph
}

pub fn view_window_selector(
    state: &WindowSelectorState,
    view: &View<WindowSelectorState>,
) -> LayerTree {
    let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;

    let font_size: f32 = 20.0 * draw_scale;
    let current = state
        .current_selection
        .map(|x| state.rects.get(x).unwrap().clone())
        .map(|window_selection| {
            let mut paragraph = get_paragraph_for_text(&window_selection.window_title, font_size);

            paragraph.layout(1000.0 * draw_scale);
            let range: std::ops::Range<usize> = 0..window_selection.window_title.len();
            let rects = paragraph.get_rects_for_range(
                range,
                skia::textlayout::RectHeightStyle::Tight,
                skia::textlayout::RectWidthStyle::Tight,
            );
            let text_bounding_box = rects.iter().fold(skia::Rect::new_empty(), |acc, b| {
                skia::Rect::join2(acc, b.rect)
            });
            (window_selection, text_bounding_box)
        });

    let window_selection = current
        .as_ref()
        .map(|(window_selection, _)| window_selection.clone());

    let draw_container = Some(move |canvas: &skia::Canvas, w, h| {
        if window_selection.is_some() {
            let window_selection = window_selection.as_ref().unwrap();
            let color = crate::theme::accent_color().c4f();
            let mut paint = skia::Paint::new(color, None);
            paint.set_stroke(true);
            paint.set_stroke_width(8.0 * draw_scale);
            let rrect = skia::RRect::new_rect_xy(
                skia::Rect::from_xywh(
                    window_selection.x,
                    window_selection.y,
                    window_selection.w,
                    window_selection.h,
                )
                .with_outset((draw_scale * 8.0, draw_scale * 8.0)),
                16.0 * draw_scale,
                16.0 * draw_scale,
            );

            canvas.draw_rrect(rrect, &paint);
        }
        skia::Rect::from_xywh(0.0, 0.0, w, h)
    });

    let text_padding_x: f32 = 10.0 * draw_scale;
    let text_padding_y: f32 = 5.0 * draw_scale;
    // let text_x = 0.0;
    // let text_y = 0.0;
    let (text_rect, text_bounding_box) = current
        .as_ref()
        .map(|(rect, bb)| (rect.clone(), *bb))
        .unwrap_or((WindowSelection::default(), skia::Rect::new_empty()));
    let text_layer_size = layers::types::Size::points(
        if text_bounding_box.width() == 0.0 {
            0.0
        } else {
            text_bounding_box.width() + text_padding_x * 2.0
        },
        if text_bounding_box.height() == 0.0 {
            0.0
        } else {
            text_bounding_box.height() + text_padding_y * 2.0
        },
    );
    LayerTreeBuilder::default()
        .key(view.get_key())
        .position(((0.0, 0.0).into(), None))
        .size(layers::types::Size::percent(1.0, 1.0))
        .content(draw_container)
        // .picture_cached(false)
        // .image_cache(false)
        .children(vec![LayerTreeBuilder::default()
            .key("window_selector_label")
            .layout_style(taffy::Style {
                position: taffy::Position::Absolute,
                ..Default::default()
            })
            .position((
                (
                    text_rect.x + text_rect.w / 2.0 - text_bounding_box.width() / 2.0,
                    text_rect.y + text_rect.h / 2.0 - text_bounding_box.height() / 2.0,
                )
                    .into(),
                None,
            ))
            .size((text_layer_size, None))
            .blend_mode(layers::prelude::BlendMode::BackgroundBlur)
            .border_corner_radius((BorderRadius::new_single(8.0 * draw_scale), None))
            .background_color((
                PaintColor::Solid {
                    color: Color::new_rgba(1.0, 1.0, 1.0, 0.4),
                },
                None,
            ))
            .shadow_color((Color::new_rgba(0.0, 0.0, 0.0, 0.2), None))
            .shadow_offset(((0.0, 0.0).into(), None))
            .shadow_radius((5.0, None))
            // .shadow_spread((10.0, None))
            .content(Some(move |canvas: &skia::Canvas, w, h| {
                let mut paragraph = get_paragraph_for_text(&text_rect.window_title, font_size);
                paragraph.layout(w);
                // let text_x = TEXT_PADDING_X;
                let text_y = text_padding_y;

                paragraph.paint(canvas, (0.0, text_y));
                let safe = 200.0 * draw_scale;
                skia::Rect::from_xywh(-safe, -safe, w + safe * 2.0, h + safe * 2.0)
            }))
            .build()
            .unwrap()])
        .build()
        .unwrap()
}

impl WindowSelectorView {
    fn compute_layout_hash(
        windows: &[WindowSelectorWindow],
        layout_rect: &LayoutRect,
        offset_y: f32,
    ) -> u64 {
        let mut hasher = DefaultHasher::new();
        layout_rect.hash(&mut hasher);
        offset_y.to_bits().hash(&mut hasher);
        // sort the windows by window.id.protocol_id()
        let windows_sorted = {
            let mut ws = windows.to_vec();
            ws.sort_by_key(|w| w.id.protocol_id());
            ws
        };
        for window in &windows_sorted {
            window.id.hash(&mut hasher);
            window.rect.hash(&mut hasher);
            window.title.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn is_layout_up_to_date(
        &self,
        layout_rect: &LayoutRect,
        offset_y: f32,
        windows: &[WindowSelectorWindow],
    ) -> bool {
        let layout_hash = Self::compute_layout_hash(windows, layout_rect, offset_y);
        let stored_hash = *self.layout_hash.read().unwrap();
        let has_bin = !self.expose_bin.read().unwrap().is_empty();
        stored_hash == layout_hash && has_bin
    }

    /// Updates the window selector layout and state.
    ///
    /// Recalculates the layout of window previews in the selector view when the geometry or window list changes.
    /// It uses a hash to detect changes in layout parameters and only recomputes the layout if necessary. The function updates
    /// the internal bin mapping window IDs to their layout rectangles, computes scaling for each window preview, and updates
    /// the selector state with the new positions and sizes. The view is then refreshed to reflect the new state.
    pub fn update_windows(
        &self,
        layout_rect: LayoutRect,
        offset_y: f32,
        windows: &[WindowSelectorWindow],
    ) {
        let layout_hash = Self::compute_layout_hash(windows, &layout_rect, offset_y);
        let mut bin = self.expose_bin.write().unwrap();
        let mut stored_hash = self.layout_hash.write().unwrap();

        // No-op if nothing changed and bin already exists
        if *stored_hash == layout_hash && !bin.is_empty() {
            return;
        }

        // Recalculate layout only when geometry changed
        if *stored_hash != layout_hash {
            bin.clear();
            natural_layout(
                &mut bin,
                windows
                    .iter()
                    .map(|window| (window.id.clone(), window.rect)),
                &layout_rect,
                false,
            );
            *stored_hash = layout_hash;
        }

        let mut state = WindowSelectorState {
            rects: vec![],
            current_selection: None,
        };

        for (index, window) in windows.iter().enumerate() {
            if let Some(rect) = bin.get(&window.id) {
                let scale_x = rect.width / window.rect.width;
                let scale_y = rect.height / window.rect.height;
                let scale = scale_x.min(scale_y).min(1.0);

                state.rects.push(WindowSelection {
                    x: rect.x,
                    y: rect.y + offset_y,
                    w: window.rect.width * scale,
                    h: window.rect.height * scale,
                    window_title: window.title.clone(),
                    index,
                    window_id: Some(window.id.clone()),
                });
            }
        }

        self.view.update_state(&state);
    }
}
impl<Backend: crate::state::Backend> ViewInteractions<Backend> for WindowSelectorView {
    fn id(&self) -> Option<usize> {
        self.view
            .layer
            .read()
            .unwrap()
            .as_ref()
            .map(|l| l.id.0.into())
    }

    fn is_alive(&self) -> bool {
        !self
            .view
            .layer
            .read()
            .unwrap()
            .as_ref()
            .map(|l| l.hidden())
            .unwrap_or(true)
    }
    fn on_motion(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        otto: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        // println!("on_motion");
        let mut state = self.view.get_state().clone();
        let screen_scale = Config::with(|config| config.screen_scale);
        let location = event.location.to_physical(screen_scale);
        let cursor_point = (location.x as f32, location.y as f32);
        self.record_cursor_location(cursor_point);

        // If dragging, update drag position and check for drop targets
        let is_dragging = self.drag_state.read().unwrap().is_some();
        if is_dragging {
            self.update_drag_position(cursor_point);

            // Check if dragged window intersects with any workspace preview drop target
            let drop_targets = otto.workspaces.workspace_selector_view.get_drop_targets();
            let mut new_drop_target = None;

            // Get the dragged window's bounds
            if let Some(drag_state) = self.drag_state.read().unwrap().as_ref() {
                let drag_bounds = drag_state.window_layer.render_bounds_transformed();

                for target in drop_targets {
                    // Map view index to workspace position
                    let Some(target_pos) = otto
                        .workspaces
                        .workspace_position_by_view_index(target.workspace_index)
                    else {
                        tracing::warn!(
                            "Expose drop hover: workspace view index {} not found",
                            target.workspace_index
                        );
                        continue;
                    };
                    if target_pos == otto.workspaces.get_current_workspace_index() {
                        continue; // Skip current workspace
                    }
                    // Skip fullscreen workspaces - can't drop windows into them
                    if let Some(ws) = otto.workspaces.get_workspace_at(target_pos) {
                        if ws.get_fullscreen_mode() {
                            continue;
                        }
                    }
                    // Use Skia's intersect to check if drag bounds overlap with drop target
                    if drag_bounds.intersects(target.drop_layer.render_bounds_transformed()) {
                        new_drop_target = Some(target.workspace_index);
                        break;
                    }
                }
            }

            // Update drag state and visual feedback if target changed
            let current_target = self
                .drag_state
                .read()
                .unwrap()
                .as_ref()
                .and_then(|ds| ds.current_drop_target);
            if current_target != new_drop_target {
                if let Some(drag_state) = self.drag_state.write().unwrap().as_mut() {
                    drag_state.current_drop_target = new_drop_target;
                }
                otto.workspaces
                    .workspace_selector_view
                    .set_drop_hover(new_drop_target);
            }

            return;
        }

        // Check for drag threshold
        if self.pointer_down.load(Ordering::SeqCst) {
            if let Some(_selection) = self.pressed_selection.read().unwrap().clone() {
                if let Some(start) = *self.press_location.read().unwrap() {
                    let screen_scale = Config::with(|config| config.screen_scale) as f32;
                    let drag_threshold = WINDOW_SELECTOR_DRAG_THRESHOLD_LOGICAL * screen_scale;

                    let delta_x = (cursor_point.0 - start.0).abs();
                    let delta_y = (cursor_point.1 - start.1).abs();
                    if delta_x >= drag_threshold || delta_y >= drag_threshold {
                        if let Some(window_id) = self.try_activate_drag(cursor_point, otto) {
                            otto.workspaces.start_window_selector_drag(&window_id);
                            self.update_drag_position(cursor_point);
                            let cursor = CursorImageStatus::Named(CursorIcon::Move);
                            otto.set_cursor(&cursor);
                            return;
                        }
                    }
                }
            }
        }
        let rect = state
            .rects
            .iter()
            .find(|rect| {
                if rect.x < location.x as f32
                    && rect.x + rect.w > location.x as f32
                    && rect.y < location.y as f32
                    && rect.y + rect.h > location.y as f32
                {
                    state.current_selection = Some(rect.index);
                    let cursor = CursorImageStatus::Named(CursorIcon::Pointer);
                    otto.set_cursor(&cursor);
                    true
                } else {
                    let cursor = CursorImageStatus::Named(CursorIcon::default());
                    otto.set_cursor(&cursor);
                    false
                }
            })
            .map(|x| x.index);

        self.view.update_state(&WindowSelectorState {
            rects: state.rects,
            current_selection: rect,
        });
    }
    fn on_button(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        otto: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        match event.state {
            ButtonState::Pressed => {
                self.pointer_down.store(true, Ordering::SeqCst);
                let pointer_location = self.current_pointer_or_default(otto);
                *self.press_location.write().unwrap() = Some(pointer_location);

                let state = self.view.get_state();
                let selection = state
                    .current_selection
                    .and_then(|index| state.rects.get(index).cloned());

                if let Some(selection) = selection {
                    *self.pressed_selection.write().unwrap() = Some(selection);
                } else {
                    self.clear_press_context();
                }
            }
            ButtonState::Released => {
                self.pointer_down.store(false, Ordering::SeqCst);
                let was_dragging = self.drag_state.read().unwrap().is_some();

                if was_dragging {
                    if let Some(drag_state) = self.stop_dragging() {
                        let drop_target = drag_state.current_drop_target;

                        // Clear drop hover visual feedback
                        otto.workspaces.workspace_selector_view.set_drop_hover(None);

                        if let Some(target_workspace) = drop_target {
                            let target_pos = otto
                                .workspaces
                                .workspace_position_by_view_index(target_workspace);
                            if target_pos.is_none() {
                                tracing::warn!(
                                    "Expose drop: workspace {} not found, restoring window {:?}",
                                    target_workspace,
                                    drag_state.window_id
                                );
                            }
                            // Drop window onto target workspace
                            if let (Some(window_element), Some(target_pos)) = (
                                otto.workspaces
                                    .windows_map
                                    .get(&drag_state.window_id)
                                    .cloned(),
                                target_pos,
                            ) {
                                // Get position in current workspace before moving
                                let position = otto
                                    .workspaces
                                    .space()
                                    .and_then(|s| s.element_location(&window_element))
                                    .unwrap_or_default();

                                // Clear dragging state
                                *otto.workspaces.expose_dragged_window.lock().unwrap() = None;

                                // Move window to target workspace
                                // Note: unmap_window no longer removes the mirror layer to avoid SlotMap key issues
                                otto.workspaces.move_window_to_workspace(
                                    &window_element,
                                    target_pos,
                                    position,
                                );
                                // Refresh expose view
                                otto.workspaces.expose_set_visible(true);
                            } else {
                                tracing::warn!(
                                    "Expose drop: window {:?} not found in windows_map, ending drag only",
                                    drag_state.window_id
                                );
                                otto.workspaces
                                    .end_window_selector_drag(&drag_state.window_id);
                            }
                        } else {
                            tracing::trace!(
                                "Expose drop: no target workspace, restoring window {:?} to original position",
                                drag_state.window_id
                            );
                            // No drop target - restore to original position
                            self.restore_rect_to_state(drag_state.selection.clone());
                            self.restore_layer_order_from_state();
                            otto.workspaces
                                .end_window_selector_drag(&drag_state.window_id);
                            otto.workspaces.expose_update_if_needed();
                        }
                    }
                    self.clear_press_context();
                    otto.set_cursor(&CursorImageStatus::default_named());
                    return;
                }
                self.clear_press_context();

                let selector_state = self.view.get_state();
                if let Some(index) = selector_state.current_selection {
                    if let Some(window_selection) = selector_state.rects.get(index) {
                        if let Some(wid) = window_selection.window_id.clone() {
                            otto.workspaces.focus_app_with_window(&wid);
                            otto.set_keyboard_focus_on_surface(&wid);
                        }
                    }
                }
                otto.workspaces.expose_set_visible(false);
                otto.set_cursor(&CursorImageStatus::default_named());
                let state = WindowSelectorState {
                    current_selection: None,
                    ..selector_state
                };
                self.view.update_state(&state);
            }
        }
    }
}
