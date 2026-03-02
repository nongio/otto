use layers::{
    engine::NodeRef,
    prelude::{taffy, Layer, LayerTree, LayerTreeBuilder, View},
    taffy::prelude::FromLength,
    types::{BlendMode, BorderRadius, PaintColor, Size},
};

use crate::workspaces::dock::BASE_ICON_SIZE;

use super::{
    model::AppSwitcherModel,
    render::{draw_appswitcher_overlay, layout_metrics},
};

/// Build the full app-switcher layer tree from the current model state.
///
/// The panel and its children are rebuilt declaratively; the `View` engine
/// diffs the tree and only applies changed properties, so this is efficient.
pub fn render_appswitcher(state: &AppSwitcherModel, view: &View<AppSwitcherModel>) -> LayerTree {
    let (w, h, available_icon_size, icon_padding, gap, _, _) = layout_metrics(state);
    let slot_size = available_icon_size + icon_padding * 2.0;

    let apps_children: Vec<LayerTree> = state
        .apps
        .iter()
        .zip(state.icon_stacks.iter())
        .enumerate()
        .filter_map(|(index, (app, node_ref))| {
            let node_ref: NodeRef = (*node_ref)?;
            let identifier = app.identifier.clone();
            let view_ref = view.clone();
            let scale = if BASE_ICON_SIZE > 0.0 {
                (slot_size * 0.8) / BASE_ICON_SIZE
            } else {
                1.0
            };
            let wrap = LayerTreeBuilder::with_key(format!("switcher_wrap_{}", identifier))
                .layout_style(taffy::Style {
                    size: taffy::Size {
                        width: taffy::Dimension::Length(slot_size),
                        height: taffy::Dimension::Length(slot_size),
                    },
                    ..Default::default()
                })
                .children(vec![LayerTreeBuilder::with_key(format!(
                    "switcher_mirror_{}",
                    identifier
                ))
                .layout_style(taffy::Style {
                    position: taffy::Position::Absolute,
                    size: taffy::Size {
                        width: taffy::Dimension::Length(BASE_ICON_SIZE),
                        height: taffy::Dimension::Length(BASE_ICON_SIZE),
                    },
                    ..Default::default()
                })
                .opacity((1.0, None))
                .position(layers::prelude::Point {
                    x: slot_size / 2.0,
                    y: slot_size / 2.0,
                })
                .anchor_point(layers::prelude::Point { x: 0.5, y: 0.5 })
                .scale(layers::prelude::Point::new(scale, scale))
                .replicate_node(Some(node_ref))
                .picture_cached(false)
                .on_pointer_in(move |_: &Layer, _x, _y| {
                    view_ref.update_state(&AppSwitcherModel {
                        current_app: index,
                        ..view_ref.get_state()
                    });
                })
                .build()
                .unwrap()])
                .build()
                .unwrap();
            Some(wrap)
        })
        .collect();

    LayerTreeBuilder::with_key("appswitcher_apps_container")
        .layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            display: taffy::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            gap: taffy::Size::<taffy::LengthPercentage>::from_length(gap),
            size: taffy::Size {
                width: taffy::Dimension::Length(w),
                height: taffy::Dimension::Length(h),
            },
            ..Default::default()
        })
        .pointer_events(false)
        .background_color(PaintColor::Solid {
            color: layers::prelude::Color::new_rgba(0.0, 0.0, 0.0, 0.0),
        })
        .children(apps_children)
        .build()
        .unwrap()
}

/// Build a panel LayerTree that wraps the apps container with the blur background
/// and selection overlay drawing.
pub fn render_appswitcher_panel(
    state: &AppSwitcherModel,
    view: &View<AppSwitcherModel>,
) -> LayerTree {
    let (w, h, _, _, _, _, _) = layout_metrics(state);
    let overlay = draw_appswitcher_overlay(state);
    let apps_tree = render_appswitcher(state, view);

    LayerTreeBuilder::with_key("appswitcher_panel")
        .blend_mode(BlendMode::BackgroundBlur)
        .background_color(PaintColor::Solid {
            color: crate::theme::theme_colors().materials_thin,
        })
        .size((
            Size::points(w, h),
            Some(layers::engine::animation::Transition::spring(0.4, 0.0)),
        ))
        .border_corner_radius((BorderRadius::new_single(h / 8.0), None))
        .layout_style(taffy::Style {
            display: taffy::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            ..Default::default()
        })
        .content(Some(overlay))
        .pointer_events(false)
        .children(vec![apps_tree])
        .build()
        .unwrap()
}
