/// Marker stored in `Output::user_data()` to identify virtual outputs.
/// Used to exclude them from window placement, maximize, and output-under-cursor queries.
pub struct VirtualOutputMarker {
    /// When true the output participates in pointer reach / focus /
    /// window placement like a physical screen (config `interactive`).
    pub interactive: bool,
}

/// Returns true if the output is a virtual (PipeWire) output.
pub fn is_virtual_output(output: &smithay::output::Output) -> bool {
    output.user_data().get::<VirtualOutputMarker>().is_some()
}

/// A virtual output the pointer must NOT reach (non-interactive).
pub fn is_unreachable_virtual_output(output: &smithay::output::Output) -> bool {
    output
        .user_data()
        .get::<VirtualOutputMarker>()
        .map(|m| !m.interactive)
        .unwrap_or(false)
}

use smithay::{
    backend::{
        allocator::{gbm::GbmDevice, Fourcc},
        drm::DrmDeviceFd,
        renderer::damage::OutputDamageTracker,
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::wayland_server::backend::GlobalId,
    utils::{Logical, Point, Rectangle, Size},
};

use crate::{
    config::VirtualOutputConfig,
    screenshare::{BackendCapabilities, PipeWireStream, StreamConfig},
};

/// The logical size a virtual output of `resolution` occupies, which is what
/// the arrangement is laid out in — the mode divided by the screen scale, the
/// same figure [`crate::udev::device`] derives for a connector.
pub fn logical_size(config: &VirtualOutputConfig) -> Size<i32, Logical> {
    let screen_scale = crate::config::Config::with(|c| c.screen_scale);
    Size::from((
        (config.resolution.width as f64 / screen_scale) as i32,
        (config.resolution.height as f64 / screen_scale) as i32,
    ))
}

/// The rectangles the mapped outputs occupy, which is what a new one has to
/// stay clear of. See [`placement`].
pub fn mapped_rects<B: crate::state::Backend + 'static>(
    state: &crate::state::Otto<B>,
) -> Vec<Rectangle<i32, Logical>> {
    state
        .workspaces
        .outputs()
        .filter_map(|output| state.workspaces.output_geometry(output))
        .collect()
}

/// Where a virtual output goes, given the outputs already mapped.
///
/// Outputs cannot overlap — there is no mirroring feature, and two screens on
/// the same coordinates make every "which output is under this point" answer
/// arbitrary: the pointer, window placement, and the Displays pane's canvas,
/// where an overlapped screen can no longer be clicked at all and so can
/// never be selected or removed. `udev::device` already applies this rule to
/// a physical connector, rejecting a configured position that overlaps and
/// falling back to auto-placement; a virtual output was exempt from it and
/// defaulted to `(0, 0)`, which is exactly where the laptop panel is.
///
/// So: honour `configured` when it leaves the output clear, otherwise place it
/// to the right of everything else.
pub fn placement(
    configured: Option<Point<i32, Logical>>,
    size: Size<i32, Logical>,
    existing: &[Rectangle<i32, Logical>],
) -> Point<i32, Logical> {
    let clear = |pos: Point<i32, Logical>| {
        let rect = Rectangle::new(pos, size);
        !existing.iter().any(|other| other.overlaps(rect))
    };

    if let Some(pos) = configured.filter(|&pos| clear(pos)) {
        return pos;
    }

    // The right edge of the arrangement rather than the sum of the widths:
    // summing assumes every output starts where the previous one ended, and
    // one placed by config need not.
    let x = existing
        .iter()
        .map(|rect| rect.loc.x + rect.size.w)
        .max()
        .unwrap_or(0);
    Point::from((x, 0))
}

/// Runtime state for one virtual output.
pub struct VirtualOutputState {
    /// The Smithay output (Wayland global, workspace mapping).
    pub output: Output,
    /// The output's Wayland global. Not a guard: dropping the id advertises
    /// the output for the rest of the session regardless, so taking the
    /// output down means handing this to `DisplayHandle::remove_global`.
    pub global: GlobalId,
    /// PipeWire stream receiving rendered frames.
    pub pipewire_stream: PipeWireStream,
    /// Damage tracker for this output (always renders full frames, age=0).
    pub damage_tracker: OutputDamageTracker,
    /// Frame rendered last cycle, awaiting its GPU fence before the dmabuf is
    /// queued to PipeWire. Resolved (non-blocking `is_reached()`) at the start
    /// of the next `render_virtual_outputs` tick so the fence wait never blocks
    /// the main loop / input dispatch.
    pub pending_frame: Option<smithay::backend::renderer::sync::SyncPoint>,
    /// When `pending_frame` was stashed. A fence that never signals would
    /// otherwise stall this output's render loop forever (no new frame is
    /// started while one is pending), so after [`PENDING_FRAME_DEADLINE`] the
    /// frame is handed over regardless.
    pub pending_since: Option<std::time::Instant>,
}

/// How long a rendered frame may wait for its GPU fence before it is queued
/// anyway. Generous compared with a frame (a stalled fence, not a slow GPU, is
/// what this catches); the worst case is one possibly-incomplete frame in the
/// stream, against a stream that otherwise never advances again.
pub const PENDING_FRAME_DEADLINE: std::time::Duration = std::time::Duration::from_millis(500);

impl VirtualOutputState {
    /// Build an `Output` from config (without registering a Wayland global yet).
    ///
    /// The caller is responsible for calling `output.create_global::<D>()` and
    /// storing the returned `GlobalId` in `global`, then calling `finish()`.
    ///
    /// `position` is resolved by [`placement`] against the outputs already
    /// mapped, and is the same point the caller maps the output at — the
    /// output's own state and the workspace mapping disagreeing about where a
    /// screen is puts the pointer and the windows on different arrangements.
    pub fn build_output(config: &VirtualOutputConfig, position: Point<i32, Logical>) -> Output {
        let output = Output::new(
            config.name.clone(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::None,
                make: "Otto".to_string(),
                model: "Virtual".to_string(),
                serial_number: String::new(),
            },
        );

        let mode = Mode {
            size: (
                config.resolution.width as i32,
                config.resolution.height as i32,
            )
                .into(),
            refresh: (config.refresh_hz * 1000.0) as i32,
        };

        let screen_scale = crate::config::Config::with(|c| c.screen_scale);

        output.set_preferred(mode);
        output.change_current_state(
            Some(mode),
            None,
            Some(Scale::Fractional(screen_scale)),
            Some(position),
        );
        output
            .user_data()
            .insert_if_missing(|| VirtualOutputMarker {
                interactive: config.interactive,
            });

        output
    }

    /// Create a PipeWire stream and damage tracker for `output`, then start streaming.
    ///
    /// Returns the state and the PipeWire node ID that clients connect to.
    pub fn start(
        output: Output,
        global: GlobalId,
        config: &VirtualOutputConfig,
        gbm_device: Option<GbmDevice<DrmDeviceFd>>,
        format_modifiers: Vec<u64>,
    ) -> Result<(Self, u32), String> {
        let damage_tracker = OutputDamageTracker::from_output(&output);

        let capabilities = if gbm_device.is_some() {
            BackendCapabilities {
                supports_dmabuf: true,
                formats: vec![Fourcc::Argb8888, Fourcc::Xrgb8888],
                modifiers: format_modifiers.iter().map(|&m| m as i64).collect(),
            }
        } else {
            BackendCapabilities::default()
        };

        let stream_config = StreamConfig {
            width: config.resolution.width,
            height: config.resolution.height,
            framerate_num: config.refresh_hz.round() as u32,
            framerate_denom: 1,
            capabilities,
            gbm_device,
            label: Some(config.name.clone()),
        };

        let mut pipewire_stream = PipeWireStream::new(stream_config);
        let node_id = pipewire_stream
            .start_sync()
            .map_err(|e| format!("Failed to start PipeWire stream for '{}': {e}", config.name))?;

        let state = Self {
            output,
            global,
            pipewire_stream,
            damage_tracker,
            pending_frame: None,
            pending_since: None,
        };

        Ok((state, node_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    /// One 1080p screen, the size every virtual output in these tests has.
    fn size() -> Size<i32, Logical> {
        Size::from((1920, 1080))
    }

    #[test]
    fn the_first_output_starts_at_the_origin() {
        assert_eq!(placement(None, size(), &[]), Point::from((0, 0)));
    }

    #[test]
    fn an_unpositioned_output_goes_beside_the_screen_it_would_have_covered() {
        // The bug this exists for: with no position of its own a virtual
        // output landed on (0, 0), exactly over the laptop panel, where the
        // Displays pane could no longer click it.
        let panel = rect(0, 0, 1920, 1080);
        assert_eq!(placement(None, size(), &[panel]), Point::from((1920, 0)));
    }

    #[test]
    fn each_one_goes_past_the_last() {
        let existing = [rect(0, 0, 1920, 1080), rect(1920, 0, 1920, 1080)];
        assert_eq!(placement(None, size(), &existing), Point::from((3840, 0)));
    }

    #[test]
    fn a_position_clear_of_everything_is_honoured() {
        let panel = rect(0, 0, 1920, 1080);
        let above = Point::from((0, -1080));
        assert_eq!(placement(Some(above), size(), &[panel]), above);
    }

    #[test]
    fn a_position_that_overlaps_is_refused() {
        // Not clamped or nudged: auto-placement is what the physical path
        // falls back to, and the two have to agree.
        let panel = rect(0, 0, 1920, 1080);
        let over = Point::from((100, 100));
        assert_eq!(
            placement(Some(over), size(), &[panel]),
            Point::from((1920, 0))
        );
    }

    #[test]
    fn touching_edges_do_not_count_as_overlapping() {
        let panel = rect(0, 0, 1920, 1080);
        let beside = Point::from((1920, 0));
        assert_eq!(placement(Some(beside), size(), &[panel]), beside);
    }

    #[test]
    fn the_right_edge_wins_over_the_sum_of_the_widths() {
        // A screen placed by config at 3000 leaves the arrangement 4920 wide
        // while the widths only sum to 3840 — which is inside it.
        let existing = [rect(0, 0, 1920, 1080), rect(3000, 0, 1920, 1080)];
        assert_eq!(placement(None, size(), &existing), Point::from((4920, 0)));
    }
}
