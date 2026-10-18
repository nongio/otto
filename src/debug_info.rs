//! What a bug report needs to know about the running compositor.
//!
//! Served as JSON by `org.otto.Compositor.DebugInfo` (see
//! `docs/developer/compositor-dbus-api.md`) and read by `otto-msg doctor`.

// Rust guideline compliant 2026-02-21

use serde_json::{json, Value};

use crate::state::{Backend, Otto};

impl<B: Backend + 'static> Otto<B> {
    /// The version, backend, renderer and outputs, as one JSON object.
    ///
    /// Outputs are sorted by name. Sizes are in physical pixels, refresh in
    /// millihertz and positions in logical points, the units Smithay keeps.
    pub fn debug_info_json(&self) -> Value {
        let renderer = crate::renderer_info::renderer_info().map(|info| {
            json!({
                "api": info.api,
                "vendor": info.vendor,
                "renderer": info.renderer,
                "version": info.version,
            })
        });
        #[cfg(feature = "vulkan")]
        let vulkan_fallback = crate::renderer::vulkan_api::vulkan_fallback();
        #[cfg(not(feature = "vulkan"))]
        let vulkan_fallback: Option<&str> = None;

        let mut outputs: Vec<_> = self.workspaces.outputs().cloned().collect();
        outputs.sort_by_key(|output| output.name());
        let outputs: Vec<Value> = outputs
            .iter()
            .map(|output| {
                let props = output.physical_properties();
                let mode = output.current_mode();
                let position = self
                    .workspaces
                    .output_geometry(output)
                    .map(|geometry| geometry.loc)
                    .unwrap_or_default();
                json!({
                    "name": output.name(),
                    "make": props.make,
                    "model": props.model,
                    "width": mode.map_or(0, |m| m.size.w),
                    "height": mode.map_or(0, |m| m.size.h),
                    "refresh_mhz": mode.map_or(0, |m| m.refresh),
                    "scale": output.current_scale().fractional_scale(),
                    "x": position.x,
                    "y": position.y,
                    "virtual": crate::virtual_output::is_virtual_output(output),
                })
            })
            .collect();

        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "git_commit": option_env!("OTTO_GIT_COMMIT"),
            "backend": self.backend_data.backend_name(),
            "renderer": renderer,
            "vulkan_fallback": vulkan_fallback,
            "config_files": crate::config::config_layer_paths(),
            "log_path": crate::session_log::path(),
            "outputs": outputs,
        })
    }
}
