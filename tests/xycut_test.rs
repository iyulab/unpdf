use unpdf::parser::xycut::{xycut_segment, Block};

fn make_block(x: f32, y: f32, w: f32, h: f32) -> Block {
    Block {
        x,
        y,
        width: w,
        height: h,
    }
}

#[test]
fn test_single_column() {
    let blocks = vec![
        make_block(72.0, 700.0, 200.0, 12.0),
        make_block(72.0, 680.0, 180.0, 12.0),
        make_block(72.0, 660.0, 210.0, 12.0),
    ];
    let groups = xycut_segment(&blocks, 20.0, 15.0);
    assert_eq!(groups.len(), 1);
}

#[test]
fn test_two_columns() {
    let blocks = vec![
        make_block(72.0, 700.0, 200.0, 12.0),
        make_block(72.0, 680.0, 200.0, 12.0),
        make_block(350.0, 700.0, 200.0, 12.0),
        make_block(350.0, 680.0, 200.0, 12.0),
    ];
    let groups = xycut_segment(&blocks, 20.0, 15.0);
    assert_eq!(groups.len(), 2, "Should detect two columns");
    assert!(
        groups[0][0].x < groups[1][0].x,
        "Left column should come first"
    );
}

#[test]
fn test_header_plus_two_columns() {
    let blocks = vec![
        make_block(72.0, 750.0, 468.0, 14.0),
        make_block(72.0, 700.0, 200.0, 12.0),
        make_block(72.0, 680.0, 200.0, 12.0),
        make_block(350.0, 700.0, 200.0, 12.0),
        make_block(350.0, 680.0, 200.0, 12.0),
    ];
    let groups = xycut_segment(&blocks, 20.0, 15.0);
    assert!(
        groups.len() >= 2,
        "Should separate header from columns, got {}",
        groups.len()
    );
}

#[test]
fn test_three_columns() {
    let blocks = vec![
        make_block(30.0, 700.0, 150.0, 12.0),
        make_block(30.0, 680.0, 150.0, 12.0),
        make_block(220.0, 700.0, 150.0, 12.0),
        make_block(220.0, 680.0, 150.0, 12.0),
        make_block(410.0, 700.0, 150.0, 12.0),
        make_block(410.0, 680.0, 150.0, 12.0),
    ];
    let groups = xycut_segment(&blocks, 20.0, 15.0);
    assert_eq!(groups.len(), 3, "Should detect three columns");
}

#[test]
fn test_empty_input() {
    let groups = xycut_segment(&[], 20.0, 15.0);
    assert!(groups.is_empty());
}

#[test]
fn test_single_block() {
    let blocks = vec![make_block(72.0, 700.0, 200.0, 12.0)];
    let groups = xycut_segment(&blocks, 20.0, 15.0);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 1);
}

// --- Integration tests: XY-Cut layout via parse_file ---

use std::path::Path;
use unpdf::parse_file;

#[test]
fn test_multicolumn_pdf_with_xycut() {
    let path = Path::new("test-files/complex/multicolumn.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    let text = doc.plain_text();
    assert!(!text.is_empty());
}

#[test]
fn test_two_column_pdf_with_xycut() {
    let path = Path::new("test-files/complex/two-column.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    let text = doc.plain_text();
    assert!(!text.is_empty());
    // T-C3-2 regression: two-column PDFs must produce substantial text without
    // mid-sentence word splits at column boundaries.
    let word_count = text.split_whitespace().count();
    assert!(
        word_count > 100,
        "Two-column PDF should extract substantial text, got {} words",
        word_count
    );
    // No excessive word-splitting (common column join artifacts look like "text\nmore text")
    // The quality should be "good" (not encrypted, not scan)
    assert!(
        doc.extraction_quality.is_good(),
        "Two-column PDF quality should be good"
    );
}

#[test]
fn test_scan_pdf_detection() {
    let path = Path::new("test-files/realworld/patent-document.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    // T-C1-4 regression: image-only Hancom PDF must be classified as scan
    assert!(
        doc.extraction_quality.is_scan_pdf,
        "patent-document.pdf should be detected as scan PDF"
    );
    assert_eq!(
        doc.extraction_quality.char_count, 0,
        "Scan PDF should have no extracted text"
    );
    let warning = doc.extraction_quality.warning_message();
    assert!(warning.is_some(), "Scan PDF should emit a warning");
    assert!(
        warning.unwrap().contains("scanned image"),
        "Warning should mention scanned image"
    );
}

#[test]
fn test_extraction_quality_serializes_is_scan_pdf() {
    let q = unpdf::ExtractionQuality {
        char_count: 0,
        word_count: 0,
        replacement_char_count: 0,
        encrypted: false,
        suppressed_ocr_pages: 0,
        is_scan_pdf: true,
    };
    let json = serde_json::to_string(&q).unwrap();
    assert!(
        json.contains("is_scan_pdf"),
        "is_scan_pdf should appear: {json}"
    );
    assert!(json.contains("true"), "is_scan_pdf should be true: {json}");
}
