//! Running Otto straight from its source checkout.
//!
//! A packaged Otto finds its wallpaper under `/usr/share/otto`, its icon theme
//! under `/usr/share/icons` and Inter wherever fontconfig looks. A
//! `cargo run -- --winit` from a fresh clone has none of them installed, and
//! comes up on a flat gradient, hicolor's handful of icons and DejaVu.
//!
//! So the checkout stands in for the install. The wallpaper is read from
//! `resources/`, and `scripts/fetch-dev-assets.sh` stages the icon theme and
//! Inter under `target/share`, which [`use_staged_assets`] puts in front of
//! the system's own directories. Every one of these is only a fallback: an
//! installed asset always wins.

use std::path::{Path, PathBuf};

/// The theme the packages install and the shipped configuration names.
pub const ICON_THEME: &str = "Otto-MacTahoe";

/// The source checkout this binary was built from, if it is still there.
pub fn dir() -> Option<&'static Path> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    dir.join("resources")
        .join("wallpaper.jpg")
        .is_file()
        .then_some(dir)
}

/// Where `scripts/fetch-dev-assets.sh` stages the icon theme and the font: an
/// XDG data directory of its own.
fn staged_share() -> Option<PathBuf> {
    dir().map(|dir| dir.join("target").join("share"))
}

/// The wallpaper the packages ship, from the install or else the checkout.
pub fn wallpaper() -> Option<PathBuf> {
    otto_kit::xdg::data_dirs()
        .into_iter()
        .map(|dir| dir.join("otto").join("wallpaper.jpg"))
        .chain(dir().map(|dir| dir.join("resources").join("wallpaper.jpg")))
        .find(|path| path.is_file())
}

/// Make the assets staged in the checkout visible to the compositor and to
/// everything it launches.
///
/// Runs before the configuration is first read, since the default icon theme
/// depends on what is installed.
pub fn use_staged_assets() {
    let Some(share) = staged_share() else {
        return;
    };

    if share
        .join("icons")
        .join(ICON_THEME)
        .join("index.theme")
        .is_file()
    {
        let mut dirs = vec![share.clone()];
        dirs.extend(otto_kit::xdg::data_dirs());
        if let Ok(joined) = std::env::join_paths(dirs) {
            tracing::info!("icons from the checkout: {}", share.display());
            // SAFETY: called from main before any thread reads the environment
            unsafe { std::env::set_var("XDG_DATA_DIRS", joined) };
        }
    }

    // fontconfig reads no XDG data directory for fonts, so the staged
    // configuration includes the system's and adds the checkout's fonts.
    let fonts_conf = share.join("fonts.conf");
    if fonts_conf.is_file() && std::env::var_os("FONTCONFIG_FILE").is_none() {
        tracing::info!("fonts from the checkout: {}", fonts_conf.display());
        // SAFETY: as above, and before fontconfig is first initialised
        unsafe { std::env::set_var("FONTCONFIG_FILE", &fonts_conf) };
    }

    if !otto_kit::icon_theme::is_installed(ICON_THEME) {
        tracing::warn!(
            "{ICON_THEME} is not installed: run scripts/fetch-dev-assets.sh \
             to stage it and Inter in the checkout"
        );
    }
}
