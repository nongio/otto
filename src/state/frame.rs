//! The per-frame step every rendering backend runs around drawing an output.
//!
//! Each backend draws its own way, but what clients are told afterwards is
//! the same everywhere: every window is classified into a frame-callback
//! throttle tier ([`FramePacing::classify`]), then [`frame_done`] sends the
//! frame callbacks and dmabuf feedback ([`post_repaint`]) and collects the
//! `wp_presentation` feedback ([`take_presentation_feedback`]).
//!
//! What genuinely differs between backends is a parameter, documented where
//! it is taken:
//!
//! - occlusion: only the DRM backend classifies windows as occluded;
//! - capture: only the DRM backend has a per-frame screencast tap;
//! - dmabuf feedback: only the DRM backend has scanout planes to advertise;
//! - presentation timing: the DRM backend presents with the page flip, the
//!   nested backends right away ([`Presentation`]).
//!
//! The headless backend renders nothing and is not a caller: without render
//! element states there is no primary scanout output to pace by, so it sends
//! plain unthrottled frame callbacks of its own (`headless::send_frames`).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::element::{
        default_primary_scanout_output_compare, utils::select_dmabuf_feedback, RenderElementStates,
    },
    desktop::utils::{
        surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
        update_surface_primary_scanout_output, OutputPresentationFeedback,
    },
    output::Output,
    reexports::{
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::{backend::ObjectId, Resource},
    },
    utils::{Logical, Monotonic, Rectangle, Time},
    wayland::{
        dmabuf::DmabufFeedback, fractional_scale::with_fractional_scale, presentation::Refresh,
    },
};

use super::window_throttle::{self, WindowThrottleState};
use crate::{shell::WindowElement, workspaces::Workspaces};

#[derive(Debug, Copy, Clone)]
pub struct SurfaceDmabufFeedback<'a> {
    pub render_feedback: &'a DmabufFeedback,
    pub scanout_feedback: &'a DmabufFeedback,
}

/// How fast each surface on an output is given frame callbacks this frame.
#[derive(Debug, Default, Clone)]
pub struct FramePacing {
    /// Throttle tier of every mapped window, keyed by window id. A window
    /// missing from the map is paced at full rate.
    pub windows: HashMap<ObjectId, WindowThrottleState>,
    /// `background`/`bottom` layer surfaces hidden behind a window, given the
    /// occluded trickle (see `window_throttle::occluded_layer_surface_ids`).
    pub occluded_layers: HashSet<ObjectId>,
}

impl FramePacing {
    /// Classify every window in `window_elements`, and the layer surfaces on
    /// `output`, into their frame-callback tiers.
    ///
    /// Takes the compositor's fields rather than the whole state so a backend
    /// can call it while holding a mutable borrow of its own backend data.
    ///
    /// - `classify_occlusion`: whether a window fully covered by an opaque
    ///   one is demoted to the occluded tier. The DRM backend passes `true`;
    ///   the winit and X11 backends have always passed `false`, so there a
    ///   covered window stays at the secondary rate. Layer-surface occlusion
    ///   is classified either way.
    /// - `captured_ids`: windows being screencast, pinned to full rate. Only
    ///   the DRM backend serves screencasts from its frames; the nested
    ///   backends pass an empty set.
    pub fn classify(
        workspaces: &Workspaces,
        output: &Output,
        window_elements: &[&WindowElement],
        background_effects: &HashMap<ObjectId, (Rectangle<i32, Logical>, i32)>,
        pointer_interaction: &Option<(ObjectId, Instant)>,
        classify_occlusion: bool,
        captured_ids: &HashSet<ObjectId>,
    ) -> Self {
        let expose_active = workspaces.mirrors_active();
        let effect_surfaces: HashSet<_> = background_effects.keys().cloned().collect();
        let translucent_ids =
            window_throttle::translucent_window_ids(window_elements, &effect_surfaces);
        let occluded_ids = if classify_occlusion {
            workspaces.occluded_window_ids(&translucent_ids)
        } else {
            HashSet::new()
        };
        let interacting_ids = window_throttle::interacting_ids(pointer_interaction);
        let windows = window_throttle::classify_windows(
            workspaces,
            window_elements,
            &occluded_ids,
            expose_active,
            captured_ids,
            &interacting_ids,
        );
        let occluded_layers = window_throttle::occluded_layer_surface_ids(
            workspaces,
            output,
            expose_active,
            &translucent_ids,
        );
        Self {
            windows,
            occluded_layers,
        }
    }
}

/// When the frame just drawn reaches the screen, for [`frame_done`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Presentation {
    /// The host shows the frame as soon as it is submitted (winit, X11): the
    /// presentation feedback is marked presented now, at the output's nominal
    /// refresh, with no hardware sequence number.
    Immediate,
    /// The frame is shown on a later page flip (DRM): the feedback is handed
    /// back to be queued with the frame and presented with the flip's
    /// hardware timestamp.
    OnPageFlip,
}

/// Tell clients an output's frame is done: send every window and layer
/// surface on `output` its (throttled) frame callback and dmabuf feedback,
/// and, if `rendered`, collect the windows' presentation feedback.
///
/// Frame callbacks go out whether or not anything was rendered — a client
/// waiting on one must not stall because the scene had no damage.
///
/// With [`Presentation::Immediate`] the feedback is presented here and `None`
/// is returned; with [`Presentation::OnPageFlip`] it is returned (when
/// `rendered`) for the caller to queue with the frame.
///
/// `send_callbacks` false skips the `wl_callback`s: the udev loop sends them
/// from its VBlank handler instead ([`send_frame_callbacks`]), and here only
/// when no page flip is coming to do it.
#[allow(clippy::too_many_arguments)]
pub fn frame_done(
    output: &Output,
    render_element_states: &RenderElementStates,
    window_elements: &[&WindowElement],
    dmabuf_feedback: Option<SurfaceDmabufFeedback<'_>>,
    time: Time<Monotonic>,
    pacing: &FramePacing,
    rendered: bool,
    presentation: Presentation,
    send_callbacks: bool,
) -> Option<OutputPresentationFeedback> {
    post_repaint(
        output,
        render_element_states,
        window_elements,
        dmabuf_feedback,
        time,
        pacing,
        send_callbacks,
    );

    if !rendered {
        return None;
    }
    let mut feedback =
        take_presentation_feedback(output, window_elements, render_element_states, pacing);
    match presentation {
        Presentation::OnPageFlip => Some(feedback),
        Presentation::Immediate => {
            feedback.presented(
                time,
                nominal_refresh(output),
                0,
                wp_presentation_feedback::Kind::Vsync,
            );
            None
        }
    }
}

/// The output's refresh interval from its current mode (mHz → ns).
fn nominal_refresh(output: &Output) -> Refresh {
    output
        .current_mode()
        .map(|mode| {
            Refresh::fixed(Duration::from_nanos(
                1_000_000_000_000 / mode.refresh as u64,
            ))
        })
        .unwrap_or(Refresh::Unknown)
}

/// Send every window and layer surface on `output` its (throttled) frame
/// callback, and nothing else.
///
/// The udev loop calls this from the VBlank handler, as early in the frame
/// period as a client can be told to paint. The compositor itself renders
/// late (see `docs/developer/render_loop.md`), so a client woken at render
/// time would have only the compositor's own budget left to produce the next
/// buffer: a GTK window spending 4 ms on the GPU missed one VBlank in six at
/// 120 Hz. Woken at VBlank it has the whole period minus that budget.
#[profiling::function]
pub fn send_frame_callbacks(
    output: &Output,
    window_elements: &[&WindowElement],
    time: impl Into<Duration>,
    pacing: &FramePacing,
    log: &mut FrameCallbackLog,
) {
    let time = time.into();
    let map = smithay::desktop::layer_map_for_output(output);
    log.retain(|id, _| {
        window_elements.iter().any(|w| w.id() == *id)
            || map.layers().any(|l| l.wl_surface().id() == *id)
    });
    for window in window_elements {
        let throttle = pacing
            .windows
            .get(&window.id())
            .map(|s| s.throttle())
            .unwrap_or(Duration::ZERO);
        window.send_frame(output, time, Some(throttle), |_, _| Some(output.clone()));
        log.insert(window.id(), time);
    }
    for layer_surface in map.layers() {
        layer_surface.send_frame(
            output,
            time,
            Some(layer_throttle(pacing, layer_surface)),
            surface_primary_scanout_output,
        );
        log.insert(layer_surface.wl_surface().id(), time);
    }
}

/// When each toplevel or layer surface (by its root `wl_surface` id) was
/// last offered its frame callbacks. See [`frame_callback_on_commit`].
pub type FrameCallbackLog = HashMap<ObjectId, Duration>;

/// A commit that arrives after this frame's VBlank send: give the surface
/// its frame callback now if it has gone most of a period without one.
///
/// A client's buffer commit is held back until the buffer's GPU work is
/// done (`add_blocker` in the shell). A client that needs most of the period
/// to render lands its commit after the VBlank handler has already sent the
/// callbacks, so with VBlank sends alone it would get its callback one whole
/// VBlank later and lock to half the refresh rate (GTK at 120 Hz: 60 fps).
/// Sending on the applied commit lets it pipeline: it starts the next frame
/// while the compositor is still to compose this one. The gap keeps every
/// client on the VBlank cadence: only a surface that has already waited a
/// whole period (its commit just missed this VBlank's send) is served here;
/// anything sooner waits for the next VBlank. A shorter gap let a client
/// cycle faster than the refresh, two of its frames then met one compose
/// and the next compose found none: dropped frames, not extra ones.
pub fn frame_callback_on_commit<B: crate::state::Backend>(
    state: &mut crate::state::Otto<B>,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) {
    use smithay::wayland::compositor::get_parent;
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    let outputs: Vec<Output> = state.workspaces.outputs().cloned().collect();
    let Some(first) = outputs.first() else {
        return;
    };
    let now: Duration = state.clock.now().into();
    let period = first
        .current_mode()
        .map(|mode| Duration::from_nanos(1_000_000_000_000 / mode.refresh as u64))
        .unwrap_or(Duration::from_micros(16_667));
    // Just under a period: the VBlank handler runs a little after the VBlank
    // itself, and a commit applied in that sliver has waited one full period.
    let min_gap = period.mul_f32(0.95);
    let key = root.id();
    if let Some(last) = state.frame_callback_log.get(&key).copied() {
        if now.saturating_sub(last) < min_gap {
            // Too soon: the VBlank cadence (the handler, or its tick when
            // no flip happened) delivers this one.
            return;
        }
    }
    send_frame_callbacks_to(state, &key, now);
}

/// Send the frame callbacks of one toplevel or layer surface (by root id)
/// on every output, and log the time.
fn send_frame_callbacks_to<B: crate::state::Backend>(
    state: &mut crate::state::Otto<B>,
    key: &ObjectId,
    now: Duration,
) {
    let outputs: Vec<Output> = state.workspaces.outputs().cloned().collect();
    if let Some(window) = state.workspaces.get_window_for_surface(key).cloned() {
        state.frame_callback_log.insert(key.clone(), now);
        for output in &outputs {
            window.send_frame(output, now, None, |_, _| Some(output.clone()));
        }
        return;
    }
    for output in &outputs {
        let map = smithay::desktop::layer_map_for_output(output);
        let layer = map
            .layers()
            .find(|layer| layer.wl_surface().id() == *key)
            .cloned();
        if let Some(layer) = layer {
            state.frame_callback_log.insert(key.clone(), now);
            layer.send_frame(output, now, None, surface_primary_scanout_output);
            return;
        }
    }
}

/// Background/bottom surfaces hidden behind a window get the same 2 Hz
/// trickle as an occluded window (see `occluded_layer_surface_ids`);
/// everything else paints at full rate.
fn layer_throttle(
    pacing: &FramePacing,
    layer_surface: &smithay::desktop::LayerSurface,
) -> Duration {
    if pacing
        .occluded_layers
        .contains(&layer_surface.wl_surface().id())
    {
        WindowThrottleState::Occluded.throttle()
    } else {
        Duration::ZERO
    }
}

#[profiling::function]
fn post_repaint(
    output: &Output,
    render_element_states: &RenderElementStates,
    window_elements: &[&WindowElement],
    dmabuf_feedback: Option<SurfaceDmabufFeedback<'_>>,
    time: impl Into<Duration>,
    pacing: &FramePacing,
    send_callbacks: bool,
) {
    let time = time.into();
    let default_throttle = Duration::ZERO;

    window_elements.iter().for_each(|window| {
        window.with_surfaces(|surface, states| {
            let primary_scanout_output = update_surface_primary_scanout_output(
                surface,
                output,
                states,
                None,
                render_element_states,
                default_primary_scanout_output_compare,
            );

            if let Some(output) = primary_scanout_output {
                with_fractional_scale(states, |fraction_scale| {
                    fraction_scale.set_preferred_scale(output.current_scale().fractional_scale());
                });
            }
        });

        // Per-window throttle based on user-visibility classification. Missing
        // entries (should be rare) fall through to full-rate, matching the
        // previous behaviour.
        if send_callbacks {
            let throttle = pacing
                .windows
                .get(&window.id())
                .map(|s| s.throttle())
                .unwrap_or(default_throttle);
            window.send_frame(output, time, Some(throttle), |_, _| Some(output.clone()));
        }
        // Send frame to all windows since we're processing all workspaces
        if let Some(dmabuf_feedback) = dmabuf_feedback {
            window.send_dmabuf_feedback(output, surface_primary_scanout_output, |surface, _| {
                select_dmabuf_feedback(
                    surface,
                    render_element_states,
                    dmabuf_feedback.render_feedback,
                    dmabuf_feedback.scanout_feedback,
                )
            });
        }
    });
    let map = smithay::desktop::layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.with_surfaces(|surface, states| {
            let primary_scanout_output = update_surface_primary_scanout_output(
                surface,
                output,
                states,
                None,
                render_element_states,
                default_primary_scanout_output_compare,
            );

            if let Some(output) = primary_scanout_output {
                with_fractional_scale(states, |fraction_scale| {
                    fraction_scale.set_preferred_scale(output.current_scale().fractional_scale());
                });
            }
        });

        if send_callbacks {
            layer_surface.send_frame(
                output,
                time,
                Some(layer_throttle(pacing, layer_surface)),
                surface_primary_scanout_output,
            );
        }
        if let Some(dmabuf_feedback) = dmabuf_feedback {
            layer_surface.send_dmabuf_feedback(
                output,
                surface_primary_scanout_output,
                |surface, _| {
                    select_dmabuf_feedback(
                        surface,
                        render_element_states,
                        dmabuf_feedback.render_feedback,
                        dmabuf_feedback.scanout_feedback,
                    )
                },
            );
        }
    }
}

#[profiling::function]
fn take_presentation_feedback(
    output: &Output,
    window_elements: &[&WindowElement],
    render_element_states: &RenderElementStates,
    pacing: &FramePacing,
) -> OutputPresentationFeedback {
    let mut output_presentation_feedback = OutputPresentationFeedback::new(output);

    window_elements.iter().for_each(|window| {
        // Windows are composited through the scene, not as per-surface
        // render elements, so `render_element_states` never names their
        // surfaces and the surface's primary scanout output is never set.
        // Smithay only takes feedback for a surface on its primary output;
        // asking it would leave every window's feedback behind to be
        // `discarded` on the next commit, and a client that never hears
        // `presented` cannot pace itself to the display. A window the user
        // can see was presented by this flip; one that is occluded,
        // minimized or on another workspace was not.
        let shown = pacing.windows.get(&window.id()).is_none_or(|tier| {
            !matches!(
                tier,
                WindowThrottleState::Occluded
                    | WindowThrottleState::Minimized
                    | WindowThrottleState::HiddenWorkspace
            )
        });
        if !shown {
            return;
        }
        window.take_presentation_feedback(
            &mut output_presentation_feedback,
            |_, _| Some(output.clone()),
            |surface, _| {
                surface_presentation_feedback_flags_from_states(
                    surface,
                    None,
                    render_element_states,
                )
            },
        );
    });

    // TODO: layer-shell surfaces do not send presentation feedback yet.

    output_presentation_feedback
}
