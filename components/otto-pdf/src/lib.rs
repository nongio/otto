//! Editing a PDF in place: filling its form and marking it up.
//!
//! What a viewer needs to put controls over a page it has already drawn,
//! and to write back what the person did with them: the form's fields and
//! the page's annotations, read out in the page's own coordinates, and the
//! edits that change them — a field's value, an annotation added, moved,
//! recoloured or taken away — saved over the file.
//!
//! The work is PDFium's, loaded from `libpdfium.so` when first asked for (see
//! [`library`]). A field is filled through PDFium's form-fill environment,
//! which is what draws its new appearance, so the value shows in any viewer,
//! not only in one that redraws fields itself. PDFium keeps one global state
//! and is not safe to call from two threads at once, so one thread owns it
//! and every [`Document`] is a handle that talks to that thread.
//!
//! # Coordinates
//!
//! Everything here is in PDF points, from the top-left corner of the page as
//! it is shown: its crop box, turned by its `/Rotate`. That is the space a
//! page is laid out in by `pdfinfo`'s sizes, so a rect from here lands on the
//! rasterised page with no further conversion. The engine turns them into
//! the page's own bottom-up space and back.

// Rust guideline compliant 2026-02-21

// A `pub` item nothing outside the crate can reach is an accident: say
// `pub(crate)` or `pub(super)`, so the public API is what lib.rs exports.
#![warn(unreachable_pub)]

mod bindings;
mod engine;
mod geometry;
pub mod library;
mod worker;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

pub use worker::Document;

/// A rect on a page: points from the shown page's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left: left.min(right),
            top: top.min(bottom),
            right: left.max(right),
            bottom: top.max(bottom),
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn contains(&self, (x, y): (f32, f32)) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Self {
        Self {
            left: self.left + dx,
            top: self.top + dy,
            right: self.right + dx,
            bottom: self.bottom + dy,
        }
    }
}

/// An sRGB colour, each channel 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

/// One page: its size as shown, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Page {
    pub width: f32,
    pub height: f32,
}

/// Names a field's widget or an annotation in one [`Snapshot`]: the page,
/// and the annotation's place in the page's list. Edits renumber a page's
/// list, so an id is good for the snapshot it came from, and every edit
/// answers with a fresh one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnnotId {
    pub page: usize,
    pub index: usize,
}

/// What kind of field a widget belongs to, with what it holds.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    /// Text typed into a box.
    Text {
        value: String,
        multiline: bool,
        password: bool,
        /// The most characters it takes, if it limits them.
        max_len: Option<usize>,
        /// Laid out as one box per character.
        comb: bool,
    },
    /// A box ticked on or off.
    Checkbox { checked: bool },
    /// One of a group: ticking it unticks the others with the same name.
    Radio {
        checked: bool,
        /// What the group's value becomes when this one is ticked.
        export: String,
    },
    /// One choice from a list that drops down, optionally typed instead.
    Combo {
        options: Vec<String>,
        selected: Option<usize>,
        value: String,
        editable: bool,
    },
    /// Choices from a list shown open.
    List {
        options: Vec<String>,
        selected: Vec<usize>,
        multiple: bool,
    },
    /// A push button: it runs a script, which nothing here does.
    Button,
    /// A place for a digital signature, signed or not.
    Signature { signed: bool },
}

/// One widget of a form field: where it is and what it holds. A field shown
/// in two places (a radio group, a name repeated on every page) has one
/// widget each, with the same `name`.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub id: AnnotId,
    /// The field's full name, `parent.child`.
    pub name: String,
    /// What a reader is told it is for, when the form says (`/TU`).
    pub tooltip: Option<String>,
    pub rect: Rect,
    pub kind: FieldKind,
    pub read_only: bool,
    pub required: bool,
    /// The text size its appearance asks for; 0 means sized to fit.
    pub font_size: f32,
}

/// The kinds of annotation the person can make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Markup {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
}

/// What an annotation is.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotKind {
    /// Marked text: one quad per line of it, each as four corners in the
    /// order top-left, top-right, bottom-left, bottom-right.
    Markup {
        markup: Markup,
        quads: Vec<[(f32, f32); 4]>,
    },
    /// Freehand strokes.
    Ink {
        strokes: Vec<Vec<(f32, f32)>>,
        width: f32,
    },
    /// Text in a box on the page.
    Text { text: String, size: f32 },
    /// A sticky note: a pin, with what it says.
    Note { text: String },
    /// A stamp: a picture, a signature, a rubber stamp's word.
    Stamp,
    /// Anything else: a link, a shape, a file. Shown, never edited.
    Other { subtype: String },
}

/// One annotation on a page, form widgets aside.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub id: AnnotId,
    pub rect: Rect,
    pub kind: AnnotKind,
    pub color: Option<Rgb>,
    /// Who wrote it, when it says (`/T`).
    pub author: Option<String>,
    /// Whether the person may move, change or delete it here: markup, ink,
    /// text, notes and stamps that aren't locked.
    pub editable: bool,
}

/// What the document is now: its pages, every field's widget and every
/// other annotation, in page order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub pages: Vec<Page>,
    pub fields: Vec<Field>,
    pub annotations: Vec<Annotation>,
    /// Whether it carries a digital signature, so a save must append to the
    /// file rather than rewrite it, and an edit after it shows as one.
    pub signed: bool,
}

impl Snapshot {
    pub fn field(&self, id: AnnotId) -> Option<&Field> {
        self.fields.iter().find(|field| field.id == id)
    }

    pub fn annotation(&self, id: AnnotId) -> Option<&Annotation> {
        self.annotations
            .iter()
            .find(|annotation| annotation.id == id)
    }

    /// The topmost field widget at a point on `page`.
    pub fn field_at(&self, page: usize, at: (f32, f32)) -> Option<&Field> {
        self.fields
            .iter()
            .rev()
            .find(|field| field.id.page == page && field.rect.contains(at))
    }

    /// The topmost annotation at a point on `page`.
    pub fn annotation_at(&self, page: usize, at: (f32, f32)) -> Option<&Annotation> {
        self.annotations
            .iter()
            .rev()
            .find(|annotation| annotation.id.page == page && annotation.rect.contains(at))
    }
}

/// An annotation to add.
#[derive(Debug, Clone, PartialEq)]
pub enum NewAnnotation {
    /// Marked text, a quad per line (see [`AnnotKind::Markup`]).
    Markup {
        markup: Markup,
        quads: Vec<[(f32, f32); 4]>,
        color: Rgb,
    },
    /// Freehand strokes.
    Ink {
        strokes: Vec<Vec<(f32, f32)>>,
        width: f32,
        color: Rgb,
    },
    /// Text in a box whose top-left corner is `at`, sized to the text.
    Text {
        at: (f32, f32),
        text: String,
        size: f32,
        color: Rgb,
    },
    /// A sticky note pinned at `at`.
    Note {
        at: (f32, f32),
        text: String,
        color: Rgb,
    },
    /// A picture laid over `rect`: RGBA pixels, `width` × `height`,
    /// unpremultiplied. A signature drawn on screen arrives as one.
    Image {
        rect: Rect,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

/// A change to the document.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Type `text` into a text field or an editable combo, replacing what
    /// was there.
    SetText {
        field: AnnotId,
        text: String,
    },
    /// Tick or untick a checkbox, or tick a radio button (unticking the
    /// rest of its group; a radio can't be unticked on its own).
    SetChecked {
        field: AnnotId,
        checked: bool,
    },
    /// Choose option `index` of a combo or a list; for a list that takes
    /// several, `add` keeps the others chosen.
    Choose {
        field: AnnotId,
        index: usize,
        add: bool,
    },
    Add {
        page: usize,
        annotation: NewAnnotation,
    },
    /// Move an annotation by `(dx, dy)` points.
    Move {
        annotation: AnnotId,
        dx: f32,
        dy: f32,
    },
    /// Change a text box's or a note's text.
    SetContents {
        annotation: AnnotId,
        text: String,
    },
    SetColor {
        annotation: AnnotId,
        color: Rgb,
    },
    Delete {
        annotation: AnnotId,
    },
}

/// The farthest from a page's corner a point may be, and the farthest an
/// annotation may move, in points: well past any real page (the largest a
/// PDF page may be is 200 inches, 14 400 points), short of where `f32`
/// loses whole points.
pub const MAX_COORD: f32 = 100_000.0;
/// The widest a stroke may be, in points.
pub const MAX_LINE_WIDTH: f32 = 100.0;
/// The largest text may be, in points.
pub const MAX_TEXT_SIZE: f32 = 1_000.0;
/// The most characters a text, a note or a field's value may hold.
pub const MAX_TEXT_LEN: usize = 100_000;
/// The most UTF-16 units typed into a choice field that takes typing,
/// which PDFium is handed one keystroke at a time.
pub const MAX_TYPED: usize = 4_096;
/// The most pixels a placed picture may be on either side.
pub const MAX_IMAGE_SIDE: u32 = 8_192;

impl Edit {
    /// Whether every number in the edit is one a page can hold: finite, and
    /// within [`MAX_COORD`] and the other bounds above. Says what isn't.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`], saying which number is out of bounds.
    pub fn check(&self) -> Result<()> {
        match self {
            Self::SetText { text, .. } | Self::SetContents { text, .. } => check_text(text),
            Self::SetChecked { .. } | Self::Choose { .. } | Self::Delete { .. } => Ok(()),
            Self::Move { dx, dy, .. } => {
                check_coord("a move", *dx)?;
                check_coord("a move", *dy)
            }
            Self::SetColor { color, .. } => check_color(*color),
            Self::Add { annotation, .. } => annotation.check(),
        }
    }
}

impl NewAnnotation {
    fn check(&self) -> Result<()> {
        match self {
            Self::Markup { quads, color, .. } => {
                check_color(*color)?;
                quads
                    .iter()
                    .flatten()
                    .try_for_each(|point| check_point("marked text", *point))
            }
            Self::Ink {
                strokes,
                width,
                color,
            } => {
                check_color(*color)?;
                if !(width.is_finite() && *width > 0.0 && *width <= MAX_LINE_WIDTH) {
                    return Err(Error::Invalid(format!(
                        "ink takes a width above 0 and up to {MAX_LINE_WIDTH}, not {width}"
                    )));
                }
                strokes
                    .iter()
                    .flatten()
                    .try_for_each(|point| check_point("ink", *point))
            }
            Self::Text {
                at,
                text,
                size,
                color,
            } => {
                check_color(*color)?;
                check_point("text", *at)?;
                check_text(text)?;
                if !(size.is_finite() && *size > 0.0 && *size <= MAX_TEXT_SIZE) {
                    return Err(Error::Invalid(format!(
                        "text takes a size above 0 and up to {MAX_TEXT_SIZE}, not {size}"
                    )));
                }
                Ok(())
            }
            Self::Note { at, text, color } => {
                check_color(*color)?;
                check_point("a note", *at)?;
                check_text(text)
            }
            Self::Image {
                rect,
                width,
                height,
                ..
            } => {
                check_point("a picture", (rect.left, rect.top))?;
                check_point("a picture", (rect.right, rect.bottom))?;
                if *width > MAX_IMAGE_SIDE || *height > MAX_IMAGE_SIDE {
                    return Err(Error::Invalid(format!(
                        "a picture is at most {MAX_IMAGE_SIDE} pixels a side, not {width}×{height}"
                    )));
                }
                Ok(())
            }
        }
    }
}

fn check_coord(what: &str, value: f32) -> Result<()> {
    if value.is_finite() && value.abs() <= MAX_COORD {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "{what} takes numbers within ±{MAX_COORD} points, not {value}"
        )))
    }
}

fn check_point(what: &str, (x, y): (f32, f32)) -> Result<()> {
    check_coord(what, x)?;
    check_coord(what, y)
}

fn check_color(Rgb(r, g, b): Rgb) -> Result<()> {
    if [r, g, b].iter().all(|channel| channel.is_finite()) {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "a colour takes channels from 0 to 1, not ({r}, {g}, {b})"
        )))
    }
}

fn check_text(text: &str) -> Result<()> {
    let len = text.chars().count();
    if len <= MAX_TEXT_LEN {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "text is at most {MAX_TEXT_LEN} characters, not {len}"
        )))
    }
}

/// Why something couldn't be done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// PDFium isn't installed, or wouldn't load; the text says what was
    /// tried.
    Unavailable(String),
    /// The file needs a password, and none (or the wrong one) was given.
    Password,
    /// The file isn't a PDF PDFium can read.
    Unreadable(String),
    /// An id that names nothing in the document as it is now, or an edit
    /// that doesn't apply to what it names.
    Invalid(String),
    /// PDFium refused the change.
    Failed(String),
    /// The file couldn't be written.
    Io(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(why) => write!(f, "PDFium isn't available: {why}"),
            Self::Password => write!(f, "the document needs a password"),
            Self::Unreadable(why) => write!(f, "not a readable PDF: {why}"),
            Self::Invalid(why) => write!(f, "{why}"),
            Self::Failed(why) => write!(f, "{why}"),
            Self::Io(why) => write!(f, "could not write the file: {why}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Whether PDFium can be loaded here.
pub fn available() -> bool {
    library::load().is_ok()
}

/// Open the PDF at `path` for editing, with its password if it has one.
pub fn open(path: &Path, password: Option<&str>) -> Result<Document> {
    Document::open(PathBuf::from(path), password.map(str::to_owned))
}
