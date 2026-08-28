//! End-to-end lattice-mode table detection: real PDF bytes with drawn ruling
//! lines, through the actual public parse API — not unit-level `ContentOp`
//! construction. Confirms the extraction → grid-inference → cell-assignment
//! pipeline is actually wired into `PdfParser`'s output, not just internally
//! self-consistent.

mod common;

use unpdf::model::Block;
use unpdf::PdfParser;

#[test]
fn bordered_grid_is_extracted_as_a_table() {
    let doc = PdfParser::from_bytes(&common::bordered_table_pdf())
        .and_then(|p| p.parse())
        .expect("synthetic bordered-table PDF should parse");

    assert_eq!(doc.pages.len(), 1);
    let tables: Vec<&unpdf::model::Table> = doc.pages[0]
        .elements
        .iter()
        .filter_map(|b| match b {
            Block::Table(t) => Some(t),
            _ => None,
        })
        .collect();

    assert_eq!(
        tables.len(),
        1,
        "expected exactly one table from the drawn 2x2 grid, got elements: {:?}",
        doc.pages[0].elements
    );

    let table = tables[0];
    assert_eq!(table.row_count(), 2);
    assert_eq!(table.column_count(), 2);
    assert_eq!(table.rows[0].cells[0].plain_text(), "Name");
    assert_eq!(table.rows[0].cells[1].plain_text(), "Age");
    assert_eq!(table.rows[1].cells[0].plain_text(), "Alice");
    assert_eq!(table.rows[1].cells[1].plain_text(), "30");
}
