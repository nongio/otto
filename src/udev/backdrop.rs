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
//! Two-stage build in one small surface: draw bg → snapshot (the expose
//! backdrop), then draw the middle plane on top → snapshot (the overlay
//! backdrop). Rebuilt only when a lower plane recorded damage intersecting
//! an active consumer's region; the fresh snapshot's unique_id is what
//! makes the consumers re-render.

use smithay::output::Output;

use crate::render_elements::scene_dmabuf_element::SceneDmabufElement;

use super::types::{SurfaceData, UdevRenderer};

const BACKDROP_SCALE: f32 = 0.25;

/// Rebuild the two-stage backdrop composite when needed, render the middle
/// plane (windows or expose), and hand the fresh composite to the
/// blur-bearing upper planes (overlay, switcher, dock), rendering the
/// active ones. The bg plane must already be rendered by the caller.
pub(super) fn update_backdrop_and_upper_planes(
    surface: &mut SurfaceData,
    renderer: &mut UdevRenderer<'_>,
    output: &Output,
    expose_active: bool,
    overlay_active: bool,
    switcher_active: bool,
    dock_visible: bool,
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
    let hits_interest = |d: &Option<layers::skia::Rect>| {
        d.map_or(false, |r| {
            interest.iter().any(|i| {
                r.left() < i.right()
                    && r.right() > i.left()
                    && r.top() < i.bottom()
                    && r.bottom() > i.top()
            })
        })
    };
    let any_consumer = !interest.is_empty();
    let lower_damaged = bg_damage.is_some() || middle_damage.is_some();
    let rebuild = any_consumer
        && (surface.backdrop_image.is_none()
            || surface.backdrop_dirty
            || hits_interest(&bg_damage)
            || hits_interest(&middle_damage));
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
                        expose.set_backdrop(Some((bg_small, BACKDROP_SCALE)));
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
                        &middle_img, None, dst, sampling, &paint,
                    );
                }
                bs.context.flush_and_submit();
                surface.backdrop_image = Some(bs.surface.image_snapshot());
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
    let upper_backdrop = surface
        .backdrop_image
        .clone()
        .map(|img| (img, BACKDROP_SCALE));
    if let Some(el) = &surface.overlay_dmabuf_element {
        el.set_backdrop(upper_backdrop.clone());
        if overlay_active {
            el.render(renderer.as_mut());
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
