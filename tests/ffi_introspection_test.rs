//! FFI introspection surface: extraction quality and per-page stats.
//!
//! FileFlux 등 C-ABI 소비자가 "빈 텍스트"의 원인(스캔본/빈 페이지/파싱 오류)을
//! 구분할 수 있도록 `unpdf_get_extraction_quality` / `unpdf_page_stats` 를 검증한다.
#![cfg(feature = "ffi")]

mod common;

use std::ffi::CStr;
use std::os::raw::c_char;
use std::ptr;

use common::{image_only_pdf, text_pdf};
use unpdf::ffi::{
    unpdf_free_document, unpdf_free_string, unpdf_get_extraction_quality, unpdf_last_error,
    unpdf_page_stats, unpdf_parse_bytes,
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
