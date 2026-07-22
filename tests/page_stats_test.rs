//! Per-page content-stream operator statistics.
//!
//! Consumers (FileFlux 등) need to tell an image-only scanned page apart from a
//! genuinely blank page. The parser counts text-showing operators (`Tj`/`TJ`/`'`/`"`)
//! and XObject `Do` invocations per page so the distinction survives into the model.

mod common;

use common::{blank_pdf, image_only_pdf, mixed_pdf, text_pdf};
use unpdf::parse_bytes;

#[test]
fn image_only_page_has_image_ops_and_no_text_ops() {
    let doc = parse_bytes(&image_only_pdf()).unwrap();
    assert_eq!(doc.pages.len(), 1);
    assert_eq!(doc.pages[0].text_op_count, 0);
    assert!(doc.pages[0].image_op_count >= 1);
    // 기존 doc-level 판별과 일관성 유지.
    assert!(doc.extraction_quality.is_scan_pdf);
}

#[test]
fn text_page_has_text_ops_and_no_image_ops() {
    let doc = parse_bytes(&text_pdf()).unwrap();
    assert_eq!(doc.pages.len(), 1);
    assert!(doc.pages[0].text_op_count >= 1);
    assert_eq!(doc.pages[0].image_op_count, 0);
}

#[test]
fn blank_page_has_no_ops() {
    let doc = parse_bytes(&blank_pdf()).unwrap();
    assert_eq!(doc.pages.len(), 1);
    assert_eq!(doc.pages[0].text_op_count, 0);
    assert_eq!(doc.pages[0].image_op_count, 0);
}

/// 혼합 문서(1p 텍스트 + 2p 이미지)의 페이지 단위 판별 — doc-level `is_scan_pdf`
/// 는 텍스트 발견 시 조기 false 라 이 구분을 못 하므로 페이지 통계가 유일한 경로.
#[test]
fn mixed_document_distinguishes_pages() {
    let doc = parse_bytes(&mixed_pdf()).unwrap();
    assert_eq!(doc.pages.len(), 2);
    assert!(doc.pages[0].text_op_count >= 1);
    assert_eq!(doc.pages[0].image_op_count, 0);
    assert_eq!(doc.pages[1].text_op_count, 0);
    assert!(doc.pages[1].image_op_count >= 1);
    assert!(!doc.extraction_quality.is_scan_pdf);
}
