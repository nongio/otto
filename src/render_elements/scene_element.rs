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

use crate::{renderer::active, skia_renderer::SkiaRenderer, udev::UdevRenderer};

#[derive(Clone)]
pub struct SceneElement {
    id: Id,
    commit_counter: CommitCounter,
    engine: Arc<Engine>,
    last_update: Instant,
    /// Whether the engine had transactions in flight when the last tick
    /// ended. Decides whether the time since then was an animation running
    /// slowly (count it all) or the loop sitting idle (count a frame of it).
    animating_after_last_tick: bool,
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

/// Longest step the engine clock advances on the first tick after an idle
/// stretch — two frames at 60 Hz. See `SceneElement::update`.
const MAX_ENGINE_STEP_SECS: f32 = 1.0 / 30.0;

impl SceneElement {
    pub fn with_engine(engine: Arc<Engine>) -> Self {
        Self {
            id: Id::new(),
            commit_counter: CommitCounter::default(),
            engine,
            last_update: Instant::now(),
            animating_after_last_tick: false,
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
        // snapped instead of fading. Idle time is not animation time: when
        // nothing was in flight at the end of the last tick, the gap since
        // then is idle and only a frame of it is credited to the clock.
        //
        // While something *is* in flight the gap is real animation time and
        // has to be credited in full, however long the frame took — capping
        // it there put a maximize or tile into slow motion whenever the
        // client's repaints held the loop under 30 Hz.
        let elapsed = self.last_update.elapsed().as_secs_f32();
        let dt = if self.animating_after_last_tick {
            elapsed
        } else {
            elapsed.min(MAX_ENGINE_STEP_SECS)
        };
        self.last_update = Instant::now();

        #[cfg(feature = "perf-counters")]
        let mut stats = self.perf_stats.borrow_mut();
        #[cfg(feature = "perf-counters")]
        {
            stats.total_updates += 1;
        }

        let updated = self.engine.update(dt);
        self.animating_after_last_tick = self.engine.pending_transactions_count() > 0;
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
            // The damage as the rectangles it was reported in, in this
            // element's space and inside it. The engine also offers their
            // bounding box, and that used to be all that was handed on — so
            // a window repainting on another workspace (a rectangle off the
            // left of the output) and a caret blinking on this one became a
            // box spanning the screen between them, repainted at the
            // window's frame rate for nothing anyone could see. Kept apart
            // and cut to the output, the off-screen rectangle is simply not
            // there. A rectangle that survives the cut is repainted in its
            // own pass (see `draw`), so their number is capped: past it the
            // box of what is left is cheaper than the passes.
            const MAX_DAMAGE_RECTS: usize = 8;
            let (ox, oy) = self
                .output_root
                .and_then(|oid| self.engine.get_layer(&oid))
                .map(|layer| {
                    let pos = layer.render_position();
                    (pos.x, pos.y)
                })
                .unwrap_or((0.0, 0.0));
            let output = Rectangle::<i32, Physical>::new(
                (0, 0).into(),
                (self.size.0 as i32, self.size.1 as i32).into(),
            );
            let to_local = |r: layers::skia::Rect| -> Option<Rectangle<i32, Physical>> {
                let local = Rectangle::<i32, Physical>::new(
                    ((r.left - ox).floor() as i32, (r.top - oy).floor() as i32).into(),
                    (
                        (r.right - ox).ceil() as i32 - (r.left - ox).floor() as i32,
                        (r.bottom - oy).ceil() as i32 - (r.top - oy).floor() as i32,
                    )
                        .into(),
                );
                local.intersection(output)
            };
            let mut rects: Vec<Rectangle<i32, Physical>> = self
                .engine
                .damage_rects()
                .into_iter()
                .filter(|r| !r.is_empty())
                .filter_map(to_local)
                .collect();
            rects.sort_by_key(|r| (r.loc.x, r.loc.y, r.size.w, r.size.h));
            rects.dedup();
            if rects.len() > MAX_DAMAGE_RECTS {
                let all = rects.iter().skip(1).fold(rects[0], |acc, r| acc.merge(*r));
                rects = vec![all];
            }
            // Everything that changed may lie off this output; the frame
            // then has nothing to repaint here, and `draw` still runs to
            // consume the engine's damage for every output that shares it.
            if !rects.is_empty() {
                self.damage.borrow_mut().add(rects);
            }
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
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), <UdevRenderer<'renderer> as RendererSuper>::Error> {
        let frame: &mut active::Frame<'_> = frame.as_mut();
        self.draw_scene(frame.skia_surface.canvas(), dst, damage);
        Ok(())
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
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), <SkiaRenderer as RendererSuper>::Error> {
        self.draw_scene(frame.skia_surface.canvas(), dst, damage);
        Ok(())
    }
}

impl SceneElement {
    /// Draws the damaged part of the scene into `canvas` at `dst`.
    ///
    /// Damage is relative to `dst`.
    fn draw_scene(
        &self,
        canvas: &layers::skia::Canvas,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
    ) {
        #[cfg(feature = "profile-with-puffin")]
        profiling::puffin::profile_scope!("render_scene");
        tracing::debug!(target: "otto::planes", "scene draw {damage:?}");

        let scene = self.engine.scene();
        // Use per-output root if set, otherwise fall back to global scene root.
        let root_id = self.output_root.or_else(|| self.engine.scene_root());

        // The damage rects in canvas coordinates: each is offset by the
        // destination position so it aligns with scene space, and kept
        // inside the output.
        let output_clip =
            layers::skia::IRect::from_xywh(dst.loc.x, dst.loc.y, dst.size.w, dst.size.h);
        let rects: Vec<layers::skia::IRect> = damage
            .iter()
            .filter_map(|r| {
                let r = layers::skia::IRect::from_xywh(
                    r.loc.x + dst.loc.x,
                    r.loc.y + dst.loc.y,
                    r.size.w,
                    r.size.h,
                );
                layers::skia::IRect::intersect(&r, &output_clip)
            })
            .filter(|r| !r.is_empty())
            .collect();

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

        // Where the scene's own position has to be re-based for this element.
        let translate = self.output_root.and_then(|oid| {
            let layer = self.engine.get_layer(&oid)?;
            let pos = layer.render_position();
            let (dx, dy) = match self.subtree_origin {
                // Plane subtree: the tree renders root-local, which loses
                // the ancestor scroll offset — re-apply the dynamic part
                // of the root's global position, minus the output's
                // static origin (same correction as SceneDmabufElement).
                Some((ox, oy)) => (pos.x - ox, pos.y - oy),
                None => (-pos.x, -pos.y),
            };
            (dx != 0.0 || dy != 0.0).then_some((dx, dy))
        });

        // A frosted shape has to be blurred whole: its blur reads the pixels
        // beneath it, and past the edge of a pass those pixels are last
        // frame's — the shape's own frost among them — so a pass that covers
        // part of the shape blurs frost into frost and leaves a seam where it
        // stopped, and lay-rs keeps a seam-free blurred backdrop only from a
        // pass that covered the whole shape. Every pass that touches a
        // `BackgroundBlur` shape is grown to cover it, and passes that then
        // overlap are merged, so the frost is blurred once, in one piece.
        let (dx, dy) = translate.unwrap_or((0.0, 0.0));
        let blur_shapes: Vec<layers::skia::IRect> = scene.with_arena(|arena| {
            arena
                .iter()
                .filter(|node| !node.is_removed())
                .map(|node| node.get())
                .filter(|node| !node.hidden())
                .map(|node| node.render_layer())
                .filter(|layer| {
                    layer.blend_mode == layers::prelude::BlendMode::BackgroundBlur
                        && layer.premultiplied_opacity > 0.0
                })
                .map(|layer| {
                    let b = layer.global_transformed_bounds.with_offset((dx, dy));
                    let ir: layers::skia::IRect = layers::skia::RoundOut::round_out(&b);
                    ir
                })
                .filter(|r| !r.is_empty())
                .collect()
        });
        let hits = |a: &layers::skia::IRect, b: &layers::skia::IRect| {
            a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
        };
        let covers = |a: &layers::skia::IRect, b: &layers::skia::IRect| {
            a.left <= b.left && a.top <= b.top && a.right >= b.right && a.bottom >= b.bottom
        };
        let union = |a: &layers::skia::IRect, b: &layers::skia::IRect| {
            layers::skia::IRect::from_ltrb(
                a.left.min(b.left),
                a.top.min(b.top),
                a.right.max(b.right),
                a.bottom.max(b.bottom),
            )
        };
        let mut rects = rects;
        if !blur_shapes.is_empty() {
            let mut grown = true;
            while grown {
                grown = false;
                for rect in rects.iter_mut() {
                    for shape in &blur_shapes {
                        if hits(rect, shape) && !covers(rect, shape) {
                            *rect = union(rect, shape);
                            grown = true;
                        }
                    }
                }
                // Merge passes that overlap, so no shape is blurred twice and
                // no pixel is painted twice.
                let mut merged: Vec<layers::skia::IRect> = Vec::with_capacity(rects.len());
                for rect in rects.drain(..) {
                    let mut rect = rect;
                    let mut i = 0;
                    while i < merged.len() {
                        if hits(&merged[i], &rect) {
                            rect = union(&rect, &merged.swap_remove(i));
                            grown = true;
                        } else {
                            i += 1;
                        }
                    }
                    merged.push(rect);
                }
                rects = merged;
            }
            for rect in rects.iter_mut() {
                if let Some(clipped) = layers::skia::IRect::intersect(rect, &output_clip) {
                    *rect = clipped;
                }
            }
        }

        let scene_draw_t = std::time::Instant::now();
        scene.with_arena(|arena| {
            scene.with_renderable_arena(|renderable_arena| {
                let Some(root_id) = root_id else {
                    self.engine.clear_damage();
                    return;
                };
                // One pass per damage rect, each under a plain rectangular
                // clip. The rects used to be joined into one Skia region and
                // clipped in a single pass, but a region of more than one
                // rectangle is a path clip on this render target, and what
                // Skia painted under it was the region's bounding box for
                // some draws and the exact rects for others: with a window
                // repainting on another workspace and a card open above the
                // chrome, the bar and the dock shadow came out wiped in the
                // box's corners, cut along the card's edges. A rectangle
                // clips the same way for every draw. Damage arrives from the
                // tracker already merged, so the passes are few, and each
                // one culls what lies outside its own rect.
                for rect in &rects {
                    let save_point = canvas.save();
                    canvas.clip_irect(*rect, Some(layers::skia::ClipOp::Intersect));
                    if let Some((dx, dy)) = translate {
                        canvas.translate((dx, dy));
                    }
                    // The rect is forwarded as the damage region so
                    // render_node_tree can cull whole untouched subtrees
                    // instead of re-walking them per frame.
                    let mut region = layers::skia::Region::new();
                    region.set_rect(*rect);
                    render_node_tree(
                        root_id,
                        arena,
                        renderable_arena,
                        canvas,
                        1.0,
                        occluded_ref,
                        Some(&region),
                        None,
                    );
                    canvas.restore_to_count(save_point);
                }
                self.engine.clear_damage();
            });
        });
        crate::render_phase_stats::record_scene_draw(scene_draw_t.elapsed());
    }
}
