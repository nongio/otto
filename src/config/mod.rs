use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod default_apps;
pub mod file;
pub mod shortcuts;

use shortcuts::{build_bindings, RunCommandConfig, ShortcutBinding, ShortcutMap};
use toml::map::Entry;
use tracing::warn;

use crate::theme::ThemeScheme;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub screen_scale: f64,
    #[serde(default)]
    pub displays: DisplaysConfig,
    pub cursor_theme: String,
    pub icon_theme: Option<String>,
    pub cursor_size: u32,
    #[serde(default)]
    pub input: InputConfig,
    #[serde(default)]
    pub dock: DockConfig,
    #[serde(default)]
    pub appswitcher: AppSwitcherConfig,
    #[serde(default)]
    pub layer_shell: LayerShellConfig,
    #[serde(default)]
    pub power_management: PowerManagementConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    pub font_family: String,
    pub keyboard_repeat_delay: i32,
    pub keyboard_repeat_rate: i32,
    pub theme_scheme: ThemeScheme,
    pub gtk_theme: Option<String>,
    pub background_image: String,
    pub background_color: String,
    pub locales: Vec<String>,
    pub use_10bit_color: bool,
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    #[serde(default = "shortcuts::default_shortcut_map")]
    pub keyboard_shortcuts: ShortcutMap,
    #[serde(default)]
    pub virtual_outputs: Vec<VirtualOutputConfig>,
    #[serde(default)]
    pub occlusion_culling: bool,
    #[serde(default)]
    pub login: LoginConfig,
    #[serde(default)]
    pub lock: LockConfig,
    #[serde(default)]
    pub workspaces: WorkspacesConfig,
    #[serde(default)]
    pub exec_once: Vec<RunCommandConfig>,
    #[serde(default)]
    pub xdg_autostart: bool,
    #[serde(default)]
    pub systemd_notify: bool,
    #[serde(skip)]
    #[serde(default)]
    shortcut_bindings: Vec<ShortcutBinding>,
}

/// The live configuration.
///
/// `RwLock<Arc<Config>>` rather than a plain `Config` so a reader never holds
/// the lock while it works: [`Config::with`] clones the `Arc` and releases the
/// lock immediately, which keeps the render path free of any chance of blocking
/// behind a writer, and lets a closure passed to `with` read the config again
/// without deadlocking.
static CONFIG: OnceLock<RwLock<Arc<Config>>> = OnceLock::new();

/// Serialises read-modify-write updates so two concurrent [`Config::update`]
/// calls cannot lose one another's change. Held *around* the update rather than
/// inside the `RwLock`, so the caller's closure may read the config freely.
static CONFIG_WRITE: Mutex<()> = Mutex::new(());

fn config_cell() -> &'static RwLock<Arc<Config>> {
    CONFIG.get_or_init(|| RwLock::new(Arc::new(Config::init())))
}

impl Default for Config {
    fn default() -> Self {
        let mut config = Self {
            // 1.0, not the author's HiDPI 2.0: this is what a fresh install
            // with no /etc/otto/config.toml runs at, and a 2.0 default makes
            // the cursor and every panel twice the size it should be.
            screen_scale: 1.0,
            displays: DisplaysConfig::default(),
            cursor_theme: "Notwaita-Black".to_string(),
            icon_theme: None,
            cursor_size: 24,
            input: InputConfig::default(),
            dock: DockConfig::default(),
            appswitcher: AppSwitcherConfig::default(),
            layer_shell: LayerShellConfig::default(),
            power_management: PowerManagementConfig::default(),
            audio: AudioConfig::default(),
            font_family: "Inter".to_string(),
            keyboard_repeat_delay: 300,
            keyboard_repeat_rate: 30,
            theme_scheme: ThemeScheme::Light,
            gtk_theme: None,
            background_image: "".to_string(),
            background_color: "#1a1a2e".to_string(),
            locales: vec!["en".to_string()],
            use_10bit_color: false,
            accent_color: default_accent_color(),
            keyboard_shortcuts: shortcuts::default_shortcut_map(),
            shortcut_bindings: Vec::new(),
            virtual_outputs: Vec::new(),
            occlusion_culling: false,
            login: LoginConfig::default(),
            lock: LockConfig::default(),
            workspaces: WorkspacesConfig::default(),
            exec_once: Vec::new(),
            xdg_autostart: false,
            systemd_notify: false,
        };
        config.rebuild_shortcut_bindings();
        config
    }
}
pub const WINIT_DISPLAY_ID: &str = "winit";

impl Config {
    /// Run `f` against the current configuration snapshot.
    ///
    /// The snapshot is immutable, so a value read here cannot change underneath
    /// `f`; a concurrent [`Config::update`] is seen by the *next* call.
    pub fn with<R>(f: impl FnOnce(&Config) -> R) -> R {
        let snapshot = Self::current();
        f(&snapshot)
    }

    /// The current configuration snapshot, kept alive for as long as the caller
    /// holds it.
    pub fn current() -> Arc<Config> {
        config_cell()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Replace the live configuration with a mutated copy of the current one.
    ///
    /// Copy-on-write: readers holding an older snapshot keep seeing it, which is
    /// what makes changing configuration on the fly safe from the render path.
    pub fn update(f: impl FnOnce(&mut Config)) -> Arc<Config> {
        let _guard = CONFIG_WRITE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut next = (*Self::current()).clone();
        f(&mut next);
        next.rebuild_shortcut_bindings();
        Self::store(next)
    }

    /// Install `config` as the live configuration, returning the new snapshot.
    fn store(config: Config) -> Arc<Config> {
        let next = Arc::new(config);
        *config_cell()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = next.clone();
        next
    }

    /// Re-read every configuration layer from disk and install the result.
    ///
    /// Returns the previous snapshot alongside the new one so the caller can
    /// work out which settings actually changed.
    /// A layer that fails to parse aborts the reload: a half-edited file must
    /// leave the running configuration alone rather than silently revert keys to
    /// the layer below.
    pub fn reload() -> Result<(Arc<Config>, Arc<Config>), String> {
        let _guard = CONFIG_WRITE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (config, errors) = Self::load_layered_reporting();
        if let Some(error) = errors.into_iter().next() {
            return Err(error);
        }
        let previous = Self::current();
        let next = Self::store(config);
        Ok((previous, next))
    }

    fn init() -> Self {
        let (config, errors) = Self::load_layered_reporting();
        for error in &errors {
            warn!("{error}");
        }

        // Environment variables for Wayland session
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        std::env::set_var("XDG_CURRENT_DESKTOP", "otto");

        tracing::info!("Config initialized: {:#?}", config.theme_scheme);
        config
    }

    /// Merge every configuration layer, lowest priority first, reporting the
    /// layers that could not be parsed instead of quietly dropping them.
    fn load_layered_reporting() -> (Self, Vec<String>) {
        let mut errors: Vec<String> = Vec::new();
        let mut merged =
            toml::Value::try_from(Self::default()).expect("default config is always valid toml");

        // Runs before the layer is read, since it rewrites the file.
        if let Some(user_config) = get_user_config_path() {
            prune_materialized_dock_keys_in_file(&user_config);
        }

        let layers = config_layers();
        let found_any_config = !layers.is_empty();

        for layer in layers {
            let content = match std::fs::read_to_string(&layer) {
                Ok(content) => content,
                // The layer existed when it was listed; a read error now is
                // worth reporting rather than skipping in silence.
                Err(err) => {
                    errors.push(format!("Failed to read {}: {err}", layer.display()));
                    continue;
                }
            };
            match content.parse::<toml::Value>() {
                Ok(value) => {
                    merge_value(&mut merged, value);
                    tracing::info!("Loaded config layer from {}", layer.display());
                }
                Err(err) => errors.push(format!("Failed to parse {}: {err}", layer.display())),
            }
        }

        if !found_any_config {
            warn!(
                "No configuration file found, using default config. \
                 Copy /etc/otto/config.example.toml to \
                 ~/.config/otto/config.toml (or /etc/otto/config.toml) to \
                 customise the dock, displays and input."
            );
        }

        report_where_settings_are_written();

        let mut config: Config = merged.try_into().unwrap_or_else(|err| {
            errors.push(format!("Invalid config overrides: {err}"));
            Self::default()
        });

        config.rebuild_shortcut_bindings();
        (config, errors)
    }

    fn rebuild_shortcut_bindings(&mut self) {
        self.shortcut_bindings = build_bindings(&self.keyboard_shortcuts);
    }

    pub fn shortcut_bindings(&self) -> &[ShortcutBinding] {
        &self.shortcut_bindings
    }

    pub fn resolve_display_profile(
        &self,
        name: &str,
        descriptor: &DisplayDescriptor<'_>,
    ) -> Option<DisplayProfile> {
        self.displays.resolve(name, descriptor)
    }
}

fn merge_value(base: &mut toml::Value, overrides: toml::Value) {
    match (base, overrides) {
        (toml::Value::Table(base_map), toml::Value::Table(override_map)) => {
            for (key, override_value) in override_map {
                match base_map.entry(key) {
                    Entry::Occupied(mut entry) => merge_value(entry.get_mut(), override_value),
                    Entry::Vacant(entry) => {
                        entry.insert(override_value);
                    }
                }
            }
        }
        (base_value, override_value) => {
            *base_value = override_value;
        }
    }
}

/// The local override file, resolved against the compositor's working
/// directory: the dev loop's `cargo run` from a checkout picks up the
/// checkout's file.
const LOCAL_CONFIG_FILE: &str = "otto_config.toml";

/// Resolve a working-directory-relative config file to an absolute path, so
/// that a layer is named the same way wherever it is reported — "the file that
/// overrides your setting" is only useful with a directory attached.
fn in_working_directory(name: &str) -> PathBuf {
    match std::env::current_dir() {
        Ok(dir) => dir.join(name),
        Err(_) => PathBuf::from(name),
    }
}

fn get_system_config_path() -> Option<PathBuf> {
    let path = PathBuf::from("/etc/otto/config.toml");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Where the user's own configuration lives, whether or not it is there yet.
fn user_config_file() -> Option<PathBuf> {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join(".config"))
        })?;

    Some(config_dir.join("otto").join("config.toml"))
}

fn get_user_config_path() -> Option<PathBuf> {
    user_config_file().filter(|path| path.exists())
}

/// The configuration files that are on disk, lowest priority first — the
/// layers [`Config::load_layered_reporting`] merges, in the order it merges
/// them:
///
/// 1. `/etc/otto/config.toml` — system-wide
/// 2. `$XDG_CONFIG_HOME/otto/config.toml` — the user's own
/// 3. `./otto_config.toml` — local override, relative to the *working
///    directory*, so a session started from `$HOME` reads `~/otto_config.toml`
/// 4. `otto_config.<backend>.toml` — backend override, only with `OTTO_BACKEND`
///
/// The last entry wins every key it sets, so it is the only file a write can
/// land in and be seen: [`writable_config_path`] takes the top of this stack.
/// Loading and writing share this list on purpose — a write target that is not
/// the top layer is a setting that applies live and then silently reverts on
/// the next reload.
fn config_layers() -> Vec<PathBuf> {
    let mut layers = Vec::new();

    if let Some(system) = get_system_config_path() {
        layers.push(system);
    }
    if let Some(user) = get_user_config_path() {
        layers.push(user);
    }

    let local = in_working_directory(LOCAL_CONFIG_FILE);
    if local.is_file() {
        layers.push(local);
    }

    if let Ok(backend) = std::env::var("OTTO_BACKEND") {
        if let Some(override_path) = backend_override_candidates(&backend)
            .iter()
            .map(|candidate| in_working_directory(candidate))
            .find(|path| path.is_file())
        {
            layers.push(override_path);
        }
    }

    layers
}

/// The file a setting is persisted to: the highest-priority layer that is
/// actually loaded, so what is written is what takes effect.
///
/// The system config is never written — it belongs to the package, not to the
/// user, and a session usually cannot write it anyway; a user config that does
/// not exist yet is created rather than dropping a stray `otto_config.toml`
/// into whatever directory the session happens to have started from.
pub fn writable_config_path() -> PathBuf {
    config_layers()
        .into_iter()
        .rev()
        .find(|path| Some(path.as_path()) != get_system_config_path().as_deref())
        .or_else(user_config_file)
        .unwrap_or_else(|| in_working_directory(LOCAL_CONFIG_FILE))
}

/// The local override candidates for `backend`, most specific first.
fn backend_override_candidates(backend: &str) -> Vec<String> {
    match backend {
        "winit" => vec!["otto_config.winit.toml".into()],
        "tty-udev" => vec![
            "otto_config.tty-udev.toml".into(),
            "otto_config.udev.toml".into(),
        ],
        "x11" => vec![
            "otto_config.x11.toml".into(),
            "otto_config.udev.toml".into(),
        ],
        other => vec![format!("otto_config.{other}.toml")],
    }
}

/// Say which file a changed setting will be written to, when that is not the
/// user's own config.
///
/// A setting is persisted into the top configuration layer, because that is the
/// only file whose keys are the effective ones. When a local `otto_config.toml`
/// sits above the user config — a checkout the session was started from, or a
/// leftover in the home directory — that file quietly becomes the one the
/// settings app edits, and `~/.config/otto` stops having any effect. Neither is
/// wrong, but it is worth being able to read off the log rather than work out
/// from a setting that will not stick.
fn report_where_settings_are_written() {
    let writable = writable_config_path();
    let Some(user_config) = get_user_config_path() else {
        return;
    };
    if writable == user_config {
        return;
    }

    warn!(
        "Settings are written to {}: it overrides {}, which is only read. \
         Remove it, or the keys it repeats, to configure Otto from {}.",
        writable.display(),
        user_config.display(),
        user_config.display()
    );
}

/// Persist the dock's bookmark list.
///
/// Bookmarks are the one piece of `[dock]` the dock still writes itself: they
/// are a list the user builds by dragging and by context menu, not a scalar
/// setting, so they have no schema identifier. Every scalar dock setting is
/// written by [`crate::settings::set`], one key at a time.
///
/// Only `dock.bookmarks` is touched; every other key, section and comment in
/// the file is left alone.
pub fn save_dock_bookmarks(bookmarks: &[DockBookmark]) {
    let path = writable_config_path();
    let mut doc = match file::load_document(&path) {
        Ok(doc) => doc,
        Err(err) => {
            warn!("Not saving dock bookmarks: {err}");
            return;
        }
    };

    let serialized = toml::Value::try_from(SerializedBookmarks(bookmarks))
        .ok()
        .and_then(|value| file::to_edit_value(&value));
    let Some(value) = serialized else {
        warn!("Failed to serialize dock bookmarks");
        return;
    };

    if let Err(err) = file::set_key(&mut doc, "dock.bookmarks", value) {
        warn!("Failed to save dock bookmarks: {err}");
        return;
    }
    if let Err(err) = file::store_document(&path, &doc) {
        warn!("Failed to save dock bookmarks: {err}");
    }
}

/// Newtype so the bookmark list can be serialized on its own with the same
/// compact representation `DockConfig` uses.
struct SerializedBookmarks<'a>(&'a [DockBookmark]);

impl Serialize for SerializedBookmarks<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_dock_bookmarks(self.0, serializer)
    }
}

/// Write `value` to the dotted key `path` in the writable config file, touching
/// nothing else. This is the one way a setting is persisted.
pub fn persist_key(path: &str, value: &toml::Value) -> Result<(), String> {
    let file_path = writable_config_path();
    let mut doc = file::load_document(&file_path)?;
    let value = file::to_edit_value(value).ok_or_else(|| format!("cannot store `{path}`"))?;
    file::set_key(&mut doc, path, value)?;
    file::store_document(&file_path, &doc)
}

/// Add or replace one `[[virtual_outputs]]` entry in the writable config file,
/// matched by name. Everything else in the array — and in the file — is left
/// alone, the same promise [`persist_key`] makes for scalars.
///
/// Virtual outputs are a list, not a dotted key, so they cannot go through the
/// settings schema: there is no fixed identifier for "the third virtual
/// output". They get their own writer instead of bending the schema around a
/// collection.
pub fn persist_virtual_output(config: &VirtualOutputConfig) -> Result<(), String> {
    let file_path = writable_config_path();
    let mut doc = file::load_document(&file_path)?;
    file::upsert_virtual_output(&mut doc, config)?;
    file::store_document(&file_path, &doc)
}

/// Drop the `[[virtual_outputs]]` entry called `name`. Removing one that is
/// not there succeeds and changes nothing.
pub fn forget_virtual_output(name: &str) -> Result<(), String> {
    let file_path = writable_config_path();
    let mut doc = file::load_document(&file_path)?;
    if !file::remove_virtual_output(&mut doc, name) {
        return Ok(());
    }
    file::store_document(&file_path, &doc)
}

/// Remove the dotted key `path` from the writable config file, so its value
/// falls back to the lower configuration layers. Removing a key that is not
/// there succeeds and changes nothing.
pub fn forget_key(path: &str) -> Result<(), String> {
    let file_path = writable_config_path();
    let mut doc = file::load_document(&file_path)?;
    if !file::remove_key(&mut doc, path) {
        return Ok(());
    }
    file::store_document(&file_path, &doc)
}

/// The value the writable config file stores for the dotted key `path`, if any.
///
/// This is the *persisted* value, which is not the same as the effective one: a
/// live interaction such as a dock resize changes the running configuration
/// long before the drag settles and the key is written.
pub fn stored_key(path: &str) -> Option<toml::Value> {
    let file_path = writable_config_path();
    let doc = file::load_document(&file_path).ok()?;
    file::to_toml_value(file::get_key(&doc, path)?)
}

/// The `[dock]` keys the builds this migration cleans up after materialized:
/// back then none of them were dock-owned. (`size` is written by the handle
/// drag now, but a copy of the *default* still carries no intent, so it is
/// still worth pruning.)
const HAND_EDITED_DOCK_KEYS: [&str; 6] = [
    "size",
    "genie_scale",
    "genie_span",
    "colorize_icons",
    "colorize_color",
    "colorize_intensity",
];

/// Clean up after the builds that rewrote the whole `[dock]` table: those left
/// a copy of every inherited value in the user config, where it shadows
/// `/etc/otto/config.toml` for good, so editing the system config looks like it
/// does nothing.
///
/// Rewrites `path` in place if anything was pruned. Nothing else in the file is
/// touched, but the rewrite goes through `toml` and so drops comments — a
/// trade-off no longer made anywhere else (see [`file`]), and it happens at most
/// once per affected install.
fn prune_materialized_dock_keys_in_file(path: &std::path::Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut doc) = content.parse::<toml::Value>() else {
        return; // a parse error is reported when the file is loaded
    };

    let pruned = prune_materialized_dock_keys(&mut doc);
    if pruned.is_empty() {
        return;
    }

    match toml::to_string_pretty(&doc) {
        Ok(serialized) => match std::fs::write(path, serialized) {
            Ok(()) => tracing::info!(
                "Dropped [dock] {} from {}: written by an older build, not hand-edited",
                pruned.join(", "),
                path.display()
            ),
            Err(err) => warn!("Failed to rewrite {}: {err}", path.display()),
        },
        Err(err) => warn!("Failed to serialize {}: {err}", path.display()),
    }
}

/// Remove the hand-edited-only `[dock]` keys of `doc` that carry no intent, and
/// report which ones went.
///
/// Only a machine-written table is touched: an older build wrote *all* of
/// [`HAND_EDITED_DOCK_KEYS`] at once, so anything less than the full set is
/// somebody's hand-written config and is left alone. Within such a table, a key
/// holding a value the user could have chosen stays; one holding the built-in
/// default — or the zero those builds wrote when the key was absent, which no
/// documented setting uses — can only have been copied in, so it goes and lets
/// the lower-priority configs be seen again.
fn prune_materialized_dock_keys(doc: &mut toml::Value) -> Vec<&'static str> {
    let Some(dock) = doc.get_mut("dock").and_then(toml::Value::as_table_mut) else {
        return Vec::new();
    };
    if !HAND_EDITED_DOCK_KEYS
        .iter()
        .all(|key| dock.contains_key(*key))
    {
        return Vec::new();
    }

    let defaults =
        toml::Value::try_from(DockConfig::default()).expect("dock defaults are always valid toml");

    let mut pruned = Vec::new();
    for key in HAND_EDITED_DOCK_KEYS {
        let Some(value) = dock.get(key) else { continue };
        let is_default = defaults
            .get(key)
            .is_some_and(|default| same_toml_scalar(value, default));
        if is_default || is_zeroed(value) {
            dock.remove(key);
            pruned.push(key);
        }
    }
    pruned
}

/// Whether `value` is what the old derived `DockConfig::default()` produced for a
/// missing key: a zero size or intensity and an empty color, none of which are a
/// usable setting.
fn is_zeroed(value: &toml::Value) -> bool {
    match value {
        toml::Value::String(text) => text.is_empty(),
        toml::Value::Boolean(flag) => !flag,
        other => as_number(other) == Some(0.0),
    }
}

/// TOML equality that treats `size = 1` and `size = 1.0` as the same value.
fn same_toml_scalar(a: &toml::Value, b: &toml::Value) -> bool {
    match (as_number(a), as_number(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

fn as_number(value: &toml::Value) -> Option<f64> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|i| i as f64))
}

/// Which screen edge the dock is docked to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockPosition {
    /// Horizontal dock along the bottom edge (the default).
    #[default]
    Bottom,
    /// Vertical dock along the left edge.
    #[serde(alias = "left-side")]
    Left,
    /// Vertical dock along the right edge.
    #[serde(alias = "right-side")]
    Right,
}

impl DockPosition {
    /// Whether the dock runs along a screen side, i.e. stacks its icons
    /// vertically. Most of the layout code only needs this distinction.
    pub fn is_vertical(self) -> bool {
        matches!(self, DockPosition::Left | DockPosition::Right)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockConfig {
    #[serde(default = "default_dock_size")]
    pub size: f64,
    /// Screen edge the dock lives on: `"bottom"` (default), `"left"` or `"right"`.
    #[serde(default)]
    pub position: DockPosition,
    #[serde(default = "default_genie_scale")]
    pub genie_scale: f64,
    #[serde(default = "default_genie_span")]
    pub genie_span: f64,
    #[serde(default)]
    pub colorize_icons: bool,
    #[serde(default = "default_dock_colorize_color")]
    pub colorize_color: String,
    #[serde(default = "default_dock_colorize_intensity")]
    pub colorize_intensity: f64,
    #[serde(default)]
    pub autohide: bool,
    #[serde(default = "default_magnification")]
    pub magnification: bool,
    #[serde(
        default,
        serialize_with = "serialize_dock_bookmarks",
        deserialize_with = "deserialize_dock_bookmarks"
    )]
    pub bookmarks: Vec<DockBookmark>,
}

/// Must stay in sync with the `#[serde(default = ...)]` functions above:
/// `Config::init` seeds the merge from `Config::default()` serialized to TOML,
/// so a derived (all-zero) `Default` would shadow those functions and silently
/// give a `size` of 0 (clamped to 0.5) whenever the key is absent.
impl Default for DockConfig {
    fn default() -> Self {
        Self {
            size: default_dock_size(),
            position: DockPosition::default(),
            genie_scale: default_genie_scale(),
            genie_span: default_genie_span(),
            colorize_icons: false,
            colorize_color: default_dock_colorize_color(),
            colorize_intensity: default_dock_colorize_intensity(),
            autohide: false,
            magnification: default_magnification(),
            bookmarks: Vec::new(),
        }
    }
}

/// App switcher (cmd-tab panel) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSwitcherConfig {
    /// Show the switcher on the output the pointer is on (default: true).
    /// When false it always appears on the primary output.
    #[serde(default = "default_appswitcher_follow_cursor")]
    pub follow_cursor: bool,
}

impl Default for AppSwitcherConfig {
    fn default() -> Self {
        Self {
            follow_cursor: default_appswitcher_follow_cursor(),
        }
    }
}

fn default_appswitcher_follow_cursor() -> bool {
    true
}

/// Settings that only apply when Otto runs as a greeter host (`otto --login`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginConfig {
    /// The greeter client to launch. It is the only client Otto starts in
    /// login mode, and its first toplevel is forced fullscreen on the primary
    /// output.
    pub greeter_command: String,
    /// Arguments passed to `greeter_command`.
    pub greeter_args: Vec<String>,
}

impl Default for LoginConfig {
    fn default() -> Self {
        Self {
            greeter_command: "otto-greeter".to_string(),
            greeter_args: Vec::new(),
        }
    }
}

/// Workspace settings. Only holds the names the user typed in the workspace
/// selector so far — everything else about a workspace is runtime state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspacesConfig {
    /// Custom workspace names, keyed by `"<output name>:<position>"` — see
    /// [`workspace_name_key`]. Workspaces are per output, and positions shift
    /// when one is removed, so this is a best-effort restore, not an identity.
    pub names: BTreeMap<String, String>,
}

/// Key under which the name of workspace `position` on `output` is stored.
pub fn workspace_name_key(output: &str, position: usize) -> String {
    format!("{output}:{position}")
}

/// Persist custom workspace names into the `[workspaces]` section of the
/// writable config file, replacing the whole `names` table (the compositor
/// holds the authoritative set) and leaving every other section alone.
pub fn save_workspace_names(names: &BTreeMap<String, String>) {
    let path = writable_config_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml::Value = raw
        .parse()
        .unwrap_or(toml::Value::Table(Default::default()));

    if let Some(table) = doc.as_table_mut() {
        let workspaces = table
            .entry("workspaces".to_string())
            .or_insert_with(|| toml::Value::Table(Default::default()));
        if let Some(workspaces) = workspaces.as_table_mut() {
            if let Ok(names) = toml::Value::try_from(names) {
                workspaces.insert("names".to_string(), names);
            }
        }
    }

    match toml::to_string_pretty(&doc) {
        Ok(serialized) => {
            if let Err(err) = std::fs::write(&path, serialized) {
                warn!(
                    "Failed to save workspace names to {}: {err}",
                    path.display()
                );
            }
        }
        Err(err) => warn!("Failed to serialize workspace names: {err}"),
    }
}

/// Settings for locking the running session (`ext-session-lock-v1`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LockConfig {
    /// The locker to launch for the `lock` action. It authenticates the user
    /// itself; Otto only hides the session behind it.
    pub locker_command: String,
    /// Arguments passed to `locker_command`.
    pub locker_args: Vec<String>,
    /// Lock the session after this many seconds with no input from the user.
    /// `0` (the default) never locks on its own.
    ///
    /// A client holding an `idle-inhibit-unstable-v1` inhibitor — a video
    /// player, a presentation — holds the lock off while it plays.
    pub auto_lock_timeout: u64,
}

impl Default for LockConfig {
    fn default() -> Self {
        Self {
            locker_command: "otto-lock".to_string(),
            locker_args: Vec::new(),
            auto_lock_timeout: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerShellConfig {
    /// Maximum exclusive zone allowed for top edge in logical points (0 = unlimited)
    #[serde(default = "default_max_top")]
    pub max_top: i32,
    /// Maximum exclusive zone allowed for bottom edge in logical points (0 = unlimited)
    #[serde(default = "default_max_bottom")]
    pub max_bottom: i32,
    /// Maximum exclusive zone allowed for left edge in logical points (0 = unlimited)
    #[serde(default = "default_max_left")]
    pub max_left: i32,
    /// Maximum exclusive zone allowed for right edge in logical points (0 = unlimited)
    #[serde(default = "default_max_right")]
    pub max_right: i32,
}

impl Default for LayerShellConfig {
    fn default() -> Self {
        Self {
            max_top: default_max_top(),
            max_bottom: default_max_bottom(),
            max_left: default_max_left(),
            max_right: default_max_right(),
        }
    }
}

fn default_max_top() -> i32 {
    100 // Max 100 logical points for top panels
}

fn default_max_bottom() -> i32 {
    100 // Max 100 logical points for bottom panels/docks
}

fn default_max_left() -> i32 {
    50 // Max 50 logical points for side panels
}

fn default_max_right() -> i32 {
    50 // Max 50 logical points for side panels
}

fn default_magnification() -> bool {
    true
}

fn default_dock_size() -> f64 {
    1.0
}

fn default_genie_scale() -> f64 {
    0.5
}

fn default_genie_span() -> f64 {
    10.0
}

fn default_dock_colorize_color() -> String {
    "#ffffff".to_string()
}

fn default_dock_colorize_intensity() -> f64 {
    1.0
}

fn default_accent_color() -> String {
    "blue".to_string()
}

/// Power management configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerManagementConfig {
    /// Enable Otto's lid switch handling (default: true)
    /// When enabled, Otto manages display state on lid close/open
    /// When disabled, all lid handling is delegated to systemd-logind
    #[serde(default = "default_manage_lid_switch")]
    pub manage_lid_switch: bool,

    /// What to do when laptop lid closes (default: "auto")
    /// Options:
    ///   "auto" - Normal laptop: disable screen, then suspend via logind —
    ///     unless an external monitor is connected or a remote client (RDP
    ///     bridge / screenshare) is actively consuming frames
    ///   "lock" - Like "auto", but lock the session first, the way
    ///     `on_power_button = "lock"` does, so the machine wakes to the locker
    ///   "disable_internal_screen" - Always disable screen but stay running,
    ///     never suspend (for display managers/kiosks)
    #[serde(default = "default_on_lid_close")]
    pub on_lid_close: LidCloseAction,

    /// What to do when the hardware power button is pressed (default: "lock").
    /// Set to "ignore" to leave the key to systemd-logind / the focused client.
    #[serde(default = "default_on_power_button")]
    pub on_power_button: PowerButtonAction,
}

/// Action to take when laptop lid is closed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum LidCloseAction {
    /// Normal laptop behavior: disable screen, then suspend — unless an
    /// external monitor is connected or a remote session is active
    #[default]
    Auto,
    /// [`LidCloseAction::Auto`] plus a session lock, so the machine wakes to
    /// the locker. Skipped in the same cases the suspend is: a clamshell or
    /// remote session is still in use, and stays unlocked
    Lock,
    /// Always disable screen but keep running, never suspend (for display
    /// managers/kiosks)
    DisableInternalScreen,
}

/// Action to take when the hardware power button is pressed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PowerButtonAction {
    /// Leave the key alone: systemd-logind's `HandlePowerKey` decides, and the
    /// keysym is delivered to the focused client like any other key
    Ignore,
    /// Lock the session by launching the configured locker
    #[default]
    Lock,
    /// Suspend the system (via logind)
    Suspend,
    /// Power the machine off (via logind)
    Shutdown,
}

impl Default for PowerManagementConfig {
    fn default() -> Self {
        Self {
            manage_lid_switch: default_manage_lid_switch(),
            on_lid_close: default_on_lid_close(),
            on_power_button: default_on_power_button(),
        }
    }
}

fn default_on_power_button() -> PowerButtonAction {
    PowerButtonAction::Lock
}

fn default_manage_lid_switch() -> bool {
    true
}

fn default_on_lid_close() -> LidCloseAction {
    LidCloseAction::Auto
}

/// Audio configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Enable sound feedback for UI events (default: true)
    #[serde(default = "default_sound_enabled")]
    pub sound_enabled: bool,

    /// XDG Sound Theme name (default: None for auto-detection)
    /// Examples: "freedesktop", "Pop", "ocean"
    /// When None, Otto will auto-detect the system sound theme
    #[serde(default)]
    pub sound_theme: Option<String>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sound_enabled: default_sound_enabled(),
            sound_theme: None,
        }
    }
}

fn default_sound_enabled() -> bool {
    true
}

/// Input device configuration
///
/// Note: These settings map directly to libinput configuration options.
/// Names reflect libinput's terminology for compatibility and documentation purposes.
///
/// TODO: Consider providing more user-friendly option names/descriptions while
/// maintaining backward compatibility with libinput terminology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    #[serde(default = "default_tap_enabled")]
    pub tap_enabled: bool,
    #[serde(default = "default_tap_drag_enabled")]
    pub tap_drag_enabled: bool,
    #[serde(default = "default_tap_drag_lock_enabled")]
    pub tap_drag_lock_enabled: bool,
    #[serde(default = "default_touchpad_click_method")]
    pub touchpad_click_method: TouchpadClickMethod,
    #[serde(default = "default_touchpad_dwt_enabled")]
    pub touchpad_dwt_enabled: bool,
    #[serde(default = "default_touchpad_natural_scroll_enabled")]
    pub touchpad_natural_scroll_enabled: bool,
    #[serde(default = "default_touchpad_left_handed")]
    pub touchpad_left_handed: bool,
    #[serde(default = "default_touchpad_middle_emulation_enabled")]
    pub touchpad_middle_emulation_enabled: bool,
    /// Scroll speed multiplier applied in software. Default is 1.0 (no change).
    /// Values > 1.0 increase scroll speed; values between 0.0 and 1.0 decrease it.
    /// Negative values are clamped to 0.0 to prevent inverted scrolling.
    #[serde(
        default = "default_scroll_speed",
        deserialize_with = "deserialize_scroll_speed"
    )]
    pub scroll_speed: f64,
    /// Pointer acceleration speed. Range: -1.0 (slowest) to 1.0 (fastest), default 0.0.
    /// Applies to all pointer devices (mice and touchpads).
    #[serde(default = "default_pointer_accel_speed")]
    pub pointer_accel_speed: f64,
    /// Pointer acceleration profile. "flat" disables acceleration (raw speed),
    /// "adaptive" applies libinput's default adaptive acceleration curve.
    #[serde(default = "default_pointer_accel_profile")]
    pub pointer_accel_profile: PointerAccelProfile,
    #[serde(default)]
    pub xkb_layout: Option<String>,
    #[serde(default)]
    pub xkb_variant: Option<String>,
    #[serde(default)]
    pub xkb_options: Vec<String>,
    /// Whether Otto's shortcuts follow the Cmd key alone.
    ///
    /// Only meaningful when the layout folds Cmd into Control — see
    /// `altwin:ctrl_win` in `xkb_options`. Then Cmd and the real Ctrl key
    /// produce the same event, and a `Ctrl+W` binding fires from both, closing
    /// the window when the user meant `^W` in a terminal. With this on, Otto
    /// matches its shortcuts on Cmd and leaves the real Ctrl key to the
    /// focused application.
    ///
    /// Unset means "whenever `altwin:ctrl_win` is in `xkb_options`", which is
    /// the only layout it makes sense for. Set it explicitly to force it either
    /// way.
    #[serde(default)]
    pub mac_style_modifiers: Option<bool>,
}

/// Touchpad click method configuration
///
/// Maps to libinput's LIBINPUT_CONFIG_CLICK_METHOD_* enum values.
/// See: https://wayland.freedesktop.org/libinput/doc/latest/clickpad_softbuttons.html
///
/// TODO: Consider more intuitive naming like "finger_count" vs "button_areas"
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TouchpadClickMethod {
    /// Click behavior depends on number of fingers (1=left, 2=right, 3=middle)
    /// Corresponds to LIBINPUT_CONFIG_CLICK_METHOD_CLICKFINGER
    Clickfinger,
    /// Traditional button areas (top-right corner = right click)
    /// Corresponds to LIBINPUT_CONFIG_CLICK_METHOD_BUTTON_AREAS
    ButtonAreas,
}

/// Pointer acceleration profile.
///
/// Maps to libinput's LIBINPUT_CONFIG_ACCEL_PROFILE_* enum values.
/// See: https://wayland.freedesktop.org/libinput/doc/latest/pointer-acceleration.html
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PointerAccelProfile {
    /// No acceleration; pointer speed is directly proportional to physical movement.
    /// Corresponds to LIBINPUT_CONFIG_ACCEL_PROFILE_FLAT
    Flat,
    /// libinput's default adaptive acceleration curve.
    /// Corresponds to LIBINPUT_CONFIG_ACCEL_PROFILE_ADAPTIVE
    Adaptive,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            tap_enabled: default_tap_enabled(),
            tap_drag_enabled: default_tap_drag_enabled(),
            tap_drag_lock_enabled: default_tap_drag_lock_enabled(),
            touchpad_click_method: default_touchpad_click_method(),
            touchpad_dwt_enabled: default_touchpad_dwt_enabled(),
            touchpad_natural_scroll_enabled: default_touchpad_natural_scroll_enabled(),
            touchpad_left_handed: default_touchpad_left_handed(),
            touchpad_middle_emulation_enabled: default_touchpad_middle_emulation_enabled(),
            scroll_speed: default_scroll_speed(),
            pointer_accel_speed: default_pointer_accel_speed(),
            pointer_accel_profile: default_pointer_accel_profile(),
            xkb_layout: None,
            xkb_variant: None,
            xkb_options: Vec::new(),
            mac_style_modifiers: None,
        }
    }
}

fn default_tap_enabled() -> bool {
    true
}

fn default_tap_drag_enabled() -> bool {
    true
}

fn default_tap_drag_lock_enabled() -> bool {
    false
}

fn default_touchpad_click_method() -> TouchpadClickMethod {
    TouchpadClickMethod::Clickfinger
}

fn default_touchpad_dwt_enabled() -> bool {
    true
}

fn default_touchpad_natural_scroll_enabled() -> bool {
    true
}

fn default_touchpad_left_handed() -> bool {
    false
}

fn default_touchpad_middle_emulation_enabled() -> bool {
    false
}

fn default_scroll_speed() -> f64 {
    1.0
}

fn deserialize_scroll_speed<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    Ok(value.max(0.0))
}

fn default_pointer_accel_speed() -> f64 {
    0.0
}

fn default_pointer_accel_profile() -> PointerAccelProfile {
    PointerAccelProfile::Adaptive
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockBookmark {
    pub desktop_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub exec_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum DockBookmarkToml {
    Compact(String),
    Detailed {
        desktop_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exec_args: Vec<String>,
    },
}

fn serialize_dock_bookmarks<S>(bookmarks: &[DockBookmark], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let encoded: Vec<DockBookmarkToml> = bookmarks
        .iter()
        .map(|bookmark| {
            if bookmark.label.is_none() && bookmark.exec_args.is_empty() {
                DockBookmarkToml::Compact(bookmark.desktop_id.clone())
            } else {
                DockBookmarkToml::Detailed {
                    desktop_id: bookmark.desktop_id.clone(),
                    label: bookmark.label.clone(),
                    exec_args: bookmark.exec_args.clone(),
                }
            }
        })
        .collect();

    encoded.serialize(serializer)
}

fn deserialize_dock_bookmarks<'de, D>(deserializer: D) -> Result<Vec<DockBookmark>, D::Error>
where
    D: Deserializer<'de>,
{
    let encoded = Vec::<DockBookmarkToml>::deserialize(deserializer)?;
    Ok(encoded
        .into_iter()
        .map(|bookmark| match bookmark {
            DockBookmarkToml::Compact(desktop_id) => DockBookmark {
                desktop_id,
                label: None,
                exec_args: Vec::new(),
            },
            DockBookmarkToml::Detailed {
                desktop_id,
                label,
                exec_args,
            } => DockBookmark {
                desktop_id,
                label,
                exec_args,
            },
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplaysConfig {
    #[serde(default)]
    pub named: BTreeMap<String, DisplayProfile>,
    #[serde(default)]
    pub generic: Vec<DisplayProfileMatch>,
}

impl DisplaysConfig {
    pub fn resolve(
        &self,
        name: &str,
        descriptor: &DisplayDescriptor<'_>,
    ) -> Option<DisplayProfile> {
        if let Some(profile) = self.named.get(name) {
            return Some(profile.clone());
        }

        self.generic
            .iter()
            .find(|entry| entry.matcher.matches(name, descriptor))
            .map(|entry| entry.profile.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayProfile {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub resolution: Option<DisplayResolution>,
    #[serde(default)]
    pub refresh_hz: Option<f64>,
    #[serde(default)]
    pub position: Option<DisplayPosition>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisplayResolution {
    pub width: u32,
    pub height: u32,
}

impl DisplayResolution {
    #[allow(dead_code)]
    pub fn as_f64(self) -> (f64, f64) {
        (self.width as f64, self.height as f64)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DisplayPosition {
    pub x: i32,
    pub y: i32,
}

/// Configuration for a virtual (headless) output streamed via PipeWire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualOutputConfig {
    /// Unique name for the virtual output (e.g. "virtual-1").
    pub name: String,
    /// Output resolution.
    pub resolution: DisplayResolution,
    /// Target refresh rate in Hz.
    #[serde(default = "default_virtual_refresh_hz")]
    pub refresh_hz: f64,
    /// Output position in the compositor layout.
    #[serde(default)]
    pub position: Option<DisplayPosition>,
    /// When true the pointer can enter this output and windows can be
    /// placed/focused on it, like a physical screen. Default false:
    /// headless outputs are invisible, so reaching them loses content.
    #[serde(default)]
    pub interactive: bool,
    /// When true this output becomes the primary one, so the dock, app
    /// switcher and expose render on it. A headless-only setup (remote
    /// desktop served from a virtual output) needs this: otherwise the
    /// chrome stays on a physical screen nobody is looking at.
    #[serde(default)]
    pub primary: bool,
}

fn default_virtual_refresh_hz() -> f64 {
    60.0
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayProfileMatch {
    #[serde(default, rename = "match")]
    pub matcher: DisplayMatcher,
    #[serde(flatten)]
    pub profile: DisplayProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayMatcher {
    #[serde(default)]
    pub connector: Option<String>,
    #[serde(default)]
    pub connector_prefix: Option<String>,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub kind: Option<DisplayKind>,
}

impl DisplayMatcher {
    fn matches(&self, connector: &str, descriptor: &DisplayDescriptor<'_>) -> bool {
        if let Some(expected) = &self.connector {
            if expected != connector && descriptor.connector != expected {
                return false;
            }
        }

        if let Some(prefix) = &self.connector_prefix {
            let matches_actual = connector.starts_with(prefix);
            let matches_descriptor = descriptor.connector.starts_with(prefix);
            if !matches_actual && !matches_descriptor {
                return false;
            }
        }

        if let Some(expected_vendor) = &self.vendor {
            match descriptor.vendor {
                Some(vendor) if equals_ignore_case(vendor, expected_vendor) => {}
                _ => return false,
            }
        }

        if let Some(expected_model) = &self.model {
            match descriptor.model {
                Some(model) if equals_ignore_case(model, expected_model) => {}
                _ => return false,
            }
        }

        if let Some(expected_kind) = self.kind {
            if descriptor.kind.unwrap_or(DisplayKind::Unknown) != expected_kind {
                return false;
            }
        }

        true
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DisplayKind {
    Internal,
    External,
    Virtual,
    #[default]
    Unknown,
}

#[derive(Debug, Clone)]
pub struct DisplayDescriptor<'a> {
    pub connector: &'a str,
    pub vendor: Option<&'a str>,
    pub model: Option<&'a str>,
    pub kind: Option<DisplayKind>,
}

impl<'a> DisplayDescriptor<'a> {
    #[allow(dead_code)]
    pub fn new(connector: &'a str) -> Self {
        Self {
            connector,
            vendor: None,
            model: None,
            kind: None,
        }
    }
}

fn equals_ignore_case(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    use std::fs;

    #[test]
    fn theme_scheme_defaults_to_light() {
        let config = Config::default();
        assert!(matches!(config.theme_scheme, ThemeScheme::Light));
    }

    #[test]
    fn theme_scheme_overrides_to_dark_in_toml() {
        let overrides = r#"
            theme_scheme = "Dark"
        "#;

        let config: Config = toml::from_str(overrides).expect("Config should deserialize");
        assert!(matches!(config.theme_scheme, ThemeScheme::Dark));
    }

    /// A temporary configuration environment: an empty `XDG_CONFIG_HOME`, no
    /// backend override, and a working directory of its own, so a test can
    /// place a layer at each priority without seeing the developer's real
    /// files — the local override in particular is resolved against the
    /// working directory, which is the checkout when tests run.
    struct ConfigEnv {
        /// Held only to keep the directory alive: dropping it deletes the
        /// tree every other field points into.
        #[allow(dead_code)]
        dir: tempfile::TempDir,
        /// The temporary directory as the process sees it once it is the
        /// working directory, which is what the layer paths are built from.
        path: PathBuf,
        old_xdg: Option<String>,
        old_home: Option<String>,
        old_backend: Option<String>,
        old_cwd: PathBuf,
    }

    impl ConfigEnv {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let old_cwd = env::current_dir().unwrap();
            let old_xdg = env::var("XDG_CONFIG_HOME").ok();
            let old_home = env::var("HOME").ok();
            let old_backend = env::var("OTTO_BACKEND").ok();

            env::set_var("XDG_CONFIG_HOME", dir.path());
            env::remove_var("OTTO_BACKEND");
            env::set_current_dir(dir.path()).unwrap();
            let path = env::current_dir().unwrap();

            ConfigEnv {
                dir,
                path,
                old_xdg,
                old_home,
                old_backend,
                old_cwd,
            }
        }

        /// Write the user layer and return its path.
        fn user_config(&self, contents: &str) -> PathBuf {
            let path = self.path.join("otto").join("config.toml");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }

        /// Write the local override — the layer above the user's — and return
        /// its path. It lives in the working directory, which is this one.
        fn local_config(&self, contents: &str) -> PathBuf {
            let path = self.path.join(LOCAL_CONFIG_FILE);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for ConfigEnv {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.old_cwd);
            match self.old_xdg.take() {
                Some(old) => env::set_var("XDG_CONFIG_HOME", old),
                None => env::remove_var("XDG_CONFIG_HOME"),
            }
            match self.old_home.take() {
                Some(old) => env::set_var("HOME", old),
                None => env::remove_var("HOME"),
            }
            match self.old_backend.take() {
                Some(old) => env::set_var("OTTO_BACKEND", old),
                None => env::remove_var("OTTO_BACKEND"),
            }
        }
    }

    /// `cargo run` from a checkout, or a session started from a directory with
    /// a leftover `otto_config.toml`: that file is merged over the user's own,
    /// so it is where a setting has to be written to take effect.
    #[test]
    #[serial]
    fn the_writable_config_is_the_layer_that_wins() {
        let env = ConfigEnv::new();
        let user = env.user_config("[dock]\nsize = 0.9\n");
        assert_eq!(writable_config_path(), user);

        let local = env.local_config("[dock]\nsize = 1.0\n");
        assert_eq!(writable_config_path(), local);
    }

    /// With nothing on disk, a setting creates the user's config rather than
    /// dropping a stray `otto_config.toml` into whatever directory the session
    /// happened to start from — which is how such a file gets there.
    #[test]
    #[serial]
    fn an_absent_config_is_written_where_the_user_config_belongs() {
        let env = ConfigEnv::new();
        assert_eq!(
            writable_config_path(),
            env.path.join("otto").join("config.toml")
        );
    }

    /// A backend override is the top layer, so it is where a setting goes —
    /// and the user config below it is only read.
    #[test]
    #[serial]
    fn a_backend_override_takes_the_writes() {
        let env = ConfigEnv::new();
        env.user_config("[dock]\nsize = 0.9\n");
        let backend_override = env.path.join("otto_config.winit.toml");
        fs::write(&backend_override, "[dock]\nsize = 1.4\n").unwrap();

        env::set_var("OTTO_BACKEND", "winit");
        assert_eq!(writable_config_path(), backend_override);
        env::remove_var("OTTO_BACKEND");
    }

    #[test]
    #[serial]
    fn test_get_user_config_path_with_xdg_config_home() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Set XDG_CONFIG_HOME temporarily
        let old_xdg = env::var("XDG_CONFIG_HOME").ok();
        env::set_var("XDG_CONFIG_HOME", temp_dir.path());

        // Create the config file
        let config_dir = temp_dir.path().join("otto");
        fs::create_dir_all(&config_dir).unwrap();
        let config_file = config_dir.join("config.toml");
        fs::write(&config_file, "# test config").unwrap();

        let path = get_user_config_path();
        assert!(path.is_some());
        assert_eq!(path.unwrap(), config_file);

        // Cleanup
        if let Some(old) = old_xdg {
            env::set_var("XDG_CONFIG_HOME", old);
        } else {
            env::remove_var("XDG_CONFIG_HOME");
        }
        // temp_dir automatically cleaned up when dropped
    }

    #[test]
    #[serial]
    fn test_get_user_config_path_without_file() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Set XDG_CONFIG_HOME to a dir without config
        let old_xdg = env::var("XDG_CONFIG_HOME").ok();
        env::set_var("XDG_CONFIG_HOME", temp_dir.path());

        let path = get_user_config_path();
        assert!(path.is_none());

        // Cleanup
        if let Some(old) = old_xdg {
            env::set_var("XDG_CONFIG_HOME", old);
        } else {
            env::remove_var("XDG_CONFIG_HOME");
        }
        // temp_dir automatically cleaned up when dropped
    }

    #[test]
    fn test_get_system_config_path() {
        // System config path is fixed
        let path = get_system_config_path();

        // Only returns Some if the file exists
        if let Some(p) = path {
            assert_eq!(p, PathBuf::from("/etc/otto/config.toml"));
        }
    }

    #[test]
    fn test_config_merge_priority() {
        // Test that config values merge correctly with priority
        let mut base =
            toml::Value::try_from(Config::default()).expect("default config is valid toml");

        // Override with custom values
        let override_toml = r#"
            screen_scale = 3.0
            font_family = "Custom Font"
        "#;
        let override_value: toml::Value = override_toml.parse().unwrap();

        merge_value(&mut base, override_value);

        let config: Config = base.try_into().unwrap();
        assert_eq!(config.screen_scale, 3.0);
        assert_eq!(config.font_family, "Custom Font");
    }

    #[test]
    fn test_config_partial_override() {
        // Test that partial overrides work correctly
        let mut base =
            toml::Value::try_from(Config::default()).expect("default config is valid toml");

        // Override only screen_scale, leave other values
        let override_toml = r#"
            screen_scale = 1.5
        "#;
        let override_value: toml::Value = override_toml.parse().unwrap();

        merge_value(&mut base, override_value);

        let config: Config = base.try_into().unwrap();
        assert_eq!(config.screen_scale, 1.5);
        // Other defaults should remain
        assert_eq!(config.cursor_theme, "Notwaita-Black");
    }

    #[test]
    fn test_dock_bookmarks_compact_deserialize() {
        let raw = r#"
            [dock]
            bookmarks = ["firefox.desktop", "kitty.desktop"]
        "#;
        let config: Config =
            toml::from_str(raw).expect("compact dock bookmarks should deserialize");

        assert_eq!(config.dock.bookmarks.len(), 2);
        assert_eq!(config.dock.bookmarks[0].desktop_id, "firefox.desktop");
        assert_eq!(config.dock.bookmarks[0].label, None);
        assert!(config.dock.bookmarks[0].exec_args.is_empty());
        assert_eq!(config.dock.bookmarks[1].desktop_id, "kitty.desktop");
    }

    #[test]
    fn test_dock_bookmarks_mixed_deserialize() {
        let raw = r#"
            [dock]
            bookmarks = [
                "firefox.desktop",
                { desktop_id = "kitty.desktop", exec_args = ["--single-instance"] }
            ]
        "#;
        let config: Config = toml::from_str(raw).expect("mixed dock bookmarks should deserialize");

        assert_eq!(config.dock.bookmarks.len(), 2);
        assert_eq!(config.dock.bookmarks[0].desktop_id, "firefox.desktop");
        assert!(config.dock.bookmarks[0].exec_args.is_empty());
        assert_eq!(config.dock.bookmarks[1].desktop_id, "kitty.desktop");
        assert_eq!(
            config.dock.bookmarks[1].exec_args,
            vec!["--single-instance".to_string()]
        );
    }

    #[test]
    fn test_dock_defaults_match_serde_defaults() {
        // `Config::init` seeds the config merge from `Config::default()` serialized
        // to TOML, so `DockConfig::default()` — not the `#[serde(default = ...)]`
        // functions — is what a config file without a `[dock] size` ends up with.
        let dock = DockConfig::default();
        assert_eq!(dock.size, default_dock_size());
        assert_eq!(dock.genie_scale, default_genie_scale());
        assert_eq!(dock.genie_span, default_genie_span());
        assert_eq!(dock.colorize_color, default_dock_colorize_color());
        assert_eq!(dock.colorize_intensity, default_dock_colorize_intensity());
        assert_eq!(dock.magnification, default_magnification());

        let merged =
            toml::Value::try_from(Config::default()).expect("default config is valid toml");
        let from_defaults: Config = merged.try_into().expect("default config round-trips");
        assert_eq!(from_defaults.dock.size, default_dock_size());
    }

    /// Exactly what [`save_dock_bookmarks`] writes, minus the filesystem.
    fn merge_dock_bookmarks(doc: &mut toml_edit::DocumentMut, bookmarks: &[DockBookmark]) {
        let value = toml::Value::try_from(SerializedBookmarks(bookmarks))
            .ok()
            .and_then(|value| file::to_edit_value(&value))
            .expect("bookmarks are valid toml");
        file::set_key(doc, "dock.bookmarks", value).expect("bookmarks are settable");
    }

    #[test]
    fn test_save_dock_config_keeps_hand_edited_keys() {
        let raw = "# hand written\n[dock]\nsize = 1.6\n# the minimise animation\ngenie_scale = 0.3\nautohide = false\n";
        let mut doc: toml_edit::DocumentMut = raw.parse().expect("config should parse");

        merge_dock_bookmarks(
            &mut doc,
            &[DockBookmark {
                desktop_id: "firefox.desktop".to_string(),
                label: None,
                exec_args: Vec::new(),
            }],
        );

        // The dock's own list is written…
        assert_eq!(
            file::get_key(&doc, "dock.bookmarks")
                .and_then(|v| v.as_array())
                .map(|b| b.len()),
            Some(1)
        );
        // …and nothing else in the file moves: not the keys the user set,
        assert_eq!(
            file::get_key(&doc, "dock.size").and_then(|v| v.as_float()),
            Some(1.6)
        );
        assert_eq!(
            file::get_key(&doc, "dock.genie_scale").and_then(|v| v.as_float()),
            Some(0.3)
        );
        assert_eq!(
            file::get_key(&doc, "dock.autohide").and_then(|v| v.as_bool()),
            Some(false)
        );
        // nor their comments.
        let out = doc.to_string();
        assert!(out.contains("# hand written"), "{out}");
        assert!(out.contains("# the minimise animation"), "{out}");
    }

    #[test]
    #[serial]
    fn test_persist_and_forget_touch_only_one_key() {
        let env = ConfigEnv::new();
        let config_file = env.user_config(
            "# my desktop\nscreen_scale = 2.0\n\n[dock]\n# tuned by hand\ngenie_scale = 0.3\n",
        );

        persist_key("dock.size", &toml::Value::Float(1.25)).expect("size should persist");
        let written = fs::read_to_string(&config_file).unwrap();
        assert!(written.contains("size = 1.25"), "{written}");
        assert!(written.contains("# my desktop"), "{written}");
        assert!(written.contains("# tuned by hand"), "{written}");
        assert!(written.contains("genie_scale = 0.3"), "{written}");
        assert!(stored_key("dock.size").is_some());

        // Resetting takes the key back out, and nothing else with it.
        forget_key("dock.size").expect("size should be forgettable");
        let written = fs::read_to_string(&config_file).unwrap();
        assert!(!written.contains("size = 1.25"), "{written}");
        assert!(written.contains("genie_scale = 0.3"), "{written}");
        assert!(stored_key("dock.size").is_none());

        // …and once the last dock key goes, so does the empty `[dock]` table.
        forget_key("dock.genie_scale").expect("genie_scale should be forgettable");
        let written = fs::read_to_string(&config_file).unwrap();
        assert!(!written.contains("[dock]"), "{written}");
        assert!(written.contains("screen_scale = 2.0"), "{written}");

        // Forgetting a key that was never written is not an error.
        forget_key("dock.autohide").expect("an absent key resets cleanly");
    }

    #[test]
    fn test_dock_position_round_trip() {
        let config: Config = toml::from_str(
            r#"
            [dock]
            position = "left"
        "#,
        )
        .expect("dock position should deserialize");
        assert_eq!(config.dock.position, DockPosition::Left);
        assert!(config.dock.position.is_vertical());

        // Absent means bottom, and the dock writes its own choice back.
        let default: Config = toml::from_str("[dock]\n").expect("empty dock table should parse");
        assert_eq!(default.dock.position, DockPosition::Bottom);

        let mut doc: toml_edit::DocumentMut = "[dock]\n".parse().expect("config should parse");
        let value = toml::Value::try_from(DockPosition::Right)
            .ok()
            .and_then(|value| file::to_edit_value(&value))
            .expect("a dock position is valid toml");
        file::set_key(&mut doc, "dock.position", value).expect("position is settable");
        assert_eq!(
            file::get_key(&doc, "dock.position").and_then(|v| v.as_str()),
            Some("right")
        );
    }

    #[test]
    fn test_prune_materialized_dock_keys() {
        // The whole table as an older build wrote it, with `size` since changed.
        let raw = r##"
            [dock]
            size = 2.0
            genie_scale = 0.5
            genie_span = 10.0
            colorize_icons = false
            colorize_color = "#ffffff"
            colorize_intensity = 1.0
            autohide = true
            magnification = true
        "##;
        let mut doc: toml::Value = raw.parse().expect("config should parse");
        let pruned = prune_materialized_dock_keys(&mut doc);

        assert!(!pruned.contains(&"size"));
        assert!(pruned.contains(&"genie_scale"));
        let dock = doc.get("dock").expect("dock table is present");
        // The value the user picked stays, the copies of the defaults go…
        assert_eq!(dock.get("size").and_then(toml::Value::as_float), Some(2.0));
        assert!(dock.get("genie_scale").is_none());
        assert!(dock.get("colorize_color").is_none());
        // …and the dock's own keys are none of this migration's business.
        assert_eq!(
            dock.get("autohide").and_then(toml::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn test_prune_drops_the_zeroed_defaults_old_builds_wrote() {
        // What an old build actually left behind: `size` copied from the built-in
        // default, `colorize_*` copied from the derived `Default` that zeroed them,
        // `genie_*` copied from a real config.
        let raw = r#"
            [dock]
            size = 1.0
            genie_scale = 0.3
            genie_span = 23.0
            colorize_icons = false
            colorize_color = ""
            colorize_intensity = 0.0
        "#;
        let mut doc: toml::Value = raw.parse().expect("config should parse");
        prune_materialized_dock_keys(&mut doc);

        let dock = doc.get("dock").expect("dock table is present");
        assert!(dock.get("size").is_none());
        assert!(dock.get("colorize_color").is_none());
        assert!(dock.get("colorize_intensity").is_none());
        // A value that is neither the default nor a zero could have been chosen.
        assert_eq!(
            dock.get("genie_span").and_then(toml::Value::as_float),
            Some(23.0)
        );
    }

    #[test]
    fn test_prune_leaves_hand_written_dock_tables_alone() {
        // Nobody hand-writes all six keys, so a partial table is somebody's config
        // even when a value happens to equal the default.
        let raw = format!(
            "[dock]\nsize = 1.0\ngenie_scale = {}\n",
            default_genie_scale()
        );
        let mut doc: toml::Value = raw.parse().expect("config should parse");

        assert!(prune_materialized_dock_keys(&mut doc).is_empty());
        assert_eq!(
            doc.get("dock").and_then(|d| d.get("genie_scale")),
            Some(&toml::Value::Float(default_genie_scale()))
        );
    }

    #[test]
    fn test_prune_matches_defaults_written_as_integers() {
        let raw = r##"
            [dock]
            size = 1
            genie_scale = 0.5
            genie_span = 10
            colorize_icons = false
            colorize_color = "#ffffff"
            colorize_intensity = 1
        "##;
        let mut doc: toml::Value = raw.parse().expect("config should parse");

        assert_eq!(prune_materialized_dock_keys(&mut doc).len(), 6);
        let dock = doc.get("dock").expect("dock table is present");
        assert!(dock.as_table().expect("dock is a table").is_empty());
    }

    #[test]
    fn test_dock_bookmarks_compact_serialize() {
        let dock = DockConfig {
            bookmarks: vec![
                DockBookmark {
                    desktop_id: "firefox.desktop".to_string(),
                    label: None,
                    exec_args: Vec::new(),
                },
                DockBookmark {
                    desktop_id: "kitty.desktop".to_string(),
                    label: None,
                    exec_args: Vec::new(),
                },
            ],
            ..DockConfig::default()
        };
        let value = toml::Value::try_from(dock).expect("dock config should serialize");
        let bookmarks = value
            .get("bookmarks")
            .and_then(toml::Value::as_array)
            .expect("bookmarks should be serialized as array");

        assert_eq!(bookmarks.len(), 2);
        assert_eq!(bookmarks[0].as_str(), Some("firefox.desktop"));
        assert_eq!(bookmarks[1].as_str(), Some("kitty.desktop"));
    }

    #[test]
    fn test_dock_bookmarks_detailed_serialize_when_needed() {
        let dock = DockConfig {
            bookmarks: vec![DockBookmark {
                desktop_id: "kitty.desktop".to_string(),
                label: None,
                exec_args: vec!["--single-instance".to_string()],
            }],
            ..DockConfig::default()
        };
        let value = toml::Value::try_from(dock).expect("dock config should serialize");
        let bookmarks = value
            .get("bookmarks")
            .and_then(toml::Value::as_array)
            .expect("bookmarks should be serialized as array");
        let first = bookmarks[0]
            .as_table()
            .expect("bookmark should serialize as detailed table when args exist");

        assert_eq!(
            first.get("desktop_id").and_then(toml::Value::as_str),
            Some("kitty.desktop")
        );
        assert_eq!(
            first
                .get("exec_args")
                .and_then(toml::Value::as_array)
                .map(|arr| arr.len()),
            Some(1)
        );
    }

    #[test]
    #[serial]
    fn test_backend_override_candidates() {
        let winit = backend_override_candidates("winit");
        assert_eq!(winit, vec!["otto_config.winit.toml"]);

        let udev = backend_override_candidates("tty-udev");
        assert_eq!(
            udev,
            vec!["otto_config.tty-udev.toml", "otto_config.udev.toml"]
        );

        let x11 = backend_override_candidates("x11");
        assert_eq!(x11, vec!["otto_config.x11.toml", "otto_config.udev.toml"]);

        let custom = backend_override_candidates("custom");
        assert_eq!(custom, vec!["otto_config.custom.toml"]);
    }

    #[test]
    fn test_scroll_speed_negative_clamping() {
        // Test that negative values are clamped to 0.0
        let val: f64 = (-2.5f64).max(0.0);
        assert_eq!(val, 0.0, "negative scroll_speed should be clamped to 0.0");
    }

    #[test]
    fn test_scroll_speed_positive_preserved() {
        // Test that positive values are preserved
        let val: f64 = (2.5f64).max(0.0);
        assert_eq!(val, 2.5, "positive scroll_speed should be preserved");
    }

    #[test]
    fn test_scroll_speed_zero_preserved() {
        // Test that zero is preserved
        let val: f64 = (0.0f64).max(0.0);
        assert_eq!(val, 0.0, "zero scroll_speed should be preserved");
    }

    #[test]
    fn test_scroll_speed_default() {
        // Test default value
        let val = default_scroll_speed();
        assert_eq!(val, 1.0, "scroll_speed should default to 1.0");
    }

    #[test]
    fn test_exec_once_deserialization() {
        let toml_str = r#"
            [[exec_once]]
            cmd = "waybar"

            [[exec_once]]
            cmd = "swaybg"
            args = ["-i", "/path/to/wallpaper.png"]
        "#;

        let config: Config = toml::from_str(toml_str).expect("Config should deserialize");
        assert_eq!(config.exec_once.len(), 2);
        assert_eq!(config.exec_once[0].cmd, "waybar");
        assert!(
            config.exec_once[0].args.is_empty(),
            "args should default to empty"
        );
        assert_eq!(config.exec_once[1].cmd, "swaybg");
        assert_eq!(
            config.exec_once[1].args,
            vec!["-i", "/path/to/wallpaper.png"]
        );
    }

    #[test]
    fn test_exec_once_defaults_to_empty() {
        let config = Config::default();
        assert!(
            config.exec_once.is_empty(),
            "exec_once should default to empty"
        );
    }

    /// The example config is what the packages install as
    /// `/etc/otto/config.toml`, so it is the default a fresh install runs
    /// with. It has to parse, and it has to bring up the desktop the user
    /// guide describes — every binding in it resolves, the bar and the
    /// island start, and the dock is not empty.
    #[test]
    fn shipped_example_config_is_a_usable_default() {
        let toml_str = include_str!("../../otto_config.example.toml");
        let mut config: Config = toml::from_str(toml_str).expect("example config deserializes");
        config.rebuild_shortcut_bindings();

        let started: Vec<&str> = config.exec_once.iter().map(|e| e.cmd.as_str()).collect();
        assert!(started.contains(&"otto-bar"), "the top bar autostarts");
        assert!(
            started.contains(&"otto-islands"),
            "the dynamic island autostarts"
        );

        assert!(!config.dock.bookmarks.is_empty(), "the dock has bookmarks");
        assert!(
            config.displays.named.is_empty() && config.displays.generic.is_empty(),
            "no display profile forces a mode or a position on unknown hardware"
        );

        // Every binding in the file resolved: a skipped one only warns.
        assert_eq!(
            config.shortcut_bindings().len(),
            config.keyboard_shortcuts.len(),
            "every shipped shortcut parses"
        );
        assert!(
            !config.shortcut_bindings().iter().any(|b| matches!(
                b.action,
                shortcuts::ShortcutAction::Builtin(shortcuts::BuiltinAction::Quit)
            )),
            "quitting rides on the always-on keys, not a shipped binding"
        );
    }
}
