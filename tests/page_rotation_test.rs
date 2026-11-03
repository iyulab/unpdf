//! A page turned by `/Rotate` is read as it is shown.
//!
//! The viewer applies `/Rotate` to everything drawn on a page (ISO 32000-1 §7.7.3.3), so a table
//! set across a landscape page shows its rows left to right however the producer wrote the
//! content stream. A reader that stays in the unturned space sees the table on its side: the
//! rules run the wrong way and every column is a line of text.

mod common;

use unpdf::render::{to_markdown, RenderOptions};
use unpdf::PdfParser;

const W: f32 = 612.0;
const H: f32 = 792.0;

/// Where a point of the page as shown lies in the unturned page, and the text matrix whose
/// baseline runs to the right as shown.
fn unturned(rotation: u16, x: f32, y: f32) -> ((f32, f32), [f32; 4]) {
    match rotation {
        90 => ((W - y, x), [0.0, 1.0, -1.0, 0.0]),
        180 => ((W - x, H - y), [-1.0, 0.0, 0.0, -1.0]),
        270 => ((y, H - x), [0.0, -1.0, 1.0, 0.0]),
        _ => ((x, y), [1.0, 0.0, 0.0, 1.0]),
    }
}

/// A booktabs table — a rule above, one under the header, one under the last row — drawn so
/// that it reads upright on the page as shown after `/Rotate` is applied.
fn turned_table(rotation: u16) -> Vec<u8> {
    let mut content = String::new();
    // Rules as shown: from x 90 to 400 at y 512, 492 and 448.
    for y in [512.0, 492.0, 448.0] {
        let ((x0, y0), _) = unturned(rotation, 90.0, y);
        let ((x1, y1), _) = unturned(rotation, 400.0, y);
        content.push_str(&format!("0.5 w {x0} {y0} m {x1} {y1} l S\n"));
    }
    let cells = [
        (496.0, "Name", "Value"),
        (478.0, "alpha", "10"),
        (462.0, "beta", "20"),
    ];
    for (y, left, right) in cells {
        for (x, text) in [(100.0, left), (300.0, right)] {
            let ((ex, ey), m) = unturned(rotation, x, y);
            content.push_str(&format!(
                "BT /F1 10 Tf {} {} {} {} {ex} {ey} Tm ({text}) Tj ET\n",
                m[0], m[1], m[2], m[3]
            ));
        }
    }
    common::assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Rotate {rotation}\
             /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
        )
        .into_bytes(),
        common::stream_object(
            &format!("<</Length {}>>", content.len()),
            content.as_bytes(),
        ),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
    ])
}

fn markdown(pdf: &[u8]) -> String {
    let doc = PdfParser::from_bytes(pdf).unwrap().parse().unwrap();
    to_markdown(&doc, &RenderOptions::default()).unwrap()
}

#[test]
fn a_table_on_a_page_turned_a_quarter_or_half_is_read_as_shown() {
    for rotation in [0, 90, 180, 270] {
        let md = markdown(&turned_table(rotation));
        let lines: Vec<&str> = md.lines().filter(|l| l.starts_with('|')).collect();
        assert_eq!(
            lines,
            [
                "| Name | Value |",
                "| --- | --- |",
                "| alpha | 10 |",
                "| beta | 20 |"
            ],
            "/Rotate {rotation}: {md}"
        );
    }
}
