//! FFI error classification: `unpdf_last_error_kind`.
//!
//! 파싱/렌더링이 **실패**했을 때 C-ABI 소비자가 사유를 문자열 매칭 없이 분류할 수 있어야 한다.
//! (`claudedocs/issues/ISSUE-unpdf-20260723-image-only-parse-robustness.md` 기대 동작 #2)
//!
//! 메시지와 kind 는 언제나 함께 쓰이고 함께 지워진다 — 그 커플링이 여기서 검증하는 핵심이다.
#![cfg(feature = "ffi")]

mod common;

use std::ffi::CString;
use std::ptr;

use common::text_pdf;
use unpdf::ffi::{
    unpdf_free_document, unpdf_free_string, unpdf_last_error, unpdf_last_error_kind,
    unpdf_page_stats, unpdf_page_to_text, unpdf_parse_bytes, unpdf_parse_file,
    UNPDF_ERROR_INVALID_ARGUMENT, UNPDF_ERROR_NONE,
};
use unpdf::ErrorKind;

#[test]
fn null_argument_is_classified_as_invalid_argument() {
    unsafe {
        let doc = unpdf_parse_file(ptr::null());
        assert!(doc.is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_INVALID_ARGUMENT);
        assert!(!unpdf_last_error().is_null());
    }
}

#[test]
fn non_pdf_bytes_are_classified_not_left_generic() {
    let junk = b"this is plainly not a PDF at all";
    unsafe {
        let doc = unpdf_parse_bytes(junk.as_ptr(), junk.len());
        assert!(doc.is_null());

        // 어떤 사유든 분류되어야 한다 — "메시지는 있는데 kind 는 NONE" 이 되면 표면이 무의미하다.
        let kind = unpdf_last_error_kind();
        assert_ne!(kind, UNPDF_ERROR_NONE);
        assert!(!unpdf_last_error().is_null());
    }
}

#[test]
fn page_out_of_range_is_classified() {
    let bytes = text_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        assert!(unpdf_page_stats(doc, 99).is_null());
        assert_eq!(unpdf_last_error_kind(), ErrorKind::PageOutOfRange as i32);

        assert!(unpdf_page_to_text(doc, 99).is_null());
        assert_eq!(unpdf_last_error_kind(), ErrorKind::PageOutOfRange as i32);

        unpdf_free_document(doc);
    }
}

/// 성공 호출은 kind 를 반드시 되돌려 놓는다 — 그러지 않으면 소비자가 직전 실패를
/// 현재 결과의 사유로 오독한다.
#[test]
fn successful_call_clears_the_previous_kind() {
    let bytes = text_pdf();
    unsafe {
        // 먼저 실패를 하나 남긴다.
        assert!(unpdf_parse_file(ptr::null()).is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_INVALID_ARGUMENT);

        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_NONE);
        assert!(unpdf_last_error().is_null());

        let stats = unpdf_page_stats(doc, 1);
        assert!(!stats.is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_NONE);
        unpdf_free_string(stats);

        unpdf_free_document(doc);
    }
}

/// 다른 스레드의 실패가 이 스레드의 분류를 오염시키지 않는다 (TLS 이므로).
#[test]
fn error_kind_is_thread_local() {
    unsafe {
        assert!(unpdf_parse_file(ptr::null()).is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_INVALID_ARGUMENT);
    }

    let other = std::thread::spawn(|| unpdf_last_error_kind())
        .join()
        .unwrap();
    assert_eq!(other, UNPDF_ERROR_NONE);
}

/// 존재하지 않는 경로는 I/O 실패로 분류된다 — 손상된 PDF 와 구분 가능해야 한다.
#[test]
fn missing_file_is_classified_as_io() {
    let path = CString::new("./definitely-not-a-real-file-9f3a.pdf").unwrap();
    unsafe {
        let doc = unpdf_parse_file(path.as_ptr());
        assert!(doc.is_null());
        assert_eq!(unpdf_last_error_kind(), ErrorKind::Io as i32);
    }
}
