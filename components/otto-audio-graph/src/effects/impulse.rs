//! Impulse response files: a speaker correction's file is copied into
//! `$XDG_DATA_HOME/otto/audio/impulses/` when it is chosen, and the routing
//! file names the copy, so the effect keeps working when the original moves
//! or goes. PipeWire's convolver reads the copy (any format libsndfile
//! reads: WAV, the `.irs` files EasyEffects uses, FLAC) and says when it
//! cannot.

// Rust guideline compliant 2026-02-21

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The largest impulse response taken. A speaker or room correction is a
/// few KiB to a few MiB; anything far bigger is not one, and the convolver
/// would hold all of it in memory.
pub const MAX_IMPULSE_BYTES: u64 = 64 * 1024 * 1024;

/// Where copied impulse responses are kept.
#[must_use]
pub fn impulses_dir() -> Option<PathBuf> {
    otto_foundations::xdg::data_home().map(|dir| dir.join("otto/audio/impulses"))
}

/// Why an impulse response could not be copied.
#[derive(Debug)]
pub enum ImpulseError {
    /// There is no data directory to copy it into.
    NoDataDir,
    /// The file is over [`MAX_IMPULSE_BYTES`].
    TooBig,
    /// Reading the file or writing the copy failed.
    Io(io::Error),
}

impl fmt::Display for ImpulseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDataDir => write!(f, "no data directory"),
            Self::TooBig => write!(f, "the file is too big for an impulse response"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ImpulseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ImpulseError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Copy the impulse response at `source` into [`impulses_dir`]; returns the
/// copy's file name, which is what an effect's `impulse` keeps.
///
/// # Errors
///
/// When there is no data directory, the file is too big, or it cannot be
/// read or copied.
pub fn import_impulse(source: &Path) -> Result<String, ImpulseError> {
    let dir = impulses_dir().ok_or(ImpulseError::NoDataDir)?;
    import_into(&dir, source)
}

/// [`import_impulse`] into `dir`. A file already there under the same name
/// with the same bytes is reused; one with other bytes is kept, and the
/// copy gets a free name (`room-2.wav`).
pub(super) fn import_into(dir: &Path, source: &Path) -> Result<String, ImpulseError> {
    if std::fs::metadata(source)?.len() > MAX_IMPULSE_BYTES {
        return Err(ImpulseError::TooBig);
    }
    let bytes = std::fs::read(source)?;
    std::fs::create_dir_all(dir)?;
    let file = source.file_name().map_or_else(
        || "impulse.wav".into(),
        |n| n.to_string_lossy().into_owned(),
    );
    let path = Path::new(&file);
    let stem = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()));
    let mut name = file.clone();
    let mut n = 2;
    loop {
        let target = dir.join(&name);
        match std::fs::read(&target) {
            Ok(existing) if existing == bytes => return Ok(name),
            Ok(_) => {
                name = format!("{stem}-{n}{}", extension.as_deref().unwrap_or(""));
                n += 1;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                otto_foundations::fs::write_atomically(&target, &bytes)?;
                return Ok(name);
            }
            Err(error) => return Err(error.into()),
        }
    }
}

/// Where an effect's `impulse` is read from: an absolute path as written,
/// a file name in [`impulses_dir`].
pub(super) fn resolve(impulse: &str) -> Option<PathBuf> {
    let path = Path::new(impulse);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        impulses_dir().map(|dir| dir.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_keeps_its_name_and_a_clash_gets_a_new_one() {
        let from = tempfile::tempdir().unwrap();
        let into = tempfile::tempdir().unwrap();
        let source = from.path().join("room.irs");
        std::fs::write(&source, b"RIFF one").unwrap();
        assert_eq!(import_into(into.path(), &source).unwrap(), "room.irs");
        // The same file again is the same copy.
        assert_eq!(import_into(into.path(), &source).unwrap(), "room.irs");
        std::fs::write(&source, b"RIFF two").unwrap();
        assert_eq!(import_into(into.path(), &source).unwrap(), "room-2.irs");
        assert_eq!(
            std::fs::read(into.path().join("room.irs")).unwrap(),
            b"RIFF one"
        );
        // The copy outlives the original.
        std::fs::remove_file(&source).unwrap();
        assert!(into.path().join("room-2.irs").is_file());
    }

    #[test]
    fn a_missing_source_is_an_error() {
        let into = tempfile::tempdir().unwrap();
        let missing = into.path().join("nope.wav");
        assert!(matches!(
            import_into(into.path(), &missing),
            Err(ImpulseError::Io(_))
        ));
    }
}
