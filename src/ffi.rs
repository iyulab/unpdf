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
//! to retrieve the error message.

use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::catch_unwind;
use std::ptr;

use crate::model::Document;
use crate::render::{JsonFormat, RenderOptions};

// Thread-local storage for the last error message.
thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Set the last error message.
fn set_last_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Clear the last error message.
fn clear_last_error() {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = None;
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
        set_last_error("path is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let path_str = CStr::from_ptr(path).to_str().map_err(|e| e.to_string())?;

        crate::parse_file(path_str)
            .map(|doc| Box::into_raw(Box::new(UnpdfDocument { inner: doc })))
            .map_err(|e| e.to_string())
    });

    match result {
        Ok(Ok(doc)) => doc,
        Ok(Err(e)) => {
            set_last_error(&e);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic occurred during parsing");
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
        set_last_error("data is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let bytes = std::slice::from_raw_parts(data, len);

        crate::parse_bytes(bytes)
            .map(|doc| Box::into_raw(Box::new(UnpdfDocument { inner: doc })))
            .map_err(|e| e.to_string())
    });

    match result {
        Ok(Ok(doc)) => doc,
        Ok(Err(e)) => {
            set_last_error(&e);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic occurred during parsing");
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
pub unsafe extern "C" fn unpdf_to_markdown(
    doc: *const UnpdfDocument,
    flags: u32,
) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error("document is null");
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

        crate::render::to_markdown(document, &options).map_err(|e| e.to_string())
    });

    match result {
        Ok(Ok(md)) => match CString::new(md) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error("output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(&e);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic occurred during rendering");
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
        set_last_error("document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        let options = RenderOptions::default();
        crate::render::to_text(document, &options).map_err(|e| e.to_string())
    });

    match result {
        Ok(Ok(text)) => match CString::new(text) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error("output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(&e);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic occurred during rendering");
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
pub unsafe extern "C" fn unpdf_to_json(
    doc: *const UnpdfDocument,
    format: c_int,
) -> *mut c_char {
    clear_last_error();

    if doc.is_null() {
        set_last_error("document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        let json_format = if format == UNPDF_JSON_COMPACT {
            JsonFormat::Compact
        } else {
            JsonFormat::Pretty
        };
        crate::render::to_json(document, json_format).map_err(|e| e.to_string())
    });

    match result {
        Ok(Ok(json)) => match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error("output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(&e);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic occurred during rendering");
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
        set_last_error("document is null");
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
                set_last_error("output contains null byte");
                ptr::null_mut()
            }
        },
        Err(_) => {
            set_last_error("panic occurred");
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
        set_last_error("document is null");
        return -1;
    }

    match catch_unwind(|| (*doc).inner.pages.len() as c_int) {
        Ok(count) => count,
        Err(_) => {
            set_last_error("panic occurred");
            -1
        }
    }
}

/// Get the number of resources in a document.
///
/// # Safety
///
/// - `doc` must be a valid document handle.
/// - Returns -1 on error.
#[no_mangle]
pub unsafe extern "C" fn unpdf_resource_count(doc: *const UnpdfDocument) -> c_int {
    if doc.is_null() {
        set_last_error("document is null");
        return -1;
    }

    match catch_unwind(|| (*doc).inner.resources.len() as c_int) {
        Ok(count) => count,
        Err(_) => {
            set_last_error("panic occurred");
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
        set_last_error("document is null");
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
            set_last_error("panic occurred");
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
        set_last_error("document is null");
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
            set_last_error("panic occurred");
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
        set_last_error("document is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let document = &(*doc).inner;
        let ids: Vec<&String> = document.resources.keys().collect();
        serde_json::to_string(&ids).map_err(|e| e.to_string())
    });

    match result {
        Ok(Ok(json)) => match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error("output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(&e);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic occurred");
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
        set_last_error("document is null");
        return ptr::null_mut();
    }

    if resource_id.is_null() {
        set_last_error("resource_id is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let id_str = CStr::from_ptr(resource_id)
            .to_str()
            .map_err(|e| e.to_string())?;

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
                serde_json::to_string(&info).map_err(|e| e.to_string())
            }
            None => Err(format!("resource not found: {}", id_str)),
        }
    });

    match result {
        Ok(Ok(json)) => match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => {
                set_last_error("output contains null byte");
                ptr::null_mut()
            }
        },
        Ok(Err(e)) => {
            set_last_error(&e);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic occurred");
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
        set_last_error("document is null");
        return ptr::null_mut();
    }

    if resource_id.is_null() {
        set_last_error("resource_id is null");
        return ptr::null_mut();
    }

    if out_len.is_null() {
        set_last_error("out_len is null");
        return ptr::null_mut();
    }

    let result = catch_unwind(|| {
        let id_str = CStr::from_ptr(resource_id)
            .to_str()
            .map_err(|e| e.to_string())?;

        let document = &(*doc).inner;

        match document.resources.get(id_str) {
            Some(resource) => {
                let data = resource.data.clone();
                let len = data.len();
                let boxed = data.into_boxed_slice();
                let ptr = Box::into_raw(boxed) as *mut u8;
                Ok((ptr, len))
            }
            None => Err(format!("resource not found: {}", id_str)),
        }
    });

    match result {
        Ok(Ok((ptr, len))) => {
            *out_len = len;
            ptr
        }
        Ok(Err(e)) => {
            set_last_error(&e);
            *out_len = 0;
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic occurred");
            *out_len = 0;
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
    fn test_free_null() {
        // Should not crash
        unsafe {
            unpdf_free_document(ptr::null_mut());
            unpdf_free_string(ptr::null_mut());
        }
    }
}
