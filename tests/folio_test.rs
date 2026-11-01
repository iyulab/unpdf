//! A page number is left out by where it sits — the page's own margin, below (or above) all
//! other text — not by being a number: a chart's value or a table's row keeps its numbers.

mod common;

use common::{assemble, page_pdf, stream_object};
use unpdf::model::{Block, Page};
use unpdf::parser::{backend::RawBackend, LayoutAnalyzer};

const LETTER: &str = "/MediaBox[0 0 612 792]";

fn markdown(content: &str) -> String {
    let doc = unpdf::parse_bytes(&page_pdf("", LETTER, content.as_bytes())).expect("parses");
    unpdf::render::to_markdown(&doc, &unpdf::RenderOptions::default()).expect("renders")
}

fn parse_page(content: &str) -> Page {
    let doc = unpdf::parse_bytes(&page_pdf("", LETTER, content.as_bytes())).expect("parses");
    doc.pages.into_iter().next().unwrap()
}

const BODY: &str = "BT /F1 11 Tf 72 700 Td (A paragraph of body text on the page.) Tj ET\n\
                    BT /F1 11 Tf 72 686 Td (It goes on for a second line.) Tj ET\n";

/// A folio 9% above the bottom edge — inside the page's margin, outside the 5% band.
#[test]
fn a_folio_in_the_bottom_margin_is_left_out() {
    let md = markdown(&format!("{BODY}BT /F1 11 Tf 500 72 Td (362) Tj ET"));
    assert!(md.contains("body text"), "{md}");
    assert!(!md.contains("362"), "{md}");
}

/// A running title at one side and the folio at the other share a baseline.
#[test]
fn a_folio_beside_a_running_title_is_left_out() {
    let md = markdown(&format!(
        "{BODY}BT /F1 8 Tf 72 72 Td (Milken Institute School of Public Health) Tj ET\n\
         BT /F1 8 Tf 560 72 Td (6) Tj ET"
    ));
    assert!(!md.contains(" 6") && !md.contains("\n6"), "{md}");
}

/// A chart's values sit mid-page, each a line of its own: content, not page numbers.
#[test]
fn a_charts_values_mid_page_are_kept() {
    let md = markdown(&format!(
        "{BODY}BT /F1 7 Tf 100 500 Td (100) Tj ET\nBT /F1 7 Tf 100 450 Td (80) Tj ET\n\
         BT /F1 7 Tf 100 400 Td (60) Tj ET"
    ));
    for value in ["100", "80", "60"] {
        assert!(md.contains(value), "{value} kept:\n{md}");
    }
}

/// The last row of a table near the bottom has more than one number on its baseline.
#[test]
fn a_tables_last_row_near_the_bottom_is_kept() {
    let md = markdown(&format!(
        "{BODY}BT /F1 10 Tf 72 80 Td (12) Tj ET\nBT /F1 10 Tf 200 80 Td (15) Tj ET\n\
         BT /F1 10 Tf 330 80 Td (27) Tj ET"
    ));
    for value in ["12", "15", "27"] {
        assert!(md.contains(value), "{value} kept:\n{md}");
    }
}

#[test]
fn page_numbers_in_the_edge_bands_are_counted_once_without_tables() {
    let page = parse_page(&format!(
        "{BODY}BT /F1 10 Tf 500 770 Td (12) Tj ET\n\
         BT /F1 10 Tf 500 20 Td (- 12 -) Tj ET"
    ));
    assert!(!page.elements.iter().any(|b| matches!(b, Block::Table(_))));
    assert!(page.plain_text().contains("body text"));
    assert!(!page.plain_text().contains("12"));
    assert_eq!(page.omitted_page_number_chars, 6);
}

#[test]
fn an_isolated_folio_within_the_pages_margin_is_counted() {
    let page = parse_page(&format!("{BODY}BT /F1 11 Tf 500 72 Td (362) Tj ET"));
    assert!(!page.plain_text().contains("362"));
    assert_eq!(page.omitted_page_number_chars, 3);
}

#[test]
fn folios_in_both_side_margins_are_counted() {
    let page = parse_page(&format!(
        "{BODY}BT /F1 8 Tf 15 686 Td (46) Tj ET\n\
         BT /F1 8 Tf 570 686 Td (47) Tj ET"
    ));
    let text = page.plain_text();
    assert!(!text.contains("46") && !text.contains("47"), "{text}");
    assert_eq!(page.omitted_page_number_chars, 4);
}

#[test]
fn a_page_number_below_a_ruled_table_is_counted() {
    let page = parse_page(
        "2 w 70 660 m 70 700 l S 170 660 m 170 700 l S 270 660 m 270 700 l S \
         70 700 m 270 700 l S 70 680 m 270 680 l S 70 660 m 270 660 l S \
         BT /F1 12 Tf 80 690 Td (Name) Tj ET \
         BT /F1 12 Tf 180 690 Td (Age) Tj ET \
         BT /F1 12 Tf 80 670 Td (Alice) Tj ET \
         BT /F1 12 Tf 180 670 Td (30) Tj ET \
         BT /F1 10 Tf 500 20 Td (12) Tj ET",
    );
    assert!(page.elements.iter().any(|b| matches!(b, Block::Table(_))));
    let text = page.plain_text();
    assert!(text.contains("30"), "{text}");
    assert!(!text.contains("12"), "{text}");
    assert_eq!(page.omitted_page_number_chars, 2);
}

#[test]
fn a_folio_only_page_stays_empty_when_raw_fallback_runs() {
    let page = parse_page("BT /F1 10 Tf 500 20 Td (362) Tj ET");
    assert!(page.plain_text().is_empty());
    assert_eq!(page.omitted_page_number_chars, 3);
}

#[test]
fn kept_numbers_and_margin_whitespace_are_not_reported_as_omitted() {
    let page = parse_page(&format!(
        "{BODY}BT /F1 10 Tf 100 400 Td (100) Tj ET\n\
         BT /F1 10 Tf 100 20 Td (   ) Tj ET"
    ));
    assert!(page.plain_text().contains("100"));
    assert_eq!(page.omitted_page_number_chars, 0);
}

#[test]
fn decorations_count_as_characters_independent_of_encoding_or_run_boundaries() {
    for (folio, expected) in [
        ("(Page 12) Tj", 6),
        ("<d020313220d0> Tj", 4),
        ("(1) Tj (2) Tj", 2),
    ] {
        let page = parse_page(&format!("{BODY}BT /F1 10 Tf 500 20 Td {folio} ET"));
        assert_eq!(page.omitted_page_number_chars, expected, "{folio}");
        assert!(!page.plain_text().contains("12"));
    }
}

#[test]
fn extracting_the_same_page_again_replaces_its_omission_count() {
    let pdf = page_pdf(
        "",
        LETTER,
        format!("{BODY}BT /F1 10 Tf 500 20 Td (12) Tj ET").as_bytes(),
    );
    let backend = RawBackend::load_bytes(&pdf).unwrap();
    let mut analyzer = LayoutAnalyzer::new(&backend);
    for _ in 0..2 {
        analyzer.extract_page_blocks(1).unwrap();
        assert_eq!(analyzer.page_facts().omitted_page_number_chars, 2);
    }
    let mut spans = analyzer.extract_page_spans(1).unwrap();
    assert_eq!(analyzer.page_facts().omitted_page_number_chars, 0);
    for _ in 0..2 {
        analyzer.filter_spans_for_page(&mut spans, 1);
        assert_eq!(analyzer.page_facts().omitted_page_number_chars, 2);
    }
}

#[test]
fn page_json_omits_zero_defaults_missing_counts_and_preserves_nonzero_counts() {
    let page = parse_page(BODY);
    let value = serde_json::to_value(&page).unwrap();
    assert!(value.get("omitted_page_number_chars").is_none());
    let restored: Page = serde_json::from_value(value).unwrap();
    assert_eq!(restored.omitted_page_number_chars, 0);

    let page = parse_page("BT /F1 10 Tf 500 20 Td (12) Tj ET");
    let value = serde_json::to_value(&page).unwrap();
    assert_eq!(value["omitted_page_number_chars"], 2);
    let restored: Page = serde_json::from_value(value).unwrap();
    assert_eq!(restored.omitted_page_number_chars, 2);
}

#[test]
fn batch_and_streaming_keep_each_pages_omission_count() {
    let content = [
        format!("{BODY}BT /F1 10 Tf 500 20 Td (12) Tj ET"),
        BODY.to_string(),
        "BT /F1 10 Tf 500 20 Td (362) Tj ET".to_string(),
    ];
    let font_id = 3 + 2 * content.len();
    let mut objects = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R 5 0 R 7 0 R]/Count 3>>".to_vec(),
    ];
    for (i, content) in content.iter().enumerate() {
        objects.push(
            format!(
                "<</Type/Page/Parent 2 0 R{LETTER}/Resources<</Font<</F1 {font_id} 0 R>>>>/Contents {} 0 R>>",
                4 + 2 * i
            )
            .into_bytes(),
        );
        objects.push(stream_object(
            &format!("<</Length {}>>", content.len()),
            content.as_bytes(),
        ));
    }
    objects.push(b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec());
    let parser = unpdf::PdfParser::from_bytes(&assemble(objects)).unwrap();
    let document = parser.parse().unwrap();
    let expected = [2, 0, 3];
    let counts: Vec<_> = document
        .pages
        .iter()
        .map(|p| p.omitted_page_number_chars)
        .collect();
    assert_eq!(counts, expected);
    let mut streamed = Vec::new();
    parser
        .for_each_page(unpdf::PageStreamOptions::default(), |event| {
            if let unpdf::ParseEvent::PageParsed(page) = event {
                streamed.push((page.number, page.omitted_page_number_chars));
            }
            std::ops::ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(streamed, [(1, 2), (2, 0), (3, 3)]);
}
