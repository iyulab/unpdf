use unpdf::parser::xycut::{xycut_segment, Block, XyCutConfig};

/// Unconditional cuts only — the gutter rule disabled.
const WIDE_ONLY: XyCutConfig = XyCutConfig {
    min_x_gap: 20.0,
    min_y_gap: 15.0,
    min_gutter: 20.0,
};

/// The thresholds the layout pass uses for an 11pt body.
const BODY_11PT: XyCutConfig = XyCutConfig {
    min_x_gap: 60.0,
    min_y_gap: 36.0,
    min_gutter: 11.0,
};

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
    let groups = xycut_segment(&blocks, &WIDE_ONLY);
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
    let groups = xycut_segment(&blocks, &WIDE_ONLY);
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
    let groups = xycut_segment(&blocks, &WIDE_ONLY);
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
    let groups = xycut_segment(&blocks, &WIDE_ONLY);
    assert_eq!(groups.len(), 3, "Should detect three columns");
}

#[test]
fn test_empty_input() {
    let groups = xycut_segment(&[], &WIDE_ONLY);
    assert!(groups.is_empty());
}

#[test]
fn test_single_block() {
    let blocks = vec![make_block(72.0, 700.0, 200.0, 12.0)];
    let groups = xycut_segment(&blocks, &WIDE_ONLY);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 1);
}

/// Two text columns separated by a 14pt gutter: far below an unconditional cut, but
/// both sides are wide text, so it is a gutter.
#[test]
fn a_narrow_gutter_between_two_text_columns_splits() {
    let mut blocks = Vec::new();
    for i in 0..5 {
        let y = 700.0 - i as f32 * 13.0;
        blocks.push(make_block(72.0, y, 220.0, 11.0));
        blocks.push(make_block(306.0, y, 220.0, 11.0));
    }
    let groups = xycut_segment(&blocks, &BODY_11PT);
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert!(groups[0].iter().all(|b| b.x < 100.0), "left column first");
}

/// The channel between a numbered list's markers and its items is as narrow as a
/// gutter, but one side is a sliver: it must not split the list into a column of
/// numbers followed by a column of items.
#[test]
fn a_list_marker_channel_does_not_split() {
    let mut blocks = Vec::new();
    for i in 0..5 {
        let y = 700.0 - i as f32 * 13.0;
        blocks.push(make_block(72.0, y, 10.0, 11.0));
        blocks.push(make_block(96.0, y, 400.0, 11.0));
    }
    let groups = xycut_segment(&blocks, &BODY_11PT);
    assert_eq!(groups.len(), 1, "{groups:?}");
}

#[test]
fn test_extraction_quality_serializes_is_scan_pdf() {
    let q = unpdf::ExtractionQuality {
        is_scan_pdf: true,
        ..Default::default()
    };
    let json = serde_json::to_string(&q).unwrap();
    assert!(
        json.contains("is_scan_pdf"),
        "is_scan_pdf should appear: {json}"
    );
    assert!(json.contains("true"), "is_scan_pdf should be true: {json}");
}
