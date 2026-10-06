//! What a page record says about how well the page was read: where images cover it, the
//! page box and rotation, rotated text, ruled grids, and the reading order's regions.

mod common;

use common::{bordered_table_pdf, page_pdf};
use unpdf::parse_bytes;

const A4: &str = "/MediaBox[0 0 595 842]";

fn only_page(pdf: &[u8]) -> unpdf::model::Page {
    let doc = parse_bytes(pdf).unwrap();
    assert_eq!(doc.pages.len(), 1);
    doc.pages.into_iter().next().unwrap()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn a_full_page_image_covers_the_page_and_a_logo_does_not() {
    let scan = only_page(&page_pdf("", A4, b"q 595 0 0 842 0 0 cm /Im0 Do Q"));
    assert!(close(scan.image_coverage, 1.0), "{}", scan.image_coverage);

    let logo = only_page(&page_pdf("", A4, b"q 60 0 0 60 72 700 cm /Im0 Do Q"));
    assert_eq!(logo.image_op_count, 1);
    assert!(
        close(logo.image_coverage, 3600.0 / (595.0 * 842.0)),
        "{}",
        logo.image_coverage
    );
}

#[test]
fn coverage_is_the_union_of_image_rectangles_clipped_to_the_page() {
    // The same top half painted twice covers half the page, not all of it.
    let twice = only_page(&page_pdf(
        "",
        A4,
        b"q 595 0 0 421 0 421 cm /Im0 Do Q q 595 0 0 421 0 421 cm /Im0 Do Q",
    ));
    assert_eq!(twice.image_op_count, 2);
    assert!(close(twice.image_coverage, 0.5), "{}", twice.image_coverage);

    // An image hanging off the right edge counts only where it is on the page.
    let off_edge = only_page(&page_pdf("", A4, b"q 595 0 0 842 300 0 cm /Im0 Do Q"));
    assert!(
        close(off_edge.image_coverage, 295.0 / 595.0),
        "{}",
        off_edge.image_coverage
    );
}

#[test]
fn an_inline_image_is_an_image_paint() {
    let pdf = page_pdf(
        "",
        A4,
        b"q 595 0 0 842 0 0 cm BI /W 1 /H 1 /CS /G /BPC 8 ID \x80 EI Q",
    );
    let doc = parse_bytes(&pdf).unwrap();
    let page = &doc.pages[0];
    assert_eq!(page.image_op_count, 1);
    assert!(close(page.image_coverage, 1.0), "{}", page.image_coverage);
    assert!(
        doc.extraction_quality.is_scan_pdf,
        "an inline-image scan is a scan"
    );
}

#[test]
fn a_page_box_off_the_origin_keeps_its_size_and_its_margins() {
    // The box starts at (100, 100): 595 x 842, with its bottom margin at y = 100..142.
    let pdf = page_pdf(
        "",
        "/MediaBox[100 100 695 942]",
        b"q 595 0 0 842 100 100 cm /Im0 Do Q \
          BT /F1 11 Tf 160 500 Td (Body text in the middle of the page.) Tj ET \
          BT /F1 11 Tf 390 110 Td (12) Tj ET",
    );
    let doc = parse_bytes(&pdf).unwrap();
    let page = &doc.pages[0];
    assert_eq!((page.width, page.height), (595.0, 842.0));
    assert!(close(page.image_coverage, 1.0), "{}", page.image_coverage);

    let text = doc.plain_text();
    assert!(text.contains("Body text"), "{text}");
    assert!(
        !text.contains("12"),
        "the page number in the bottom margin is dropped: {text}"
    );
}

#[test]
fn rotation_is_read_from_the_page_tree_and_normalized() {
    let inherited = only_page(&page_pdf("/Rotate 90", A4, b""));
    assert_eq!(inherited.rotation, 90);

    let own = only_page(&page_pdf("/Rotate 90", &format!("{A4}/Rotate -90"), b""));
    assert_eq!(own.rotation, 270, "the page's own value wins, normalized");

    let none = only_page(&page_pdf("", A4, b""));
    assert_eq!(none.rotation, 0);
}

#[test]
fn text_not_set_left_to_right_is_counted() {
    let page = only_page(&page_pdf(
        "",
        A4,
        b"BT /F1 12 Tf 72 700 Td (Horizontal line of text.) Tj ET \
          BT /F1 12 Tf 0 1 -1 0 500 300 Tm (Text running up the margin) Tj ET \
          BT /F1 12 Tf -1 0 0 -1 400 100 Tm (Upside down) Tj ET",
    ));
    assert_eq!(page.rotated_text_runs, 2);
}

#[test]
fn ruled_grids_and_the_tables_built_from_them() {
    let table = only_page(&bordered_table_pdf());
    assert!(table.ruled_grids >= 1);
    assert_eq!(table.ruled_tables, table.ruled_grids);

    // A drawn 2x2 grid with nothing in it is a grid, not a table.
    let empty_grid = only_page(&page_pdf(
        "",
        A4,
        b"BT /F1 11 Tf 72 780 Td (A line above an empty grid.) Tj ET \
          0.5 w 100 400 m 400 400 l S 100 500 m 400 500 l S 100 600 m 400 600 l S \
          100 400 m 100 600 l S 250 400 m 250 600 l S 400 400 m 400 600 l S",
    ));
    assert_eq!(empty_grid.ruled_grids, 1);
    assert_eq!(empty_grid.ruled_tables, 0);
}

/// Five lines in a left column and four in a right one, the right set half a line lower —
/// columns of running text, whose lines do not pair up into table rows.
fn two_columns(left_x: f32, left_text: &str, right_x: f32, right_text: &str) -> Vec<u8> {
    let mut content = String::new();
    for i in 0..5 {
        let y = 700 - i * 14;
        content.push_str(&format!(
            "BT /F1 11 Tf {left_x} {y} Td ({left_text} {i}) Tj ET "
        ));
        if i < 4 {
            let y = y - 7;
            content.push_str(&format!(
                "BT /F1 11 Tf {right_x} {y} Td ({right_text} {i}) Tj ET "
            ));
        }
    }
    page_pdf("", A4, content.as_bytes())
}

#[test]
fn two_columns_are_two_regions_side_by_side() {
    let page = only_page(&two_columns(
        72.0,
        "Left column of running text set to fill",
        320.0,
        "Right column of running text set to fill",
    ));
    assert_eq!(page.reading_regions, 2);
    assert_eq!(page.column_count, 2);
    assert_eq!(page.ambiguous_layout_regions, 0);
}

#[test]
fn a_single_column_is_one_region() {
    let mut content = String::new();
    for i in 0..5 {
        content.push_str(&format!(
            "BT /F1 11 Tf 72 {} Td (A full line of body text running across the page {i}.) Tj ET ",
            700 - i * 14
        ));
    }
    let page = only_page(&page_pdf("", A4, content.as_bytes()));
    assert_eq!(page.reading_regions, 1);
    assert_eq!(page.column_count, 1);
    assert_eq!(page.ambiguous_layout_regions, 0);
}

/// A narrow sidebar beside the body, a short gutter between them: each side reads as a
/// column of text, but the sidebar spans less of the region than the gutter rule asks
/// for, so the region is read across — and the record says the reading order guessed.
#[test]
fn columns_the_split_rules_decline_are_reported_as_ambiguous() {
    let page = only_page(&two_columns(
        72.0,
        "Sidebar note text here",
        215.0,
        "The body column runs much wider than the sidebar does",
    ));
    assert_eq!(page.reading_regions, 1, "the split rules declined this one");
    assert_eq!(page.ambiguous_layout_regions, 1);
}
