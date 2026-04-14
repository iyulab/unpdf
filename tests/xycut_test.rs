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
}
