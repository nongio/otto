//! Running Otto straight from its source checkout.
//!
//! A packaged Otto finds its wallpaper under `/usr/share/otto`, its icon theme
//! under `/usr/share/icons` and Inter wherever fontconfig looks. A
//! `cargo run -- --winit` from a fresh clone has none of them installed, and
//! comes up on a flat gradient, hicolor's handful of icons and DejaVu.
//!
//! So the checkout stands in for the install. The wallpaper is read from
//! `resources/`. [`use_checkout_assets`] links the apps' desktop entries into
//! `target/share`, where `scripts/fetch-dev-assets.sh` also stages the icon
//! theme and Inter, and puts that directory in front of the system's own, and
//! the build directory in front of `PATH`: a run from the checkout launches
//! the checkout's apps. Whether the shipped look is used at all is up to the
//! configuration: only while there is none (`crate::config::demo_config`).

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

/// The checkout's own desktop entries, as the packages install them to
/// `/usr/share/applications`: the apps the shipped dock pins.
const DESKTOP_ENTRIES: [&str; 4] = [
    "otto-files.desktop",
    "otto-preview.desktop",
    "otto-settings.desktop",
    "otto-trash.desktop",
];

/// Make the checkout stand in for the install, for the compositor and for
/// everything it launches.
///
/// Runs before the configuration is first read, since the icon theme an
/// unconfigured Otto picks depends on what is installed.
pub fn use_checkout_assets() {
    let (Some(checkout), Some(share)) = (dir(), staged_share()) else {
        return;
    };

    // The apps are the ones built next to this binary: `cargo build
    // --workspace` puts otto-bar, otto-files and the rest there, and their
    // desktop entries name them by bare command.
    if let Some(bin) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .filter(|bin| bin.starts_with(checkout))
    {
        prepend_env("PATH", bin, Vec::new());
    }

    // The desktop entries and otto-files' own icon, linked into the staged
    // data directory under the names an install gives them. Best effort: a
    // link that cannot be made is an icon missing from the dock, not a reason
    // to stop the session.
    let mut links: Vec<(PathBuf, PathBuf)> = DESKTOP_ENTRIES
        .iter()
        .map(|entry| {
            (
                checkout.join("resources").join(entry),
                share.join("applications").join(entry),
            )
        })
        .collect();
    links.push((
        checkout.join("components/otto-files/resources/icons/hicolor"),
        share.join("icons").join("hicolor"),
    ));
    for (target, link) in links {
        if link.symlink_metadata().is_ok() {
            continue;
        }
        let made = link
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::os::unix::fs::symlink(&target, &link));
        if let Err(err) = made {
            tracing::warn!("cannot link {}: {err}", link.display());
        }
    }

    tracing::info!(
        "apps, icons and fonts from the checkout: {}",
        share.display()
    );
    prepend_env("XDG_DATA_DIRS", share.clone(), otto_kit::xdg::data_dirs());

    // fontconfig reads no XDG data directory for fonts, so the staged
    // configuration includes the system's and adds the checkout's fonts.
    let fonts_conf = share.join("fonts.conf");
    if fonts_conf.is_file() && std::env::var_os("FONTCONFIG_FILE").is_none() {
        // SAFETY: called from main before any thread reads the environment,
        // and before fontconfig is first initialised
        unsafe { std::env::set_var("FONTCONFIG_FILE", &fonts_conf) };
    }

    if !otto_kit::icon_theme::is_installed(ICON_THEME) {
        tracing::warn!(
            "{ICON_THEME} is not installed: run scripts/fetch-dev-assets.sh \
             to stage it and Inter in the checkout"
        );
    }
}

/// Put `dir` in front of the path list in `var`. An unset variable is read as
/// `default` — XDG_DATA_DIRS has one that setting it would otherwise drop.
fn prepend_env(var: &str, dir: PathBuf, default: Vec<PathBuf>) {
    let rest = match std::env::var_os(var) {
        Some(value) => std::env::split_paths(&value).collect(),
        None => default,
    };
    let dirs = std::iter::once(dir.clone()).chain(rest.into_iter().filter(|d| *d != dir));
    if let Ok(joined) = std::env::join_paths(dirs) {
        // SAFETY: called from main before any thread reads the environment
        unsafe { std::env::set_var(var, joined) };
    }
}
