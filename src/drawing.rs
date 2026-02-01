#![allow(clippy::too_many_arguments)]

use smithay::{
    backend::renderer::{
        element::surface::WaylandSurfaceRenderElement, ImportAll, ImportMem, Renderer,
    },
    render_elements,
};
#[cfg(feature = "fps_ticker")]
use smithay::{
    backend::renderer::{
        element::{Element, Id, RenderElement},
        utils::CommitCounter,
        Frame,
    },
    utils::{Buffer, Logical, Rectangle, Size, Transform},
};

pub static CLEAR_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
pub static CLEAR_COLOR_FULLSCREEN: [f32; 4] = [0.0, 0.0, 0.0, 0.0];

// Old PointerElement removed - cursor rendering now handled directly in backends using CursorManager

render_elements! {
    pub PointerRenderElement<R> where
        R: ImportAll + ImportMem;
    Surface=WaylandSurfaceRenderElement<R>,
    Memory=smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement<R>,
}

impl<R: Renderer> std::fmt::Debug for PointerRenderElement<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Surface(arg0) => f.debug_tuple("Surface").field(arg0).finish(),
            Self::Memory(arg0) => f.debug_tuple("Memory").field(arg0).finish(),
            Self::_GenericCatcher(arg0) => f.debug_tuple("_GenericCatcher").field(arg0).finish(),
        }
    }
}

#[cfg(feature = "fps_ticker")]
pub static FPS_NUMBERS_PNG: &[u8] = include_bytes!("../resources/numbers.png");

#[cfg(feature = "fps_ticker")]
#[derive(Debug, Clone)]
pub struct FpsElement<T: Texture> {
    id: Id,
    value: u32,
    texture: T,
    commit_counter: CommitCounter,
}

#[cfg(feature = "fps_ticker")]
impl<T: Texture> FpsElement<T> {
    pub fn new(texture: T) -> Self {
        FpsElement {
            id: Id::new(),
            texture,
            value: 0,
            commit_counter: CommitCounter::default(),
        }
    }

    pub fn update_fps(&mut self, fps: u32) {
        if self.value != fps {
            self.value = fps;
            self.commit_counter.increment();
        }
    }
}

#[cfg(feature = "fps_ticker")]
impl<T> Element for FpsElement<T>
where
    T: Texture + 'static,
{
    fn id(&self) -> &Id {
        &self.id
    }

    fn location(&self, _scale: Scale<f64>) -> Point<i32, Physical> {
        (0, 0).into()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        let digits = if self.value < 10 {
            1
        } else if self.value < 100 {
            2
        } else {
            3
        };
        Rectangle::new((0, 0).into(), (24 * digits, 35).into()).to_f64()
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        let digits = if self.value < 10 {
            1
        } else if self.value < 100 {
            2
        } else {
            3
        };
        Rectangle::new((0, 0).into(), (24 * digits, 35).into()).to_physical_precise_round(1.0)
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit_counter
    }
}

#[cfg(feature = "fps_ticker")]
impl<R> RenderElement<R> for FpsElement<<R as RendererSuper>::TextureId>
where
    R: Renderer + ImportAll,
    <R as RendererSuper>::TextureId: 'static,
{
    fn draw(
        &self,
        frame: &mut <R as RendererSuper>::Frame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), R::Error> {
        // FIXME: respect the src for cropping
        let scale = dst.size.to_f64() / self.src().size;
        let value_str = std::cmp::min(self.value, 999).to_string();
        let mut offset: Point<f64, Physical> = Point::from((0.0, 0.0));
        for digit in value_str.chars().map(|d| d.to_digit(10).unwrap()) {
            let digit_location = dst.loc.to_f64() + offset;
            let digit_size = Size::<i32, Logical>::from((22, 35))
                .to_f64()
                .to_physical(scale);
            let dst = Rectangle::new(
                digit_location.to_i32_round().into(),
                ((digit_size.to_point() + digit_location).to_i32_round()
                    - digit_location.to_i32_round())
                .to_size(),
            );
            let damage = damage
                .iter()
                .cloned()
                .flat_map(|x| x.intersection(dst))
                .map(|mut x| {
                    x.loc -= dst.loc;
                    x
                })
                .collect::<Vec<_>>();
            let texture_src: Rectangle<i32, Buffer> = match digit {
                9 => Rectangle::new((0, 0).into(), (22, 35).into()),
                6 => Rectangle::new((22, 0).into(), (22, 35).into()),
                3 => Rectangle::new((44, 0).into(), (22, 35).into()),
                1 => Rectangle::new((66, 0).into(), (22, 35).into()),
                8 => Rectangle::new((0, 35).into(), (22, 35).into()),
                0 => Rectangle::new((22, 35).into(), (22, 35).into()),
                2 => Rectangle::new((44, 35).into(), (22, 35).into()),
                7 => Rectangle::new((0, 70).into(), (22, 35).into()),
                4 => Rectangle::new((22, 70).into(), (22, 35).into()),
                5 => Rectangle::new((44, 70).into(), (22, 35).into()),
                _ => unreachable!(),
            };

            frame.render_texture_from_to(
                &self.texture,
                texture_src.to_f64(),
                dst,
                &damage,
                &[],
                Transform::Normal,
                1.0,
            )?;
            offset += Point::from((24.0, 0.0)).to_physical(scale);
        }

        Ok(())
    }
}
