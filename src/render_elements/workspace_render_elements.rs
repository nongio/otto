use smithay::{
    backend::renderer::{
        element::{
            memory::MemoryRenderBufferRenderElement, surface::WaylandSurfaceRenderElement, Element,
            Id, RenderElement,
        },
        utils::{CommitCounter, DamageSet},
        ImportAll, ImportMem,
    },
    utils::{Physical, Rectangle, Scale},
};

use crate::drawing::PointerRenderElement;

use super::scene_element::SceneElement;

#[cfg(feature = "fps_ticker")]
use crate::drawing::FpsElement;

smithay::backend::renderer::element::render_elements! {
    pub WorkspaceRenderElements<'a, R> where
        R: ImportAll + ImportMem + 'a,
        SceneElement: (RenderElement<R>);
    Pointer=PointerRenderElement<R>,
    Cursor=MemoryRenderBufferRenderElement<R>,
    Surface=WaylandSurfaceRenderElement<R>,
    Scene=SceneElement,
    // this is needed to make the macro work with a lifetime specifier in the where clauses
    PhantomElement=PhantomElement<'a>,
    #[cfg(feature = "fps_ticker")]
    Fps=FpsElement<<R as smithay::backend::renderer::Renderer>::TextureId>,
}

// this is needed to make the macro work with a lifetime specifier in the where clauses
pub struct PhantomElement<'a> {
    id: Id,
    commit_counter: CommitCounter,
    phantom: std::marker::PhantomData<&'a ()>,
}

impl Element for PhantomElement<'_> {
    fn id(&self) -> &Id {
        &self.id
    }

    fn location(
        &self,
        _scale: smithay::utils::Scale<f64>,
    ) -> smithay::utils::Point<i32, smithay::utils::Physical> {
        (0, 0).into()
    }

    fn src(&self) -> smithay::utils::Rectangle<f64, smithay::utils::Buffer> {
        smithay::utils::Rectangle::new((0, 0).into(), (0, 0).into()).to_f64()
    }

    fn geometry(
        &self,
        scale: smithay::utils::Scale<f64>,
    ) -> smithay::utils::Rectangle<i32, smithay::utils::Physical> {
        smithay::utils::Rectangle::new(self.location(scale), (0, 0).into())
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit_counter
    }
    /// Get the damage since the provided commit relative to the element
    fn damage_since(
        &self,
        scale: Scale<f64>,
        _commit: Option<CommitCounter>,
    ) -> smithay::backend::renderer::utils::DamageSet<i32, Physical> {
        DamageSet::from_slice(&[Rectangle::new((0, 0).into(), self.geometry(scale).size)])
    }
    fn alpha(&self) -> f32 {
        0.0
    }
}

#[allow(clippy::extra_unused_lifetimes)]
impl<'renderer, 'alloc, R> RenderElement<R> for PhantomElement<'renderer>
where
    R: smithay::backend::renderer::Renderer + 'renderer,
{
    fn draw(
        &self,
        _frame: &mut <R as smithay::backend::renderer::RendererSuper>::Frame<'_, '_>,
        _src: smithay::utils::Rectangle<f64, smithay::utils::Buffer>,
        _dst: smithay::utils::Rectangle<i32, smithay::utils::Physical>,
        _damage: &[smithay::utils::Rectangle<i32, smithay::utils::Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <R as smithay::backend::renderer::RendererSuper>::Error> {
        Ok(())
    }
}
