//! A font that declares itself bold or italic is, whatever its name says.
//!
//! Many families name their styles something other than "Bold" and "Italic" — URW's `-Medi`
//! and `-ReguItal`, TeX's `CMBX12` and `CMTI10`. The PDF says so instead: the font
//! descriptor's `/FontWeight`, `/ItalicAngle` and `/Flags` (ISO 32000-1 §9.8.1), or the
//! embedded program's own weight.

mod common;

use unpdf::render::{to_markdown, RenderOptions};
use unpdf::PdfParser;

/// One page: plain body lines around a title line set in `/F2`, whose descriptor carries
/// `descriptor` entries (an upright `/ItalicAngle 0` unless they give one). `/F1` is
/// Helvetica.
fn page(descriptor: &str) -> Vec<u8> {
    let angle = if descriptor.contains("/ItalicAngle") {
        ""
    } else {
        "/ItalicAngle 0"
    };
    let content = b"BT /F1 11 Tf 72 700 Td (Body text before the title line.) Tj ET \
        BT /F2 11 Tf 72 680 Td (Steps for Using the Microscope) Tj ET \
        BT /F1 11 Tf 72 660 Td (Body text after the title line.) Tj ET\n";
    let widths = vec!["500"; 224].join(" ");
    common::assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
          /Resources<</Font<</F1 5 0 R/F2 6 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        common::stream_object(&format!("<</Length {}>>", content.len()), content),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>".to_vec(),
        format!(
            "<</Type/Font/Subtype/Type1/BaseFont/UnpdfSerif-Medi/Encoding/WinAnsiEncoding\
              /FirstChar 32/LastChar 255/Widths[{widths}]/FontDescriptor 7 0 R>>"
        )
        .into_bytes(),
        format!(
            "<</Type/FontDescriptor/FontName/UnpdfSerif-Medi/FontBBox[0 0 1000 800]\
              {angle}/Ascent 800/Descent -200/CapHeight 800/StemV 140{descriptor}>>"
        )
        .into_bytes(),
    ])
}

fn markdown(pdf: &[u8]) -> String {
    let doc = PdfParser::from_bytes(pdf).and_then(|p| p.parse()).unwrap();
    to_markdown(&doc, &RenderOptions::default()).unwrap()
}

#[test]
fn force_bold_makes_the_font_bold() {
    let md = markdown(&page("/Flags 262178"));
    assert!(md.contains("# Steps for Using the Microscope"), "{md}");
}

#[test]
fn a_heavy_font_weight_makes_the_font_bold() {
    let md = markdown(&page("/Flags 32/FontWeight 700"));
    assert!(md.contains("# Steps for Using the Microscope"), "{md}");
}

#[test]
fn a_font_that_declares_nothing_is_read_by_its_name() {
    let md = markdown(&page("/Flags 32"));
    assert!(!md.contains("# Steps for Using the Microscope"), "{md}");
    assert!(!md.contains("*Steps"), "{md}");
}

#[test]
fn a_leaning_italic_angle_makes_the_font_italic() {
    let md = markdown(&page("/Flags 32/ItalicAngle -14.04"));
    assert!(md.contains("*Steps for Using the Microscope*"), "{md}");
}

#[test]
fn the_italic_flag_makes_the_font_italic() {
    let md = markdown(&page("/Flags 96"));
    assert!(md.contains("*Steps for Using the Microscope*"), "{md}");
}

#[test]
fn a_fraction_of_a_degree_is_not_a_lean() {
    let md = markdown(&page("/Flags 32/ItalicAngle -0.5"));
    assert!(!md.contains("*Steps"), "{md}");
}
