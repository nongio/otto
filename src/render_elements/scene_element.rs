use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};

#[cfg(feature = "perf-counters")]
use std::time::Duration;

use layers::{drawing::render_node_tree, engine::{Engine, NodeRef}, prelude::Layer};

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
    empty_damage_count: Rc<RefCell<u32>>,
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
            empty_damage_count: Rc::new(RefCell::new(0)),
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

        tracing::info!(
            target: "screen_composer::perf.scene",
            total_updates = self.total_updates,
            updates_per_sec = delta_updates,
            updates_with_scene_changes = delta_changes,
            updates_with_damage = delta_damage,
            updates_without_changes = delta_no_change,
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
            let size = self.engine.get_layer(&oid)
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
        let damage = self.damage.borrow().damage_since(commit);

        match damage {
            Some(rects) if !rects.is_empty() => {
                *self.empty_damage_count.borrow_mut() = 0;
                DamageSet::from_slice(&rects)
            }
            None if geometry_size.w > 0 && geometry_size.h > 0 => {
                let mut count = self.empty_damage_count.borrow_mut();
                *count += 1;
                if *count >= 3 {
                    *count = 0;
                    let full_damage = Rectangle::new((0, 0).into(), geometry_size);
                    DamageSet::from_slice(&[full_damage])
                } else {
                    DamageSet::default()
                }
            }
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

        scene.with_arena(|arena| {
            scene.with_renderable_arena(|renderable_arena| {
                if let Some(root_id) = root_id {
                    // Check if we should use damage clipping or render full scene
                    // Only use damage clipping when:
                    // 1. Damage is provided (not empty)
                    // 2. Damage is partial (not covering entire element)
                    // This ensures buffer preservation is working correctly

                    let should_clip = if damage.is_empty() {
                        false // No damage info, render full
                    } else {
                        // Check if damage covers the entire element
                        let total_damage_area: i32 =
                            damage.iter().map(|r| r.size.w * r.size.h).sum();
                        let element_area = dst.size.w * dst.size.h;

                        // If damage is less than 95% of element, use clipping
                        // (95% threshold to account for rounding)
                        total_damage_area < (element_area * 95 / 100)
                    };

                    if should_clip {
                        // Use Skia Region for efficient multi-rect clipping
                        let mut clip_region = layers::skia::Region::new();
                        for d in damage.iter() {
                            if d.size.w <= 0 || d.size.h <= 0 {
                                continue;
                            }
                            let irect = layers::skia::IRect::from_xywh(
                                d.loc.x, d.loc.y, d.size.w, d.size.h,
                            );
                            clip_region.op_rect(irect, layers::skia::region::RegionOp::Union);
                        }

                        // Render scene once with complex clip region
                        if !clip_region.is_empty() {
                            canvas.save();
                            canvas.clip_region(&clip_region, Some(layers::skia::ClipOp::Intersect));
                            render_node_tree(root_id, arena, renderable_arena, canvas, 1.0);
                            canvas.restore();
                        }
                    } else {
                        // No damage or full screen damage - render everything
                        render_node_tree(root_id, arena, renderable_arena, canvas, 1.0);
                    }
                    // Optional debug: outline damage rect
                    // let mut paint = layers::skia::Paint::default();
                    // paint.set_color(layers::skia::Color::from_argb(255, 255, 0, 0));
                    // paint.set_stroke(true);
                    // paint.set_stroke_width(1.0);
                    // for damage_rect in damage.iter() {
                    //     if !damage_rect.is_empty() {
                    //         let r = layers::skia::Rect::from_xywh(
                    //             damage_rect.loc.x as f32,
                    //             damage_rect.loc.y as f32,
                    //             damage_rect.size.w as f32,
                    //             damage_rect.size.h as f32,
                    //         );
                    //         canvas.draw_rect(r, &paint);
                    //     }
                    // }
                    // let typeface = crate::workspace::utils::FONT_CACHE
                    // .with(|font_cache| {
                    //     font_cache
                    //         .font_mgr
                    //         .match_family_style("Inter", layers::skia::FontStyle::default())
                    // })
                    // .unwrap();
                    // let font = layers::skia::Font::from_typeface_with_params(typeface, 22.0, 1.0, 0.0);
                    // let pos = self.engine.get_pointer_position();
                    // canvas.draw_str(format!("{},{}", pos.x, pos.y), (50.0, 50.0), &font, &paint);
                }
                self.engine.clear_damage();
            });
        });
        canvas.restore_to_count(save_point);
        Ok(())
    }
}
