//! One open document on the PDFium thread: its handles, what was read from
//! them, and the edits and saves made through them.
//!
//! Nothing here is `Send`: a [`Doc`] holds raw PDFium handles and lives and
//! dies on the worker thread that made it. Within this module a handle held
//! by a [`Doc`], [`PageHandle`] or [`AnnotHandle`] is always live, which is
//! what every `unsafe` call below relies on.

// Rust guideline compliant 2026-02-21

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_int, c_uint, c_ulong, c_void, CStr, CString};
use std::marker::PhantomData;

use crate::bindings::{self as ffi, Bindings};
use crate::geometry::{bounds, PageGeometry};
use crate::{
    AnnotId, AnnotKind, Annotation, Edit, Error, Field, FieldKind, Markup, NewAnnotation, Rect,
    Result, Rgb, Snapshot,
};

/// A rect in a page's user space: `(left, bottom, right, top)`.
type UserRect = (f32, f32, f32, f32);

/// Space between a text box's edge and its text, in points.
const TEXT_PADDING: f32 = 2.0;
/// A text box's distance from one baseline to the next, in text sizes.
const LINE_HEIGHT: f32 = 1.2;
/// The side of a sticky note's icon, in points.
const NOTE_SIZE: f32 = 20.0;

const FIELD_READ_ONLY: c_int = 1 << 0;
const FIELD_REQUIRED: c_int = 1 << 1;
const FIELD_MULTILINE: c_int = 1 << 12;
const FIELD_PASSWORD: c_int = 1 << 13;
const FIELD_COMBO_EDIT: c_int = 1 << 18;
const FIELD_MULTI_SELECT: c_int = 1 << 21;
const FIELD_COMB: c_int = 1 << 24;

/// What was read from one page.
#[derive(Debug, Clone)]
struct PageSnap {
    geometry: PageGeometry,
    fields: Vec<Field>,
    annotations: Vec<Annotation>,
}

/// An open document.
pub(crate) struct Doc {
    ffi: &'static Bindings,
    raw: ffi::FPDF_DOCUMENT,
    /// Null when PDFium gave no form-fill environment; fields are then not
    /// read.
    form: ffi::FPDF_FORMHANDLE,
    /// PDFium keeps a pointer to this for the environment's life.
    _form_info: Box<ffi::FPDF_FORMFILLINFO>,
    /// The file's bytes, which PDFium reads from until the document closes.
    _bytes: Vec<u8>,
    pages: Vec<PageSnap>,
    signed: bool,
}

impl Drop for Doc {
    fn drop(&mut self) {
        // SAFETY: both handles are live and owned by this `Doc`; every page
        // and annotation handle borrows it, so none outlives this.
        unsafe {
            if !self.form.is_null() {
                (self.ffi.FPDFDOC_ExitFormFillEnvironment)(self.form);
            }
            (self.ffi.FPDF_CloseDocument)(self.raw);
        }
    }
}

impl Doc {
    /// Opens a document from its bytes and reads every page.
    pub fn open(ffi: &'static Bindings, bytes: Vec<u8>, password: Option<String>) -> Result<Self> {
        let password = password
            .map(CString::new)
            .transpose()
            .map_err(|_| Error::Password)?;
        let password_ptr = password.as_ref().map_or(std::ptr::null(), |p| p.as_ptr());
        // SAFETY: `bytes` is moved into the returned `Doc`, whose heap buffer
        // stays put and outlives the document; the password is a C string
        // that lives through the call.
        let raw = unsafe {
            (ffi.FPDF_LoadMemDocument64)(bytes.as_ptr().cast::<c_void>(), bytes.len(), password_ptr)
        };
        if raw.is_null() {
            // SAFETY: no arguments; reads the thread's last PDFium error.
            let code = unsafe { (ffi.FPDF_GetLastError)() };
            return Err(match code {
                ffi::FPDF_ERR_PASSWORD => Error::Password,
                2 => Error::Unreadable("the file couldn't be read".into()),
                5 => Error::Unreadable("it is locked in a way PDFium doesn't support".into()),
                _ => Error::Unreadable("PDFium couldn't parse it".into()),
            });
        }
        let mut form_info = Box::new(ffi::FPDF_FORMFILLINFO::version_2());
        // SAFETY: `raw` is live; `form_info` is boxed into the `Doc`, so the
        // pointer PDFium keeps stays valid until the environment exits.
        let form = unsafe { (ffi.FPDFDOC_InitFormFillEnvironment)(raw, &mut *form_info) };
        let mut doc = Self {
            ffi,
            raw,
            form,
            _form_info: form_info,
            _bytes: bytes,
            pages: Vec::new(),
            signed: false,
        };
        // SAFETY: `doc.raw` is live.
        let count = unsafe { (ffi.FPDF_GetPageCount)(doc.raw) }.max(0) as usize;
        for index in 0..count {
            let page = doc.read_page(index)?;
            doc.pages.push(page);
        }
        doc.signed = doc.has_signature();
        Ok(doc)
    }

    /// Draws page `index` as it is now, form fields and annotations
    /// included, `width` pixels wide on white: RGBA, four bytes a pixel,
    /// opaque. Returns the height with the pixels.
    pub fn render(&self, index: usize, width: u32) -> Result<(u32, Vec<u8>)> {
        let shown = self
            .pages
            .get(index)
            .ok_or_else(|| Error::Invalid(format!("there is no page {}", index + 1)))?
            .geometry
            .page();
        let width = width.clamp(1, 8192);
        let height = ((width as f32 * shown.height / shown.width.max(1.0)).round() as u32).max(1);
        let page = self.page(index)?;
        let stride = width as usize * 4;
        let mut pixels = vec![0u8; stride * height as usize];
        let (w, h) = (width as c_int, height as c_int);
        // SAFETY: the bitmap wraps `pixels`, which is `stride * height`
        // bytes and outlives it (destroyed below, before `pixels` moves);
        // the page and the form are live.
        unsafe {
            let bitmap = (self.ffi.FPDFBitmap_CreateEx)(
                w,
                h,
                ffi::FPDF_BITMAP_BGRA,
                pixels.as_mut_ptr().cast::<c_void>(),
                stride as c_int,
            );
            if bitmap.is_null() {
                return Err(Error::Failed("couldn't make a bitmap to draw into".into()));
            }
            (self.ffi.FPDFBitmap_FillRect)(bitmap, 0, 0, w, h, 0xFFFF_FFFF);
            let flags = ffi::FPDF_RENDER_ANNOT | ffi::FPDF_RENDER_REVERSE_BYTE_ORDER;
            (self.ffi.FPDF_RenderPageBitmap)(bitmap, page.raw, 0, 0, w, h, 0, flags);
            if !self.form.is_null() {
                (self.ffi.FPDF_FFLDraw)(self.form, bitmap, page.raw, 0, 0, w, h, 0, flags);
            }
            (self.ffi.FPDFBitmap_Destroy)(bitmap);
        }
        drop(page);
        Ok((height, pixels))
    }

    /// Whether any signature field holds a signature: PDFium counts the
    /// empty ones too, so each is asked for its contents.
    fn has_signature(&self) -> bool {
        let ffi = self.ffi;
        // SAFETY: `raw` is live; signature handles belong to the document
        // and are only read while it is; a null buffer with length 0 asks
        // for the size alone.
        unsafe {
            let count = (ffi.FPDF_GetSignatureCount)(self.raw);
            (0..count).any(|index| {
                let signature = (ffi.FPDF_GetSignatureObject)(self.raw, index);
                !signature.is_null()
                    && (ffi.FPDFSignatureObj_GetContents)(signature, std::ptr::null_mut(), 0) > 0
            })
        }
    }

    /// What the document is now.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            pages: self.pages.iter().map(|page| page.geometry.page()).collect(),
            fields: self
                .pages
                .iter()
                .flat_map(|page| page.fields.iter().cloned())
                .collect(),
            annotations: self
                .pages
                .iter()
                .flat_map(|page| page.annotations.iter().cloned())
                .collect(),
            signed: self.signed,
        }
    }

    /// The whole file as it would be saved now: appended to the original
    /// when `incremental`, written afresh otherwise. Encryption is kept.
    pub fn save(&self, incremental: bool) -> Result<Vec<u8>> {
        let flags = if incremental {
            ffi::FPDF_INCREMENTAL
        } else {
            ffi::FPDF_NO_INCREMENTAL
        };
        let mut sink = Sink {
            write: ffi::FPDF_FILEWRITE {
                version: 1,
                WriteBlock: Some(write_block),
            },
            bytes: Vec::new(),
        };
        // SAFETY: `raw` is live; `sink` outlives the call, and its first
        // field is the `FPDF_FILEWRITE` whose address `write_block` casts
        // back to the `Sink`.
        let ok = unsafe { (self.ffi.FPDF_SaveAsCopy)(self.raw, &mut sink.write, flags) };
        if ok == 0 || sink.bytes.is_empty() {
            return Err(Error::Failed("PDFium couldn't write the document".into()));
        }
        Ok(sink.bytes)
    }

    /// Applies `edits` in memory, then reads again the pages they touched.
    ///
    /// Every id names an entry of the snapshot from before the call. Adds,
    /// moves and changes run in order; deletions run last, highest index
    /// first, so no edit renumbers what a later one names. On an error the
    /// edits before the failing one stay made. Answers whether anything
    /// changed, with the first error.
    pub fn apply(&mut self, edits: &[Edit]) -> (bool, Result<()>) {
        let mut touched = BTreeSet::new();
        let mut deletions: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        let mut changed = false;
        let mut outcome = Ok(());
        for (number, edit) in edits.iter().enumerate() {
            match self.apply_one(edit, &mut touched, &mut deletions) {
                Ok(made) => changed |= made,
                Err(error) => {
                    outcome = Err(in_edit(number, error));
                    break;
                }
            }
        }
        for (page, indices) in &deletions {
            match self.remove(*page, indices) {
                Ok(()) => {
                    changed = true;
                    touched.insert(*page);
                }
                Err(error) if outcome.is_ok() => outcome = Err(error),
                Err(_) => {}
            }
        }
        for page in touched {
            match self.read_page(page) {
                Ok(snap) => self.pages[page] = snap,
                Err(error) if outcome.is_ok() => outcome = Err(error),
                Err(_) => {}
            }
        }
        (changed, outcome)
    }

    fn apply_one(
        &self,
        edit: &Edit,
        touched: &mut BTreeSet<usize>,
        deletions: &mut BTreeMap<usize, BTreeSet<usize>>,
    ) -> Result<bool> {
        match edit {
            Edit::SetText { field, text } => {
                let field = self.field(*field)?;
                touched.extend(self.pages_with_field(&field.name));
                self.set_text(&field, text)
            }
            Edit::SetChecked { field, checked } => {
                let field = self.field(*field)?;
                touched.extend(self.pages_with_field(&field.name));
                self.set_checked(&field, *checked)
            }
            Edit::Choose { field, index, add } => {
                let field = self.field(*field)?;
                touched.extend(self.pages_with_field(&field.name));
                self.choose(&field, *index, *add)
            }
            Edit::Add { page, annotation } => {
                if *page >= self.pages.len() {
                    return Err(Error::Invalid(format!("there is no page {}", page + 1)));
                }
                touched.insert(*page);
                self.add(*page, annotation).map(|()| true)
            }
            Edit::Move { annotation, dx, dy } => {
                let annotation = self.editable(*annotation)?;
                touched.insert(annotation.id.page);
                self.shift(&annotation, *dx, *dy).map(|()| true)
            }
            Edit::SetContents { annotation, text } => {
                let annotation = self.editable(*annotation)?;
                touched.insert(annotation.id.page);
                self.set_contents(&annotation, text).map(|()| true)
            }
            Edit::SetColor { annotation, color } => {
                let annotation = self.editable(*annotation)?;
                touched.insert(annotation.id.page);
                self.set_color(&annotation, *color).map(|()| true)
            }
            Edit::Delete { annotation } => {
                let annotation = self.editable(*annotation)?;
                let page = self.page(annotation.id.page)?;
                let handle = page.annotation(&annotation)?;
                let doomed = deletions.entry(annotation.id.page).or_default();
                if !doomed.insert(annotation.id.index) {
                    return Err(Error::Invalid(
                        "that annotation is already being deleted".into(),
                    ));
                }
                // A note's popup goes with it.
                if let Some(popup) = page.linked_index(&handle, c"Popup") {
                    doomed.insert(popup);
                }
                Ok(false)
            }
        }
    }

    // Reading.

    fn read_page(&self, index: usize) -> Result<PageSnap> {
        // Annotations are read before the form's view of the page exists:
        // making it draws an appearance for each that lacks one, and PDFium
        // won't tell an annotation's colour past its appearance.
        let mut page = self.load_page(index, false)?;
        let mut snap = PageSnap {
            geometry: page.geometry,
            fields: Vec::new(),
            annotations: Vec::new(),
        };
        let id = |annot_index| AnnotId {
            page: index,
            index: annot_index,
        };
        let mut widgets = Vec::new();
        for annot_index in 0..page.annot_count() {
            let Ok(annot) = page.annot(annot_index) else {
                continue;
            };
            match annot.subtype() {
                ffi::FPDF_ANNOT_WIDGET => widgets.push(annot_index),
                // A popup belongs to the note that opens it.
                ffi::FPDF_ANNOT_POPUP => {}
                subtype => {
                    let annotation = read_annotation(&page, &annot, id(annot_index), subtype);
                    // PDFium draws a note that has no appearance yellow,
                    // whatever its colour; it gets one in its own.
                    if let (AnnotKind::Note { .. }, Some(color), Some(rect)) =
                        (&annotation.kind, annotation.color, annot.rect())
                    {
                        if annot.appearance().is_empty() {
                            let _ = annot.set_appearance(&note_appearance(rect, color));
                        }
                    }
                    snap.annotations.push(annotation);
                }
            }
        }
        page.attach_form();
        // Drawing an ink annotation grows its rect to cover the line width.
        for annotation in &mut snap.annotations {
            if let AnnotKind::Ink { .. } = annotation.kind {
                if let Some(rect) = page
                    .annot(annotation.id.index)
                    .ok()
                    .and_then(|annot| annot.rect())
                {
                    annotation.rect = page.geometry.rect_to_shown(rect);
                }
            }
        }
        for annot_index in widgets {
            let Ok(annot) = page.annot(annot_index) else {
                continue;
            };
            if let Some(field) = self.read_field(&page, &annot, id(annot_index)) {
                snap.fields.push(field);
            }
        }
        Ok(snap)
    }

    fn read_field(
        &self,
        page: &PageHandle<'_>,
        annot: &AnnotHandle<'_>,
        id: AnnotId,
    ) -> Option<Field> {
        let form = self.form;
        if form.is_null() {
            return None;
        }
        let ffi = self.ffi;
        let raw = annot.raw;
        // SAFETY (every call in this function): `form` and `raw` are live
        // handles of this document, and buffers come from `wide_string`,
        // which passes their true length.
        let field_type = unsafe { (ffi.FPDFAnnot_GetFormFieldType)(form, raw) };
        let flags = unsafe { (ffi.FPDFAnnot_GetFormFieldFlags)(form, raw) }.max(0);
        let value = || {
            one_line_break(wide_string(|buffer, length| unsafe {
                (ffi.FPDFAnnot_GetFormFieldValue)(form, raw, buffer, length)
            }))
        };
        let checked = || unsafe { (ffi.FPDFAnnot_IsChecked)(form, raw) } != 0;
        let options = || {
            let count = unsafe { (ffi.FPDFAnnot_GetOptionCount)(form, raw) }.max(0);
            (0..count)
                .map(|index| {
                    let label = wide_string(|buffer, length| unsafe {
                        (ffi.FPDFAnnot_GetOptionLabel)(form, raw, index, buffer, length)
                    });
                    let selected =
                        unsafe { (ffi.FPDFAnnot_IsOptionSelected)(form, raw, index) } != 0;
                    (label, selected)
                })
                .collect::<Vec<_>>()
        };
        let kind = match field_type {
            ffi::FPDF_FORMFIELD_TEXTFIELD => FieldKind::Text {
                value: value(),
                multiline: flags & FIELD_MULTILINE != 0,
                password: flags & FIELD_PASSWORD != 0,
                max_len: annot
                    .number(c"MaxLen")
                    .filter(|max| *max >= 1.0)
                    .map(|max| max as usize),
                comb: flags & FIELD_COMB != 0,
            },
            ffi::FPDF_FORMFIELD_CHECKBOX => FieldKind::Checkbox { checked: checked() },
            ffi::FPDF_FORMFIELD_RADIOBUTTON => FieldKind::Radio {
                checked: checked(),
                export: wide_string(|buffer, length| unsafe {
                    (ffi.FPDFAnnot_GetFormFieldExportValue)(form, raw, buffer, length)
                }),
            },
            ffi::FPDF_FORMFIELD_COMBOBOX => {
                let options = options();
                let value = value();
                let selected = options
                    .iter()
                    .position(|(_, selected)| *selected)
                    .or_else(|| options.iter().position(|(label, _)| *label == value));
                FieldKind::Combo {
                    options: options.into_iter().map(|(label, _)| label).collect(),
                    selected,
                    value,
                    editable: flags & FIELD_COMBO_EDIT != 0,
                }
            }
            ffi::FPDF_FORMFIELD_LISTBOX => {
                let options = options();
                FieldKind::List {
                    selected: options
                        .iter()
                        .enumerate()
                        .filter(|(_, (_, selected))| *selected)
                        .map(|(index, _)| index)
                        .collect(),
                    options: options.into_iter().map(|(label, _)| label).collect(),
                    multiple: flags & FIELD_MULTI_SELECT != 0,
                }
            }
            ffi::FPDF_FORMFIELD_PUSHBUTTON => FieldKind::Button,
            ffi::FPDF_FORMFIELD_SIGNATURE => FieldKind::Signature {
                signed: annot.has_key(c"V"),
            },
            _ => return None,
        };
        let name = wide_string(|buffer, length| unsafe {
            (ffi.FPDFAnnot_GetFormFieldName)(form, raw, buffer, length)
        });
        let tooltip = wide_string(|buffer, length| unsafe {
            (ffi.FPDFAnnot_GetFormFieldAlternateName)(form, raw, buffer, length)
        });
        let mut font_size = 0.0;
        if unsafe { (ffi.FPDFAnnot_GetFontSize)(form, raw, &mut font_size) } == 0 {
            font_size = 0.0;
        }
        Some(Field {
            id,
            name,
            tooltip: Some(tooltip).filter(|tooltip| !tooltip.is_empty()),
            rect: annot
                .rect()
                .map(|rect| page.geometry.rect_to_shown(rect))
                .unwrap_or_default(),
            kind,
            read_only: flags & FIELD_READ_ONLY != 0,
            required: flags & FIELD_REQUIRED != 0,
            font_size,
        })
    }

    // Looking up what an edit names.

    fn field(&self, id: AnnotId) -> Result<Field> {
        self.pages
            .get(id.page)
            .and_then(|page| page.fields.iter().find(|field| field.id == id))
            .cloned()
            .ok_or_else(|| Error::Invalid(format!("no field at {}", describe(id))))
    }

    fn editable(&self, id: AnnotId) -> Result<Annotation> {
        let annotation = self
            .pages
            .get(id.page)
            .and_then(|page| {
                page.annotations
                    .iter()
                    .find(|annotation| annotation.id == id)
            })
            .cloned()
            .ok_or_else(|| Error::Invalid(format!("no annotation at {}", describe(id))))?;
        if !annotation.editable {
            return Err(Error::Invalid(format!(
                "the annotation at {} can't be edited",
                describe(id)
            )));
        }
        Ok(annotation)
    }

    fn pages_with_field(&self, name: &str) -> Vec<usize> {
        self.pages
            .iter()
            .enumerate()
            .filter(|(_, page)| page.fields.iter().any(|field| field.name == name))
            .map(|(index, _)| index)
            .collect()
    }

    /// A page, with the form's view of it.
    fn page(&self, index: usize) -> Result<PageHandle<'_>> {
        self.load_page(index, true)
    }

    fn load_page(&self, index: usize, with_form: bool) -> Result<PageHandle<'_>> {
        let page_index = c_int::try_from(index)
            .map_err(|_| Error::Invalid(format!("there is no page {}", index + 1)))?;
        // SAFETY: `raw` is live; a bad index yields null, handled below.
        let raw = unsafe { (self.ffi.FPDF_LoadPage)(self.raw, page_index) };
        if raw.is_null() {
            return Err(Error::Failed(format!("page {} wouldn't load", index + 1)));
        }
        let mut bbox = ffi::FS_RECTF::default();
        // SAFETY: `raw` is the live page just loaded and `bbox` is writable.
        let (has_box, turns) = unsafe {
            (
                (self.ffi.FPDF_GetPageBoundingBox)(raw, &mut bbox) != 0,
                (self.ffi.FPDFPage_GetRotation)(raw),
            )
        };
        let corners = if has_box {
            (bbox.left, bbox.bottom, bbox.right, bbox.top)
        } else {
            (0.0, 0.0, 612.0, 792.0)
        };
        let mut page = PageHandle {
            doc: self,
            raw,
            geometry: PageGeometry::new(corners, turns.clamp(0, 3) * 90),
            with_form: false,
        };
        if with_form {
            page.attach_form();
        }
        Ok(page)
    }

    // Filling the form.

    fn set_text(&self, field: &Field, text: &str) -> Result<bool> {
        let (current, max_len) = match &field.kind {
            FieldKind::Text { value, max_len, .. } => (value.clone(), *max_len),
            FieldKind::Combo {
                value,
                editable: true,
                ..
            } => (value.clone(), None),
            FieldKind::Combo { .. } => {
                return Err(Error::Invalid(format!(
                    "“{}” only takes one of its choices",
                    field.name
                )));
            }
            _ => {
                return Err(Error::Invalid(format!(
                    "“{}” isn't a text field",
                    field.name
                )))
            }
        };
        refuse_read_only(field)?;
        let text = one_line_break(text.to_owned());
        let wanted: String = match max_len {
            Some(max) => text.chars().take(max).collect(),
            None => text.clone(),
        };
        if current == wanted {
            return Ok(false);
        }
        let page = self.page(field.id.page)?;
        let annot = page.widget(field.id.index)?;
        let typed = utf16z(&text);
        let ffi = self.ffi;
        // SAFETY: the form, page and annotation are live; `typed` is a
        // nul-terminated UTF-16 string that outlives the call.
        unsafe {
            if (ffi.FORM_SetFocusedAnnot)(self.form, annot.raw) == 0 {
                return Err(Error::Failed(format!(
                    "PDFium wouldn't focus “{}”",
                    field.name
                )));
            }
            (ffi.FORM_SelectAllText)(self.form, page.raw);
            if let FieldKind::Combo { .. } = field.kind {
                // A combo only takes typed text as its value when it comes
                // in as keystrokes; replacing the selection leaves it on
                // the choice it had.
                if text.is_empty() {
                    (ffi.FORM_OnKeyDown)(self.form, page.raw, ffi::FWL_VKEY_DELETE, 0);
                }
                for unit in text.encode_utf16() {
                    (ffi.FORM_OnChar)(self.form, page.raw, c_int::from(unit), 0);
                }
            } else {
                (ffi.FORM_ReplaceSelection)(self.form, page.raw, typed.as_ptr());
            }
            (ffi.FORM_ForceToKillFocus)(self.form);
        }
        let now = self.field_value(&annot);
        if now != wanted {
            return Err(Error::Failed(format!(
                "PDFium didn't take the text for “{}”",
                field.name
            )));
        }
        Ok(true)
    }

    fn set_checked(&self, field: &Field, checked: bool) -> Result<bool> {
        let (current, radio) = match field.kind {
            FieldKind::Checkbox { checked } => (checked, false),
            FieldKind::Radio { checked, .. } => (checked, true),
            _ => {
                return Err(Error::Invalid(format!(
                    "“{}” isn't a checkbox or a radio button",
                    field.name
                )))
            }
        };
        refuse_read_only(field)?;
        if current == checked {
            return Ok(false);
        }
        if radio && !checked {
            return Err(Error::Invalid(format!(
                "a radio button of “{}” can't be unticked on its own: tick another",
                field.name
            )));
        }
        let page = self.page(field.id.page)?;
        let annot = page.widget(field.id.index)?;
        let (left, bottom, right, top) = annot
            .rect()
            .ok_or_else(|| Error::Failed(format!("“{}” has no place on the page", field.name)))?;
        let (x, y) = (f64::from(left + right) / 2.0, f64::from(bottom + top) / 2.0);
        let ffi = self.ffi;
        // SAFETY: the form and page are live; a click is plain input.
        let now = unsafe {
            (ffi.FORM_OnMouseMove)(self.form, page.raw, 0, x, y);
            (ffi.FORM_OnLButtonDown)(self.form, page.raw, 0, x, y);
            (ffi.FORM_OnLButtonUp)(self.form, page.raw, 0, x, y);
            (ffi.FORM_ForceToKillFocus)(self.form);
            (ffi.FPDFAnnot_IsChecked)(self.form, annot.raw) != 0
        };
        if now != checked {
            return Err(Error::Failed(format!(
                "PDFium didn't tick “{}”",
                field.name
            )));
        }
        Ok(true)
    }

    fn choose(&self, field: &Field, index: usize, add: bool) -> Result<bool> {
        let (count, selected, deselect_others) = match &field.kind {
            FieldKind::Combo {
                options, selected, ..
            } => (
                options.len(),
                selected.iter().copied().collect::<Vec<_>>(),
                false,
            ),
            FieldKind::List {
                options,
                selected,
                multiple,
            } => (options.len(), selected.clone(), !(*multiple && add)),
            _ => return Err(Error::Invalid(format!("“{}” has no choices", field.name))),
        };
        refuse_read_only(field)?;
        if index >= count {
            return Err(Error::Invalid(format!(
                "“{}” has {count} choices, not {}",
                field.name,
                index + 1
            )));
        }
        let others: Vec<usize> = if deselect_others {
            selected
                .iter()
                .copied()
                .filter(|other| *other != index)
                .collect()
        } else {
            Vec::new()
        };
        if selected.contains(&index) && others.is_empty() {
            return Ok(false);
        }
        let page = self.page(field.id.page)?;
        let annot = page.widget(field.id.index)?;
        let ffi = self.ffi;
        // SAFETY: the form, page and annotation are live; indices are below
        // the option count, which fits a c_int because PDFium returned it as
        // one.
        let chosen = unsafe {
            if (ffi.FORM_SetFocusedAnnot)(self.form, annot.raw) == 0 {
                return Err(Error::Failed(format!(
                    "PDFium wouldn't focus “{}”",
                    field.name
                )));
            }
            if let FieldKind::List { .. } = field.kind {
                for other in &others {
                    (ffi.FORM_SetIndexSelected)(self.form, page.raw, *other as c_int, 0);
                }
            }
            (ffi.FORM_SetIndexSelected)(self.form, page.raw, index as c_int, 1);
            (ffi.FORM_ForceToKillFocus)(self.form);
            (ffi.FPDFAnnot_IsOptionSelected)(self.form, annot.raw, index as c_int) != 0
        };
        if !chosen {
            return Err(Error::Failed(format!(
                "PDFium didn't choose that for “{}”",
                field.name
            )));
        }
        Ok(true)
    }

    fn field_value(&self, annot: &AnnotHandle<'_>) -> String {
        let (ffi, form, raw) = (self.ffi, self.form, annot.raw);
        // SAFETY: live form and annotation; `wide_string` passes the true
        // buffer length.
        one_line_break(wide_string(|buffer, length| unsafe {
            (ffi.FPDFAnnot_GetFormFieldValue)(form, raw, buffer, length)
        }))
    }

    // Annotations.

    fn add(&self, page_index: usize, new: &NewAnnotation) -> Result<()> {
        let page = self.page(page_index)?;
        let geometry = page.geometry;
        match new {
            NewAnnotation::Markup {
                markup,
                quads,
                color,
            } => {
                if quads.is_empty() {
                    return Err(Error::Invalid("marked text needs at least one line".into()));
                }
                let quads: Vec<[(f32, f32); 4]> = quads
                    .iter()
                    .map(|quad| geometry.quad_to_user(*quad))
                    .collect();
                let rect = user_bounds(quads.iter().flatten().copied(), 0.0)
                    .ok_or_else(|| Error::Invalid("marked text needs at least one line".into()))?;
                page.build(markup_subtype(*markup), |annot| {
                    annot.set_color(*color, 1.0)?;
                    for quad in &quads {
                        annot.append_quad(quad)?;
                    }
                    annot.set_rect(rect)?;
                    annot.set_flags(ffi::FPDF_ANNOT_FLAG_PRINT)
                })
            }
            NewAnnotation::Ink {
                strokes,
                width,
                color,
            } => {
                if strokes.iter().all(Vec::is_empty) {
                    return Err(Error::Invalid("ink needs at least one point".into()));
                }
                if width.is_nan() || *width <= 0.0 {
                    return Err(Error::Invalid("ink needs a width above zero".into()));
                }
                let strokes: Vec<Vec<(f32, f32)>> = strokes
                    .iter()
                    .filter(|stroke| !stroke.is_empty())
                    .map(|stroke| {
                        stroke
                            .iter()
                            .map(|point| geometry.to_user(*point))
                            .collect()
                    })
                    .collect();
                // PDFium grows the rect by half the width when it draws the
                // strokes, which is then just what they cover.
                let rect = user_bounds(strokes.iter().flatten().copied(), 0.0)
                    .ok_or_else(|| Error::Invalid("ink needs at least one point".into()))?;
                page.build(ffi::FPDF_ANNOT_INK, |annot| {
                    annot.set_color(*color, 1.0)?;
                    for stroke in &strokes {
                        annot.add_stroke(stroke)?;
                    }
                    annot.set_rect(rect)?;
                    annot.set_border(*width)?;
                    annot.set_flags(ffi::FPDF_ANNOT_FLAG_PRINT)
                })
            }
            NewAnnotation::Text {
                at,
                text,
                size,
                color,
            } => {
                if size.is_nan() || *size <= 0.0 {
                    return Err(Error::Invalid("text needs a size above zero".into()));
                }
                page.build(ffi::FPDF_ANNOT_FREETEXT, |annot| {
                    annot.set_string(c"Contents", text)?;
                    annot.set_string(c"DA", &default_appearance("Helv", *size, *color))?;
                    annot.set_border(0.0)?;
                    self.lay_out_text(&page, annot, *at, text, *size, *color)?;
                    annot.set_flags(ffi::FPDF_ANNOT_FLAG_PRINT)
                })
            }
            NewAnnotation::Note { at, text, color } => {
                let rect = geometry.rect_to_user(Rect::new(
                    at.0,
                    at.1,
                    at.0 + NOTE_SIZE,
                    at.1 + NOTE_SIZE,
                ));
                page.build(ffi::FPDF_ANNOT_TEXT, |annot| {
                    annot.set_rect(rect)?;
                    annot.set_string(c"Contents", text)?;
                    annot.set_color(*color, 1.0)?;
                    annot.set_appearance(&note_appearance(rect, *color))?;
                    annot.set_flags(ffi::FPDF_ANNOT_FLAG_PRINT)
                })
            }
            NewAnnotation::Image {
                rect,
                width,
                height,
                rgba,
            } => {
                let bgra = to_bgra(*width, *height, rgba)?;
                if !(rect.width() > 0.0 && rect.height() > 0.0) {
                    return Err(Error::Invalid("a picture needs a rect with an area".into()));
                }
                let user = geometry.rect_to_user(*rect);
                let matrix = geometry.upright_matrix(*rect);
                page.build(ffi::FPDF_ANNOT_STAMP, |annot| {
                    annot.set_rect(user)?;
                    let image = page.image(*width, *height, bgra, matrix)?;
                    annot.append_object(image)?;
                    annot.set_flags(ffi::FPDF_ANNOT_FLAG_PRINT)
                })
            }
        }
    }

    fn shift(&self, annotation: &Annotation, dx: f32, dy: f32) -> Result<()> {
        let page = self.page(annotation.id.page)?;
        let annot = page.annotation(annotation)?;
        let geometry = page.geometry;
        let origin = geometry.to_user((0.0, 0.0));
        let moved = geometry.to_user((dx, dy));
        let (ux, uy) = (moved.0 - origin.0, moved.1 - origin.1);
        let shift = |(x, y): (f32, f32)| (x + ux, y + uy);
        let (left, bottom, right, top) = annot
            .rect()
            .ok_or_else(|| Error::Failed("the annotation has no place on the page".into()))?;
        match annot.subtype() {
            ffi::FPDF_ANNOT_HIGHLIGHT
            | ffi::FPDF_ANNOT_UNDERLINE
            | ffi::FPDF_ANNOT_SQUIGGLY
            | ffi::FPDF_ANNOT_STRIKEOUT => {
                // Drawn from the quads, so drawn again where they now are.
                annot.clear_appearance();
                for (index, quad) in annot.quads().into_iter().enumerate() {
                    annot.set_quad(index, &quad.map(shift))?;
                }
            }
            ffi::FPDF_ANNOT_INK => {
                let strokes: Vec<Vec<(f32, f32)>> = annot
                    .strokes()
                    .into_iter()
                    .map(|stroke| stroke.into_iter().map(shift).collect())
                    .collect();
                annot.remove_strokes()?;
                for stroke in &strokes {
                    annot.add_stroke(stroke)?;
                }
                return annot.redraw_ink();
            }
            // The rest keep their appearance, which is drawn into the rect
            // wherever it is.
            _ => {}
        }
        annot.set_rect((left + ux, bottom + uy, right + ux, top + uy))
    }

    fn set_contents(&self, annotation: &Annotation, text: &str) -> Result<()> {
        let page = self.page(annotation.id.page)?;
        let annot = page.annotation(annotation)?;
        match &annotation.kind {
            AnnotKind::Note { .. } => annot.set_string(c"Contents", text),
            AnnotKind::Text { .. } => {
                let (at, _, size, color) = text_box_state(&page, &annot);
                annot.set_string(c"Contents", text)?;
                self.lay_out_text(&page, &annot, at, text, size, color)
            }
            _ => Err(Error::Invalid(
                "only a text box or a note has text to change".into(),
            )),
        }
    }

    fn set_color(&self, annotation: &Annotation, color: Rgb) -> Result<()> {
        let page = self.page(annotation.id.page)?;
        let annot = page.annotation(annotation)?;
        match &annotation.kind {
            AnnotKind::Text { .. } => {
                let (at, text, size, _) = text_box_state(&page, &annot);
                let (font, _, _) = parse_appearance(&annot.string(c"DA"));
                annot.set_string(
                    c"DA",
                    &default_appearance(font.as_deref().unwrap_or("Helv"), size, color),
                )?;
                self.lay_out_text(&page, &annot, at, &text, size, color)
            }
            AnnotKind::Markup { .. } | AnnotKind::Ink { .. } | AnnotKind::Note { .. } => {
                let opacity = annot.number(c"CA").unwrap_or(1.0);
                // PDFium won't recolour over an appearance; it draws a new
                // one from the colour.
                annot.clear_appearance();
                annot.set_color(color, opacity)?;
                match annotation.kind {
                    AnnotKind::Ink { .. } => annot.redraw_ink(),
                    // PDFium draws every note yellow, so it gets its own.
                    AnnotKind::Note { .. } => match annot.rect() {
                        Some(rect) => annot.set_appearance(&note_appearance(rect, color)),
                        None => Ok(()),
                    },
                    _ => Ok(()),
                }
            }
            _ => Err(Error::Invalid("a stamp has no colour of its own".into())),
        }
    }

    /// Sizes a text box to its text, top-left corner at `at` as shown, and
    /// draws its appearance: the text in Helvetica, line by line, upright
    /// on the shown page. PDFium's own drawing of a text box runs every line
    /// into one and frames it, so it isn't left to that; without the font
    /// the box is left for each viewer to draw.
    fn lay_out_text(
        &self,
        page: &PageHandle<'_>,
        annot: &AnnotHandle<'_>,
        at: (f32, f32),
        text: &str,
        size: f32,
        color: Rgb,
    ) -> Result<()> {
        let font = self.font(c"Helvetica");
        let shown = match &font {
            Some(font) => text_box(at, text, size, |line| font.measure(line, size)),
            None => text_box(at, text, size, |line| estimate_width(line, size)),
        };
        annot.set_rect(page.geometry.rect_to_user(shown))?;
        match font {
            Some(font) => annot.set_appearance(&font.draw(text, size, color, shown, page.geometry)),
            None => {
                annot.clear_appearance();
                Ok(())
            }
        }
    }

    /// One of the standard fonts, loaded into the document.
    fn font(&self, name: &CStr) -> Option<Font> {
        // SAFETY: `raw` is live and `name` is a C string.
        let raw = unsafe { (self.ffi.FPDFText_LoadStandardFont)(self.raw, name.as_ptr()) };
        (!raw.is_null()).then_some(Font { ffi: self.ffi, raw })
    }

    fn remove(&self, page_index: usize, indices: &BTreeSet<usize>) -> Result<()> {
        let page = self.page(page_index)?;
        for index in indices.iter().rev() {
            let raw_index =
                c_int::try_from(*index).map_err(|_| Error::Invalid("no such annotation".into()))?;
            // SAFETY: the page is live; no handle to the annotation is open.
            if unsafe { (self.ffi.FPDFPage_RemoveAnnot)(page.raw, raw_index) } == 0 {
                return Err(Error::Failed(format!(
                    "PDFium wouldn't delete the annotation at {}",
                    describe(AnnotId {
                        page: page_index,
                        index: *index
                    })
                )));
            }
        }
        Ok(())
    }
}

/// A loaded font; closed on drop.
struct Font {
    ffi: &'static Bindings,
    raw: ffi::FPDF_FONT,
}

impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: the font is live and closed only here.
        unsafe { (self.ffi.FPDFFont_Close)(self.raw) };
    }
}

// SAFETY (every method below): `self.raw` is a live font, out-pointers
// point at locals, and glyph paths and segments are only read while the
// font that owns them is open.
impl Font {
    fn advance(&self, character: char, size: f32) -> f32 {
        let mut width = 0.0;
        let ok = unsafe {
            (self.ffi.FPDFFont_GetGlyphWidth)(self.raw, u32::from(character), size, &mut width)
        };
        if ok != 0 && width > 0.0 {
            width
        } else {
            estimate_width(" ", size)
        }
    }

    fn measure(&self, line: &str, size: f32) -> f32 {
        line.chars()
            .map(|character| self.advance(character, size))
            .sum()
    }

    fn ascent(&self, size: f32) -> f32 {
        let mut ascent = 0.0;
        let ok = unsafe { (self.ffi.FPDFFont_GetAscent)(self.raw, size, &mut ascent) };
        if ok != 0 {
            ascent.clamp(0.6 * size, size)
        } else {
            0.8 * size
        }
    }

    /// Appends `character`'s outline at `(x, y)` as path operators, and
    /// answers whether it drew anything.
    fn outline(&self, character: char, size: f32, (x, y): (f32, f32), out: &mut String) -> bool {
        let ffi = self.ffi;
        // The outline comes in ems whatever size is asked for, so it is
        // asked for at one and scaled here.
        let path = unsafe { (ffi.FPDFFont_GetGlyphPath)(self.raw, u32::from(character), 1.0) };
        if path.is_null() {
            return false;
        }
        let count = unsafe { (ffi.FPDFGlyphPath_CountGlyphSegments)(path) };
        let mut curve = Vec::with_capacity(3);
        let mut drew = false;
        for index in 0..count.max(0) {
            let segment = unsafe { (ffi.FPDFGlyphPath_GetGlyphPathSegment)(path, index) };
            if segment.is_null() {
                continue;
            }
            let (mut px, mut py) = (0.0, 0.0);
            if unsafe { (ffi.FPDFPathSegment_GetPoint)(segment, &mut px, &mut py) } == 0 {
                continue;
            }
            let point = format!("{} {}", number(x + px * size), number(y + py * size));
            match unsafe { (ffi.FPDFPathSegment_GetType)(segment) } {
                ffi::FPDF_SEGMENT_MOVETO => out.push_str(&format!("{point} m\n")),
                ffi::FPDF_SEGMENT_LINETO => out.push_str(&format!("{point} l\n")),
                ffi::FPDF_SEGMENT_BEZIERTO => {
                    curve.push(point);
                    if curve.len() == 3 {
                        out.push_str(&format!("{} c\n", curve.join(" ")));
                        curve.clear();
                    }
                }
                _ => continue,
            }
            drew = true;
            if unsafe { (ffi.FPDFPathSegment_GetClose)(segment) } != 0 {
                out.push_str("h\n");
            }
        }
        drew
    }

    /// A text box's appearance: `text` filled as outlines, so the stream
    /// needs no font of its own, laid out in `shown` and turned upright on
    /// the shown page.
    fn draw(
        &self,
        text: &str,
        size: f32,
        color: Rgb,
        shown: Rect,
        geometry: PageGeometry,
    ) -> String {
        let (width, height) = (shown.width().max(1.0), shown.height().max(1.0));
        let [a, b, c, d, e, f] = geometry.upright_matrix(shown);
        let mut out = format!(
            "q {} {} {} {} {} {} cm {} {} {} rg\n",
            number(a / width),
            number(b / width),
            number(c / height),
            number(d / height),
            number(e),
            number(f),
            number(color.0.clamp(0.0, 1.0)),
            number(color.1.clamp(0.0, 1.0)),
            number(color.2.clamp(0.0, 1.0))
        );
        let ascent = self.ascent(size);
        for (row, line) in text.split('\n').enumerate() {
            let baseline = height - TEXT_PADDING - ascent - row as f32 * LINE_HEIGHT * size;
            let mut x = TEXT_PADDING;
            for character in line.chars() {
                if self.outline(character, size, (x, baseline), &mut out) {
                    out.push_str("f\n");
                }
                x += self.advance(character, size);
            }
        }
        out.push_str("Q\n");
        out
    }
}

/// One loaded page, with the form's view of it; closed on drop.
struct PageHandle<'d> {
    doc: &'d Doc,
    raw: ffi::FPDF_PAGE,
    geometry: PageGeometry,
    /// Whether the form has its view of the page, which form input needs.
    with_form: bool,
}

impl Drop for PageHandle<'_> {
    fn drop(&mut self) {
        let doc = self.doc;
        // SAFETY: the page is live and every annotation handle borrowing it
        // has been dropped already.
        unsafe {
            if self.with_form {
                (doc.ffi.FORM_ForceToKillFocus)(doc.form);
                (doc.ffi.FORM_OnBeforeClosePage)(self.raw, doc.form);
            }
            (doc.ffi.FPDF_ClosePage)(self.raw);
        }
    }
}

impl PageHandle<'_> {
    /// Gives the form its view of the page, once.
    fn attach_form(&mut self) {
        if self.with_form || self.doc.form.is_null() {
            return;
        }
        // SAFETY: the page and the form are live; the view is taken down
        // in `drop`.
        unsafe { (self.doc.ffi.FORM_OnAfterLoadPage)(self.raw, self.doc.form) };
        self.with_form = true;
    }

    fn annot_count(&self) -> usize {
        // SAFETY: the page is live.
        unsafe { (self.doc.ffi.FPDFPage_GetAnnotCount)(self.raw) }.max(0) as usize
    }

    fn annot(&self, index: usize) -> Result<AnnotHandle<'_>> {
        let missing = || Error::Invalid("that annotation isn't on the page any more".into());
        let raw_index = c_int::try_from(index).map_err(|_| missing())?;
        // SAFETY: the page is live; a bad index yields null.
        let raw = unsafe { (self.doc.ffi.FPDFPage_GetAnnot)(self.raw, raw_index) };
        if raw.is_null() {
            return Err(missing());
        }
        Ok(AnnotHandle {
            ffi: self.doc.ffi,
            raw,
            _page: PhantomData,
        })
    }

    /// The widget at `index`, checked to be one.
    fn widget(&self, index: usize) -> Result<AnnotHandle<'_>> {
        let annot = self.annot(index)?;
        if annot.subtype() != ffi::FPDF_ANNOT_WIDGET || self.doc.form.is_null() {
            return Err(Error::Invalid(
                "that field isn't on the page any more".into(),
            ));
        }
        Ok(annot)
    }

    /// The annotation `annotation` names, checked to still be that kind.
    fn annotation(&self, annotation: &Annotation) -> Result<AnnotHandle<'_>> {
        let annot = self.annot(annotation.id.index)?;
        if kind_of(annot.subtype()) != std::mem::discriminant(&annotation.kind) {
            return Err(Error::Invalid(
                "that annotation isn't on the page any more".into(),
            ));
        }
        Ok(annot)
    }

    /// The index of the annotation `annot` points to under `key`.
    fn linked_index(&self, annot: &AnnotHandle<'_>, key: &CStr) -> Option<usize> {
        let ffi = self.doc.ffi;
        // SAFETY: the annotation is live and `key` is a C string; the linked
        // handle, when there is one, is closed before returning.
        unsafe {
            let linked = (ffi.FPDFAnnot_GetLinkedAnnot)(annot.raw, key.as_ptr());
            if linked.is_null() {
                return None;
            }
            let index = (ffi.FPDFPage_GetAnnotIndex)(self.raw, linked);
            (ffi.FPDFPage_CloseAnnot)(linked);
            usize::try_from(index).ok()
        }
    }

    /// Creates an annotation of `subtype` and fills it in with `fill`,
    /// taking it off the page again if that fails.
    fn build(
        &self,
        subtype: c_int,
        fill: impl FnOnce(&AnnotHandle<'_>) -> Result<()>,
    ) -> Result<()> {
        let ffi = self.doc.ffi;
        // SAFETY: the page is live.
        let raw = unsafe { (ffi.FPDFPage_CreateAnnot)(self.raw, subtype) };
        if raw.is_null() {
            return Err(Error::Failed("PDFium wouldn't add the annotation".into()));
        }
        let annot = AnnotHandle {
            ffi,
            raw,
            _page: PhantomData,
        };
        let filled = fill(&annot);
        if filled.is_err() {
            // SAFETY: the page and the new annotation are live.
            let index = unsafe { (ffi.FPDFPage_GetAnnotIndex)(self.raw, annot.raw) };
            drop(annot);
            if index >= 0 {
                // SAFETY: the page is live and the handle was just closed.
                unsafe { (ffi.FPDFPage_RemoveAnnot)(self.raw, index) };
            }
        }
        filled
    }

    /// A new image object holding `bgra`, placed by `matrix`.
    fn image(
        &self,
        width: u32,
        height: u32,
        mut bgra: Vec<u8>,
        matrix: [f32; 6],
    ) -> Result<PageObject> {
        let ffi = self.doc.ffi;
        let too_big = || Error::Invalid("the picture is too big".into());
        let w = c_int::try_from(width).map_err(|_| too_big())?;
        let h = c_int::try_from(height).map_err(|_| too_big())?;
        let stride = w.checked_mul(4).ok_or_else(too_big)?;
        // SAFETY: `bgra` holds `height` rows of `stride` bytes (checked by
        // `to_bgra`) and outlives the bitmap, which is destroyed below after
        // `SetBitmap` has copied it into the image.
        unsafe {
            let bitmap = (ffi.FPDFBitmap_CreateEx)(
                w,
                h,
                ffi::FPDFBITMAP_BGRA,
                bgra.as_mut_ptr().cast::<c_void>(),
                stride,
            );
            if bitmap.is_null() {
                return Err(Error::Failed("PDFium couldn't hold the picture".into()));
            }
            let raw = (ffi.FPDFPageObj_NewImageObj)(self.doc.raw);
            if raw.is_null() {
                (ffi.FPDFBitmap_Destroy)(bitmap);
                return Err(Error::Failed("PDFium couldn't make an image".into()));
            }
            let object = PageObject { ffi, raw };
            let mut page = self.raw;
            let set = (ffi.FPDFImageObj_SetBitmap)(&mut page, 1, object.raw, bitmap);
            (ffi.FPDFBitmap_Destroy)(bitmap);
            if set == 0 {
                return Err(Error::Failed("PDFium couldn't take the picture".into()));
            }
            let [a, b, c, d, e, f] = matrix;
            let matrix = ffi::FS_MATRIX { a, b, c, d, e, f };
            if (ffi.FPDFPageObj_SetMatrix)(object.raw, &matrix) == 0 {
                return Err(Error::Failed("PDFium couldn't place the picture".into()));
            }
            Ok(object)
        }
    }
}

/// A page object not yet given to anything; destroyed on drop unless it was.
struct PageObject {
    ffi: &'static Bindings,
    raw: ffi::FPDF_PAGEOBJECT,
}

impl Drop for PageObject {
    fn drop(&mut self) {
        // SAFETY: the object is live and owned by nothing else.
        unsafe { (self.ffi.FPDFPageObj_Destroy)(self.raw) };
    }
}

/// One annotation of a loaded page; closed on drop.
struct AnnotHandle<'p> {
    ffi: &'static Bindings,
    raw: ffi::FPDF_ANNOTATION,
    _page: PhantomData<&'p ()>,
}

impl Drop for AnnotHandle<'_> {
    fn drop(&mut self) {
        // SAFETY: the handle is live and closed only here.
        unsafe { (self.ffi.FPDFPage_CloseAnnot)(self.raw) };
    }
}

// SAFETY (every method below): `self.raw` is a live annotation handle, keys
// are C strings, and every out-pointer points at a local of the declared
// type; buffers come from `wide_string`, which passes their true length.
impl AnnotHandle<'_> {
    fn subtype(&self) -> c_int {
        unsafe { (self.ffi.FPDFAnnot_GetSubtype)(self.raw) }
    }

    fn rect(&self) -> Option<UserRect> {
        let mut rect = ffi::FS_RECTF::default();
        (unsafe { (self.ffi.FPDFAnnot_GetRect)(self.raw, &mut rect) } != 0).then_some((
            rect.left,
            rect.bottom,
            rect.right,
            rect.top,
        ))
    }

    fn set_rect(&self, (left, bottom, right, top): UserRect) -> Result<()> {
        let rect = ffi::FS_RECTF {
            left,
            top,
            right,
            bottom,
        };
        check(
            unsafe { (self.ffi.FPDFAnnot_SetRect)(self.raw, &rect) },
            "place the annotation",
        )
    }

    fn flags(&self) -> c_int {
        unsafe { (self.ffi.FPDFAnnot_GetFlags)(self.raw) }
    }

    fn set_flags(&self, flags: c_int) -> Result<()> {
        check(
            unsafe { (self.ffi.FPDFAnnot_SetFlags)(self.raw, flags) },
            "set the annotation's flags",
        )
    }

    fn has_key(&self, key: &CStr) -> bool {
        unsafe { (self.ffi.FPDFAnnot_HasKey)(self.raw, key.as_ptr()) != 0 }
    }

    fn string(&self, key: &CStr) -> String {
        wide_string(|buffer, length| unsafe {
            (self.ffi.FPDFAnnot_GetStringValue)(self.raw, key.as_ptr(), buffer, length)
        })
    }

    fn set_string(&self, key: &CStr, value: &str) -> Result<()> {
        let value = utf16z(value);
        check(
            unsafe { (self.ffi.FPDFAnnot_SetStringValue)(self.raw, key.as_ptr(), value.as_ptr()) },
            "write the annotation",
        )
    }

    fn number(&self, key: &CStr) -> Option<f32> {
        let mut value = 0.0;
        (unsafe { (self.ffi.FPDFAnnot_GetNumberValue)(self.raw, key.as_ptr(), &mut value) } != 0)
            .then_some(value)
    }

    fn color(&self) -> Option<Rgb> {
        let (mut r, mut g, mut b, mut a): (c_uint, c_uint, c_uint, c_uint) = (0, 0, 0, 0);
        let ok = unsafe {
            (self.ffi.FPDFAnnot_GetColor)(
                self.raw,
                ffi::FPDFANNOT_COLORTYPE_COLOR,
                &mut r,
                &mut g,
                &mut b,
                &mut a,
            )
        };
        (ok != 0).then(|| Rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
    }

    fn set_color(&self, color: Rgb, opacity: f32) -> Result<()> {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as c_uint;
        check(
            unsafe {
                (self.ffi.FPDFAnnot_SetColor)(
                    self.raw,
                    ffi::FPDFANNOT_COLORTYPE_COLOR,
                    channel(color.0),
                    channel(color.1),
                    channel(color.2),
                    channel(opacity),
                )
            },
            "colour the annotation",
        )
    }

    /// The normal appearance's content stream, empty when it has none.
    fn appearance(&self) -> String {
        wide_string(|buffer, length| unsafe {
            (self.ffi.FPDFAnnot_GetAP)(
                self.raw,
                ffi::FPDF_ANNOT_APPEARANCEMODE_NORMAL,
                buffer,
                length,
            )
        })
    }

    /// Sets the normal appearance to `content`, drawn in user space over
    /// the annotation's rect.
    fn set_appearance(&self, content: &str) -> Result<()> {
        let content = utf16z(content);
        check(
            unsafe {
                (self.ffi.FPDFAnnot_SetAP)(
                    self.raw,
                    ffi::FPDF_ANNOT_APPEARANCEMODE_NORMAL,
                    content.as_ptr(),
                )
            },
            "draw the annotation",
        )
    }

    /// Drops every appearance, so a viewer draws it from its entries.
    fn clear_appearance(&self) {
        unsafe {
            (self.ffi.FPDFAnnot_SetAP)(
                self.raw,
                ffi::FPDF_ANNOT_APPEARANCEMODE_NORMAL,
                std::ptr::null(),
            )
        };
    }

    fn quads(&self) -> Vec<[(f32, f32); 4]> {
        let count = unsafe { (self.ffi.FPDFAnnot_CountAttachmentPoints)(self.raw) };
        (0..count)
            .filter_map(|index| {
                let mut quad = ffi::FS_QUADPOINTSF::default();
                (unsafe { (self.ffi.FPDFAnnot_GetAttachmentPoints)(self.raw, index, &mut quad) }
                    != 0)
                    .then_some([
                        (quad.x1, quad.y1),
                        (quad.x2, quad.y2),
                        (quad.x3, quad.y3),
                        (quad.x4, quad.y4),
                    ])
            })
            .collect()
    }

    fn append_quad(&self, quad: &[(f32, f32); 4]) -> Result<()> {
        let quad = to_quadpoints(quad);
        check(
            unsafe { (self.ffi.FPDFAnnot_AppendAttachmentPoints)(self.raw, &quad) },
            "mark the text",
        )
    }

    fn set_quad(&self, index: usize, quad: &[(f32, f32); 4]) -> Result<()> {
        let quad = to_quadpoints(quad);
        check(
            unsafe { (self.ffi.FPDFAnnot_SetAttachmentPoints)(self.raw, index, &quad) },
            "move the marked text",
        )
    }

    fn strokes(&self) -> Vec<Vec<(f32, f32)>> {
        let count = unsafe { (self.ffi.FPDFAnnot_GetInkListCount)(self.raw) };
        (0..count)
            .map(|index| {
                let length = unsafe {
                    (self.ffi.FPDFAnnot_GetInkListPath)(self.raw, index, std::ptr::null_mut(), 0)
                };
                let mut points = vec![ffi::FS_POINTF::default(); length as usize];
                let filled = unsafe {
                    (self.ffi.FPDFAnnot_GetInkListPath)(
                        self.raw,
                        index,
                        points.as_mut_ptr(),
                        length,
                    )
                };
                points.truncate(filled.min(length) as usize);
                points.into_iter().map(|point| (point.x, point.y)).collect()
            })
            .collect()
    }

    fn add_stroke(&self, stroke: &[(f32, f32)]) -> Result<()> {
        let points: Vec<ffi::FS_POINTF> = stroke
            .iter()
            .map(|&(x, y)| ffi::FS_POINTF { x, y })
            .collect();
        let index =
            unsafe { (self.ffi.FPDFAnnot_AddInkStroke)(self.raw, points.as_ptr(), points.len()) };
        if index < 0 {
            return Err(Error::Failed("PDFium couldn't draw the stroke".into()));
        }
        Ok(())
    }

    fn remove_strokes(&self) -> Result<()> {
        check(
            unsafe { (self.ffi.FPDFAnnot_RemoveInkList)(self.raw) },
            "move the strokes",
        )
    }

    /// Drops an ink annotation's appearance for PDFium to draw again, with
    /// its rect shrunk to the bare strokes: drawing grows the rect by half
    /// the line width.
    fn redraw_ink(&self) -> Result<()> {
        self.clear_appearance();
        match user_bounds(self.strokes().into_iter().flatten(), 0.0) {
            Some(rect) => self.set_rect(rect),
            None => Ok(()),
        }
    }

    fn border_width(&self) -> Option<f32> {
        let (mut horizontal, mut vertical, mut width) = (0.0, 0.0, 0.0);
        let ok = unsafe {
            (self.ffi.FPDFAnnot_GetBorder)(self.raw, &mut horizontal, &mut vertical, &mut width)
        };
        (ok != 0).then_some(width)
    }

    fn set_border(&self, width: f32) -> Result<()> {
        check(
            unsafe { (self.ffi.FPDFAnnot_SetBorder)(self.raw, 0.0, 0.0, width) },
            "set the line width",
        )
    }

    /// Hands `object` to the annotation, which owns it from then on.
    fn append_object(&self, object: PageObject) -> Result<()> {
        let appended = unsafe { (self.ffi.FPDFAnnot_AppendObject)(self.raw, object.raw) };
        if appended == 0 {
            return Err(Error::Failed(
                "PDFium couldn't put the picture in the stamp".into(),
            ));
        }
        // The annotation owns it now; don't destroy it.
        std::mem::forget(object);
        Ok(())
    }
}

fn read_annotation(
    page: &PageHandle<'_>,
    annot: &AnnotHandle<'_>,
    id: AnnotId,
    subtype: c_int,
) -> Annotation {
    let geometry = page.geometry;
    let kind = match subtype {
        ffi::FPDF_ANNOT_HIGHLIGHT
        | ffi::FPDF_ANNOT_UNDERLINE
        | ffi::FPDF_ANNOT_SQUIGGLY
        | ffi::FPDF_ANNOT_STRIKEOUT => AnnotKind::Markup {
            markup: match subtype {
                ffi::FPDF_ANNOT_HIGHLIGHT => Markup::Highlight,
                ffi::FPDF_ANNOT_UNDERLINE => Markup::Underline,
                ffi::FPDF_ANNOT_SQUIGGLY => Markup::Squiggly,
                _ => Markup::StrikeOut,
            },
            quads: annot
                .quads()
                .into_iter()
                .map(|quad| geometry.quad_to_shown(quad))
                .collect(),
        },
        ffi::FPDF_ANNOT_INK => AnnotKind::Ink {
            strokes: annot
                .strokes()
                .into_iter()
                .map(|stroke| {
                    stroke
                        .into_iter()
                        .map(|point| geometry.to_shown(point))
                        .collect()
                })
                .collect(),
            width: annot
                .border_width()
                .filter(|width| *width > 0.0)
                .unwrap_or(1.0),
        },
        ffi::FPDF_ANNOT_FREETEXT => AnnotKind::Text {
            text: annot.string(c"Contents"),
            size: parse_appearance(&annot.string(c"DA")).1.unwrap_or(12.0),
        },
        ffi::FPDF_ANNOT_TEXT => AnnotKind::Note {
            text: annot.string(c"Contents"),
        },
        ffi::FPDF_ANNOT_STAMP => AnnotKind::Stamp,
        other => AnnotKind::Other {
            subtype: subtype_name(other)
                .map(str::to_owned)
                .unwrap_or_else(|| annot.string(c"Subtype")),
        },
    };
    let color = match &kind {
        // A text box's own colour is its text's, in its appearance string.
        AnnotKind::Text { .. } => parse_appearance(&annot.string(c"DA"))
            .2
            .or_else(|| annot.color()),
        AnnotKind::Stamp | AnnotKind::Other { .. } => annot.color(),
        _ => annot.color().or_else(|| {
            let stroked = matches!(
                kind,
                AnnotKind::Ink { .. }
                    | AnnotKind::Markup {
                        markup: Markup::Underline | Markup::StrikeOut | Markup::Squiggly,
                        ..
                    }
            );
            appearance_color(&annot.appearance(), stroked)
        }),
    };
    let locked = annot.flags() & (ffi::FPDF_ANNOT_FLAG_READONLY | ffi::FPDF_ANNOT_FLAG_LOCKED) != 0;
    let editable = !locked
        && matches!(
            kind,
            AnnotKind::Markup { .. }
                | AnnotKind::Ink { .. }
                | AnnotKind::Text { .. }
                | AnnotKind::Note { .. }
                | AnnotKind::Stamp
        );
    let author = annot.string(c"T");
    Annotation {
        id,
        rect: annot
            .rect()
            .map(|rect| geometry.rect_to_shown(rect))
            .unwrap_or_default(),
        kind,
        color,
        author: Some(author).filter(|author| !author.is_empty()),
        editable,
    }
}

/// A text box as it is now, earlier edits in the same call included: its
/// shown top-left corner, text, size and colour.
fn text_box_state(
    page: &PageHandle<'_>,
    annot: &AnnotHandle<'_>,
) -> ((f32, f32), String, f32, Rgb) {
    let rect = annot
        .rect()
        .map(|rect| page.geometry.rect_to_shown(rect))
        .unwrap_or_default();
    let (_, size, color) = parse_appearance(&annot.string(c"DA"));
    (
        (rect.left, rect.top),
        annot.string(c"Contents"),
        size.filter(|size| *size > 0.0).unwrap_or(12.0),
        color.unwrap_or(Rgb(0.0, 0.0, 0.0)),
    )
}

/// Which [`AnnotKind`] an annotation subtype reads as.
fn kind_of(subtype: c_int) -> std::mem::Discriminant<AnnotKind> {
    std::mem::discriminant(&match subtype {
        ffi::FPDF_ANNOT_HIGHLIGHT
        | ffi::FPDF_ANNOT_UNDERLINE
        | ffi::FPDF_ANNOT_SQUIGGLY
        | ffi::FPDF_ANNOT_STRIKEOUT => AnnotKind::Markup {
            markup: Markup::Highlight,
            quads: Vec::new(),
        },
        ffi::FPDF_ANNOT_INK => AnnotKind::Ink {
            strokes: Vec::new(),
            width: 0.0,
        },
        ffi::FPDF_ANNOT_FREETEXT => AnnotKind::Text {
            text: String::new(),
            size: 0.0,
        },
        ffi::FPDF_ANNOT_TEXT => AnnotKind::Note {
            text: String::new(),
        },
        ffi::FPDF_ANNOT_STAMP => AnnotKind::Stamp,
        _ => AnnotKind::Other {
            subtype: String::new(),
        },
    })
}

fn markup_subtype(markup: Markup) -> c_int {
    match markup {
        Markup::Highlight => ffi::FPDF_ANNOT_HIGHLIGHT,
        Markup::Underline => ffi::FPDF_ANNOT_UNDERLINE,
        Markup::StrikeOut => ffi::FPDF_ANNOT_STRIKEOUT,
        Markup::Squiggly => ffi::FPDF_ANNOT_SQUIGGLY,
    }
}

fn subtype_name(subtype: c_int) -> Option<&'static str> {
    Some(match subtype {
        2 => "Link",
        4 => "Line",
        5 => "Square",
        6 => "Circle",
        7 => "Polygon",
        8 => "PolyLine",
        14 => "Caret",
        17 => "FileAttachment",
        18 => "Sound",
        19 => "Movie",
        21 => "Screen",
        22 => "PrinterMark",
        23 => "TrapNet",
        24 => "Watermark",
        25 => "3D",
        26 => "RichMedia",
        28 => "Redact",
        _ => return None,
    })
}

/// The box a text annotation's text fills, its top-left corner at `at`:
/// as wide as its widest line by `measure`, and `LINE_HEIGHT` times the
/// size per line down, with padding all round.
pub(crate) fn text_box(
    at: (f32, f32),
    text: &str,
    size: f32,
    measure: impl Fn(&str) -> f32,
) -> Rect {
    let (count, widest) = text
        .split('\n')
        .fold((0usize, 0.0f32), |(count, widest), line| {
            (count + 1, widest.max(measure(line)))
        });
    let width = widest.max(estimate_width(" ", size)) + 2.0 * TEXT_PADDING;
    let height = count.max(1) as f32 * LINE_HEIGHT * size + 2.0 * TEXT_PADDING;
    Rect::new(at.0, at.1, at.0 + width, at.1 + height)
}

/// A line's width without a font to measure it: half the size per
/// character.
fn estimate_width(line: &str, size: f32) -> f32 {
    line.chars().count() as f32 * 0.5 * size
}

/// A sticky note's icon over `rect`, filled with `color`: a speech
/// bubble with three lines of text in it, drawn for a 20-point square and
/// stretched to the rect.
pub(crate) fn note_appearance((left, bottom, right, top): UserRect, color: Rgb) -> String {
    let sx = (right - left) / NOTE_SIZE;
    let sy = (top - bottom) / NOTE_SIZE;
    let x = |at: f32| number(left + at * sx);
    let y = |at: f32| number(bottom + at * sy);
    let mut out = format!(
        "{} {} {} rg 0 0 0 RG 1 w\n",
        number(color.0.clamp(0.0, 1.0)),
        number(color.1.clamp(0.0, 1.0)),
        number(color.2.clamp(0.0, 1.0))
    );
    let bubble = [
        (0.5, 4.5),
        (0.5, 19.5),
        (19.5, 19.5),
        (19.5, 4.5),
        (8.5, 4.5),
        (6.5, 0.5),
        (4.5, 4.5),
    ];
    for (index, (px, py)) in bubble.iter().enumerate() {
        let op = if index == 0 { "m" } else { "l" };
        out.push_str(&format!("{} {} {op}\n", x(*px), y(*py)));
    }
    out.push_str("h\n");
    for line in [15.75, 12.0, 8.25] {
        out.push_str(&format!(
            "{} {} m {} {} l\n",
            x(2.5),
            y(line),
            x(17.5),
            y(line)
        ));
    }
    out.push_str("B*\n");
    out
}

/// A `/DA` string: font, size and fill colour.
pub(crate) fn default_appearance(font: &str, size: f32, color: Rgb) -> String {
    format!(
        "/{font} {} Tf {} {} {} rg",
        number(size),
        number(color.0.clamp(0.0, 1.0)),
        number(color.1.clamp(0.0, 1.0)),
        number(color.2.clamp(0.0, 1.0))
    )
}

/// The font name, size and fill colour a `/DA` string sets.
pub(crate) fn parse_appearance(da: &str) -> (Option<String>, Option<f32>, Option<Rgb>) {
    let tokens: Vec<&str> = da.split_whitespace().collect();
    let mut font = None;
    let mut size = None;
    let mut color = None;
    for (index, token) in tokens.iter().enumerate() {
        let before = |back: usize| index.checked_sub(back).and_then(|at| tokens.get(at));
        match *token {
            "Tf" => {
                size = before(1).and_then(|value| value.parse::<f32>().ok());
                font = before(2)
                    .and_then(|name| name.strip_prefix('/'))
                    .map(str::to_owned);
            }
            "rg" | "g" | "k" => color = color_operands(&tokens[..index], token).or(color),
            _ => {}
        }
    }
    (font, size, color)
}

/// The colour an appearance stream first paints with: its stroke colour
/// when `stroked`, its fill colour otherwise, falling back to the other.
pub(crate) fn appearance_color(stream: &str, stroked: bool) -> Option<Rgb> {
    let tokens: Vec<&str> = stream.split_whitespace().collect();
    let mut fallback = None;
    for (index, token) in tokens.iter().enumerate() {
        let stroke = match *token {
            "RG" | "G" | "K" => true,
            "rg" | "g" | "k" => false,
            _ => continue,
        };
        if let Some(color) = color_operands(&tokens[..index], token) {
            if stroke == stroked {
                return Some(color);
            }
            fallback = fallback.or(Some(color));
        }
    }
    fallback
}

/// The colour an `rg`/`g`/`k` (or stroking) operator sets, from the
/// operands just before it.
fn color_operands(before: &[&str], operator: &str) -> Option<Rgb> {
    let take = |count: usize| -> Option<Vec<f32>> {
        let start = before.len().checked_sub(count)?;
        before[start..]
            .iter()
            .map(|value| value.parse::<f32>().ok())
            .collect()
    };
    match operator {
        "rg" | "RG" => take(3).map(|v| Rgb(v[0], v[1], v[2])),
        "g" | "G" => take(1).map(|v| Rgb(v[0], v[0], v[0])),
        "k" | "K" => take(4).map(|v| {
            let white = 1.0 - v[3];
            Rgb(
                (1.0 - v[0]) * white,
                (1.0 - v[1]) * white,
                (1.0 - v[2]) * white,
            )
        }),
        _ => None,
    }
}

/// A number as PDF content writes it: no exponent, no trailing zeros.
fn number(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

/// RGBA pixels, unpremultiplied, as PDFium's BGRA.
fn to_bgra(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(rgba.len()) {
        return Err(Error::Invalid(format!(
            "a {width}×{height} picture needs {} bytes, not {}",
            expected.map_or_else(|| "too many".into(), |bytes| bytes.to_string()),
            rgba.len()
        )));
    }
    Ok(rgba
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
        .collect())
}

fn user_bounds(points: impl IntoIterator<Item = (f32, f32)>, margin: f32) -> Option<UserRect> {
    // `bounds` keeps the larger y as `bottom`; in user space that is the top.
    bounds(points, margin).map(|rect| (rect.left, rect.top, rect.right, rect.bottom))
}

fn to_quadpoints(quad: &[(f32, f32); 4]) -> ffi::FS_QUADPOINTSF {
    ffi::FS_QUADPOINTSF {
        x1: quad[0].0,
        y1: quad[0].1,
        x2: quad[1].0,
        y2: quad[1].1,
        x3: quad[2].0,
        y3: quad[2].1,
        x4: quad[3].0,
        y4: quad[3].1,
    }
}

fn refuse_read_only(field: &Field) -> Result<()> {
    if field.read_only {
        return Err(Error::Invalid(format!("“{}” is read-only", field.name)));
    }
    Ok(())
}

fn check(ok: ffi::FPDF_BOOL, what: &str) -> Result<()> {
    if ok == 0 {
        return Err(Error::Failed(format!("PDFium couldn't {what}")));
    }
    Ok(())
}

fn describe(id: AnnotId) -> String {
    format!("page {}, #{}", id.page + 1, id.index)
}

/// Puts which edit failed in front of an error about it.
fn in_edit(number: usize, error: Error) -> Error {
    match error {
        Error::Invalid(why) => Error::Invalid(format!("edit {}: {why}", number + 1)),
        Error::Failed(why) => Error::Failed(format!("edit {}: {why}", number + 1)),
        other => other,
    }
}

/// Text with every line break as `\n`: PDFium writes a typed one as
/// `\r\n`, and files hold any of the three kinds.
fn one_line_break(text: String) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text
    }
}

/// A UTF-16 string with a nul at the end, as PDFium takes text.
fn utf16z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Calls one of PDFium's "length first, then fill" UTF-16 getters: `get`
/// takes a buffer and its length in bytes and answers the bytes needed,
/// terminator included.
fn wide_string(mut get: impl FnMut(*mut ffi::FPDF_WCHAR, c_ulong) -> c_ulong) -> String {
    let needed = get(std::ptr::null_mut(), 0) as usize;
    if needed <= 2 {
        return String::new();
    }
    let mut buffer = vec![0u16; needed.div_ceil(2)];
    let length = (buffer.len() * 2) as c_ulong;
    let written = get(buffer.as_mut_ptr(), length) as usize;
    if written > needed {
        // It grew between the calls; the buffer was too short to be filled.
        return String::new();
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Collects what `FPDF_SaveAsCopy` writes.
#[repr(C)]
struct Sink {
    /// First, so the pointer PDFium hands back is the `Sink`'s too.
    write: ffi::FPDF_FILEWRITE,
    bytes: Vec<u8>,
}

unsafe extern "C" fn write_block(
    this: *mut ffi::FPDF_FILEWRITE,
    data: *const c_void,
    size: c_ulong,
) -> c_int {
    if this.is_null() || (data.is_null() && size > 0) {
        return 0;
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: `this` is the `write` field of the `Sink` that `Doc::save`
        // passed to `FPDF_SaveAsCopy`, which is `repr(C)` with it first and
        // is alive and otherwise untouched for the whole call.
        let sink = unsafe { &mut *this.cast::<Sink>() };
        if size > 0 {
            // SAFETY: PDFium passes `size` readable bytes at `data`.
            let block = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) };
            sink.bytes.extend_from_slice(block);
        }
        1
    }))
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_box_grows_with_lines_and_the_longest_line() {
        let estimate = |line: &str| estimate_width(line, 10.0);
        let one = text_box((10.0, 20.0), "abcd", 10.0, estimate);
        assert_eq!(
            one,
            Rect::new(10.0, 20.0, 10.0 + 20.0 + 4.0, 20.0 + 12.0 + 4.0)
        );
        let two = text_box((0.0, 0.0), "ab\nabcdef", 10.0, estimate);
        assert_eq!(two.width(), 30.0 + 4.0);
        assert_eq!(two.height(), 24.0 + 4.0);
        let empty = text_box((0.0, 0.0), "", 10.0, estimate);
        assert!(empty.width() > 0.0 && empty.height() > 0.0);
    }

    #[test]
    fn appearance_strings_round_trip() {
        let da = default_appearance("Helv", 14.0, Rgb(0.8, 0.0, 0.25));
        assert_eq!(da, "/Helv 14 Tf 0.8 0 0.25 rg");
        let (font, size, color) = parse_appearance(&da);
        assert_eq!(font.as_deref(), Some("Helv"));
        assert_eq!(size, Some(14.0));
        assert_eq!(color, Some(Rgb(0.8, 0.0, 0.25)));
        assert_eq!(parse_appearance("0 g /Cour 9.5 Tf").1, Some(9.5));
        assert_eq!(
            parse_appearance("0 g /Cour 9.5 Tf").2,
            Some(Rgb(0.0, 0.0, 0.0))
        );
    }

    #[test]
    fn appearance_color_prefers_the_asked_for_operator() {
        let stream = "/GS gs 1 0.9 0 rg 0 0 1 RG 10 10 m 20 20 l S";
        assert_eq!(appearance_color(stream, false), Some(Rgb(1.0, 0.9, 0.0)));
        assert_eq!(appearance_color(stream, true), Some(Rgb(0.0, 0.0, 1.0)));
        assert_eq!(
            appearance_color("0.5 G 1 w", false),
            Some(Rgb(0.5, 0.5, 0.5))
        );
        assert_eq!(appearance_color("q Q", false), None);
    }

    #[test]
    fn numbers_drop_trailing_zeros() {
        assert_eq!(number(12.0), "12");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(-0.0001), "0");
    }

    #[test]
    fn rgba_turns_into_bgra_and_checks_its_length() {
        assert_eq!(to_bgra(1, 1, &[1, 2, 3, 4]).unwrap(), vec![3, 2, 1, 4]);
        assert!(matches!(
            to_bgra(2, 1, &[1, 2, 3, 4]),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(to_bgra(0, 0, &[]), Err(Error::Invalid(_))));
    }
}
