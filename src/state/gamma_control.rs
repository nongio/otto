// wlr-gamma-control-unstable-v1 protocol implementation
//
// Allows privileged clients (like wlsunset, redshift) to set gamma tables
// for individual outputs.

use std::collections::HashMap;

use smithay::output::Output;
use smithay::reexports::wayland_server::{
    protocol::wl_output::WlOutput, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch,
};
use tracing::{debug, warn};

// Protocol bindings generation
pub mod gen {
    pub use smithay::reexports::wayland_server;
    pub use smithay::reexports::wayland_server::protocol::__interfaces::*;
    pub use smithay::reexports::wayland_server::protocol::*;
    pub use smithay::reexports::wayland_server::*;

    wayland_scanner::generate_interfaces!("./protocols/wlr-gamma-control-unstable-v1.xml");
    wayland_scanner::generate_server_code!("./protocols/wlr-gamma-control-unstable-v1.xml");
}

use gen::zwlr_gamma_control_manager_v1::{self, ZwlrGammaControlManagerV1};
use gen::zwlr_gamma_control_v1::{self, ZwlrGammaControlV1};

use crate::state::{Backend, Otto};

/// Global state for gamma control protocol
pub struct GammaControlManagerState {
    /// Active gamma controls per output
    controls: HashMap<WlOutput, ZwlrGammaControlV1>,
}

impl Default for GammaControlManagerState {
    fn default() -> Self {
        Self::new()
    }
}

impl GammaControlManagerState {
    pub fn new() -> Self {
        Self {
            controls: HashMap::new(),
        }
    }

    /// Register a new gamma control for an output
    fn register_control(&mut self, output: WlOutput, control: ZwlrGammaControlV1) -> bool {
        // Only one gamma control per output
        if self.controls.contains_key(&output) {
            return false;
        }
        self.controls.insert(output, control);
        true
    }

    /// Unregister gamma control for an output
    fn unregister_control(&mut self, control: &ZwlrGammaControlV1) {
        self.controls.retain(|_, v| v != control);
    }
}

/// Per-control state
pub struct GammaControlState {
    pub output: WlOutput,
    pub gamma_size: u32,
}

impl<BackendData: Backend> GlobalDispatch<ZwlrGammaControlManagerV1, (), Otto<BackendData>>
    for GammaControlManagerState
{
    fn bind(
        _state: &mut Otto<BackendData>,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<ZwlrGammaControlManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        data_init.init(resource, ());
    }
}

impl<BackendData: Backend> Dispatch<ZwlrGammaControlManagerV1, (), Otto<BackendData>>
    for GammaControlManagerState
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        _resource: &ZwlrGammaControlManagerV1,
        request: zwlr_gamma_control_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        match request {
            zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } => {
                // Find the smithay Output for this wl_output
                let smithay_output = Output::from_resource(&output);

                let gamma_size = if let Some(smithay_output) = smithay_output {
                    // Get gamma size from backend
                    state.get_gamma_size(&smithay_output).unwrap_or(0)
                } else {
                    0
                };

                if gamma_size == 0 {
                    // Output doesn't exist or doesn't support gamma
                    let control = data_init.init(
                        id,
                        GammaControlState {
                            output: output.clone(),
                            gamma_size: 0,
                        },
                    );
                    control.failed();
                    return;
                }

                // Check if output already has a gamma control
                if state.gamma_control_manager.controls.contains_key(&output) {
                    let control = data_init.init(
                        id,
                        GammaControlState {
                            output: output.clone(),
                            gamma_size,
                        },
                    );
                    control.failed();
                    return;
                }

                let control = data_init.init(
                    id,
                    GammaControlState {
                        output: output.clone(),
                        gamma_size,
                    },
                );

                // Register the control
                if !state
                    .gamma_control_manager
                    .register_control(output.clone(), control.clone())
                {
                    control.failed();
                    return;
                }

                // Send gamma size
                control.gamma_size(gamma_size);

                debug!("Gamma control created for output with size {}", gamma_size);
            }
            zwlr_gamma_control_manager_v1::Request::Destroy => {
                // Manager destroyed, controls remain active
            }
        }
    }
}

impl<BackendData: Backend> Dispatch<ZwlrGammaControlV1, GammaControlState, Otto<BackendData>>
    for GammaControlManagerState
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        resource: &ZwlrGammaControlV1,
        request: zwlr_gamma_control_v1::Request,
        data: &GammaControlState,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        match request {
            zwlr_gamma_control_v1::Request::SetGamma { fd } => {
                let fd = fd;
                let gamma_size = data.gamma_size as usize;

                // Read gamma table from file descriptor
                let table_size = gamma_size * 3 * std::mem::size_of::<u16>();
                let mut buffer = vec![0u8; table_size];

                use std::io::Read;

                let mut file = std::fs::File::from(fd);
                let result = file.read_exact(&mut buffer);

                if result.is_err() {
                    warn!("Failed to read gamma table from fd");
                    resource.failed();
                    return;
                }

                // Parse into u16 arrays without relying on potentially unaligned casts
                let u16_data: Vec<u16> = buffer
                    .chunks_exact(std::mem::size_of::<u16>())
                    .map(|b| u16::from_ne_bytes([b[0], b[1]]))
                    .collect();

                let red = &u16_data[0..gamma_size];
                let green = &u16_data[gamma_size..gamma_size * 2];
                let blue = &u16_data[gamma_size * 2..gamma_size * 3];

                // Find the smithay Output
                let smithay_output = Output::from_resource(&data.output);

                if let Some(output) = smithay_output {
                    // Apply gamma via backend
                    if let Err(e) = state.apply_gamma(&output, red, green, blue) {
                        warn!("Failed to apply gamma: {}", e);
                        resource.failed();
                        return;
                    }

                    debug!("Applied gamma table for output");
                } else {
                    warn!("Output no longer exists");
                    resource.failed();
                }
            }
            zwlr_gamma_control_v1::Request::Destroy => {
                // Find the smithay Output and reset gamma
                let smithay_output = Output::from_resource(&data.output);

                if let Some(output) = smithay_output {
                    // Reset gamma to neutral
                    let _ = state.reset_gamma(&output);
                    debug!("Reset gamma for output");
                }

                // Unregister
                state.gamma_control_manager.unregister_control(resource);
            }
        }
    }

    fn destroyed(
        state: &mut Otto<BackendData>,
        _client: wayland_server::backend::ClientId,
        resource: &ZwlrGammaControlV1,
        data: &GammaControlState,
    ) {
        // Client disconnected, reset gamma
        let smithay_output = Output::from_resource(&data.output);

        if let Some(output) = smithay_output {
            let _ = state.reset_gamma(&output);
            debug!("Reset gamma on client disconnect");
        }

        state.gamma_control_manager.unregister_control(resource);
    }
}
