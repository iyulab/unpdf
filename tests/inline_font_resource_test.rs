//! Font dictionaries written inline in a `/Resources /Font` dictionary.
//!
//! The `/Font` resource maps names to font dictionaries; ISO 32000-1 does not require
//! those to be indirect objects, so a font may be written as the value of its name. Such a
//! font is read like any other: its text, widths and declared style. The fixtures are
//! written by `tests/fixtures/make_inline_font_fixtures.py`.

use unpdf::parser::backend::{PdfBackend, RawBackend, ResourceScope};
use unpdf::parser::{LayoutAnalyzer, TextSpan};
use unpdf::{PageStreamOptions, ParseEvent, PdfParser};

const TYPE0: &[u8] = include_bytes!("fixtures/inline-font-type0.pdf");
const SIMPLE: &[u8] = include_bytes!("fixtures/inline-font-simple.pdf");
const TO_UNICODE: &[u8] = include_bytes!("fixtures/inline-font-tounicode.pdf");
const FORM: &[u8] = include_bytes!("fixtures/inline-font-form.pdf");
const TWO_PAGES: &[u8] = include_bytes!("fixtures/inline-font-two-pages.pdf");

/// Every line must come back whole, and nothing may have been discarded.
fn assert_reads(pdf: &[u8], lines: &[&str]) {
    let doc = unpdf::parse_bytes(pdf).expect("fixture parses");
    let text = doc.plain_text();
    for line in lines {
        assert!(text.contains(line), "expected {line:?} in {text:?}");
    }
    assert_eq!(
        doc.extraction_quality.suppressed_text_runs, 0,
        "no run may be discarded, got {text:?}"
    );
}

fn spans(pdf: &[u8], page: u32) -> Vec<TextSpan> {
    let backend = RawBackend::load_bytes(pdf).expect("fixture parses");
    LayoutAnalyzer::new(&backend)
        .extract_page_spans(page)
        .expect("spans")
}

fn first_page(backend: &RawBackend) -> ResourceScope {
    ResourceScope::page(backend.pages()[&1])
}

#[test]
fn an_inline_composite_font_under_a_predefined_unicode_cmap() {
    assert_reads(TYPE0, &["한글 문서"]);
}

#[test]
fn an_inline_composite_font_is_measured_by_its_descendant_widths() {
    let backend = RawBackend::load_bytes(TYPE0).expect("fixture parses");
    // `한` takes the default width, the space (CID 1) the 333 its `/W` gives it.
    let advances = backend
        .glyph_advances(first_page(&backend), b"F1", &[0xD5, 0x5C, 0x00, 0x20])
        .expect("an inline composite font's widths are read");
    let widths: Vec<f32> = advances.iter().map(|a| a.width).collect();
    assert_eq!(widths, [1000.0, 333.0]);
}

#[test]
fn inline_simple_fonts_read_through_their_encoding() {
    // WinAnsiEncoding puts curly quotes, the euro sign and the em dash in 0x80-0x9F, where
    // a byte-wise reading finds only control characters.
    assert_reads(
        SIMPLE,
        &["\u{201C}Café\u{201D} costs € 5 — paid", "Heavy words"],
    );
}

#[test]
fn an_inline_font_is_listed_with_the_style_it_declares() {
    let backend = RawBackend::load_bytes(SIMPLE).expect("fixture parses");
    let fonts = backend.page_fonts(first_page(&backend)).expect("fonts");
    let f2 = fonts
        .iter()
        .find(|f| f.name == b"F2")
        .unwrap_or_else(|| panic!("the inline /F2 is listed: {fonts:?}"));
    assert_eq!(f2.base_font, "Plain");
    assert!(f2.bold, "its descriptor declares weight 700");

    let heavy = spans(SIMPLE, 1)
        .into_iter()
        .find(|s| s.text == "Heavy words")
        .expect("the F2 run");
    assert!(heavy.is_bold, "{heavy:?}");
    assert!(heavy.width_measured, "{heavy:?}");
    assert!((heavy.width - 11.0 * 0.6 * 12.0).abs() < 0.01, "{heavy:?}");
}

#[test]
fn an_inline_standard_font_is_measured_by_its_built_in_widths() {
    let backend = RawBackend::load_bytes(SIMPLE).expect("fixture parses");
    let advances = backend
        .glyph_advances(first_page(&backend), b"F1", b"Ca")
        .expect("Helvetica's widths are known");
    let widths: Vec<f32> = advances.iter().map(|a| a.width).collect();
    assert_eq!(widths, [722.0, 556.0]);
}

#[test]
fn an_inline_composite_font_reads_through_its_to_unicode_stream() {
    assert_reads(TO_UNICODE, &["한글 문서"]);
}

#[test]
fn an_inline_font_in_a_form_xobjects_own_resources() {
    assert_reads(FORM, &["한글 문서"]);
}

/// Two pages each name a different inline font `/F1`. What is read from one page's font
/// must not answer for the other's, in either order.
#[test]
fn inline_fonts_of_the_same_name_on_two_pages_stay_apart() {
    assert_reads(TWO_PAGES, &["\u{201C}quoted\u{201D} text", "한글 문서"]);

    let backend = RawBackend::load_bytes(TWO_PAGES).expect("fixture parses");
    let pages = backend.pages();
    let (one, two) = (
        ResourceScope::page(pages[&1]),
        ResourceScope::page(pages[&2]),
    );
    for _ in 0..2 {
        let simple = backend.decode_text(one, b"F1", b"\x93A\x94");
        assert_eq!(simple.text, "\u{201C}A\u{201D}");
        let composite = backend.decode_text(two, b"F1", &[0xD5, 0x5C]);
        assert_eq!(composite.text, "한");

        let simple = backend.glyph_advances(one, b"F1", b"AB").expect("widths");
        assert_eq!(
            simple.iter().map(|a| a.width).collect::<Vec<_>>(),
            [500.0; 2]
        );
        let composite = backend
            .glyph_advances(two, b"F1", &[0xD5, 0x5C])
            .expect("widths");
        assert_eq!(
            composite.iter().map(|a| a.width).collect::<Vec<_>>(),
            [1000.0]
        );
    }
}

/// The page-at-a-time path decodes through the same fonts as the whole-document one.
#[test]
fn the_page_stream_reads_inline_fonts_too() {
    let parser = PdfParser::from_bytes(TWO_PAGES).expect("fixture opens");
    let mut pages = Vec::new();
    let quality = parser
        .for_each_page(PageStreamOptions::default(), |event| {
            if let ParseEvent::PageParsed(page) = event {
                pages.push(page.plain_text());
            }
            std::ops::ControlFlow::Continue(())
        })
        .expect("pages stream");
    assert_eq!(pages.len(), 2);
    assert!(
        pages[0].contains("\u{201C}quoted\u{201D} text"),
        "{pages:?}"
    );
    assert!(pages[1].contains("한글 문서"), "{pages:?}");
    assert_eq!(quality.suppressed_text_runs, 0);
}
