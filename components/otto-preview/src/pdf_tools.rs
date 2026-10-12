//! The PDF's form and annotations as the agent reads and changes them: JSON
//! in, edits out, so the agent fills a form through the same engine as the
//! person instead of writing the file with a script of its own.
//!
//! An agent names a field by its name, which it can read from the form,
//! rather than by where its widget is; a value says what to do with it: text
//! for a text field, true or false for a checkbox, the export value for a
//! radio group, an option's label for a list. Annotations are named
//! `page:index`, as `preview_annotations` lists them, and are good until the
//! next change to the page.

// Rust guideline compliant 2026-02-21

use otto_pdf::{
    AnnotId, AnnotKind, Edit, Field, FieldKind, Markup, NewAnnotation, Rect, Rgb, Snapshot,
};
use serde_json::{json, Value};

/// The colour markup is made in when the agent names none.
const DEFAULT_COLOR: Rgb = Rgb(1.0, 0.85, 0.1);

fn id_string(id: AnnotId) -> String {
    format!("{}:{}", id.page, id.index)
}

fn parse_id(text: &str) -> Option<AnnotId> {
    let (page, index) = text.split_once(':')?;
    Some(AnnotId {
        page: page.trim().parse().ok()?,
        index: index.trim().parse().ok()?,
    })
}

fn rect_json(rect: Rect) -> Value {
    json!([rect.left, rect.top, rect.right, rect.bottom])
}

fn color_json(color: Option<Rgb>) -> Value {
    match color {
        Some(Rgb(r, g, b)) => {
            let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            json!(format!(
                "#{:02x}{:02x}{:02x}",
                channel(r),
                channel(g),
                channel(b)
            ))
        }
        None => Value::Null,
    }
}

/// `#rrggbb` (or `rrggbb`) as a colour.
fn parse_color(text: &str) -> Option<Rgb> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| {
        u8::from_str_radix(hex.get(at..at + 2)?, 16)
            .ok()
            .map(|value| f32::from(value) / 255.0)
    };
    Some(Rgb(channel(0)?, channel(2)?, channel(4)?))
}

/// Every field, one entry per field name, with each place it shows.
pub fn fields_json(snapshot: &Snapshot) -> Value {
    let mut names: Vec<&str> = Vec::new();
    for field in &snapshot.fields {
        if !names.contains(&field.name.as_str()) {
            names.push(&field.name);
        }
    }
    let fields: Vec<Value> = names
        .into_iter()
        .map(|name| {
            let widgets: Vec<&Field> = snapshot
                .fields
                .iter()
                .filter(|field| field.name == name)
                .collect();
            let first = widgets[0];
            let mut entry = json!({
                "name": name,
                "label": first.tooltip,
                "read_only": first.read_only,
                "required": first.required,
                "places": widgets.iter().map(|field| json!({
                    "page": field.id.page,
                    "rect": rect_json(field.rect),
                })).collect::<Vec<_>>(),
            });
            let (kind, value, extra) = match &first.kind {
                FieldKind::Text {
                    value,
                    multiline,
                    max_len,
                    ..
                } => (
                    "text",
                    json!(value),
                    json!({ "multiline": multiline, "max_length": max_len }),
                ),
                FieldKind::Checkbox { checked } => ("checkbox", json!(checked), json!({})),
                FieldKind::Radio { .. } => {
                    let chosen = widgets.iter().find_map(|field| match &field.kind {
                        FieldKind::Radio {
                            checked: true,
                            export,
                        } => Some(export.clone()),
                        _ => None,
                    });
                    let options: Vec<String> = widgets
                        .iter()
                        .filter_map(|field| match &field.kind {
                            FieldKind::Radio { export, .. } => Some(export.clone()),
                            _ => None,
                        })
                        .collect();
                    ("radio", json!(chosen), json!({ "options": options }))
                }
                FieldKind::Combo {
                    options,
                    value,
                    editable,
                    ..
                } => (
                    "choice",
                    json!(value),
                    json!({ "options": options, "typed": editable }),
                ),
                FieldKind::List {
                    options,
                    selected,
                    multiple,
                } => (
                    "list",
                    json!(selected
                        .iter()
                        .filter_map(|index| options.get(*index))
                        .collect::<Vec<_>>()),
                    json!({ "options": options, "multiple": multiple }),
                ),
                FieldKind::Button => ("button", Value::Null, json!({})),
                FieldKind::Signature { signed } => ("signature", json!(signed), json!({})),
            };
            entry["type"] = json!(kind);
            entry["value"] = value;
            if let (Value::Object(entry), Value::Object(extra)) = (&mut entry, extra) {
                entry.extend(extra);
            }
            entry
        })
        .collect();
    json!({ "signed": snapshot.signed, "fields": fields })
}

/// Every annotation that isn't a form widget.
pub fn annotations_json(snapshot: &Snapshot) -> Value {
    let annotations: Vec<Value> = snapshot
        .annotations
        .iter()
        .map(|annotation| {
            let (kind, text) = match &annotation.kind {
                AnnotKind::Markup { markup, .. } => (
                    match markup {
                        Markup::Highlight => "highlight",
                        Markup::Underline => "underline",
                        Markup::StrikeOut => "strikeout",
                        Markup::Squiggly => "squiggly",
                    },
                    None,
                ),
                AnnotKind::Ink { .. } => ("ink", None),
                AnnotKind::Text { text, .. } => ("text", Some(text.clone())),
                AnnotKind::Note { text } => ("note", Some(text.clone())),
                AnnotKind::Stamp => ("stamp", None),
                AnnotKind::Other { subtype } => (subtype.as_str(), None),
            };
            json!({
                "id": id_string(annotation.id),
                "page": annotation.id.page,
                "type": kind,
                "rect": rect_json(annotation.rect),
                "color": color_json(annotation.color),
                "text": text,
                "author": annotation.author,
                "editable": annotation.editable,
            })
        })
        .collect();
    json!({ "annotations": annotations })
}

/// The edits a `preview_edit` call asks for, against the document as it is.
///
/// # Errors
///
/// A field or annotation that isn't there, a value that doesn't fit its
/// field, or an annotation described without what it needs, with what was
/// wrong.
pub fn edits_from_json(snapshot: &Snapshot, request: &Value) -> Result<Vec<Edit>, String> {
    let mut edits = Vec::new();
    if let Some(fill) = request.get("fill") {
        let fill = fill
            .as_object()
            .ok_or("fill is an object of field names and values")?;
        for (name, value) in fill {
            edits.push(fill_edit(snapshot, name, value)?);
        }
    }
    for add in list(request, "add")? {
        edits.push(add_edit(add)?);
    }
    for change in list(request, "change")? {
        let id = annotation_id(snapshot, change.get("id"))?;
        if let (Some(dx), Some(dy)) = (number(change, "dx"), number(change, "dy")) {
            edits.push(Edit::Move {
                annotation: id,
                dx,
                dy,
            });
        }
        if let Some(text) = change.get("text").and_then(Value::as_str) {
            edits.push(Edit::SetContents {
                annotation: id,
                text: text.to_owned(),
            });
        }
        if let Some(color) = change.get("color").and_then(Value::as_str) {
            let color = parse_color(color).ok_or_else(|| format!("{color} is not #rrggbb"))?;
            edits.push(Edit::SetColor {
                annotation: id,
                color,
            });
        }
    }
    let mut deletes = Vec::new();
    for delete in list(request, "delete")? {
        deletes.push(annotation_id(snapshot, Some(delete))?);
    }
    // Last on each page first, so one delete doesn't renumber the next.
    deletes.sort_by(|a, b| b.cmp(a));
    deletes.dedup();
    edits.extend(
        deletes
            .into_iter()
            .map(|annotation| Edit::Delete { annotation }),
    );
    if edits.is_empty() {
        return Err("nothing to do: give fill, add, change or delete".into());
    }
    // Numbers that are no point on a page, sizes out of reason, text too
    // long, refused before anything is made.
    for edit in &edits {
        edit.check().map_err(|err| err.to_string())?;
    }
    Ok(edits)
}

fn list<'a>(request: &'a Value, key: &str) -> Result<&'a [Value], String> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(format!("{key} is a list")),
    }
}

fn number(value: &Value, key: &str) -> Option<f32> {
    value.get(key).and_then(Value::as_f64).map(|n| n as f32)
}

fn annotation_id(snapshot: &Snapshot, id: Option<&Value>) -> Result<AnnotId, String> {
    let text = id
        .and_then(Value::as_str)
        .ok_or("an annotation is named by its id, \"page:index\"")?;
    let id = parse_id(text).ok_or_else(|| format!("{text} is not an id like \"0:3\""))?;
    let annotation = snapshot
        .annotation(id)
        .ok_or_else(|| format!("no annotation {text}; read preview_annotations again"))?;
    if !annotation.editable {
        return Err(format!("annotation {text} can't be changed"));
    }
    Ok(id)
}

fn fill_edit(snapshot: &Snapshot, name: &str, value: &Value) -> Result<Edit, String> {
    let widgets: Vec<&Field> = snapshot
        .fields
        .iter()
        .filter(|field| field.name == name)
        .collect();
    let first = widgets
        .first()
        .ok_or_else(|| format!("no field named {name}"))?;
    if first.read_only {
        return Err(format!("{name} is read-only"));
    }
    let wrong = |wanted: &str| format!("{name} takes {wanted}, not {value}");
    match &first.kind {
        FieldKind::Text { .. } => {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                _ => return Err(wrong("text")),
            };
            Ok(Edit::SetText {
                field: first.id,
                text,
            })
        }
        FieldKind::Checkbox { .. } => Ok(Edit::SetChecked {
            field: first.id,
            checked: value.as_bool().ok_or_else(|| wrong("true or false"))?,
        }),
        FieldKind::Radio { .. } => {
            let wanted = value.as_str().ok_or_else(|| wrong("one of its options"))?;
            let widget = widgets
                .iter()
                .find(|field| matches!(&field.kind, FieldKind::Radio { export, .. } if export == wanted))
                .ok_or_else(|| wrong("one of its options"))?;
            Ok(Edit::SetChecked {
                field: widget.id,
                checked: true,
            })
        }
        FieldKind::Combo {
            options, editable, ..
        } => {
            let wanted = value.as_str().ok_or_else(|| wrong("one of its options"))?;
            match options.iter().position(|option| option == wanted) {
                Some(index) => Ok(Edit::Choose {
                    field: first.id,
                    index,
                    add: false,
                }),
                None if *editable => Ok(Edit::SetText {
                    field: first.id,
                    text: wanted.to_owned(),
                }),
                None => Err(wrong("one of its options")),
            }
        }
        FieldKind::List { options, .. } => {
            let wanted = value.as_str().ok_or_else(|| wrong("one of its options"))?;
            let index = options
                .iter()
                .position(|option| option == wanted)
                .ok_or_else(|| wrong("one of its options"))?;
            Ok(Edit::Choose {
                field: first.id,
                index,
                add: false,
            })
        }
        FieldKind::Button | FieldKind::Signature { .. } => Err(format!("{name} can't be filled")),
    }
}

fn point_list(value: Option<&Value>) -> Option<Vec<(f32, f32)>> {
    value?
        .as_array()?
        .iter()
        .map(|point| {
            let point = point.as_array()?;
            Some((
                point.first()?.as_f64()? as f32,
                point.get(1)?.as_f64()? as f32,
            ))
        })
        .collect()
}

fn rect_of(value: Option<&Value>) -> Option<Rect> {
    let numbers: Vec<f32> = value?
        .as_array()?
        .iter()
        .map(|n| n.as_f64().map(|n| n as f32))
        .collect::<Option<_>>()?;
    match numbers[..] {
        [left, top, right, bottom] => Some(Rect::new(left, top, right, bottom)),
        _ => None,
    }
}

fn add_edit(add: &Value) -> Result<Edit, String> {
    let kind = add
        .get("type")
        .and_then(Value::as_str)
        .ok_or("each annotation to add has a type")?;
    let page = add
        .get("page")
        .and_then(Value::as_u64)
        .ok_or("each annotation to add has a 0-based page")? as usize;
    let color = match add.get("color").and_then(Value::as_str) {
        Some(color) => parse_color(color).ok_or_else(|| format!("{color} is not #rrggbb"))?,
        None if kind == "highlight" => DEFAULT_COLOR,
        None if kind == "note" => DEFAULT_COLOR,
        None => Rgb(0.9, 0.18, 0.15),
    };
    let markup = match kind {
        "highlight" => Some(Markup::Highlight),
        "underline" => Some(Markup::Underline),
        "strikeout" => Some(Markup::StrikeOut),
        "squiggly" => Some(Markup::Squiggly),
        _ => None,
    };
    let annotation = if let Some(markup) = markup {
        // One rect per line of text.
        let rects: Vec<Rect> =
            match add.get("rects").and_then(Value::as_array) {
                Some(rects) => rects
                    .iter()
                    .map(|rect| rect_of(Some(rect)))
                    .collect::<Option<_>>()
                    .ok_or("rects are [left, top, right, bottom] each")?,
                None => vec![rect_of(add.get("rect"))
                    .ok_or("markup takes rect, or rects for several lines")?],
            };
        NewAnnotation::Markup {
            markup,
            quads: rects
                .into_iter()
                .map(|r| {
                    [
                        (r.left, r.top),
                        (r.right, r.top),
                        (r.left, r.bottom),
                        (r.right, r.bottom),
                    ]
                })
                .collect(),
            color,
        }
    } else {
        match kind {
            "ink" => {
                let strokes = add
                    .get("strokes")
                    .and_then(Value::as_array)
                    .ok_or("ink takes strokes: [[[x, y], ...], ...]")?
                    .iter()
                    .map(|stroke| point_list(Some(stroke)))
                    .collect::<Option<Vec<_>>>()
                    .ok_or("ink takes strokes: [[[x, y], ...], ...]")?;
                NewAnnotation::Ink {
                    strokes,
                    width: number(add, "width").unwrap_or(1.5),
                    color,
                }
            }
            "text" | "note" => {
                let text = add
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("text and notes take text")?
                    .to_owned();
                let at = point_list(Some(&json!([add.get("at").cloned().unwrap_or_default()])))
                    .and_then(|points| points.first().copied())
                    .ok_or("text and notes take at: [x, y]")?;
                if kind == "text" {
                    NewAnnotation::Text {
                        at,
                        text,
                        size: number(add, "size").unwrap_or(12.0),
                        color: add
                            .get("color")
                            .and_then(Value::as_str)
                            .and_then(parse_color)
                            .unwrap_or(Rgb(0.08, 0.08, 0.1)),
                    }
                } else {
                    NewAnnotation::Note { at, text, color }
                }
            }
            other => {
                return Err(format!(
                    "{other} can't be added: highlight, underline, strikeout, squiggly, ink, text or note"
                ))
            }
        }
    };
    Ok(Edit::Add { page, annotation })
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_pdf::{Annotation, Page};

    fn snapshot() -> Snapshot {
        let field = |page, index, name: &str, kind| Field {
            id: AnnotId { page, index },
            name: name.to_owned(),
            tooltip: None,
            rect: Rect::new(10.0, 10.0, 60.0, 22.0),
            kind,
            read_only: false,
            required: false,
            font_size: 0.0,
        };
        let radio = |export: &str, checked| FieldKind::Radio {
            checked,
            export: export.into(),
        };
        Snapshot {
            pages: vec![
                Page {
                    width: 595.0,
                    height: 842.0
                };
                2
            ],
            fields: vec![
                field(
                    0,
                    0,
                    "name",
                    FieldKind::Text {
                        value: String::new(),
                        multiline: false,
                        password: false,
                        max_len: None,
                        comb: false,
                    },
                ),
                field(0, 1, "title", radio("Mr", true)),
                field(0, 2, "title", radio("Ms", false)),
                field(1, 0, "agree", FieldKind::Checkbox { checked: false }),
                field(
                    1,
                    1,
                    "country",
                    FieldKind::Combo {
                        options: vec!["Italy".into(), "Spain".into()],
                        selected: None,
                        value: String::new(),
                        editable: false,
                    },
                ),
            ],
            annotations: vec![Annotation {
                id: AnnotId { page: 1, index: 2 },
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                kind: AnnotKind::Ink {
                    strokes: Vec::new(),
                    width: 1.0,
                },
                color: None,
                author: None,
                editable: true,
            }],
            signed: false,
        }
    }

    #[test]
    fn fields_are_listed_once_per_name_with_every_place() {
        let fields = fields_json(&snapshot());
        let fields = fields["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 4);
        let title = &fields[1];
        assert_eq!(title["type"], "radio");
        assert_eq!(title["value"], "Mr");
        assert_eq!(title["options"], json!(["Mr", "Ms"]));
        assert_eq!(title["places"].as_array().unwrap().len(), 2);
        assert_eq!(fields[3]["options"], json!(["Italy", "Spain"]));
    }

    #[test]
    fn a_fill_names_fields_and_their_values() {
        let edits = edits_from_json(
            &snapshot(),
            &json!({ "fill": { "name": "Mario", "title": "Ms", "agree": true, "country": "Spain" } }),
        )
        .unwrap();
        assert!(edits.contains(&Edit::SetText {
            field: AnnotId { page: 0, index: 0 },
            text: "Mario".into()
        }));
        // The radio button whose export value is the one asked for.
        assert!(edits.contains(&Edit::SetChecked {
            field: AnnotId { page: 0, index: 2 },
            checked: true
        }));
        assert!(edits.contains(&Edit::Choose {
            field: AnnotId { page: 1, index: 1 },
            index: 1,
            add: false
        }));
    }

    #[test]
    fn a_wrong_value_or_name_says_what_is_wrong() {
        let err = edits_from_json(&snapshot(), &json!({ "fill": { "agree": "yes" } })).unwrap_err();
        assert!(err.contains("true or false"), "{err}");
        let err = edits_from_json(&snapshot(), &json!({ "fill": { "nope": 1 } })).unwrap_err();
        assert!(err.contains("no field"), "{err}");
        let err = edits_from_json(&snapshot(), &json!({ "delete": ["0:9"] })).unwrap_err();
        assert!(err.contains("no annotation"), "{err}");
        // Numbers no page holds, which as f32 are infinite or absurd.
        let err = edits_from_json(
            &snapshot(),
            &json!({ "change": [{ "id": "1:2", "dx": 1e300, "dy": 0 }] }),
        )
        .unwrap_err();
        assert!(err.contains("a move"), "{err}");
        let err = edits_from_json(
            &snapshot(),
            &json!({ "add": [{ "type": "ink", "page": 0, "strokes": [[[0, 0], [1, 1]]], "width": -2 }] }),
        )
        .unwrap_err();
        assert!(err.contains("width"), "{err}");
        let err = edits_from_json(
            &snapshot(),
            &json!({ "add": [{ "type": "text", "page": 0, "at": [10, 1e7], "text": "x" }] }),
        )
        .unwrap_err();
        assert!(err.contains("text"), "{err}");
    }

    #[test]
    fn markup_takes_a_rect_per_line_and_deletes_go_last_first() {
        let edits = edits_from_json(
            &snapshot(),
            &json!({
                "add": [{ "type": "highlight", "page": 0, "rects": [[10, 20, 100, 32], [10, 34, 80, 46]] }],
                "delete": ["1:2"],
            }),
        )
        .unwrap();
        let Edit::Add {
            page: 0,
            annotation: NewAnnotation::Markup { quads, color, .. },
        } = &edits[0]
        else {
            panic!("{edits:?}");
        };
        assert_eq!(quads.len(), 2);
        assert_eq!(quads[0][0], (10.0, 20.0));
        assert_eq!(quads[0][3], (100.0, 32.0));
        assert_eq!(*color, DEFAULT_COLOR);
        assert_eq!(
            edits[1],
            Edit::Delete {
                annotation: AnnotId { page: 1, index: 2 }
            }
        );
    }

    #[test]
    fn colours_read_and_write_as_hex() {
        assert_eq!(parse_color("#ff0000"), Some(Rgb(1.0, 0.0, 0.0)));
        assert_eq!(parse_color("nope"), None);
        assert_eq!(color_json(Some(Rgb(0.0, 1.0, 0.0))), json!("#00ff00"));
    }
}
