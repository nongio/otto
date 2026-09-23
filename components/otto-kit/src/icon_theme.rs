//! Icon theme detection.
//!
//! Otto publishes its icon theme as `org.freedesktop.appearance icon-theme` on
//! the settings portal. That key is Otto's own: neither the KDE nor the GNOME
//! portal sets it, and an empty theme name makes every lookup search `hicolor`
//! alone — which ships no `folder` and no mimetype icons, so a file listing
//! draws without any. An app takes its theme from, in order:
//!
//! 1. the portal's `org.freedesktop.appearance icon-theme` (Otto);
//! 2. the portal's desktop namespaces — `org.gnome.desktop.interface
//!    icon-theme` (GNOME), `org.kde.kdeglobals.Icons Theme` (Plasma);
//! 3. [`detect_icon_theme`], read synchronously at startup, so the first frame
//!    already has icons.
//!
//! No freedesktop standard records which theme the user chose — each desktop
//! keeps it in its own settings — so [`detect_icon_theme`] reads those: the
//! user's own choice in any desktop first, then the defaults the distribution
//! ships, then any complete theme that is installed. The compositor makes the
//! same guess once, on a user's first session, and writes it to their
//! configuration.
//!
//! A theme is only taken if it is actually installed. Any otto-kit app gets the
//! result through `current_icon_theme()`.

use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};
use zbus::zvariant::{OwnedValue, Value};

/// The current icon theme name. Empty string means auto-detect / no preference.
static ICON_THEME: LazyLock<RwLock<String>> = LazyLock::new(|| RwLock::new(String::new()));

/// Read the current icon theme name.
///
/// Returns `None` if no theme has been configured (empty string from portal).
pub fn current_icon_theme() -> Option<String> {
    let theme = ICON_THEME.read().unwrap();
    if theme.is_empty() {
        None
    } else {
        Some(theme.clone())
    }
}

/// Replace the theme, dropping every icon resolved against the old one.
fn set_theme(theme: String) {
    {
        let mut current = ICON_THEME.write().unwrap();
        if *current == theme {
            return;
        }
        *current = theme;
    }
    // Every cached icon — and every cached miss — answers for the theme that
    // has just been replaced.
    crate::icons::clear_cache();
    crate::portal_runtime::theme_changed();
}

/// Settle on a theme from the desktop's own files, and spawn a background task
/// that:
/// 1. Reads the theme from the XDG Settings portal.
/// 2. Subscribes to `SettingChanged` and updates the value on every change.
///
/// Safe to call multiple times — only one watcher is ever active.
pub fn spawn_icon_theme_watcher() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    // Before the portal answers — and in place of it, where there is none.
    if current_icon_theme().is_none() {
        if let Some(theme) = detect_icon_theme() {
            tracing::debug!("icon-theme from the desktop's files: {theme}");
            set_theme(theme);
        }
    }

    crate::portal_runtime::spawn("icon-theme-watcher", async move {
        if let Err(e) = run_watcher().await {
            tracing::warn!("icon-theme watcher stopped: {e}");
        }
    });
}

/// Extract a string from a possibly variant-wrapped `Value`.
fn extract_string(val: Value<'_>) -> Option<String> {
    match val {
        Value::Str(s) => Some(s.to_string()),
        Value::Value(inner) => extract_string(*inner),
        _ => None,
    }
}

/// Where the portal carries an icon theme, most specific first. Otto's key
/// wins over a desktop's own wherever both are answered.
const PORTAL_KEYS: [(&str, &str); 3] = [
    ("org.freedesktop.appearance", "icon-theme"),
    ("org.gnome.desktop.interface", "icon-theme"),
    ("org.kde.kdeglobals.Icons", "Theme"),
];

async fn run_watcher() -> Result<(), zbus::Error> {
    use zbus::{proxy, Connection};

    #[proxy(
        interface = "org.freedesktop.portal.Settings",
        default_service = "org.freedesktop.portal.Desktop",
        default_path = "/org/freedesktop/portal/desktop"
    )]
    trait Settings {
        fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;
        #[zbus(signal)]
        fn setting_changed(&self, namespace: &str, key: &str, value: Value<'_>)
            -> zbus::Result<()>;
    }

    let conn = Connection::session().await?;
    let proxy = SettingsProxy::new(&conn).await?;

    // The rank of the key the current theme came from; a change under a less
    // specific key does not override a more specific one.
    let mut source = PORTAL_KEYS.len();
    for (rank, (namespace, key)) in PORTAL_KEYS.iter().enumerate() {
        match proxy.read(namespace, key).await {
            Ok(owned) => {
                let val: Value<'_> = owned.into();
                if let Some(theme) = extract_string(val).filter(|t| is_installed(t)) {
                    tracing::debug!("icon-theme initial value from {namespace}: {theme}");
                    set_theme(theme);
                    source = rank;
                    break;
                }
            }
            Err(e) => tracing::debug!("icon-theme read of {namespace} failed: {e}"),
        }
    }

    // Watch for changes via zbus signal stream.
    let mut stream = proxy.receive_setting_changed().await?;
    loop {
        use futures_util::StreamExt as _;
        let Some(signal) = stream.next().await else {
            break;
        };
        let args = signal.args()?;
        let Some(rank) = PORTAL_KEYS
            .iter()
            .position(|(namespace, key)| args.namespace == *namespace && args.key == *key)
        else {
            continue;
        };
        if rank > source {
            continue;
        }
        if let Some(theme) = extract_string(args.value).filter(|t| is_installed(t)) {
            tracing::debug!("icon-theme changed to: {theme}");
            source = rank;
            set_theme(theme);
        }
    }

    Ok(())
}

/// Guess the icon theme from the desktops' own settings.
///
/// For when nothing names a theme: the compositor's first run writes it to a
/// new user's configuration, and otto-kit apps use it before the portal
/// answers. Tries, in order:
///
/// 1. the running desktop's own setting, where that desktop is not Otto;
/// 2. a theme the user picked in any desktop — Plasma's `kdeglobals`, GNOME,
///    Cinnamon and MATE through `dconf`, Xfce's `xsettings.xml`, GTK's
///    `settings.ini`, then qt6ct and qt5ct;
/// 3. the distribution's defaults — GSettings overrides (where Ubuntu, Mint,
///    elementary and Zorin name their themes), then the system-wide
///    `kdeglobals`, `xsettings.xml` and `settings.ini`;
/// 4. Breeze, then Adwaita.
///
/// Only an installed theme is returned. Reading `dconf` spawns a process, so
/// this belongs at startup, not on a hot path.
pub fn detect_icon_theme() -> Option<String> {
    detect(&Dirs::from_env(), dconf_read)
}

/// Whether a theme called `name` is installed.
fn is_installed(name: &str) -> bool {
    Dirs::from_env().has_theme(name)
}

/// The directories the guess reads, resolved once from the XDG variables.
#[derive(Debug, Default)]
struct Dirs {
    home: Option<PathBuf>,
    config_home: Option<PathBuf>,
    config_dirs: Vec<PathBuf>,
    data_home: Option<PathBuf>,
    data_dirs: Vec<PathBuf>,
    /// Where GTK looks for its system-wide `settings.ini` besides
    /// `$XDG_CONFIG_DIRS`.
    sysconf_dir: Option<PathBuf>,
    current_desktops: Vec<String>,
}

impl Dirs {
    fn from_env() -> Self {
        let absolute = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        let list = |var: &str, default: &str| -> Vec<PathBuf> {
            let value = std::env::var(var)
                .ok()
                .filter(|dirs| !dirs.is_empty())
                .unwrap_or_else(|| default.to_string());
            value
                .split(':')
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .collect()
        };
        let home = absolute("HOME");
        Self {
            config_home: absolute("XDG_CONFIG_HOME")
                .or_else(|| home.as_ref().map(|h| h.join(".config"))),
            config_dirs: list("XDG_CONFIG_DIRS", "/etc/xdg"),
            data_home: absolute("XDG_DATA_HOME")
                .or_else(|| home.as_ref().map(|h| h.join(".local/share"))),
            data_dirs: list("XDG_DATA_DIRS", "/usr/local/share:/usr/share"),
            sysconf_dir: Some(PathBuf::from("/etc")),
            current_desktops: std::env::var("XDG_CURRENT_DESKTOP")
                .map(|d| d.split(':').map(str::to_string).collect())
                .unwrap_or_default(),
            home,
        }
    }

    fn is_current(&self, desktops: &[&str]) -> bool {
        self.current_desktops
            .iter()
            .any(|current| desktops.iter().any(|d| current.eq_ignore_ascii_case(d)))
    }

    /// Whether `name` has an `index.theme` anywhere icons are looked for.
    fn has_theme(&self, name: &str) -> bool {
        if name.is_empty() || name.contains('/') {
            return false;
        }
        let mut roots = self
            .home
            .iter()
            .map(|home| home.join(".icons"))
            .chain(self.data_home.iter().map(|d| d.join("icons")))
            .chain(self.data_dirs.iter().map(|d| d.join("icons")));
        roots.any(|root| root.join(name).join("index.theme").is_file())
    }
}

/// One place a desktop keeps its icon theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    UserKdeglobals,
    /// A key in the user's dconf database.
    Dconf(&'static str),
    UserXsettings,
    UserGtk,
    UserQtct,
    /// A GSettings schema's `icon-theme`, as the distribution overrides it.
    SchemaOverride(&'static str),
    SystemKdeglobals,
    SystemXsettings,
    SystemGtk,
    /// A theme taken by name alone.
    Named(&'static str),
}

const GNOME_KEY: Source = Source::Dconf("/org/gnome/desktop/interface/icon-theme");
const CINNAMON_KEY: Source = Source::Dconf("/org/cinnamon/desktop/interface/icon-theme");
const MATE_KEY: Source = Source::Dconf("/org/mate/desktop/interface/icon-theme");

/// Where the user may have picked a theme, most specific first.
const USER_SOURCES: [Source; 7] = [
    Source::UserKdeglobals,
    GNOME_KEY,
    CINNAMON_KEY,
    MATE_KEY,
    Source::UserXsettings,
    Source::UserGtk,
    Source::UserQtct,
];

/// Where a distribution sets its default theme.
const SYSTEM_SOURCES: [Source; 6] = [
    Source::SchemaOverride("org.gnome.desktop.interface"),
    Source::SchemaOverride("org.cinnamon.desktop.interface"),
    Source::SchemaOverride("org.mate.interface"),
    Source::SystemKdeglobals,
    Source::SystemXsettings,
    Source::SystemGtk,
];

/// Complete themes to fall back on. Anything is better than `hicolor` alone.
const FALLBACKS: [Source; 2] = [Source::Named("breeze"), Source::Named("Adwaita")];

/// The sources to try, the running desktop's own first.
fn sources(dirs: &Dirs) -> Vec<Source> {
    let current: &[Source] = if dirs.is_current(&["KDE"]) {
        // Plasma's own default when kdeglobals names none.
        &[
            Source::UserKdeglobals,
            Source::SystemKdeglobals,
            Source::Named("breeze"),
        ]
    } else if dirs.is_current(&["GNOME", "Unity", "Budgie", "Pantheon"]) {
        &[GNOME_KEY]
    } else if dirs.is_current(&["X-Cinnamon"]) {
        &[CINNAMON_KEY]
    } else if dirs.is_current(&["MATE"]) {
        &[MATE_KEY]
    } else if dirs.is_current(&["XFCE"]) {
        &[Source::UserXsettings, Source::SystemXsettings]
    } else {
        &[]
    };
    let mut order: Vec<Source> = Vec::new();
    for source in current
        .iter()
        .chain(&USER_SOURCES)
        .chain(&SYSTEM_SOURCES)
        .chain(&FALLBACKS)
    {
        if !order.contains(source) {
            order.push(*source);
        }
    }
    order
}

fn detect(dirs: &Dirs, dconf: impl Fn(&str) -> Option<String>) -> Option<String> {
    sources(dirs).into_iter().find_map(|source| {
        let theme = read_source(dirs, source, &dconf).filter(|theme| dirs.has_theme(theme))?;
        tracing::debug!(?source, theme, "icon theme guessed");
        Some(theme)
    })
}

fn read_source(
    dirs: &Dirs,
    source: Source,
    dconf: &impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let user = |relative: &str| {
        let path = dirs.config_home.as_ref()?.join(relative);
        std::fs::read_to_string(path).ok()
    };
    let system = |relative: &str| {
        dirs.config_dirs
            .iter()
            .find_map(|dir| std::fs::read_to_string(dir.join(relative)).ok())
    };
    let kdeglobals = |text: String| ini_value(&text, "Icons", "Theme");
    let gtk = |text: String| ini_value(&text, "Settings", "gtk-icon-theme-name");

    match source {
        Source::UserKdeglobals => user("kdeglobals").and_then(kdeglobals),
        Source::Dconf(key) => {
            // Values the user set live in this file; without it `dconf` has
            // nothing to say, so do not spawn it.
            let db = dirs.config_home.as_ref()?.join("dconf/user");
            db.is_file().then(|| dconf(key)).flatten()
        }
        Source::UserXsettings => user(XSETTINGS).and_then(|text| xsettings_icon_theme(&text)),
        Source::UserGtk => ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
            .into_iter()
            .find_map(|file| user(file).and_then(gtk)),
        Source::UserQtct => ["qt6ct/qt6ct.conf", "qt5ct/qt5ct.conf"]
            .into_iter()
            .find_map(|file| user(file).and_then(|t| ini_value(&t, "Appearance", "icon_theme"))),
        Source::SchemaOverride(schema) => schema_override(&dirs.data_dirs, schema),
        Source::SystemKdeglobals => system("kdeglobals").and_then(kdeglobals),
        Source::SystemXsettings => system(XSETTINGS).and_then(|text| xsettings_icon_theme(&text)),
        Source::SystemGtk => dirs
            .config_dirs
            .iter()
            .chain(&dirs.sysconf_dir)
            .flat_map(|dir| {
                [
                    dir.join("gtk-4.0/settings.ini"),
                    dir.join("gtk-3.0/settings.ini"),
                ]
            })
            .find_map(|path| std::fs::read_to_string(path).ok().and_then(gtk)),
        Source::Named(theme) => Some(theme.to_string()),
    }
}

/// Where Xfce keeps its XSETTINGS, relative to a config directory.
const XSETTINGS: &str = "xfce4/xfconf/xfce-perchannel-xml/xsettings.xml";

/// `Net/IconThemeName` from an xfconf `xsettings.xml` channel.
fn xsettings_icon_theme(text: &str) -> Option<String> {
    let tag = &text[text.find("name=\"IconThemeName\"")?..];
    let tag = &tag[..tag.find('>')?];
    let value = tag.split_once("value=\"")?.1;
    let value = &value[..value.find('"')?];
    (!value.is_empty()).then(|| value.to_string())
}

/// `icon-theme` for `schema` from the `*.gschema.override` files in the first
/// data directory that has one; later files win, as `glib-compile-schemas`
/// applies them. A desktop-specific group (`[schema:ubuntu]`) counts too: it
/// is the distribution naming its theme.
fn schema_override(data_dirs: &[PathBuf], schema: &str) -> Option<String> {
    data_dirs.iter().find_map(|dir| {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join("glib-2.0/schemas"))
            .ok()?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".gschema.override"))
            })
            .collect();
        files.sort();
        files.iter().rev().find_map(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            ini_last(
                &text,
                |group| {
                    group == schema
                        || group
                            .strip_prefix(schema)
                            .is_some_and(|rest| rest.starts_with(':'))
                },
                "icon-theme",
            )
        })
    })
}

/// A GSettings key from `dconf`, unquoted.
fn dconf_read(key: &str) -> Option<String> {
    let output = std::process::Command::new("dconf")
        .args(["read", key])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    unquote(String::from_utf8(output.stdout).ok()?.trim())
}

/// A value with its surrounding quotes removed, `None` when empty. Covers
/// plain INI values and GVariant strings (`'Yaru'`) alike.
fn unquote(value: &str) -> Option<String> {
    let value = value.trim();
    let value = ['"', '\'']
        .iter()
        .find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
        .unwrap_or(value);
    (!value.is_empty()).then(|| value.to_string())
}

/// `key` in `[group]` of an INI-style file — the format of `kdeglobals`,
/// GTK's `settings.ini` and the Qt configuration tools.
fn ini_value(text: &str, group: &str, key: &str) -> Option<String> {
    ini_last(text, |name| name == group, key)
}

/// The last `key` in any group `group_matches` accepts; a later entry
/// overrides an earlier one, as in every reader of these files.
fn ini_last(text: &str, group_matches: impl Fn(&str) -> bool, key: &str) -> Option<String> {
    let mut in_group = false;
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_group = group_matches(name);
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() == key {
            found = unquote(v).or(found);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{detect, ini_value, xsettings_icon_theme, Dirs};
    use std::path::PathBuf;

    /// A home and a system laid out under a temporary directory.
    struct Fixture {
        root: PathBuf,
        dirs: Dirs,
    }

    impl Fixture {
        fn new(current_desktop: &str) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "otto-icon-theme-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            let dirs = Dirs {
                home: Some(root.join("home")),
                config_home: Some(root.join("home/.config")),
                config_dirs: vec![root.join("etc/xdg")],
                data_home: Some(root.join("home/.local/share")),
                data_dirs: vec![root.join("usr/share")],
                sysconf_dir: Some(root.join("etc")),
                current_desktops: vec![current_desktop.to_string()],
            };
            Self { root, dirs }
        }

        fn write(&self, relative: &str, text: &str) -> &Self {
            let path = self.root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
            self
        }

        fn install(&self, theme: &str) -> &Self {
            self.write(
                &format!("usr/share/icons/{theme}/index.theme"),
                "[Icon Theme]\n",
            )
        }

        fn detect(&self, dconf: impl Fn(&str) -> Option<String>) -> Option<String> {
            detect(&self.dirs, dconf)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn no_dconf(_: &str) -> Option<String> {
        None
    }

    const OVERRIDES: &str = "usr/share/glib-2.0/schemas";

    #[test]
    fn a_distribution_override_names_the_theme() {
        let fx = Fixture::new("otto");
        fx.install("Adwaita").install("Yaru");
        fx.write(
            &format!("{OVERRIDES}/00_base.gschema.override"),
            "[org.gnome.desktop.interface]\nicon-theme='Adwaita'\n",
        );
        fx.write(
            &format!("{OVERRIDES}/10_ubuntu-settings.gschema.override"),
            "[org.gnome.desktop.interface:ubuntu]\ngtk-theme='Yaru'\nicon-theme='Yaru'\n",
        );
        assert_eq!(fx.detect(no_dconf).as_deref(), Some("Yaru"));
    }

    #[test]
    fn the_users_choice_beats_the_distribution() {
        let fx = Fixture::new("otto");
        fx.install("Yaru").install("Papirus");
        fx.write(
            &format!("{OVERRIDES}/10_ubuntu-settings.gschema.override"),
            "[org.gnome.desktop.interface:ubuntu]\nicon-theme='Yaru'\n",
        );
        fx.write(
            "home/.config/gtk-3.0/settings.ini",
            "[Settings]\ngtk-icon-theme-name=Papirus\n",
        );
        assert_eq!(fx.detect(no_dconf).as_deref(), Some("Papirus"));
    }

    #[test]
    fn a_theme_that_is_not_installed_is_passed_over() {
        let fx = Fixture::new("otto");
        fx.install("Adwaita");
        fx.write("home/.config/kdeglobals", "[Icons]\nTheme=WhiteSur\n");
        assert_eq!(fx.detect(no_dconf).as_deref(), Some("Adwaita"));
    }

    #[test]
    fn dconf_is_read_only_when_the_user_has_a_database() {
        let fx = Fixture::new("otto");
        fx.install("Adwaita").install("Papirus");
        let dconf = |key: &str| {
            key.starts_with("/org/gnome/")
                .then(|| "Papirus".to_string())
        };
        assert_eq!(fx.detect(dconf).as_deref(), Some("Adwaita"));
        fx.write("home/.config/dconf/user", "");
        assert_eq!(fx.detect(dconf).as_deref(), Some("Papirus"));
    }

    #[test]
    fn plasma_falls_back_to_breeze_before_other_desktops() {
        let fx = Fixture::new("KDE");
        fx.install("breeze").install("Papirus");
        fx.write("home/.config/dconf/user", "");
        let dconf = |_: &str| Some("Papirus".to_string());
        assert_eq!(fx.detect(dconf).as_deref(), Some("breeze"));
    }

    #[test]
    fn xfce_system_settings_are_read() {
        let fx = Fixture::new("otto");
        fx.install("elementary-xfce-dark");
        fx.write(
            "etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xsettings.xml",
            r#"<channel name="xsettings" version="1.0">
  <property name="Net" type="empty">
    <property name="ThemeName" type="string" value="Greybird"/>
    <property name="IconThemeName" type="string" value="elementary-xfce-dark"/>
  </property>
</channel>"#,
        );
        assert_eq!(fx.detect(no_dconf).as_deref(), Some("elementary-xfce-dark"));
    }

    #[test]
    fn nothing_installed_means_no_guess() {
        let fx = Fixture::new("otto");
        fx.write("home/.config/kdeglobals", "[Icons]\nTheme=breeze\n");
        assert_eq!(fx.detect(no_dconf), None);
    }

    #[test]
    fn xsettings_without_an_icon_theme_names_none() {
        let text = r#"<property name="ThemeName" type="string" value="Greybird"/>"#;
        assert_eq!(xsettings_icon_theme(text), None);
    }

    #[test]
    fn reads_a_key_from_its_own_group_only() {
        let kdeglobals =
            "[General]\nTheme=wrong\n\n[Icons]\nTheme=WhiteSur\n\n[KDE]\nTheme=also-wrong\n";
        assert_eq!(
            ini_value(kdeglobals, "Icons", "Theme").as_deref(),
            Some("WhiteSur")
        );
        let gtk = "[Settings]\ngtk-theme-name=Adwaita\ngtk-icon-theme-name = \"Newaita\"\n";
        assert_eq!(
            ini_value(gtk, "Settings", "gtk-icon-theme-name").as_deref(),
            Some("Newaita")
        );
        assert_eq!(ini_value(gtk, "Icons", "Theme"), None);
    }
}
