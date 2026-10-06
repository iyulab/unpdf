//! Text in a font that embeds its program reads through the program's own encoding.
//!
//! ISO 32000-1 §9.6.6.1: a font with no `/Encoding` uses its program's built-in encoding,
//! and an encoding dictionary with no `/BaseEncoding` applies its `/Differences` to that
//! built-in encoding when the program is embedded. TeX's fonts are the common case: they
//! put ligatures, quotes and dashes at codes no Latin encoding has there, so guessing
//! Latin-1 drops the ligatures (control codes) and turns quotes and dashes into other
//! punctuation.

mod common;

use unpdf::parse_bytes;

/// A Type 1 program whose built-in encoding is `entries` (code, glyph name). Only the
/// cleartext part matters for reading text; the private part holds a `.notdef`.
fn type1_program(entries: &[(u8, &str)]) -> Vec<u8> {
    fn encrypt(plain: &[u8], key: u16) -> Vec<u8> {
        let mut r = key;
        [0u8; 4]
            .iter()
            .chain(plain)
            .map(|&p| {
                let c = p ^ (r >> 8) as u8;
                r = u16::from(c)
                    .wrapping_add(r)
                    .wrapping_mul(52845)
                    .wrapping_add(22719);
                c
            })
            .collect()
    }
    let mut encoding = String::from("256 array 0 1 255 {1 index exch /.notdef put} for\n");
    for (code, name) in entries {
        encoding.push_str(&format!("dup {code} /{name} put\n"));
    }
    let mut program = format!(
        "%!PS-AdobeFont-1.0: UnpdfTeXLike\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n\
         /Encoding {encoding}readonly def\ncurrentfile eexec\n"
    )
    .into_bytes();
    let notdef = encrypt(&[139, 139, 13, 14], 4330);
    let mut private = b"dup /Private 3 dict dup begin /lenIV 4 def\n".to_vec();
    private.extend(b"2 index /CharStrings 1 dict dup begin\n");
    private.extend(format!("/.notdef {} RD ", notdef.len()).as_bytes());
    private.extend(notdef);
    private.extend(b" ND\nend end\n");
    program.extend(encrypt(&private, 55665));
    program
}

/// One page showing `text` (raw codes) in a Type 1 font embedding `program`, whose font
/// dictionary carries `encoding` (an `/Encoding` entry, or nothing).
fn page(program: &[u8], encoding: &str, text: &[u8]) -> Vec<u8> {
    let mut content = b"BT /F1 12 Tf 72 700 Td (".to_vec();
    for &b in text {
        if matches!(b, b'(' | b')' | b'\\') || b < 0x20 {
            content.extend(format!("\\{b:03o}").as_bytes());
        } else {
            content.push(b);
        }
    }
    content.extend(b") Tj ET\n");
    let objects = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Resources<</Font<</F1 5 0 R>>>>\
          /Contents 4 0 R>>"
            .to_vec(),
        common::stream_object(&format!("<</Length {}>>", content.len()), &content),
        format!(
            "<</Type/Font/Subtype/Type1/BaseFont/UnpdfTeXLike/FirstChar 0/LastChar 255\
              /Widths[{}]{encoding}/FontDescriptor 6 0 R>>",
            vec!["500"; 256].join(" ")
        )
        .into_bytes(),
        b"<</Type/FontDescriptor/FontName/UnpdfTeXLike/Flags 32/FontBBox[0 0 1000 800]\
          /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile 7 0 R>>"
            .to_vec(),
        common::stream_object(
            &format!(
                "<</Length1 {} /Length2 0 /Length3 0/Length {}>>",
                program.len(),
                program.len()
            ),
            program,
        ),
    ];
    common::assemble(objects)
}

/// The OT1-like layout TeX's text fonts use, as far as these tests need it.
const TEX_LIKE: &[(u8, &str)] = &[
    (11, "ff"),
    (12, "fi"),
    (14, "ffi"),
    (92, "quotedblleft"),
    (34, "quotedblright"),
    (123, "endash"),
    (b'c', "c"),
    (b'o', "o"),
    (b'e', "e"),
    (b'n', "n"),
    (b't', "t"),
    (b'd', "d"),
    (b'r', "r"),
    (b's', "s"),
    (b'i', "i"),
    (b'1', "one"),
    (b'2', "two"),
];

fn text_of(pdf: &[u8]) -> String {
    parse_bytes(pdf).unwrap().plain_text()
}

#[test]
fn a_font_with_no_encoding_reads_through_its_programs_encoding() {
    // "\x5Ccoe\x0Ecient\x22 di\x0Berent 1\x7B2"
    let text = b"\x5Ccoe\x0Ecient\x22 di\x0Berent 1\x7B2";
    let out = text_of(&page(&type1_program(TEX_LIKE), "", text));
    assert!(
        out.contains("coe\u{FB03}cient") || out.contains("coefficient"),
        "{out:?}"
    );
    assert!(
        out.contains("di\u{FB00}erent") || out.contains("different"),
        "{out:?}"
    );
    assert!(
        out.contains('\u{201C}') && out.contains('\u{201D}'),
        "{out:?}"
    );
    assert!(out.contains("1\u{2013}2"), "{out:?}");
}

#[test]
fn differences_without_a_base_apply_to_the_programs_encoding() {
    // The dictionary renames one code; every other code keeps the program's meaning — not
    // StandardEncoding's, where 92 is a backslash and 123 a brace.
    let text = b"\x5Cd\x7B";
    let out = text_of(&page(
        &type1_program(TEX_LIKE),
        "/Encoding<</Type/Encoding/Differences[100/e]>>",
        text,
    ));
    assert!(out.contains("\u{201C}e\u{2013}"), "{out:?}");
}
