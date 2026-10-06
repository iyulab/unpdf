use unpdf::parser::xycut::{xycut_partition, xycut_segment, Block, XyCutConfig};

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

/// A caption spanning both columns blocks the gutter channel, so neither a vertical
/// nor a horizontal cut is clean. The columns above and below it are still columns:
/// the region splits into bands at the spanning block and each band reads left
/// column, then right.
#[test]
fn a_block_spanning_the_gutter_splits_the_page_into_bands() {
    let mut blocks = Vec::new();
    for i in 0..4 {
        let y = 700.0 - i as f32 * 13.0;
        blocks.push(make_block(72.0, y, 220.0, 11.0));
        blocks.push(make_block(306.0, y, 220.0, 11.0));
    }
    // The caption sits one line below the band above — no 36pt band of whitespace.
    blocks.push(make_block(72.0, 635.0, 454.0, 11.0));
    for i in 0..4 {
        let y = 622.0 - i as f32 * 13.0;
        blocks.push(make_block(72.0, y, 220.0, 11.0));
        blocks.push(make_block(306.0, y, 220.0, 11.0));
    }
    let groups = xycut_segment(&blocks, &BODY_11PT);
    let order: Vec<(f32, f32)> = groups.iter().map(|g| (g[0].x, g[0].y)).collect();
    assert_eq!(
        order,
        vec![
            (72.0, 700.0),
            (306.0, 700.0),
            (72.0, 635.0),
            (72.0, 622.0),
            (306.0, 622.0)
        ],
        "{groups:?}"
    );
}

/// Single-column text whose short lines (paragraph ends) happen to leave a channel:
/// the short lines are a minority, so wide lines are not cut into bands.
#[test]
fn single_column_text_with_short_lines_is_not_banded() {
    let mut blocks = Vec::new();
    for i in 0..10 {
        let y = 700.0 - i as f32 * 13.0;
        let w = if i % 3 == 2 { 150.0 } else { 454.0 };
        blocks.push(make_block(72.0, y, w, 11.0));
    }
    let groups = xycut_segment(&blocks, &BODY_11PT);
    assert_eq!(groups.len(), 1, "{groups:?}");
}

/// A table of contents: entries on the left, page numbers far right, one entry long
/// enough to run across the channel between them. The page numbers are not a text
/// column, so the long entry is not a spanning block to band at.
#[test]
fn a_table_of_contents_is_not_banded_at_its_long_entry() {
    let mut blocks = Vec::new();
    for i in 0..8 {
        let y = 700.0 - i as f32 * 13.0;
        let w = if i == 4 { 440.0 } else { 160.0 };
        blocks.push(make_block(72.0, y, w, 11.0));
        blocks.push(make_block(516.0, y, 10.0, 11.0));
    }
    let groups = xycut_segment(&blocks, &BODY_11PT);
    assert_eq!(groups.len(), 1, "{groups:?}");
}

/// Partitioning assigns every block to exactly one group, by index — even blocks drawn at
/// the same position, which matching by position would put in both groups or neither.
#[test]
fn partition_assigns_every_block_once() {
    let blocks = vec![
        make_block(72.0, 700.0, 200.0, 12.0),
        make_block(72.0, 700.0, 200.0, 12.0), // drawn twice, same spot
        make_block(72.0, 680.0, 200.0, 12.0),
        make_block(350.0, 700.0, 200.0, 12.0),
        make_block(350.0, 680.0, 200.0, 12.0),
    ];
    let seg = xycut_partition(&blocks, &WIDE_ONLY);
    let mut seen: Vec<usize> = seg.groups.iter().flatten().copied().collect();
    seen.sort();
    assert_eq!(seen, [0, 1, 2, 3, 4]);
    assert_eq!(seg.groups.len(), 2);
    assert_eq!(seg.column_count(&blocks), 2);
    assert_eq!(seg.ambiguous_regions, 0);
}

/// A heading over two columns: three regions, at most two side by side.
#[test]
fn column_count_is_the_most_regions_side_by_side() {
    let blocks = vec![
        make_block(72.0, 750.0, 468.0, 14.0),
        make_block(72.0, 700.0, 200.0, 12.0),
        make_block(72.0, 680.0, 200.0, 12.0),
        make_block(350.0, 700.0, 200.0, 12.0),
        make_block(350.0, 680.0, 200.0, 12.0),
    ];
    let seg = xycut_partition(&blocks, &WIDE_ONLY);
    assert_eq!(seg.groups.len(), 3);
    assert_eq!(seg.column_count(&blocks), 2);
}

/// Two columns of text whose gutter the split rules decline (the left side spans too
/// little of the region): read across, and reported as a guess.
#[test]
fn declined_columns_are_ambiguous() {
    let mut blocks = Vec::new();
    for i in 0..4 {
        let y = 700.0 - i as f32 * 14.0;
        blocks.push(make_block(72.0, y, 120.0, 11.0));
        blocks.push(make_block(214.0, y, 290.0, 11.0));
    }
    let seg = xycut_partition(&blocks, &BODY_11PT);
    assert_eq!(seg.groups.len(), 1);
    assert_eq!(seg.ambiguous_regions, 1);
}

/// A list's markers beside its items leave a channel too, but markers are not a column of
/// text: not ambiguous.
#[test]
fn list_markers_are_not_a_column() {
    let mut blocks = Vec::new();
    for i in 0..4 {
        let y = 700.0 - i as f32 * 14.0;
        blocks.push(make_block(72.0, y, 6.0, 11.0));
        blocks.push(make_block(90.0, y, 400.0, 11.0));
    }
    let seg = xycut_partition(&blocks, &BODY_11PT);
    assert_eq!(seg.groups.len(), 1);
    assert_eq!(seg.ambiguous_regions, 0);
}
