use std::path::Path;
use unpdf::parser::raw::RawDocument;

/// 픽스처가 없으면 (CI — `test-files/` gitignored) 읽어 반환, 없으면 None.
fn try_read(rel: &str) -> Option<Vec<u8>> {
    if !Path::new(rel).exists() {
        eprintln!("skipping: fixture not present at {}", rel);
        return None;
    }
    std::fs::read(rel).ok()
}

#[test]
fn test_parse_trivial_pdf() {
    let Some(data) = try_read("test-files/basic/trivial.pdf") else { return };
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
    assert!(!doc.version.is_empty());
}

#[test]
fn test_parse_outline_pdf() {
    let Some(data) = try_read("test-files/basic/outline.pdf") else { return };
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_sample_pdf() {
    let Some(data) = try_read("test-files/basic/sample-1mb.pdf") else { return };
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_cjk_korean() {
    let Some(data) = try_read("test-files/cjk/korean-test.pdf") else { return };
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_cjk_arabic() {
    let Some(data) = try_read("test-files/cjk/arabic.pdf") else { return };
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_complex_multicolumn() {
    let Some(data) = try_read("test-files/complex/multicolumn.pdf") else { return };
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_scientific_arxiv() {
    let Some(data) = try_read("test-files/scientific/arxiv-sample.pdf") else { return };
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_tables() {
    let Some(data) = try_read("test-files/tables/sample-tables.pdf") else { return };
    // This PDF is encrypted. load() now attempts decryption with empty password.
    match RawDocument::load(&data) {
        Ok(doc) => {
            assert!(doc.is_encrypted());
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("encrypted") || msg.contains("Encrypted"),
                "Error should be about encryption: {}",
                msg
            );
        }
    }
}

#[test]
fn test_parse_images_pdf() {
    let Some(data) = try_read("test-files/images/sample-with-images.pdf") else { return };
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_parse_forms_pdf() {
    let Some(data) = try_read("test-files/forms/pdf-form-sample.pdf") else { return };
    let doc = RawDocument::load(&data).unwrap();
    assert!(doc.page_count() > 0);
}

#[test]
fn test_encrypted_pdf_detected() {
    let Some(data) = try_read("test-files/encrypted/password-protected.pdf") else { return };
    match RawDocument::load(&data) {
        Ok(doc) => assert!(doc.is_encrypted()),
        Err(_) => {}
    }
}

#[test]
fn test_patent_document_vectorized_text() {
    use unpdf::parser::backend::{PdfBackend, RawBackend};

    let path = "test-files/realworld/patent-document.pdf";
    if !Path::new(path).exists() {
        eprintln!("skipping: fixture not present at {}", path);
        return;
    }
    let raw = RawBackend::load_file(path).unwrap();
    let pages = raw.pages();
    assert_eq!(pages.len(), 13);

    let first_page = *pages.values().next().unwrap();
    let content = raw.page_content(first_page).unwrap();
    let ops = raw.decode_content(&content).unwrap();
    let text_ops: usize = ops
        .iter()
        .filter(|op| op.operator == "Tj" || op.operator == "TJ")
        .count();
    assert_eq!(text_ops, 0);
}

#[test]
fn test_iphone_info_korean_extraction() {
    let path = "test-files/realworld/iphone-info.pdf";
    if !Path::new(path).exists() {
        eprintln!("skipping: fixture not present at {}", path);
        return;
    }
    let doc = unpdf::parse_file(path).unwrap();
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
