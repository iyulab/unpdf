//! Where tables land in the reading order of a page, and what is not a table.

mod common;

use common::page_pdf;
use unpdf::model::Block;
use unpdf::parse_bytes;

const A4: &str = "/MediaBox[0 0 595 842]";

/// The page's blocks, as text, in output order — a table reads as `[table: first cell]`.
fn reading_order(pdf: &[u8]) -> Vec<String> {
    let doc = parse_bytes(pdf).unwrap();
    doc.pages[0]
        .elements
        .iter()
        .map(|b| match b {
            Block::Table(t) => format!(
                "[table: {}]",
                t.rows
                    .first()
                    .and_then(|r| r.cells.first())
                    .map(|c| c.plain_text())
                    .unwrap_or_default()
                    .trim()
            ),
            other => {
                let mut text = String::new();
                other.append_plain_text(&mut text);
                text.trim().to_string()
            }
        })
        .collect()
}

fn position(order: &[String], needle: &str) -> usize {
    order
        .iter()
        .position(|t| t.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in {order:#?}"))
}

/// Two columns of running text, each line on its own height — the right column set half a
/// line lower — are text, not a table whose rows each hold one cell.
#[test]
fn staggered_text_columns_are_not_a_table() {
    let mut content = String::new();
    for i in 0..5 {
        let y = 700 - i * 14;
        content.push_str(&format!("BT /F1 11 Tf 72 {y} Td (Left line {i}) Tj ET "));
        if i < 4 {
            let y = y - 7;
            content.push_str(&format!("BT /F1 11 Tf 320 {y} Td (Right line {i}) Tj ET "));
        }
    }
    let doc = parse_bytes(&page_pdf("", A4, content.as_bytes())).unwrap();
    let tables = doc.pages[0]
        .elements
        .iter()
        .filter(|b| matches!(b, Block::Table(_)))
        .count();
    assert_eq!(tables, 0, "{:#?}", doc.pages[0].elements);
}

/// A ruled table set in the right column of a two-column page is read where it stands:
/// after the whole left column and the right column's text above it, before the right
/// column's text below it. Sorting every block by height interleaved the two columns.
#[test]
fn a_table_in_a_column_is_read_in_that_column() {
    let mut content = String::new();
    // Left column: eight lines from the top of the page down.
    for i in 0..8 {
        let y = 760 - i * 14;
        content.push_str(&format!(
            "BT /F1 11 Tf 72 {y} Td (Left column running text line number {i}) Tj ET "
        ));
    }
    // Right column: three lines above the table, three below.
    for i in 0..3 {
        let y = 760 - i * 14;
        content.push_str(&format!(
            "BT /F1 11 Tf 320 {y} Td (Right column text above the table {i}) Tj ET "
        ));
    }
    for i in 0..3 {
        let y = 620 - i * 14;
        content.push_str(&format!(
            "BT /F1 11 Tf 320 {y} Td (Right column text below the table {i}) Tj ET "
        ));
    }
    // A 2x2 ruled table in the right column, between y = 640 and 700.
    content.push_str(
        "1 w 320 700 m 520 700 l S 320 670 m 520 670 l S 320 640 m 520 640 l S \
         320 640 m 320 700 l S 420 640 m 420 700 l S 520 640 m 520 700 l S \
         BT /F1 11 Tf 330 682 Td (Metric) Tj ET BT /F1 11 Tf 430 682 Td (Value) Tj ET \
         BT /F1 11 Tf 330 652 Td (Speed) Tj ET BT /F1 11 Tf 430 652 Td (42) Tj ET ",
    );
    let order = reading_order(&page_pdf("", A4, content.as_bytes()));

    let left_last = position(&order, "line number 7");
    let right_above = position(&order, "above the table 0");
    let table = position(&order, "[table: Metric]");
    let right_below = position(&order, "below the table 0");
    assert!(
        left_last < right_above,
        "the left column comes first: {order:#?}"
    );
    assert!(right_above < table, "{order:#?}");
    assert!(table < right_below, "{order:#?}");
    assert!(
        order
            .iter()
            .all(|t| !(t.contains("Left column") && t.contains("Right column"))),
        "no paragraph joins the two columns: {order:#?}"
    );
}
