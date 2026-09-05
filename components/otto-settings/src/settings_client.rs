//! Client for `org.otto.Settings`.
//!
//! Speaks the contract in `docs/developer/settings-dbus-api.md`. The schema,
//! the current values and the overridden set are fetched once at startup and
//! kept in a process-wide store, so the draw path — which runs on every frame
//! and cannot block — reads them without touching the bus.
//!
//! The compositor side is still being built. Until it answers, the store stays
//! **offline**: panes fall back to the placeholder values in [`crate::model`]
//! and the app presents itself as read-only rather than pretending a `Set`
//! landed.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Type, Value as ZValue};

const BUS_NAME: &str = "org.otto.Settings";
const OBJECT_PATH: &str = "/org/otto/Settings";
const INTERFACE: &str = "org.otto.Settings";

/// A setting's value, in the shapes the contract's `type` column allows.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Double(f64),
    Text(String),
    List(Vec<String>),
}

#[allow(dead_code)] // the full accessor set is used as more panes are wired
impl Value {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Value::Double(v) => Some(*v as f32),
            Value::Int(v) => Some(*v as f32),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(v) => Some(v),
            _ => None,
        }
    }

    fn from_zbus(value: &OwnedValue) -> Option<Self> {
        match &**value {
            ZValue::Bool(v) => Some(Value::Bool(*v)),
            ZValue::I32(v) => Some(Value::Int(*v)),
            ZValue::U32(v) => Some(Value::Int(*v as i32)),
            ZValue::F64(v) => Some(Value::Double(*v)),
            ZValue::Str(v) => Some(Value::Text(v.to_string())),
            ZValue::Array(array) => Some(Value::List(
                array
                    .iter()
                    .filter_map(|item| match item {
                        ZValue::Str(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect(),
            )),
            _ => None,
        }
    }

    fn to_zbus(&self) -> ZValue<'static> {
        match self {
            Value::Bool(v) => ZValue::Bool(*v),
            Value::Int(v) => ZValue::I32(*v),
            Value::Double(v) => ZValue::F64(*v),
            Value::Text(v) => ZValue::Str(v.clone().into()),
            Value::List(v) => {
                let mut array = zbus::zvariant::Array::new(<&str>::signature());
                for item in v {
                    // The element type is fixed above, so this cannot fail.
                    let _ = array.append(ZValue::Str(item.clone().into()));
                }
                ZValue::Array(array)
            }
        }
    }
}

/// The type the schema declares for a setting.
///
/// The compositor never coerces — sending a double to an `int` setting is
/// refused — so a control has to emit the declared shape rather than whatever
/// its own arithmetic produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    Bool,
    Int,
    Double,
    #[default]
    Text,
    /// A colour. `choices`, when present, are palette names the compositor
    /// resolves — swatches to offer — but a `#RRGGBB` literal is accepted
    /// too, which is what separates this from a plain enumeration.
    Color,
    List,
}

impl Kind {
    fn parse(raw: &str) -> Self {
        match raw {
            "bool" => Kind::Bool,
            "int" => Kind::Int,
            "double" => Kind::Double,
            "color" => Kind::Color,
            "string-list" => Kind::List,
            _ => Kind::Text,
        }
    }
}

/// What the compositor says happens when a setting is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apply {
    Live,
    Restart,
    Unsupported,
}

impl Apply {
    fn parse(raw: &str) -> Self {
        match raw {
            "live" => Apply::Live,
            "restart" => Apply::Restart,
            _ => Apply::Unsupported,
        }
    }
}

/// One entry of the served schema. Only the fields the app actually renders
/// from are kept; unknown keys in the reply are ignored, as the contract
/// requires, so the compositor can add more without breaking this build.
#[derive(Debug, Clone)]
#[allow(dead_code)] // ditto: panes read more of the schema as they are wired
pub struct Desc {
    pub id: String,
    pub kind: Kind,
    pub label: String,
    pub description: String,
    pub apply: Apply,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Granularity to snap a slider to, when the setting has one.
    pub step: Option<f64>,
    /// The inherited value, used as a detent so a slider can land exactly on
    /// it rather than a hair either side.
    pub default: Option<f64>,
    pub choices: Vec<String>,
    /// Human names for `choices`, in the same order, when the configuration
    /// tokens are not fit to show. Empty when they are.
    pub choice_labels: Vec<String>,
}

impl Desc {
    /// What to show the user in place of a stored value. Falls back to the
    /// value itself, which covers every setting the compositor did not bother
    /// to label and every value it does not recognise.
    pub fn display(&self, value: &str) -> String {
        self.choices
            .iter()
            .position(|c| c == value)
            .and_then(|i| self.choice_labels.get(i))
            .cloned()
            .unwrap_or_else(|| value.to_string())
    }
}

#[derive(Default)]
struct Store {
    online: bool,
    /// Settings changed in this session that the compositor said need a
    /// restart. Not the same as the schema's `apply` field: most settings
    /// need a restart, but the badge is only meaningful once one has actually
    /// been changed.
    pending_restart: HashSet<String>,
    schema: HashMap<String, Desc>,
    values: HashMap<String, Value>,
    overridden: HashSet<String>,
}

static STORE: OnceLock<RwLock<Store>> = OnceLock::new();
static CONNECTION: OnceLock<Option<Connection>> = OnceLock::new();

fn store() -> &'static RwLock<Store> {
    STORE.get_or_init(|| RwLock::new(Store::default()))
}

/// Whether the compositor answered and the app is showing live values.
pub fn is_online() -> bool {
    store().read().map(|s| s.online).unwrap_or(false)
}

/// The current value of a setting, if the compositor served one.
pub fn value(id: &str) -> Option<Value> {
    store().read().ok()?.values.get(id).cloned()
}

/// Whether a setting is set in the user's own config file, and so offers a
/// revert.
///
/// Nothing reads this yet: the badge that used to show it was an unlabelled
/// glyph that could not be clicked and read as a restart marker, so it was
/// removed. The override set is still tracked here, and [`reset`] still
/// written, for the undo affordance that replaces it.
#[allow(dead_code)]
pub fn is_overridden(id: &str) -> bool {
    store()
        .read()
        .map(|s| s.overridden.contains(id))
        .unwrap_or(false)
}

/// Whether this session has changed a setting that is waiting on a restart.
pub fn is_pending_restart(id: &str) -> bool {
    store()
        .read()
        .map(|s| s.pending_restart.contains(id))
        .unwrap_or(false)
}

/// The schema entry for a setting, if the compositor served one.
/// Quantise a slider's raw drag position to something the user meant.
///
/// A drag lands on whatever float the pixel maps to, which is how a pointer
/// speed ends up reading `-0.01`. Snapping to the schema's step fixes that,
/// and a detent at the default lets the value land exactly back on it — the
/// one position a user is most likely to be aiming for.
pub fn snap(id: &str, raw: f32) -> f32 {
    let store = STORE.get_or_init(Default::default).read().unwrap();
    let Some(desc) = store.schema.get(id) else {
        return raw;
    };

    let raw = raw as f64;
    let mut value = match desc.step {
        Some(step) if step > 0.0 => (raw / step).round() * step,
        _ => raw,
    };

    if let (Some(default), Some(step)) = (desc.default, desc.step) {
        if (raw - default).abs() <= step / 2.0 {
            value = default;
        }
    }

    if let Some(min) = desc.min {
        value = value.max(min);
    }
    if let Some(max) = desc.max {
        value = value.min(max);
    }
    value as f32
}

/// The human name for a stored enum value, for display only. Controls keep
/// holding the configuration token, so hit-testing and `Set` are unaffected.
pub fn display_choice(id: &str, value: &str) -> String {
    let store = STORE.get_or_init(Default::default).read().unwrap();
    match store.schema.get(id) {
        Some(desc) => desc.display(value),
        None => value.to_string(),
    }
}

pub fn describe(id: &str) -> Option<Desc> {
    store().read().ok()?.schema.get(id).cloned()
}

/// Connect and populate the store. Failure is not fatal: the app stays usable
/// against a compositor that does not serve the interface yet.
pub fn connect() {
    let connection = CONNECTION.get_or_init(|| match Connection::session() {
        Ok(connection) => Some(connection),
        Err(err) => {
            eprintln!("settings: no session bus ({err}); running offline");
            None
        }
    });

    let Some(connection) = connection.as_ref() else {
        return;
    };

    let schema = match fetch_schema(connection) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("settings: {BUS_NAME} unavailable ({err}); running offline");
            return;
        }
    };

    let values = fetch_values(connection).unwrap_or_default();
    let overridden = fetch_overridden(connection).unwrap_or_default();

    if let Ok(mut store) = store().write() {
        store.online = true;
        store.schema = schema;
        store.values = values;
        store.overridden = overridden;
    }
}

/// Set when a `Changed` signal has updated the store and a redraw is owed.
/// Cleared by [`take_dirty`], which the app polls from `App::on_update` —
/// `AppRunner` calls that on the main thread every time round the event
/// loop, which is also the only thread allowed to touch `Window`.
static DIRTY: AtomicBool = AtomicBool::new(false);

/// Whether a `Changed` signal has landed since the last call. `on_update`
/// polls this and, if it's set, calls `Window::request_frame()` itself —
/// this module never touches `Window` because the listener thread that sets
/// the flag is not the main thread, and `Window` is not `Send`.
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::Relaxed)
}

/// Watch `Changed` on a background thread and fold every update into the
/// store, so a change from anywhere — another client's `Set`, an
/// in-compositor interaction like dragging the dock handle, an external edit
/// of the config file, or this app's own `Set` echoing back — is reflected
/// without the draw path ever touching the bus.
///
/// This runs on its own thread rather than folding into the app's poll loop
/// because the client only has `zbus::blocking`: a `SignalIterator::next()`
/// blocks until a message arrives, and blocking the main thread on the bus
/// would freeze drawing and pointer handling for as long as nothing changes.
/// The thread is deliberately kept thin — it only ever touches the
/// (thread-safe, `RwLock`-guarded) store and the `DIRTY` flag, then wakes the
/// main loop with `AppContext::request_wakeup()`, which is documented as
/// safe to call from any thread. The actual `request_frame()` happens back
/// on the main thread, in `on_update`, once the wakeup's `poll()` returns.
pub fn spawn_change_listener() {
    let Some(Some(connection)) = CONNECTION.get() else {
        return;
    };
    let connection = connection.clone();

    let spawned = std::thread::Builder::new()
        .name("settings-changed".into())
        .spawn(move || {
            let proxy =
                match zbus::blocking::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE) {
                    Ok(proxy) => proxy,
                    Err(err) => {
                        eprintln!(
                            "settings: cannot watch {BUS_NAME} ({err}); \
                             external changes will not be reflected"
                        );
                        return;
                    }
                };
            let signals = match proxy.receive_signal("Changed") {
                Ok(signals) => signals,
                Err(err) => {
                    eprintln!("settings: cannot subscribe to Changed ({err})");
                    return;
                }
            };

            for message in signals {
                let changed: HashMap<String, OwnedValue> = match message.body().deserialize() {
                    Ok(changed) => changed,
                    Err(err) => {
                        eprintln!("settings: malformed Changed signal ({err})");
                        continue;
                    }
                };

                let mut any = false;
                if let Ok(mut store) = store().write() {
                    for (id, value) in &changed {
                        if let Some(value) = Value::from_zbus(value) {
                            store.values.insert(id.clone(), value);
                            any = true;
                        }
                    }
                }

                // No recognisable value in the signal body: nothing to
                // redraw for, and waking the main loop would just spin it.
                if any {
                    DIRTY.store(true, Ordering::Relaxed);
                    otto_kit::AppContext::request_wakeup();
                }
            }
        });

    if let Err(err) = spawned {
        eprintln!("settings: could not start the Changed listener thread ({err})");
    }
}

fn call<B, R>(connection: &Connection, method: &str, body: &B) -> zbus::Result<R>
where
    B: serde::ser::Serialize + zbus::zvariant::DynamicType,
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
{
    connection
        .call_method(Some(BUS_NAME), OBJECT_PATH, Some(INTERFACE), method, body)?
        .body()
        .deserialize()
}

fn fetch_schema(connection: &Connection) -> zbus::Result<HashMap<String, Desc>> {
    let raw: Vec<HashMap<String, OwnedValue>> = call(connection, "Describe", &())?;

    Ok(raw
        .into_iter()
        .filter_map(|entry| {
            let id = string_field(&entry, "id")?;
            let desc = Desc {
                kind: Kind::parse(&string_field(&entry, "type").unwrap_or_default()),
                label: string_field(&entry, "label").unwrap_or_else(|| id.clone()),
                description: string_field(&entry, "description").unwrap_or_default(),
                apply: Apply::parse(&string_field(&entry, "apply").unwrap_or_default()),
                min: entry.get("min").and_then(number_field),
                max: entry.get("max").and_then(number_field),
                step: entry.get("step").and_then(number_field),
                default: entry.get("default").and_then(number_field),
                choices: string_list(&entry, "choices"),
                choice_labels: string_list(&entry, "choice_labels"),
                id: id.clone(),
            };
            Some((id, desc))
        })
        .collect())
}

fn fetch_values(connection: &Connection) -> zbus::Result<HashMap<String, Value>> {
    let raw: HashMap<String, OwnedValue> = call(connection, "GetAll", &())?;
    Ok(raw
        .iter()
        .filter_map(|(id, value)| Value::from_zbus(value).map(|v| (id.clone(), v)))
        .collect())
}

fn fetch_overridden(connection: &Connection) -> zbus::Result<HashSet<String>> {
    let raw: Vec<String> = call(connection, "GetOverridden", &())?;
    Ok(raw.into_iter().collect())
}

fn string_field(entry: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    match Value::from_zbus(entry.get(key)?)? {
        Value::Text(text) => Some(text),
        _ => None,
    }
}

fn string_list(entry: &HashMap<String, OwnedValue>, key: &str) -> Vec<String> {
    match entry.get(key).and_then(Value::from_zbus) {
        Some(Value::List(items)) => items,
        _ => Vec::new(),
    }
}

fn number_field(value: &OwnedValue) -> Option<f64> {
    match Value::from_zbus(value)? {
        Value::Double(v) => Some(v),
        Value::Int(v) => Some(v as f64),
        _ => None,
    }
}

/// Wrap a number as the schema declares it, so a slider bound to an `int`
/// setting does not send a double and get refused.
pub fn number_for(id: &str, value: f32) -> Value {
    match describe(id).map(|d| d.kind) {
        Some(Kind::Int) => Value::Int(value.round() as i32),
        _ => Value::Double(value as f64),
    }
}

/// The value a text field should send for `id`.
///
/// The mirror of `number_for`, and of the display side in `model.rs`, which
/// already renders a list setting into one comma-separated field. Committing
/// always sent a plain string, so a list setting — `locales` is the only one
/// today — was refused by the compositor on its type and the edit silently did
/// nothing.
///
/// Splitting on commas is the inverse of the `", "` join used to display it.
/// Empty items are dropped rather than sent, so a trailing comma mid-typing
/// does not become an empty locale.
pub fn text_for(id: &str, value: &str) -> Value {
    match describe(id).map(|d| d.kind) {
        Some(Kind::List) => Value::List(
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        _ => Value::Text(value.to_string()),
    }
}

/// Outcome of a `Set`, as the app needs to present it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetOutcome {
    Applied,
    PendingRestart,
    /// The compositor refused, or is not there. Carries a message to show.
    Failed(String),
}

/// Set a value, and reflect it in the store so the next frame draws it.
///
/// The store is updated from the value we sent rather than by re-reading:
/// `Changed` will arrive too, and agreeing with it is the point. A failure
/// leaves the store untouched, so the UI snaps back to the real value.
pub fn set(id: &str, value: Value) -> SetOutcome {
    let Some(Some(connection)) = CONNECTION.get() else {
        return SetOutcome::Failed("not connected to the compositor".into());
    };

    let status: zbus::Result<String> = call(connection, "Set", &(id, value.to_zbus()));

    match status {
        Ok(status) => {
            if let Ok(mut store) = store().write() {
                store.values.insert(id.to_string(), value);
                store.overridden.insert(id.to_string());
            }
            match status.as_str() {
                "pending-restart" => {
                    if let Ok(mut store) = store().write() {
                        store.pending_restart.insert(id.to_string());
                    }
                    SetOutcome::PendingRestart
                }
                _ => SetOutcome::Applied,
            }
        }
        Err(err) => SetOutcome::Failed(err.to_string()),
    }
}

/// The file a changed setting is written to, as the compositor reports it.
///
/// Asked rather than worked out: configuration is layered, and which layer is
/// writable depends on what exists on this machine.
///
/// Only a *successful* answer is cached. Caching the failure would freeze the
/// row at "not known" for the whole run over one call that came too early or
/// reached a compositor too old to answer.
pub fn config_path() -> Option<String> {
    static PATH: RwLock<Option<String>> = RwLock::new(None);
    if let Some(path) = PATH.read().ok().and_then(|p| p.clone()) {
        return Some(path);
    }
    let Some(Some(connection)) = CONNECTION.get() else {
        return None;
    };
    let path = call::<_, String>(connection, "ConfigPath", &()).ok()?;
    if let Ok(mut cached) = PATH.write() {
        *cached = Some(path.clone());
    }
    Some(path)
}

/// Persist how one display should be driven. Applies at the next start — see
/// `SetOutputProfile` in the compositor's settings service.
///
/// A zero size leaves the resolution unset and a zero rate leaves the refresh
/// unset, so moving a display does not have to name a mode for it.
pub fn set_output_profile(
    connector: &str,
    width: u32,
    height: u32,
    refresh_hz: f64,
    x: i32,
    y: i32,
    primary: bool,
) -> SetOutcome {
    let Some(Some(connection)) = CONNECTION.get() else {
        return SetOutcome::Failed("not connected to the compositor".into());
    };
    let status: zbus::Result<String> = call(
        connection,
        "SetOutputProfile",
        &(connector, width, height, refresh_hz, x, y, primary),
    );
    match status {
        Ok(status) if status == "pending-restart" => SetOutcome::PendingRestart,
        Ok(_) => SetOutcome::Applied,
        Err(err) => SetOutcome::Failed(err.to_string()),
    }
}

/// Drop a setting back to whatever the lower config layers provide.
#[allow(dead_code)] // wired when rows grow a revert affordance
pub fn reset(id: &str) -> SetOutcome {
    let Some(Some(connection)) = CONNECTION.get() else {
        return SetOutcome::Failed("not connected to the compositor".into());
    };

    let status: zbus::Result<String> = call(connection, "Reset", &(id,));

    match status {
        Ok(_) => {
            // The effective value now comes from a lower layer and we do not
            // know it, so re-read rather than guess.
            if let Ok(values) = fetch_values(connection) {
                if let Ok(mut store) = store().write() {
                    store.values = values;
                    store.overridden.remove(id);
                }
            }
            SetOutcome::Applied
        }
        Err(err) => SetOutcome::Failed(err.to_string()),
    }
}

#[cfg(test)]
mod commit_type_tests {
    use super::*;

    /// Put one description in the store, the way `connect` would.
    ///
    /// `text_for` asks the store what kind a setting is, so these tests need
    /// the schema the compositor would have served. Seeding it directly keeps
    /// them from needing a live compositor.
    fn seed(id: &str, kind: Kind) {
        let mut store = store().write().unwrap();
        store.schema.insert(
            id.to_string(),
            Desc {
                id: id.to_string(),
                kind,
                label: String::new(),
                description: String::new(),
                apply: Apply::Restart,
                min: None,
                max: None,
                step: None,
                default: None,
                choices: Vec::new(),
                choice_labels: Vec::new(),
            },
        );
    }

    /// A list setting sends a list, not a string.
    ///
    /// `locales` is declared `StrList`, so committing the field as a plain
    /// string is refused by the compositor on its type — and refused
    /// silently, from the user's side: the field keeps showing what was typed
    /// while nothing was saved. Worth a test precisely because the failure
    /// looks like success.
    #[test]
    fn a_list_setting_splits_on_commas() {
        seed("locales", Kind::List);
        assert_eq!(
            text_for("locales", "it_IT, it, en"),
            Value::List(vec!["it_IT".into(), "it".into(), "en".into()])
        );
    }

    /// Splitting is the exact inverse of how the value is displayed.
    #[test]
    fn splitting_inverts_the_display_join() {
        seed("locales", Kind::List);
        let items = vec!["it_IT".to_string(), "en".to_string()];
        assert_eq!(text_for("locales", &items.join(", ")), Value::List(items));
    }

    /// A comma left mid-typing does not become an empty locale.
    #[test]
    fn empty_items_are_dropped() {
        seed("locales", Kind::List);
        assert_eq!(
            text_for("locales", "it, , en,"),
            Value::List(vec!["it".into(), "en".into()])
        );
    }

    /// Everything that is not a list still sends a plain string. An unknown
    /// id — the Displays pane's unbound fields — falls here too.
    #[test]
    fn other_settings_still_send_text() {
        seed("font_family", Kind::Text);
        assert_eq!(
            text_for("font_family", "Inter"),
            Value::Text("Inter".into())
        );
        assert_eq!(text_for("not.a.setting", "x"), Value::Text("x".into()));
    }
}
