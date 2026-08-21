//! Cross-plane backdrop (vibrancy) for the udev backend.
//!
//! The blur-bearing planes (overlay UI: dock, switcher, menus, OSD — and
//! expose) render into their own buffers, so their `BackgroundBlur` layers
//! can't see the planes below. Build a DOWNSCALED composite of the lower
//! planes and hand it to them via lay-rs' external-backdrop API
//! (`render_node_tree`'s backdrop parameter seeds it behind the blur shapes
//! with DstOver). Downscaled because the blur re-downscales its input
//! anyway: a low-res backdrop is imperceptible after blurring but far
//! cheaper to build, hold and sample than a full-res snapshot.
//!
//! Staged build in one small surface: draw bg → snapshot (the middle plane's
//! backdrop — window titlebars, or expose's hover label), draw the middle
//! plane on top → the desktop composite. Blur the desktop for the
//! dock/switcher planes. Then, because popups stack in the
//! overlay plane (a submenu must blur the popup beneath it), draw the popup
//! subtree on top of the desktop and blur the WHOLE image for the overlay
//! plane — the blur happens before any per-popup clip, so popups seed a
//! pre-blurred image and skip their own shape-clipped blur (no faded rim, like
//! the islands) yet still show the popups underneath.
//!
//! Rebuilt when a lower plane recorded damage intersecting an active consumer's
//! region, OR when the popup subtree changed; the fresh snapshot's unique_id is
//! what makes the consumers re-render.

use std::sync::Arc;

use layers::drawing::{render_node_tree, vibrancy_color_filter};
use layers::prelude::{Engine, NodeRef};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::output::Output;

use crate::render_elements::scene_dmabuf_element::SceneDmabufElement;

use super::types::{BackdropSurface, SurfaceData, UdevRenderer};

const BACKDROP_SCALE: f32 = 0.25;

/// Blur sigma applied to the downscaled backdrop composite. The backdrop is
/// `BACKDROP_SCALE` of the scene, so this is roughly the full-res sigma
/// (~40) times `BACKDROP_SCALE`. Blurring the whole composite once here lets
/// the blur-bearing consumers seed it directly and skip their own shape-clipped
/// blur (which leaves a faded, seed-exposing rim at the layer edge).
const BACKDROP_BLUR_SIGMA: f32 = 10.0;

/// The vibrancy tone map is lay-rs' own — see
/// [`layers::drawing::vibrancy_color_filter`].
///
/// It has to be: a consumer seeding this pre-blurred backdrop skips lay-rs'
/// blur pass and, with it, the grading lay-rs would have applied. Anything
/// graded differently here shows up as the same frosted surface taking on two
/// tints depending on which path drew it — a window's material reads one way
/// on its plane and another in expose, where the previews blur in the scene.
/// One function, evaluated once per thread, is what keeps them equal.
///
/// Gaussian-blur `image` into a fresh same-size GPU surface, apply the vibrancy
/// tone map, and return the snapshot. Returns `None` if the surface or filter
/// can't be built (caller falls back to the raw image).
fn blur_image(
    image: &layers::skia::Image,
    ctx: &mut layers::skia::gpu::DirectContext,
    sigma: f32,
) -> Option<layers::skia::Image> {
    let info = layers::skia::ImageInfo::new(
        (image.width(), image.height()),
        layers::skia::ColorType::RGBA8888,
        layers::skia::AlphaType::Premul,
        None,
    );
    let mut surface = layers::skia::gpu::surfaces::render_target(
        ctx,
        layers::skia::gpu::Budgeted::No,
        &info,
        None,
        layers::skia::gpu::SurfaceOrigin::TopLeft,
        None,
        false,
        false,
    )?;
    let blur = layers::skia::image_filters::blur(
        (sigma, sigma),
        layers::skia::TileMode::Clamp,
        None,
        None,
    )?;
    // Tone map on top of the blur: one filter chain, one pass.
    let filter = layers::skia::image_filters::color_filter(vibrancy_color_filter(), blur, None)?;
    let mut paint = layers::skia::Paint::default();
    paint.set_image_filter(filter);
    {
        let canvas = surface.canvas();
        canvas.clear(layers::skia::Color4f::new(0.0, 0.0, 0.0, 1.0));
        canvas.draw_image(image, (0, 0), Some(&paint));
    }
    ctx.flush_and_submit();
    Some(surface.image_snapshot())
}

/// Draw the popup subtree onto the (already desktop-filled) backdrop surface at
/// `BACKDROP_SCALE`, aligned with the desktop composite. Mirrors the transform
/// `SceneDmabufElement::render` applies (translate by the root's scene position,
/// minus the output's scene origin) plus the backdrop's downscale, so the popups
/// land exactly where they sit on screen. Blurring the result then folds them
/// into the overlay plane's backdrop. Popups are primary-output-only.
fn draw_popups(
    bs: &mut BackdropSurface,
    engine: &Arc<Engine>,
    popup_root: NodeRef,
    scene_origin: (i32, i32),
) {
    let scene = engine.scene();
    let canvas = bs.surface.canvas();
    let save = canvas.save();
    canvas.scale((BACKDROP_SCALE, BACKDROP_SCALE));
    if let Some(layer) = engine.get_layer(&popup_root) {
        let pos = layer.render_position();
        canvas.translate((pos.x - scene_origin.0 as f32, pos.y - scene_origin.1 as f32));
    }
    scene.with_arena(|arena| {
        scene.with_renderable_arena(|renderable_arena| {
            render_node_tree(
                popup_root,
                arena,
                renderable_arena,
                canvas,
                1.0,
                None,
                None,
                None,
            );
        });
    });
    canvas.restore_to_count(save);
}

/// Minimum spacing between two backdrop rebuilds caused by DESKTOP damage
/// (bg/middle planes, promoted commits, popup repaints). A client redrawing
/// at frame rate under a blur consumer must not force the composite plus a
/// full-res re-render of every blur plane per commit — blur is a
/// low-frequency visual, and `backdrop_dirty` carries the staleness to the
/// next allowed frame. Only discrete events bypass this: the first build, and
/// a STRUCTURAL popup change (a popup appearing has to blur correctly on its
/// first frame). Popup *damage* used to bypass it too, which meant any client
/// commit that dirtied a popup layer — and before the guards in
/// `configure_surface_layer` that was every commit of the popup's parent
/// window — forced a full-screen rebuild at client frame rate.
const DESKTOP_REBUILD_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Whether `r` touches any interest rect. Pure — see `decide_rebuild`.
fn rect_hits_interest(interest: &[layers::skia::Rect], r: &layers::skia::Rect) -> bool {
    interest.iter().any(|i| {
        r.left() < i.right() && r.right() > i.left() && r.top() < i.bottom() && r.bottom() > i.top()
    })
}

/// Inputs to the per-frame rebuild decision, pre-reduced to booleans by the
/// caller (interest intersection via [`rect_hits_interest`]).
struct RebuildInputs {
    /// Any blur consumer is on screen at all.
    any_consumer: bool,
    /// A composite already exists (its absence forces an immediate build).
    have_backdrop: bool,
    /// Staleness carried over from frames that skipped a rebuild.
    dirty: bool,
    bg_hit: bool,
    middle_hit: bool,
    promoted_hits: bool,
    /// The popup subtree recorded damage while the overlay is active. An
    /// ordinary trigger, under the rate limit: a popup redrawing its content
    /// (a menu highlight tracking the pointer, a spinner) is exactly the
    /// frame-rate source the limit exists for.
    popup_hit: bool,
    /// A popup mapped, unmapped, became visible or moved this frame
    /// (`PopupOverlayView::structure_generation`). Discrete and rare, and the
    /// blur under a popup that just appeared has to be right on its first
    /// frame — so this, unlike popup repaints, bypasses the rate limit.
    popup_structural: bool,
    /// On-screen lower-plane damage exists (whether or not it hits interest).
    lower_damaged: bool,
    /// An interactive animation is running (expose, a workspace swipe or its
    /// settle animation): the blur must track the moving content per frame —
    /// a 10 Hz blur under a 120 Hz scroll reads as judder — so the rate
    /// limit is suspended for its duration. Transient by construction, so
    /// this cannot re-open the idle rebuild storm.
    fluid: bool,
    last_desktop_rebuild: Option<std::time::Instant>,
    now: std::time::Instant,
}

struct RebuildDecision {
    rebuild: bool,
    /// `backdrop_dirty` after this frame.
    dirty_after: bool,
    /// Set `last_desktop_rebuild` to this when present.
    stamp_desktop_rebuild: Option<std::time::Instant>,
}

/// The per-frame backdrop rebuild decision, extracted pure so the rate
/// limiter and its dirty-flag bookkeeping are unit-testable: the regression
/// this guards against is the idle rebuild storm — a blinking cursor or an
/// animating client forcing composite + full-res blur-plane re-renders at
/// client frame rate, keeping the GPU (and fans) out of idle. See the tests
/// at the bottom of this file.
fn decide_rebuild(i: RebuildInputs) -> RebuildDecision {
    let desktop_trigger = i.dirty || i.bg_hit || i.middle_hit || i.promoted_hits || i.popup_hit;
    let desktop_rate_ok = i.fluid
        || i.last_desktop_rebuild
            .is_none_or(|t| i.now.duration_since(t) >= DESKTOP_REBUILD_MIN_INTERVAL);
    let rebuild = i.any_consumer
        && (!i.have_backdrop || (desktop_trigger && desktop_rate_ok) || i.popup_structural);
    RebuildDecision {
        rebuild,
        dirty_after: if rebuild {
            false
        } else {
            // Popup damage is folded into the overlay backdrop, so a popup
            // repaint held back by the rate limit is staleness that has to
            // reach the next allowed frame just like lower-plane damage.
            i.dirty || i.lower_damaged || i.popup_hit
        },
        stamp_desktop_rebuild: rebuild.then_some(i.now),
    }
}

/// Where lower-plane damage has to land to change what an open popup's blur
/// shows: each popup's own on-screen bounds (children included — a menu's
/// shadow paints past its layer), in output-local physical px, outset by the
/// blur sampling radius. Hidden and unplaced popups contribute nothing.
///
/// Empty means no popup is currently sampling the backdrop; the caller's other
/// interest rects (or the full-output fallback) still apply.
fn popup_interest_rects(
    engine: &Arc<Engine>,
    popup_root: Option<NodeRef>,
    scene_origin: (i32, i32),
) -> Vec<layers::skia::Rect> {
    // ~3σ of the full-res blur: content further away cannot visibly change
    // what the blur samples. Matches `BLUR_PAD` in `udev::render`.
    const BLUR_PAD: f32 = 160.0;
    let Some(root) = popup_root.and_then(|r| engine.get_layer(&r)) else {
        return Vec::new();
    };
    root.children()
        .into_iter()
        .filter(|popup| !popup.hidden())
        .filter_map(|popup| {
            let mut r = popup.render_bounds_with_children_transformed();
            if r.is_empty() {
                return None;
            }
            r.offset((-(scene_origin.0 as f32), -(scene_origin.1 as f32)));
            r.outset((BLUR_PAD, BLUR_PAD));
            Some(r)
        })
        .collect()
}

/// Rebuild the backdrop composites when needed, render the middle plane
/// (windows or expose), and hand the fresh composites to the blur-bearing
/// upper planes (overlay, switcher, dock), rendering the active ones. The
/// overlay composite also folds in the popup subtree (see `draw_popups`). The
/// bg plane must already be rendered by the caller.
#[allow(clippy::too_many_arguments)] // plane-state plumbing, all of it per-frame
pub(super) fn update_backdrop_and_upper_planes(
    surface: &mut SurfaceData,
    renderer: &mut UdevRenderer<'_>,
    output: &Output,
    expose_active: bool,
    overlay_active: bool,
    switcher_active: bool,
    dock_visible: bool,
    engine: &Arc<Engine>,
    popup_root: Option<NodeRef>,
    // Direct-scanout windows are hidden in the windows plane, so their pixels
    // exist in no plane snapshot — only in the client dmabuf KMS scans out.
    // Each entry is that dmabuf plus its on-screen rect in global scene
    // physical px; the rebuild blits them zero-copy on top of the middle
    // plane (import is a cache hit re-binding the same EGLImage).
    promoted: &[(Dmabuf, layers::skia::Rect)],
    // A promoted window committed a new buffer this frame (their commits
    // produce no scene damage, so this is the only change signal).
    promoted_commit: bool,
    // A workspace swipe or its settle animation is running — suspends the
    // desktop-rebuild rate limit so the blur tracks the scroll per frame
    // (see `RebuildInputs::fluid`). Expose being active does the same.
    fluid_animation: bool,
    // A popup mapped, unmapped, became visible or moved this frame — see
    // `RebuildInputs::popup_structural`.
    popup_structural: bool,
    // Where lower-plane damage must land to matter to the overlay plane's
    // blur consumers (output-local physical px, already blur-outset).
    // `None` = anywhere on the output — the pre-existing conservative
    // behavior, used while anything transient/unbounded is on screen.
    overlay_interest: Option<&[layers::skia::Rect]>,
) {
    // (The bg plane was already rendered above, before the branch.)
    let bg_damage = surface
        .scene_dmabuf_element
        .as_ref()
        .and_then(|el| el.subtree_damage());
    let middle_el = if expose_active {
        surface.expose_dmabuf_element.as_ref()
    } else {
        surface.windows_dmabuf_element.as_ref()
    };
    // The promoted window renders in a plane of its own, above the middle
    // plane. For everything the backdrop cares about it IS part of the middle
    // stack — the chrome planes blurring the desktop must see it, and its
    // repaints must trigger a rebuild — so its damage joins the middle plane's
    // and its buffer is composited straight after it.
    let window_el = surface
        .window_dmabuf_element
        .as_ref()
        .filter(|_| surface.window_plane_active && !expose_active);
    let middle_damage = match (
        middle_el.and_then(|el| el.subtree_damage()),
        window_el.and_then(|el| el.subtree_damage()),
    ) {
        (Some(mut m), Some(w)) => {
            m.join(w);
            Some(m)
        }
        (m, w) => m.or(w),
    };
    // The output's static scene position, so popups (global scene coords) map
    // into the output-local backdrop surface.
    let scene_origin = surface
        .overlay_dmabuf_element
        .as_ref()
        .map(|el| el.scene_origin())
        .unwrap_or((0, 0));

    // The composite only matters to the blur-bearing consumers that
    // are actually on screen, and only where they sample it: the dock
    // and switcher sample their own strips; the overlay UI and expose
    // can blur anywhere. Lower-plane damage outside every active
    // region (the common case: a window updating above the dock) must
    // NOT rebuild — a rebuild forces every blur consumer to re-render
    // its full buffer. Damage skipped this way marks the composite
    // dirty so a later-activating consumer still gets fresh content.
    let mut interest: Vec<layers::skia::Rect> = Vec::new();
    let full_output_interest = expose_active || (overlay_active && overlay_interest.is_none());
    if full_output_interest {
        if let Some(m) = output.current_mode() {
            interest.push(layers::skia::Rect::from_wh(
                m.size.w as f32,
                m.size.h as f32,
            ));
        }
    } else {
        // Overlay chrome with known bounds (bar, islands): only damage
        // reaching those rects can change what their blur shows.
        if overlay_active {
            interest.extend(overlay_interest.into_iter().flatten().copied());
            // Popups blur what is under them and are folded into this
            // plane's backdrop, so they are consumers too — but bounded
            // ones. Their own rects are the interest; a window redrawing
            // elsewhere on screen cannot change what a tooltip's blur
            // samples, and treating an open popup as full-output interest
            // (which is what it used to mean) made every client commit
            // anywhere a rebuild trigger.
            interest.extend(popup_interest_rects(engine, popup_root, scene_origin));
        }
        let strip_rect = |el: &Option<SceneDmabufElement>| {
            el.as_ref().map(|el| {
                use smithay::backend::renderer::element::Element as _;
                let geo = el.geometry(smithay::utils::Scale::from(1.0));
                layers::skia::Rect::from_xywh(
                    geo.loc.x as f32,
                    geo.loc.y as f32,
                    geo.size.w as f32,
                    geo.size.h as f32,
                )
            })
        };
        if dock_visible {
            interest.extend(strip_rect(&surface.dock_dmabuf_element));
        }
        if switcher_active {
            interest.extend(strip_rect(&surface.switcher_dmabuf_element));
        }
    }
    let intersects = |r: &layers::skia::Rect| rect_hits_interest(&interest, r);
    let hits_interest = |d: &Option<layers::skia::Rect>| d.is_some_and(|r| intersects(&r));
    // Promoted rects are global scene coords; interest rects are output-local.
    let promoted_hits = promoted_commit
        && promoted.iter().any(|(_, r)| {
            intersects(&r.with_offset((-(scene_origin.0 as f32), -(scene_origin.1 as f32))))
        });
    let any_consumer = !interest.is_empty();
    // Damage that lands entirely outside this output's buffer can never reach
    // any consumer — now or after one activates later — so it must not mark the
    // composite dirty. The common case is a window on a workspace scrolled off
    // screen: it damages the windows subtree every frame it draws, and treating
    // that as "lower planes changed" rebuilt the backdrop (and forced a full
    // re-render of every blur-bearing plane) on every other frame, driven by a
    // window nobody can see.
    let on_screen = |d: &Option<layers::skia::Rect>| {
        d.is_some_and(|r| {
            output.current_mode().is_some_and(|m| {
                let local = r.with_offset((-(scene_origin.0 as f32), -(scene_origin.1 as f32)));
                local.left() < m.size.w as f32
                    && local.right() > 0.0
                    && local.top() < m.size.h as f32
                    && local.bottom() > 0.0
            })
        })
    };
    let lower_damaged = on_screen(&bg_damage)
        || on_screen(&middle_damage)
        || (promoted_commit && !promoted.is_empty());
    // Popups live in the overlay plane and are folded into its backdrop, so a
    // popup opening/closing/animating must rebuild too — its own subtree damage
    // (islands are a separate subtree, so their animations don't trigger this).
    let popup_damage = popup_root.and_then(|r| engine.subtree_damage(r));
    let decision = decide_rebuild(RebuildInputs {
        any_consumer,
        have_backdrop: surface.backdrop_image.is_some(),
        dirty: surface.backdrop_dirty,
        bg_hit: hits_interest(&bg_damage),
        middle_hit: hits_interest(&middle_damage),
        promoted_hits,
        popup_hit: overlay_active && popup_damage.is_some(),
        popup_structural: overlay_active && popup_structural,
        lower_damaged,
        fluid: expose_active || fluid_animation,
        last_desktop_rebuild: surface.last_desktop_rebuild,
        now: std::time::Instant::now(),
    });
    let rebuild = decision.rebuild;
    // Diagnostic (`touch /tmp/otto-perfdbg`): everything that decides a
    // backdrop rebuild this frame, plus the engine's pending transactions —
    // the signal that keeps the render loop out of idle. One line per frame
    // while the toggle exists; for chasing "the compositor never sleeps".
    if std::path::Path::new("/tmp/otto-perfdbg").exists() {
        let mut txs = engine.debug_pending_transactions();
        let tx_count = txs.len();
        txs.truncate(8);
        let txs: Vec<String> = txs
            .into_iter()
            .map(|(node, change, animated)| {
                let mut change = change;
                change.truncate(96);
                format!("n{node}{}:{change}", if animated { "(anim)" } else { "" })
            })
            .collect();
        tracing::info!(
            target: "otto::perfdbg",
            "rebuild={rebuild} dirty={} bg={:?} mid={:?} popup={:?} bg_hit={} mid_hit={} promoted_hits={} popup_struct={popup_structural} overlay_active={overlay_active} tx={tx_count} {txs:?}",
            surface.backdrop_dirty,
            bg_damage,
            middle_damage,
            popup_damage,
            hits_interest(&bg_damage),
            hits_interest(&middle_damage),
            promoted_hits,
        );
    }
    surface.backdrop_dirty = decision.dirty_after;
    if let Some(t) = decision.stamp_desktop_rebuild {
        surface.last_desktop_rebuild = Some(t);
    }
    // The bg-only snapshot is cached across rebuilds, so it needs its own
    // change signal: any background damage, whether or not this frame rebuilds
    // (and whether or not it intersects a consumer — a wallpaper change that
    // misses the dock strip still has to reach a window titlebar's blur).
    if on_screen(&bg_damage) {
        surface.backdrop_bg_dirty = true;
    }
    if rebuild {
        let bg_img = surface
            .scene_dmabuf_element
            .as_ref()
            .and_then(|el| el.snapshot());
        let ctx = surface
            .scene_dmabuf_element
            .as_ref()
            .and_then(|el| el.gr_context());
        if let (Some(bg_img), Some(mut ctx)) = (bg_img, ctx) {
            let (out_w, out_h) = output
                .current_mode()
                .map(|m| (m.size.w, m.size.h))
                .unwrap_or((bg_img.width(), bg_img.height()));
            let (bw, bh) = (
                ((out_w as f32 * BACKDROP_SCALE) as i32).max(1),
                ((out_h as f32 * BACKDROP_SCALE) as i32).max(1),
            );
            if surface.backdrop_surface.is_none() {
                let image_info = layers::skia::ImageInfo::new(
                    (bw, bh),
                    layers::skia::ColorType::RGBA8888,
                    layers::skia::AlphaType::Premul,
                    None,
                );
                surface.backdrop_surface = layers::skia::gpu::surfaces::render_target(
                    &mut ctx,
                    layers::skia::gpu::Budgeted::No,
                    &image_info,
                    None,
                    layers::skia::gpu::SurfaceOrigin::TopLeft,
                    None,
                    false,
                    false,
                )
                .map(|surface| super::types::BackdropSurface {
                    surface,
                    context: ctx.clone(),
                });
            }
            if let Some(bs) = surface.backdrop_surface.as_mut() {
                let dst = layers::skia::Rect::from_xywh(0.0, 0.0, bw as f32, bh as f32);
                let sampling = layers::skia::SamplingOptions::new(
                    layers::skia::FilterMode::Linear,
                    layers::skia::MipmapMode::None,
                );
                let paint = layers::skia::Paint::default();
                // Stage 1: bg only — the MIDDLE plane's backdrop (windows or
                // expose). Both carry blur: a window titlebar in the windows
                // plane, and the hover label in the expose plane. Their buffers
                // hold no wallpaper, so without this seed they blur transparent
                // pixels and come out flat grey.
                {
                    let canvas = bs.surface.canvas();
                    canvas.clear(layers::skia::Color4f::new(0.0, 0.0, 0.0, 1.0));
                    canvas.draw_image_rect_with_sampling_options(
                        &bg_img, None, dst, sampling, &paint,
                    );
                }
                // Snapshot it only when the background actually changed. A
                // consumer re-renders its FULL buffer whenever its backdrop's
                // unique_id changes, so a fresh snapshot per rebuild would turn
                // every window animation into a full windows-plane redraw.
                if surface.backdrop_bg_image.is_none() || surface.backdrop_bg_dirty {
                    bs.context.flush_and_submit();
                    let bg_small = bs.surface.image_snapshot();
                    // Pre-blur so a plain blur layer seeds it and skips its own
                    // (shape-clipped, rim-leaving) blur. The raw copy goes along
                    // for `blur_include_content` layers — window titlebars and
                    // the expose hover label — which blur this plus whatever the
                    // same pass already painted behind them.
                    let blurred = blur_image(&bg_small, &mut bs.context, BACKDROP_BLUR_SIGMA);
                    surface.backdrop_bg_preblurred = blurred.is_some();
                    surface.backdrop_bg_image = Some(blurred.unwrap_or_else(|| bg_small.clone()));
                    surface.backdrop_bg_raw = Some(bg_small);
                    surface.backdrop_bg_dirty = false;
                }
                let middle_backdrop = surface.backdrop_bg_image.clone().map(|img| {
                    (
                        img,
                        BACKDROP_SCALE,
                        surface.backdrop_bg_preblurred,
                        surface.backdrop_bg_raw.clone(),
                    )
                });
                if let Some(el) = middle_el {
                    el.set_backdrop(middle_backdrop);
                }
                // Render the middle plane now, with its fresh backdrop.
                if let Some(el) = middle_el {
                    el.render(renderer.as_mut());
                }
                // Stage 2: + middle plane — the overlay backdrop.
                if let Some(middle_img) = middle_el.and_then(|el| el.snapshot()) {
                    let canvas = bs.surface.canvas();
                    canvas.draw_image_rect_with_sampling_options(
                        &middle_img,
                        None,
                        dst,
                        sampling,
                        &paint,
                    );
                }
                // + the promoted window's own plane, which sits above the
                // middle plane and below all chrome. Rendered here, with the
                // composite-so-far as its backdrop, so a window carrying a
                // `BackgroundBlur` material still blurs the desktop beneath
                // it across the plane boundary. The seed is handed over raw
                // (not pre-blurred): the window blurs it itself, clipped to
                // its own rounded shape.
                if let Some(el) = window_el {
                    let seed = bs.surface.image_snapshot();
                    el.set_backdrop(Some((seed, BACKDROP_SCALE, false, None)));
                    el.render(renderer.as_mut());
                    if let Some(img) = el.snapshot() {
                        let (px, py) = el.position;
                        let (pw, ph) = el.size();
                        let win_dst = layers::skia::Rect::from_xywh(
                            px as f32 * BACKDROP_SCALE,
                            py as f32 * BACKDROP_SCALE,
                            pw as f32 * BACKDROP_SCALE,
                            ph as f32 * BACKDROP_SCALE,
                        );
                        let canvas = bs.surface.canvas();
                        canvas.draw_image_rect_with_sampling_options(
                            &img, None, win_dst, sampling, &paint,
                        );
                    }
                }
                // + promoted windows: their content_layer is hidden, so the
                // middle snapshot only has their shadows. Blit each client
                // dmabuf (the buffer KMS is scanning out) on top — buffer
                // reuse, not a re-render.
                for (dmabuf, rect) in promoted {
                    use smithay::backend::renderer::ImportDma as _;
                    let img = match renderer.as_mut().import_dmabuf(dmabuf, None) {
                        Ok(tex) => tex.image,
                        Err(e) => {
                            tracing::debug!(
                                target: "otto::planes",
                                "backdrop: promoted dmabuf import failed: {e:?}"
                            );
                            continue;
                        }
                    };
                    let win_dst = layers::skia::Rect::from_xywh(
                        (rect.left() - scene_origin.0 as f32) * BACKDROP_SCALE,
                        (rect.top() - scene_origin.1 as f32) * BACKDROP_SCALE,
                        rect.width() * BACKDROP_SCALE,
                        rect.height() * BACKDROP_SCALE,
                    );
                    let canvas = bs.surface.canvas();
                    canvas.draw_image_rect_with_sampling_options(
                        &img, None, win_dst, sampling, &paint,
                    );
                }
                bs.context.flush_and_submit();
                // The unblurred desktop composite — the "backdrop cache". Blur it
                // once for the dock/switcher planes (they must not show popups).
                let desktop = bs.surface.image_snapshot();
                surface.backdrop_raw_image = Some(desktop.clone());
                let desktop_blurred = blur_image(&desktop, &mut bs.context, BACKDROP_BLUR_SIGMA);
                surface.backdrop_preblurred = desktop_blurred.is_some();
                surface.backdrop_image = Some(desktop_blurred.unwrap_or_else(|| desktop.clone()));

                // The overlay plane hosts stacked popups: a submenu must blur the
                // popup(s) beneath it. Draw the popup subtree on top of the desktop
                // cache and blur the WHOLE image — the blur happens before any
                // per-popup clip, so the popups seed this pre-blurred image and
                // skip their own shape-clipped blur (no faded rim, like islands),
                // yet still see the popups underneath. No popups → reuse the
                // desktop blur so islands keep their usual vibrancy.
                let has_popups = popup_root
                    .and_then(|r| engine.get_layer(&r))
                    .is_some_and(|l| !l.children().is_empty());
                let overlay_src = if has_popups {
                    draw_popups(bs, engine, popup_root.unwrap(), scene_origin);
                    bs.context.flush_and_submit();
                    bs.surface.image_snapshot()
                } else {
                    desktop
                };
                let overlay_blurred =
                    blur_image(&overlay_src, &mut bs.context, BACKDROP_BLUR_SIGMA);
                surface.backdrop_overlay_image = Some(overlay_blurred.unwrap_or(overlay_src));
            }
        } else if let Some(el) = middle_el {
            // No bg snapshot/context yet (first frames) — still render
            // the middle plane so the stack stays warm.
            el.render(renderer.as_mut());
        }
    } else if let Some(el) = middle_el {
        // No rebuild this frame: the middle plane is not in `interest` (its
        // blur is bounded by the windows, which move constantly — making it a
        // rebuild trigger would rebuild the composite every frame). Re-hand it
        // the cached bg snapshot instead: same image, same unique_id, so this
        // costs nothing unless the plane had none yet.
        el.set_backdrop(surface.backdrop_bg_image.clone().map(|img| {
            (
                img,
                BACKDROP_SCALE,
                surface.backdrop_bg_preblurred,
                surface.backdrop_bg_raw.clone(),
            )
        }));
        el.render(renderer.as_mut());
    }

    // All blur-bearing upper planes consume the same composite; each
    // re-renders only when the snapshot's unique_id changes or its own
    // subtree is damaged.
    let preblurred = surface.backdrop_preblurred;
    // Overlay plane: desktop + popups (falls back to desktop-only before the
    // first build). Dock/switcher: desktop only.
    // The raw copy is what `blur_include_content` layers (stacked popups) use:
    // they blur raw desktop + the same-pass content painted behind them, so a
    // submenu blurs the menu it overlaps instead of letting it show through
    // sharp. Everything else in the plane seeds the pre-blurred image.
    let overlay_raw = surface.backdrop_raw_image.clone();
    let overlay_backdrop = surface
        .backdrop_overlay_image
        .clone()
        .or_else(|| surface.backdrop_image.clone())
        .map(|img| (img, BACKDROP_SCALE, preblurred, overlay_raw));
    let upper_backdrop = surface
        .backdrop_image
        .clone()
        .map(|img| (img, BACKDROP_SCALE, preblurred, None));
    if let Some(el) = &surface.overlay_dmabuf_element {
        el.set_backdrop(overlay_backdrop);
        if overlay_active {
            let dmg = el.subtree_damage();
            let rendered = el.render(renderer.as_mut());
            // Per-frame — keep out of the default (info) filter: at idle this
            // was ~30 lines/sec of disk writes into session.log.
            tracing::debug!(
                target: "otto::planes",
                "overlay plane: subtree_damage={:?} rendered={}",
                dmg,
                rendered
            );
        }
    }
    if let Some(el) = &surface.switcher_dmabuf_element {
        el.set_backdrop(upper_backdrop.clone());
        if switcher_active {
            el.render(renderer.as_mut());
        }
    }
    if let Some(el) = &surface.dock_dmabuf_element {
        el.set_backdrop(upper_backdrop);
        if dock_visible {
            el.render(renderer.as_mut());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn rect(l: f32, t: f32, r: f32, b: f32) -> layers::skia::Rect {
        layers::skia::Rect::new(l, t, r, b)
    }

    /// Steady-state inputs: composite exists, chrome consumers on screen,
    /// no damage anywhere.
    fn quiet(now: Instant) -> RebuildInputs {
        RebuildInputs {
            any_consumer: true,
            have_backdrop: true,
            dirty: false,
            bg_hit: false,
            middle_hit: false,
            promoted_hits: false,
            popup_hit: false,
            popup_structural: false,
            lower_damaged: false,
            fluid: false,
            last_desktop_rebuild: None,
            now,
        }
    }

    #[test]
    fn narrowed_interest_ignores_damage_below_the_chrome_band() {
        // The regression: overlay chrome made the interest the FULL output,
        // so a window redrawing anywhere forced a rebuild. With the interest
        // narrowed to the chrome rects, damage below the band must not hit.
        let chrome_band = [rect(0.0, 0.0, 2880.0, 280.0)];
        let window_damage_low = rect(400.0, 600.0, 2000.0, 1500.0);
        let window_damage_high = rect(400.0, 10.0, 2000.0, 1500.0);
        assert!(!rect_hits_interest(&chrome_band, &window_damage_low));
        assert!(rect_hits_interest(&chrome_band, &window_damage_high));
    }

    #[test]
    fn desktop_damage_rebuilds_are_rate_limited() {
        // An animating client damages the interest region every frame; only
        // one rebuild per DESKTOP_REBUILD_MIN_INTERVAL may result.
        let t0 = Instant::now();
        let frame = Duration::from_millis(8); // ~120 Hz
        let mut last: Option<Instant> = None;
        let mut dirty = false;
        let mut rebuilds = 0u32;
        for n in 0..250 {
            let now = t0 + frame * n;
            let d = decide_rebuild(RebuildInputs {
                dirty,
                middle_hit: true,
                lower_damaged: true,
                last_desktop_rebuild: last,
                now,
                ..quiet(now)
            });
            dirty = d.dirty_after;
            if let Some(t) = d.stamp_desktop_rebuild {
                last = Some(t);
            }
            if d.rebuild {
                rebuilds += 1;
            }
        }
        // 250 frames * 8ms = 2s → at 100ms spacing at most 21 rebuilds
        // (first one is free). Well under the 250 the storm produced.
        assert!(
            rebuilds <= 21,
            "rebuilds themselves rate-limited: {rebuilds}"
        );
        assert!(rebuilds >= 15, "still rebuilding regularly: {rebuilds}");
    }

    #[test]
    fn dirty_flag_waits_for_the_rate_limit() {
        // The flip-flop regression: rebuild consumed dirty, next frame's
        // damage set it again → rebuild every other frame. Dirty must wait
        // out the interval, not bypass it.
        let t0 = Instant::now();
        let d = decide_rebuild(RebuildInputs {
            dirty: true,
            lower_damaged: true,
            last_desktop_rebuild: Some(t0 - Duration::from_millis(20)),
            now: t0,
            ..quiet(t0)
        });
        assert!(!d.rebuild, "dirty must not bypass the rate limit");
        assert!(d.dirty_after, "staleness must be carried, not dropped");

        let d = decide_rebuild(RebuildInputs {
            dirty: true,
            last_desktop_rebuild: Some(t0 - DESKTOP_REBUILD_MIN_INTERVAL),
            now: t0,
            ..quiet(t0)
        });
        assert!(d.rebuild, "carried dirt rebuilds once the interval elapses");
        assert!(!d.dirty_after);
    }

    #[test]
    fn interactive_triggers_bypass_the_rate_limit() {
        let t0 = Instant::now();
        // A popup appearing/moving: immediate, even right after a rebuild —
        // its blur must be right on the first frame it is visible.
        let d = decide_rebuild(RebuildInputs {
            popup_structural: true,
            last_desktop_rebuild: Some(t0 - Duration::from_millis(1)),
            now: t0,
            ..quiet(t0)
        });
        assert!(d.rebuild, "a structural popup change rebuilds immediately");
        // Missing composite: immediate.
        let d = decide_rebuild(RebuildInputs {
            have_backdrop: false,
            last_desktop_rebuild: Some(t0 - Duration::from_millis(1)),
            now: t0,
            ..quiet(t0)
        });
        assert!(d.rebuild, "first build is never deferred");
    }

    #[test]
    fn popup_repaints_are_rate_limited_like_any_other_damage() {
        // The regression this replaces: popup subtree damage bypassed the
        // rate limit outright, so a client repainting under (or near) a popup
        // forced a full-screen composite + blur + a re-render of every
        // blur-bearing plane at client frame rate. Measured at 23 rebuilds/s
        // against a 10/s cap with a tooltip up.
        let t0 = Instant::now();
        let frame = Duration::from_millis(8); // ~120 Hz
        let mut last: Option<Instant> = None;
        let mut dirty = false;
        let mut rebuilds = 0u32;
        for n in 0..250 {
            let now = t0 + frame * n;
            let d = decide_rebuild(RebuildInputs {
                dirty,
                popup_hit: true,
                last_desktop_rebuild: last,
                now,
                ..quiet(now)
            });
            dirty = d.dirty_after;
            if let Some(t) = d.stamp_desktop_rebuild {
                last = Some(t);
            }
            if d.rebuild {
                rebuilds += 1;
            }
        }
        assert!(rebuilds <= 21, "popup repaints rate-limited: {rebuilds}");
        assert!(
            rebuilds >= 15,
            "the popup still refreshes regularly: {rebuilds}"
        );
    }

    #[test]
    fn a_rate_limited_popup_repaint_is_not_forgotten() {
        // Popup damage held back by the limit has to survive as staleness,
        // or the blur under the popup keeps whatever it had until unrelated
        // desktop damage happens to arrive.
        let t0 = Instant::now();
        let d = decide_rebuild(RebuildInputs {
            popup_hit: true,
            last_desktop_rebuild: Some(t0 - Duration::from_millis(20)),
            now: t0,
            ..quiet(t0)
        });
        assert!(!d.rebuild);
        assert!(d.dirty_after, "the deferred popup repaint is carried");
    }

    #[test]
    fn fluid_animation_suspends_the_rate_limit() {
        // During expose or a workspace swipe the blur must track the moving
        // content per frame — a 10 Hz blur under a 120 Hz scroll reads as
        // judder — so damage rebuilds every frame while `fluid` holds.
        let t0 = Instant::now();
        let d = decide_rebuild(RebuildInputs {
            middle_hit: true,
            lower_damaged: true,
            fluid: true,
            last_desktop_rebuild: Some(t0 - Duration::from_millis(1)),
            now: t0,
            ..quiet(t0)
        });
        assert!(d.rebuild, "fluid animations rebuild every damaged frame");
    }

    #[test]
    fn no_consumer_never_rebuilds_but_marks_dirty() {
        let t0 = Instant::now();
        let d = decide_rebuild(RebuildInputs {
            any_consumer: false,
            middle_hit: true,
            lower_damaged: true,
            now: t0,
            ..quiet(t0)
        });
        assert!(!d.rebuild);
        assert!(
            d.dirty_after,
            "a later-activating consumer needs fresh content"
        );
    }

    #[test]
    fn quiet_frames_do_nothing() {
        let t0 = Instant::now();
        let d = decide_rebuild(quiet(t0));
        assert!(!d.rebuild);
        assert!(!d.dirty_after);
        assert!(d.stamp_desktop_rebuild.is_none());
    }
}
