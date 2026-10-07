//! `unpdf_render_page`: a page of a parsed document as a PNG, through the C ABI.
#![cfg(feature = "ffi")]

mod common;

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use common::{image_only_pdf, text_pdf};
use unpdf::error::ErrorKind;
use unpdf::ffi::{
    unpdf_free_bytes, unpdf_free_document, unpdf_free_string, unpdf_last_error_kind,
    unpdf_parse_bytes, unpdf_render_page,
};

unsafe fn take_string(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null());
    let s = CStr::from_ptr(ptr).to_str().unwrap().to_owned();
    unpdf_free_string(ptr);
    s
}

#[test]
fn a_page_renders_to_a_png_with_its_report() {
    let bytes = image_only_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        let options = CString::new(r#"{"dpi": 72}"#).unwrap();
        let mut len = 0usize;
        let mut info: *mut c_char = ptr::null_mut();
        let png = unpdf_render_page(doc, 1, options.as_ptr(), &mut len, &mut info);
        assert!(!png.is_null());
        let data = std::slice::from_raw_parts(png, len);
        assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n");
        // The image-only fixture is an A4 page: 595 x 842 points, one pixel each at 72 dpi.
        assert_eq!(u32::from_be_bytes(data[16..20].try_into().unwrap()), 595);
        assert_eq!(u32::from_be_bytes(data[20..24].try_into().unwrap()), 842);
        unpdf_free_bytes(png, len);

        let info: serde_json::Value = serde_json::from_str(&take_string(info)).unwrap();
        assert_eq!(info["width"], 595);
        assert_eq!(info["height"], 842);
        assert_eq!(info["gaps"]["images"], 0);

        unpdf_free_document(doc);
    }
}

#[test]
fn text_in_a_standard_font_that_is_not_embedded_is_drawn_in_a_stand_in_face() {
    let bytes = text_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        let mut len = 0usize;
        let mut info: *mut c_char = ptr::null_mut();
        let png = unpdf_render_page(doc, 1, ptr::null(), &mut len, &mut info);
        assert!(!png.is_null(), "null options are the defaults");
        unpdf_free_bytes(png, len);
        let info: serde_json::Value = serde_json::from_str(&take_string(info)).unwrap();
        // The C ABI carries the stand-in faces: the text is painted, and reported as such.
        assert_eq!(info["gaps"]["text_runs"], 0, "{info}");
        assert!(
            info["substituted_text_runs"].as_u64().unwrap() >= 1,
            "{info}"
        );
        // 150 dpi by default.
        assert_eq!(info["width"], 1240);
        unpdf_free_document(doc);
    }
}

#[test]
fn a_page_out_of_range_fails_with_its_kind() {
    let bytes = image_only_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        let mut len = 7usize;
        let png = unpdf_render_page(doc, 2, ptr::null(), &mut len, ptr::null_mut());
        assert!(png.is_null());
        assert_eq!(len, 0);
        assert_eq!(unpdf_last_error_kind(), ErrorKind::PageOutOfRange as i32);
        unpdf_free_document(doc);
    }
}

#[test]
fn malformed_options_are_an_invalid_argument() {
    let bytes = image_only_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        let mut len = 0usize;
        for bad in [
            r#"{"region": "bleed"}"#,
            r#"{"dpi": "high"}"#,
            r#"{"scale": 2}"#,
        ] {
            let options = CString::new(bad).unwrap();
            let png = unpdf_render_page(doc, 1, options.as_ptr(), &mut len, ptr::null_mut());
            assert!(png.is_null(), "{bad}");
            assert_eq!(
                unpdf_last_error_kind(),
                unparser_shared::kind::INVALID_ARGUMENT,
                "{bad}"
            );
        }
        unpdf_free_document(doc);
    }
}
