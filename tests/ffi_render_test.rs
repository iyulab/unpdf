//! `unpdf_render_page`, `unpdf_page_size` and `unpdf_page_pixels`: a page of a document as a
//! PNG, and its size before it is painted, through the C ABI.
#![cfg(feature = "ffi")]

mod common;

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use common::{image_only_pdf, page_pdf, text_pdf};
use unpdf::error::ErrorKind;
use unpdf::ffi::{
    unpdf_free_bytes, unpdf_free_document, unpdf_free_string, unpdf_last_error_kind,
    unpdf_open_bytes_with_options, unpdf_page_pixels, unpdf_page_size, unpdf_parse_bytes,
    unpdf_render_page,
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
        assert_eq!(info["gaps"]["colors"], 0);

        unpdf_free_document(doc);
    }
}

#[test]
fn a_color_that_does_not_convert_is_reported_in_the_gaps() {
    let bytes = page_pdf(
        "",
        "/MediaBox[0 0 100 100]",
        b"/Nope cs 1 scn 0 0 50 50 re f",
    );
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        let mut len = 0usize;
        let mut info: *mut c_char = ptr::null_mut();
        let png = unpdf_render_page(doc, 1, ptr::null(), &mut len, &mut info);
        assert!(!png.is_null());
        unpdf_free_bytes(png, len);
        let info: serde_json::Value = serde_json::from_str(&take_string(info)).unwrap();
        assert_eq!(info["gaps"]["colors"], 1, "{info}");
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
fn painting_past_max_work_stops_with_its_kind() {
    let bytes = image_only_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        let mut len = 7usize;
        // The first charge is the page's own pixels: 595 x 842 at 72 dpi.
        let options = CString::new(r#"{"dpi": 72, "max_work": 1000}"#).unwrap();
        let png = unpdf_render_page(doc, 1, options.as_ptr(), &mut len, ptr::null_mut());
        assert!(png.is_null());
        assert_eq!(len, 0);
        assert_eq!(unpdf_last_error_kind(), ErrorKind::RasterStopped as i32);

        let options = CString::new(r#"{"dpi": 72, "max_work": 100000000}"#).unwrap();
        let png = unpdf_render_page(doc, 1, options.as_ptr(), &mut len, ptr::null_mut());
        assert!(!png.is_null(), "a budget the page fits in paints it");
        unpdf_free_bytes(png, len);
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
            r#"{"max_work": -1}"#,
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

fn size_of(
    doc: *const unpdf::ffi::UnpdfDocument,
    page: i32,
    region: *const c_char,
) -> serde_json::Value {
    unsafe { serde_json::from_str(&take_string(unpdf_page_size(doc, page, region))).unwrap() }
}

#[test]
fn a_pages_size_is_read_before_it_is_painted() {
    let bytes = page_pdf(
        "",
        "/MediaBox[0 0 595 842]/CropBox[100 100 300 400]/Rotate 90",
        b"",
    );
    unsafe {
        let doc = unpdf_open_bytes_with_options(bytes.as_ptr(), bytes.len(), ptr::null());
        assert!(!doc.is_null());

        let crop = size_of(doc, 1, ptr::null());
        assert_eq!(
            (crop["width"].as_f64(), crop["height"].as_f64()),
            (Some(300.0), Some(200.0))
        );
        let named = CString::new("crop").unwrap();
        assert_eq!(size_of(doc, 1, named.as_ptr()), crop);
        let media = CString::new("media").unwrap();
        let whole = size_of(doc, 1, media.as_ptr());
        assert_eq!(
            (whole["width"].as_f64(), whole["height"].as_f64()),
            (Some(842.0), Some(595.0))
        );

        let options = CString::new(r#"{"dpi": 72}"#).unwrap();
        let mut len = 0usize;
        let mut info: *mut c_char = ptr::null_mut();
        let png = unpdf_render_page(doc, 1, options.as_ptr(), &mut len, &mut info);
        assert!(!png.is_null());
        unpdf_free_bytes(png, len);
        let info: serde_json::Value = serde_json::from_str(&take_string(info)).unwrap();
        assert_eq!(
            (info["width"].as_f64(), info["height"].as_f64()),
            (crop["width"].as_f64(), crop["height"].as_f64())
        );
        unpdf_free_document(doc);
    }
}

#[test]
fn a_sizes_pixels_are_the_pixels_render_page_paints() {
    let bytes = page_pdf("", "/MediaBox[0 0 396 612]", b"");
    unsafe {
        let doc = unpdf_open_bytes_with_options(bytes.as_ptr(), bytes.len(), ptr::null());
        assert!(!doc.is_null());
        let size = size_of(doc, 1, ptr::null());
        let side = |name: &str| size[name].as_f64().unwrap() as f32;
        let (mut width, mut height) = (0u32, 0u32);
        assert_eq!(
            unpdf_page_pixels(side("width"), side("height"), 93.0, &mut width, &mut height),
            0
        );
        assert_eq!((width, height), (511, 791));

        let options = CString::new(r#"{"dpi": 93}"#).unwrap();
        let mut len = 0usize;
        let mut info: *mut c_char = ptr::null_mut();
        let png = unpdf_render_page(doc, 1, options.as_ptr(), &mut len, &mut info);
        assert!(!png.is_null());
        unpdf_free_bytes(png, len);
        let info: serde_json::Value = serde_json::from_str(&take_string(info)).unwrap();
        assert_eq!(
            (info["width"].as_u64(), info["height"].as_u64()),
            (Some(u64::from(width)), Some(u64::from(height)))
        );
        unpdf_free_document(doc);

        assert_eq!(
            unpdf_page_pixels(396.0, 612.0, 93.0, ptr::null_mut(), &mut height),
            -1
        );
        assert_eq!(
            unpdf_last_error_kind(),
            unparser_shared::kind::INVALID_ARGUMENT
        );
    }
}

#[test]
fn a_size_that_cannot_be_read_fails_with_its_kind() {
    let bytes = image_only_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(unpdf_page_size(doc, 2, ptr::null()).is_null());
        assert_eq!(unpdf_last_error_kind(), ErrorKind::PageOutOfRange as i32);
        assert!(unpdf_page_size(doc, 0, ptr::null()).is_null());
        assert_eq!(unpdf_last_error_kind(), ErrorKind::PageOutOfRange as i32);
        let bleed = CString::new("bleed").unwrap();
        assert!(unpdf_page_size(doc, 1, bleed.as_ptr()).is_null());
        assert_eq!(
            unpdf_last_error_kind(),
            unparser_shared::kind::INVALID_ARGUMENT
        );
        assert!(unpdf_page_size(ptr::null(), 1, ptr::null()).is_null());
        assert_eq!(
            unpdf_last_error_kind(),
            unparser_shared::kind::INVALID_ARGUMENT
        );
        unpdf_free_document(doc);
    }

    let unbounded = page_pdf(
        "",
        &format!("/MediaBox[0 0 1{}.0 792]", "0".repeat(39)),
        b"",
    );
    unsafe {
        let doc = unpdf_open_bytes_with_options(unbounded.as_ptr(), unbounded.len(), ptr::null());
        assert!(!doc.is_null());
        assert!(unpdf_page_size(doc, 1, ptr::null()).is_null());
        assert_eq!(unpdf_last_error_kind(), ErrorKind::Render as i32);
        unpdf_free_document(doc);
    }
}
