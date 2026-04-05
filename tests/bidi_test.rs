use unpdf::parser::bidi;

#[test]
fn test_contains_rtl_arabic() {
    assert!(bidi::contains_rtl("مرحبا"));
    assert!(bidi::contains_rtl("Hello مرحبا World"));
}

#[test]
fn test_contains_rtl_hebrew() {
    assert!(bidi::contains_rtl("שלום"));
}

#[test]
fn test_contains_rtl_latin() {
    assert!(!bidi::contains_rtl("Hello World"));
    assert!(!bidi::contains_rtl("123 abc"));
}

#[test]
fn test_reorder_bidi_latin() {
    // Latin text should pass through unchanged
    let result = bidi::reorder_bidi("Hello World");
    assert_eq!(result, "Hello World");
}

#[test]
fn test_reorder_bidi_arabic() {
    // Just verify it doesn't panic and returns non-empty
    let result = bidi::reorder_bidi("مرحبا");
    assert!(!result.is_empty());
}

#[test]
fn test_arabic_pdf_extraction() {
    use std::path::Path;
    let path = Path::new("test-files/cjk/arabic.pdf");
    if !path.exists() {
        return;
    }
    let doc = unpdf::parse_file(path).unwrap();
    let text = doc.plain_text();
    assert!(!text.is_empty(), "Should extract text from Arabic PDF");
}
