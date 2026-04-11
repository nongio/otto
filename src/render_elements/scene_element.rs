use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};

#[cfg(feature = "perf-counters")]
use std::time::Duration;

use layers::{
    drawing::render_node_tree,
    engine::{Engine, NodeRef},
    prelude::Layer,
};

use smithay::{
    backend::renderer::{
        element::{Element, Id, RenderElement},
        utils::{CommitCounter, DamageBag, DamageSet},
        RendererSuper,
    },
    utils::{Buffer, Physical, Point, Rectangle, Scale},
};

use crate::{skia_renderer::SkiaRenderer, udev::UdevRenderer};

#[derive(Clone)]
pub struct SceneElement {
    id: Id,
    commit_counter: CommitCounter,
    engine: Arc<Engine>,
    last_update: Instant,
    pub size: (f32, f32),
    damage: Rc<RefCell<DamageBag<i32, Physical>>>,
    /// When set, render from this node instead of the global scene root.
    /// Used to render only a specific output's sub-tree (coordinates are output-local).
    pub output_root: Option<NodeRef>,
    #[cfg(feature = "perf-counters")]
    perf_stats: Rc<RefCell<ScenePerfStats>>,
}

impl SceneElement {
    pub fn with_engine(engine: Arc<Engine>) -> Self {
        Self {
            id: Id::new(),
            commit_counter: CommitCounter::default(),
            engine,
            last_update: Instant::now(),
            size: (0.0, 0.0),
            damage: Rc::new(RefCell::new(DamageBag::new(5))),
            output_root: None,
            #[cfg(feature = "perf-counters")]
            perf_stats: Rc::new(RefCell::new(ScenePerfStats::new())),
        }
    }

    /// Return a clone of this element that renders from the given output layer node.
    pub fn for_output_layer(&self, layer: &Layer) -> Self {
        let mut clone = self.clone();
        clone.output_root = Some(layer.id);
        clone
    }
    #[profiling::function]
    pub fn update(&mut self) -> bool {
        let dt = self.last_update.elapsed().as_secs_f32();
        self.last_update = Instant::now();

        #[cfg(feature = "perf-counters")]
        let mut stats = self.perf_stats.borrow_mut();
        #[cfg(feature = "perf-counters")]
        {
            stats.total_updates += 1;
        }

        let updated = self.engine.update(dt);
        if !updated {
            #[cfg(feature = "perf-counters")]
            stats.log_if_due();
            return false;
        }

        // Reset occlusion data for the new frame; each output will
        // recompute its own occlusion set during draw().
        self.engine.clear_occlusion();

        #[cfg(feature = "perf-counters")]
        {
            stats.updates_with_changes += 1;
        }

        self.commit_counter.increment();
        let scene_damage = self.engine.damage();
        let has_damage = !scene_damage.is_empty();

        #[cfg(feature = "perf-counters")]
        {
            if has_damage {
                stats.updates_with_damage += 1;
            }
            stats.log_if_due();
        }

        if has_damage {
            self.commit_counter.increment();
            let safe = 0;
            let damage = Rectangle::new(
                (
                    scene_damage.x() as i32 - safe,
                    scene_damage.y() as i32 - safe,
                )
                    .into(),
                (
                    scene_damage.width() as i32 + safe * 2,
                    scene_damage.height() as i32 + safe * 2,
                )
                    .into(),
            );
            self.damage.borrow_mut().add(vec![damage]);
        }

        has_damage
    }
    pub fn root_layer(&self) -> Option<Layer> {
        self.engine
            .scene_root()
            .and_then(|id| self.engine.get_layer(&id))
    }
    pub fn set_size(&mut self, width: f32, height: f32) {
        self.engine.scene_set_size(width, height);
        self.size = (width, height);
    }
    /// Returns true if the scene graph has pending animations/transactions.
    pub fn has_pending_animations(&self) -> bool {
        self.engine.pending_transactions_count() > 0
    }
}

#[cfg(feature = "perf-counters")]
#[derive(Debug)]
struct ScenePerfStats {
    total_updates: u64,
    updates_with_changes: u64,
    updates_with_damage: u64,
    last_log: Instant,
    prev_logged_updates: u64,
    prev_logged_changes: u64,
    prev_logged_damage: u64,
}

#[cfg(feature = "perf-counters")]
impl ScenePerfStats {
    fn new() -> Self {
        Self {
            total_updates: 0,
            updates_with_changes: 0,
            updates_with_damage: 0,
            last_log: Instant::now(),
            prev_logged_updates: 0,
            prev_logged_changes: 0,
            prev_logged_damage: 0,
        }
    }

    fn log_if_due(&mut self) {
        if self.last_log.elapsed() < Duration::from_secs(1) {
            return;
        }

        let delta_updates = self.total_updates - self.prev_logged_updates;
        let delta_changes = self.updates_with_changes - self.prev_logged_changes;
        let delta_damage = self.updates_with_damage - self.prev_logged_damage;
        let delta_no_change = delta_updates.saturating_sub(delta_changes);

        tracing::debug!(
            total_updates = self.total_updates,
            updates_per_sec = delta_updates,
            updates_with_scene_changes = delta_changes,
            updates_with_damage = delta_damage,
            updates_without_changes = delta_no_change,
            "scene perf counters",
        );

        self.prev_logged_updates = self.total_updates;
        self.prev_logged_changes = self.updates_with_changes;
        self.prev_logged_damage = self.updates_with_damage;
        self.last_log = Instant::now();
    }
}

impl Element for SceneElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn location(&self, _scale: Scale<f64>) -> Point<i32, Physical> {
        if self.output_root.is_some() {
            // Per-output element: always at (0,0) in the output framebuffer.
            // Canvas translation in draw() maps scene coords to output-local coords.
            return (0, 0).into();
        }
        if let Some(root) = self.root_layer() {
            let bounds = root.render_bounds_transformed();
            (bounds.x() as i32, bounds.y() as i32).into()
        } else {
            (0, 0).into()
        }
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::new((0, 0).into(), (100, 100).into()).to_f64()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        if let Some(oid) = self.output_root {
            // Per-output element: geometry fills the output framebuffer from (0,0).
            let size = self
                .engine
                .get_layer(&oid)
                .map(|l| {
                    let b = l.render_bounds_transformed();
                    (b.width() as i32, b.height() as i32).into()
                })
                .unwrap_or_default();
            return Rectangle::new((0, 0).into(), size);
        }
        if let Some(root) = self.root_layer() {
            let bounds = root.render_bounds_transformed();
            Rectangle::new(
                self.location(scale),
                (bounds.width() as i32, bounds.height() as i32).into(),
            )
        } else {
            Rectangle::new(self.location(scale), (0, 0).into())
        }
    }

    fn current_commit(&self) -> CommitCounter {
        self.damage.borrow().current_commit()
    }
    /// Get the damage since the provided commit relative to the element
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> smithay::backend::renderer::utils::DamageSet<i32, Physical> {
        let geometry_size = self.geometry(scale).size;
        if geometry_size.w <= 0 || geometry_size.h <= 0 {
            return DamageSet::default();
        }

        let full_damage = Rectangle::new((0, 0).into(), geometry_size);
        let damage = self.damage.borrow().damage_since(commit);

        match damage {
            // Known damage rects — return them as partial damage.
            // The canvas will be clipped to these rects so only the
            // changed region is cleared and redrawn.
            Some(rects) if !rects.is_empty() => DamageSet::from_slice(&rects),
            // Commit too old or unknown (new buffer) — must repaint everything.
            None => DamageSet::from_slice(&[full_damage]),
            // Nothing changed — Smithay can safely skip this element.
            _ => DamageSet::default(),
        }
    }
    fn alpha(&self) -> f32 {
        1.0
    }
}

impl<'renderer> RenderElement<UdevRenderer<'renderer>> for SceneElement {
    fn draw(
        &self,
        frame: &mut <UdevRenderer<'renderer> as RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <UdevRenderer<'renderer> as RendererSuper>::Error> {
        RenderElement::<SkiaRenderer>::draw(self, frame.as_mut(), src, dst, damage, opaque_regions)
            .map_err(|e| e.into())
    }
}

impl RenderElement<SkiaRenderer> for SceneElement {
    fn draw<'frame>(
        &self,
        frame: &mut <SkiaRenderer as RendererSuper>::Frame<'frame, 'frame>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <SkiaRenderer as RendererSuper>::Error> {
        #[cfg(feature = "profile-with-puffin")]
        profiling::puffin::profile_scope!("render_scene");
        let mut surface = frame.skia_surface.clone();

        let canvas = surface.canvas();
        let scene = self.engine.scene();
        // Use per-output root if set, otherwise fall back to global scene root.
        let root_id = self.output_root.or_else(|| self.engine.scene_root());
        let save_point = canvas.save();

        // Clip to the output destination rectangle to prevent drawing outside screen bounds.
        let output_clip = layers::skia::Rect::from_xywh(
            dst.loc.x as f32,
            dst.loc.y as f32,
            dst.size.w as f32,
            dst.size.h as f32,
        );
        canvas.clip_rect(output_clip, Some(layers::skia::ClipOp::Intersect), false);

        // Build a Skia Region from the damage rects for canvas clipping and
        // node-level culling. Each damage rect is offset by the destination
        // position so it aligns with scene-space coordinates on the canvas.
        let damage_region = if !damage.is_empty() {
            let irects: Vec<layers::skia::IRect> = damage
                .iter()
                .map(|r| {
                    layers::skia::IRect::from_xywh(
                        r.loc.x + dst.loc.x,
                        r.loc.y + dst.loc.y,
                        r.size.w,
                        r.size.h,
                    )
                })
                .collect();
            let mut region = layers::skia::Region::new();
            region.set_rects(&irects);
            // Clip the canvas to the damage region so Skia skips drawing
            // outside the damaged area entirely.
            canvas.clip_region(&region, Some(layers::skia::ClipOp::Intersect));
            Some(region)
        } else {
            None
        };

        // If rendering from an output sub-tree, translate so the output_layer's
        // scene-space position maps to (0,0) on the output framebuffer.
        if let Some(oid) = self.output_root {
            if let Some(layer) = self.engine.get_layer(&oid) {
                let pos = layer.render_position();
                if pos.x != 0.0 || pos.y != 0.0 {
                    canvas.translate((-pos.x, -pos.y));
                }
            }
        }

        // Compute occlusion for this output's root and retrieve the occluded set.
        let occluded_set = if crate::config::Config::with(|c| c.occlusion_culling) {
            if let Some(root_id) = root_id {
                self.engine.compute_occlusion(root_id);
                scene.occlusion_map().and_then(|m| m.get(&root_id).cloned())
            } else {
                None
            }
        } else {
            None
        };
        let occluded_ref = occluded_set.as_ref();
        // When occlusion culling is disabled, also skip damage-based subtree
        // culling so that no layer is hidden by any rendering optimisation.
        // The canvas is already clipped to the damage region by Skia, so
        // correctness is preserved — only extra tree traversal is incurred.
        let damage_ref = if occluded_ref.is_some() {
            damage_region.as_ref()
        } else {
            None
        };

        scene.with_arena(|arena| {
            scene.with_renderable_arena(|renderable_arena| {
                if let Some(root_id) = root_id {
                    render_node_tree(
                        root_id,
                        arena,
                        renderable_arena,
                        canvas,
                        1.0,
                        occluded_ref,
                        damage_ref,
                    );
                }
                self.engine.clear_damage();
            });
        });
        canvas.restore_to_count(save_point);

        Ok(())
    }
}
