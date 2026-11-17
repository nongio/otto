//! A display as its EDID describes it, for the frame-pacing simulator.
//!
//! Only what pacing needs is read: the preferred mode's size and its exact
//! refresh rate, which comes out of the pixel clock and the blanking, so a
//! "60 Hz" panel is 59.95 Hz or 60.02 Hz as the hardware would scan it out.
//! [`synthesize`] builds a valid base block (CVT reduced-blanking timings) for
//! a given size and rate; [`parse`] reads one back, or a real panel's from
//! `/sys/class/drm/*/edid`.

/// The preferred mode of a display.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayMode {
    /// The monitor name descriptor, or "Unknown".
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Refresh rate in millihertz, as DRM reports it.
    pub refresh_mhz: i32,
    /// Pixel clock in kHz.
    pub pixel_clock_khz: u32,
}

impl DisplayMode {
    /// The refresh period in microseconds.
    pub fn period_us(&self) -> f64 {
        1_000_000_000.0 / self.refresh_mhz as f64
    }

    /// `1920x1080@59.95`.
    pub fn label(&self) -> String {
        format!(
            "{}x{}@{:.2}",
            self.width,
            self.height,
            self.refresh_mhz as f64 / 1000.0
        )
    }
}

/// Why a blob is not an EDID this module can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdidError {
    TooShort(usize),
    BadHeader,
    BadChecksum,
    NoDetailedTiming,
}

impl std::fmt::Display for EdidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EdidError::TooShort(len) => write!(f, "EDID is {len} bytes, a base block is 128"),
            EdidError::BadHeader => write!(f, "EDID header is not 00 FF FF FF FF FF FF 00"),
            EdidError::BadChecksum => write!(f, "EDID base block checksum does not add up"),
            EdidError::NoDetailedTiming => write!(f, "EDID has no detailed timing descriptor"),
        }
    }
}

impl std::error::Error for EdidError {}

const SRGB_CHROMATICITY: [u8; 10] = [0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54];

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

/// Reads the preferred mode (the first detailed timing descriptor) and the
/// monitor name of an EDID base block. Extension blocks are ignored.
pub fn parse(edid: &[u8]) -> Result<DisplayMode, EdidError> {
    if edid.len() < 128 {
        return Err(EdidError::TooShort(edid.len()));
    }
    let base = &edid[..128];
    if base[..8] != HEADER {
        return Err(EdidError::BadHeader);
    }
    if base.iter().fold(0u8, |acc, b| acc.wrapping_add(*b)) != 0 {
        return Err(EdidError::BadChecksum);
    }
    let mut mode = None;
    let mut name = None;
    for desc in base[54..126].chunks_exact(18) {
        let clock = u16::from_le_bytes([desc[0], desc[1]]);
        if clock != 0 {
            if mode.is_none() {
                mode = Some(detailed_timing(desc));
            }
        } else if desc[3] == 0xFC {
            let text: String = desc[5..18]
                .iter()
                .take_while(|&&b| b != 0x0A)
                .map(|&b| b as char)
                .collect();
            name = Some(text.trim_end().to_string());
        }
    }
    let (width, height, pixel_clock_khz, total) = mode.ok_or(EdidError::NoDetailedTiming)?;
    let refresh_mhz = (pixel_clock_khz as u64 * 1_000_000 / total.max(1)) as i32;
    Ok(DisplayMode {
        name: name.unwrap_or_else(|| "Unknown".to_string()),
        width,
        height,
        refresh_mhz,
        pixel_clock_khz,
    })
}

/// `(width, height, pixel clock in kHz, pixels per frame)` of a detailed
/// timing descriptor.
fn detailed_timing(d: &[u8]) -> (u32, u32, u32, u64) {
    let clock_khz = u16::from_le_bytes([d[0], d[1]]) as u32 * 10;
    let h_active = d[2] as u32 | ((d[4] as u32 >> 4) << 8);
    let h_blank = d[3] as u32 | ((d[4] as u32 & 0xF) << 8);
    let v_active = d[5] as u32 | ((d[7] as u32 >> 4) << 8);
    let v_blank = d[6] as u32 | ((d[7] as u32 & 0xF) << 8);
    let total = (h_active + h_blank) as u64 * (v_active + v_blank) as u64;
    (h_active, v_active, clock_khz, total)
}

/// A valid 128-byte EDID base block for a `width`×`height` panel at about
/// `refresh_hz`, named `name` (at most 13 characters are kept).
///
/// # Panics
///
/// Panics when the mode's pixel clock does not fit the base block's
/// detailed timing (655.35 MHz: 4K above 60 Hz, 1440p above 160 Hz).
///
/// The timings are CVT reduced blanking: 160 pixels of horizontal blanking
/// and at least 460 µs of vertical blanking, with the pixel clock rounded
/// down to the 10 kHz the descriptor stores. The exact rate is what that
/// clock gives, so it lands just under the nominal one, as on real panels.
pub fn synthesize(name: &str, width: u32, height: u32, refresh_hz: f64) -> [u8; 128] {
    const H_BLANK: u32 = 160;
    const H_FRONT: u32 = 48;
    const H_SYNC: u32 = 32;
    const V_FRONT: u32 = 3;
    const V_SYNC: u32 = 6;
    const MIN_V_BACK: u32 = 6;
    const MIN_VBI_US: f64 = 460.0;

    let h_period_us = (1_000_000.0 / refresh_hz - MIN_VBI_US) / height as f64;
    let vbi_lines = (MIN_VBI_US / h_period_us).floor() as u32 + 1;
    let v_blank = vbi_lines.max(V_FRONT + V_SYNC + MIN_V_BACK);
    let h_total = width + H_BLANK;
    let v_total = height + v_blank;
    let clock_10khz = (refresh_hz * h_total as f64 * v_total as f64 / 10_000.0).floor();
    assert!(
        clock_10khz <= u16::MAX as f64,
        "{width}x{height}@{refresh_hz} needs a {:.0} MHz pixel clock; a base block's \
         detailed timing stops at 655.35 MHz (larger modes live in a DisplayID extension)",
        clock_10khz / 100.0
    );
    let clock_10khz = clock_10khz as u16;

    let mut e = [0u8; 128];
    e[..8].copy_from_slice(&HEADER);
    // Manufacturer "OTO", product 0x0001, serial 0.
    let pnp = |c: u8| (c - b'A' + 1) as u16;
    let id = (pnp(b'O') << 10) | (pnp(b'T') << 5) | pnp(b'O');
    e[8..10].copy_from_slice(&id.to_be_bytes());
    e[10] = 0x01;
    e[16] = 1; // week
    e[17] = 36; // 2026
    e[18] = 1; // EDID 1.4
    e[19] = 4;
    e[20] = 0xA5; // digital, 8 bpc, DisplayPort
    e[21] = (width as f64 * 0.0265 / 10.0).round() as u8; // size in cm, ~96 dpi
    e[22] = (height as f64 * 0.0265 / 10.0).round() as u8;
    e[23] = 120; // gamma 2.2
                 // sRGB default, preferred timing is native.
    e[24] = 0x06;
    // sRGB chromaticity.
    e[25..35].copy_from_slice(&SRGB_CHROMATICITY);
    // No established or standard timings.
    for slot in e[38..54].chunks_exact_mut(2) {
        slot.copy_from_slice(&[0x01, 0x01]);
    }

    let d = &mut e[54..72];
    d[0..2].copy_from_slice(&clock_10khz.to_le_bytes());
    d[2] = width as u8;
    d[3] = H_BLANK as u8;
    d[4] = (((width >> 8) & 0xF) << 4) as u8 | ((H_BLANK >> 8) & 0xF) as u8;
    d[5] = height as u8;
    d[6] = v_blank as u8;
    d[7] = (((height >> 8) & 0xF) << 4) as u8 | ((v_blank >> 8) & 0xF) as u8;
    d[8] = H_FRONT as u8;
    d[9] = H_SYNC as u8;
    d[10] = ((V_FRONT & 0xF) << 4) as u8 | (V_SYNC & 0xF) as u8;
    d[11] = 0;
    let w_mm = (width as f64 * 0.265).round() as u32;
    let h_mm = (height as f64 * 0.265).round() as u32;
    d[12] = w_mm as u8;
    d[13] = h_mm as u8;
    d[14] = (((w_mm >> 8) & 0xF) << 4) as u8 | ((h_mm >> 8) & 0xF) as u8;
    d[17] = 0x1A; // digital separate sync, +hsync, -vsync

    // Monitor name.
    let n = &mut e[72..90];
    n[3] = 0xFC;
    let bytes: Vec<u8> = name
        .bytes()
        .filter(|b| b.is_ascii_graphic() || *b == b' ')
        .take(13)
        .collect();
    n[5..18].fill(0x20);
    n[5..5 + bytes.len()].copy_from_slice(&bytes);
    if bytes.len() < 13 {
        n[5 + bytes.len()] = 0x0A;
    }
    // Two dummy descriptors.
    e[93] = 0x10;
    e[111] = 0x10;

    e[127] = 0u8.wrapping_sub(e[..127].iter().fold(0u8, |acc, b| acc.wrapping_add(*b)));
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_synthesized_edid_reads_back() {
        let edid = synthesize("Otto 4K", 3840, 2160, 60.0);
        let mode = parse(&edid).unwrap();
        assert_eq!(mode.name, "Otto 4K");
        assert_eq!((mode.width, mode.height), (3840, 2160));
        // Just under 60 Hz: the clock is rounded down to 10 kHz.
        assert!(
            (59_900..=60_000).contains(&mode.refresh_mhz),
            "{}",
            mode.refresh_mhz
        );
    }

    #[test]
    fn high_refresh_panels_keep_their_rate() {
        for hz in [75.0, 120.0, 144.0, 165.0, 240.0] {
            let mode = parse(&synthesize("p", 1920, 1080, hz)).unwrap();
            let got = mode.refresh_mhz as f64 / 1000.0;
            assert!((hz - got).abs() < 0.1, "{hz} Hz came back as {got}");
        }
    }

    #[test]
    fn a_damaged_edid_is_refused() {
        let mut edid = synthesize("x", 1920, 1080, 60.0);
        edid[100] ^= 1;
        assert_eq!(parse(&edid), Err(EdidError::BadChecksum));
        assert_eq!(parse(&edid[..64]), Err(EdidError::TooShort(64)));
        let mut edid = synthesize("x", 1920, 1080, 60.0);
        edid[0] = 1;
        assert_eq!(parse(&edid), Err(EdidError::BadHeader));
    }

    /// A real panel's block: Dell U2415, 1920x1200 at 59.95 Hz.
    #[test]
    fn a_real_panel_reads_as_scanned_out() {
        let mut edid = synthesize("DELL U2415", 1920, 1200, 60.0);
        // Its detailed timing: 154.00 MHz, 2080x1235.
        let d = &mut edid[54..72];
        d[0..2].copy_from_slice(&15_400u16.to_le_bytes());
        d[2] = 0x80;
        d[3] = 0xA0;
        d[4] = 0x70;
        d[5] = 0xB0;
        d[6] = 0x23;
        d[7] = 0x40;
        edid[127] = 0;
        edid[127] = 0u8.wrapping_sub(edid.iter().fold(0u8, |a, b| a.wrapping_add(*b)));
        let mode = parse(&edid).unwrap();
        assert_eq!((mode.width, mode.height), (1920, 1200));
        assert_eq!(mode.refresh_mhz, 59_950);
        assert_eq!(mode.name, "DELL U2415");
    }
}
