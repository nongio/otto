use std::sync::{atomic::AtomicBool, Arc};

use layers::{
    engine::animation::{TimingFunction, Transition},
    prelude::{taffy, BorderRadius, Layer, LayerTree, LayerTreeBuilder, Point, View},
    taffy::style::Style,
    types::{BlendMode, Size},
    view::RenderLayerTree,
};
use otto_kit::components::{
    context_menu::{ContextMenuRenderer, ContextMenuState, ContextMenuStyle},
    menu_item::MenuItem,
};
use smithay::utils::IsAlive;

use crate::theme::theme_colors;

use crate::config::Config;

/// A compositor-integrated context menu view
///
/// Wraps the otto-kit `ContextMenuState` + `ContextMenuRenderer` components
/// and integrates them with the compositor's layer-based architecture.
///
/// # Layer Structure
///
/// ```diagram
/// ContextMenuView
/// └── wrap_layer `context_menu_container`
///     └── view_layer (mounted by View<ContextMenuState>)
///         └── menu-container
///             ├── menu-depth-0  (root menu)
///             ├── menu-depth-1  (first submenu)
///             └── menu-depth-N  (nested submenus)
/// ```
#[derive(Debug, Clone)]
pub struct ContextMenuView {
    pub wrap_layer: Layer,
    pub view_layer: Layer,
    pub view: View<ContextMenuState>,
    active: Arc<AtomicBool>,
}

impl PartialEq for ContextMenuView {
    fn eq(&self, other: &Self) -> bool {
        self.wrap_layer == other.wrap_layer
    }
}

impl IsAlive for ContextMenuView {
    fn alive(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl ContextMenuView {
    /// Create a new context menu view
    pub fn new(base_layer: &Layer, items: Vec<MenuItem>) -> Self {
        let layers_engine = base_layer.engine.clone();
        let wrap = layers_engine.new_layer();
        wrap.set_key("context_menu_container");
        wrap.set_size(Size::percent(1.0, 1.0), None);
        wrap.set_layout_style(Style {
            position: taffy::style::Position::Absolute,
            display: taffy::style::Display::Flex,
            justify_content: Some(taffy::JustifyContent::FlexStart),
            align_items: Some(taffy::AlignItems::FlexStart),
            ..Default::default()
        });
        // wrap.set_pointer_events(true);
        let view_layer = layers_engine.new_layer();
        layers_engine.add_layer(&wrap);
        wrap.add_sublayer(&view_layer);
        // view_layer.set_pointer_events(true);

        view_layer.set_anchor_point((0.5, 1.0), None);
        let initial_state = ContextMenuState::new(items);
        let view = View::new("context_menu_inner", initial_state, Box::new(render_menu));
        view.mount_layer(view_layer.clone());

        base_layer.add_sublayer(&wrap);
        Self {
            wrap_layer: wrap,
            view_layer,
            view,
            active: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Show the context menu at the given position with a fade-in animation
    pub fn show_at(&self, x: f32, y: f32) {
        println!("Showing context menu at ({}, {})", x, y);
        let scale = Config::with(|c| c.screen_scale) as f32;
        self.view_layer.set_position(
            Point {
                x: x * scale,
                y: y * scale,
            },
            None,
        );
        self.active
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.wrap_layer.set_opacity(0.0, None);
        self.wrap_layer.set_hidden(false);
        self.wrap_layer.set_opacity(
            1.0,
            Some(Transition {
                delay: 0.0,
                timing: TimingFunction::ease_out_quad(0.15),
            }),
        );
    }

    /// Hide the context menu with a fade-out animation
    pub fn hide(&self) {
        self.active
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.wrap_layer
            .set_opacity(
                0.0,
                Some(Transition {
                    delay: 0.0,
                    timing: TimingFunction::ease_in_quad(0.05),
                }),
            )
            .on_finish(
                |l: &Layer, _p: f32| {
                    l.set_hidden(true);
                },
                true,
            );
    }

    /// Update the menu items and reset navigation state
    pub fn set_items(&self, items: Vec<MenuItem>) {
        let mut state = self.view.get_state();
        state.set_items(items);
        state.reset();
        self.view.update_state(&state);
    }

    /// Update the menu style
    pub fn set_style(&self, style: ContextMenuStyle) {
        let mut state = self.view.get_state();
        state.style = style;
        self.view.update_state(&state);
    }

    /// Select the next non-separator item at the current depth
    pub fn select_next(&self) {
        let mut state = self.view.get_state();
        state.select_next_at_depth(None);
        self.view.update_state(&state);
    }

    /// Select the previous non-separator item at the current depth
    pub fn select_previous(&self) {
        let mut state = self.view.get_state();
        state.select_previous_at_depth(None);
        self.view.update_state(&state);
    }

    /// Open the submenu for the currently selected item, returns true on success
    pub fn open_submenu(&self) -> bool {
        let mut state = self.view.get_state();
        let depth = state.depth();
        let has_submenu = state.selected_has_submenu(None);
        let selected_idx = state.selected_index(None);

        if has_submenu {
            if let Some(idx) = selected_idx {
                state.open_submenu(depth, idx);
                state.select_at_depth(depth + 1, Some(0));
                self.view.update_state(&state);
                return true;
            }
        }
        false
    }

    /// Close the current submenu level, returns true if a submenu was closed
    pub fn close_submenu(&self) -> bool {
        let mut state = self.view.get_state();
        let depth = state.depth();
        if depth > 0 {
            state.close_submenus_from(depth - 1);
            self.view.update_state(&state);
            return true;
        }
        false
    }

    /// Get the label of the currently selected item (if any)
    pub fn selected_label(&self) -> Option<String> {
        let state = self.view.get_state();
        state.selected_label(None).map(|s| s.to_string())
    }

    /// Reset navigation state (selection, submenus)
    pub fn reset(&self) {
        let mut state = self.view.get_state();
        state.reset();
        self.view.update_state(&state);
    }

    /// Whether the menu is currently visible
    pub fn is_active(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Current submenu depth (0 = root)
    pub fn depth(&self) -> usize {
        self.view.get_state().depth()
    }

    /// Flash the selected item at `depth`/`idx`, then send `label` on the returned receiver.
    ///
    /// Mirrors the client-side pulse: deselect (50 ms) → reselect (100 ms) → send.
    /// The caller should close the menu and execute the action when the receiver fires.
    /// Trigger a visual pulse on the selected item, then close the menu.
    /// The action should be executed immediately by the caller before calling this.
    pub fn pulse_then_close(
        &self,
        depth: usize,
        idx: usize,
        dock: crate::workspaces::dock::DockView,
    ) {
        let view = self.view.clone();
        tokio::spawn(async move {
            let mut state = view.get_state();
            state.select_at_depth(depth, None);
            view.update_state(&state);

            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

            let mut state = view.get_state();
            state.select_at_depth(depth, Some(idx));
            view.update_state(&state);

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            dock.close_context_menu();
        });
    }

    /// Trigger a visual pulse on the selected item (fire-and-forget).
    /// The action should be executed immediately by the caller; this only animates.
    pub fn pulse(&self, depth: usize, idx: usize) {
        let view = self.view.clone();
        tokio::spawn(async move {
            let mut state = view.get_state();
            state.select_at_depth(depth, None);
            view.update_state(&state);

            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

            let mut state = view.get_state();
            state.select_at_depth(depth, Some(idx));
            view.update_state(&state);

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        });
    }
}

/// Render function for `View<ContextMenuState>`
///
/// Produces a `LayerTree` with one layer per depth level, laid out horizontally.
fn render_menu(state: &ContextMenuState, _view: &View<ContextMenuState>) -> LayerTree {
    let draw_scale = Config::with(|c| c.screen_scale) as f32;
    let mut style = state.style.clone();
    style.draw_scale = draw_scale;

    let mut children = Vec::new();
    let mut x_offset = 0.0_f32;

    for depth in 0..=state.depth() {
        let items_vec = state.items_at_depth(depth).to_vec();
        if items_vec.is_empty() {
            continue;
        }

        let selected = state.selected_at_depth(depth);
        let (width, height) = ContextMenuRenderer::measure_items(&items_vec, &style);

        let items_for_closure = items_vec.clone();
        let style_for_closure = style.clone();

        let draw_fn = move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
            ContextMenuRenderer::render_depth(
                canvas,
                &items_for_closure,
                selected,
                &style_for_closure,
                w,
                h,
            );
            layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
        };

        let shadow_color = theme_colors().shadow_color;
        let depth_layer = LayerTreeBuilder::default()
            .key(format!("menu-depth-{}", depth))
            .position(Point::new(x_offset, 0.0))
            .size(Size::points(width, height))
            .opacity((
                1.0,
                Some(Transition {
                    delay: 0.05 * depth as f32,
                    timing: TimingFunction::ease_out_quad(0.2),
                }),
            ))
            .border_corner_radius(BorderRadius::new_single(style.corner_radius * draw_scale))
            .blend_mode(BlendMode::BackgroundBlur)
            .shadow_color(shadow_color)
            .shadow_offset(((0.0, 4.0 * draw_scale).into(), None))
            .shadow_radius((16.0 * draw_scale, None))
            .content(Some(draw_fn))
            .build()
            .unwrap();

        children.push(depth_layer);
        x_offset += width + 8.0 * draw_scale;
    }

    LayerTreeBuilder::default()
        .key("menu-container")
        .children(children)
        .build()
        .unwrap()
}
