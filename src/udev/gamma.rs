// DRM gamma control implementation
//
// Provides low-level gamma table management for DRM outputs.

use smithay::{
    backend::drm::DrmDeviceFd,
    reexports::drm::control::{crtc, Device as ControlDevice},
};
use tracing::{debug, warn};

use crate::state::gamma_control::generate_gamma_lut;

/// Apply gamma LUT from raw u16 arrays (for protocol use)
pub fn apply_gamma_lut(
    drm_fd: &DrmDeviceFd,
    crtc: crtc::Handle,
    red: &[u16],
    green: &[u16],
    blue: &[u16],
) -> Result<(), String> {
    if red.len() != green.len() || green.len() != blue.len() {
        return Err("Gamma LUT arrays must have equal length".to_string());
    }

    let gamma_size = match drm_fd.get_crtc(crtc) {
        Ok(crtc_info) => crtc_info.gamma_length() as usize,
        Err(e) => {
            return Err(format!("Failed to get CRTC info: {}", e));
        }
    };

    if gamma_size == 0 {
        return Err("CRTC does not support gamma tables".to_string());
    }

    if red.len() != gamma_size {
        return Err(format!(
            "Gamma LUT size mismatch: expected {}, got {}",
            gamma_size,
            red.len()
        ));
    }

    debug!("Applying gamma LUT to CRTC {:?}", crtc);

    if let Err(e) = drm_fd.set_gamma(crtc, red, green, blue) {
        return Err(format!("Failed to set gamma: {}", e));
    }

    debug!("✓ Gamma LUT applied");
    Ok(())
}

/// Get gamma size for a CRTC
pub fn get_gamma_size(drm_fd: &DrmDeviceFd, crtc: crtc::Handle) -> Result<u32, String> {
    match drm_fd.get_crtc(crtc) {
        Ok(crtc_info) => Ok(crtc_info.gamma_length()),
        Err(e) => Err(format!("Failed to get CRTC info: {}", e)),
    }
}

/// Reset gamma to linear (neutral)
pub fn reset_gamma(drm_fd: &DrmDeviceFd, crtc: crtc::Handle) -> Result<(), String> {
    let gamma_size = match drm_fd.get_crtc(crtc) {
        Ok(crtc_info) => crtc_info.gamma_length() as usize,
        Err(e) => {
            return Err(format!("Failed to get CRTC info: {}", e));
        }
    };

    if gamma_size == 0 {
        return Ok(()); // Nothing to reset
    }

    // Generate neutral/linear gamma (6500K is neutral)
    let (red, green, blue) = generate_gamma_lut(6500, gamma_size);

    if let Err(e) = drm_fd.set_gamma(crtc, &red, &green, &blue) {
        warn!("Failed to reset gamma: {}", e);
        return Err(format!("Failed to reset gamma: {}", e));
    }

    debug!("✓ Gamma reset to neutral");
    Ok(())
}
