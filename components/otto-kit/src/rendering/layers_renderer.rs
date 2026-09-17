use layers::prelude::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Bridge between hello-design's Wayland/Skia rendering and layers engine
pub struct LayersRenderer {
    engine: Arc<Engine>,
    _last_frame: Instant,
    /// Held for the length of an engine update. The renderer thread ticks the
    /// engine, and a paint on the UI thread flushes it too; the two must not
    /// run an update at the same time.
    updating: Mutex<()>,
}

impl LayersRenderer {
    /// Create a new layers renderer with given dimensions
    pub fn new(width: f32, height: f32) -> Self {
        let engine = Engine::create(width, height);
        let root = engine.new_layer();
        root.set_key("root");
        root.set_size(layers::types::Size::points(width, height), None);
        let _ = engine.add_layer(&root);
        engine.scene_set_root(root);
        engine.scene_set_size(width, height);

        #[cfg(feature = "debugger")]
        engine.start_debugger();
        Self {
            engine,
            _last_frame: Instant::now(),
            updating: Mutex::new(()),
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
        let _updating = self.updating.lock().unwrap_or_else(|e| e.into_inner());
        self.engine.update(0.016)
    }

    /// Apply the changes scheduled since the last update, without moving the
    /// animation clock.
    ///
    /// A layer change — `set_draw_content` above all — only lands when the
    /// engine next updates, and the renderer thread does that on its own
    /// 12 ms tick. A paint straight after a keystroke would otherwise draw
    /// the scene as it was before the keystroke, and find none of its damage;
    /// with nothing left asking for a paint, the change stayed off screen
    /// until something else — a caret blink — drew again.
    pub fn flush(&self) -> bool {
        let _updating = self.updating.lock().unwrap_or_else(|e| e.into_inner());
        self.engine.update(0.0)
    }

    /// Flush, then take the damage accumulated since the last call. Under the
    /// same lock as the update, so damage the renderer thread adds while this
    /// reads is not cleared unseen.
    pub fn take_damage(&self) -> skia_safe::Rect {
        let _updating = self.updating.lock().unwrap_or_else(|e| e.into_inner());
        self.engine.update(0.0);
        let damage = self.engine.damage();
        self.engine.clear_damage();
        damage
    }

    /// Resize the renderer
    pub fn resize(&mut self, width: f32, height: f32) {
        self.engine.scene_set_size(width, height);
    }
}
