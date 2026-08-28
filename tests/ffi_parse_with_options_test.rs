//! `unpdf_parse_file_with_options` / `unpdf_parse_bytes_with_options` — the C ABI surface for
//! `ParseOptions`, previously unreachable from any C-ABI-based binding (C#, Python). A null
//! `options_json` must behave exactly like the option-less entry points; a non-null one must
//! actually reach `ParseOptions` (regression coverage for the resource-extraction opt-in this
//! surface exists to unblock — docket #125).
#![cfg(feature = "ffi")]

mod common;

use std::ffi::CString;

use common::image_only_pdf;
use unpdf::ffi::{
    unpdf_free_document, unpdf_last_error, unpdf_last_error_kind, unpdf_parse_bytes,
    unpdf_parse_bytes_with_options, unpdf_parse_file_with_options, unpdf_resource_count,
    UNPDF_ERROR_INVALID_ARGUMENT,
};

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// A one-page PDF with a single `DCTDecode`-tagged image XObject of the given pixel size.
/// The bytes are not a real decodable JPEG — nothing here decodes it — but the `Filter`
/// entry is what the parser uses to classify a resource as a renderable image format
/// (as opposed to the raw/undecoded pixel buffer `common::image_only_pdf` produces).
fn jpeg_pdf(width: u32, height: u32) -> Vec<u8> {
    let content = b"q 595 0 0 842 0 0 cm /Im0 Do Q\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</XObject<</Im0 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        stream_object(
            &format!(
                "<</Type/XObject/Subtype/Image/Width {width}/Height {height}\
                  /ColorSpace/DeviceRGB/BitsPerComponent 8/Filter/DCTDecode/Length 4>>"
            ),
            &[0xFF, 0xD8, 0xFF, 0xD9],
        ),
    ];
    assemble(objects)
}

fn stream_object(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut obj = dict.as_bytes().to_vec();
    obj.extend_from_slice(b"\nstream\n");
    obj.extend_from_slice(data);
    obj.extend_from_slice(b"\nendstream");
    obj
}

fn assemble(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }

    let xref_start = pdf.len();
    let size = objects.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in &offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xref_start}\n%%EOF\n")
            .as_bytes(),
    );
    pdf
}

#[test]
fn null_options_json_matches_the_option_less_entry_point() {
    let bytes = jpeg_pdf(100, 100);
    unsafe {
        let baseline = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!baseline.is_null());
        assert_eq!(unpdf_resource_count(baseline), 0);
        unpdf_free_document(baseline);

        let doc = unpdf_parse_bytes_with_options(bytes.as_ptr(), bytes.len(), std::ptr::null());
        assert!(!doc.is_null());
        assert_eq!(unpdf_resource_count(doc), 0);
        unpdf_free_document(doc);
    }
}

#[test]
fn extract_resources_opt_in_populates_the_resource_inventory() {
    let bytes = jpeg_pdf(100, 100);
    let options = cstr(r#"{"extract_resources":true}"#);
    unsafe {
        let doc = unpdf_parse_bytes_with_options(bytes.as_ptr(), bytes.len(), options.as_ptr());
        assert!(!doc.is_null());
        assert_eq!(
            unpdf_resource_count(doc),
            1,
            "extract_resources:true must reach ParseOptions and populate the resource inventory \
             (docket #125 — this was previously unreachable from any C-ABI binding)"
        );
        unpdf_free_document(doc);
    }
}

#[test]
fn raw_undecoded_image_formats_are_never_surfaced_even_with_extract_resources() {
    // `common::image_only_pdf` has no /Filter — an undecoded pixel buffer most consumers
    // (GetResourceData callers included) cannot render. `parse()`'s resource collection
    // must apply the same raw/bin exclusion `parse_single_page`'s already did, or this
    // silently reappears whenever the two collection paths drift (they had drifted: this
    // exact case returned resource_count 1 before the shared-filter fix in this cycle).
    let bytes = image_only_pdf();
    let options = cstr(r#"{"extract_resources":true,"min_image_dimension":0}"#);
    unsafe {
        let doc = unpdf_parse_bytes_with_options(bytes.as_ptr(), bytes.len(), options.as_ptr());
        assert!(!doc.is_null());
        assert_eq!(unpdf_resource_count(doc), 0);
        unpdf_free_document(doc);
    }
}

#[test]
fn min_image_dimension_still_drops_small_decodable_images_by_default() {
    let bytes = jpeg_pdf(10, 10);
    let options = cstr(r#"{"extract_resources":true}"#);
    unsafe {
        let doc = unpdf_parse_bytes_with_options(bytes.as_ptr(), bytes.len(), options.as_ptr());
        assert!(!doc.is_null());
        assert_eq!(
            unpdf_resource_count(doc),
            0,
            "10x10 is below the default 64px cutoff"
        );
        unpdf_free_document(doc);
    }
}

#[test]
fn min_image_dimension_override_keeps_the_small_decodable_image() {
    let bytes = jpeg_pdf(10, 10);
    let options = cstr(r#"{"extract_resources":true,"min_image_dimension":0}"#);
    unsafe {
        let doc = unpdf_parse_bytes_with_options(bytes.as_ptr(), bytes.len(), options.as_ptr());
        assert!(!doc.is_null());
        assert_eq!(unpdf_resource_count(doc), 1);
        unpdf_free_document(doc);
    }
}

#[test]
fn malformed_options_json_returns_null_with_invalid_argument() {
    let bytes = jpeg_pdf(100, 100);
    let options = cstr("not json");
    unsafe {
        let doc = unpdf_parse_bytes_with_options(bytes.as_ptr(), bytes.len(), options.as_ptr());
        assert!(doc.is_null());
        assert!(!unpdf_last_error().is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_INVALID_ARGUMENT);
    }
}

#[test]
fn parse_file_with_options_reaches_extract_resources_too() {
    let bytes = jpeg_pdf(100, 100);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "unpdf-ffi-with-options-test-{}.pdf",
        std::process::id()
    ));
    std::fs::write(&path, &bytes).unwrap();

    let path_cstr = cstr(path.to_str().unwrap());
    let options = cstr(r#"{"extract_resources":true}"#);
    unsafe {
        let doc = unpdf_parse_file_with_options(path_cstr.as_ptr(), options.as_ptr());
        assert!(!doc.is_null());
        assert_eq!(unpdf_resource_count(doc), 1);
        unpdf_free_document(doc);
    }

    let _ = std::fs::remove_file(&path);
}
