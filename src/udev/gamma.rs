// DRM gamma control implementation
//
// Provides low-level gamma table management for DRM outputs.

use smithay::{
    backend::drm::DrmDeviceFd,
    reexports::drm::control::{crtc, Device as ControlDevice},
};
use tracing::{debug, warn};

/// Generate a simple gamma LUT with color temperature adjustment
///
/// temperature: 1000-10000 (lower = warmer/more red, higher = cooler/more blue)
/// size: number of entries in the LUT (typically 256 or 1024)
pub fn generate_gamma_lut(temperature: u32, size: usize) -> (Vec<u16>, Vec<u16>, Vec<u16>) {
    let temp_f = temperature as f64;

    // Simple color temperature calculation
    // Based on Planckian locus approximation
    let (r_mult, g_mult, b_mult) = if temp_f < 6600.0 {
        // Warm (reduce blue, slightly reduce green)
        let factor = (temp_f - 1000.0) / 5600.0; // 0.0 at 1000K, 1.0 at 6600K
        (1.0, 0.7 + 0.3 * factor, 0.3 + 0.7 * factor)
    } else {
        // Cool (reduce red)
        let factor = (temp_f - 6600.0) / 3400.0; // 0.0 at 6600K, 1.0 at 10000K
        (1.0 - 0.3 * factor.min(1.0), 1.0, 1.0)
    };

    let mut red = Vec::with_capacity(size);
    let mut green = Vec::with_capacity(size);
    let mut blue = Vec::with_capacity(size);

    for i in 0..size {
        // Linear ramp from 0 to 65535
        let value = ((i as f64 / (size - 1) as f64) * 65535.0) as u16;

        red.push(((value as f64) * r_mult).min(65535.0) as u16);
        green.push(((value as f64) * g_mult).min(65535.0) as u16);
        blue.push(((value as f64) * b_mult).min(65535.0) as u16);
    }

    (red, green, blue)
}

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
