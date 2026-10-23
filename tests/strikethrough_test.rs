//! A rule drawn through a run's text, the way a strike-out is set, strikes the run.

mod common;

use unpdf::render::{to_markdown, RenderOptions};
use unpdf::PdfParser;

fn width(text: &str) -> f32 {
    text.chars().map(common::helvetica_advance).sum::<f32>() * 12.0 / 1000.0
}

/// Two lines of Helvetica 12: the first with a rule through `struck` alone, the second with
/// a rule that runs on well past both ends of its text, as a chart's line through a label.
fn struck_and_crossed_pdf() -> Vec<u8> {
    let start = 72.0 + width("keep this ");
    let end = start + width("struck");
    let content = format!(
        "BT /F1 12 Tf 72 700 Td (keep this ) Tj (struck) Tj ( word) Tj ET \
         0.6 w {start} 703.5 m {end} 703.5 l S \
         BT /F1 12 Tf 72 670 Td (a label on a line) Tj ET \
         0.6 w 40 673.5 m 400 673.5 l S\n"
    );
    common::page_pdf("", "/MediaBox[0 0 595 842]", content.as_bytes())
}

fn markdown_of(pdf: &[u8]) -> String {
    let doc = PdfParser::from_bytes(pdf)
        .and_then(|p| p.parse())
        .expect("synthetic PDF should parse");
    to_markdown(&doc, &RenderOptions::default()).expect("markdown renders")
}

fn markdown() -> String {
    markdown_of(&struck_and_crossed_pdf())
}

/// One line of Helvetica 12 with a rule from the start of its second run to `rule_end`.
fn struck_second_run_pdf(runs: [&str; 3], rule_end: f32) -> String {
    let start = 72.0 + width(runs[0]);
    let end = start + rule_end;
    let [first, second, third] = runs;
    let content = format!(
        "BT /F1 12 Tf 72 700 Td ({first}) Tj ({second}) Tj ({third}) Tj ET \
         0.6 w {start} 703.5 m {end} 703.5 l S\n"
    );
    markdown_of(&common::page_pdf(
        "",
        "/MediaBox[0 0 595 842]",
        content.as_bytes(),
    ))
}

#[test]
fn a_rule_through_a_run_strikes_it() {
    let md = markdown();
    assert!(md.contains("keep this ~~struck~~ word"), "{md:?}");
}

#[test]
fn a_rule_running_past_the_text_strikes_nothing() {
    let md = markdown();
    assert!(md.contains("a label on a line"), "{md:?}");
    assert_eq!(md.matches("~~").count(), 2, "{md:?}");
    assert!(md.contains("~~struck~~"), "{md:?}");
}

#[test]
fn a_rule_ending_where_the_next_run_starts_strikes_only_its_run() {
    for noise in [-0.01, 0.0, 0.01] {
        let md = struck_second_run_pdf(["keep ", "struck", "next"], width("struck") + noise);
        assert!(md.contains("keep ~~struck~~next"), "{noise}: {md:?}");
    }
}

#[test]
fn a_rule_over_a_run_and_its_trailing_space_strikes_the_run() {
    let md = struck_second_run_pdf(["keep this ", "struck ", "word"], width("struck "));
    assert!(md.contains("~~struck"), "{md:?}");
    assert!(!md.contains("~~word") && !md.contains("word~~"), "{md:?}");
}
