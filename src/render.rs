use smithay::{
    backend::renderer::{
        damage::{Error as OutputDamageTrackerError, OutputDamageTracker, RenderOutputResult},
        element::{
            self,
            surface::render_elements_from_surface_tree,
            utils::{
                ConstrainAlign, ConstrainScaleBehavior, CropRenderElement, RelocateRenderElement,
                RescaleRenderElement,
            },
        },
        ImportAll, ImportMem, Renderer, RendererSuper,
    },
    desktop::space::{constrain_space_element, ConstrainBehavior, ConstrainReference, Space},
    output::Output,
    reexports::wayland_server::protocol::wl_surface,
    utils::{self, Point, Rectangle, Size},
};

use crate::{
    drawing::{PointerRenderElement, CLEAR_COLOR},
    render_elements::{output_render_elements::OutputRenderElements, scene_element::SceneElement},
    shell::{WindowElement, WindowRenderElement},
};

pub fn space_preview_elements<'a, R, C>(
    renderer: &'a mut R,
    space: &'a Space<WindowElement>,
    output: &'a Output,
) -> impl Iterator<Item = C> + 'a
where
    R: Renderer + ImportAll + ImportMem,
    <R as RendererSuper>::TextureId: Clone + 'static,
    C: From<CropRenderElement<RelocateRenderElement<RescaleRenderElement<WindowRenderElement<R>>>>>
        + 'a,
{
    let constrain_behavior = ConstrainBehavior {
        reference: ConstrainReference::BoundingBox,
        behavior: ConstrainScaleBehavior::Fit,
        align: ConstrainAlign::CENTER,
    };

    let preview_padding = 10;

    let elements_on_space = space.elements_for_output(output).count();
    let output_scale = output.current_scale().fractional_scale();
    let output_transform = output.current_transform();
    let output_size = output
        .current_mode()
        .map(|mode| {
            output_transform
                .transform_size(mode.size)
                .to_f64()
                .to_logical(output_scale)
        })
        .unwrap_or_default();

    let max_elements_per_row = 4;
    let elements_per_row = usize::min(elements_on_space, max_elements_per_row);
    let rows = f64::ceil(elements_on_space as f64 / elements_per_row as f64);

    let preview_size = Size::from((
        f64::round(output_size.w / elements_per_row as f64) as i32 - preview_padding * 2,
        f64::round(output_size.h / rows) as i32 - preview_padding * 2,
    ));

    space
        .elements_for_output(output)
        .enumerate()
        .flat_map(move |(element_index, window)| {
            let column = element_index % elements_per_row;
            let row = element_index / elements_per_row;
            let preview_location = Point::from((
                preview_padding + (preview_padding + preview_size.w) * column as i32,
                preview_padding + (preview_padding + preview_size.h) * row as i32,
            ));
            let constrain = Rectangle::new(preview_location, preview_size);
            constrain_space_element(
                renderer,
                window,
                preview_location,
                1.0,
                output_scale,
                constrain,
                constrain_behavior,
            )
        })
}

#[profiling::function]
pub fn output_elements<'a, 'frame, R>(
    _output: &Output,
    window_elements: impl IntoIterator<Item = &'a WindowElement>,
    workspace_elements: impl IntoIterator<
        Item = impl Into<OutputRenderElements<'frame, R, WindowRenderElement<R>>>,
    >,
    dnd: Option<&wl_surface::WlSurface>,
    renderer: &mut R,
) -> (
    Vec<OutputRenderElements<'frame, R, WindowRenderElement<R>>>,
    [f32; 4],
)
where
    R: Renderer + ImportAll + ImportMem,
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    let mut output_render_elements = Vec::new();
    let _dnd_element = dnd.map(|dnd| {
        let location: utils::Point<i32, utils::Physical> = (0_i32, 0_i32).into();
        let _pointer_element = render_elements_from_surface_tree::<R, PointerRenderElement<R>>(
            renderer,
            dnd,
            location,
            1.0,
            1.0,
            element::Kind::Unspecified,
        );
    });

    // Render window elements from all workspaces - but skip rendering for now
    // as this needs to be handled by the workspace scene elements
    let _window_count = window_elements.into_iter().count();

    output_render_elements.extend(workspace_elements.into_iter().map(|e| e.into()));

    (output_render_elements, CLEAR_COLOR)
}

#[allow(clippy::too_many_arguments)]
pub fn render_output<'frame, R>(
    output: &Output,
    window_elements: &[&WindowElement],
    custom_elements: impl IntoIterator<
        Item = impl Into<OutputRenderElements<'frame, R, WindowRenderElement<R>>>,
    >,
    dnd: Option<&wl_surface::WlSurface>,
    renderer: &mut R,
    framebuffer: &mut <R as RendererSuper>::Framebuffer<'frame>,
    damage_tracker: &'frame mut OutputDamageTracker,
    age: usize,
) -> Result<RenderOutputResult<'frame>, OutputDamageTrackerError<<R as RendererSuper>::Error>>
where
    R: Renderer + ImportAll + ImportMem + 'frame,
    <R as RendererSuper>::TextureId: Clone + 'static,
    <R as RendererSuper>::Error: std::error::Error,
    SceneElement: smithay::backend::renderer::element::RenderElement<R>,
{
    let (elements, clear_color) = output_elements(
        output,
        window_elements.iter().copied(),
        custom_elements,
        dnd,
        renderer,
    );

    let result = damage_tracker.render_output(renderer, framebuffer, age, &elements, clear_color);

    result
}
