use layers::prelude::*;
use std::sync::Arc;
use std::time::Instant;

/// Bridge between hello-design's Wayland/Skia rendering and layers engine
pub struct LayersRenderer {
    engine: Arc<Engine>,
    _last_frame: Instant,
}

impl LayersRenderer {
    /// Create a new layers renderer with given dimensions
    pub fn new(width: f32, height: f32) -> Self {
        let engine = Engine::create(width, height);
        let root = engine.new_layer();
        root.set_key("root");
        root.set_size(layers::types::Size::points(width, height), None);
        engine.add_layer(&root);
        engine.scene_set_root(root);
        engine.scene_set_size(width, height);

        engine.start_debugger();
        Self {
            engine,
            _last_frame: Instant::now(),
        }
    }

    /// Get a reference to the engine
    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }

    /// Update layout and animations (call once per frame)
    /// Returns true if a redraw is needed
    /// Limited to 60fps - will skip updates if called too frequently
    pub fn update(&self) -> bool {
        // Update engine (layout + animations)
        self.engine.update(0.016)
    }

    /// Resize the renderer
    pub fn resize(&mut self, width: f32, height: f32) {
        self.engine.scene_set_size(width, height);
    }
}
