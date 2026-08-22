//! The settings service: schema, validation, apply, persistence, announcement.
//!
//! Every change to a configured value — from the settings app, from a dock
//! context menu, from a handle drag — goes through [`set`], so that all of them
//! validate, apply, persist and announce identically. The order is fixed by
//! `specs/settings-app.md`: validate → apply → persist → announce, and a failure
//! to apply persists nothing.

pub mod apply;
pub mod schema;
pub mod value;

use std::sync::{Mutex, OnceLock};

use schema::{Apply, Invalid, SettingSpec};
use value::{SettingType, SettingValue};

use crate::config::Config;
use crate::state::{Backend, Otto};

/// What a successful `Set`/`Reset` reports back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Live now, and persisted.
    Applied,
    /// Persisted; takes effect on restart.
    PendingRestart,
}

impl Status {
    pub fn wire_name(self) -> &'static str {
        match self {
            Status::Applied => "applied",
            Status::PendingRestart => "pending-restart",
        }
    }
}

/// Why a `Set`/`Reset` failed. Each variant maps to one D-Bus error name.
#[derive(Debug)]
pub enum SetError {
    Unknown(String),
    InvalidType(String),
    OutOfRange(String),
    Unsupported(String),
    ApplyFailed(String),
}

impl std::fmt::Display for SetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetError::Unknown(message)
            | SetError::InvalidType(message)
            | SetError::OutOfRange(message)
            | SetError::Unsupported(message)
            | SetError::ApplyFailed(message) => f.write_str(message),
        }
    }
}

/// Walk a dotted path into a TOML document.
fn toml_at<'a>(doc: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let mut value = doc;
    for segment in path.split('.') {
        value = value.get(segment)?;
    }
    Some(value)
}

/// Write a dotted path into a TOML document, creating tables as needed.
fn toml_set(doc: &mut toml::Value, path: &str, new: toml::Value) -> Result<(), String> {
    let segments: Vec<&str> = path.split('.').collect();
    let (leaf, parents) = segments
        .split_last()
        .ok_or_else(|| "empty setting path".to_string())?;

    let mut value = doc;
    for segment in parents {
        let table = value
            .as_table_mut()
            .ok_or_else(|| format!("`{segment}` is not a table"))?;
        value = table
            .entry(segment.to_string())
            .or_insert_with(|| toml::Value::Table(Default::default()));
    }
    value
        .as_table_mut()
        .ok_or_else(|| format!("`{path}` is not a table"))?
        .insert(leaf.to_string(), new);
    Ok(())
}

/// The effective value of `spec` in `config`.
fn value_in(config_toml: &toml::Value, spec: &SettingSpec) -> SettingValue {
    toml_at(config_toml, spec.id)
        .and_then(|value| SettingValue::from_toml(value, spec.ty))
        // An `Option` field that is unset serialises to nothing at all; the
        // client is told the empty value rather than being left without one,
        // because `GetAll` must answer for every identifier `Describe` lists.
        .unwrap_or_else(|| SettingValue::empty(spec.ty))
}

/// `Config` as TOML, which is how settings are addressed generically.
fn config_toml(config: &Config) -> toml::Value {
    toml::Value::try_from(config).expect("config is always valid toml")
}

/// The current effective value of every setting, in schema order.
pub fn all_values() -> Vec<(&'static str, SettingValue)> {
    let doc = config_toml(&Config::current());
    schema::SETTINGS
        .iter()
        .map(|spec| (spec.id, value_in(&doc, spec)))
        .collect()
}

/// The current effective value of one setting.
pub fn value_of(id: &str) -> Option<SettingValue> {
    let spec = schema::lookup(id)?;
    Some(value_in(&config_toml(&Config::current()), spec))
}

/// The built-in default of one setting — what the compositor would use with no
/// configuration file at all.
pub fn default_of(spec: &SettingSpec) -> SettingValue {
    static DEFAULTS: OnceLock<toml::Value> = OnceLock::new();
    let doc = DEFAULTS.get_or_init(|| config_toml(&Config::default()));
    value_in(doc, spec)
}

/// The identifiers currently set in the writable configuration file — what the
/// app needs to offer a per-setting revert.
pub fn overridden() -> Vec<&'static str> {
    let path = crate::config::writable_config_path();
    let Ok(doc) = crate::config::file::load_document(&path) else {
        return Vec::new();
    };
    schema::SETTINGS
        .iter()
        .filter(|spec| crate::config::file::get_key(&doc, spec.id).is_some())
        .map(|spec| spec.id)
        .collect()
}

/// Produce a copy of `config` with `id` set to `value`.
///
/// Goes through TOML rather than a field-by-field match: the identifier scheme
/// *is* the config structure, so one generic path cannot drift out of sync with
/// the schema the way forty hand-written arms would.
fn config_with(config: &Config, id: &str, value: &SettingValue) -> Result<Config, String> {
    let mut doc = config_toml(config);
    toml_set(&mut doc, id, value.to_toml())?;
    doc.try_into()
        .map_err(|err| format!("`{id}` is not a value the compositor accepts: {err}"))
}

/// Set one setting: validate, apply, persist, announce.
pub fn set<B: Backend + 'static>(
    state: &mut Otto<B>,
    id: &str,
    value: SettingValue,
) -> Result<Status, SetError> {
    let spec =
        schema::lookup(id).ok_or_else(|| SetError::Unknown(format!("no such setting `{id}`")))?;

    match spec.validate(&value) {
        Ok(()) => {}
        Err(Invalid::Type(message)) => return Err(SetError::InvalidType(message)),
        Err(Invalid::Range(message)) => return Err(SetError::OutOfRange(message)),
    }

    if spec.apply == Apply::Unsupported {
        return Err(SetError::Unsupported(format!(
            "`{id}` cannot be changed on this system"
        )));
    }

    // Nothing to do only when the running system *and* the file already agree.
    // The file has to be checked separately: a live interaction such as a dock
    // resize changes the running configuration first and comes here to persist
    // it, so an effective-value comparison alone would drop the write.
    let stored = crate::config::stored_key(id);
    if value_of(id).as_ref() == Some(&value)
        && stored
            .as_ref()
            .and_then(|stored| SettingValue::from_toml(stored, spec.ty))
            .as_ref()
            == Some(&value)
    {
        return Ok(status_for(spec));
    }

    // Apply. The previous snapshot is kept so a failure can leave the running
    // system exactly as it was — nothing is persisted unless the apply worked.
    let previous = Config::current();
    Config::update(|config| {
        if let Ok(next) = config_with(config, id, &value) {
            *config = next;
        }
    });
    if value_of(id).as_ref() != Some(&value) {
        Config::update(|config| *config = (*previous).clone());
        return Err(SetError::ApplyFailed(format!(
            "`{id}` could not be stored in the running configuration"
        )));
    }

    if spec.apply == Apply::Live {
        if let Err(reason) = apply::apply_live(state, id) {
            Config::update(|config| *config = (*previous).clone());
            let _ = apply::apply_live(state, id);
            return Err(SetError::ApplyFailed(reason));
        }
    }

    // Persist. Only this key is written; every other key, section and comment
    // in the file is left alone.
    if let Err(reason) = crate::config::persist_key(id, &value.to_toml()) {
        Config::update(|config| *config = (*previous).clone());
        if spec.apply == Apply::Live {
            let _ = apply::apply_live(state, id);
        }
        return Err(SetError::ApplyFailed(format!(
            "could not persist: {reason}"
        )));
    }

    announce(&[(id.to_string(), value)]);
    Ok(status_for(spec))
}

/// Remove a setting from the writable configuration file, so it falls back to
/// whatever the lower layers provide.
pub fn reset<B: Backend + 'static>(state: &mut Otto<B>, id: &str) -> Result<Status, SetError> {
    let spec =
        schema::lookup(id).ok_or_else(|| SetError::Unknown(format!("no such setting `{id}`")))?;

    if spec.apply == Apply::Unsupported {
        return Err(SetError::Unsupported(format!(
            "`{id}` cannot be changed on this system"
        )));
    }

    crate::config::forget_key(id)
        .map_err(|reason| SetError::ApplyFailed(format!("could not persist: {reason}")))?;

    // The value now comes from the lower layers, which only a full reload can
    // tell us — and the reload may turn up other keys that changed on disk
    // since, so everything that moved is applied and announced.
    let (previous, next) = Config::reload()
        .map_err(|reason| SetError::ApplyFailed(format!("could not reload config: {reason}")))?;
    reconcile(state, &previous, &next);

    Ok(status_for(spec))
}

/// Apply and announce every setting whose effective value differs between two
/// configuration snapshots.
pub fn reconcile<B: Backend + 'static>(state: &mut Otto<B>, previous: &Config, next: &Config) {
    let before = config_toml(previous);
    let after = config_toml(next);

    let mut changes = Vec::new();
    for spec in schema::SETTINGS {
        let old = value_in(&before, spec);
        let new = value_in(&after, spec);
        if old == new {
            continue;
        }
        if spec.apply == Apply::Live {
            if let Err(reason) = apply::apply_live(state, spec.id) {
                tracing::warn!("Could not apply `{}` live: {reason}", spec.id);
            }
        }
        changes.push((spec.id.to_string(), new));
    }

    announce(&changes);
}

fn status_for(spec: &SettingSpec) -> Status {
    match spec.apply {
        Apply::Live => Status::Applied,
        _ => Status::PendingRestart,
    }
}

/// Where announcements go. Set by the D-Bus service when it registers; unset in
/// tests and in a headless run without a session bus, where announcing is a
/// no-op rather than an error.
type Announcer = Box<dyn Fn(Vec<(String, SettingValue)>) + Send + Sync>;
static ANNOUNCER: OnceLock<Mutex<Option<Announcer>>> = OnceLock::new();

fn announcer() -> &'static Mutex<Option<Announcer>> {
    ANNOUNCER.get_or_init(|| Mutex::new(None))
}

/// Install the sink the `Changed` signal is emitted through.
pub fn set_announcer(announcer_fn: Announcer) {
    *announcer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(announcer_fn);
}

/// Announce changed effective values to every observer.
///
/// Called for changes from any source, including in-compositor interactions, so
/// a settings app showing a value updates when the dock is dragged.
pub fn announce(changes: &[(String, SettingValue)]) {
    if changes.is_empty() {
        return;
    }
    let guard = announcer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(announcer) = guard.as_ref() {
        announcer(changes.to_vec());
    }
}

/// The schema as the bus serves it: one dictionary per setting.
pub fn describe() -> Vec<std::collections::HashMap<String, zbus::zvariant::OwnedValue>> {
    schema::SETTINGS
        .iter()
        .map(|spec| {
            let mut entry: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
                std::collections::HashMap::new();
            let put = |entry: &mut std::collections::HashMap<String, _>,
                       key: &str,
                       value: SettingValue| {
                entry.insert(key.to_string(), value.to_variant());
            };
            put(&mut entry, "id", SettingValue::Str(spec.id.to_string()));
            put(
                &mut entry,
                "type",
                SettingValue::Str(spec.ty.wire_name().to_string()),
            );
            put(
                &mut entry,
                "section",
                SettingValue::Str(spec.section().to_string()),
            );
            put(
                &mut entry,
                "label",
                SettingValue::Str(spec.label.to_string()),
            );
            put(
                &mut entry,
                "description",
                SettingValue::Str(spec.description.to_string()),
            );
            put(
                &mut entry,
                "apply",
                SettingValue::Str(spec.apply.wire_name().to_string()),
            );
            entry.insert("default".to_string(), default_of(spec).to_variant());
            if let Some(min) = spec.min {
                entry.insert("min".to_string(), numeric(spec.ty, min).to_variant());
            }
            if let Some(max) = spec.max {
                entry.insert("max".to_string(), numeric(spec.ty, max).to_variant());
            }
            if let Some(step) = spec.step {
                entry.insert("step".to_string(), numeric(spec.ty, step).to_variant());
            }
            if !spec.choices.is_empty() {
                put(
                    &mut entry,
                    "choices",
                    SettingValue::StrList(spec.choices.iter().map(|c| c.to_string()).collect()),
                );
            }
            if !spec.choice_labels.is_empty() {
                put(
                    &mut entry,
                    "choice_labels",
                    SettingValue::StrList(
                        spec.choice_labels.iter().map(|c| c.to_string()).collect(),
                    ),
                );
            }
            entry
        })
        .collect()
}

/// A bound, typed the way the setting itself is typed, so a client can compare
/// it against the value without converting.
fn numeric(ty: SettingType, number: f64) -> SettingValue {
    match ty {
        SettingType::Int => SettingValue::Int(number as i64),
        _ => SettingValue::Double(number),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_read_out_of_the_config_by_dotted_path() {
        let mut config = Config::default();
        config.dock.size = 1.75;
        config.input.pointer_accel_speed = -0.5;
        let doc = config_toml(&config);

        assert_eq!(
            value_in(&doc, schema::lookup("dock.size").expect("exists")),
            SettingValue::Double(1.75)
        );
        assert_eq!(
            value_in(
                &doc,
                schema::lookup("input.pointer_accel_speed").expect("exists")
            ),
            SettingValue::Double(-0.5)
        );
        assert_eq!(
            value_in(&doc, schema::lookup("dock.position").expect("exists")),
            SettingValue::Str("bottom".to_string())
        );
    }

    #[test]
    fn an_unset_option_reads_as_the_empty_value() {
        let doc = config_toml(&Config::default());
        assert_eq!(
            value_in(&doc, schema::lookup("icon_theme").expect("exists")),
            SettingValue::Str(String::new())
        );
    }

    #[test]
    fn setting_a_value_round_trips_through_the_config() {
        let config = Config::default();
        let next = config_with(&config, "dock.autohide", &SettingValue::Bool(true))
            .expect("dock.autohide is settable");
        assert!(next.dock.autohide);
        assert_eq!(next.dock.size, config.dock.size);

        let next = config_with(&next, "dock.position", &SettingValue::Str("left".into()))
            .expect("dock.position is settable");
        assert_eq!(next.dock.position, crate::config::DockPosition::Left);
        // The unrelated change above is still there.
        assert!(next.dock.autohide);
    }

    #[test]
    fn a_value_the_config_cannot_hold_is_refused() {
        let config = Config::default();
        assert!(config_with(
            &config,
            "dock.position",
            &SettingValue::Str("sideways".into())
        )
        .is_err());
    }

    #[test]
    fn defaults_come_from_the_built_in_config() {
        assert_eq!(
            default_of(schema::lookup("dock.size").expect("exists")),
            SettingValue::Double(1.0)
        );
        assert_eq!(
            default_of(schema::lookup("dock.magnification").expect("exists")),
            SettingValue::Bool(true)
        );
    }

    #[test]
    fn describe_answers_for_every_setting_get_all_lists() {
        let described: Vec<String> = describe()
            .into_iter()
            .filter_map(|entry| {
                entry
                    .get("id")
                    .and_then(|id| <&str>::try_from(&**id).ok())
                    .map(str::to_string)
            })
            .collect();
        assert_eq!(described.len(), schema::SETTINGS.len());
        for spec in schema::SETTINGS {
            assert!(described.iter().any(|id| id == spec.id));
        }
    }
}
