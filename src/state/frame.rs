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
#[derive(Debug, Default)]
pub struct FramePacing {
    /// Throttle tier of every mapped window, keyed by window id. A window
    /// missing from the map is paced at full rate.
    #[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
    pub windows: HashMap<ObjectId, WindowThrottleState>,
    /// `background`/`bottom` layer surfaces hidden behind a window, given the
    /// occluded trickle (see `window_throttle::occluded_layer_surface_ids`).
    #[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
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
    #[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
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
) -> Option<OutputPresentationFeedback> {
    post_repaint(
        output,
        render_element_states,
        window_elements,
        dmabuf_feedback,
        time,
        pacing,
    );

    if !rendered {
        return None;
    }
    let mut feedback = take_presentation_feedback(output, window_elements, render_element_states);
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

#[profiling::function]
fn post_repaint(
    output: &Output,
    render_element_states: &RenderElementStates,
    window_elements: &[&WindowElement],
    dmabuf_feedback: Option<SurfaceDmabufFeedback<'_>>,
    time: impl Into<Duration>,
    pacing: &FramePacing,
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
        let throttle = pacing
            .windows
            .get(&window.id())
            .map(|s| s.throttle())
            .unwrap_or(default_throttle);
        window.send_frame(output, time, Some(throttle), |_, _| Some(output.clone()));
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

        // Background/bottom surfaces hidden behind a window get the same 2 Hz
        // trickle as an occluded window (see `occluded_layer_surface_ids`);
        // everything else paints at full rate.
        let layer_throttle = if pacing
            .occluded_layers
            .contains(&layer_surface.wl_surface().id())
        {
            WindowThrottleState::Occluded.throttle()
        } else {
            Duration::ZERO
        };
        layer_surface.send_frame(
            output,
            time,
            Some(layer_throttle),
            surface_primary_scanout_output,
        );
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
) -> OutputPresentationFeedback {
    let mut output_presentation_feedback = OutputPresentationFeedback::new(output);

    window_elements.iter().for_each(|window| {
        // Process all windows since we're handling all workspaces
        window.take_presentation_feedback(
            &mut output_presentation_feedback,
            surface_primary_scanout_output,
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
