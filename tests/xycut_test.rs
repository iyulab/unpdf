use unpdf::parser::xycut::{
    text_column_gutter, xycut_partition, xycut_segment, Block, XyCutConfig,
};

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

/// A gutter barely wider than the minimum, where one line of the left column runs two
/// points into it — as a justified line's measured or estimated width does. The columns
/// still split: a block's right edge is read a little way in.
#[test]
fn a_line_reaching_slightly_into_a_narrow_gutter_does_not_close_it() {
    const BODY_7PT: XyCutConfig = XyCutConfig {
        min_x_gap: 60.0,
        min_y_gap: 36.0,
        min_gutter: 8.0,
    };
    let mut blocks = Vec::new();
    for i in 0..6 {
        let y = 700.0 - i as f32 * 8.0;
        // The left column ends at 290, the right one starts at 299: a 9-point gutter.
        let left_width = if i == 3 { 202.2 } else { 200.0 };
        blocks.push(make_block(90.0, y, left_width, 7.0));
        blocks.push(make_block(299.0, y, 200.0, 7.0));
    }
    let groups = xycut_segment(&blocks, &BODY_7PT);
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert!(groups[0].iter().all(|b| b.x < 100.0), "left column first");
}

/// Four columns of 7pt text, with a box set across the last three below them: the
/// middle gutters are closed by the box, and the gutter beside the first column leaves it a
/// quarter of the region. It is a column of text all the same, and is read on its own.
#[test]
fn an_outer_column_of_a_four_column_region_splits_off() {
    const BODY_7PT: XyCutConfig = XyCutConfig {
        min_x_gap: 60.0,
        min_y_gap: 36.0,
        min_gutter: 8.0,
    };
    let mut blocks = Vec::new();
    for column in 0..4 {
        let x = 297.0 + column as f32 * 79.0;
        // The first column runs the whole height, the others stop above the box.
        let lines = if column == 0 { 30 } else { 20 };
        for i in 0..lines {
            blocks.push(make_block(x, 500.0 - i as f32 * 8.0, 70.0, 7.0));
        }
    }
    // The box across columns 2-4, below their text and beside column 1's.
    blocks.push(make_block(376.0, 300.0, 227.0, 20.0));
    let groups = xycut_segment(&blocks, &BODY_7PT);
    assert!(
        groups[0].len() == 30 && groups[0].iter().all(|b| b.x < 300.0),
        "the first column is read first and on its own: {groups:?}"
    );
}

/// A table's narrow column of short figures beside its other columns: under the side share,
/// and its cells leave most of it empty, so it is not split off as a column of text.
#[test]
fn a_narrow_column_of_short_cells_is_not_split_off() {
    const BODY_7PT: XyCutConfig = XyCutConfig {
        min_x_gap: 60.0,
        min_y_gap: 36.0,
        min_gutter: 8.0,
    };
    let mut blocks = Vec::new();
    for i in 0..10 {
        let y = 500.0 - i as f32 * 8.0;
        // A 55-wide column of figures 14 wide under a heading cell that fills it, then text
        // across the rest of the row.
        let cell = if i == 0 { 55.0 } else { 14.0 };
        blocks.push(make_block(100.0, y, cell, 7.0));
        blocks.push(make_block(170.0, y, 330.0, 7.0));
    }
    let groups = xycut_segment(&blocks, &BODY_7PT);
    assert!(
        groups.iter().all(|g| g.iter().any(|b| b.x > 160.0)),
        "the figures stay with their rows: {groups:?}"
    );
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

/// A short running head centred over the gutter, a line or two above two columns: as
/// narrow as a column's line, it closes the gutter on its own line all the same. It is
/// banded off, and the columns read one after the other -- not their first lines across.
#[test]
fn a_short_running_head_over_the_gutter_is_banded_off() {
    let mut blocks = vec![make_block(271.0, 797.0, 56.0, 10.0)];
    for i in 0..6 {
        let y = 761.0 - i as f32 * 12.6;
        blocks.push(make_block(57.0, y, 229.0, 11.0));
        blocks.push(make_block(312.0, y, 229.0, 11.0));
    }
    let groups = xycut_segment(&blocks, &BODY_11PT);
    let order: Vec<(f32, f32)> = groups.iter().map(|g| (g[0].x, g[0].y)).collect();
    assert_eq!(
        order,
        vec![(271.0, 797.0), (57.0, 761.0), (312.0, 761.0)],
        "{groups:?}"
    );
}

/// The same with a running foot below whose first half -- as narrow as a column's line --
/// crosses the gutter too: two short lines across it, both banded off.
#[test]
fn a_running_head_and_foot_over_the_gutter_are_banded_off() {
    let mut blocks = vec![make_block(271.0, 797.0, 56.0, 10.0)];
    for i in 0..6 {
        let y = 761.0 - i as f32 * 12.6;
        blocks.push(make_block(57.0, y, 229.0, 11.0));
        blocks.push(make_block(312.0, y, 229.0, 11.0));
    }
    blocks.push(make_block(219.0, 660.0, 236.0, 10.0));
    blocks.push(make_block(470.0, 660.0, 70.0, 10.0));
    let groups = xycut_segment(&blocks, &BODY_11PT);
    let order: Vec<(f32, f32)> = groups.iter().map(|g| (g[0].x, g[0].y)).collect();
    assert_eq!(
        order,
        vec![
            (271.0, 797.0),
            (57.0, 761.0),
            (312.0, 761.0),
            (219.0, 660.0)
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

/// Two justified columns drawn one block per word: each line of `words` blocks `word` wide
/// and `space` apart, `lines` lines 18 apart, the right column starting at `right_x`.
fn word_per_block_columns(lines: usize, right_x: f32) -> Vec<Block> {
    let mut blocks = Vec::new();
    for i in 0..lines {
        let y = 700.0 - i as f32 * 18.0;
        for (x0, n) in [(72.0, 5), (right_x, 5)] {
            for w in 0..n {
                blocks.push(make_block(x0 + w as f32 * 38.0, y, 32.0, 10.0));
            }
        }
    }
    blocks
}

#[test]
fn columns_drawn_word_by_word_are_still_columns() {
    // Every block is a word — far narrower than a quarter of the region — but the lines
    // they make fill their columns.
    let blocks = word_per_block_columns(8, 280.0);
    let seg = xycut_partition(&blocks, &BODY_11PT);
    assert_eq!(seg.column_count(&blocks), 2, "{:?}", seg.groups);
    let gutter = text_column_gutter(&blocks, BODY_11PT.min_gutter).expect("a gutter");
    assert!(gutter > 262.0 && gutter < 280.0, "gutter at {gutter}");
}

#[test]
fn a_footnote_drawn_word_by_word_across_the_gutter_does_not_join_the_columns() {
    // A footnote under both columns, too close for a horizontal cut, drawn in word pieces
    // none of which crosses the gutter on its own.
    let mut blocks = word_per_block_columns(8, 280.0);
    for w in 0..10 {
        blocks.push(make_block(72.0 + w as f32 * 26.0, 545.0, 22.0, 8.0));
    }
    let seg = xycut_partition(&blocks, &BODY_11PT);
    assert_eq!(seg.column_count(&blocks), 2, "{:?}", seg.groups);
}

#[test]
fn a_figure_in_one_column_does_not_unmake_the_column() {
    // A chart's tick labels add many short lines to the left column; most of its text is
    // still in lines that fill it.
    let mut blocks = word_per_block_columns(8, 280.0);
    for t in 0..12 {
        blocks.push(make_block(72.0, 540.0 - t as f32 * 9.0, 10.0, 6.0));
        blocks.push(make_block(240.0, 540.0 - t as f32 * 9.0, 10.0, 6.0));
    }
    assert!(text_column_gutter(&blocks, BODY_11PT.min_gutter).is_some());
}

#[test]
fn the_gutter_is_found_past_a_wider_channel_beside_a_margin_tab() {
    // A thumb-index tab of stacked letters at the right margin leaves a channel wider
    // than the gutter; the gutter is the one with text on both sides.
    let mut blocks = word_per_block_columns(8, 280.0);
    for t in 0..8 {
        blocks.push(make_block(500.0, 700.0 - t as f32 * 8.0, 6.0, 6.0));
    }
    let gutter = text_column_gutter(&blocks, BODY_11PT.min_gutter).expect("a gutter");
    assert!(gutter < 280.0, "gutter at {gutter}");
}

#[test]
fn a_table_of_contents_is_not_two_text_columns() {
    // Entries against page numbers: the numbers make no column of text.
    let blocks: Vec<Block> = (0..8)
        .flat_map(|i| {
            let y = 700.0 - i as f32 * 18.0;
            [
                make_block(72.0, y, 220.0, 10.0),
                make_block(470.0, y, 12.0, 10.0),
            ]
        })
        .collect();
    assert!(text_column_gutter(&blocks, BODY_11PT.min_gutter).is_none());
}

/// One column of justified lines drawn word by word, 18 apart, from x=72 to about 260.
fn one_column(lines: usize) -> Vec<Block> {
    (0..lines)
        .flat_map(|i| {
            let y = 700.0 - i as f32 * 18.0;
            (0..5).map(move |w| make_block(72.0 + w as f32 * 38.0, y, 32.0, 10.0))
        })
        .collect()
}

#[test]
fn letters_stacked_down_the_margin_are_read_apart_from_the_text() {
    // A thumb-index tab: one letter every 7pt beside the column, mostly off its baselines.
    let mut blocks = one_column(8);
    let tab: Vec<usize> = (0..12)
        .map(|t| {
            blocks.push(make_block(300.0, 696.0 - t as f32 * 7.0, 6.0, 6.0));
            blocks.len() - 1
        })
        .collect();
    let seg = xycut_partition(&blocks, &BODY_11PT);
    let group_of = |i: usize| seg.groups.iter().position(|g| g.contains(&i)).unwrap();
    let tab_group = group_of(tab[0]);
    assert!(
        tab.iter().all(|&i| group_of(i) == tab_group),
        "{:?}",
        seg.groups
    );
    assert!(
        (0..40).all(|i| group_of(i) != tab_group),
        "the tab must not share a group with the text: {:?}",
        seg.groups
    );
}

#[test]
fn list_markers_on_the_texts_baselines_stay_with_it() {
    // Markers in a narrow strip, each on the baseline of the line it marks.
    let mut blocks: Vec<Block> = Vec::new();
    for i in 0..6 {
        let y = 700.0 - i as f32 * 18.0;
        blocks.push(make_block(72.0, y, 8.0, 10.0));
        blocks.push(make_block(100.0, y, 180.0, 10.0));
    }
    let seg = xycut_partition(&blocks, &BODY_11PT);
    assert_eq!(seg.groups.len(), 1, "{:?}", seg.groups);
}

#[test]
fn a_band_of_footnotes_closing_the_gutter_is_cut_off_from_the_columns() {
    // Footnotes under both columns, a line's space below them (too little for a plain
    // horizontal cut), some lines full width and some — continuations — no wider than a
    // column yet crossing the gutter.
    let mut blocks = word_per_block_columns(10, 280.0);
    let footnotes = [
        (72.0, 500.0, 300.0),
        (72.0, 490.0, 400.0),
        (86.0, 481.0, 200.0),
        (72.0, 471.0, 380.0),
        (86.0, 462.0, 210.0),
    ];
    for (x, y, w) in footnotes {
        // Drawn in word pieces.
        let mut at = x;
        while at < x + w {
            blocks.push(make_block(at, y, 18.0, 7.0));
            at += 21.0;
        }
    }
    let seg = xycut_partition(&blocks, &BODY_11PT);
    let group_of = |i: usize| seg.groups.iter().position(|g| g.contains(&i)).unwrap();
    // Left column's first word and right column's first word are read apart.
    assert_ne!(group_of(0), group_of(5), "{:?}", seg.groups);
    assert_eq!(seg.column_count(&blocks), 2, "{:?}", seg.groups);
}

#[test]
fn single_column_text_over_a_wide_table_is_not_cut_into_columns() {
    // One column of text; below it a table whose cells leave channels. Nothing above or
    // below reads as two columns of text, so no band is cut on that account.
    let mut blocks = one_column(8);
    for r in 0..4 {
        let y = 540.0 - r as f32 * 14.0;
        for c in 0..4 {
            blocks.push(make_block(72.0 + c as f32 * 60.0, y, 20.0, 10.0));
        }
    }
    let seg = xycut_partition(&blocks, &BODY_11PT);
    let group_of = |i: usize| seg.groups.iter().position(|g| g.contains(&i)).unwrap();
    // The text's words stay together, line by line.
    assert!(
        (0..40).all(|i| group_of(i) == group_of(0)),
        "{:?}",
        seg.groups
    );
}

/// The thresholds the layout pass uses for a 9.5pt body: a gutter is half an em.
const BODY_9_5PT: XyCutConfig = XyCutConfig {
    min_x_gap: 60.0,
    min_y_gap: 36.0,
    min_gutter: 4.75,
};

/// Two columns set six points apart in 9.5pt type -- two thirds of an em, as printed reports
/// set them. Read as one region, every line ran across both columns.
#[test]
fn a_gutter_two_thirds_of_an_em_wide_splits_two_columns() {
    let mut blocks = Vec::new();
    for i in 0..8 {
        let y = 700.0 - i as f32 * 12.0;
        blocks.push(make_block(94.0, y, 229.0, 9.5));
        blocks.push(make_block(329.0, y, 228.0, 9.5));
    }
    let groups = xycut_segment(&blocks, &BODY_9_5PT);
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert!(groups[0].iter().all(|b| b.x < 100.0), "left column first");
}

/// A two-line paragraph drawn a word at a time, its second line short: under the space after
/// one word of the first line there is a clear channel, with five words beyond it. Five words
/// are one line, not a column of text.
#[test]
fn a_short_paragraph_drawn_word_by_word_is_not_two_columns() {
    const BODY_11_5PT: XyCutConfig = XyCutConfig {
        min_x_gap: 60.0,
        min_y_gap: 36.0,
        min_gutter: 5.75,
    };
    let mut blocks = Vec::new();
    // First line: words up to 361, a 6pt space, then five more words to 560.
    let mut x = 54.0;
    for w in [41.0, 50.0, 20.0, 51.0, 62.0, 61.0 - 22.0] {
        blocks.push(make_block(x, 700.0, w, 11.5));
        x += w + 4.0;
    }
    let mut x = 367.0;
    for w in [22.0, 14.0, 46.0, 75.0, 20.0] {
        blocks.push(make_block(x, 700.0, w, 11.5));
        x += w + 4.0;
    }
    // Second line: short.
    let mut x = 54.0;
    for w in [24.0, 40.0, 20.0, 30.0, 18.0, 40.0] {
        blocks.push(make_block(x, 686.0, w, 11.5));
        x += w + 4.0;
    }
    let groups = xycut_segment(&blocks, &BODY_11_5PT);
    assert_eq!(groups.len(), 1, "{groups:?}");
}

#[test]
fn a_line_drawn_word_by_word_with_em_wide_spaces_is_not_two_columns() {
    const BODY_10PT: XyCutConfig = XyCutConfig {
        min_x_gap: 60.0,
        min_y_gap: 36.0,
        min_gutter: 5.0,
    };
    let mut blocks = Vec::new();
    let mut x = 72.0;
    for glyphs in [1.0, 3.0, 3.0, 3.0, 1.0, 3.0, 2.0, 6.0] {
        blocks.push(make_block(x, 709.0, glyphs * 10.0, 10.0));
        // Spaces a little wider than an em: past the old two-gutter join limit, not on it.
        x += glyphs * 10.0 + 12.0;
    }
    let groups = xycut_segment(&blocks, &BODY_10PT);
    assert_eq!(groups.len(), 1, "{groups:?}");
}

/// A byline centred under a title stands over the columns, not in the one below it. Its
/// descent reaches below the top of the columns' first line, so no horizontal cut sets it
/// apart; read as the middle column's first line it came after the whole left column.
/// (Coordinates from a magazine page: three columns 167 pt wide, a 12 pt byline over the
/// middle one.)
#[test]
fn a_byline_centred_over_three_columns_is_read_before_them() {
    let mut blocks = vec![make_block(226.6, 604.5, 157.2, 12.0)];
    for row in 0..8 {
        let top = 596.0 - row as f32 * 12.0;
        for x in [36.7, 222.3, 407.8] {
            blocks.push(make_block(x, top, 167.0, 10.0));
        }
    }
    let config = XyCutConfig {
        min_x_gap: 60.0,
        min_y_gap: 36.0,
        min_gutter: 5.0,
    };
    let seg = xycut_partition(&blocks, &config);
    assert_eq!(seg.groups.first(), Some(&vec![0]), "{:?}", seg.groups);
    assert_eq!(
        seg.groups.len(),
        4,
        "byline, then three columns: {:?}",
        seg.groups
    );
}

/// A column's own first line is not lifted out of it for standing higher than the other
/// columns begin: it starts at its column's edge, not centred on the region.
#[test]
fn a_column_starting_higher_than_the_others_keeps_its_first_line() {
    let mut blocks = vec![make_block(36.7, 608.0, 167.0, 10.0)];
    for row in 0..8 {
        let top = 596.0 - row as f32 * 12.0;
        for x in [36.7, 222.3, 407.8] {
            blocks.push(make_block(x, top, 167.0, 10.0));
        }
    }
    let config = XyCutConfig {
        min_x_gap: 60.0,
        min_y_gap: 36.0,
        min_gutter: 5.0,
    };
    let seg = xycut_partition(&blocks, &config);
    assert_eq!(seg.groups.len(), 3, "three columns: {:?}", seg.groups);
    assert!(seg.groups[0].contains(&0), "{:?}", seg.groups);
}

/// A heading over one column that starts at that column's edge is the column's, even when it
/// falls near the centre of the region (a sidebar beside two columns puts it there).
#[test]
fn a_heading_at_a_columns_edge_near_the_regions_centre_stays_with_its_column() {
    // A 17 pt sidebar at the left; the heading over the middle column starts at its edge.
    let mut blocks = vec![make_block(277.3, 581.7, 227.7, 17.0)];
    for row in 0..8 {
        let top = 560.0 - row as f32 * 22.0;
        blocks.push(make_block(18.0, top + 4.0, 210.0, 17.0));
        blocks.push(make_block(277.3, top, 219.4, 11.0));
        blocks.push(make_block(547.0, top, 220.0, 11.0));
    }
    let config = XyCutConfig {
        min_x_gap: 60.0,
        min_y_gap: 36.0,
        min_gutter: 5.5,
    };
    let seg = xycut_partition(&blocks, &config);
    assert_ne!(seg.groups.first(), Some(&vec![0]), "{:?}", seg.groups);
}

/// Two column systems stacked with no whitespace band between them -- an article's three
/// narrow flows over a two-column body, say: the boundary is where the gutter moves, and each
/// system's columns read one after the other before the next system's.
#[test]
fn stacked_column_systems_split_where_the_gutter_moves() {
    let mut blocks = Vec::new();
    for i in 0..6 {
        let y = 700.0 - i as f32 * 13.0;
        blocks.push(make_block(72.0, y, 170.0, 11.0));
        blocks.push(make_block(256.0, y, 170.0, 11.0));
    }
    for i in 0..6 {
        let y = 620.0 - i as f32 * 13.0;
        blocks.push(make_block(72.0, y, 220.0, 11.0));
        blocks.push(make_block(306.0, y, 220.0, 11.0));
    }
    let groups = xycut_segment(&blocks, &BODY_11PT);
    let order: Vec<(f32, f32)> = groups.iter().map(|g| (g[0].x, g[0].y)).collect();
    assert_eq!(
        order,
        vec![(72.0, 700.0), (256.0, 700.0), (72.0, 620.0), (306.0, 620.0)],
        "{groups:?}"
    );
}

/// A title of two lines set over two columns, too close above them for a plain cut, with
/// a drop cap two lines tall opening the left column: the title is banded off as itself,
/// and the columns' first two rows stay in the columns.
#[test]
fn a_two_line_title_over_columns_is_banded_off_without_their_first_rows() {
    let mut blocks = vec![
        make_block(110.0, 760.0, 380.0, 20.0),
        make_block(150.0, 738.0, 300.0, 20.0),
        // The drop cap: two rows tall, so rows one and two overlap it.
        make_block(72.0, 714.0, 22.0, 24.0),
    ];
    for i in 0..8 {
        let y = 714.0 - i as f32 * 13.0;
        let left = if i < 2 { 98.0 } else { 72.0 };
        blocks.push(make_block(left, y, 292.0 - left, 11.0));
        blocks.push(make_block(306.0, y, 220.0, 11.0));
    }
    let groups = xycut_segment(&blocks, &BODY_11PT);
    let title = groups
        .iter()
        .find(|g| g.iter().any(|b| b.y == 760.0))
        .expect("the title is read");
    assert!(
        title.iter().all(|b| b.y >= 738.0),
        "the title's group holds no column row: {groups:?}"
    );
}

#[test]
fn text_set_absurdly_far_apart_is_segmented_in_a_bounded_profile() {
    let far = 1.0e30;
    let blocks = vec![
        make_block(0.0, 700.0, 200.0, 12.0),
        make_block(0.0, 680.0, 200.0, 12.0),
        make_block(far, 700.0, 200.0, 12.0),
        make_block(far, 680.0, 200.0, 12.0),
    ];
    let groups = xycut_segment(&blocks, &WIDE_ONLY);
    assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), blocks.len());
}

#[test]
fn text_spread_past_the_largest_float_is_left_unsplit() {
    let edge = 3.0e38;
    for blocks in [
        vec![
            make_block(-edge, 700.0, 200.0, 12.0),
            make_block(edge, 700.0, 200.0, 12.0),
        ],
        vec![
            make_block(72.0, -edge, 200.0, 12.0),
            make_block(72.0, edge, 200.0, 12.0),
        ],
    ] {
        let groups = xycut_segment(&blocks, &WIDE_ONLY);
        assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), blocks.len());
    }
}

#[test]
fn text_one_float_apart_at_an_extreme_position_is_left_unsplit() {
    let at = |bits: u32| f32::from_bits(bits);
    for blocks in [
        vec![
            make_block(at(0x7149_f2ca), 700.0, 200.0, 12.0),
            make_block(at(0x7149_f2cb), 700.0, 200.0, 12.0),
        ],
        vec![
            make_block(72.0, at(0x7149_f2cb), 200.0, 12.0),
            make_block(72.0, at(0x7149_f2cc), 200.0, 12.0),
        ],
    ] {
        let groups = xycut_segment(&blocks, &WIDE_ONLY);
        assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), blocks.len());
    }
}
