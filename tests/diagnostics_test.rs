use unpdf::ExtractionQuality;

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
