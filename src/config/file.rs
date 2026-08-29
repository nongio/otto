//! Surgical edits to the writable configuration file.
//!
//! Otto's configuration is layered and hand-editable, so persisting a setting
//! may only touch that setting's key. Rewriting a whole table would materialise
//! a copy of every inherited value into the highest-priority layer, where it
//! shadows the same key in every lower one for good — see the rationale under
//! *Persistence* in `specs/settings-app.md`.
//!
//! The document is edited with `toml_edit` rather than `toml::Value` so
//! comments, key order and whitespace outside the touched key survive the
//! round trip byte for byte.

use std::path::Path;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

/// Parse `path` as an editable document. A missing file is an empty document;
/// an unparsable one is an error, because writing over it would destroy
/// whatever the user was in the middle of typing.
pub fn load_document(path: &Path) -> Result<DocumentMut, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
    };
    raw.parse::<DocumentMut>()
        .map_err(|err| format!("cannot parse {}: {err}", path.display()))
}

/// Write `doc` back to `path`, creating the parent directory if needed.
pub fn store_document(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
    }
    std::fs::write(path, doc.to_string())
        .map_err(|err| format!("cannot write {}: {err}", path.display()))
}

/// Split a dotted identifier (`dock.size`) into its segments.
fn segments(path: &str) -> Vec<&str> {
    path.split('.').filter(|s| !s.is_empty()).collect()
}

/// Set the dotted key `path` to `value`, creating the tables it lives in.
///
/// Fails when a segment on the way is occupied by something that is not a
/// table: silently replacing an array or a scalar the user wrote would lose it.
pub fn set_key(doc: &mut DocumentMut, path: &str, value: Value) -> Result<(), String> {
    let segments = segments(path);
    let Some((leaf, parents)) = segments.split_last() else {
        return Err("empty setting path".to_string());
    };

    let mut table = doc.as_table_mut();
    for segment in parents {
        let entry = table
            .entry(segment)
            .or_insert_with(|| Item::Table(Table::new()));
        table = entry
            .as_table_mut()
            .ok_or_else(|| format!("`{segment}` is not a table"))?;
    }

    match table.entry(leaf) {
        toml_edit::Entry::Occupied(mut entry) => {
            // Keep the decor (any trailing comment on the line) of the key we
            // are replacing; only the value changes.
            let decor = entry
                .get()
                .as_value()
                .map(|existing| existing.decor().clone());
            let mut value = value;
            if let Some(decor) = decor {
                *value.decor_mut() = decor;
            }
            entry.insert(Item::Value(value));
        }
        toml_edit::Entry::Vacant(entry) => {
            entry.insert(Item::Value(value));
        }
    }
    Ok(())
}

/// Remove the dotted key `path`, and any table the removal leaves empty.
///
/// Returns whether the key was there at all: resetting a setting that was never
/// overridden is not an error, it just changes nothing.
pub fn remove_key(doc: &mut DocumentMut, path: &str) -> bool {
    fn remove_in(table: &mut Table, segments: &[&str]) -> bool {
        let Some((head, rest)) = segments.split_first() else {
            return false;
        };
        if rest.is_empty() {
            return table.remove(head).is_some();
        }
        let Some(child) = table.get_mut(head).and_then(Item::as_table_mut) else {
            return false;
        };
        let removed = remove_in(child, rest);
        // A table that only existed to hold the key we just removed carries no
        // intent of its own, so it goes too.
        if removed && child.is_empty() {
            table.remove(head);
        }
        removed
    }

    remove_in(doc.as_table_mut(), &segments(path))
}

/// The value at the dotted key `path`, if the document has one.
pub fn get_key<'a>(doc: &'a DocumentMut, path: &str) -> Option<&'a Value> {
    let mut item: &Item = doc.as_item();
    for segment in segments(path) {
        item = item.as_table_like()?.get(segment)?;
    }
    item.as_value()
}

/// Convert a `toml::Value` — what the rest of the config code speaks — into the
/// editable representation.
pub fn to_edit_value(value: &toml::Value) -> Option<Value> {
    Some(match value {
        toml::Value::String(text) => Value::from(text.as_str()),
        toml::Value::Integer(number) => Value::from(*number),
        toml::Value::Float(number) => Value::from(*number),
        toml::Value::Boolean(flag) => Value::from(*flag),
        toml::Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(to_edit_value(item)?);
            }
            Value::Array(array)
        }
        toml::Value::Table(entries) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, entry) in entries {
                table.insert(key, to_edit_value(entry)?);
            }
            Value::InlineTable(table)
        }
        toml::Value::Datetime(_) => return None,
    })
}

/// Convert back, so a value read out of the writable file can be compared with
/// the value about to be written to it.
pub fn to_toml_value(value: &Value) -> Option<toml::Value> {
    Some(match value {
        Value::String(text) => toml::Value::String(text.value().clone()),
        Value::Integer(number) => toml::Value::Integer(*number.value()),
        Value::Float(number) => toml::Value::Float(*number.value()),
        Value::Boolean(flag) => toml::Value::Boolean(*flag.value()),
        Value::Array(items) => {
            toml::Value::Array(items.iter().map(to_toml_value).collect::<Option<_>>()?)
        }
        Value::InlineTable(table) => toml::Value::Table(
            table
                .iter()
                .map(|(key, value)| Some((key.to_string(), to_toml_value(value)?)))
                .collect::<Option<_>>()?,
        ),
        Value::Datetime(_) => return None,
    })
}

/// The `[[virtual_outputs]]` array, created if the file has none.
fn virtual_outputs_array(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, String> {
    let entry = doc
        .as_table_mut()
        .entry("virtual_outputs")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    entry
        .as_array_of_tables_mut()
        .ok_or_else(|| "`virtual_outputs` is not a list of tables".to_string())
}

/// Add `config` to `[[virtual_outputs]]`, replacing any entry with the same
/// name. Only the fields the compositor needs are written, so a hand-edited
/// entry keeps whatever else it carries.
pub fn upsert_virtual_output(
    doc: &mut DocumentMut,
    config: &crate::config::VirtualOutputConfig,
) -> Result<(), String> {
    let array = virtual_outputs_array(doc)?;
    let existing = array
        .iter()
        .position(|table| table.get("name").and_then(|n| n.as_str()) == Some(&config.name));

    let table = match existing {
        Some(index) => array.get_mut(index).expect("index came from the array"),
        None => {
            array.push(Table::new());
            let last = array.len() - 1;
            array.get_mut(last).expect("just pushed")
        }
    };

    table["name"] = toml_edit::value(config.name.clone());
    table["resolution"] = toml_edit::value({
        let mut resolution = toml_edit::InlineTable::new();
        resolution.insert("width", (config.resolution.width as i64).into());
        resolution.insert("height", (config.resolution.height as i64).into());
        resolution
    });
    table["refresh_hz"] = toml_edit::value(config.refresh_hz);
    table["interactive"] = toml_edit::value(config.interactive);
    if let Some(position) = config.position {
        table["position"] = toml_edit::value({
            let mut point = toml_edit::InlineTable::new();
            point.insert("x", (position.x as i64).into());
            point.insert("y", (position.y as i64).into());
            point
        });
    }
    Ok(())
}

/// Drop the `[[virtual_outputs]]` entry named `name`. Returns whether the
/// document changed.
pub fn remove_virtual_output(doc: &mut DocumentMut, name: &str) -> bool {
    let Some(array) = doc
        .as_table_mut()
        .get_mut("virtual_outputs")
        .and_then(Item::as_array_of_tables_mut)
    else {
        return false;
    };

    let before = array.len();
    array.retain(|table| table.get("name").and_then(|n| n.as_str()) != Some(name));
    if array.is_empty() {
        doc.as_table_mut().remove("virtual_outputs");
    }
    array_len_changed(before, doc)
}

/// Whether removing left the document different from how it started.
fn array_len_changed(before: usize, doc: &DocumentMut) -> bool {
    let now = doc
        .as_table()
        .get("virtual_outputs")
        .and_then(Item::as_array_of_tables)
        .map(ArrayOfTables::len)
        .unwrap_or(0);
    now != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> DocumentMut {
        raw.parse().expect("document should parse")
    }

    #[test]
    fn set_key_leaves_comments_and_neighbours_alone() {
        let mut document = doc("# my config\n[dock]\n# how big\nsize = 1.6\ngenie_scale = 0.3\n\n[input]\ntap_enabled = false\n");
        set_key(&mut document, "dock.size", Value::from(1.25)).expect("set should succeed");

        let out = document.to_string();
        assert!(out.contains("# my config"), "{out}");
        assert!(out.contains("# how big"), "{out}");
        assert!(out.contains("size = 1.25"), "{out}");
        assert!(out.contains("genie_scale = 0.3"), "{out}");
        assert!(out.contains("tap_enabled = false"), "{out}");
    }

    #[test]
    fn set_key_creates_missing_tables() {
        let mut document = doc("screen_scale = 2.0\n");
        set_key(&mut document, "dock.autohide", Value::from(true)).expect("set should succeed");

        assert_eq!(
            get_key(&document, "dock.autohide").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(document.to_string().contains("screen_scale = 2.0"));
    }

    #[test]
    fn set_key_refuses_to_overwrite_a_non_table_parent() {
        let mut document = doc("dock = 3\n");
        assert!(set_key(&mut document, "dock.size", Value::from(1.0)).is_err());
        assert_eq!(document.to_string(), "dock = 3\n");
    }

    #[test]
    fn remove_key_drops_the_table_it_empties() {
        let mut document = doc("[dock]\nsize = 1.5\n\n[input]\ntap_enabled = false\n");
        assert!(remove_key(&mut document, "dock.size"));

        let out = document.to_string();
        assert!(!out.contains("[dock]"), "{out}");
        assert!(out.contains("[input]"), "{out}");
    }

    #[test]
    fn remove_key_keeps_a_table_that_still_has_keys() {
        let mut document = doc("[dock]\nsize = 1.5\ngenie_scale = 0.3\n");
        assert!(remove_key(&mut document, "dock.size"));

        let out = document.to_string();
        assert!(out.contains("[dock]"), "{out}");
        assert!(out.contains("genie_scale = 0.3"), "{out}");
        assert!(!out.contains("size = 1.5"), "{out}");
    }

    #[test]
    fn remove_key_reports_a_key_that_was_never_there() {
        let mut document = doc("[dock]\nsize = 1.5\n");
        assert!(!remove_key(&mut document, "dock.autohide"));
        assert!(!remove_key(&mut document, "audio.sound_enabled"));
        assert!(document.to_string().contains("size = 1.5"));
    }

    fn virtual_output(name: &str, width: u32) -> crate::config::VirtualOutputConfig {
        crate::config::VirtualOutputConfig {
            name: name.to_string(),
            resolution: crate::config::DisplayResolution {
                width,
                height: 1080,
            },
            refresh_hz: 60.0,
            position: None,
            interactive: true,
            primary: false,
        }
    }

    #[test]
    fn a_virtual_output_is_appended_then_replaced_by_name() {
        let mut document = doc("screen_scale = 2.0\n");
        upsert_virtual_output(&mut document, &virtual_output("virtual-1", 1920)).unwrap();
        upsert_virtual_output(&mut document, &virtual_output("virtual-2", 1280)).unwrap();
        // Same name again: replaced in place, not appended a second time.
        upsert_virtual_output(&mut document, &virtual_output("virtual-1", 3840)).unwrap();

        let array = document["virtual_outputs"].as_array_of_tables().unwrap();
        assert_eq!(array.len(), 2);
        assert_eq!(array.get(0).unwrap()["name"].as_str(), Some("virtual-1"));
        assert_eq!(
            array.get(0).unwrap()["resolution"]["width"].as_integer(),
            Some(3840)
        );
        // The unrelated key is untouched.
        assert_eq!(document["screen_scale"].as_float(), Some(2.0));
    }

    #[test]
    fn removing_a_virtual_output_leaves_the_others_and_drops_an_empty_array() {
        let mut document = doc("");
        upsert_virtual_output(&mut document, &virtual_output("virtual-1", 1920)).unwrap();
        upsert_virtual_output(&mut document, &virtual_output("virtual-2", 1280)).unwrap();

        assert!(remove_virtual_output(&mut document, "virtual-1"));
        let array = document["virtual_outputs"].as_array_of_tables().unwrap();
        assert_eq!(array.len(), 1);
        assert_eq!(array.get(0).unwrap()["name"].as_str(), Some("virtual-2"));

        // Removing the last one takes the array with it rather than leaving an
        // empty `virtual_outputs` behind.
        assert!(remove_virtual_output(&mut document, "virtual-2"));
        assert!(document.get("virtual_outputs").is_none());

        // Removing one that was never there is not an error and changes nothing.
        assert!(!remove_virtual_output(&mut document, "virtual-9"));
    }

    #[test]
    fn get_key_reads_top_level_and_nested_keys() {
        let document = doc("screen_scale = 2.0\n[dock]\nsize = 1.5\n");
        assert_eq!(
            get_key(&document, "screen_scale").and_then(|v| v.as_float()),
            Some(2.0)
        );
        assert_eq!(
            get_key(&document, "dock.size").and_then(|v| v.as_float()),
            Some(1.5)
        );
        assert!(get_key(&document, "dock.autohide").is_none());
        assert!(get_key(&document, "nope.at.all").is_none());
    }

    #[test]
    fn dotted_keys_are_reachable_too() {
        let mut document = doc("dock.size = 1.5\n");
        assert_eq!(
            get_key(&document, "dock.size").and_then(|v| v.as_float()),
            Some(1.5)
        );
        set_key(&mut document, "dock.size", Value::from(0.75)).expect("set should succeed");
        assert_eq!(
            get_key(&document, "dock.size").and_then(|v| v.as_float()),
            Some(0.75)
        );
    }
}
