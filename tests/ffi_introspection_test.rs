//! FFI introspection surface: extraction quality and per-page stats.
//!
//! FileFlux 등 C-ABI 소비자가 "빈 텍스트"의 원인(스캔본/빈 페이지/파싱 오류)을
//! 구분할 수 있도록 `unpdf_get_extraction_quality` / `unpdf_page_stats` 를 검증한다.
#![cfg(feature = "ffi")]

mod common;

use std::ffi::CStr;
use std::os::raw::c_char;
use std::ptr;

use common::{
    bordered_table_pdf, image_only_pdf, page_pdf, page_pdf_with_xobjects, stream_object,
    suppressed_text_run_pdf, text_pdf,
};
use unpdf::ffi::{
    unpdf_free_document, unpdf_free_string, unpdf_get_extraction_quality, unpdf_last_error,
    unpdf_page_stats, unpdf_parse_bytes, unpdf_tables,
};

/// Helper: consume an FFI string result into an owned Rust String.
unsafe fn take_string(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null());
    let s = CStr::from_ptr(ptr).to_str().unwrap().to_owned();
    unpdf_free_string(ptr);
    s
}

#[test]
fn extraction_quality_reports_scan_pdf() {
    let bytes = image_only_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        let json = take_string(unpdf_get_extraction_quality(doc));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["is_scan_pdf"], true);
        assert_eq!(v["char_count"], 0);

        unpdf_free_document(doc);
    }
}

#[test]
fn extraction_quality_reports_text_pdf() {
    let bytes = text_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        let json = take_string(unpdf_get_extraction_quality(doc));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["is_scan_pdf"], false);
        assert!(v["char_count"].as_u64().unwrap() > 0);

        unpdf_free_document(doc);
    }
}

#[test]
fn page_stats_distinguishes_image_only_page() {
    let bytes = image_only_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        let json = take_string(unpdf_page_stats(doc, 1));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["text_op_count"], 0);
        assert!(v["image_op_count"].as_u64().unwrap() >= 1);
        assert_eq!(v["ocr_text_suppressed"], false);
        assert_eq!(v["link_annotations_incomplete"], false);
        // A full-page scan: the image covers the page, and there is no text to order.
        assert!(
            (v["image_coverage"].as_f64().unwrap() - 1.0).abs() < 1e-3,
            "{json}"
        );
        assert_eq!(v["bilevel_image_coverage"], 0.0, "{json}");
        assert_eq!(v["rotation"], 0);
        assert_eq!(v["reading_regions"], 0);
        assert_eq!(v["column_count"], 0);
        for key in [
            "rotated_text_runs",
            "fill_op_count",
            "invisible_text_chars",
            "painted_text_chars",
            "margin_text_chars",
            "omitted_page_number_chars",
            "clipped_text_runs",
            "blank_text_runs",
            "undecoded_text_codes",
            "ruled_grids",
            "ruled_tables",
            "ambiguous_layout_regions",
        ] {
            assert_eq!(v[key], 0, "{key} in {json}");
        }

        unpdf_free_document(doc);
    }
}

#[test]
fn page_stats_reports_text_page() {
    let bytes = text_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        let json = take_string(unpdf_page_stats(doc, 1));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["text_op_count"].as_u64().unwrap() >= 1);
        assert_eq!(v["image_op_count"], 0);
        assert_eq!(v["image_coverage"], 0.0);
        assert_eq!(v["reading_regions"], 1);
        assert_eq!(v["column_count"], 1);
        // Always present, so a consumer never has to test for absence.
        assert_eq!(v["unreadable_fonts"], serde_json::json!([]), "{json}");

        unpdf_free_document(doc);
    }
}

#[test]
fn page_stats_reports_omitted_page_number_chars() {
    let bytes = page_pdf(
        "",
        "/MediaBox[0 0 595 842]",
        b"BT /F1 12 Tf 72 700 Td (Body text.) Tj ET \
          BT /F1 10 Tf 290 20 Td (12) Tj ET",
    );
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());
        let json = take_string(unpdf_page_stats(doc, 1));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["omitted_page_number_chars"], 2, "{json}");
        unpdf_free_document(doc);
    }
}

#[test]
fn page_stats_reports_incomplete_annotations_after_a_scan_bound() {
    let bytes = page_pdf(
        "",
        &format!("/MediaBox[0 0 595 842]/Annots[{}]", "null ".repeat(4097)),
        b"BT /F1 12 Tf 72 700 Td (Body text.) Tj ET",
    );
    unsafe {
        let document = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!document.is_null());
        let json = take_string(unpdf_page_stats(document, 1));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["link_annotations_incomplete"], true, "{json}");
        unpdf_free_document(document);
    }
}

#[test]
fn page_stats_reports_bilevel_image_coverage() {
    let stencil = stream_object(
        "<</Type/XObject/Subtype/Image/Width 8/Height 8/ImageMask true/Length 1>>",
        &[0x80],
    );
    let bytes = page_pdf_with_xobjects(
        "/MediaBox[0 0 595 842]",
        b"q 595 0 0 421 0 0 cm /Im Do Q",
        vec![("Im", stencil)],
    );
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());
        let json = take_string(unpdf_page_stats(doc, 1));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        for key in ["image_coverage", "bilevel_image_coverage"] {
            assert!(
                (value[key].as_f64().unwrap() - 0.5).abs() < 1e-3,
                "{key} in {json}"
            );
        }
        unpdf_free_document(doc);
    }
}

#[test]
fn page_stats_out_of_range_returns_null_with_error() {
    let bytes = text_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        let stats = unpdf_page_stats(doc, 99);
        assert!(stats.is_null());
        assert!(!unpdf_last_error().is_null());

        unpdf_free_document(doc);
    }
}

#[test]
fn null_document_returns_null() {
    unsafe {
        assert!(unpdf_get_extraction_quality(ptr::null()).is_null());
        assert!(unpdf_page_stats(ptr::null(), 1).is_null());
    }
}

#[test]
fn page_stats_names_the_unreadable_font() {
    let bytes = suppressed_text_run_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        let json = take_string(unpdf_page_stats(doc, 1));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v["unreadable_fonts"],
            serde_json::json!([{"name": "NoMap", "reason": "composite_unresolved", "runs": 1}]),
            "{json}"
        );
        assert_eq!(v["suppressed_text_runs"], 1, "{json}");

        unpdf_free_document(doc);
    }
}

#[test]
fn tables_come_as_csv_with_their_place() {
    let bytes = bordered_table_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        let json = take_string(unpdf_tables(doc, 0));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v,
            serde_json::json!([{"page": 1, "index": 1, "text": "Name,Age\r\nAlice,30\r\n"}]),
            "{json}"
        );
        let tsv: serde_json::Value =
            serde_json::from_str(&take_string(unpdf_tables(doc, 1))).unwrap();
        assert_eq!(tsv[0]["text"], "Name\tAge\r\nAlice\t30\r\n");

        unpdf_free_document(doc);
    }
}

#[test]
fn a_document_without_tables_has_an_empty_list() {
    let bytes = text_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());
        assert_eq!(take_string(unpdf_tables(doc, 0)), "[]");
        unpdf_free_document(doc);
    }
}
