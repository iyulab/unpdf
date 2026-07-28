//! C-ABI Foreign Function Interface for unpdf.
//!
//! This module provides C-compatible bindings for using unpdf from other languages
//! such as C, C++, C#, Python, and any language with C FFI support.
//!
//! # Memory Management
//!
//! All strings returned by this library must be freed using `unpdf_free_string`.
//! All document handles must be freed using `unpdf_free_document`.
//!
//! # Error Handling
//!
//! Functions that can fail return a null pointer on error. Use `unpdf_last_error`
//! to retrieve the error message and `unpdf_last_error_kind` to classify it without
//! parsing that message.

use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::catch_unwind;
use std::ptr;

use crate::error::ErrorKind;
use crate::model::Document;
use crate::render::{JsonFormat, RenderOptions};

/// `unpdf_last_error_kind` value when no error is recorded on this thread.
pub const UNPDF_ERROR_NONE: c_int = 0;

// Values 1..=17 are [`ErrorKind`] discriminants — core failure reasons.
// Values 100+ are FFI-boundary reasons with no core `Error` counterpart.

/// An argument was null or not valid UTF-8.
pub const UNPDF_ERROR_INVALID_ARGUMENT: c_int = 100;
/// A panic was caught at the FFI boundary.
pub const UNPDF_ERROR_PANIC: c_int = 101;
/// The produced output contains an interior NUL byte and cannot cross the C ABI.
pub const UNPDF_ERROR_INVALID_OUTPUT: c_int = 102;

/// A failure carried out of a `catch_unwind` closure: its classification plus message.
type FfiError = (c_int, String);

/// Classify a core error and render its message, for return from a closure.
fn ffi_err(e: crate::Error) -> FfiError {
    (e.kind() as c_int, e.to_string())
}

/// Classify a JSON serialization failure — producing output is rendering.
fn json_err(e: serde_json::Error) -> FfiError {
    (ErrorKind::Render as c_int, e.to_string())
}

// Thread-local storage for the last error message and its classification.
// The two are always written together so a caller never sees a message paired
// with a stale kind.
thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
    static LAST_ERROR_KIND: RefCell<c_int> = const { RefCell::new(UNPDF_ERROR_NONE) };
}

/// Set the last error message and its classification.
fn set_last_error(kind: c_int, msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
    LAST_ERROR_KIND.with(|k| {
        *k.borrow_mut() = kind;
    });
}

/// Clear the last error message and its classification.
fn clear_last_error() {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = None;
    });
    LAST_ERROR_KIND.with(|k| {
        *k.borrow_mut() = UNPDF_ERROR_NONE;
    });
}

/// Opaque handle to a parsed document.
#[repr(C)]
pub struct UnpdfDocument {
    inner: Document,
}

/// Flags for markdown rendering.
pub const UNPDF_FLAG_FRONTMATTER: u32 = 1;
pub const UNPDF_FLAG_ESCAPE_SPECIAL: u32 = 2;
pub const UNPDF_FLAG_PARAGRAPH_SPACING: u32 = 4;

/// JSON format options.
pub const UNPDF_JSON_PRETTY: c_int = 0;
pub const UNPDF_JSON_COMPACT: c_int = 1;

/// Get the version of the library.
///
/// # Safety
///
/// Returns a static string that must not be freed.
#[no_mangle]
pub extern "C" fn unpdf_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Get the last error message.
///
/// # Safety
///
/// Returns a pointer to a thread-local error string. The pointer is valid until
/// the next call to any unpdf function on the same thread.
#[no_mangle]
pub extern "C" fn unpdf_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(ptr::null())
    })
}

/// Classify the last error without parsing its message.
///
/// Returns `UNPDF_ERROR_NONE` (0) when the last call on this thread succeeded.
/// Values 1..=17 are core failure reasons (see `UnpdfErrorKind` in `unpdf.h`);
/// values 100+ are FFI-boundary reasons. Treat an unrecognised value as a generic
/// failure — new reasons take new numbers and never renumber existing ones.
///
/// # Safety
///
/// Reads thread-local state written by the immediately preceding unpdf call on the
/// same thread, in lockstep with `unpdf_last_error`.
#[no_mangle]
pub extern "C" fn unpdf_last_error_kind() -> c_int {
    LAST_ERROR_KIND.with(|k| *k.borrow())
}

/// Parse a document from a file path.
///
/// # Safety
///
/// - `path` must be a valid null-terminated UTF-8 string.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned handle must be freed with `unpdf_free_document`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_parse_file(path: *const c_char) -> *mut UnpdfDocument {
    clear_last_error();

    if path.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "path is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let path_str = CStr::from_ptr(path)
            .to_str()
            .map_err(|e| (UNPDF_ERROR_INVALID_ARGUMENT, e.to_string()))?;

        crate::parse_file(path_str)
            .map(|doc| Box::into_raw(Box::new(UnpdfDocument { inner: doc })))
            .map_err(ffi_err)
    });

    match result {
        Ok(Ok(doc)) => doc,
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred during parsing");
            ptr::null_mut()
        }
    }
}

/// Parse a document from a byte buffer.
///
/// # Safety
///
/// - `data` must be a valid pointer to a byte buffer of at least `len` bytes.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned handle must be freed with `unpdf_free_document`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_parse_bytes(data: *const u8, len: usize) -> *mut UnpdfDocument {
    clear_last_error();

    if data.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "data is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let bytes = std::slice::from_raw_parts(data, len);

        crate::parse_bytes(bytes)
            .map(|doc| Box::into_raw(Box::new(UnpdfDocument { inner: doc })))
            .map_err(ffi_err)
    });

    match result {
        Ok(Ok(doc)) => doc,
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred during parsing");
            ptr::null_mut()
        }
    }
}

/// Free a document handle.
///
/// # Safety
///
/// - `doc` must be a valid pointer returned by `unpdf_parse_file` or `unpdf_parse_bytes`.
/// - After calling this function, the handle is invalid and must not be used.
#[no_mangle]
pub unsafe extern "C" fn unpdf_free_document(doc: *mut UnpdfDocument) {
    if !doc.is_null() {
        let _ = Box::from_raw(doc);
    }
}

/// Convert a document to Markdown.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - `flags` is a bitwise OR of `UNPDF_FLAG_*` constants.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_to_markdown(doc: *const UnpdfDocument, flags: u32) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;

        let mut options = RenderOptions::new();

        if flags & UNPDF_FLAG_FRONTMATTER != 0 {
            options.include_frontmatter = true;
        }
        if flags & UNPDF_FLAG_ESCAPE_SPECIAL != 0 {
            options.escape_special_chars = true;
        }
        // PARAGRAPH_SPACING: no direct field in unpdf's RenderOptions,
        // treat as no-op for now

        crate::render::to_markdown(document, &options).map_err(ffi_err)
    });

    match result {
        Ok(Ok(md)) => match CString::new(md) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred during rendering");
            ptr::null_mut()
        }
    }
}

/// Convert a document to plain text.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_to_text(doc: *const UnpdfDocument) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        let options = RenderOptions::default();
        crate::render::to_text(document, &options).map_err(ffi_err)
    });

    match result {
        Ok(Ok(text)) => match CString::new(text) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred during rendering");
            ptr::null_mut()
        }
    }
}

/// Convert a document to JSON.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - `format` is one of `UNPDF_JSON_PRETTY` or `UNPDF_JSON_COMPACT`.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_to_json(doc: *const UnpdfDocument, format: c_int) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        let json_format = if format == UNPDF_JSON_COMPACT {
            JsonFormat::Compact
        } else {
            JsonFormat::Pretty
        };
        crate::render::to_json(document, json_format).map_err(ffi_err)
    });

    match result {
        Ok(Ok(json)) => match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred during rendering");
            ptr::null_mut()
        }
    }
}

/// Get the plain text content of a document.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - Returns null on error.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_plain_text(doc: *const UnpdfDocument) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        document.plain_text()
    });

    match result {
        Ok(text) => match CString::new(text) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            ptr::null_mut()
        }
    }
}

/// Get the number of sections (pages) in a document.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - Returns -1 on error.
#[no_mangle]
pub unsafe extern "C" fn unpdf_section_count(doc: *const UnpdfDocument) -> c_int {
    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return -1;
    }

    match catch_unwind(|| (*doc).inner.pages.len() as c_int) {
        Ok(count) => count,
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            -1
        }
    }
}

/// Get the number of extracted resources (images) in a document.
///
/// Semantics: counts entries in the document's resource inventory, which is
/// populated only when parsing runs with `extract_resources` enabled. The FFI
/// parse entry points use default options where resource extraction is **off**
/// (since 0.4.0, to bound peak memory), so this returns `0` for every document
/// parsed through `unpdf_parse_file` / `unpdf_parse_bytes`. It is **not** a
/// count of images referenced by page content streams — for detecting
/// image-only (scanned) pages use `unpdf_page_stats` or
/// `unpdf_get_extraction_quality` instead.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - Returns -1 on error.
#[no_mangle]
pub unsafe extern "C" fn unpdf_resource_count(doc: *const UnpdfDocument) -> c_int {
    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return -1;
    }

    match catch_unwind(|| (*doc).inner.resources.len() as c_int) {
        Ok(count) => count,
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            -1
        }
    }
}

/// Get the document title.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - Returns null if no title is set.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_get_title(doc: *const UnpdfDocument) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        (*doc)
            .inner
            .metadata
            .title
            .as_ref()
            .and_then(|t| CString::new(t.as_str()).ok())
    });

    match result {
        Ok(Some(s)) => s.into_raw(),
        Ok(None) => ptr::null_mut(),
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            ptr::null_mut()
        }
    }
}

/// Get the document author.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - Returns null if no author is set.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_get_author(doc: *const UnpdfDocument) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        (*doc)
            .inner
            .metadata
            .author
            .as_ref()
            .and_then(|a| CString::new(a.as_str()).ok())
    });

    match result {
        Ok(Some(s)) => s.into_raw(),
        Ok(None) => ptr::null_mut(),
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            ptr::null_mut()
        }
    }
}

/// Get all resource IDs as a JSON array.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_get_resource_ids(doc: *const UnpdfDocument) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        let ids: Vec<&String> = document.resources.keys().collect();
        serde_json::to_string(&ids).map_err(json_err)
    });

    match result {
        Ok(Ok(json)) => match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            ptr::null_mut()
        }
    }
}

/// Get extraction quality diagnostics as a JSON object.
///
/// Fields: `char_count`, `word_count`, `replacement_char_count`, `encrypted`,
/// `is_scan_pdf`, `suppressed_ocr_pages`. `is_scan_pdf` is `true` when sampled
/// pages draw images with no text-showing operators — the document-level
/// "scanned document, OCR required" signal. For page-level discrimination
/// (mixed documents) use `unpdf_page_stats`.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_get_extraction_quality(doc: *const UnpdfDocument) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result =
        catch_unwind(|| serde_json::to_string(&(*doc).inner.extraction_quality).map_err(json_err));

    match result {
        Ok(Ok(json)) => match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            ptr::null_mut()
        }
    }
}

/// Get per-page content-stream operator statistics as a JSON object.
///
/// Returns `{"page":N,"text_op_count":N,"image_op_count":N,"ocr_text_suppressed":bool}`.
///
/// - `text_op_count`: number of text-showing operators (`Tj`/`TJ`/`'`/`"`).
/// - `image_op_count`: number of XObject `Do` invocations (mostly images;
///   may include form XObjects).
/// - Both `0` → genuinely blank page. `text_op_count == 0` with
///   `image_op_count > 0` → image-only (scanned) page, OCR required.
/// - Note: a *searchable* scan (page image plus an invisible OCR text layer)
///   reports `text_op_count > 0` — combine with `ocr_text_suppressed` to
///   detect scans whose OCR layer was dropped as unreadable.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - `page_number` is 1-indexed.
/// - Returns null if the page is out of range or on error.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_page_stats(
    doc: *const UnpdfDocument,
    page_number: c_int,
) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        let page = document
            .pages
            .iter()
            .find(|p| p.number == page_number as u32)
            .ok_or_else(|| {
                (
                    ErrorKind::PageOutOfRange as c_int,
                    format!(
                        "page {} out of range (document has {} pages)",
                        page_number,
                        document.pages.len()
                    ),
                )
            })?;
        serde_json::to_string(&serde_json::json!({
            "page": page.number,
            "text_op_count": page.text_op_count,
            "image_op_count": page.image_op_count,
            "ocr_text_suppressed": page.ocr_text_suppressed,
        }))
        .map_err(json_err)
    });

    match result {
        Ok(Ok(json)) => match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            ptr::null_mut()
        }
    }
}

/// Get resource metadata as JSON (without binary data).
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - `resource_id` must be a valid null-terminated UTF-8 string.
/// - Returns null if resource not found or on error.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_get_resource_info(
    doc: *const UnpdfDocument,
    resource_id: *const c_char,
) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    if resource_id.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "resource_id is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let id_str = CStr::from_ptr(resource_id)
            .to_str()
            .map_err(|e| (UNPDF_ERROR_INVALID_ARGUMENT, e.to_string()))?;

        let document = &(*doc).inner;

        match document.resources.get(id_str) {
            Some(resource) => {
                let info = serde_json::json!({
                    "id": id_str,
                    "type": resource.resource_type,
                    "filename": resource.filename,
                    "mime_type": resource.mime_type,
                    "size": resource.size(),
                    "width": resource.width,
                    "height": resource.height,
                });
                serde_json::to_string(&info).map_err(json_err)
            }
            None => Err((
                ErrorKind::ResourceNotFound as c_int,
                format!("resource not found: {}", id_str),
            )),
        }
    });

    match result {
        Ok(Ok(json)) => match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            ptr::null_mut()
        }
    }
}

/// Get resource binary data.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - `resource_id` must be a valid null-terminated UTF-8 string.
/// - `out_len` must be a valid pointer to receive the data length.
/// - Returns null if resource not found or on error.
/// - The returned pointer must be freed with `unpdf_free_bytes`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_get_resource_data(
    doc: *const UnpdfDocument,
    resource_id: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    if resource_id.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "resource_id is null");
        return ptr::null_mut();
    }

    if out_len.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "out_len is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let id_str = CStr::from_ptr(resource_id)
            .to_str()
            .map_err(|e| (UNPDF_ERROR_INVALID_ARGUMENT, e.to_string()))?;

        let document = &(*doc).inner;

        match document.resources.get(id_str) {
            Some(resource) => {
                let data = resource.data.clone();
                let len = data.len();
                let boxed = data.into_boxed_slice();
                let ptr = Box::into_raw(boxed) as *mut u8;
                Ok((ptr, len))
            }
            None => Err((
                ErrorKind::ResourceNotFound as c_int,
                format!("resource not found: {}", id_str),
            )),
        }
    });

    match result {
        Ok(Ok((ptr, len))) => {
            *out_len = len;
            ptr
        }
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            *out_len = 0;
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            *out_len = 0;
            ptr::null_mut()
        }
    }
}

/// Convert a single page to Markdown.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - `page_num` is 1-indexed.
/// - `flags` is a bitwise OR of `UNPDF_FLAG_*` constants.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_page_to_markdown(
    doc: *const UnpdfDocument,
    page_num: c_int,
    flags: u32,
) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        let page = document.get_page(page_num as u32).ok_or_else(|| {
            (
                ErrorKind::PageOutOfRange as c_int,
                format!(
                    "page {} out of range (document has {} pages)",
                    page_num,
                    document.page_count()
                ),
            )
        })?;

        let mut options = RenderOptions::new();
        if flags & UNPDF_FLAG_FRONTMATTER != 0 {
            options.include_frontmatter = true;
        }
        if flags & UNPDF_FLAG_ESCAPE_SPECIAL != 0 {
            options.escape_special_chars = true;
        }

        // Create a single-page document for rendering
        let mut single_page_doc = Document::new();
        single_page_doc.add_page(page.clone());

        crate::render::to_markdown(&single_page_doc, &options).map_err(ffi_err)
    });

    match result {
        Ok(Ok(md)) => match CString::new(md) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred during page rendering");
            ptr::null_mut()
        }
    }
}

/// Get the plain text of a single page.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - `page_num` is 1-indexed.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned string must be freed with `unpdf_free_string`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_page_to_text(
    doc: *const UnpdfDocument,
    page_num: c_int,
) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error(UNPDF_ERROR_INVALID_ARGUMENT, "document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        let page = document.get_page(page_num as u32).ok_or_else(|| {
            (
                ErrorKind::PageOutOfRange as c_int,
                format!(
                    "page {} out of range (document has {} pages)",
                    page_num,
                    document.page_count()
                ),
            )
        })?;

        Ok::<String, FfiError>(page.plain_text())
    });

    match result {
        Ok(Ok(text)) => match CString::new(text) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error(UNPDF_ERROR_INVALID_OUTPUT, "output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(e.0, &e.1);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error(UNPDF_ERROR_PANIC, "panic occurred");
            ptr::null_mut()
        }
    }
}

/// Free a string allocated by this library.
///
/// # Safety
///
/// - `s` must be a pointer returned by an unpdf function, or null.
/// - After calling this function, the pointer is invalid and must not be used.
#[no_mangle]
pub unsafe extern "C" fn unpdf_free_string(s: *mut c_char) {
    if !s.is_null() {
        let _ = CString::from_raw(s);
    }
}

/// Free binary data allocated by `unpdf_get_resource_data`.
///
/// # Safety
///
/// - `data` must be a pointer returned by `unpdf_get_resource_data`, or null.
/// - `len` must be the length returned by `unpdf_get_resource_data`.
/// - After calling this function, the pointer is invalid and must not be used.
#[no_mangle]
pub unsafe extern "C" fn unpdf_free_bytes(data: *mut u8, len: usize) {
    if !data.is_null() && len > 0 {
        let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(data, len));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::path::Path;

    #[test]
    fn test_version() {
        let version = unpdf_version();
        assert!(!version.is_null());
        let version_str = unsafe { CStr::from_ptr(version) }.to_str().unwrap();
        assert!(!version_str.is_empty());
    }

    #[test]
    fn test_parse_null_path() {
        let doc = unsafe { unpdf_parse_file(ptr::null()) };
        assert!(doc.is_null());

        let error = unpdf_last_error();
        assert!(!error.is_null());
    }

    #[test]
    fn test_parse_invalid_path() {
        let path = CString::new("nonexistent.pdf").unwrap();
        let doc = unsafe { unpdf_parse_file(path.as_ptr()) };
        assert!(doc.is_null());

        let error = unpdf_last_error();
        assert!(!error.is_null());
    }

    #[test]
    fn test_parse_and_convert() {
        let path = "test-files/sample.pdf";
        if !Path::new(path).exists() {
            return;
        }

        let path_cstr = CString::new(path).unwrap();
        let doc = unsafe { unpdf_parse_file(path_cstr.as_ptr()) };
        assert!(!doc.is_null());

        // Test markdown conversion
        let md = unsafe { unpdf_to_markdown(doc, 0) };
        assert!(!md.is_null());
        unsafe { unpdf_free_string(md) };

        // Test text conversion
        let text = unsafe { unpdf_to_text(doc) };
        assert!(!text.is_null());
        unsafe { unpdf_free_string(text) };

        // Test JSON conversion
        let json = unsafe { unpdf_to_json(doc, UNPDF_JSON_PRETTY) };
        assert!(!json.is_null());
        unsafe { unpdf_free_string(json) };

        // Test section count
        let count = unsafe { unpdf_section_count(doc) };
        assert!(count >= 0);

        // Free document
        unsafe { unpdf_free_document(doc) };
    }

    #[test]
    fn test_null_document_operations() {
        let md = unsafe { unpdf_to_markdown(ptr::null(), 0) };
        assert!(md.is_null());

        let text = unsafe { unpdf_to_text(ptr::null()) };
        assert!(text.is_null());

        let json = unsafe { unpdf_to_json(ptr::null(), 0) };
        assert!(json.is_null());

        let count = unsafe { unpdf_section_count(ptr::null()) };
        assert_eq!(count, -1);

        let res_count = unsafe { unpdf_resource_count(ptr::null()) };
        assert_eq!(res_count, -1);
    }

    #[test]
    fn test_page_null_document() {
        let md = unsafe { unpdf_page_to_markdown(ptr::null(), 1, 0) };
        assert!(md.is_null());

        let text = unsafe { unpdf_page_to_text(ptr::null(), 1) };
        assert!(text.is_null());
    }

    #[test]
    fn test_free_null() {
        // Should not crash
        unsafe {
            unpdf_free_document(ptr::null_mut());
            unpdf_free_string(ptr::null_mut());
        }
    }
}
