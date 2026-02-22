use std::{
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
};

use layers::prelude::*;
use smithay::{
    backend::input::ButtonState,
    input::pointer::{CursorIcon, CursorImageStatus},
};

use crate::{
    config::Config,
    interactive_view::ViewInteractions,
    theme::{self, theme_colors},
    utils::{
        button_press_filter, button_press_scale, button_release_filter, button_release_scale,
        draw_named_icon, draw_text_content, Observer,
    },
};

use super::WorkspacesModel;

pub const WORKSPACE_SELECTOR_PREVIEW_WIDTH: f32 = 300.0;

#[derive(Clone, Debug)]
pub struct WorkspaceDropTarget {
    pub workspace_index: usize,
    pub drop_layer: layers::prelude::Layer,
}

#[derive(Clone, Debug)]
pub struct WorkspaceViewState {
    name: String,
    index: usize,
    workspace_node: Option<NodeRef>,
    workspace_width: f32,
    workspace_height: f32,
    fullscreen: bool,
    window_count: usize,
}

impl Hash for WorkspaceViewState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.index.hash(state);
        self.workspace_node.hash(state);
        self.workspace_width.to_bits().hash(state);
        self.workspace_height.to_bits().hash(state);
        self.fullscreen.hash(state);
        self.window_count.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceSelectorViewState {
    workspaces: Vec<WorkspaceViewState>,
    current: usize,
    drop_hover_index: Option<usize>,
}

impl Hash for WorkspaceSelectorViewState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.workspaces.hash(state);
        self.current.hash(state);
        self.drop_hover_index.hash(state);
    }
}

#[derive(Clone)]
pub struct WorkspaceSelectorView {
    pub layer: layers::prelude::Layer,
    pub view: layers::prelude::View<WorkspaceSelectorViewState>,
    pub cursor_location: Arc<RwLock<Point>>,
    pub drop_targets: Arc<RwLock<Vec<WorkspaceDropTarget>>>,
    pub drop_hover_index: Arc<RwLock<Option<usize>>>,
    pressed_action: Arc<RwLock<Option<String>>>,
}

/// # WorkspaceSelectorView Layer Structure
///
/// ```diagram
/// WorkspaceSelectorView
/// ├── layer (view(render_workspace_selector_view))
/// │   ├── workspace_selector_view_content
/// │   │   └── workspace_selector_desktop_{x}
/// │   │       └── workspace_selector_desktop_content_{x}
/// │   │           ├── workspace_desktop_content_preview_{x}
/// │   │           └── workspace_selector_desktop_remove_{x}
/// │   └── workspace_selector_add
/// ```
///
/// - `layer`: The root layer for the window selector view.
///
///
impl WorkspaceSelectorView {
    pub fn new(_layers_engine: Arc<Engine>, layer: Layer) -> Self {
        let state = WorkspaceSelectorViewState {
            workspaces: Vec::new(),
            current: 0,
            drop_hover_index: None,
        };
        let view = View::new(
            "workspace_selector_view",
            state,
            render_workspace_selector_view,
        );
        layer.set_pointer_events(false);
        layer.set_position((0.0, -250.0), None);
        layer.set_opacity(0.0, None);
        view.set_layer(layer.clone());
        layer.set_hidden(true);
        
        let drop_targets = Arc::new(RwLock::new(Vec::new()));
        let drop_hover_index = Arc::new(RwLock::new(None));
        let pressed_action = Arc::new(RwLock::new(None));

        // Setup post-render hook to update drop targets
        let drop_targets_clone = drop_targets.clone();
        view.add_post_render_hook(move |state, view, _layer| {
            let targets: Vec<WorkspaceDropTarget> = state
                .workspaces
                .iter()
                .filter_map(|w| {
                    let key = format!("workspace_selector_desktop_content_{}", w.index);
                    view.layer_by_key(&key).map(|layer| WorkspaceDropTarget {
                        workspace_index: w.index,
                        drop_layer: layer.clone(),
                    })
                })
                .collect();
            *drop_targets_clone.write().unwrap() = targets;
        });

        Self {
            // engine: layers_engine,
            layer,
            view,
            cursor_location: Arc::new(RwLock::new(Point::default())),
            drop_targets,
            drop_hover_index,
            pressed_action,
        }
    }

    /// Get current drop targets (updated after each render)
    pub fn get_drop_targets(&self) -> Vec<WorkspaceDropTarget> {
        self.drop_targets.read().unwrap().clone()
    }

    /// Set which workspace is being hovered during drag (for visual feedback)
    pub fn set_drop_hover(&self, workspace_index: Option<usize>) {
        *self.drop_hover_index.write().unwrap() = workspace_index;

        // Update view state to trigger re-render with new hover indication
        let mut state = self.view.get_state().clone();
        state.drop_hover_index = workspace_index;
        self.view.update_state(&state);
    }

    /// Get the currently hovered workspace index
    pub fn get_drop_hover(&self) -> Option<usize> {
        *self.drop_hover_index.read().unwrap()
    }
}

fn render_workspace_selector_view(
    state: &WorkspaceSelectorViewState,
    view: &View<WorkspaceSelectorViewState>,
) -> LayerTree {
    let worspaces = state.workspaces.clone();

    let workspaces_tree = worspaces
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let workspace_index = w.index;
            let current = i == state.current;
            let mut state_drop_hover_index: i32 = -1;
            if state.drop_hover_index.is_some() {
                state_drop_hover_index = state.drop_hover_index.unwrap() as i32;
            }
            let is_drop_hover = state_drop_hover_index - 1 == (i as i32) && !current;

            let mut border_width = 0.0;
            let border_color = crate::theme::accent_color();

            if current {
                border_width = 8.0;
            }
            let mut color_filter = None;
            if is_drop_hover {
                let darken_color = layers::skia::Color::from_argb(100, 100, 100, 100);
                let add = layers::skia::Color::from_argb(0, 0, 0, 0);
                color_filter = layers::skia::color_filters::lighting(darken_color, add);
            }

            let workspace_width = w.workspace_width.max(1.0);
            let workspace_height = w.workspace_height.max(1.0);
            let preview_width = WORKSPACE_SELECTOR_PREVIEW_WIDTH;
            let scale = preview_width / workspace_width;
            let preview_height = workspace_height * scale;

            LayerTreeBuilder::with_key(format!(
                "workspace_selector_desktop_{}",
                workspace_index.clone()
            ))
            .layout_style(taffy::Style {
                display: taffy::Display::Flex,
                position: taffy::Position::Relative,
                flex_direction: taffy::FlexDirection::Column,
                align_items: Some(taffy::AlignItems::Center),
                justify_content: Some(taffy::AlignContent::Center),
                gap: taffy::Size::length(20.0),
                ..Default::default()
            })
            .size((
                layers::types::Size {
                    width: layers::taffy::style::Dimension::Length(preview_width),
                    height: layers::taffy::style::Dimension::Auto,
                },
                None,
            ))
            .children(vec![
                LayerTreeBuilder::with_key(format!(
                    "workspace_selector_desktop_content_{}",
                    workspace_index.clone()
                ))
                .layout_style(taffy::Style {
                    position: taffy::Position::Relative,
                    ..Default::default()
                })
                .size((
                    layers::types::Size {
                        width: layers::taffy::style::Dimension::Length(preview_width),
                        height: layers::taffy::style::Dimension::Length(preview_height),
                    },
                    None,
                ))
                .on_pointer_move({
                    let view_ref = view.clone();
                    move |_layer: &Layer, _x, _y| {
                        let key = format!("workspace_selector_desktop_remove_{}", workspace_index);
                        if let Some(remove_button) = view_ref.layer_by_key(key.as_str()) {
                            remove_button.set_opacity(1.0, Transition::spring(0.3, 0.1));
                            remove_button
                                .set_scale(Point::new(1.0, 1.0), Transition::spring(0.3, 0.1));
                        }
                    }
                })
                .on_pointer_out({
                    let view_ref = view.clone();
                    move |_layer: &Layer, _x, _y| {
                        let key = format!("workspace_selector_desktop_remove_{}", workspace_index);
                        if let Some(remove_button) = view_ref.layer_by_key(key.as_str()) {
                            remove_button.set_opacity(0.0, Transition::spring(0.3, 0.1));
                            remove_button
                                .set_scale(Point::new(0.8, 0.8), Transition::spring(0.3, 0.1));
                        }
                    }
                })
                .children::<LayerTree>({
                    let children: Vec<Option<LayerTree>> = vec![
                        Some(
                            LayerTreeBuilder::with_key(format!(
                                "workspace_selector_desktop_content_mirror_{}",
                                workspace_index.clone()
                            ))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .size((
                                layers::types::Size {
                                    width: layers::taffy::style::Dimension::Length(workspace_width),
                                    height: layers::taffy::style::Dimension::Length(
                                        workspace_height,
                                    ),
                                },
                                None,
                            ))
                            .scale(Point::new(scale, scale))
                            .replicate_node(w.workspace_node)
                            .picture_cached(true)
                            .image_cache(true)
                            .color_filter(color_filter)
                            .border_corner_radius(BorderRadius::new_single(20.0 / scale))
                            .clip_children(true)
                            .clip_content(true)
                            .pointer_events(true)
                            .on_pointer_press(button_press_filter())
                            .on_pointer_release(button_release_filter())
                            .on_pointer_out(button_release_filter())
                            .build()
                            .unwrap(),
                        ),
                        Some(
                            LayerTreeBuilder::with_key(format!(
                                "workspace_selector_desktop_border_{}",
                                w.index
                            ))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .position(Point::new(0.0, 0.0))
                            .size((
                                layers::types::Size {
                                    width: layers::taffy::style::Dimension::Percent(1.0),
                                    height: layers::taffy::style::Dimension::Percent(1.0),
                                },
                                None,
                            ))
                            .border_width((border_width, None))
                            .border_color(border_color)
                            .border_corner_radius(BorderRadius::new_single(20.0))
                            .pointer_events(false)
                            .build()
                            .unwrap(),
                        ),
                        // Only show remove button if not current workspace and not a non-empty fullscreen workspace
                        (!(current || w.fullscreen && w.window_count > 0)).then(|| -> LayerTree {
                            LayerTreeBuilder::with_key(format!(
                                "workspace_selector_desktop_remove_{}",
                                w.index
                            ))
                            .layout_style(taffy::Style {
                                position: taffy::Position::Absolute,
                                ..Default::default()
                            })
                            .anchor_point(Point::new(0.5, 0.5))
                            .scale(Point::new(0.2, 0.2))
                            .opacity((0.0, None))
                            .position(Point::new(preview_width, 0.0))
                            .size((
                                layers::types::Size {
                                    width: layers::taffy::style::Dimension::Length(50.0),
                                    height: layers::taffy::style::Dimension::Length(50.0),
                                },
                                None,
                            ))
                            .background_color(theme_colors().materials_ultrathick)
                            .blend_mode(BlendMode::BackgroundBlur)
                            .border_corner_radius(BorderRadius::new_single(25.0))
                            .content(draw_named_icon("close-symbolic"))
                            .shadow_color((Color::new_rgba(0.0, 0.0, 0.0, 0.2), None))
                            .shadow_offset(((0.0, 0.0).into(), None))
                            .shadow_radius((5.0, None))
                            .image_cache(true)
                            .on_pointer_press(button_press_scale(0.9))
                            .on_pointer_release(button_release_scale())
                            .build()
                            .unwrap()
                        }),
                    ];
                    children
                })
                .build()
                .unwrap(),
                LayerTreeBuilder::with_key(format!("workspace_selector_desktop_label_{}", w.index))
                    .layout_style(taffy::Style {
                        position: taffy::Position::Relative,
                        ..Default::default()
                    })
                    .size((
                        layers::types::Size {
                            width: layers::taffy::style::Dimension::Percent(1.0),
                            height: layers::taffy::style::Dimension::Length(40.0),
                        },
                        None,
                    ))
                    // .background_color(theme_colors().accents_purple)
                    .content(draw_text_content(
                        w.name.clone(),
                        theme::text_styles::title_3_regular(),
                        layers::skia::textlayout::TextAlign::Center,
                    ))
                    .build()
                    .unwrap(),
            ])
            .build()
            .unwrap()
        })
        .collect();
    LayerTreeBuilder::with_key("workspace_selector_view")
        .layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            display: taffy::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            ..Default::default()
        })
        .size((
            layers::types::Size {
                width: layers::taffy::style::Dimension::Percent(1.0),
                height: layers::taffy::style::Dimension::Auto,
            },
            None,
        ))
        .background_color(theme_colors().materials_medium)
        .blend_mode(BlendMode::BackgroundBlur)
        .shadow_color(theme_colors().shadow_color)
        .shadow_offset(((0.0, -5.0).into(), None))
        .shadow_radius((20.0, None))
        .children(vec![
            LayerTreeBuilder::with_key("workspace_selector_view_content")
                .layout_style(taffy::Style {
                    display: taffy::Display::Flex,
                    flex_direction: taffy::FlexDirection::Row,
                    align_items: Some(taffy::AlignItems::Center),
                    justify_content: Some(taffy::AlignContent::Center),
                    gap: taffy::length(50.0),
                    padding: taffy::Rect {
                        bottom: taffy::length(20.0),
                        top: taffy::length(30.0),
                        left: taffy::length(10.0),
                        right: taffy::length(10.0),
                    },
                    ..Default::default()
                })
                .size((
                    layers::types::Size {
                        width: layers::taffy::style::Dimension::Percent(1.0),
                        height: layers::taffy::style::Dimension::Auto,
                    },
                    None,
                ))
                .children(workspaces_tree)
                .build()
                .unwrap(),
            LayerTreeBuilder::default()
                .key("workspace_selector_desktop_add")
                .layout_style(taffy::Style {
                    ..Default::default()
                })
                .size((
                    layers::types::Size {
                        width: layers::taffy::style::Dimension::Length(80.0),
                        height: layers::taffy::style::Dimension::Length(80.0),
                    },
                    None,
                ))
                .content(draw_named_icon("plus-symbolic"))
                .image_cache(true)
                .on_pointer_press(button_press_scale(0.9))
                .on_pointer_release(button_release_scale())
                .build()
                .unwrap(),
        ])
        .build()
        .unwrap()
}

impl Observer<WorkspacesModel> for WorkspaceSelectorView {
    fn notify(&self, model: &WorkspacesModel) {
        let mut state = self.view.get_state();
        state.workspaces = model
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, w)| WorkspaceViewState {
                name: w
                    .get_name()
                    .unwrap_or_else(|| format!("Workspace {}", i + 1)),
                index: w.index,
                workspace_node: Some(w.workspace_layer.id()),
                workspace_width: model.width as f32,
                workspace_height: model.height as f32,
                fullscreen: w.get_fullscreen_mode(),
                window_count: w.windows_list.read().unwrap().len(),
            })
            .collect();
        state.current = model.current_workspace;
        self.view.update_state(&state);
    }
}

impl<Backend: crate::state::Backend> ViewInteractions<Backend> for WorkspaceSelectorView {
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
        data: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        let state = self.view.get_state().clone();
        let screen_scale = Config::with(|config| config.screen_scale);
        let location = event.location.to_physical(screen_scale);
        let location = layers::types::Point::new(location.x as f32, location.y as f32);
        let mut hover = false;
        if self
            .view
            .hover_layer("workspace_selector_desktop_add", &location)
        {
            hover = true;
        }
        for w in state.workspaces.iter() {
            if self.view.hover_layer(
                &format!("workspace_selector_desktop_{}", w.index),
                &location,
            ) {
                hover = true;
                break;
            }
            if self.view.hover_layer(
                &format!("workspace_selector_desktop_remove_{}", w.index),
                &location,
            ) {
                hover = true;
                break;
            }
        }

        if hover {
            let cursor = CursorImageStatus::Named(CursorIcon::Pointer);
            data.set_cursor(&cursor);
        } else {
            let cursor = CursorImageStatus::Named(CursorIcon::default());
            data.set_cursor(&cursor);
        }
        let mut cursor_location = self.cursor_location.write().unwrap();
        *cursor_location = location;
    }
    fn on_button(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        otto: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        let location = self.cursor_location.read().unwrap();
        let state = self.view.get_state().clone();
        let get_position_worspace_by_index = |index: usize| -> Option<usize> {
            state.workspaces.iter().position(|w| w.index == index)
        };
        let hovered_key = |loc: &Point| -> Option<String> {
            // check add first so it has priority over overlaps
            if self.view.hover_layer("workspace_selector_desktop_add", loc) {
                return Some("workspace_selector_desktop_add".to_string());
            }

            for w in state.workspaces.iter() {
                let remove_key = format!("workspace_selector_desktop_remove_{}", w.index);
                if self.view.hover_layer(&remove_key, loc) {
                    return Some(remove_key);
                }

                let workspace_key = format!("workspace_selector_desktop_{}", w.index);
                if self.view.hover_layer(&workspace_key, loc) {
                    return Some(workspace_key);
                }
            }
            None
        };

        match event.state {
            ButtonState::Pressed => {
                let mut pressed = self.pressed_action.write().unwrap();
                *pressed = hovered_key(&location);
            }
            ButtonState::Released => {
                let release_key = hovered_key(&location);
                let mut pressed = self.pressed_action.write().unwrap();
                if let (Some(pressed_key), Some(release_key)) = (pressed.clone(), release_key) {
                    if pressed_key == release_key {
                        if release_key == "workspace_selector_desktop_add" {
                            otto.workspaces.add_workspace();
                        } else if let Some(index) = release_key
                            .strip_prefix("workspace_selector_desktop_remove_")
                            .and_then(|idx| idx.parse::<usize>().ok())
                        {
                            if let Some(pos) = get_position_worspace_by_index(index) {
                                otto.workspaces.remove_workspace_at(pos);
                            }
                        } else if let Some(index) = release_key
                            .strip_prefix("workspace_selector_desktop_")
                            .and_then(|idx| idx.parse::<usize>().ok())
                        {
                            if let Some(pos) = get_position_worspace_by_index(index) {
                                otto.set_current_workspace_index(pos);
                            }
                        }
                    }
                }
                *pressed = None;
            }
        }
    }
}
