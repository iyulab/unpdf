//! C-ABI FFI bindings for cross-language integration.
//!
//! This module provides a C-compatible API for using unpdf from other languages
//! such as C#, Python, and Node.js.

use std::ffi::{c_char, CStr, CString};
use std::path::Path;
use std::ptr;

use crate::render::{JsonFormat, RenderOptions};
use crate::{parse_file, render};

/// Result structure returned by FFI functions.
#[repr(C)]
pub struct UnpdfResult {
    /// Whether the operation succeeded.
    pub success: bool,
    /// The result data (null if failed). Must be freed with `unpdf_free_string`.
    pub data: *mut c_char,
    /// Error message (null if succeeded). Must be freed with `unpdf_free_string`.
    pub error: *mut c_char,
}

impl UnpdfResult {
    fn success(data: String) -> Self {
        Self {
            success: true,
            data: CString::new(data).unwrap_or_default().into_raw(),
            error: ptr::null_mut(),
        }
    }

    fn error(message: String) -> Self {
        Self {
            success: false,
            data: ptr::null_mut(),
            error: CString::new(message).unwrap_or_default().into_raw(),
        }
    }
}

/// Convert a PDF file to Markdown.
///
/// # Safety
///
/// The `path` must be a valid null-terminated UTF-8 string.
/// The returned result must be freed with `unpdf_free_result`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_to_markdown(path: *const c_char) -> UnpdfResult {
    if path.is_null() {
        return UnpdfResult::error("Path cannot be null".to_string());
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return UnpdfResult::error("Invalid UTF-8 path".to_string()),
    };

    match to_markdown_internal(Path::new(path_str)) {
        Ok(markdown) => UnpdfResult::success(markdown),
        Err(e) => UnpdfResult::error(e.to_string()),
    }
}

fn to_markdown_internal(path: &Path) -> crate::Result<String> {
    let doc = parse_file(path)?;
    let options = RenderOptions::default();
    render::to_markdown(&doc, &options)
}

/// Convert a PDF file to plain text.
///
/// # Safety
///
/// The `path` must be a valid null-terminated UTF-8 string.
/// The returned result must be freed with `unpdf_free_result`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_to_text(path: *const c_char) -> UnpdfResult {
    if path.is_null() {
        return UnpdfResult::error("Path cannot be null".to_string());
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return UnpdfResult::error("Invalid UTF-8 path".to_string()),
    };

    match to_text_internal(Path::new(path_str)) {
        Ok(text) => UnpdfResult::success(text),
        Err(e) => UnpdfResult::error(e.to_string()),
    }
}

fn to_text_internal(path: &Path) -> crate::Result<String> {
    let doc = parse_file(path)?;
    let options = RenderOptions::default();
    render::to_text(&doc, &options)
}

/// Convert a PDF file to JSON.
///
/// # Safety
///
/// The `path` must be a valid null-terminated UTF-8 string.
/// The returned result must be freed with `unpdf_free_result`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_to_json(path: *const c_char, pretty: bool) -> UnpdfResult {
    if path.is_null() {
        return UnpdfResult::error("Path cannot be null".to_string());
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return UnpdfResult::error("Invalid UTF-8 path".to_string()),
    };

    let format = if pretty {
        JsonFormat::Pretty
    } else {
        JsonFormat::Compact
    };

    match to_json_internal(Path::new(path_str), format) {
        Ok(json) => UnpdfResult::success(json),
        Err(e) => UnpdfResult::error(e.to_string()),
    }
}

fn to_json_internal(path: &Path, format: JsonFormat) -> crate::Result<String> {
    let doc = parse_file(path)?;
    render::to_json(&doc, format)
}

/// Get document information as JSON.
///
/// # Safety
///
/// The `path` must be a valid null-terminated UTF-8 string.
/// The returned result must be freed with `unpdf_free_result`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_get_info(path: *const c_char) -> UnpdfResult {
    if path.is_null() {
        return UnpdfResult::error("Path cannot be null".to_string());
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return UnpdfResult::error("Invalid UTF-8 path".to_string()),
    };

    match get_info_internal(Path::new(path_str)) {
        Ok(info) => UnpdfResult::success(info),
        Err(e) => UnpdfResult::error(e.to_string()),
    }
}

fn get_info_internal(path: &Path) -> crate::Result<String> {
    let doc = parse_file(path)?;
    let info = serde_json::json!({
        "title": doc.metadata.title,
        "author": doc.metadata.author,
        "subject": doc.metadata.subject,
        "keywords": doc.metadata.keywords,
        "creator": doc.metadata.creator,
        "producer": doc.metadata.producer,
        "created": doc.metadata.created.map(|d| d.to_rfc3339()),
        "modified": doc.metadata.modified.map(|d| d.to_rfc3339()),
        "pdf_version": doc.metadata.pdf_version,
        "page_count": doc.metadata.page_count,
        "encrypted": doc.metadata.encrypted,
    });
    Ok(serde_json::to_string_pretty(&info).unwrap_or_default())
}

/// Get the page count of a PDF file.
///
/// # Safety
///
/// The `path` must be a valid null-terminated UTF-8 string.
/// Returns -1 on error.
#[no_mangle]
pub unsafe extern "C" fn unpdf_get_page_count(path: *const c_char) -> i32 {
    if path.is_null() {
        return -1;
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    match parse_file(path_str) {
        Ok(doc) => doc.metadata.page_count as i32,
        Err(_) => -1,
    }
}

/// Check if a file is a valid PDF.
///
/// # Safety
///
/// The `path` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn unpdf_is_pdf(path: *const c_char) -> bool {
    if path.is_null() {
        return false;
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };

    crate::detect::detect_format_from_path(Path::new(path_str)).is_ok()
}

/// Free a result returned by any unpdf function.
///
/// # Safety
///
/// The `result` must have been returned by an unpdf function.
/// This function should only be called once per result.
#[no_mangle]
pub unsafe extern "C" fn unpdf_free_result(result: UnpdfResult) {
    if !result.data.is_null() {
        drop(CString::from_raw(result.data));
    }
    if !result.error.is_null() {
        drop(CString::from_raw(result.error));
    }
}

/// Free a string allocated by unpdf.
///
/// # Safety
///
/// The `ptr` must have been allocated by unpdf.
/// This function should only be called once per pointer.
#[no_mangle]
pub unsafe extern "C" fn unpdf_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

/// Get the version of the unpdf library.
///
/// # Safety
///
/// The returned string is statically allocated and should not be freed.
#[no_mangle]
pub extern "C" fn unpdf_version() -> *const c_char {
    static VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();
    VERSION.as_ptr() as *const c_char
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        let version = unpdf_version();
        assert!(!version.is_null());
    }

    #[test]
    fn test_null_path() {
        unsafe {
            let result = unpdf_to_markdown(ptr::null());
            assert!(!result.success);
            assert!(!result.error.is_null());
            unpdf_free_result(result);
        }
    }

    #[test]
    fn test_is_pdf_null() {
        unsafe {
            assert!(!unpdf_is_pdf(ptr::null()));
        }
    }

    #[test]
    fn test_get_page_count_null() {
        unsafe {
            assert_eq!(unpdf_get_page_count(ptr::null()), -1);
        }
    }
}
