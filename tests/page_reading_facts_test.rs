//! What a page record says about how well the page was read: where images cover it, the
//! page box and rotation, rotated, invisible, clipped, margin and blank text, path fills,
//! ruled grids, and the reading order's regions.

mod common;

use common::{bordered_table_pdf, page_pdf, page_pdf_with_xobjects, stream_object};
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

fn image_xobject(entries: &str) -> Vec<u8> {
    stream_object(
        &format!("<</Type/XObject/Subtype/Image/Width 8/Height 8{entries}/Length 1>>"),
        &[0x80],
    )
}

fn page_with_image(entries: &str) -> unpdf::model::Page {
    only_page(&page_pdf_with_xobjects(
        A4,
        b"q 595 0 0 842 0 0 cm /Im Do Q",
        vec![("Im", image_xobject(entries))],
    ))
}

#[test]
fn stencils_one_bit_samples_fax_and_jbig2_images_are_bilevel() {
    for (kind, entries) in [
        ("one bit", "/ColorSpace/DeviceGray/BitsPerComponent 1"),
        ("stencil", "/ImageMask true"),
        ("fax", "/Filter/CCITTFaxDecode"),
        ("jbig2 after flate", "/Filter[/FlateDecode/JBIG2Decode]"),
    ] {
        let page = page_with_image(entries);
        assert!(close(page.image_coverage, 1.0), "{kind}");
        assert!(
            close(page.bilevel_image_coverage, 1.0),
            "{kind}: {}",
            page.bilevel_image_coverage
        );
    }
}

#[test]
fn colour_and_grey_images_are_not_bilevel() {
    for (kind, entries) in [
        ("grey", "/ColorSpace/DeviceGray/BitsPerComponent 8"),
        (
            "jpeg",
            "/ColorSpace/DeviceRGB/BitsPerComponent 8/Filter/DCTDecode",
        ),
        ("no stencil", "/ImageMask false/BitsPerComponent 8"),
    ] {
        let page = page_with_image(entries);
        assert!(close(page.image_coverage, 1.0), "{kind}");
        assert_eq!(page.bilevel_image_coverage, 0.0, "{kind}");
    }
}

#[test]
fn a_painted_xobject_that_is_not_an_image_is_not_bilevel() {
    let postscript = stream_object("<</Type/XObject/Subtype/PS/ImageMask true/Length 1>>", &[0]);
    let page = only_page(&page_pdf_with_xobjects(
        A4,
        b"q 595 0 0 842 0 0 cm /Ps Do Q",
        vec![("Ps", postscript)],
    ));
    assert_eq!(page.image_op_count, 1);
    assert_eq!(page.bilevel_image_coverage, 0.0);
}

fn stencil_and_grey() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("Bw", image_xobject("/ImageMask true")),
        (
            "Gr",
            image_xobject("/ColorSpace/DeviceGray/BitsPerComponent 8"),
        ),
    ]
}

#[test]
fn bilevel_coverage_is_the_union_of_the_bilevel_paints_clipped_to_the_page() {
    let top_half_twice_and_grey_bottom = only_page(&page_pdf_with_xobjects(
        A4,
        b"q 595 0 0 421 0 421 cm /Bw Do Q q 595 0 0 421 0 421 cm /Bw Do Q \
          q 595 0 0 421 0 0 cm /Gr Do Q",
        stencil_and_grey(),
    ));
    assert!(close(top_half_twice_and_grey_bottom.image_coverage, 1.0));
    assert!(
        close(top_half_twice_and_grey_bottom.bilevel_image_coverage, 0.5),
        "{}",
        top_half_twice_and_grey_bottom.bilevel_image_coverage
    );

    let off_edge = only_page(&page_pdf_with_xobjects(
        A4,
        b"q 595 0 0 421 300 0 cm /Bw Do Q q 595 0 0 421 0 421 cm /Bw Do Q",
        stencil_and_grey(),
    ));
    assert!(
        close(off_edge.bilevel_image_coverage, 890.0 / 1190.0),
        "{}",
        off_edge.bilevel_image_coverage
    );

    let clipped = only_page(&page_pdf_with_xobjects(
        A4,
        b"0 0 100 100 re W n q 595 0 0 842 0 0 cm /Bw Do Q",
        stencil_and_grey(),
    ));
    assert!(
        close(clipped.bilevel_image_coverage, 10_000.0 / (595.0 * 842.0)),
        "{}",
        clipped.bilevel_image_coverage
    );
}

#[test]
fn bilevel_coverage_stays_within_image_coverage_when_many_paints_are_sampled() {
    let mut content = b"q 300.9 0 0 421 0 0 cm /Bw Do Q".to_vec();
    content.extend(b" q 0.5 0 0 842 594.5 0 cm /Gr Do Q".repeat(299));
    let page = only_page(&page_pdf_with_xobjects(A4, &content, stencil_and_grey()));
    assert_eq!(page.image_op_count, 300);
    assert!(
        page.bilevel_image_coverage > 0.25 && page.bilevel_image_coverage <= page.image_coverage,
        "{} of {}",
        page.bilevel_image_coverage,
        page.image_coverage
    );
}

#[test]
fn a_jpx_image_is_not_bilevel_by_its_dictionary_s_bit_depth() {
    let jp2 = include_bytes!("fixtures/halves_red_blue_8x4.jp2");
    let bilevel: Vec<f32> = [
        "/JPXDecode",
        "[/FlateDecode/JPXDecode]",
        "6 0 R",
        "[/FlateDecode 6 0 R]",
    ]
    .iter()
    .map(|filter| {
        let image = stream_object(
            &format!(
                "<</Type/XObject/Subtype/Image/Width 8/Height 4/BitsPerComponent 1\
                  /Filter {filter}/Length {}>>",
                jp2.len()
            ),
            jp2,
        );
        let page = only_page(&page_pdf_with_xobjects(
            A4,
            b"q 595 0 0 842 0 0 cm /Im Do Q",
            vec![("Im", image), ("Jpx", b"/JPXDecode".to_vec())],
        ));
        assert!(close(page.image_coverage, 1.0), "{filter}");
        page.bilevel_image_coverage
    })
    .collect();
    assert_eq!(bilevel, [0.0; 4]);
}

#[test]
fn a_bilevel_entry_given_by_reference_is_resolved() {
    for entries in [
        "/Subtype 6 0 R/ColorSpace/DeviceGray/BitsPerComponent 1",
        "/Subtype/Image/ImageMask 7 0 R",
        "/Subtype/Image/ColorSpace/DeviceGray/BitsPerComponent 8 0 R",
    ] {
        let image = stream_object(
            &format!("<</Type/XObject{entries}/Width 8/Height 8/Length 1>>"),
            &[0x80],
        );
        let page = only_page(&page_pdf_with_xobjects(
            A4,
            b"q 595 0 0 842 0 0 cm /Im Do Q",
            vec![
                ("Im", image),
                ("Image", b"/Image".to_vec()),
                ("Yes", b"true".to_vec()),
                ("One", b"1".to_vec()),
            ],
        ));
        assert!(close(page.image_coverage, 1.0), "{entries}");
        assert!(
            close(page.bilevel_image_coverage, 1.0),
            "{entries}: {}",
            page.bilevel_image_coverage
        );
    }
}

#[test]
fn an_inline_image_is_bilevel_by_its_own_dictionary() {
    for (dictionary, bilevel) in [
        ("/IM true", true),
        ("/IM true %comment\n", true),
        ("/CS /G /BPC 1", true),
        ("/F /CCF", true),
        ("/CS /G /BPC 8", false),
        ("/IM false /CS /G /BPC 8", false),
    ] {
        let content = format!("q 595 0 0 842 0 0 cm BI /W 8 /H 8 {dictionary} ID \x01 EI Q");
        let page = only_page(&page_pdf("", A4, content.as_bytes()));
        assert_eq!(page.image_op_count, 1, "{dictionary}");
        assert!(close(page.image_coverage, 1.0), "{dictionary}");
        let expected = if bilevel { 1.0 } else { 0.0 };
        assert!(
            close(page.bilevel_image_coverage, expected),
            "{dictionary}: {}",
            page.bilevel_image_coverage
        );
    }
}

#[test]
fn an_image_inside_a_form_is_named_in_the_form_s_own_resources() {
    let form_content = b"q 595 0 0 421 0 0 cm /Im0 Do Q";
    let form = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Form/BBox[0 0 595 421]\
              /Resources<</XObject<</Im0 7 0 R>>>>/Length {}>>",
            form_content.len()
        ),
        form_content,
    );
    let page = only_page(&page_pdf_with_xobjects(
        A4,
        b"q 595 0 0 421 0 0 cm /Im0 Do Q q 1 0 0 1 0 421 cm /Fm1 Do Q",
        vec![
            (
                "Im0",
                image_xobject("/ColorSpace/DeviceGray/BitsPerComponent 8"),
            ),
            ("Fm1", form),
            ("Stencil", image_xobject("/ImageMask true")),
        ],
    ));
    assert!(close(page.image_coverage, 1.0), "{}", page.image_coverage);
    assert!(
        close(page.bilevel_image_coverage, 0.5),
        "{}",
        page.bilevel_image_coverage
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
fn text_that_paints_nothing_is_counted_and_kept() {
    let page = only_page(&page_pdf(
        "",
        A4,
        b"BT /F1 12 Tf 72 700 Td (Painted on the page.) Tj ET \
          BT 3 Tr /F1 12 Tf 72 650 Td (Neither filled nor stroked.) Tj ET \
          BT 7 Tr /F1 12 Tf 72 600 Td (Only added to the clip.) Tj ET \
          BT 3 Tr /F1 12 Tf 72 550 Td (   ) Tj ET",
    ));
    assert_eq!(page.painted_text_chars, 17);
    assert_eq!(page.invisible_text_chars, 24 + 19);
    assert_eq!(page.clipped_text_runs, 0);
    let text = page.plain_text();
    for line in [
        "Painted on the page.",
        "Neither filled nor stroked.",
        "Only added to the clip.",
    ] {
        assert!(text.contains(line), "{line:?} missing from {text:?}");
    }
}

#[test]
fn clipping_text_shows_once_something_is_painted_inside_its_clip() {
    // Mode 7 adds the glyphs to the clip at `ET`; a fill inside that clip shows
    // through them (ISO 32000-1 §9.3.6). Restoring the state first ends the clip.
    let page = only_page(&page_pdf(
        "",
        A4,
        b"q BT 7 Tr /F1 24 Tf 72 600 Td (Seen through) Tj ET 0 0 595 842 re f Q \
          q BT 7 Tr /F1 24 Tf 72 500 Td (Never shown) Tj ET Q 0 0 595 842 re f \
          q BT 7 Tr /F1 24 Tf 72 400 Td (Outer) Tj ET q 0 0 595 842 re f Q Q",
    ));
    assert_eq!(page.painted_text_chars, 11 + 5);
    assert_eq!(page.invisible_text_chars, 10);
}

#[test]
fn text_the_clip_hides_is_counted_and_left_out() {
    let page = only_page(&page_pdf(
        "",
        A4,
        b"q 60 400 300 100 re W n \
          BT /F1 12 Tf 72 450 Td (Inside the clip) Tj ET \
          BT /F1 12 Tf 72 600 Td (Clipped away) Tj ( ) Tj ET Q \
          BT /F1 12 Tf 72 900 Td (Above the page) Tj ET",
    ));
    assert_eq!(page.clipped_text_runs, 2);
    assert_eq!(page.painted_text_chars, 13);
    assert_eq!(page.invisible_text_chars, 0);
    let text = page.plain_text();
    assert!(text.contains("Inside the clip"), "{text:?}");
    assert!(!text.contains("Clipped away"), "{text:?}");
    assert!(!text.contains("Above the page"), "{text:?}");
}

#[test]
fn painted_text_in_the_top_and_bottom_tenth_is_margin_text() {
    let page = only_page(&page_pdf(
        "",
        A4,
        b"BT /F1 10 Tf 72 800 Td (Running head) Tj ET \
          BT /F1 12 Tf 72 400 Td (Body text in the middle.) Tj ET \
          BT /F1 10 Tf 290 40 Td (12) Tj ET \
          BT 3 Tr /F1 12 Tf 72 60 Td (Invisible footer) Tj ET",
    ));
    assert_eq!(page.margin_text_chars, 11 + 2);
    assert_eq!(page.painted_text_chars, 11 + 20 + 2);
    assert_eq!(page.invisible_text_chars, 15);

    let off_origin = only_page(&page_pdf(
        "",
        "/MediaBox[100 100 695 942]",
        b"BT /F1 12 Tf 160 500 Td (Body text) Tj ET \
          BT /F1 10 Tf 390 110 Td (12) Tj ET",
    ));
    assert_eq!(off_origin.margin_text_chars, 2);
}

#[test]
fn on_a_page_turned_a_quarter_the_margins_are_the_box_s_left_and_right_edges() {
    // Shown turned 90 degrees, the box's left edge is the top of the page and its right edge
    // the bottom: a folio set at x 560 sits at the foot, and text low in the box at x 300 is in
    // the middle of what a reader sees.
    let content = b"BT /F1 10 Tf 560 400 Td (12) Tj ET \
          BT /F1 12 Tf 300 40 Td (Body) Tj ET";
    let turned = only_page(&page_pdf("", &format!("{A4}/Rotate 90"), content));
    assert_eq!(turned.margin_text_chars, 2);
    let upright = only_page(&page_pdf("", A4, content));
    assert_eq!(upright.margin_text_chars, 4);
}

#[test]
fn runs_of_only_whitespace_are_blank() {
    let page = only_page(&page_pdf(
        "",
        A4,
        b"BT /F1 12 Tf 72 700 Td (   ) Tj ( ) Tj ET BT /F1 12 Tf 72 650 Td (Word) Tj ET",
    ));
    assert_eq!(page.text_op_count, 3);
    assert_eq!(page.blank_text_runs, 2);
    assert_eq!(page.painted_text_chars, 4);
}

#[test]
fn path_fills_are_counted() {
    let page = only_page(&page_pdf(
        "",
        A4,
        b"0 0 10 10 re f 0 0 10 10 re F 0 0 10 10 re f* 0 0 10 10 re B \
          0 0 10 10 re B* 0 0 10 10 re b 0 0 10 10 re b* \
          0 0 m 10 10 l S 0 0 10 10 re n",
    ));
    assert_eq!(page.fill_op_count, 7);
    assert_eq!(page.text_op_count, 0);
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
