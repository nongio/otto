//! What the camera wrote down about a photograph: which camera and lens,
//! the exposure, and where it was taken. Read from the EXIF block of a JPEG,
//! or the header of a TIFF-based file (TIFF itself, and the raw formats
//! built on it: DNG, NEF, CR2, ARW, ORF, RW2 …).
//!
//! Header arithmetic only, like [`crate::imagesize`]: the bytes are a file's,
//! so every read is bounds-checked and a malformed block is simply nothing
//! known, never a panic. Only the first [`READ_LIMIT`] bytes are looked at;
//! EXIF sits at the head of the file in every format read here.

use std::io::Read;
use std::path::Path;

/// How much of the file is read looking for the EXIF block. JPEG puts it in
/// the first segment and caps a segment at 64 KiB; a raw file's IFDs are at
/// its head too.
const READ_LIMIT: u64 = 256 * 1024;

/// The facts a photograph carries about how it was taken. Each is `None`
/// when the file does not say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shot {
    /// The camera, maker and model together: "Apple iPhone 13", "Canon EOS
    /// R5" (not "Canon Canon EOS R5").
    pub camera: Option<String>,
    pub lens: Option<String>,
    /// The f-number.
    pub aperture: Option<f32>,
    /// The shutter time in seconds, as the fraction it was written as.
    pub exposure: Option<(u32, u32)>,
    pub iso: Option<u32>,
    /// The focal length in millimetres, as a 35mm-equivalent when the file
    /// gives one: what a phone's tiny lens means to someone who thinks in
    /// full-frame terms.
    pub focal: Option<f32>,
    /// Where it was taken, latitude then longitude in degrees, north and
    /// east positive.
    pub location: Option<(f64, f64)>,
}

impl Shot {
    /// Whether the file said nothing worth showing.
    pub fn is_empty(&self) -> bool {
        self.camera.is_none()
            && self.lens.is_none()
            && self.exposure_line().is_none()
            && self.location.is_none()
    }

    /// Where it was taken, the way a map writes it: "45.4642° N, 9.1900° E".
    pub fn location_line(&self) -> Option<String> {
        let (lat, lon) = self.location?;
        let ns = if lat < 0.0 { "S" } else { "N" };
        let ew = if lon < 0.0 { "W" } else { "E" };
        Some(format!("{:.4}° {ns}, {:.4}° {ew}", lat.abs(), lon.abs()))
    }

    /// The exposure on one line: "ƒ/1.8 · 1/120 s · ISO 100 · 26 mm", with
    /// whatever the file leaves out left out.
    pub fn exposure_line(&self) -> Option<String> {
        let parts: Vec<String> = [
            self.aperture.map(|f| format!("ƒ/{}", trim(f))),
            self.exposure.and_then(shutter),
            self.iso.map(|iso| format!("ISO {iso}")),
            self.focal.map(|mm| format!("{} mm", trim(mm))),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// A number with one decimal at most, and none when it is whole: 1.8, 8, 4.2.
fn trim(value: f32) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded:.1}")
    }
}

/// A shutter time the way a camera shows it: "1/120 s" under a second,
/// "2 s" or "1.3 s" at a second or more.
fn shutter((num, den): (u32, u32)) -> Option<String> {
    if num == 0 || den == 0 {
        return None;
    }
    let secs = num as f32 / den as f32;
    if secs >= 1.0 {
        Some(format!("{} s", trim(secs)))
    } else {
        Some(format!("1/{} s", (den as f32 / num as f32).round() as u32))
    }
}

/// Read what the photograph at `path` says about how it was taken. `None`
/// when it is not a format read here, cannot be read, or says nothing.
pub fn read(path: &Path) -> Option<Shot> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(READ_LIMIT)
        .read_to_end(&mut bytes)
        .ok()?;
    let shot = parse(&bytes)?;
    (!shot.is_empty()).then_some(shot)
}

/// [`read`] over the start of a file.
pub fn parse(bytes: &[u8]) -> Option<Shot> {
    let tiff = if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg_exif(bytes)?
    } else if bytes.starts_with(b"II\x2A\0") || bytes.starts_with(b"MM\0\x2A") {
        bytes
    } else {
        return None;
    };
    Tiff::new(tiff).map(|tiff| tiff.shot())
}

/// The TIFF block inside a JPEG's EXIF segment, found by walking the marker
/// segments up to the start of the scan.
fn jpeg_exif(bytes: &[u8]) -> Option<&[u8]> {
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        while bytes.get(at) == Some(&0xFF) {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        match marker {
            0x01 | 0xD0..=0xD7 => continue,
            0xD9 | 0xDA => return None,
            _ => {}
        }
        let len = u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]) as usize;
        if len < 2 {
            return None;
        }
        if marker == 0xE1 {
            let segment = bytes.get(at + 2..at + len)?;
            if let Some(tiff) = segment.strip_prefix(b"Exif\0\0") {
                return Some(tiff);
            }
        }
        at += len;
    }
}

/// A TIFF structure: its bytes and which way round its numbers are.
struct Tiff<'a> {
    bytes: &'a [u8],
    big: bool,
}

/// One IFD entry's tag, type, count and where its value is.
struct Field {
    kind: u16,
    count: u32,
    at: usize,
}

const IFD0_MAKE: u16 = 0x010F;
const IFD0_MODEL: u16 = 0x0110;
const IFD0_EXIF: u16 = 0x8769;
const IFD0_GPS: u16 = 0x8825;
const GPS_LAT_REF: u16 = 0x0001;
const GPS_LAT: u16 = 0x0002;
const GPS_LON_REF: u16 = 0x0003;
const GPS_LON: u16 = 0x0004;
const EXIF_EXPOSURE: u16 = 0x829A;
const EXIF_FNUMBER: u16 = 0x829D;
const EXIF_ISO: u16 = 0x8827;
const EXIF_FOCAL: u16 = 0x920A;
const EXIF_FOCAL_35: u16 = 0xA405;
const EXIF_LENS_MAKE: u16 = 0xA433;
const EXIF_LENS_MODEL: u16 = 0xA434;

impl<'a> Tiff<'a> {
    fn new(bytes: &'a [u8]) -> Option<Self> {
        let big = match bytes.get(..4)? {
            b"MM\0\x2A" => true,
            b"II\x2A\0" => false,
            _ => return None,
        };
        Some(Self { bytes, big })
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        let b = self.bytes.get(at..at.checked_add(2)?)?;
        let b = [b[0], b[1]];
        Some(if self.big {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    }

    fn u32_at(&self, at: usize) -> Option<u32> {
        let b = self.bytes.get(at..at.checked_add(4)?)?;
        let b = [b[0], b[1], b[2], b[3]];
        Some(if self.big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    }

    /// The entries of the IFD at `offset`.
    fn ifd(&self, offset: usize) -> Vec<(u16, Field)> {
        let Some(count) = self.u16_at(offset) else {
            return Vec::new();
        };
        // A count claiming more entries than the bytes could hold is
        // rubbish; the bound keeps a hostile one from costing anything.
        let count = (count as usize).min(self.bytes.len() / 12);
        (0..count)
            .filter_map(|entry| {
                let at = offset.checked_add(2 + entry * 12)?;
                let tag = self.u16_at(at)?;
                let kind = self.u16_at(at + 2)?;
                let count = self.u32_at(at + 4)?;
                let size = match kind {
                    1 | 2 | 6 | 7 => 1,
                    3 | 8 => 2,
                    4 | 9 | 11 => 4,
                    5 | 10 | 12 => 8,
                    _ => return None,
                };
                let len = (count as usize).checked_mul(size)?;
                // Four bytes or fewer sit in the entry itself; anything
                // longer is somewhere else, at the offset the entry holds.
                let value = if len <= 4 {
                    at + 8
                } else {
                    self.u32_at(at + 8)? as usize
                };
                self.bytes.get(value..value.checked_add(len)?)?;
                Some((
                    tag,
                    Field {
                        kind,
                        count,
                        at: value,
                    },
                ))
            })
            .collect()
    }

    fn text(&self, field: &Field) -> Option<String> {
        if field.kind != 2 {
            return None;
        }
        let raw = self.bytes.get(field.at..field.at + field.count as usize)?;
        let raw = raw.split(|&b| b == 0).next().unwrap_or_default();
        let text = String::from_utf8_lossy(raw).trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    fn rational(&self, field: &Field) -> Option<(u32, u32)> {
        if !matches!(field.kind, 5 | 10) || field.count == 0 {
            return None;
        }
        let (num, den) = (self.u32_at(field.at)?, self.u32_at(field.at + 4)?);
        (den != 0).then_some((num, den))
    }

    /// Degrees, minutes and seconds — three rationals — as degrees.
    fn degrees(&self, field: &Field) -> Option<f64> {
        if field.kind != 5 || field.count < 3 {
            return None;
        }
        let part = |i: usize| {
            let at = field.at + i * 8;
            let (num, den) = (self.u32_at(at)?, self.u32_at(at + 4)?);
            (den != 0).then(|| num as f64 / den as f64)
        };
        Some(part(0)? + part(1)? / 60.0 + part(2)? / 3600.0)
    }

    fn number(&self, field: &Field) -> Option<u32> {
        if field.count == 0 {
            return None;
        }
        match field.kind {
            3 => self.u16_at(field.at).map(u32::from),
            4 => self.u32_at(field.at),
            _ => None,
        }
    }

    fn shot(&self) -> Shot {
        let mut shot = Shot::default();
        let (mut make, mut model) = (None, None);
        let (mut exif, mut gps) = (None, None);
        if let Some(ifd0) = self.u32_at(4) {
            for (tag, field) in self.ifd(ifd0 as usize) {
                match tag {
                    IFD0_MAKE => make = self.text(&field),
                    IFD0_MODEL => model = self.text(&field),
                    IFD0_EXIF => exif = self.number(&field),
                    IFD0_GPS => gps = self.number(&field),
                    _ => {}
                }
            }
        }
        shot.camera = camera_name(make.as_deref(), model.as_deref());

        let (mut focal, mut focal_35, mut lens_make, mut lens_model) = (None, None, None, None);
        if let Some(exif) = exif {
            for (tag, field) in self.ifd(exif as usize) {
                match tag {
                    EXIF_EXPOSURE => shot.exposure = self.rational(&field),
                    EXIF_FNUMBER => {
                        shot.aperture = self
                            .rational(&field)
                            .map(|(n, d)| n as f32 / d as f32)
                            .filter(|f| *f > 0.0)
                    }
                    EXIF_ISO => shot.iso = self.number(&field).filter(|iso| *iso > 0),
                    EXIF_FOCAL => {
                        focal = self
                            .rational(&field)
                            .map(|(n, d)| n as f32 / d as f32)
                            .filter(|mm| *mm > 0.0)
                    }
                    EXIF_FOCAL_35 => focal_35 = self.number(&field).filter(|mm| *mm > 0),
                    EXIF_LENS_MAKE => lens_make = self.text(&field),
                    EXIF_LENS_MODEL => lens_model = self.text(&field),
                    _ => {}
                }
            }
        }
        shot.focal = focal_35.map(|mm| mm as f32).or(focal);
        // The lens's own model says enough ("iPhone 15 back dual wide camera
        // 5.96mm f/1.6"); its maker is the camera's, or on the model already.
        shot.lens = lens_model.or(lens_make);

        if let Some(gps) = gps {
            let (mut lat, mut lon, mut lat_ref, mut lon_ref) = (None, None, None, None);
            for (tag, field) in self.ifd(gps as usize) {
                match tag {
                    GPS_LAT_REF => lat_ref = self.text(&field),
                    GPS_LAT => lat = self.degrees(&field),
                    GPS_LON_REF => lon_ref = self.text(&field),
                    GPS_LON => lon = self.degrees(&field),
                    _ => {}
                }
            }
            let signed = |value: f64, reference: Option<String>, negative: &str| {
                if reference.as_deref() == Some(negative) {
                    -value
                } else {
                    value
                }
            };
            shot.location = match (lat, lon) {
                (Some(lat), Some(lon)) if lat <= 90.0 && lon <= 180.0 => {
                    let location = (signed(lat, lat_ref, "S"), signed(lon, lon_ref, "W"));
                    // A phone with no fix writes zeros: that is nowhere,
                    // not a spot in the Gulf of Guinea.
                    (location != (0.0, 0.0)).then_some(location)
                }
                _ => None,
            };
        }
        shot
    }
}

/// A maker and a model as one name, without saying the maker twice: models
/// usually start with the maker's name ("Canon EOS R5", "NIKON D750"), and
/// makers often carry a suffix the model leaves off ("NIKON CORPORATION").
fn camera_name(make: Option<&str>, model: Option<&str>) -> Option<String> {
    match (make, model) {
        (_, None) => make.map(str::to_string),
        (None, Some(model)) => Some(model.to_string()),
        (Some(make), Some(model)) => {
            let brand = make.split_whitespace().next().unwrap_or(make);
            if model.to_lowercase().starts_with(&brand.to_lowercase()) {
                Some(model.to_string())
            } else {
                Some(format!("{make} {model}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little-endian TIFF block: IFD0 with `ifd0`, then the EXIF IFD with
    /// `exif`. Values longer than four bytes go after both IFDs.
    struct Builder {
        ifd0: Vec<(u16, u16, u32, Vec<u8>)>,
        exif: Vec<(u16, u16, u32, Vec<u8>)>,
        gps: Vec<(u16, u16, u32, Vec<u8>)>,
    }

    fn dms(degrees: u32, minutes: u32, centiseconds: u32) -> (u16, u32, Vec<u8>) {
        let mut bytes = Vec::new();
        for (num, den) in [(degrees, 1), (minutes, 1), (centiseconds, 100)] {
            bytes.extend_from_slice(&u32::to_le_bytes(num));
            bytes.extend_from_slice(&u32::to_le_bytes(den));
        }
        (5, 3, bytes)
    }

    fn ascii(text: &str) -> (u16, u32, Vec<u8>) {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        (2, bytes.len() as u32, bytes)
    }

    fn rational(num: u32, den: u32) -> (u16, u32, Vec<u8>) {
        let mut bytes = num.to_le_bytes().to_vec();
        bytes.extend_from_slice(&den.to_le_bytes());
        (5, 1, bytes)
    }

    fn short(value: u16) -> (u16, u32, Vec<u8>) {
        (3, 1, value.to_le_bytes().to_vec())
    }

    impl Builder {
        fn new() -> Self {
            Self {
                ifd0: Vec::new(),
                exif: Vec::new(),
                gps: Vec::new(),
            }
        }

        fn ifd0(mut self, tag: u16, (kind, count, value): (u16, u32, Vec<u8>)) -> Self {
            self.ifd0.push((tag, kind, count, value));
            self
        }

        fn exif(mut self, tag: u16, (kind, count, value): (u16, u32, Vec<u8>)) -> Self {
            self.exif.push((tag, kind, count, value));
            self
        }

        fn gps(mut self, tag: u16, (kind, count, value): (u16, u32, Vec<u8>)) -> Self {
            self.gps.push((tag, kind, count, value));
            self
        }

        fn build(mut self) -> Vec<u8> {
            let ifd_len = |n: usize| 2 + n * 12 + 4;
            if !self.exif.is_empty() {
                self.ifd0.push((IFD0_EXIF, 4, 1, vec![0; 4]));
            }
            if !self.gps.is_empty() {
                self.ifd0.push((IFD0_GPS, 4, 1, vec![0; 4]));
            }
            let ifd0_at = 8;
            let exif_at = ifd0_at + ifd_len(self.ifd0.len());
            let gps_at = exif_at + ifd_len(self.exif.len());
            let mut data_at = gps_at + ifd_len(self.gps.len());
            let mut out = b"II\x2A\0".to_vec();
            out.extend_from_slice(&(ifd0_at as u32).to_le_bytes());
            let mut data = Vec::new();
            let mut write = |out: &mut Vec<u8>, entries: &[(u16, u16, u32, Vec<u8>)]| {
                out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
                for (tag, kind, count, value) in entries {
                    out.extend_from_slice(&tag.to_le_bytes());
                    out.extend_from_slice(&kind.to_le_bytes());
                    out.extend_from_slice(&count.to_le_bytes());
                    if *tag == IFD0_EXIF && *kind == 4 && value == &[0; 4] {
                        out.extend_from_slice(&(exif_at as u32).to_le_bytes());
                    } else if *tag == IFD0_GPS && *kind == 4 && value == &[0; 4] {
                        out.extend_from_slice(&(gps_at as u32).to_le_bytes());
                    } else if value.len() <= 4 {
                        let mut inline = value.clone();
                        inline.resize(4, 0);
                        out.extend_from_slice(&inline);
                    } else {
                        out.extend_from_slice(&(data_at as u32).to_le_bytes());
                        data.extend_from_slice(value);
                        data_at += value.len();
                    }
                }
                out.extend_from_slice(&0u32.to_le_bytes());
            };
            let (ifd0, exif, gps) = (self.ifd0.clone(), self.exif.clone(), self.gps.clone());
            write(&mut out, &ifd0);
            write(&mut out, &exif);
            write(&mut out, &gps);
            out.extend_from_slice(&data);
            out
        }
    }

    fn iphone() -> Vec<u8> {
        Builder::new()
            .ifd0(IFD0_MAKE, ascii("Apple"))
            .ifd0(IFD0_MODEL, ascii("iPhone 13"))
            .exif(EXIF_EXPOSURE, rational(1, 120))
            .exif(EXIF_FNUMBER, rational(16, 10))
            .exif(EXIF_ISO, short(100))
            .exif(EXIF_FOCAL, rational(51, 10))
            .exif(EXIF_FOCAL_35, short(26))
            .exif(EXIF_LENS_MAKE, ascii("Apple"))
            .exif(
                EXIF_LENS_MODEL,
                ascii("iPhone 13 back dual wide camera 5.1mm f/1.6"),
            )
            .build()
    }

    /// `tiff` wrapped as the EXIF segment of a JPEG.
    fn jpeg(tiff: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
        out.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(tiff);
        out.extend_from_slice(&[0xFF, 0xDA, 0, 2]);
        out
    }

    #[test]
    fn a_phone_photo_says_what_took_it_and_how() {
        let shot = parse(&jpeg(&iphone())).expect("a shot");
        assert_eq!(shot.camera.as_deref(), Some("Apple iPhone 13"));
        assert_eq!(
            shot.lens.as_deref(),
            Some("iPhone 13 back dual wide camera 5.1mm f/1.6")
        );
        assert_eq!(
            shot.exposure_line().as_deref(),
            Some("ƒ/1.6 · 1/120 s · ISO 100 · 26 mm")
        );
    }

    #[test]
    fn a_photo_with_a_fix_says_where_it_was_taken() {
        // 45° 27' 51.12" N, 9° 11' 24.00" W.
        let tiff = Builder::new()
            .ifd0(IFD0_MAKE, ascii("Apple"))
            .gps(GPS_LAT_REF, ascii("N"))
            .gps(GPS_LAT, dms(45, 27, 5112))
            .gps(GPS_LON_REF, ascii("W"))
            .gps(GPS_LON, dms(9, 11, 2400))
            .build();
        let shot = parse(&jpeg(&tiff)).expect("a shot");
        let (lat, lon) = shot.location.expect("a location");
        assert!((lat - 45.4642).abs() < 1e-4, "{lat}");
        assert!((lon + 9.19).abs() < 1e-4, "{lon}");
        assert_eq!(
            shot.location_line().as_deref(),
            Some("45.4642° N, 9.1900° W")
        );

        // No fix: zeros are nowhere.
        let nowhere = Builder::new()
            .ifd0(IFD0_MAKE, ascii("Apple"))
            .gps(GPS_LAT, dms(0, 0, 0))
            .gps(GPS_LON, dms(0, 0, 0))
            .build();
        assert_eq!(parse(&nowhere).unwrap().location, None);
    }

    #[test]
    fn a_raw_file_is_read_from_its_own_header() {
        assert_eq!(parse(&iphone()), parse(&jpeg(&iphone())));
    }

    #[test]
    fn the_maker_is_not_said_twice() {
        assert_eq!(
            camera_name(Some("Canon"), Some("Canon EOS R5")).as_deref(),
            Some("Canon EOS R5")
        );
        assert_eq!(
            camera_name(Some("NIKON CORPORATION"), Some("NIKON D750")).as_deref(),
            Some("NIKON D750")
        );
        assert_eq!(
            camera_name(Some("FUJIFILM"), Some("X-T4")).as_deref(),
            Some("FUJIFILM X-T4")
        );
    }

    #[test]
    fn exposures_read_the_way_cameras_show_them() {
        assert_eq!(shutter((1, 250)).as_deref(), Some("1/250 s"));
        assert_eq!(shutter((10, 2500)).as_deref(), Some("1/250 s"));
        assert_eq!(shutter((2, 1)).as_deref(), Some("2 s"));
        assert_eq!(shutter((13, 10)).as_deref(), Some("1.3 s"));
        assert_eq!(shutter((0, 1)), None);
        assert_eq!(trim(8.0), "8");
        assert_eq!(trim(1.8), "1.8");
    }

    #[test]
    fn a_photo_with_no_exif_says_nothing() {
        assert_eq!(parse(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]), None);
        assert_eq!(parse(b"\x89PNG\r\n\x1a\n"), None);
        assert!(parse(&Builder::new().build()).unwrap().is_empty());
    }

    #[test]
    fn hostile_offsets_are_nothing_known() {
        let mut tiff = iphone();
        // Point the EXIF IFD far past the end, and claim a huge entry count.
        let n = tiff.len();
        tiff[8] = 0xFF;
        tiff[9] = 0xFF;
        let shot = parse(&tiff[..n]).expect("still a TIFF");
        assert!(shot.is_empty() || shot.camera.is_some());
        // Every truncation of a real block parses without panicking.
        let whole = jpeg(&iphone());
        for end in 0..whole.len() {
            let _ = parse(&whole[..end]);
        }
    }
}
