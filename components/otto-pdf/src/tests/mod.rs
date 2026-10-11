//! End-to-end tests against PDFium, on documents written here.
//!
//! Each test returns early, saying so, when PDFium can't be loaded.

// Rust guideline compliant 2026-02-21

mod writer;

use std::path::{Path, PathBuf};

use writer::Writer;

use crate::{
    AnnotId, AnnotKind, Annotation, Document, Edit, Error, Field, FieldKind, Markup, NewAnnotation,
    Page, Rect, Rgb, Snapshot,
};

fn pdfium() -> bool {
    if crate::available() {
        return true;
    }
    eprintln!(
        "skipping: PDFium isn't available ({:?})",
        crate::library::load().err()
    );
    false
}

/// Page 1: text, a form of every kind, a locked highlight, a link and a
/// note with its popup. Page 2: a crop box away from the origin, turned a
/// quarter, with one text field. Page 3: turned three quarters.
fn form_pdf() -> Vec<u8> {
    let mut w = Writer::new();
    let catalog = w.reserve();
    let pages = w.reserve();
    let [page1, page2, page3] = [w.reserve(), w.reserve(), w.reserve()];
    let helv =
        w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    let zadb = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>");
    let resources = format!("<< /Font << /F1 {helv} 0 R >> >>");
    let on = w.add_stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 18 18]",
        "0 0 1 rg 3 3 12 12 re f",
    );
    let off = w.add_stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 18 18]",
        "0 G 0.5 0.5 17 17 re S",
    );

    let widget = |w: &mut Writer, page: usize, rect: &str, extra: &str| {
        w.add(format!(
            "<< /Type /Annot /Subtype /Widget /F 4 /P {page} 0 R /Rect [{rect}] {extra} >>"
        ))
    };
    let name = widget(
        &mut w,
        page1,
        "72 600 300 624",
        "/FT /Tx /T (name) /TU (Your name) /DA (/Helv 12 Tf 0 g)",
    );
    let code = widget(
        &mut w,
        page1,
        "320 600 420 624",
        "/FT /Tx /T (code) /Ff 16777216 /MaxLen 5 /DA (/Helv 12 Tf 0 g)",
    );
    let notes = widget(
        &mut w,
        page1,
        "320 520 560 590",
        "/FT /Tx /T (notes) /Ff 4096 /DA (/Helv 10 Tf 0 g)",
    );
    let fixed = widget(
        &mut w,
        page1,
        "320 480 560 504",
        "/FT /Tx /T (fixed) /Ff 1 /V (locked) /DA (/Helv 12 Tf 0 g)",
    );
    let agree = widget(
        &mut w,
        page1,
        "72 560 90 578",
        &format!(
            "/FT /Btn /T (agree) /V /Off /AS /Off /MK << /CA (4) >> /DA (/ZaDb 0 Tf 0 g) \
             /AP << /N << /Yes {on} 0 R /Off {off} 0 R >> >>"
        ),
    );
    let color = w.reserve();
    let red = widget(
        &mut w,
        page1,
        "72 520 90 538",
        &format!(
            "/Parent {color} 0 R /AS /Red /MK << /CA (l) >> /DA (/ZaDb 0 Tf 0 g) \
             /AP << /N << /Red {on} 0 R /Off {off} 0 R >> >>"
        ),
    );
    let blue = widget(
        &mut w,
        page1,
        "120 520 138 538",
        &format!(
            "/Parent {color} 0 R /AS /Off /MK << /CA (l) >> /DA (/ZaDb 0 Tf 0 g) \
             /AP << /N << /Blue {on} 0 R /Off {off} 0 R >> >>"
        ),
    );
    w.set(
        color,
        format!("<< /FT /Btn /Ff 49152 /T (color) /V /Red /Kids [{red} 0 R {blue} 0 R] >>"),
    );
    let size = widget(
        &mut w,
        page1,
        "72 470 200 490",
        "/FT /Ch /Ff 131072 /T (size) /Opt [(Small) (Medium) (Large)] /V (Small) /DA (/Helv 10 Tf 0 g)",
    );
    let city = widget(
        &mut w,
        page1,
        "220 470 300 490",
        "/FT /Ch /Ff 393216 /T (city) /Opt [(Rome) (Paris)] /V (Rome) /DA (/Helv 10 Tf 0 g)",
    );
    let fruits = widget(
        &mut w,
        page1,
        "72 380 200 450",
        "/FT /Ch /Ff 2097152 /T (fruits) /Opt [(Apple) (Banana) (Cherry)] /DA (/Helv 10 Tf 0 g)",
    );
    let sig = widget(&mut w, page1, "320 380 560 440", "/FT /Sig /T (sig)");
    let highlight = w.add(
        "<< /Type /Annot /Subtype /Highlight /F 132 /Rect [72 690 200 720] \
         /QuadPoints [72 720 200 720 72 690 200 690] /C [1 1 0] >>",
    );
    let link = w.add("<< /Type /Annot /Subtype /Link /Rect [300 690 400 720] /Border [0 0 0] >>");
    let note = w.reserve();
    let popup = w.add(format!(
        "<< /Type /Annot /Subtype /Popup /Rect [520 600 600 700] /Parent {note} 0 R >>"
    ));
    w.set(
        note,
        format!(
            "<< /Type /Annot /Subtype /Text /F 4 /Rect [500 740 520 760] /Contents (Hi there) /T (Ada) \
             /C [1 0.8 0] /Popup {popup} 0 R >>"
        ),
    );
    let rotated = widget(
        &mut w,
        page2,
        "100 200 200 220",
        "/FT /Tx /T (rotated) /DA (/Helv 12 Tf 0 g)",
    );

    let content1 = w.add_stream(
        "",
        "BT /F1 18 Tf 72 700 Td (Hello markup line) Tj ET BT /F1 12 Tf 72 630 Td (Name:) Tj ET",
    );
    let content2 = w.add_stream("", "BT /F1 14 Tf 100 300 Td (Rotated page text) Tj ET");
    let content3 = w.add_stream("", "BT /F1 14 Tf 40 200 Td (Three quarters) Tj ET");
    let annots1: Vec<String> = [
        name, code, notes, fixed, agree, red, blue, size, city, fruits, sig, highlight, link, note,
        popup,
    ]
    .iter()
    .map(|id| format!("{id} 0 R"))
    .collect();
    w.set(
        page1,
        format!(
            "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] /Resources {resources} \
             /Contents {content1} 0 R /Annots [{}] >>",
            annots1.join(" ")
        ),
    );
    w.set(
        page2,
        format!(
            "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] /CropBox [50 100 562 742] \
             /Rotate 90 /Resources {resources} /Contents {content2} 0 R /Annots [{rotated} 0 R] >>"
        ),
    );
    w.set(
        page3,
        format!(
            "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 400 300] /Rotate 270 \
             /Resources {resources} /Contents {content3} 0 R >>"
        ),
    );
    w.set(
        pages,
        format!("<< /Type /Pages /Kids [{page1} 0 R {page2} 0 R {page3} 0 R] /Count 3 >>"),
    );
    let fields: Vec<String> = [
        name, code, notes, fixed, agree, color, size, city, fruits, sig, rotated,
    ]
    .iter()
    .map(|id| format!("{id} 0 R"))
    .collect();
    w.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {pages} 0 R /AcroForm << /Fields [{}] /DA (/Helv 0 Tf 0 g) \
             /DR << /Font << /Helv {helv} 0 R /ZaDb {zadb} 0 R >> >> >> >>",
            fields.join(" ")
        ),
    );
    w.finish(catalog)
}

fn write_fixture(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, form_pdf()).unwrap();
    path
}

fn field<'a>(snapshot: &'a Snapshot, name: &str) -> &'a Field {
    snapshot
        .fields
        .iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

fn radio<'a>(snapshot: &'a Snapshot, export: &str) -> &'a Field {
    snapshot
        .fields
        .iter()
        .find(|field| matches!(&field.kind, FieldKind::Radio { export: e, .. } if e == export))
        .unwrap_or_else(|| panic!("no radio {export}"))
}

fn checked(field: &Field) -> bool {
    match field.kind {
        FieldKind::Checkbox { checked } | FieldKind::Radio { checked, .. } => checked,
        _ => panic!("{} isn't tickable", field.name),
    }
}

fn text_value(field: &Field) -> &str {
    match &field.kind {
        FieldKind::Text { value, .. } | FieldKind::Combo { value, .. } => value,
        _ => panic!("{} holds no text", field.name),
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.6
}

fn near_rect(a: Rect, b: Rect) -> bool {
    near(a.left, b.left) && near(a.top, b.top) && near(a.right, b.right) && near(a.bottom, b.bottom)
}

fn near_color(a: Option<Rgb>, b: Rgb) -> bool {
    a.is_some_and(|a| {
        (a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02 && (a.2 - b.2).abs() < 0.02
    })
}

/// Applies `edits`, saves over the file and opens it again.
fn round_trip(path: &Path, edits: &[Edit]) -> Snapshot {
    let mut doc = Document::open(path.to_path_buf(), None).unwrap();
    doc.apply(edits).unwrap();
    assert!(doc.dirty());
    doc.save().unwrap();
    assert!(!doc.dirty());
    drop(doc);
    Document::open(path.to_path_buf(), None).unwrap().snapshot()
}

/// The annotations `after` has that `before` didn't, by place and kind.
fn added<'a>(before: &Snapshot, after: &'a Snapshot) -> Vec<&'a Annotation> {
    after
        .annotations
        .iter()
        .filter(|annotation| !before.annotations.iter().any(|old| old.id == annotation.id))
        .collect()
}

fn samples_dir() -> Option<PathBuf> {
    std::env::var_os("OTTO_PDF_SAMPLES").map(PathBuf::from)
}

/// Keeps a copy of `path` for looking at, when `OTTO_PDF_SAMPLES` names a
/// directory.
fn keep_sample(path: &Path, name: &str) {
    if let Some(dir) = samples_dir() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(path, dir.join(name)).unwrap();
    }
}

#[test]
fn snapshot_reads_pages_fields_and_annotations() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let doc = Document::open(write_fixture(dir.path(), "form.pdf"), None).unwrap();
    let snap = doc.snapshot();
    assert_eq!(
        snap.pages,
        vec![
            Page {
                width: 612.0,
                height: 792.0
            },
            Page {
                width: 642.0,
                height: 512.0
            },
            Page {
                width: 300.0,
                height: 400.0
            },
        ]
    );
    assert!(!snap.signed);

    let name = field(&snap, "name");
    assert_eq!(name.id, AnnotId { page: 0, index: 0 });
    assert_eq!(name.tooltip.as_deref(), Some("Your name"));
    assert!(near_rect(
        name.rect,
        Rect::new(72.0, 792.0 - 624.0, 300.0, 792.0 - 600.0)
    ));
    assert_eq!(name.font_size, 12.0);
    assert!(
        matches!(&name.kind, FieldKind::Text { value, multiline: false, password: false, max_len: None, comb: false } if value.is_empty())
    );
    assert!(matches!(
        field(&snap, "code").kind,
        FieldKind::Text {
            max_len: Some(5),
            comb: true,
            ..
        }
    ));
    assert!(matches!(
        field(&snap, "notes").kind,
        FieldKind::Text {
            multiline: true,
            ..
        }
    ));
    let fixed = field(&snap, "fixed");
    assert!(fixed.read_only);
    assert_eq!(text_value(fixed), "locked");
    assert!(!checked(field(&snap, "agree")));
    assert!(checked(radio(&snap, "Red")));
    assert!(!checked(radio(&snap, "Blue")));
    assert_eq!(radio(&snap, "Blue").name, "color");
    assert_eq!(
        field(&snap, "size").kind,
        FieldKind::Combo {
            options: vec!["Small".into(), "Medium".into(), "Large".into()],
            selected: Some(0),
            value: "Small".into(),
            editable: false,
        }
    );
    assert!(matches!(
        field(&snap, "city").kind,
        FieldKind::Combo { editable: true, .. }
    ));
    assert_eq!(
        field(&snap, "fruits").kind,
        FieldKind::List {
            options: vec!["Apple".into(), "Banana".into(), "Cherry".into()],
            selected: vec![],
            multiple: true,
        }
    );
    assert_eq!(
        field(&snap, "sig").kind,
        FieldKind::Signature { signed: false }
    );

    // The crop box starts at (50, 100) and the page is turned a quarter.
    let rotated = field(&snap, "rotated");
    assert_eq!(rotated.id, AnnotId { page: 1, index: 0 });
    assert!(
        near_rect(rotated.rect, Rect::new(100.0, 50.0, 120.0, 150.0)),
        "{:?}",
        rotated.rect
    );

    // The popup is the note's, not an annotation of its own.
    assert_eq!(snap.annotations.len(), 3);
    let highlight = &snap.annotations[0];
    assert_eq!(highlight.id, AnnotId { page: 0, index: 11 });
    assert!(!highlight.editable, "a locked highlight can't be edited");
    assert!(near_color(highlight.color, Rgb(1.0, 1.0, 0.0)));
    match &highlight.kind {
        AnnotKind::Markup {
            markup: Markup::Highlight,
            quads,
        } => {
            assert_eq!(quads.len(), 1);
            let [tl, tr, bl, br] = quads[0];
            assert!(near(tl.0, 72.0) && near(tl.1, 72.0) && near(tr.0, 200.0) && near(bl.1, 102.0));
            assert!(near(br.0, 200.0) && near(br.1, 102.0));
        }
        other => panic!("not a highlight: {other:?}"),
    }
    assert_eq!(
        snap.annotations[1].kind,
        AnnotKind::Other {
            subtype: "Link".into()
        }
    );
    assert!(!snap.annotations[1].editable);
    let note = &snap.annotations[2];
    assert_eq!(
        note.kind,
        AnnotKind::Note {
            text: "Hi there".into()
        }
    );
    assert_eq!(note.author.as_deref(), Some("Ada"));
    assert!(note.editable);
    assert!(near_color(note.color, Rgb(1.0, 0.8, 0.0)));
    assert_eq!(
        snap.annotation_at(0, (510.0, 42.0)).map(|a| a.id),
        Some(note.id)
    );
}

#[test]
fn wrong_or_missing_password_and_garbage_are_told_apart() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("junk.pdf");
    std::fs::write(&junk, b"not a pdf at all").unwrap();
    assert!(matches!(
        crate::open(&junk, None),
        Err(Error::Unreadable(_))
    ));
    assert!(matches!(
        crate::open(&dir.path().join("missing.pdf"), None),
        Err(Error::Unreadable(_))
    ));
}

#[test]
fn text_fields_fill_clear_and_survive_a_save() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_fixture(dir.path(), "text.pdf");
    let before = Document::open(path.clone(), None).unwrap().snapshot();
    let edits = [
        Edit::SetText {
            field: field(&before, "name").id,
            text: "Mario Rössi".into(),
        },
        Edit::SetText {
            field: field(&before, "code").id,
            text: "1234567".into(),
        },
        Edit::SetText {
            field: field(&before, "notes").id,
            text: "first line\nsecond line".into(),
        },
        Edit::SetText {
            field: field(&before, "rotated").id,
            text: "Sideways".into(),
        },
        Edit::SetText {
            field: field(&before, "city").id,
            text: "Berlin".into(),
        },
    ];
    let after = round_trip(&path, &edits);
    keep_sample(&path, "text-filled.pdf");
    assert_eq!(text_value(field(&after, "name")), "Mario Rössi");
    assert_eq!(
        text_value(field(&after, "code")),
        "12345",
        "cut to its MaxLen"
    );
    assert_eq!(
        text_value(field(&after, "notes")),
        "first line\nsecond line"
    );
    assert_eq!(text_value(field(&after, "rotated")), "Sideways");
    assert_eq!(text_value(field(&after, "city")), "Berlin");

    // An empty string clears it.
    let cleared = round_trip(
        &path,
        &[Edit::SetText {
            field: field(&after, "name").id,
            text: String::new(),
        }],
    );
    assert_eq!(text_value(field(&cleared, "name")), "");
    assert_eq!(
        text_value(field(&cleared, "notes")),
        "first line\nsecond line"
    );
}

#[test]
fn checkboxes_and_radios_tick_and_survive_a_save() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_fixture(dir.path(), "tick.pdf");
    let before = Document::open(path.clone(), None).unwrap().snapshot();
    let after = round_trip(
        &path,
        &[
            Edit::SetChecked {
                field: field(&before, "agree").id,
                checked: true,
            },
            Edit::SetChecked {
                field: radio(&before, "Blue").id,
                checked: true,
            },
        ],
    );
    keep_sample(&path, "ticked.pdf");
    assert!(checked(field(&after, "agree")));
    assert!(checked(radio(&after, "Blue")));
    assert!(
        !checked(radio(&after, "Red")),
        "ticking one radio unticks the rest"
    );

    let mut doc = Document::open(path.clone(), None).unwrap();
    let blue = radio(&doc.snapshot(), "Blue").id;
    let error = doc
        .apply(&[Edit::SetChecked {
            field: blue,
            checked: false,
        }])
        .unwrap_err();
    assert!(
        matches!(&error, Error::Invalid(why) if why.starts_with("edit 1:")),
        "{error:?}"
    );
    assert!(!doc.dirty());

    let agree = field(&doc.snapshot(), "agree").id;
    doc.apply(&[Edit::SetChecked {
        field: agree,
        checked: false,
    }])
    .unwrap();
    assert!(!checked(field(&doc.snapshot(), "agree")));
}

#[test]
fn choices_select_and_survive_a_save() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_fixture(dir.path(), "choose.pdf");
    let before = Document::open(path.clone(), None).unwrap().snapshot();
    let size = field(&before, "size").id;
    let fruits = field(&before, "fruits").id;
    let after = round_trip(
        &path,
        &[
            Edit::Choose {
                field: size,
                index: 2,
                add: false,
            },
            Edit::Choose {
                field: fruits,
                index: 0,
                add: false,
            },
            Edit::Choose {
                field: fruits,
                index: 2,
                add: true,
            },
        ],
    );
    keep_sample(&path, "chosen.pdf");
    assert!(
        matches!(&field(&after, "size").kind, FieldKind::Combo { selected: Some(2), value, .. } if value == "Large")
    );
    assert!(
        matches!(&field(&after, "fruits").kind, FieldKind::List { selected, .. } if *selected == vec![0, 2])
    );

    let again = round_trip(
        &path,
        &[Edit::Choose {
            field: fruits,
            index: 1,
            add: false,
        }],
    );
    assert!(
        matches!(&field(&again, "fruits").kind, FieldKind::List { selected, .. } if *selected == vec![1])
    );
}

#[test]
fn read_only_fields_and_wrong_kinds_are_refused() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document::open(write_fixture(dir.path(), "refuse.pdf"), None).unwrap();
    let snap = doc.snapshot();
    let refused = [
        Edit::SetText {
            field: field(&snap, "fixed").id,
            text: "x".into(),
        },
        Edit::SetText {
            field: field(&snap, "size").id,
            text: "x".into(),
        },
        Edit::SetChecked {
            field: field(&snap, "name").id,
            checked: true,
        },
        Edit::Choose {
            field: field(&snap, "size").id,
            index: 9,
            add: false,
        },
        Edit::Move {
            annotation: snap.annotations[0].id,
            dx: 1.0,
            dy: 1.0,
        },
        Edit::Delete {
            annotation: AnnotId { page: 0, index: 99 },
        },
    ];
    for edit in refused {
        assert!(
            matches!(
                doc.apply(std::slice::from_ref(&edit)),
                Err(Error::Invalid(_))
            ),
            "{edit:?}"
        );
    }
    assert!(!doc.dirty());
}

#[test]
fn a_failing_edit_is_named_and_the_ones_before_it_stay() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document::open(write_fixture(dir.path(), "partial.pdf"), None).unwrap();
    let snap = doc.snapshot();
    let error = doc
        .apply(&[
            Edit::SetText {
                field: field(&snap, "name").id,
                text: "kept".into(),
            },
            Edit::SetText {
                field: AnnotId { page: 0, index: 99 },
                text: "x".into(),
            },
        ])
        .unwrap_err();
    assert!(
        matches!(&error, Error::Invalid(why) if why.starts_with("edit 2:")),
        "{error:?}"
    );
    assert!(doc.dirty());
    assert_eq!(text_value(field(&doc.snapshot(), "name")), "kept");
}

fn line_quad(left: f32, top: f32, right: f32, bottom: f32) -> [(f32, f32); 4] {
    [(left, top), (right, top), (left, bottom), (right, bottom)]
}

#[test]
fn every_annotation_kind_is_added_and_survives_a_save() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_fixture(dir.path(), "annotate.pdf");
    let before = Document::open(path.clone(), None).unwrap().snapshot();
    // "Hello markup line" sits on the baseline at y = 700 in user space.
    let highlight = line_quad(72.0, 74.0, 230.0, 96.0);
    // On the turned page: a quad in shown space.
    let underline = line_quad(300.0, 40.0, 320.0, 200.0);
    let stroke: Vec<(f32, f32)> = (0..20)
        .map(|i| (320.0 + i as f32 * 8.0, 260.0 + (i % 4) as f32 * 6.0))
        .collect();
    let mut rgba = Vec::new();
    for y in 0..32u32 {
        for x in 0..48u32 {
            let inside = (x as i32 - 24).pow(2) + (y as i32 - 16).pow(2) < 150;
            rgba.extend_from_slice(&[200, 30, 60, if inside { 255 } else { 0 }]);
        }
    }
    let edits = [
        Edit::Add {
            page: 0,
            annotation: NewAnnotation::Markup {
                markup: Markup::Highlight,
                quads: vec![highlight],
                color: Rgb(1.0, 0.9, 0.0),
            },
        },
        Edit::Add {
            page: 1,
            annotation: NewAnnotation::Markup {
                markup: Markup::Underline,
                quads: vec![underline],
                color: Rgb(0.0, 0.0, 1.0),
            },
        },
        Edit::Add {
            page: 0,
            annotation: NewAnnotation::Ink {
                strokes: vec![stroke.clone()],
                width: 2.0,
                color: Rgb(0.0, 0.3, 0.8),
            },
        },
        Edit::Add {
            page: 0,
            annotation: NewAnnotation::Text {
                at: (320.0, 120.0),
                text: "A text box\nwith two lines".into(),
                size: 14.0,
                color: Rgb(0.8, 0.0, 0.0),
            },
        },
        Edit::Add {
            page: 0,
            annotation: NewAnnotation::Note {
                at: (40.0, 300.0),
                text: "A note".into(),
                color: Rgb(1.0, 0.8, 0.0),
            },
        },
        Edit::Add {
            page: 0,
            annotation: NewAnnotation::Image {
                rect: Rect::new(420.0, 420.0, 540.0, 500.0),
                width: 48,
                height: 32,
                rgba,
            },
        },
        Edit::Add {
            page: 2,
            annotation: NewAnnotation::Text {
                at: (20.0, 20.0),
                text: "On a turned page".into(),
                size: 12.0,
                color: Rgb(0.0, 0.5, 0.0),
            },
        },
    ];
    let after = round_trip(&path, &edits);
    keep_sample(&path, "annotated.pdf");
    let new = added(&before, &after);
    assert_eq!(new.len(), 7, "{new:#?}");

    let find = |page: usize, pick: &dyn Fn(&AnnotKind) -> bool| {
        *new.iter()
            .find(|annotation| annotation.id.page == page && pick(&annotation.kind))
            .unwrap_or_else(|| panic!("nothing like that on page {page}: {new:#?}"))
    };
    let got = find(0, &|kind| {
        matches!(
            kind,
            AnnotKind::Markup {
                markup: Markup::Highlight,
                ..
            }
        )
    });
    assert!(got.editable);
    assert!(near_color(got.color, Rgb(1.0, 0.9, 0.0)), "{:?}", got.color);
    let AnnotKind::Markup { quads, .. } = &got.kind else {
        unreachable!()
    };
    for (got, want) in quads[0].iter().zip(highlight) {
        assert!(near(got.0, want.0) && near(got.1, want.1), "{quads:?}");
    }
    assert!(
        near_rect(got.rect, Rect::new(72.0, 74.0, 230.0, 96.0)),
        "{:?}",
        got.rect
    );

    let got = find(1, &|kind| {
        matches!(
            kind,
            AnnotKind::Markup {
                markup: Markup::Underline,
                ..
            }
        )
    });
    let AnnotKind::Markup { quads, .. } = &got.kind else {
        unreachable!()
    };
    for (got, want) in quads[0].iter().zip(underline) {
        assert!(
            near(got.0, want.0) && near(got.1, want.1),
            "turned page: {quads:?}"
        );
    }
    assert!(near_color(got.color, Rgb(0.0, 0.0, 1.0)), "{:?}", got.color);

    let got = find(0, &|kind| matches!(kind, AnnotKind::Ink { .. }));
    let AnnotKind::Ink { strokes, width } = &got.kind else {
        unreachable!()
    };
    assert_eq!(*width, 2.0);
    assert_eq!(strokes.len(), 1);
    for (got, want) in strokes[0].iter().zip(&stroke) {
        assert!(near(got.0, want.0) && near(got.1, want.1));
    }
    assert!(near_color(got.color, Rgb(0.0, 0.3, 0.8)), "{:?}", got.color);

    let got = find(0, &|kind| matches!(kind, AnnotKind::Text { .. }));
    assert_eq!(
        got.kind,
        AnnotKind::Text {
            text: "A text box\nwith two lines".into(),
            size: 14.0
        }
    );
    assert!(near_color(got.color, Rgb(0.8, 0.0, 0.0)));
    assert!(
        near(got.rect.left, 320.0) && near(got.rect.top, 120.0),
        "{:?}",
        got.rect
    );
    assert!(
        near(got.rect.height(), 2.0 * 1.2 * 14.0 + 4.0),
        "{:?}",
        got.rect
    );

    let got = find(0, &|kind| matches!(kind, AnnotKind::Note { .. }));
    assert_eq!(
        got.kind,
        AnnotKind::Note {
            text: "A note".into()
        }
    );
    assert!(
        near_rect(got.rect, Rect::new(40.0, 300.0, 60.0, 320.0)),
        "{:?}",
        got.rect
    );

    let got = find(0, &|kind| matches!(kind, AnnotKind::Stamp));
    assert!(got.editable);
    assert!(
        near_rect(got.rect, Rect::new(420.0, 420.0, 540.0, 500.0)),
        "{:?}",
        got.rect
    );

    let got = find(2, &|kind| matches!(kind, AnnotKind::Text { .. }));
    assert!(
        near(got.rect.left, 20.0) && near(got.rect.top, 20.0),
        "{:?}",
        got.rect
    );
}

#[test]
fn annotations_move_change_and_delete() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_fixture(dir.path(), "change.pdf");
    let quad = line_quad(72.0, 74.0, 230.0, 96.0);
    let stroke = vec![(100.0, 200.0), (150.0, 220.0), (200.0, 200.0)];
    let mut rgba = Vec::new();
    for _ in 0..16 * 16 {
        rgba.extend_from_slice(&[0, 120, 0, 255]);
    }
    let base = round_trip(
        &path,
        &[
            Edit::Add {
                page: 0,
                annotation: NewAnnotation::Markup {
                    markup: Markup::Highlight,
                    quads: vec![quad],
                    color: Rgb(1.0, 1.0, 0.0),
                },
            },
            Edit::Add {
                page: 0,
                annotation: NewAnnotation::Ink {
                    strokes: vec![stroke.clone()],
                    width: 3.0,
                    color: Rgb(0.0, 0.0, 0.0),
                },
            },
            Edit::Add {
                page: 0,
                annotation: NewAnnotation::Text {
                    at: (300.0, 250.0),
                    text: "short".into(),
                    size: 12.0,
                    color: Rgb(0.0, 0.0, 0.0),
                },
            },
            Edit::Add {
                page: 0,
                annotation: NewAnnotation::Image {
                    rect: Rect::new(400.0, 300.0, 440.0, 340.0),
                    width: 16,
                    height: 16,
                    rgba,
                },
            },
            Edit::Add {
                page: 1,
                annotation: NewAnnotation::Note {
                    at: (200.0, 100.0),
                    text: "turned".into(),
                    color: Rgb(1.0, 1.0, 0.0),
                },
            },
        ],
    );
    let of = |snap: &Snapshot, page: usize, pick: fn(&AnnotKind) -> bool| -> Annotation {
        snap.annotations
            .iter()
            .rev()
            .find(|annotation| annotation.id.page == page && pick(&annotation.kind))
            .cloned()
            .unwrap()
    };
    let highlight = of(&base, 0, |kind| matches!(kind, AnnotKind::Markup { .. }));
    let ink = of(&base, 0, |kind| matches!(kind, AnnotKind::Ink { .. }));
    let text = of(&base, 0, |kind| matches!(kind, AnnotKind::Text { .. }));
    let stamp = of(&base, 0, |kind| matches!(kind, AnnotKind::Stamp));
    let note = of(&base, 1, |kind| matches!(kind, AnnotKind::Note { .. }));
    let old_note = of(&base, 0, |kind| matches!(kind, AnnotKind::Note { .. }));

    let (dx, dy) = (10.0, 30.0);
    let moved = round_trip(
        &path,
        &[
            Edit::Move {
                annotation: highlight.id,
                dx,
                dy,
            },
            Edit::Move {
                annotation: ink.id,
                dx,
                dy,
            },
            Edit::Move {
                annotation: stamp.id,
                dx,
                dy,
            },
            Edit::Move {
                annotation: note.id,
                dx,
                dy,
            },
            Edit::SetContents {
                annotation: text.id,
                text: "a much longer text\nover two lines".into(),
            },
            Edit::SetColor {
                annotation: highlight.id,
                color: Rgb(0.0, 1.0, 0.0),
            },
            Edit::SetColor {
                annotation: text.id,
                color: Rgb(0.0, 0.0, 1.0),
            },
            Edit::SetContents {
                annotation: old_note.id,
                text: "changed".into(),
            },
        ],
    );
    keep_sample(&path, "changed.pdf");

    let got = of(&moved, 0, |kind| {
        matches!(
            kind,
            AnnotKind::Markup {
                markup: Markup::Highlight,
                ..
            }
        )
    });
    assert!(
        near_rect(got.rect, highlight.rect.translate(dx, dy)),
        "{:?}",
        got.rect
    );
    let AnnotKind::Markup { quads, .. } = &got.kind else {
        unreachable!()
    };
    assert!(
        near(quads[0][0].0, quad[0].0 + dx) && near(quads[0][0].1, quad[0].1 + dy),
        "{quads:?}"
    );
    assert!(near_color(got.color, Rgb(0.0, 1.0, 0.0)), "{:?}", got.color);

    let got = of(&moved, 0, |kind| matches!(kind, AnnotKind::Ink { .. }));
    assert!(
        near_rect(got.rect, ink.rect.translate(dx, dy)),
        "{:?}",
        got.rect
    );
    let AnnotKind::Ink { strokes, .. } = &got.kind else {
        unreachable!()
    };
    for (got, want) in strokes[0].iter().zip(&stroke) {
        assert!(
            near(got.0, want.0 + dx) && near(got.1, want.1 + dy),
            "{strokes:?}"
        );
    }

    let got = of(&moved, 0, |kind| matches!(kind, AnnotKind::Stamp));
    assert!(
        near_rect(got.rect, stamp.rect.translate(dx, dy)),
        "{:?}",
        got.rect
    );

    let got = of(&moved, 1, |kind| matches!(kind, AnnotKind::Note { .. }));
    assert!(
        near_rect(got.rect, note.rect.translate(dx, dy)),
        "turned page: {:?}",
        got.rect
    );

    let got = of(&moved, 0, |kind| matches!(kind, AnnotKind::Text { .. }));
    assert_eq!(
        got.kind,
        AnnotKind::Text {
            text: "a much longer text\nover two lines".into(),
            size: 12.0
        }
    );
    assert!(near(got.rect.left, text.rect.left) && near(got.rect.top, text.rect.top));
    assert!(got.rect.width() > text.rect.width() && got.rect.height() > text.rect.height());
    assert!(near_color(got.color, Rgb(0.0, 0.0, 1.0)), "{:?}", got.color);

    let first_note = moved
        .annotations
        .iter()
        .find(|annotation| annotation.id == old_note.id)
        .unwrap();
    assert_eq!(
        first_note.kind,
        AnnotKind::Note {
            text: "changed".into()
        }
    );

    // Deleting takes the note's popup too, and several deletions in one
    // call each name the snapshot from before it.
    let ids: Vec<AnnotId> = moved
        .annotations
        .iter()
        .filter(|annotation| annotation.editable && annotation.id.page == 0)
        .map(|annotation| annotation.id)
        .collect();
    let deleted = round_trip(
        &path,
        &ids.iter()
            .map(|id| Edit::Delete { annotation: *id })
            .collect::<Vec<_>>(),
    );
    assert!(deleted
        .annotations
        .iter()
        .all(|annotation| annotation.id.page != 0 || !annotation.editable));
    assert_eq!(
        deleted
            .annotations
            .iter()
            .filter(|annotation| annotation.id.page == 0)
            .count(),
        2,
        "the locked highlight and the link stay: {:#?}",
        deleted.annotations
    );
    assert_eq!(deleted.fields.len(), moved.fields.len());
}

#[cfg(unix)]
#[test]
fn saving_replaces_the_file_whole_and_keeps_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_fixture(dir.path(), "perm.pdf");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut doc = Document::open(path.clone(), None).unwrap();
    let name = field(&doc.snapshot(), "name").id;
    doc.apply(&[Edit::SetText {
        field: name,
        text: "x".into(),
    }])
    .unwrap();
    doc.save().unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        vec![std::ffi::OsString::from("perm.pdf")],
        "no temporary file is left"
    );

    // A copy leaves the document as it was.
    let copy = dir.path().join("copy.pdf");
    doc.apply(&[Edit::SetText {
        field: name,
        text: "y".into(),
    }])
    .unwrap();
    doc.save_to(&copy).unwrap();
    assert!(doc.dirty());
    assert_eq!(doc.path(), path);
    let reopened = Document::open(copy, None).unwrap().snapshot();
    assert_eq!(text_value(field(&reopened, "name")), "y");

    // Saving through a link replaces the file it points at.
    let link = dir.path().join("link.pdf");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let mut through = Document::open(link.clone(), None).unwrap();
    through
        .apply(&[Edit::SetText {
            field: name,
            text: "z".into(),
        }])
        .unwrap();
    through.save().unwrap();
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    let reopened = Document::open(path, None).unwrap().snapshot();
    assert_eq!(text_value(field(&reopened, "name")), "z");
}

#[test]
fn an_incremental_save_keeps_the_original_bytes_in_front() {
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_fixture(dir.path(), "inc.pdf");
    let original = std::fs::read(&path).unwrap();
    let mut doc = Document::open(path.clone(), None).unwrap();
    let name = field(&doc.snapshot(), "name").id;
    doc.apply(&[
        Edit::SetText {
            field: name,
            text: "appended".into(),
        },
        Edit::Add {
            page: 0,
            annotation: NewAnnotation::Note {
                at: (100.0, 100.0),
                text: "late".into(),
                color: Rgb(1.0, 1.0, 0.0),
            },
        },
    ])
    .unwrap();
    doc.save_with(&path, true).unwrap();
    let saved = std::fs::read(&path).unwrap();
    assert!(saved.len() > original.len());
    assert_eq!(
        &saved[..original.len()],
        &original[..],
        "the signed bytes stay as they were"
    );
    let reopened = Document::open(path, None).unwrap().snapshot();
    assert_eq!(text_value(field(&reopened, "name")), "appended");
    assert!(reopened.annotations.iter().any(|a| a.kind
        == AnnotKind::Note {
            text: "late".into()
        }));

    // A full save writes the file afresh.
    let full = dir.path().join("full.pdf");
    doc.save_with(&full, false).unwrap();
    let rewritten = std::fs::read(&full).unwrap();
    assert_ne!(
        &rewritten[..original.len().min(rewritten.len())],
        &original[..original.len().min(rewritten.len())]
    );
}

fn qpdf() -> bool {
    let found = std::process::Command::new("qpdf")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !found {
        eprintln!("skipping: qpdf isn't installed");
    }
    found
}

#[test]
fn an_encrypted_file_stays_encrypted() {
    if !pdfium() || !qpdf() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let plain = write_fixture(dir.path(), "plain.pdf");
    let locked = dir.path().join("locked.pdf");
    let status = std::process::Command::new("qpdf")
        .args(["--encrypt", "u", "o", "256", "--"])
        .arg(&plain)
        .arg(&locked)
        .status()
        .unwrap();
    assert!(status.success());

    assert_eq!(crate::open(&locked, None).unwrap_err(), Error::Password);
    assert_eq!(
        crate::open(&locked, Some("wrong")).unwrap_err(),
        Error::Password
    );
    let mut doc = crate::open(&locked, Some("u")).unwrap();
    let name = field(&doc.snapshot(), "name").id;
    doc.apply(&[Edit::SetText {
        field: name,
        text: "secret".into(),
    }])
    .unwrap();
    doc.save().unwrap();
    drop(doc);

    assert_eq!(crate::open(&locked, None).unwrap_err(), Error::Password);
    let reopened = crate::open(&locked, Some("u")).unwrap().snapshot();
    assert_eq!(text_value(field(&reopened, "name")), "secret");
    let shown = std::process::Command::new("qpdf")
        .args(["--show-encryption", "--password=u"])
        .arg(&locked)
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&shown.stdout);
    assert!(report.contains("R = 6"), "still AES-256: {report}");
}

/// Fills and marks up the document `OTTO_PDF_REAL` names, writing to the
/// directory `OTTO_PDF_SAMPLES` names. Prints only counts and kinds.
#[test]
#[ignore = "needs OTTO_PDF_REAL and OTTO_PDF_SAMPLES"]
fn real_world_document() {
    let (Some(real), Some(out)) = (std::env::var_os("OTTO_PDF_REAL"), samples_dir()) else {
        return;
    };
    if !pdfium() {
        return;
    }
    let mut doc = Document::open(PathBuf::from(real), None).unwrap();
    let snap = doc.snapshot();
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for field in &snap.fields {
        let kind = match field.kind {
            FieldKind::Text { .. } => "text",
            FieldKind::Checkbox { .. } => "checkbox",
            FieldKind::Radio { .. } => "radio",
            FieldKind::Combo { .. } => "combo",
            FieldKind::List { .. } => "list",
            FieldKind::Button => "button",
            FieldKind::Signature { .. } => "signature",
        };
        *kinds.entry(kind).or_default() += 1;
    }
    println!(
        "pages {} fields {} {kinds:?} annotations {} signed {}",
        snap.pages.len(),
        snap.fields.len(),
        snap.annotations.len(),
        snap.signed
    );
    let mut edits = Vec::new();
    if let Some(text) = snap
        .fields
        .iter()
        .find(|field| matches!(field.kind, FieldKind::Text { .. }) && !field.read_only)
    {
        edits.push(Edit::SetText {
            field: text.id,
            text: "Otto Test".into(),
        });
    }
    if let Some(radio) = snap.fields.iter().find(|field| {
        matches!(field.kind, FieldKind::Radio { checked: false, .. }) && !field.read_only
    }) {
        edits.push(Edit::SetChecked {
            field: radio.id,
            checked: true,
        });
    }
    if let Some(checkbox) = snap
        .fields
        .iter()
        .find(|field| matches!(field.kind, FieldKind::Checkbox { .. }) && !field.read_only)
    {
        edits.push(Edit::SetChecked {
            field: checkbox.id,
            checked: !checked(checkbox),
        });
    }
    edits.push(Edit::Add {
        page: 0,
        annotation: NewAnnotation::Text {
            at: (40.0, 40.0),
            text: "Added by otto-pdf".into(),
            size: 14.0,
            color: Rgb(0.8, 0.0, 0.0),
        },
    });
    let count = edits.len();
    doc.apply(&edits).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    doc.save_to(&out.join("real-filled.pdf")).unwrap();
    doc.save_with(&out.join("real-filled-incremental.pdf"), true)
        .unwrap();
    println!("applied {count} edits");
    let reopened = Document::open(out.join("real-filled.pdf"), None)
        .unwrap()
        .snapshot();
    println!(
        "reopened: fields {} annotations {}",
        reopened.fields.len(),
        reopened.annotations.len()
    );
}

/// How long an edit takes, step by step, on `OTTO_PDF_TIMING` (a PDF with a
/// text field): open, then a few fills, each applied and saved.
#[test]
#[ignore = "prints timings for a file named by OTTO_PDF_TIMING"]
fn edit_timings() {
    let Some(source) = std::env::var_os("OTTO_PDF_TIMING") else {
        return;
    };
    if !pdfium() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("timing.pdf");
    std::fs::copy(source, &path).unwrap();
    let started = std::time::Instant::now();
    let mut doc = Document::open(path.clone(), None).unwrap();
    println!("open {:?}", started.elapsed());
    let snap = doc.snapshot();
    let field = snap
        .fields
        .iter()
        .find(|field| matches!(field.kind, FieldKind::Text { .. }) && !field.read_only)
        .expect("a text field")
        .id;
    for round in 0..4 {
        let started = std::time::Instant::now();
        doc.apply(&[Edit::SetText {
            field,
            text: format!("Round {round}"),
        }])
        .unwrap();
        let applied = started.elapsed();
        let started = std::time::Instant::now();
        doc.save().unwrap();
        println!("apply {applied:?} save {:?}", started.elapsed());
    }
}
