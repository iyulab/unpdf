use unpdf::parser::raw::RawDocument;

#[test]
fn test_parse_trivial_pdf() {
    let data = std::fs::read("test-files/basic/trivial.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
    assert!(!doc.version.is_empty());
}

#[test]
fn test_parse_outline_pdf() {
    let data = std::fs::read("test-files/basic/outline.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_sample_pdf() {
    let data = std::fs::read("test-files/basic/sample-1mb.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_cjk_korean() {
    let data = std::fs::read("test-files/cjk/korean-test.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_cjk_arabic() {
    let data = std::fs::read("test-files/cjk/arabic.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_complex_multicolumn() {
    let data = std::fs::read("test-files/complex/multicolumn.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_scientific_arxiv() {
    let data = std::fs::read("test-files/scientific/arxiv-sample.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_tables() {
    let data = std::fs::read("test-files/tables/sample-tables.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    // This PDF is encrypted; page tree uses ObjStm which can't be
    // decompressed without decryption, so page_count is 0.
    assert!(doc.is_encrypted());
}

#[test]
fn test_parse_images_pdf() {
    let data = std::fs::read("test-files/images/sample-with-images.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_forms_pdf() {
    let data = std::fs::read("test-files/forms/pdf-form-sample.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_encrypted_pdf_detected() {
    let data = std::fs::read("test-files/encrypted/password-protected.pdf").unwrap();
    // Should either detect encryption or fail gracefully, not panic
    match RawDocument::load(&data) {
        Ok(doc) => assert!(doc.is_encrypted()),
        Err(_) => {} // Encrypted PDF failing to load is acceptable
    }
}

#[test]
fn test_patent_document_vectorized_text() {
    use unpdf::parser::backend::{PdfBackend, RawBackend};

    // patent-document.pdf has text rendered as vector paths (outlines),
    // not as text operators. This is a known limitation — zero text ops expected.
    let raw = RawBackend::load_file("test-files/realworld/patent-document.pdf").unwrap();
    let pages = raw.pages();
    assert_eq!(pages.len(), 13);

    let first_page = *pages.values().next().unwrap();
    let content = raw.page_content(first_page).unwrap();
    let ops = raw.decode_content(&content).unwrap();
    let text_ops: usize = ops
        .iter()
        .filter(|op| op.operator == "Tj" || op.operator == "TJ")
        .count();
    // No text ops — text is drawn as vector paths
    assert_eq!(text_ops, 0);
}

#[test]
fn test_iphone_info_korean_extraction() {
    // Verify that Type1 fonts with custom encoding + ToUnicode CMap
    // correctly decode Korean text after the code_width auto-correction fix.
    let doc = unpdf::parse_file("test-files/realworld/iphone-info.pdf").unwrap();
    let text = doc.plain_text();
    assert!(
        text.contains("사용") && text.contains("설명서"),
        "Korean text '사용설명서' should be extracted"
    );
    assert!(
        text.contains("iPhone"),
        "English text should also be extracted"
    );
}
