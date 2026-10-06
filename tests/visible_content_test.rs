//! Only what a page shows is its content.
//!
//! A viewer shows a page through its crop box (ISO 32000-1 §14.11.2) and every mark through
//! the clipping path in force when it is painted (§8.5.4); a form's content is clipped to its
//! `/BBox` (§8.10.1). Text placed outside — the slug area of a print-ready page, the margin
//! of a larger page placed onto a smaller one, a form's content beyond its box — is never
//! seen, and reading it out puts text no one can see into the document.

mod common;

use unpdf::parse_bytes;

fn text_of(pdf: &[u8]) -> String {
    parse_bytes(pdf).unwrap().plain_text()
}

#[test]
fn text_outside_the_crop_box_is_not_content() {
    // Beside the crop box at mid-height — where no running-head filter looks.
    let content = b"BT /F1 12 Tf 72 400 Td (Shown on the page) Tj ET \
        BT /F1 12 Tf 585 400 Td (Printed in the slug) Tj ET\n";
    let text = text_of(&common::page_pdf(
        "",
        "/MediaBox[0 0 612 800]/CropBox[36 36 576 760]",
        content,
    ));
    assert!(text.contains("Shown on the page"), "{text:?}");
    assert!(!text.contains("Printed in the slug"), "{text:?}");
}

#[test]
fn text_a_clipping_path_hides_is_not_content() {
    let content = b"BT /F1 12 Tf 72 700 Td (Outside the clip) Tj ET \
        q 60 400 300 100 re W n \
        BT /F1 12 Tf 72 450 Td (Inside the clip) Tj ET \
        BT /F1 12 Tf 72 600 Td (Clipped away) Tj ET Q \
        BT /F1 12 Tf 72 300 Td (After the restore) Tj ET\n";
    let text = text_of(&common::page_pdf("", "/MediaBox[0 0 612 792]", content));
    for shown in ["Outside the clip", "Inside the clip", "After the restore"] {
        assert!(text.contains(shown), "{shown:?} missing from {text:?}");
    }
    assert!(!text.contains("Clipped away"), "{text:?}");
}

#[test]
fn a_forms_content_beyond_its_bbox_is_not_content() {
    let form = b"BT /F1 12 Tf 10 10 Td (Inside the box) Tj ET \
        BT /F1 12 Tf 10 400 Td (Beyond the box) Tj ET\n";
    let form_obj = common::stream_object(
        &format!(
            "<</Type/XObject/Subtype/Form/BBox[0 0 300 100]\
              /Resources<</Font<</F1 6 0 R>>>>/Length {}>>",
            form.len()
        ),
        form,
    );
    let pdf = common::page_pdf_with_xobjects(
        "/MediaBox[0 0 612 792]",
        b"q 1 0 0 1 72 200 cm /Fm1 Do Q\n",
        vec![("Fm1", form_obj)],
    );
    let text = text_of(&pdf);
    assert!(text.contains("Inside the box"), "{text:?}");
    assert!(!text.contains("Beyond the box"), "{text:?}");
}

#[test]
fn a_larger_page_placed_onto_a_smaller_one_keeps_only_what_shows() {
    // The shape of a print-ready page: an A4 page placed as a form onto a smaller page,
    // clipped to the trim; the A4 page's margin carries a notice and a stamp.
    let a4 = b"BT /F1 10 Tf 72 815 Td (Margin notice) Tj ET \
        BT /F1 12 Tf 72 600 Td (Body of the page) Tj ET \
        BT /F1 8 Tf 72 15 Td (Margin stamp) Tj ET\n";
    let form_obj = common::stream_object(
        &format!(
            "<</Type/XObject/Subtype/Form/BBox[0 0 595 842]\
              /Resources<</Font<</F1 6 0 R>>>>/Length {}>>",
            a4.len()
        ),
        a4,
    );
    let pdf = common::page_pdf_with_xobjects(
        "/MediaBox[0 0 560 785]/CropBox[28 28 532 757]",
        b"q 20 20 520 745 re W n q 1 0 0 1 -18 -28 cm /Fm1 Do Q Q\n",
        vec![("Fm1", form_obj)],
    );
    let text = text_of(&pdf);
    assert!(text.contains("Body of the page"), "{text:?}");
    assert!(!text.contains("Margin notice"), "{text:?}");
    assert!(!text.contains("Margin stamp"), "{text:?}");
}
