use smithay::{
    backend::renderer::{
        damage::{Error as OutputDamageTrackerError, OutputDamageTracker, RenderOutputResult},
        element::{self, surface::render_elements_from_surface_tree},
        ImportAll, ImportMem, Renderer, RendererSuper,
    },
    output::Output,
    reexports::wayland_server::protocol::wl_surface,
    utils,
};

use crate::{
    drawing::{PointerRenderElement, CLEAR_COLOR},
    render_elements::{output_render_elements::OutputRenderElements, scene_element::SceneElement},
    shell::{WindowElement, WindowRenderElement},
};

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
    crate::render_elements::scene_dmabuf_element::SceneDmabufElement:
        smithay::backend::renderer::element::RenderElement<R>,
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
