//! A glyph named by a bare decimal number stands for the character of that value.
//!
//! No standard names such a glyph, but producers that convert bitmap fonts name each glyph
//! after the character's code (`/72` for `H`), and MuPDF reads exactly that
//! (`fz_unicode_from_glyph_name`: a name that is only decimal digits is the code point).
//! Without a `ToUnicode` map such a page came out as the codes themselves — control
//! characters and punctuation — instead of its text.

mod common;

use unpdf::parse_bytes;

/// One page showing `codes` in a Type 3 font whose `/Differences` array names code 0, 1, …
/// with `names` (given without the slash).
fn type3_page(names: &[&str], codes: &[u8]) -> Vec<u8> {
    let differences: String = names.iter().map(|n| format!("/{n} ")).collect();
    let char_procs: String = (0..names.len())
        .map(|i| format!("/{} {} 0 R ", names[i], 6 + i))
        .collect();
    let mut shown = String::new();
    for &c in codes {
        shown.push_str(&format!("\\{c:03o}"));
    }
    let content = format!("BT /F1 12 Tf 72 700 Td ({shown}) Tj ET\n").into_bytes();
    let widths = vec!["600"; names.len()].join(" ");
    let mut objects = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Resources<</Font<</F1 5 0 R>>>>\
          /Contents 4 0 R>>"
            .to_vec(),
        common::stream_object(&format!("<</Length {}>>", content.len()), &content),
        format!(
            "<</Type/Font/Subtype/Type3/FontBBox[0 0 1000 1000]/FontMatrix[0.001 0 0 0.001 0 0]\
             /CharProcs<<{char_procs}>>/Encoding<</Type/Encoding/Differences[0 {differences}]>>\
             /FirstChar 0/LastChar {}/Widths[{widths}]>>",
            names.len() - 1
        )
        .into_bytes(),
    ];
    let glyph = b"600 0 d0 100 100 400 400 re f";
    for _ in names {
        objects.push(common::stream_object(
            &format!("<</Length {}>>", glyph.len()),
            glyph,
        ));
    }
    common::assemble(objects)
}

fn text_of(pdf: &[u8]) -> String {
    parse_bytes(pdf).unwrap().plain_text()
}

#[test]
fn a_glyph_named_by_a_decimal_number_is_that_character() {
    // 72 = H, 105 = i, 33 = !
    let out = text_of(&type3_page(&["72", "105", "33"], &[0, 1, 2]));
    assert!(out.contains("Hi!"), "{out:?}");
}

#[test]
fn the_number_follows_the_names_own_order_not_the_codes() {
    // The code of a glyph is its place in /Differences; the character is the name's value.
    let out = text_of(&type3_page(&["119", "111", "119"], &[0, 1, 2]));
    assert!(out.contains("wow"), "{out:?}");
}

#[test]
fn a_number_that_is_a_control_code_is_not_a_character() {
    // 7 is BEL: the name names no printable character, so it never reaches the text.
    let out = text_of(&type3_page(&["72", "7", "105"], &[0, 1, 2]));
    assert!(out.contains('H') && out.contains('i'), "{out:?}");
    assert!(!out.contains('\u{7}'), "{out:?}");
}
