//! DBusMenu client for tray icon context menus.
//!
//! Implements the com.canonical.dbusmenu protocol used by Ayatana/libappindicator
//! tray items that don't support the SNI ContextMenu method directly.

use std::collections::HashMap;
use std::convert::TryFrom;

use zbus::zvariant::{OwnedValue, Value};
use zbus::{proxy, Connection};

// ---------------------------------------------------------------------------
// DBusMenu D-Bus proxy
// ---------------------------------------------------------------------------

#[proxy(interface = "com.canonical.dbusmenu")]
#[allow(clippy::type_complexity)]
trait DBusMenu {
    /// Check if a menu item is about to show (allows app to update it).
    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;

    /// Send an event to a menu item (e.g. "clicked").
    fn event(&self, id: i32, event_id: &str, data: &Value<'_>, timestamp: u32) -> zbus::Result<()>;

    /// Get the menu layout tree.
    /// Returns (revision, layout) where layout is (id, properties, children).
    #[allow(clippy::type_complexity)]
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: &[&str],
    ) -> zbus::Result<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))>;

    #[zbus(signal)]
    fn layout_updated(&self, revision: u32, parent: i32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn items_properties_updated(
        &self,
        updated_props: Vec<(i32, HashMap<String, OwnedValue>)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;
}

// ---------------------------------------------------------------------------
// Menu data model
// ---------------------------------------------------------------------------

/// A single menu item parsed from the dbusmenu layout.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct MenuItem {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub icon_name: Option<String>,
    /// Raw ARGB32 pixmap from `icon-data` property: (width, height, bytes)
    pub icon_data: Option<(i32, i32, Vec<u8>)>,
    pub item_type: MenuItemType,
    pub toggle_type: ToggleType,
    pub toggle_state: i32,
    pub children: Vec<MenuItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuItemType {
    Standard,
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToggleType {
    None,
    Checkmark,
    Radio,
}

/// The full menu tree fetched from a dbusmenu service.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct MenuLayout {
    pub revision: u32,
    pub items: Vec<MenuItem>,
}

// ---------------------------------------------------------------------------
// Fetch menu layout
// ---------------------------------------------------------------------------

/// Fetch the menu layout from a dbusmenu service.
pub async fn fetch_menu(
    conn: &Connection,
    service: &str,
    menu_path: &str,
) -> Result<MenuLayout, Box<dyn std::error::Error + Send + Sync>> {
    let proxy = DBusMenuProxy::builder(conn)
        .destination(service)?
        .path(menu_path)?
        .build()
        .await?;

    // Notify the app the root menu is about to show
    let _ = proxy.about_to_show(0).await;

    let (revision, layout) = proxy.get_layout(0, -1, &[]).await?;

    let items = parse_children(&layout.2);

    Ok(MenuLayout { revision, items })
}

/// Activate a menu item by sending a "clicked" event.
///
/// Because some apps (e.g. nm-applet) regenerate their menu tree frequently,
/// the `item_id` from a previous `get_layout` may already be stale.  We
/// therefore re-fetch the layout with fresh IDs and match by `item_label`.
/// The original `item_id` is used only as a fast-path shortcut.
pub async fn activate_menu_item(
    conn: &Connection,
    service: &str,
    menu_path: &str,
    item_id: i32,
    item_label: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let proxy = DBusMenuProxy::builder(conn)
        .destination(service)?
        .path(menu_path)?
        .build()
        .await?;

    // Re-fetch the layout to obtain current (valid) IDs.
    let _ = proxy.about_to_show(0).await;
    let (_revision, layout) = proxy.get_layout(0, -1, &[]).await?;
    let fresh_items = parse_children(&layout.2);

    // Resolve the item: try matching by original ID first, then by label.
    let fresh_id = find_item_id(&fresh_items, item_id, item_label);

    let target_id = fresh_id.unwrap_or(item_id);

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;

    proxy
        .event(target_id, "clicked", &Value::I32(0), timestamp)
        .await?;

    Ok(())
}

/// Find the current ID for a menu item, searching recursively.
/// Prefers an exact ID match; falls back to label match.
fn find_item_id(items: &[MenuItem], original_id: i32, label: &str) -> Option<i32> {
    // First pass: exact ID match (fast path — IDs haven't changed)
    if let Some(id) = find_by_id(items, original_id) {
        return Some(id);
    }
    // Second pass: match by label
    find_by_label(items, label)
}

fn find_by_id(items: &[MenuItem], target_id: i32) -> Option<i32> {
    for item in items {
        if item.id == target_id {
            return Some(item.id);
        }
        if let Some(id) = find_by_id(&item.children, target_id) {
            return Some(id);
        }
    }
    None
}

fn find_by_label(items: &[MenuItem], label: &str) -> Option<i32> {
    for item in items {
        if item.label == label {
            return Some(item.id);
        }
        if let Some(id) = find_by_label(&item.children, label) {
            return Some(id);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

fn parse_children(children: &[OwnedValue]) -> Vec<MenuItem> {
    let mut items = Vec::new();

    for child in children {
        if let Some(item) = parse_menu_item(child) {
            items.push(item);
        }
    }

    items
}

fn parse_menu_item(value: &OwnedValue) -> Option<MenuItem> {
    // Each child is a variant wrapping (id: i32, props: a{sv}, children: av)

    let v: Value<'_> = Value::try_from(value).ok()?;

    // Unwrap variant wrappers
    let inner = match v {
        Value::Value(boxed) => *boxed,
        other => other,
    };

    let structure = match inner {
        Value::Structure(s) => s,
        _ => return None,
    };

    let fields = structure.into_fields();
    if fields.len() < 3 {
        return None;
    }

    let id = match &fields[0] {
        Value::I32(id) => *id,
        _ => return None,
    };

    // Parse properties dict manually
    let props = extract_props(&fields[1]);

    let children_val = match &fields[2] {
        Value::Array(arr) => {
            let mut items = Vec::new();
            for v in arr.iter() {
                let owned = OwnedValue::try_from(v).ok();
                if let Some(owned) = owned {
                    if let Some(item) = parse_menu_item(&owned) {
                        items.push(item);
                    }
                }
            }
            items
        }
        _ => Vec::new(),
    };

    let label = prop_string(&props, "label").unwrap_or_default();
    let enabled = prop_bool(&props, "enabled").unwrap_or(true);
    let visible = prop_bool(&props, "visible").unwrap_or(true);
    let icon_name = prop_string(&props, "icon-name");
    let icon_data = prop_icon_data(&props, "icon-data");

    let type_str = prop_string(&props, "type").unwrap_or_default();
    let toggle_type_str = prop_string(&props, "toggle-type").unwrap_or_default();
    let toggle_state = prop_i32(&props, "toggle-state").unwrap_or(-1);

    let item_type = if type_str == "separator" {
        MenuItemType::Separator
    } else {
        MenuItemType::Standard
    };

    let toggle_type = match toggle_type_str.as_str() {
        "checkmark" => ToggleType::Checkmark,
        "radio" => ToggleType::Radio,
        _ => ToggleType::None,
    };

    Some(MenuItem {
        id,
        label,
        enabled,
        visible,
        icon_name,
        icon_data,
        item_type,
        toggle_type,
        toggle_state,
        children: children_val,
    })
}

fn extract_props(value: &Value<'_>) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();

    // The properties dict may be wrapped in a Variant
    let dict_value = match value {
        Value::Value(boxed) => boxed.as_ref(),
        other => other,
    };

    if let Value::Dict(dict) = dict_value {
        for entry in dict.iter() {
            if let (Value::Str(k), v) = entry {
                if let Ok(owned) = OwnedValue::try_from(v) {
                    map.insert(k.to_string(), owned);
                }
            }
        }
    }
    map
}

fn prop_string(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let val = props.get(key)?;
    let v: Value<'_> = Value::try_from(val).ok()?;
    match v {
        Value::Str(s) => Some(s.to_string()),
        Value::Value(boxed) => match *boxed {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    let val = props.get(key)?;
    let v: Value<'_> = Value::try_from(val).ok()?;
    match v {
        Value::Bool(b) => Some(b),
        _ => None,
    }
}

fn prop_i32(props: &HashMap<String, OwnedValue>, key: &str) -> Option<i32> {
    let val = props.get(key)?;
    let v: Value<'_> = Value::try_from(val).ok()?;
    match v {
        Value::I32(i) => Some(i),
        _ => None,
    }
}

/// Parse `icon-data` property: `a(iiay)` — array of (width, height, ARGB32 bytes).
/// Returns the first (and usually only) entry if present.
fn prop_icon_data(props: &HashMap<String, OwnedValue>, key: &str) -> Option<(i32, i32, Vec<u8>)> {
    let val = props.get(key)?;
    let v: Value<'_> = Value::try_from(val).ok()?;

    // Unwrap outer variant if wrapped
    let v = match v {
        Value::Value(boxed) => *boxed,
        other => other,
    };

    // Expect Array of Struct
    if let Value::Array(arr) = v {
        for item in arr.iter() {
            if let Value::Structure(s) = item {
                let fields = s.fields();
                if fields.len() < 3 {
                    continue;
                }
                let w = match &fields[0] {
                    Value::I32(n) => *n,
                    _ => continue,
                };
                let h = match &fields[1] {
                    Value::I32(n) => *n,
                    _ => continue,
                };
                let data: Vec<u8> = match &fields[2] {
                    Value::Array(bytes) => bytes
                        .iter()
                        .filter_map(|b| {
                            if let Value::U8(byte) = b {
                                Some(*byte)
                            } else {
                                None
                            }
                        })
                        .collect(),
                    _ => continue,
                };
                if !data.is_empty() {
                    return Some((w, h, data));
                }
            }
        }
    }
    None
}
