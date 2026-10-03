//! Content painted through Form XObjects.
//!
//! A form is a content stream of its own, painted by `Do` with its own `/Resources` and
//! `/Matrix` (ISO 32000-1 §8.10). Whatever it paints is part of the page: its text is page
//! text, its images are page images, and a page that paints only forms is not empty -- nor
//! a scan, unless what the forms paint is an image.

mod common;

use unpdf::parser::ErrorMode;
use unpdf::{parse_bytes, parse_bytes_with_options, ParseOptions};

#[test]
fn text_inside_a_form_is_extracted() {
    let doc = parse_bytes(&common::form_xobject_text_pdf()).unwrap();

    assert!(
        doc.plain_text().contains(common::FORM_XOBJECT_TEXT),
        "got: {:?}",
        doc.plain_text()
    );
    assert!(doc.extraction_quality.char_count > 0);
    assert!(!doc.extraction_quality.is_scan_pdf);
}

#[test]
fn a_form_paint_is_counted_as_a_form_not_as_an_image() {
    let doc = parse_bytes(&common::form_xobject_text_pdf()).unwrap();
    let page = &doc.pages[0];

    assert_eq!(page.form_op_count, 1);
    assert_eq!(page.image_op_count, 0);
    assert_eq!(page.text_op_count, 1, "the form's `Tj` is the page's text");
}

#[test]
fn a_scan_wrapped_in_a_form_is_still_a_scan() {
    let doc = parse_bytes(&common::form_wrapped_scan_pdf()).unwrap();
    let page = &doc.pages[0];

    assert_eq!(page.form_op_count, 1);
    assert_eq!(page.image_op_count, 1, "the image the form paints");
    assert_eq!(page.text_op_count, 0);
    assert!(doc.extraction_quality.is_scan_pdf);
}

#[test]
fn an_image_inside_a_form_is_extracted_as_a_resource() {
    let doc = parse_bytes_with_options(
        &common::form_wrapped_scan_pdf(),
        ParseOptions::default().with_resources(true),
    )
    .unwrap();

    assert_eq!(doc.resources.len(), 1, "got: {:?}", doc.resources.keys());
    assert_eq!(doc.pages[0].images.len(), 1);
}

#[test]
fn names_inside_a_form_resolve_in_the_forms_own_resources() {
    let text = parse_bytes(&common::form_with_shadowing_font_name_pdf())
        .unwrap()
        .plain_text();

    assert!(text.contains("Page text"), "got: {text:?}");
    assert!(
        text.contains("ZZZ"),
        "the form's /F1 has /Differences [65 /Z]; got: {text:?}"
    );
    assert!(
        !text.contains("AAA"),
        "decoded with the page's /F1: {text:?}"
    );
}

#[test]
fn a_forms_matrix_places_its_content_on_the_page() {
    let text = parse_bytes(&common::form_with_matrix_pdf())
        .unwrap()
        .plain_text();

    let middle = text.find("Middle line").expect("page text");
    let bottom = text.find("Bottom line").expect("form text");
    assert!(
        middle < bottom,
        "the form's /Matrix moves its line below the page's; got: {text:?}"
    );
}

#[test]
fn a_nested_form_without_resources_takes_the_pages() {
    let text = parse_bytes(&common::nested_form_pdf())
        .unwrap()
        .plain_text();

    assert!(text.contains("Outer form"), "got: {text:?}");
    assert!(text.contains("Inner form"), "got: {text:?}");
}

#[test]
fn a_form_that_paints_itself_is_drawn_once() {
    let text = parse_bytes(&common::self_painting_form_pdf())
        .unwrap()
        .plain_text();

    assert_eq!(text.matches("Drawn once").count(), 1, "got: {text:?}");
}

#[test]
fn a_form_that_cannot_be_decoded_is_reported_as_lost_content() {
    let doc = parse_bytes_with_options(
        &common::undecodable_form_pdf(),
        ParseOptions::default().with_error_mode(ErrorMode::Lenient),
    )
    .unwrap();

    assert!(
        doc.plain_text().contains("Hello World"),
        "the page's own text is kept"
    );
    assert_eq!(doc.pages[0].undecodable_content_streams, 1);
    assert_eq!(doc.extraction_quality.undecodable_content_streams, 1);
}

#[test]
fn strict_mode_fails_a_page_whose_form_cannot_be_decoded() {
    let result = parse_bytes_with_options(
        &common::undecodable_form_pdf(),
        ParseOptions::default().with_error_mode(ErrorMode::Strict),
    );

    assert!(
        result.is_err(),
        "strict must not present the page as complete"
    );
}
