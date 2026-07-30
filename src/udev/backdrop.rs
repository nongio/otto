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
//! Staged build in one small surface: draw bg → snapshot (the expose
//! backdrop), draw the middle plane on top → the desktop composite. Blur the
//! desktop for the dock/switcher planes. Then, because popups stack in the
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

use layers::drawing::render_node_tree;
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

/// Gaussian-blur `image` into a fresh same-size GPU surface and return the
/// snapshot. Returns `None` if the surface or filter can't be built (caller
/// falls back to the raw image).
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
    let mut paint = layers::skia::Paint::default();
    paint.set_image_filter(blur);
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
    let middle_damage = middle_el.and_then(|el| el.subtree_damage());
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
    if expose_active || overlay_active {
        if let Some(m) = output.current_mode() {
            interest.push(layers::skia::Rect::from_wh(
                m.size.w as f32,
                m.size.h as f32,
            ));
        }
    } else {
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
    let intersects = |r: &layers::skia::Rect| {
        interest.iter().any(|i| {
            r.left() < i.right()
                && r.right() > i.left()
                && r.top() < i.bottom()
                && r.bottom() > i.top()
        })
    };
    let hits_interest = |d: &Option<layers::skia::Rect>| d.is_some_and(|r| intersects(&r));
    // Promoted rects are global scene coords; interest rects are output-local.
    let promoted_hits = promoted_commit
        && promoted.iter().any(|(_, r)| {
            intersects(&r.with_offset((-(scene_origin.0 as f32), -(scene_origin.1 as f32))))
        });
    let any_consumer = !interest.is_empty();
    let lower_damaged =
        bg_damage.is_some() || middle_damage.is_some() || (promoted_commit && !promoted.is_empty());
    // Popups live in the overlay plane and are folded into its backdrop, so a
    // popup opening/closing/animating must rebuild too — its own subtree damage
    // (islands are a separate subtree, so their animations don't trigger this).
    let popup_damage = popup_root.and_then(|r| engine.subtree_damage(r));
    let rebuild = any_consumer
        && (surface.backdrop_image.is_none()
            || surface.backdrop_dirty
            || hits_interest(&bg_damage)
            || hits_interest(&middle_damage)
            || promoted_hits
            || (overlay_active && popup_damage.is_some()));
    if rebuild {
        surface.backdrop_dirty = false;
    } else if lower_damaged {
        surface.backdrop_dirty = true;
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
                // Stage 1: bg only — the expose backdrop.
                {
                    let canvas = bs.surface.canvas();
                    canvas.clear(layers::skia::Color4f::new(0.0, 0.0, 0.0, 1.0));
                    canvas.draw_image_rect_with_sampling_options(
                        &bg_img, None, dst, sampling, &paint,
                    );
                }
                if expose_active {
                    if let Some(expose) = &surface.expose_dmabuf_element {
                        bs.context.flush_and_submit();
                        let bg_small = bs.surface.image_snapshot();
                        // Pre-blur so expose seeds it and skips its own blur.
                        // The raw copy goes along too: the hover label sits on
                        // top of the window previews painted in this same pass
                        // and carries `blur_include_content`, so it seeds the
                        // raw image and blurs the preview underneath in.
                        match blur_image(&bg_small, &mut bs.context, BACKDROP_BLUR_SIGMA) {
                            Some(blurred) => expose.set_backdrop(Some((
                                blurred,
                                BACKDROP_SCALE,
                                true,
                                Some(bg_small),
                            ))),
                            None => {
                                expose.set_backdrop(Some((bg_small, BACKDROP_SCALE, false, None)))
                            }
                        }
                    }
                }
                // Render the middle plane now (expose renders with its
                // fresh backdrop; windows has no blur).
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
            tracing::info!(
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
