//! Throwing files away into the freedesktop trash can.
//!
//! An item in the trash lives under `files/`, and beside it under `info/` sits
//! a `.trashinfo` sidecar recording where it came from and when it left, so
//! any spec-compliant file manager can put it back.

use std::path::{Path, PathBuf};

use crate::fs::{first_free_name, move_entry};

/// `$XDG_DATA_HOME/Trash`, falling back to `~/.local/share/Trash`: the
/// "home trashcan" the freedesktop Trash spec describes.
pub fn home_trash_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir).join("Trash"));
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".local/share/Trash"))
}

/// Move `source` into the trash can at `trash`, with its sidecar.
///
/// `trash` is the can itself, the directory holding `files/` and `info/`;
/// both are created if missing. A name already in the can gets a numbered
/// variant. Returns where the item landed and where its sidecar went.
///
/// # Errors
///
/// The directory creation, move or sidecar write that failed, as text. A
/// failed sidecar write leaves the item in the can without one.
pub fn trash_into(source: &Path, trash: &Path) -> Result<(PathBuf, PathBuf), String> {
    let name = source
        .file_name()
        .ok_or_else(|| format!("{} has no name to trash it under", source.display()))?;
    let files_dir = trash.join("files");
    let info_dir = trash.join("info");
    std::fs::create_dir_all(&files_dir)
        .and_then(|()| std::fs::create_dir_all(&info_dir))
        .map_err(|e| e.to_string())?;

    let target = first_free_name(&files_dir, name);
    let trashed_name = target
        .file_name()
        .unwrap_or(name)
        .to_string_lossy()
        .into_owned();
    let info_path = info_dir.join(format!("{trashed_name}.trashinfo"));

    move_entry(source, &target)?;

    let info = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode_path(source),
        deletion_date(),
    );
    std::fs::write(&info_path, info).map_err(|e| e.to_string())?;
    Ok((target, info_path))
}

/// Percent-encode a path the way a `.trashinfo`'s `Path=` key requires:
/// everything but the unreserved characters and the `/` separator.
pub fn percent_encode_path(path: &Path) -> String {
    let mut out = String::new();
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Undo [`percent_encode_path`].
///
/// A stray `%` that is not followed by two hex digits is kept as itself rather
/// than dropped: the name is what matters, and a malformed sidecar should
/// still point somewhere recognisable.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Local time as `YYYY-MM-DDTHH:MM:SS`, the format a `.trashinfo`'s
/// `DeletionDate` key requires.
fn deletion_date() -> String {
    // SAFETY: `tm` is plain data and `localtime_r` fills every field read back.
    unsafe {
        let mut when: libc::time_t = 0;
        libc::time(&mut when);
        let mut broken: libc::tm = std::mem::zeroed();
        libc::localtime_r(&when, &mut broken);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            broken.tm_year + 1900,
            broken.tm_mon + 1,
            broken.tm_mday,
            broken.tm_hour,
            broken.tm_min,
            broken.tm_sec,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trashed_file_lands_in_the_can_with_a_sidecar() {
        let root = std::env::temp_dir().join(format!("otto-kit-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("a b.txt");
        std::fs::write(&source, "x").unwrap();
        let trash = root.join("Trash");

        let (to, info) = trash_into(&source, &trash).unwrap();

        assert!(!source.exists(), "the original is gone");
        assert_eq!(to, trash.join("files/a b.txt"));
        let sidecar = std::fs::read_to_string(&info).unwrap();
        assert!(sidecar.contains(&format!("Path={}", percent_encode_path(&source))));

        // A second item under the same name is numbered, not overwritten.
        std::fs::write(&source, "y").unwrap();
        let (again, _) = trash_into(&source, &trash).unwrap();
        assert_eq!(again, trash.join("files/a b 2.txt"));

        std::fs::remove_dir_all(&root).ok();
    }
}
