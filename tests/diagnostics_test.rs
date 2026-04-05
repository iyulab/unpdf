use std::path::Path;
use unpdf::{parse_file, ExtractionQuality};

#[test]
fn test_extraction_quality_from_text() {
    let q = ExtractionQuality::from_text("The quick brown fox jumps over the lazy dog");
    assert_eq!(q.char_count, 43);
    assert_eq!(q.word_count, 9);
    assert_eq!(q.replacement_char_count, 0);
    assert!(q.is_good());
    assert!(q.warning_message().is_none());
}

#[test]
fn test_extraction_quality_low() {
    // 3 replacement chars + 2 normal chars = 5 total, ratio = 0.6
    let q = ExtractionQuality::from_text("\u{FFFD}\u{FFFD}\u{FFFD}ab");
    assert_eq!(q.char_count, 5);
    assert_eq!(q.replacement_char_count, 3);
    assert!(!q.is_good());
    let msg = q.warning_message().unwrap();
    assert!(msg.contains("3 of 5"));
}

#[test]
fn test_extraction_quality_empty() {
    let q = ExtractionQuality::from_text("");
    assert_eq!(q.char_count, 0);
    assert_eq!(q.word_count, 0);
    assert!(!q.is_good());
    assert!(q.warning_message().is_some());
}

#[test]
fn test_basic_pdf_has_quality_metrics() {
    let path = Path::new("test-files/basic/trivial.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    assert!(doc.extraction_quality.char_count > 0);
    assert!(doc.extraction_quality.word_count > 0);
}

#[test]
fn test_encrypted_pdf_returns_error() {
    let path = Path::new("test-files/encrypted/password-protected.pdf");
    if !path.exists() {
        return;
    }
    let result = parse_file(path);
    assert!(result.is_err(), "Encrypted PDF should return error");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("encrypted") || msg.contains("Encrypted"),
        "Error should mention encryption: {}",
        msg
    );
}

#[test]
fn test_multicolumn_reading_order() {
    let path = Path::new("test-files/complex/multicolumn.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    let text = doc.plain_text();
    assert!(!text.is_empty(), "Should extract text from multicolumn PDF");
}

#[test]
fn test_two_column_reading_order() {
    let path = Path::new("test-files/complex/two-column.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    let text = doc.plain_text();
    assert!(!text.is_empty(), "Should extract text from two-column PDF");
}

#[test]
fn test_toc_dot_leader_removal() {
    use unpdf::render::{CleanupPipeline, CleanupPreset};
    let pipeline = CleanupPipeline::from_preset(CleanupPreset::Standard);
    let input = "Chapter 1: Introduction ................................ 6\n\
                 Chapter 2: Methods ...................................... 12\n\
                 Normal paragraph text without dots.";
    let output = pipeline.process(input);
    assert!(!output.contains("................................"), "Dot leaders should be removed");
    assert!(output.contains("Introduction"));
    assert!(output.contains("Normal paragraph text"));
}

#[test]
fn test_image_pdf_has_content() {
    let path = Path::new("test-files/images/sample-with-images.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    let text = doc.plain_text();
    assert!(!text.is_empty(), "Should extract text from PDF with images");
}
