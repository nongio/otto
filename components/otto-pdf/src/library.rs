//! Finding and loading `libpdfium.so`.
//!
//! PDFium is never linked: it is opened the first time something asks for
//! it, and a system without it runs everything except the editing. The
//! places looked in, in order:
//!
//! 1. `$OTTO_PDFIUM`, the path of the library file itself;
//! 2. `libpdfium.so` beside the running executable;
//! 3. `<executable's directory>/../lib/otto/libpdfium.so`;
//! 4. `/usr/lib/otto/libpdfium.so`, then `/usr/local/lib/otto/libpdfium.so`;
//! 5. `$HOME/.local/lib/otto/libpdfium.so`;
//! 6. plain `libpdfium.so`, through the system loader's own search.
//!
//! The first that loads and has every function wins. The answer, a failure
//! included, is kept for the life of the process.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::bindings::{Bindings, FPDF_LIBRARY_CONFIG};
use crate::Error;

const FILE_NAME: &str = "libpdfium.so";

static LOADED: OnceLock<Result<Bindings, String>> = OnceLock::new();

/// PDFium's functions, loading the library and initialising it the first
/// time.
///
/// # Errors
///
/// [`Error::Unavailable`] when no candidate loads, with what was tried and
/// why each failed. The same answer comes back on every later call.
pub fn load() -> Result<&'static Bindings, Error> {
    match LOADED.get_or_init(open) {
        Ok(bindings) => Ok(bindings),
        Err(why) => Err(Error::Unavailable(why.clone())),
    }
}

/// The library paths to try, in order.
fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("OTTO_PDFIUM").filter(|path| !path.is_empty()) {
        paths.push(PathBuf::from(path));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        paths.push(dir.join(FILE_NAME));
        paths.push(dir.join("../lib/otto").join(FILE_NAME));
    }
    paths.push(PathBuf::from("/usr/lib/otto").join(FILE_NAME));
    paths.push(PathBuf::from("/usr/local/lib/otto").join(FILE_NAME));
    if let Some(home) = otto_foundations::xdg::home() {
        paths.push(home.join(".local/lib/otto").join(FILE_NAME));
    }
    paths
}

fn open() -> Result<Bindings, String> {
    let mut tried = Vec::new();
    for path in candidates() {
        if !path.is_file() {
            continue;
        }
        match bind(path.as_os_str()) {
            Ok(bindings) => return Ok(init(bindings, &path)),
            Err(why) => tried.push(format!("{}: {why}", path.display())),
        }
    }
    match bind(FILE_NAME.as_ref()) {
        Ok(bindings) => return Ok(init(bindings, Path::new(FILE_NAME))),
        Err(why) => tried.push(format!("{FILE_NAME}: {why}")),
    }
    let why = tried.join("; ");
    tracing::info!("PDFium not loaded: {why}");
    Err(why)
}

fn bind(name: &std::ffi::OsStr) -> Result<Bindings, String> {
    // SAFETY: loading runs the library's initialisers; a PDFium build has
    // none with preconditions. A file at one of these paths is trusted to be
    // PDFium, as any installed library is.
    let library = unsafe { libloading::Library::new(name) }.map_err(|error| error.to_string())?;
    // SAFETY: the signatures in `bindings` are those of the PDFium headers
    // the crate is written against.
    unsafe { Bindings::resolve(library) }
}

fn init(bindings: Bindings, location: &Path) -> Bindings {
    let config = FPDF_LIBRARY_CONFIG {
        version: 2,
        m_pUserFontPaths: std::ptr::null(),
        m_pIsolate: std::ptr::null_mut(),
        m_v8EmbedderSlot: 0,
    };
    // SAFETY: called once per process, inside `OnceLock::get_or_init`, so no
    // other PDFium call can run yet; `config` is a valid version 2 config
    // that outlives the call.
    unsafe { (bindings.FPDF_InitLibraryWithConfig)(&config) };
    tracing::debug!("PDFium loaded from {}", location.display());
    bindings
}
