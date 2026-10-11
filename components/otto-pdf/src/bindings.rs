//! The PDFium calls this crate makes, resolved from `libpdfium.so` at run time.
//!
//! Hand-written against the chromium/8086 headers (`fpdfview.h`,
//! `fpdf_annot.h`, `fpdf_formfill.h`, `fpdf_edit.h`, `fpdf_save.h`,
//! `fpdf_signature.h`), and only the functions the engine uses. Every
//! function is a raw `unsafe extern "C"` pointer: a call is sound when its
//! handles are live ones PDFium gave out, its out-pointers point at writable
//! memory of the declared type and size, and no other thread is inside
//! PDFium at the same time. The engine keeps to that by owning PDFium on one
//! thread (see `worker`).

// Rust guideline compliant 2026-02-21

#![allow(non_snake_case, non_camel_case_types)]

use std::ffi::{c_char, c_double, c_float, c_int, c_uint, c_ulong, c_void};

use libloading::Library;

macro_rules! opaque {
    ($($name:ident),* $(,)?) => {
        $(
            /// An opaque PDFium object, only ever behind a pointer.
            #[repr(C)]
            pub struct $name {
                _private: [u8; 0],
            }
        )*
    };
}

opaque!(
    FpdfDocument,
    FpdfPage,
    FpdfAnnotation,
    FpdfForm,
    FpdfPageObject,
    FpdfBitmap,
    FpdfSignature,
    FpdfFont,
    FpdfGlyphPath,
    FpdfPathSegment
);

pub type FPDF_DOCUMENT = *mut FpdfDocument;
pub type FPDF_PAGE = *mut FpdfPage;
pub type FPDF_ANNOTATION = *mut FpdfAnnotation;
pub type FPDF_FORMHANDLE = *mut FpdfForm;
pub type FPDF_PAGEOBJECT = *mut FpdfPageObject;
pub type FPDF_BITMAP = *mut FpdfBitmap;
pub type FPDF_SIGNATURE = *mut FpdfSignature;
pub type FPDF_FONT = *mut FpdfFont;
pub type FPDF_GLYPHPATH = *const FpdfGlyphPath;
pub type FPDF_PATHSEGMENT = *const FpdfPathSegment;

pub const FPDF_SEGMENT_LINETO: c_int = 0;
pub const FPDF_SEGMENT_BEZIERTO: c_int = 1;
pub const FPDF_SEGMENT_MOVETO: c_int = 2;
pub type FPDF_BOOL = c_int;
pub type FPDF_DWORD = c_ulong;
pub type FPDF_WCHAR = u16;
pub type FPDF_WIDESTRING = *const FPDF_WCHAR;
pub type FPDF_BYTESTRING = *const c_char;

/// `FPDF_ERR_PASSWORD`: a password is needed, or the one given is wrong.
pub const FPDF_ERR_PASSWORD: c_ulong = 4;

/// `FPDF_INCREMENTAL`: append the changes after the original bytes.
/// `FPDFBitmap_BGRA` (`fpdfview.h`): four bytes a pixel.
pub const FPDF_BITMAP_BGRA: c_int = 4;
/// `FPDF_ANNOT` (`fpdfview.h`): draw the page's annotations.
pub const FPDF_RENDER_ANNOT: c_int = 0x01;
/// `FPDF_REVERSE_BYTE_ORDER` (`fpdfview.h`): write RGBA rather than BGRA.
pub const FPDF_RENDER_REVERSE_BYTE_ORDER: c_int = 0x10;

pub const FPDF_INCREMENTAL: FPDF_DWORD = 1 << 0;
/// `FPDF_NO_INCREMENTAL`: write the whole file afresh.
pub const FPDF_NO_INCREMENTAL: FPDF_DWORD = 1 << 1;
/// `FPDF_REMOVE_SECURITY`: drop the encryption. Never passed: a locked file
/// stays locked.
#[allow(
    dead_code,
    reason = "documents the flag the engine deliberately leaves out"
)]
pub const FPDF_REMOVE_SECURITY: FPDF_DWORD = 1 << 2;

pub const FPDFBITMAP_BGRA: c_int = 4;

pub const FPDF_ANNOT_TEXT: c_int = 1;
pub const FPDF_ANNOT_FREETEXT: c_int = 3;
pub const FPDF_ANNOT_HIGHLIGHT: c_int = 9;
pub const FPDF_ANNOT_UNDERLINE: c_int = 10;
pub const FPDF_ANNOT_SQUIGGLY: c_int = 11;
pub const FPDF_ANNOT_STRIKEOUT: c_int = 12;
pub const FPDF_ANNOT_STAMP: c_int = 13;
pub const FPDF_ANNOT_INK: c_int = 15;
pub const FPDF_ANNOT_POPUP: c_int = 16;
pub const FPDF_ANNOT_WIDGET: c_int = 20;

pub const FPDF_ANNOT_FLAG_PRINT: c_int = 1 << 2;
pub const FPDF_ANNOT_FLAG_READONLY: c_int = 1 << 6;
pub const FPDF_ANNOT_FLAG_LOCKED: c_int = 1 << 7;

pub const FPDF_ANNOT_APPEARANCEMODE_NORMAL: c_int = 0;
pub const FPDFANNOT_COLORTYPE_COLOR: c_int = 0;

/// `FWL_VKEY_Delete`, from `fpdf_fwlevent.h`.
pub const FWL_VKEY_DELETE: c_int = 0x2E;

pub const FPDF_FORMFIELD_PUSHBUTTON: c_int = 1;
pub const FPDF_FORMFIELD_CHECKBOX: c_int = 2;
pub const FPDF_FORMFIELD_RADIOBUTTON: c_int = 3;
pub const FPDF_FORMFIELD_COMBOBOX: c_int = 4;
pub const FPDF_FORMFIELD_LISTBOX: c_int = 5;
pub const FPDF_FORMFIELD_TEXTFIELD: c_int = 6;
pub const FPDF_FORMFIELD_SIGNATURE: c_int = 7;

/// `FS_RECTF`: in page space `top` is the larger y.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FS_RECTF {
    pub left: c_float,
    pub top: c_float,
    pub right: c_float,
    pub bottom: c_float,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FS_POINTF {
    pub x: c_float,
    pub y: c_float,
}

/// `FS_QUADPOINTSF`: conventionally top-left, top-right, bottom-left,
/// bottom-right.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FS_QUADPOINTSF {
    pub x1: c_float,
    pub y1: c_float,
    pub x2: c_float,
    pub y2: c_float,
    pub x3: c_float,
    pub y3: c_float,
    pub x4: c_float,
    pub y4: c_float,
}

/// `FS_MATRIX`: `| a c e |` over `| b d f |`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FS_MATRIX {
    pub a: c_float,
    pub b: c_float,
    pub c: c_float,
    pub d: c_float,
    pub e: c_float,
    pub f: c_float,
}

/// `FPDF_LIBRARY_CONFIG`, up to the version 2 fields it is declared with.
#[repr(C)]
pub struct FPDF_LIBRARY_CONFIG {
    pub version: c_int,
    pub m_pUserFontPaths: *const *const c_char,
    pub m_pIsolate: *mut c_void,
    pub m_v8EmbedderSlot: c_uint,
}

/// `FPDF_FILEWRITE`: PDFium calls `WriteBlock` with each run of bytes it
/// saves; `self_` is the pointer passed to `FPDF_SaveAsCopy`, so an embedder
/// puts this struct first in its own and casts back.
#[repr(C)]
pub struct FPDF_FILEWRITE {
    pub version: c_int,
    pub WriteBlock: Option<
        unsafe extern "C" fn(
            self_: *mut FPDF_FILEWRITE,
            data: *const c_void,
            size: c_ulong,
        ) -> c_int,
    >,
}

/// A form-fill callback the engine never provides. Every one in
/// [`FPDF_FORMFILLINFO`] is left null, which PDFium checks before calling;
/// they are all pointer-sized, so one type stands for every signature.
pub type Unused = Option<unsafe extern "C" fn()>;

/// `FPDF_FORMFILLINFO`, version 2, field for field. The engine zeroes it and
/// sets only `version`: no callbacks, no JavaScript platform, XFA off.
#[repr(C)]
pub struct FPDF_FORMFILLINFO {
    pub version: c_int,
    pub Release: Unused,
    pub FFI_Invalidate: Unused,
    pub FFI_OutputSelectedRect: Unused,
    pub FFI_SetCursor: Unused,
    pub FFI_SetTimer: Unused,
    pub FFI_KillTimer: Unused,
    pub FFI_GetLocalTime: Unused,
    pub FFI_OnChange: Unused,
    pub FFI_GetPage: Unused,
    pub FFI_GetCurrentPage: Unused,
    pub FFI_GetRotation: Unused,
    pub FFI_ExecuteNamedAction: Unused,
    pub FFI_SetTextFieldFocus: Unused,
    pub FFI_DoURIAction: Unused,
    pub FFI_DoGoToAction: Unused,
    pub m_pJsPlatform: *mut c_void,
    pub xfa_disabled: FPDF_BOOL,
    pub FFI_DisplayCaret: Unused,
    pub FFI_GetCurrentPageIndex: Unused,
    pub FFI_SetCurrentPage: Unused,
    pub FFI_GotoURL: Unused,
    pub FFI_GetPageViewRect: Unused,
    pub FFI_PageEvent: Unused,
    pub FFI_PopupMenu: Unused,
    pub FFI_OpenFile: Unused,
    pub FFI_EmailTo: Unused,
    pub FFI_UploadTo: Unused,
    pub FFI_GetPlatform: Unused,
    pub FFI_GetLanguage: Unused,
    pub FFI_DownloadFromURL: Unused,
    pub FFI_PostRequestURL: Unused,
    pub FFI_PutRequestURL: Unused,
    pub FFI_OnFocusChange: Unused,
    pub FFI_DoURIActionWithKeyboardModifier: Unused,
}

impl FPDF_FORMFILLINFO {
    /// A version 2 info with every callback null and XFA disabled.
    pub fn version_2() -> Self {
        Self {
            version: 2,
            Release: None,
            FFI_Invalidate: None,
            FFI_OutputSelectedRect: None,
            FFI_SetCursor: None,
            FFI_SetTimer: None,
            FFI_KillTimer: None,
            FFI_GetLocalTime: None,
            FFI_OnChange: None,
            FFI_GetPage: None,
            FFI_GetCurrentPage: None,
            FFI_GetRotation: None,
            FFI_ExecuteNamedAction: None,
            FFI_SetTextFieldFocus: None,
            FFI_DoURIAction: None,
            FFI_DoGoToAction: None,
            m_pJsPlatform: std::ptr::null_mut(),
            xfa_disabled: 1,
            FFI_DisplayCaret: None,
            FFI_GetCurrentPageIndex: None,
            FFI_SetCurrentPage: None,
            FFI_GotoURL: None,
            FFI_GetPageViewRect: None,
            FFI_PageEvent: None,
            FFI_PopupMenu: None,
            FFI_OpenFile: None,
            FFI_EmailTo: None,
            FFI_UploadTo: None,
            FFI_GetPlatform: None,
            FFI_GetLanguage: None,
            FFI_DownloadFromURL: None,
            FFI_PostRequestURL: None,
            FFI_PutRequestURL: None,
            FFI_OnFocusChange: None,
            FFI_DoURIActionWithKeyboardModifier: None,
        }
    }
}

// The header's layout on LP64: an int padded to 8, fifteen callbacks, the JS
// platform pointer, a padded BOOL, then seventeen more callbacks.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<FPDF_FORMFILLINFO>() == 280);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<FPDF_FILEWRITE>() == 16);

macro_rules! bindings {
    ($( fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $(-> $ret:ty)?; )*) => {
        /// PDFium's functions, resolved from the library this keeps loaded.
        pub struct Bindings {
            _library: Library,
            $( pub $name: unsafe extern "C" fn($($ty),*) $(-> $ret)?, )*
        }

        impl Bindings {
            /// Resolves every function from `library`, or names the first
            /// one it lacks.
            ///
            /// # Safety
            ///
            /// `library` must be a PDFium build whose exported functions have
            /// the signatures declared here (chromium/8086 or an ABI-compatible
            /// one).
            pub unsafe fn resolve(library: Library) -> Result<Self, String> {
                $(
                    // SAFETY: the caller vouches that the symbol has this
                    // signature; the pointer is copied out while `library`
                    // is borrowed and stays valid because `library` moves
                    // into the returned struct, which never unloads it early.
                    let $name = unsafe {
                        *library
                            .get::<unsafe extern "C" fn($($ty),*) $(-> $ret)?>(
                                concat!(stringify!($name), "\0").as_bytes(),
                            )
                            .map_err(|error| format!("{}: {error}", stringify!($name)))?
                    };
                )*
                Ok(Self { _library: library, $( $name, )* })
            }
        }
    };
}

bindings! {
    fn FPDF_InitLibraryWithConfig(config: *const FPDF_LIBRARY_CONFIG);
    fn FPDF_LoadMemDocument64(data: *const c_void, size: usize, password: FPDF_BYTESTRING) -> FPDF_DOCUMENT;
    fn FPDF_GetLastError() -> c_ulong;
    fn FPDF_CloseDocument(document: FPDF_DOCUMENT);
    fn FPDF_GetPageCount(document: FPDF_DOCUMENT) -> c_int;
    fn FPDF_LoadPage(document: FPDF_DOCUMENT, index: c_int) -> FPDF_PAGE;
    fn FPDF_ClosePage(page: FPDF_PAGE);
    fn FPDF_GetPageBoundingBox(page: FPDF_PAGE, rect: *mut FS_RECTF) -> FPDF_BOOL;
    fn FPDFPage_GetRotation(page: FPDF_PAGE) -> c_int;
    fn FPDF_SaveAsCopy(document: FPDF_DOCUMENT, write: *mut FPDF_FILEWRITE, flags: FPDF_DWORD) -> FPDF_BOOL;
    fn FPDF_GetSignatureCount(document: FPDF_DOCUMENT) -> c_int;
    fn FPDF_GetSignatureObject(document: FPDF_DOCUMENT, index: c_int) -> FPDF_SIGNATURE;
    fn FPDFSignatureObj_GetContents(signature: FPDF_SIGNATURE, buffer: *mut c_void, length: c_ulong) -> c_ulong;

    fn FPDFBitmap_CreateEx(width: c_int, height: c_int, format: c_int, first_scan: *mut c_void, stride: c_int) -> FPDF_BITMAP;
    fn FPDFBitmap_Destroy(bitmap: FPDF_BITMAP);
    fn FPDFBitmap_FillRect(bitmap: FPDF_BITMAP, left: c_int, top: c_int, width: c_int, height: c_int, color: FPDF_DWORD) -> FPDF_BOOL;
    fn FPDF_RenderPageBitmap(bitmap: FPDF_BITMAP, page: FPDF_PAGE, start_x: c_int, start_y: c_int, size_x: c_int, size_y: c_int, rotate: c_int, flags: c_int);
    fn FPDF_FFLDraw(form: FPDF_FORMHANDLE, bitmap: FPDF_BITMAP, page: FPDF_PAGE, start_x: c_int, start_y: c_int, size_x: c_int, size_y: c_int, rotate: c_int, flags: c_int);
    fn FPDFPageObj_NewImageObj(document: FPDF_DOCUMENT) -> FPDF_PAGEOBJECT;
    fn FPDFImageObj_SetBitmap(pages: *mut FPDF_PAGE, count: c_int, image: FPDF_PAGEOBJECT, bitmap: FPDF_BITMAP) -> FPDF_BOOL;
    fn FPDFPageObj_SetMatrix(object: FPDF_PAGEOBJECT, matrix: *const FS_MATRIX) -> FPDF_BOOL;
    fn FPDFPageObj_Destroy(object: FPDF_PAGEOBJECT);
    fn FPDFText_LoadStandardFont(document: FPDF_DOCUMENT, name: FPDF_BYTESTRING) -> FPDF_FONT;
    fn FPDFFont_Close(font: FPDF_FONT);
    fn FPDFFont_GetAscent(font: FPDF_FONT, size: c_float, ascent: *mut c_float) -> FPDF_BOOL;
    fn FPDFFont_GetGlyphWidth(font: FPDF_FONT, glyph: u32, size: c_float, width: *mut c_float) -> FPDF_BOOL;
    fn FPDFFont_GetGlyphPath(font: FPDF_FONT, glyph: u32, size: c_float) -> FPDF_GLYPHPATH;
    fn FPDFGlyphPath_CountGlyphSegments(path: FPDF_GLYPHPATH) -> c_int;
    fn FPDFGlyphPath_GetGlyphPathSegment(path: FPDF_GLYPHPATH, index: c_int) -> FPDF_PATHSEGMENT;
    fn FPDFPathSegment_GetPoint(segment: FPDF_PATHSEGMENT, x: *mut c_float, y: *mut c_float) -> FPDF_BOOL;
    fn FPDFPathSegment_GetType(segment: FPDF_PATHSEGMENT) -> c_int;
    fn FPDFPathSegment_GetClose(segment: FPDF_PATHSEGMENT) -> FPDF_BOOL;

    fn FPDFDOC_InitFormFillEnvironment(document: FPDF_DOCUMENT, info: *mut FPDF_FORMFILLINFO) -> FPDF_FORMHANDLE;
    fn FPDFDOC_ExitFormFillEnvironment(form: FPDF_FORMHANDLE);
    fn FORM_OnAfterLoadPage(page: FPDF_PAGE, form: FPDF_FORMHANDLE);
    fn FORM_OnBeforeClosePage(page: FPDF_PAGE, form: FPDF_FORMHANDLE);
    fn FORM_SetFocusedAnnot(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION) -> FPDF_BOOL;
    fn FORM_SelectAllText(form: FPDF_FORMHANDLE, page: FPDF_PAGE) -> FPDF_BOOL;
    fn FORM_ReplaceSelection(form: FPDF_FORMHANDLE, page: FPDF_PAGE, text: FPDF_WIDESTRING);
    fn FORM_OnChar(form: FPDF_FORMHANDLE, page: FPDF_PAGE, character: c_int, modifier: c_int) -> FPDF_BOOL;
    fn FORM_OnKeyDown(form: FPDF_FORMHANDLE, page: FPDF_PAGE, key: c_int, modifier: c_int) -> FPDF_BOOL;
    fn FORM_ForceToKillFocus(form: FPDF_FORMHANDLE) -> FPDF_BOOL;
    fn FORM_OnMouseMove(form: FPDF_FORMHANDLE, page: FPDF_PAGE, modifier: c_int, x: c_double, y: c_double) -> FPDF_BOOL;
    fn FORM_OnLButtonDown(form: FPDF_FORMHANDLE, page: FPDF_PAGE, modifier: c_int, x: c_double, y: c_double) -> FPDF_BOOL;
    fn FORM_OnLButtonUp(form: FPDF_FORMHANDLE, page: FPDF_PAGE, modifier: c_int, x: c_double, y: c_double) -> FPDF_BOOL;
    fn FORM_SetIndexSelected(form: FPDF_FORMHANDLE, page: FPDF_PAGE, index: c_int, selected: FPDF_BOOL) -> FPDF_BOOL;

    fn FPDFPage_GetAnnotCount(page: FPDF_PAGE) -> c_int;
    fn FPDFPage_GetAnnot(page: FPDF_PAGE, index: c_int) -> FPDF_ANNOTATION;
    fn FPDFPage_CloseAnnot(annot: FPDF_ANNOTATION);
    fn FPDFPage_CreateAnnot(page: FPDF_PAGE, subtype: c_int) -> FPDF_ANNOTATION;
    fn FPDFPage_RemoveAnnot(page: FPDF_PAGE, index: c_int) -> FPDF_BOOL;
    fn FPDFPage_GetAnnotIndex(page: FPDF_PAGE, annot: FPDF_ANNOTATION) -> c_int;
    fn FPDFAnnot_GetLinkedAnnot(annot: FPDF_ANNOTATION, key: FPDF_BYTESTRING) -> FPDF_ANNOTATION;
    fn FPDFAnnot_GetSubtype(annot: FPDF_ANNOTATION) -> c_int;
    fn FPDFAnnot_GetRect(annot: FPDF_ANNOTATION, rect: *mut FS_RECTF) -> FPDF_BOOL;
    fn FPDFAnnot_SetRect(annot: FPDF_ANNOTATION, rect: *const FS_RECTF) -> FPDF_BOOL;
    fn FPDFAnnot_GetColor(annot: FPDF_ANNOTATION, kind: c_int, r: *mut c_uint, g: *mut c_uint, b: *mut c_uint, a: *mut c_uint) -> FPDF_BOOL;
    fn FPDFAnnot_SetColor(annot: FPDF_ANNOTATION, kind: c_int, r: c_uint, g: c_uint, b: c_uint, a: c_uint) -> FPDF_BOOL;
    fn FPDFAnnot_GetFlags(annot: FPDF_ANNOTATION) -> c_int;
    fn FPDFAnnot_SetFlags(annot: FPDF_ANNOTATION, flags: c_int) -> FPDF_BOOL;
    fn FPDFAnnot_HasKey(annot: FPDF_ANNOTATION, key: FPDF_BYTESTRING) -> FPDF_BOOL;
    fn FPDFAnnot_GetStringValue(annot: FPDF_ANNOTATION, key: FPDF_BYTESTRING, buffer: *mut FPDF_WCHAR, length: c_ulong) -> c_ulong;
    fn FPDFAnnot_SetStringValue(annot: FPDF_ANNOTATION, key: FPDF_BYTESTRING, value: FPDF_WIDESTRING) -> FPDF_BOOL;
    fn FPDFAnnot_GetNumberValue(annot: FPDF_ANNOTATION, key: FPDF_BYTESTRING, value: *mut c_float) -> FPDF_BOOL;
    fn FPDFAnnot_SetAP(annot: FPDF_ANNOTATION, mode: c_int, value: FPDF_WIDESTRING) -> FPDF_BOOL;
    fn FPDFAnnot_GetAP(annot: FPDF_ANNOTATION, mode: c_int, buffer: *mut FPDF_WCHAR, length: c_ulong) -> c_ulong;
    fn FPDFAnnot_CountAttachmentPoints(annot: FPDF_ANNOTATION) -> usize;
    fn FPDFAnnot_GetAttachmentPoints(annot: FPDF_ANNOTATION, index: usize, quad: *mut FS_QUADPOINTSF) -> FPDF_BOOL;
    fn FPDFAnnot_SetAttachmentPoints(annot: FPDF_ANNOTATION, index: usize, quad: *const FS_QUADPOINTSF) -> FPDF_BOOL;
    fn FPDFAnnot_AppendAttachmentPoints(annot: FPDF_ANNOTATION, quad: *const FS_QUADPOINTSF) -> FPDF_BOOL;
    fn FPDFAnnot_GetInkListCount(annot: FPDF_ANNOTATION) -> c_ulong;
    fn FPDFAnnot_GetInkListPath(annot: FPDF_ANNOTATION, index: c_ulong, buffer: *mut FS_POINTF, length: c_ulong) -> c_ulong;
    fn FPDFAnnot_AddInkStroke(annot: FPDF_ANNOTATION, points: *const FS_POINTF, count: usize) -> c_int;
    fn FPDFAnnot_RemoveInkList(annot: FPDF_ANNOTATION) -> FPDF_BOOL;
    fn FPDFAnnot_GetBorder(annot: FPDF_ANNOTATION, horizontal_radius: *mut c_float, vertical_radius: *mut c_float, width: *mut c_float) -> FPDF_BOOL;
    fn FPDFAnnot_SetBorder(annot: FPDF_ANNOTATION, horizontal_radius: c_float, vertical_radius: c_float, width: c_float) -> FPDF_BOOL;
    fn FPDFAnnot_AppendObject(annot: FPDF_ANNOTATION, object: FPDF_PAGEOBJECT) -> FPDF_BOOL;

    fn FPDFAnnot_GetFormFieldName(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION, buffer: *mut FPDF_WCHAR, length: c_ulong) -> c_ulong;
    fn FPDFAnnot_GetFormFieldType(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION) -> c_int;
    fn FPDFAnnot_GetFormFieldValue(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION, buffer: *mut FPDF_WCHAR, length: c_ulong) -> c_ulong;
    fn FPDFAnnot_GetFormFieldFlags(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION) -> c_int;
    fn FPDFAnnot_GetFormFieldExportValue(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION, buffer: *mut FPDF_WCHAR, length: c_ulong) -> c_ulong;
    fn FPDFAnnot_GetFormFieldAlternateName(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION, buffer: *mut FPDF_WCHAR, length: c_ulong) -> c_ulong;
    fn FPDFAnnot_IsChecked(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION) -> FPDF_BOOL;
    fn FPDFAnnot_GetOptionCount(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION) -> c_int;
    fn FPDFAnnot_GetOptionLabel(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION, index: c_int, buffer: *mut FPDF_WCHAR, length: c_ulong) -> c_ulong;
    fn FPDFAnnot_IsOptionSelected(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION, index: c_int) -> FPDF_BOOL;
    fn FPDFAnnot_GetFontSize(form: FPDF_FORMHANDLE, annot: FPDF_ANNOTATION, value: *mut c_float) -> FPDF_BOOL;
}
