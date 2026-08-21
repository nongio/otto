use std::sync::{Arc, RwLock};

use layers::{prelude::Layer, skia};

#[derive(Clone)]
pub struct GenieEffect {
    pub genie_builder: Arc<RwLock<skia::runtime_effect::RuntimeShaderBuilder>>,
    layer: Arc<RwLock<Option<Layer>>>,
    src: Arc<RwLock<skia::Rect>>,
    pub dst: Arc<RwLock<skia::Rect>>,
    offset: Arc<RwLock<(f32, f32)>>,
    progress: Arc<RwLock<f32>>,
}
impl Default for GenieEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl GenieEffect {
    pub fn new() -> Self {
        let sksl = include_str!("./genie.sksl");

        let runtime_effect = skia::RuntimeEffect::make_for_shader(sksl, None).unwrap();

        let mut builder = skia::runtime_effect::RuntimeShaderBuilder::new(runtime_effect);
        // Vertical (dock at the bottom) unless told otherwise.
        let _ = builder.set_uniform_float("horizontal", &[0.0]);
        let _ = builder.set_uniform_float("mirrored", &[0.0]);

        Self {
            genie_builder: Arc::new(RwLock::new(builder)),
            layer: Arc::new(RwLock::new(None)),
            src: Arc::new(RwLock::new(skia::Rect::default())),
            dst: Arc::new(RwLock::new(skia::Rect::default())),
            progress: Arc::new(RwLock::new(0.0)),
            offset: Arc::new(RwLock::new((0.0, 0.0))),
        }
    }

    /// Which way the window is sucked: sideways instead of downwards
    /// (`horizontal`), and towards decreasing coordinates instead of increasing
    /// ones (`mirrored`, i.e. a dock on the left edge). Set before the
    /// animation starts; it holds until changed.
    pub fn set_direction(&self, horizontal: bool, mirrored: bool) {
        let mut builder = self.genie_builder.write().unwrap();
        let _ = builder.set_uniform_float("horizontal", &[if horizontal { 1.0 } else { 0.0 }]);
        let _ = builder.set_uniform_float("mirrored", &[if mirrored { 1.0 } else { 0.0 }]);
    }

    pub fn set_destination(&self, to_rect: skia::Rect, set_offset: bool) {
        let offset = if set_offset {
            let offset = {
                let layer = self.layer.read().unwrap();
                layer
                    .as_ref()
                    .map(|l| {
                        let r = l.render_bounds_transformed();
                        (r.x(), r.y())
                    })
                    .unwrap_or((0.0, 0.0))
            };
            let mut offset_lock = self.offset.write().unwrap();
            *offset_lock = offset;
            offset
        } else {
            *self.offset.read().unwrap()
        };

        let mut builder = self.genie_builder.write().unwrap();
        let _ = builder.set_uniform_float(
            "dst_bounds",
            &[
                to_rect.x() - offset.0,
                to_rect.y() - offset.1,
                to_rect.width(),
                to_rect.height(),
            ],
        );
        {
            let mut dst = self.dst.write().unwrap();
            *dst = to_rect;
        }
        self.update_filter_bounds();
    }
    fn update_filter_bounds(&self) {
        if let Some(layer) = &*self.layer.read().unwrap() {
            let dst = self.dst.read().unwrap();
            let src = self.src.read().unwrap();
            let p = self.progress.read().unwrap();

            if (*p) == 0.0 {
                layer.set_filter_bounds(*src);
                return;
            }

            let bounds = skia::Rect::join2(*src, *dst);
            layer.set_filter_bounds(bounds);
        }
    }
    pub fn set_source(&self, from_rect: skia::Rect) {
        let mut builder = self.genie_builder.write().unwrap();
        let _ = builder.set_uniform_float(
            "src_bounds",
            &[
                from_rect.x(),
                from_rect.y(),
                from_rect.width(),
                from_rect.height(),
            ],
        );
        {
            let mut src = self.src.write().unwrap();
            *src = from_rect;
        }
        self.update_filter_bounds();
    }
    pub fn apply(&self) {
        let layer = &*self.layer.read().unwrap();
        if let Some(layer) = layer {
            let builder = self.genie_builder.write().unwrap();
            if let Some(filter) = skia::image_filters::runtime_shader(&builder, "", None) {
                layer.set_image_filter(filter);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// `GenieEffect::new` unwraps the shader compilation, so a typo in the
    /// SkSL takes down the compositor at the first minimize.
    #[test]
    fn genie_shader_compiles() {
        let effect = GenieEffect::new();
        effect.set_direction(true, true);
        effect.set_direction(false, false);
    }
}

impl layers::prelude::Effect for GenieEffect {
    fn init(&self, layer: &Layer) {
        self.layer.write().unwrap().replace(layer.clone());
    }
    fn start(&self, layer: &Layer) {
        let render_layer = layer.render_bounds_with_children();

        self.set_source(render_layer);
        let mut builder = self.genie_builder.write().unwrap();
        let _ = builder.set_uniform_float("progress", &[0.0]);
        if let Some(filter) = skia::image_filters::runtime_shader(&builder, "", None) {
            layer.set_image_filter(filter);
        }
    }
    fn update(&self, layer: &Layer, progress: f32) {
        let mut builder = self.genie_builder.write().unwrap();
        let _ = builder.set_uniform_float("progress", &[progress]);
        {
            let mut p = self.progress.write().unwrap();
            *p = progress;
        }
        if let Some(filter) = skia::image_filters::runtime_shader(&builder, "", None) {
            if progress > 0.0 {
                layer.set_image_filter(filter);
                self.update_filter_bounds();
            }
        }
        if progress == 0.0 {
            layer.set_image_filter(None);
            layer.set_filter_bounds(None);
        }
    }
    fn finish(&self, layer: &Layer) {
        layer.set_image_filter(None);
    }
}
