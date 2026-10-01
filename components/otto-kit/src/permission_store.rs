//! Reading and revoking what apps were allowed, in xdg-permission-store.
//!
//! The store (`org.freedesktop.impl.portal.PermissionStore`) is where
//! xdg-desktop-portal keeps per-app decisions: each table maps a resource id
//! to the apps that hold a decision on it, each with a list of permission
//! strings, and an optional value of the writer's own.
//!
//! - `screencast` and `remote-desktop`: one entry per remembered share, a
//!   restore token the frontend (or Otto's portal, for an app that brings no
//!   token of its own) keeps. The data is the backend's `(suv)` restore
//!   payload; Otto's portal names the screen or window in it. Deleting the
//!   entry makes the share forgotten, so the app has to ask again.
//! - `screenshot` / `screenshot`: `yes` or `no` per app.
//! - `notifications` / `notification`: `yes` or `no` per app, written by the
//!   portal's Notification interface the first time an app notifies, and
//!   checked by it before it passes one on.
//! - `otto-agents` / `seat`: `yes` or `no` per program, Otto's own table of
//!   the programs allowed an agent seat.
//!
//! Unsandboxed apps all share the empty app id: the frontend cannot tell
//! them apart.
//!
//! Every call blocks on the bus, so callers run them off the main thread.

use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

const NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
const PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";

pub const SCREENCAST: &str = "screencast";
pub const REMOTE_DESKTOP: &str = "remote-desktop";
pub const SCREENSHOT: &str = "screenshot";
pub const NOTIFICATIONS: &str = "notifications";
/// The one resource id the `screenshot` table uses.
pub const SCREENSHOT_ID: &str = "screenshot";
/// The one resource id the `notifications` table uses.
pub const NOTIFICATION_ID: &str = "notification";
/// Otto's table of programs allowed an agent seat (`yes` or `no` per app),
/// written by the compositor when the user answers its prompt.
pub const AGENTS: &str = "otto-agents";
/// The one resource id the `otto-agents` table uses.
pub const AGENT_ID: &str = "seat";

/// What a yes/no table says about an app: `Some(true)` for `yes`,
/// `Some(false)` for `no`, `None` for no answer.
pub fn answer(permissions: &[String]) -> Option<bool> {
    if permissions.iter().any(|p| p == "yes") {
        Some(true)
    } else if permissions.iter().any(|p| p == "no") {
        Some(false)
    } else {
        None
    }
}

/// One entry of a table, as `Lookup` returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub table: String,
    pub id: String,
    /// App id and its permission strings, sorted by app id.
    pub apps: Vec<(String, Vec<String>)>,
    /// The screen or window an Otto restore payload names, when it names one.
    pub restored: Option<Restored>,
    /// The vendor of the restore payload, when there is one: `otto`, or the
    /// desktop whose portal wrote it (`KDE`, `GNOME`).
    pub vendor: Option<String>,
    /// The program an Otto restore payload was made for, by its executable's
    /// file name: written for unsandboxed apps, which the empty app id does
    /// not tell apart.
    pub program: Option<String>,
}

/// What an Otto ScreenCast restore payload says was shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restored {
    /// A screen, by connector name.
    Monitor(String),
    /// A window. Its id does not outlive the session, so it names nothing
    /// worth showing.
    Window,
}

/// A connection to the store.
pub struct Store {
    connection: Connection,
}

impl Store {
    /// Connect on the session bus. The store is started on demand, so a
    /// failure here means there is no session bus, not that it is not running.
    pub fn connect() -> Result<Self, String> {
        Connection::session()
            .map(|connection| Self { connection })
            .map_err(|err| format!("no session bus: {err}"))
    }

    fn call<B, R>(&self, method: &str, body: &B) -> Result<R, String>
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> zbus::export::serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        let reply = self
            .connection
            .call_method(Some(NAME), PATH, Some(INTERFACE), method, body)
            .map_err(|err| format!("{method}: {err}"))?;
        reply
            .body()
            .deserialize::<R>()
            .map_err(|err| format!("{method}: {err}"))
    }

    /// Every entry of `table`. A table that was never written is empty.
    pub fn entries(&self, table: &str) -> Result<Vec<Entry>, String> {
        let ids: Vec<String> = self.call("List", &(table,))?;
        let mut entries = Vec::new();
        for id in ids {
            // Deleted between the List and the Lookup: nothing to show.
            let Ok((apps, data)) =
                self.call::<_, (HashMap<String, Vec<String>>, OwnedValue)>("Lookup", &(table, &id))
            else {
                continue;
            };
            let mut apps: Vec<(String, Vec<String>)> = apps.into_iter().collect();
            apps.sort();
            entries.push(Entry {
                table: table.to_string(),
                vendor: restore_vendor(&data),
                restored: decode_restored(&data),
                program: restore_program(&data),
                id,
                apps,
            });
        }
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(entries)
    }

    /// Take `app`'s decision on `id` out of `table`, and the whole entry when
    /// no other app holds one: a restore token belongs to one app, and an
    /// entry with no apps left grants nothing but still lists.
    pub fn forget(&self, table: &str, id: &str, app: &str, others: bool) -> Result<(), String> {
        if others {
            self.call::<_, ()>("DeletePermission", &(table, id, app))
        } else {
            self.call::<_, ()>("Delete", &(table, id))
        }
    }

    /// Set `app`'s permission strings on `id` in `table`, creating the table
    /// if it is missing.
    pub fn set(
        &self,
        table: &str,
        id: &str,
        app: &str,
        permissions: &[&str],
    ) -> Result<(), String> {
        self.call::<_, ()>("SetPermission", &(table, true, id, app, permissions))
    }
}

/// Peel variant wrappers off a value: how deeply the payload is nested
/// depends on who marshalled it (see the portal's `restore.rs`).
fn unwrap_variants<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    let mut current = value;
    while let Value::Value(inner) = current {
        current = inner.as_ref();
    }
    current
}

/// The fields of a `(suv)` restore payload.
fn restore_fields<'a>(data: &'a Value<'a>) -> Option<&'a [Value<'a>]> {
    let Value::Structure(structure) = unwrap_variants(data) else {
        return None;
    };
    let fields = structure.fields();
    (fields.len() == 3).then_some(fields)
}

/// Who wrote a restore payload.
fn restore_vendor(data: &Value<'_>) -> Option<String> {
    let fields = restore_fields(data)?;
    match unwrap_variants(&fields[0]) {
        Value::Str(vendor) => Some(vendor.to_string()),
        _ => None,
    }
}

/// The `(suv)` payload's own fields, when Otto's portal wrote it.
fn otto_restore_dict<'a>(data: &'a Value<'a>) -> Option<&'a zbus::zvariant::Dict<'a, 'a>> {
    if restore_vendor(data)?.as_str() != "otto" {
        return None;
    }
    match unwrap_variants(&restore_fields(data)?[2]) {
        Value::Dict(dict) => Some(dict),
        _ => None,
    }
}

/// The program an Otto restore payload names, for an unsandboxed app.
pub fn restore_program(data: &Value<'_>) -> Option<String> {
    let dict = otto_restore_dict(data)?;
    match unwrap_variants(&dict.get::<_, Value<'_>>(&"program").ok()??) {
        Value::Str(program) if !program.is_empty() => Some(program.to_string()),
        _ => None,
    }
}

/// What an Otto restore payload says was shared: the `source-type` bit (1 a
/// screen, 2 a window) and the `id` beside it.
pub fn decode_restored(data: &Value<'_>) -> Option<Restored> {
    let dict = otto_restore_dict(data)?;
    let source_type = match unwrap_variants(&dict.get::<_, Value<'_>>(&"source-type").ok()??) {
        Value::U32(value) => *value,
        _ => return None,
    };
    match source_type {
        1 => match unwrap_variants(&dict.get::<_, Value<'_>>(&"id").ok()??) {
            Value::Str(id) if !id.is_empty() => Some(Restored::Monitor(id.to_string())),
            _ => None,
        },
        2 => Some(Restored::Window),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Str;

    fn payload(vendor: &'static str, source_type: u32, id: &str) -> Value<'static> {
        let mut data: HashMap<&str, Value<'_>> = HashMap::new();
        data.insert("source-type", Value::U32(source_type));
        data.insert("id", Value::Str(Str::from(id.to_string())));
        Value::from((
            Str::from_static(vendor),
            1u32,
            Value::Value(Box::new(Value::from(data))),
        ))
    }

    #[test]
    fn an_otto_payload_names_the_screen_it_shares() {
        assert_eq!(
            decode_restored(&payload("otto", 1, "eDP-1")),
            Some(Restored::Monitor("eDP-1".into()))
        );
        assert_eq!(
            decode_restored(&payload("otto", 2, "toplevel-3")),
            Some(Restored::Window)
        );
    }

    #[test]
    fn a_payload_kept_by_the_store_in_a_variant_still_reads() {
        let wrapped = Value::Value(Box::new(payload("otto", 1, "HDMI-A-1")));
        assert_eq!(
            decode_restored(&wrapped),
            Some(Restored::Monitor("HDMI-A-1".into()))
        );
    }

    #[test]
    fn an_unsandboxed_apps_payload_names_its_program() {
        assert_eq!(restore_program(&payload("otto", 1, "eDP-1")), None);
        let mut data: HashMap<&str, Value<'_>> = HashMap::new();
        data.insert("source-type", Value::U32(1));
        data.insert("id", Value::Str(Str::from_static("eDP-1")));
        data.insert("program", Value::Str(Str::from_static("obs")));
        let named = Value::from((
            Str::from_static("otto"),
            1u32,
            Value::Value(Box::new(Value::from(data))),
        ));
        assert_eq!(restore_program(&named).as_deref(), Some("obs"));
        assert_eq!(
            decode_restored(&named),
            Some(Restored::Monitor("eDP-1".into()))
        );
    }

    #[test]
    fn another_desktops_payload_names_only_its_vendor() {
        let kde = payload("KDE", 1, "eDP-1");
        assert_eq!(decode_restored(&kde), None);
        assert_eq!(restore_vendor(&kde).as_deref(), Some("KDE"));
        assert_eq!(restore_vendor(&Value::U8(0)), None);
    }
}
