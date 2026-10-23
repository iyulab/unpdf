//! Text positions advance by the glyph widths the font declares.
//!
//! Showing text moves the text position by each glyph's advance (ISO 32000-1
//! §9.4.4). A producer that draws one glyph per operator places each glyph at the
//! previous one's advance, so the font's `/Widths` are the only correct measure of
//! where one glyph ends and the next begins — a per-character guess is off by a
//! fraction of an em for wide and narrow glyphs, which is enough to split a word.

mod common;

use unpdf::render::{to_text, RenderOptions};
use unpdf::PdfParser;

fn text_of(pdf: &[u8]) -> String {
    let doc = PdfParser::from_bytes(pdf)
        .and_then(|p| p.parse())
        .expect("synthetic PDF should parse");
    to_text(&doc, &RenderOptions::default()).expect("text renders")
}

#[test]
fn a_word_drawn_one_glyph_at_a_time_stays_whole() {
    // `m` (0.889 em) is far wider than an average glyph: a guessed advance leaves a
    // gap after it that reads as a word break.
    let text = text_of(&common::glyph_per_operator_pdf(
        "carbon concentrating mechanisms",
        14.0,
        true,
    ));
    assert!(
        text.contains("carbon concentrating mechanisms"),
        "the line must read as three whole words, got {text:?}"
    );
}

#[test]
fn a_standard_font_without_declared_widths_is_measured_by_its_own_metrics() {
    // The standard 14 fonts may omit `/Widths` (ISO 32000-1 §9.6.2.2); their advances
    // are the fonts' published metrics. Without them the line falls back to a guess and
    // splits after the wide `m`.
    let text = text_of(&common::glyph_per_operator_pdf_without_widths(
        "carbon concentrating mechanisms",
        14.0,
    ));
    assert!(
        text.contains("carbon concentrating mechanisms"),
        "the line must read as three whole words, got {text:?}"
    );
}

#[test]
fn a_tj_that_opens_with_an_offset_starts_where_its_first_glyph_is_drawn() {
    // The right-aligned run's text origin is exactly where the left run ends; only the
    // TJ's leading offset puts `october` near the right margin. Taking the origin as the
    // run's start glues the two together.
    let text = text_of(&common::tj_offset_row_pdf());
    assert!(
        text.contains("university of somewhere october"),
        "the right-aligned run must be separated from the left one, got {text:?}"
    );
}

#[test]
fn a_word_gap_drawn_as_a_move_still_separates_words() {
    // No space glyph: the gap between words exists only as a longer `Td`. Joining
    // glyphs that abut must not also join glyphs a word space apart.
    let text = text_of(&common::glyph_per_operator_pdf(
        "carbon concentrating mechanisms",
        14.0,
        false,
    ));
    assert!(
        text.contains("carbon concentrating mechanisms"),
        "the word gaps must survive, got {text:?}"
    );
}

#[test]
fn a_glyph_drawn_into_a_gap_left_for_it_reads_in_the_gap() {
    let gap = 72.0 + "the ".chars().map(common::helvetica_advance).sum::<f32>() * 12.0 / 1000.0;
    let content =
        format!("BT /F1 12 Tf 72 700 Td [(the ) -500 ( sign)] TJ 1 0 0 1 {gap} 700 Tm (x) Tj ET\n");
    let text = text_of(&common::page_pdf(
        "",
        "/MediaBox[0 0 595 842]",
        content.as_bytes(),
    ));
    assert!(
        text.contains("the x sign"),
        "the glyph must read where it is drawn, got {text:?}"
    );
}

#[test]
fn a_glyph_drawn_into_a_gap_between_letters_joins_them() {
    let gap = 72.0 + "the".chars().map(common::helvetica_advance).sum::<f32>() * 12.0 / 1000.0;
    let content =
        format!("BT /F1 12 Tf 72 700 Td [(the) -500 (sign)] TJ 1 0 0 1 {gap} 700 Tm (x) Tj ET\n");
    let text = text_of(&common::page_pdf(
        "",
        "/MediaBox[0 0 595 842]",
        content.as_bytes(),
    ));
    assert!(
        text.contains("thexsign"),
        "the blank was the glyph's advance, not a word gap, got {text:?}"
    );
}

#[test]
fn a_gap_left_of_its_run_by_negative_character_spacing_is_not_split_at() {
    let text = text_of(&common::page_pdf(
        "",
        "/MediaBox[0 0 595 842]",
        b"BT /F1 12 Tf -20 Tc 300 700 Td [(ab) -500 (cd)] TJ 0 Tc 1 0 0 1 274 700 Tm (x) Tj ET\n",
    ));
    for part in ["ab", "cd", "x"] {
        assert!(text.contains(part), "{part:?} must survive, got {text:?}");
    }
}

#[test]
fn a_smaller_glyph_set_below_the_baseline_does_not_fill_its_gap() {
    let gap = 72.0 + "the ".chars().map(common::helvetica_advance).sum::<f32>() * 12.0 / 1000.0;
    let content = format!(
        "BT /F1 12 Tf 72 700 Td [(the ) -500 ( sign)] TJ /F1 8.4 Tf 1 0 0 1 {gap} 695.2 Tm (x) Tj ET\n"
    );
    let text = text_of(&common::page_pdf(
        "",
        "/MediaBox[0 0 595 842]",
        content.as_bytes(),
    ));
    assert!(
        text.contains("sign") && !text.contains("the x sign"),
        "only a glyph on the run's baseline fills its blank, got {text:?}"
    );
}
