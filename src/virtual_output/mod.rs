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
    utils::Point,
};

use crate::{
    config::VirtualOutputConfig,
    screenshare::{BackendCapabilities, PipeWireStream, StreamConfig},
};

/// Runtime state for one virtual output.
pub struct VirtualOutputState {
    /// The Smithay output (Wayland global, workspace mapping).
    pub output: Output,
    /// Global handle — must be kept alive for the Wayland global to exist.
    pub _global: GlobalId,
    /// PipeWire stream receiving rendered frames.
    pub pipewire_stream: PipeWireStream,
    /// Damage tracker for this output (always renders full frames, age=0).
    pub damage_tracker: OutputDamageTracker,
    /// Frame rendered last cycle, awaiting its GPU fence before the dmabuf is
    /// queued to PipeWire. Resolved (non-blocking `is_reached()`) at the start
    /// of the next `render_virtual_outputs` tick so the fence wait never blocks
    /// the main loop / input dispatch.
    pub pending_frame: Option<smithay::backend::renderer::sync::SyncPoint>,
}

impl VirtualOutputState {
    /// Build an `Output` from config (without registering a Wayland global yet).
    ///
    /// The caller is responsible for calling `output.create_global::<D>()` and
    /// storing the returned `GlobalId` in `_global`, then calling `finish()`.
    pub fn build_output(config: &VirtualOutputConfig) -> Output {
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
        let position: Point<i32, smithay::utils::Logical> = config
            .position
            .map(|p| (p.x, p.y).into())
            .unwrap_or_else(|| (0, 0).into());

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
        };

        let mut pipewire_stream = PipeWireStream::new(stream_config);
        let node_id = pipewire_stream
            .start_sync()
            .map_err(|e| format!("Failed to start PipeWire stream for '{}': {e}", config.name))?;

        let state = Self {
            output,
            _global: global,
            pipewire_stream,
            damage_tracker,
            pending_frame: None,
        };

        Ok((state, node_id))
    }
}
