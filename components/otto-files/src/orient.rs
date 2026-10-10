//! Turning and flipping a photograph without touching its pixels.
//!
//! A JPEG says which way up it goes in its EXIF Orientation tag, and every
//! viewer worth the name — Otto's own decoder included — turns it upright
//! when it shows it. So a turn or a flip is a new value for that one tag:
//! lossless, instant, and exactly undoable by writing the old value back.
//!
//! The tag is overwritten where it is, two bytes in place. A JPEG with no
//! EXIF at all (a screenshot, an export) is given a minimal block holding
//! just the tag, and written back through a temporary file. A JPEG whose EXIF
//! has no Orientation entry is left alone: adding one means moving every
//! offset in the block, which is a rewrite this does not attempt.

use std::fs;
use std::io::{self, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// One of the four buttons under the Photos info panel's picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    /// A quarter turn anticlockwise.
    Left,
    /// A quarter turn clockwise.
    Right,
    /// Mirrored left to right.
    FlipHorizontal,
    /// Mirrored top to bottom.
    FlipVertical,
}

/// The tag's value when the file has none: upright.
pub const UPRIGHT: u16 = 1;

/// How each Orientation value maps the stored picture onto the one shown, as
/// a 2×2 matrix on y-down coordinates, indexed by value − 1.
const MATRICES: [[i8; 4]; 8] = [
    [1, 0, 0, 1],   // 1: as stored
    [-1, 0, 0, 1],  // 2: mirrored left to right
    [-1, 0, 0, -1], // 3: half a turn
    [1, 0, 0, -1],  // 4: mirrored top to bottom
    [0, 1, 1, 0],   // 5: transposed
    [0, -1, 1, 0],  // 6: a quarter turn clockwise
    [0, -1, -1, 0], // 7: transversed
    [0, 1, -1, 0],  // 8: a quarter turn anticlockwise
];

impl Turn {
    fn matrix(self) -> [i8; 4] {
        match self {
            Turn::Right => MATRICES[5],
            Turn::Left => MATRICES[7],
            Turn::FlipHorizontal => MATRICES[1],
            Turn::FlipVertical => MATRICES[3],
        }
    }

    /// The Orientation value for a picture at `orientation` once this is
    /// done to it as it is shown.
    pub fn after(self, orientation: u16) -> u16 {
        let old = MATRICES[(orientation.clamp(1, 8) - 1) as usize];
        let op = self.matrix();
        let new = [
            op[0] * old[0] + op[1] * old[2],
            op[0] * old[1] + op[1] * old[3],
            op[2] * old[0] + op[3] * old[2],
            op[2] * old[1] + op[3] * old[3],
        ];
        MATRICES
            .iter()
            .position(|m| *m == new)
            .map_or(UPRIGHT, |i| i as u16 + 1)
    }
}

/// Whether `path` can be turned here: a JPEG, by its name.
pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
}

/// Do `turn` to the photograph at `path`. Returns the Orientation value it
/// had and the one it has now, which is what undoing it needs.
pub fn apply(path: &Path, turn: Turn) -> io::Result<(u16, u16)> {
    let bytes = fs::read(path)?;
    let from = match find(&bytes)? {
        Tag::At { offset, big } => read_u16(&bytes, offset, big),
        Tag::NoExif { .. } => UPRIGHT,
    };
    let to = turn.after(from);
    set_in(path, &bytes, to)?;
    Ok((from, to))
}

/// The Orientation of the photograph at `path`: upright when it says none.
pub fn orientation(path: &Path) -> io::Result<u16> {
    let bytes = fs::read(path)?;
    Ok(match find(&bytes)? {
        Tag::At { offset, big } => read_u16(&bytes, offset, big),
        Tag::NoExif { .. } => UPRIGHT,
    })
}

/// Write `value` as the Orientation of the photograph at `path`.
pub fn set(path: &Path, value: u16) -> io::Result<()> {
    let bytes = fs::read(path)?;
    set_in(path, &bytes, value)
}

fn set_in(path: &Path, bytes: &[u8], value: u16) -> io::Result<()> {
    match find(bytes)? {
        Tag::At { offset, big } => {
            let data = if big {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            };
            let mut file = fs::OpenOptions::new().write(true).open(path)?;
            file.seek(SeekFrom::Start(offset as u64))?;
            file.write_all(&data)?;
            file.sync_all()
        }
        Tag::NoExif { insert_at } => {
            let mut out = Vec::with_capacity(bytes.len() + 64);
            out.extend_from_slice(&bytes[..insert_at]);
            out.extend_from_slice(&exif_segment(value));
            out.extend_from_slice(&bytes[insert_at..]);
            replace(path, &out)
        }
    }
}

/// Where the Orientation value is, or where an EXIF block would go.
#[derive(Debug, PartialEq, Eq)]
enum Tag {
    /// The value's two bytes start at `offset` in the file, big-endian when
    /// `big`.
    At { offset: usize, big: bool },
    /// The file has no EXIF; a block would go in at `insert_at`.
    NoExif { insert_at: usize },
}

fn unsupported(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, why.to_string())
}

/// Walk the JPEG's segments to its EXIF block and through IFD0 to the
/// Orientation entry. Every read is bounds-checked: a malformed file is an
/// error, never a panic and never a write in the wrong place.
fn find(bytes: &[u8]) -> io::Result<Tag> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return Err(unsupported("not a JPEG"));
    }
    let bad = || unsupported("a damaged JPEG");
    let mut at = 2;
    // An EXIF block goes first, or after a JFIF one that came first.
    let mut insert_at = 2;
    let mut first = true;
    loop {
        if *bytes.get(at).ok_or_else(bad)? != 0xFF {
            return Err(bad());
        }
        while bytes.get(at) == Some(&0xFF) {
            at += 1;
        }
        let marker = *bytes.get(at).ok_or_else(bad)?;
        at += 1;
        match marker {
            0x01 | 0xD0..=0xD7 => continue,
            0xD9 | 0xDA => return Ok(Tag::NoExif { insert_at }),
            _ => {}
        }
        let len = u16::from_be_bytes([
            *bytes.get(at).ok_or_else(bad)?,
            *bytes.get(at + 1).ok_or_else(bad)?,
        ]) as usize;
        if len < 2 {
            return Err(bad());
        }
        let body = at + 2;
        let end = at + len;
        let segment = bytes.get(body..end).ok_or_else(bad)?;
        if marker == 0xE0 && first {
            insert_at = end;
        }
        first = false;
        if marker == 0xE1 && segment.starts_with(b"Exif\0\0") {
            let tiff_start = body + 6;
            return orientation_in(bytes.get(tiff_start..end).ok_or_else(bad)?)
                .map(|(offset, big)| Tag::At {
                    offset: tiff_start + offset,
                    big,
                })
                .ok_or_else(|| unsupported("its EXIF has no orientation to change"));
        }
        at = end;
    }
}

/// The offset of the Orientation value inside a TIFF block, and whether the
/// block is big-endian.
fn orientation_in(tiff: &[u8]) -> Option<(usize, bool)> {
    let big = match tiff.get(..4)? {
        b"MM\0\x2A" => true,
        b"II\x2A\0" => false,
        _ => return None,
    };
    let ifd = read_u32(tiff, 4, big)? as usize;
    let count = read_u16_checked(tiff, ifd, big)? as usize;
    for entry in 0..count.min(tiff.len() / 12) {
        let at = ifd.checked_add(2 + entry * 12)?;
        let tag = read_u16_checked(tiff, at, big)?;
        let kind = read_u16_checked(tiff, at + 2, big)?;
        if tag == 0x0112 && kind == 3 {
            let offset = at + 8;
            tiff.get(offset..offset + 2)?;
            return Some((offset, big));
        }
    }
    None
}

fn read_u16(bytes: &[u8], at: usize, big: bool) -> u16 {
    read_u16_checked(bytes, at, big).unwrap_or(UPRIGHT)
}

fn read_u16_checked(bytes: &[u8], at: usize, big: bool) -> Option<u16> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    let b = [b[0], b[1]];
    Some(if big {
        u16::from_be_bytes(b)
    } else {
        u16::from_le_bytes(b)
    })
}

fn read_u32(bytes: &[u8], at: usize, big: bool) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    let b = [b[0], b[1], b[2], b[3]];
    Some(if big {
        u32::from_be_bytes(b)
    } else {
        u32::from_le_bytes(b)
    })
}

/// An APP1 segment holding EXIF with one entry: the Orientation.
fn exif_segment(value: u16) -> Vec<u8> {
    let mut tiff = b"II\x2A\0".to_vec();
    tiff.extend_from_slice(&8u32.to_le_bytes()); // IFD0 straight after.
    tiff.extend_from_slice(&1u16.to_le_bytes()); // One entry:
    tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation,
    tiff.extend_from_slice(&3u16.to_le_bytes()); // a SHORT,
    tiff.extend_from_slice(&1u32.to_le_bytes()); // one of them,
    tiff.extend_from_slice(&value.to_le_bytes()); // this one,
    tiff.extend_from_slice(&[0, 0]); // padded to four bytes.
    tiff.extend_from_slice(&0u32.to_le_bytes()); // No IFD1.
    let mut out = vec![0xFF, 0xE1];
    out.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&tiff);
    out
}

/// Replace the file at `path` with `bytes`, through a temporary file beside
/// it so a failure part-way leaves the original whole. Keeps its permissions.
///
/// The temporary file is made fresh, `O_EXCL` under a name no other turn is
/// using: whatever already holds a name — a symlink planted to point the
/// write somewhere else among them — is passed over, never written through.
fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let permissions = fs::metadata(path)?.permissions();
    let (temp, mut file) = create_temp(path)?;
    let result = (|| {
        file.write_all(bytes)?;
        // On the open file, not by name: the name could be anything by now.
        file.set_permissions(fs::Permissions::from_mode(permissions.mode()))?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// The temporary name for the `attempt`th try at replacing `path`.
fn temp_name(path: &Path, attempt: u32) -> std::path::PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    dir.join(format!(
        ".{name}.otto-turn-{}-{attempt}",
        std::process::id()
    ))
}

/// A new, empty, private file beside `path`, and its name.
fn create_temp(path: &Path) -> io::Result<(std::path::PathBuf, fs::File)> {
    use std::os::unix::fs::OpenOptionsExt;
    for attempt in 0..100 {
        let temp = temp_name(path, attempt);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_quarter_turns_come_back_round() {
        for start in 1..=8 {
            let mut o = start;
            for _ in 0..4 {
                o = Turn::Right.after(o);
            }
            assert_eq!(o, start);
            assert_eq!(Turn::Left.after(Turn::Right.after(start)), start);
            assert_eq!(
                Turn::FlipHorizontal.after(Turn::FlipHorizontal.after(start)),
                start
            );
            assert_eq!(
                Turn::FlipVertical.after(Turn::FlipVertical.after(start)),
                start
            );
        }
    }

    #[test]
    fn turns_land_on_the_values_cameras_write() {
        assert_eq!(Turn::Right.after(1), 6);
        assert_eq!(Turn::Left.after(1), 8);
        assert_eq!(Turn::Right.after(Turn::Right.after(1)), 3);
        assert_eq!(Turn::FlipHorizontal.after(1), 2);
        assert_eq!(Turn::FlipVertical.after(1), 4);
        // A phone portrait (6) turned back anticlockwise is as stored.
        assert_eq!(Turn::Left.after(6), 1);
        // Mirrored, then turned: the diagonal flips.
        assert_eq!(Turn::Right.after(2), 7);
        assert_eq!(Turn::Left.after(2), 5);
    }

    /// A tiny JPEG: SOI, an optional APP1, then SOS and EOI.
    fn jpeg(app1: Option<Vec<u8>>) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        if let Some(segment) = app1 {
            out.extend_from_slice(&segment);
        }
        out.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
        out
    }

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("otto-orient-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_tagged_photo_is_turned_in_place() {
        let path = temp("tagged.jpg", &jpeg(Some(exif_segment(1))));
        let before = fs::read(&path).unwrap().len();
        assert_eq!(apply(&path, Turn::Right).unwrap(), (1, 6));
        assert_eq!(apply(&path, Turn::Right).unwrap(), (6, 3));
        assert_eq!(
            fs::read(&path).unwrap().len(),
            before,
            "two bytes, in place"
        );
        set(&path, 1).unwrap();
        assert_eq!(apply(&path, Turn::FlipHorizontal).unwrap(), (1, 2));
        let _ = fs::remove_file(&path);
    }

    /// A symlink planted at the temporary name is never written through:
    /// the file it points at is untouched and the photo still turns.
    #[test]
    fn a_symlink_at_the_temporary_name_is_not_followed() {
        let victim = temp("victim.txt", b"keep me");
        // A photo with EXIF is turned in place; one without goes through
        // `replace`, which is what makes the temporary file.
        let plain = temp("planted-plain.jpg", &jpeg(None));
        let planted_plain = temp_name(&plain, 0);
        let _ = fs::remove_file(&planted_plain);
        std::os::unix::fs::symlink(&victim, &planted_plain).unwrap();
        assert_eq!(apply(&plain, Turn::Right).unwrap(), (1, 6));

        assert_eq!(fs::read(&victim).unwrap(), b"keep me");
        assert_eq!(orientation(&plain).unwrap(), 6);
        assert!(planted_plain.symlink_metadata().unwrap().is_symlink());
        for file in [&victim, &plain, &planted_plain] {
            let _ = fs::remove_file(file);
        }
    }

    #[test]
    fn a_photo_without_exif_is_given_some() {
        let path = temp("plain.jpg", &jpeg(None));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(apply(&path, Turn::Left).unwrap(), (1, 8));
        assert_eq!(
            find(&fs::read(&path).unwrap()).unwrap(),
            Tag::At {
                offset: 2 + 4 + 6 + 8 + 2 + 8,
                big: false
            }
        );
        assert_eq!(apply(&path, Turn::Right).unwrap(), (8, 1));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn exif_without_an_orientation_is_left_alone() {
        // An EXIF block whose one entry is something else (Make).
        let mut segment = exif_segment(1);
        segment[4 + 6 + 10] = 0x0F;
        segment[4 + 6 + 11] = 0x01;
        let original = jpeg(Some(segment));
        let path = temp("noorient.jpg", &original);
        let err = apply(&path, Turn::Right).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert_eq!(fs::read(&path).unwrap(), original, "untouched");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn damaged_files_are_errors_not_writes() {
        let whole = jpeg(Some(exif_segment(6)));
        for end in 0..whole.len() - 1 {
            let _ = find(&whole[..end]);
        }
        assert!(find(b"not a jpeg").is_err());
    }
}
