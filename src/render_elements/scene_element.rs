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
    /// When set, `output_root` is a plane subtree (background / windows /
    /// expose / overlay …) rendered in isolation — exactly like the KMS
    /// plane path: ancestor visibility is ignored and the dynamic part of
    /// the root's scene position (workspace scroll) is re-applied, minus
    /// the output's static origin. See `SceneDmabufElement` for the model.
    pub subtree_origin: Option<(f32, f32)>,
    #[cfg(feature = "perf-counters")]
    perf_stats: Rc<RefCell<ScenePerfStats>>,
}

/// Longest step the engine clock advances per tick — two frames at 60 Hz.
/// See `SceneElement::update`.
const MAX_ENGINE_STEP_SECS: f32 = 1.0 / 30.0;

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
            subtree_origin: None,
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

    /// Return a clone of this element that renders one plane subtree of an
    /// output (background_plane / windows_plane / expose / overlay …) in
    /// isolation, mirroring the KMS plane path: ancestor visibility (e.g. the
    /// hidden `workspaces_layer` while expose is shown) does not apply, and
    /// the dynamic part of the root's scene position (workspace scroll) is
    /// re-applied minus the output's static `origin`. Several of these are
    /// stacked in z-order to composite a full output frame without planes.
    /// Gets a fresh element `Id` so multiple subtrees can coexist in one
    /// `render_output` call.
    pub fn for_plane_subtree(&self, layer: &Layer, origin: (f32, f32)) -> Self {
        let mut clone = self.clone();
        clone.id = Id::new();
        clone.output_root = Some(layer.id);
        clone.subtree_origin = Some(origin);
        clone
    }
    #[profiling::function]
    pub fn update(&mut self) -> bool {
        // The engine clock is the sum of the `dt`s handed to it, and a
        // transition is scheduled against that clock as of the *last* tick.
        // On udev the loop goes idle when nothing changes, so an animation
        // started from idle (a key press after a quiet hold, a background
        // task) would be timestamped hundreds of milliseconds in the past and
        // the next tick would carry it straight past its end — the switcher
        // snapped instead of fading. Idle time is not animation time: cap the
        // step at a couple of frame periods so the first tick after a gap
        // advances the clock like any other frame.
        let dt = self
            .last_update
            .elapsed()
            .as_secs_f32()
            .min(MAX_ENGINE_STEP_SECS);
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
                    // Plane subtrees (windows_plane, background_plane) have
                    // auto size — their extent is defined by their children.
                    let b = if self.subtree_origin.is_some() {
                        l.render_bounds_with_children_transformed()
                    } else {
                        l.render_bounds_transformed()
                    };
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
                if let Some((ox, oy)) = self.subtree_origin {
                    // Plane subtree: the tree renders root-local, which loses
                    // the ancestor scroll offset — re-apply the dynamic part
                    // of the root's global position, minus the output's
                    // static origin (same correction as SceneDmabufElement).
                    let (dx, dy) = (pos.x - ox, pos.y - oy);
                    if dx != 0.0 || dy != 0.0 {
                        canvas.translate((dx, dy));
                    }
                } else if pos.x != 0.0 || pos.y != 0.0 {
                    canvas.translate((-pos.x, -pos.y));
                }
            }
        }

        // Compute occlusion for this output's root and retrieve the occluded set.
        // Skipped for plane subtrees — they mirror the KMS plane path, which
        // renders without occlusion culling (`SceneDmabufElement` passes None).
        let occluded_set = if self.subtree_origin.is_none()
            && crate::config::Config::with(|c| c.occlusion_culling)
        {
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
        // The damage region is always forwarded so render_node_tree can cull
        // whole untouched subtrees instead of re-walking them per frame.
        let damage_ref = damage_region.as_ref();

        let scene_draw_t = std::time::Instant::now();
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
                        None,
                    );
                }
                self.engine.clear_damage();
            });
        });
        crate::render_phase_stats::record_scene_draw(scene_draw_t.elapsed());
        canvas.restore_to_count(save_point);

        Ok(())
    }
}
