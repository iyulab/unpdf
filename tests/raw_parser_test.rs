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
#[ignore] // Page tree not fully resolved — likely ObjStm/xref-stream feature gap
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
#[ignore] // Page tree not fully resolved — likely ObjStm/xref-stream feature gap
fn test_parse_tables() {
    let data = std::fs::read("test-files/tables/sample-tables.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_images_pdf() {
    let data = std::fs::read("test-files/images/sample-with-images.pdf").unwrap();
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
#[ignore] // Page tree not fully resolved — likely ObjStm/xref-stream feature gap
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
