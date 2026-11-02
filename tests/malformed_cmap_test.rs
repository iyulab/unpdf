//! A font whose `ToUnicode` CMap or embedded TrueType `cmap` is malformed maps nothing for the
//! damaged entries; the page is still read, with the text its other fonts draw.

mod common;

use common::{assemble, stream_object};

fn page_with_damaged_font(font: &[u8], font_objects: Vec<Vec<u8>>) -> Vec<u8> {
    let content = b"BT /F1 12 Tf 72 720 Td (Kept) Tj /F2 12 Tf 0 -20 Td <0001> Tj ET";
    let mut objects = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
          /Resources<</Font<</F1 5 0 R/F2 6 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
        font.to_vec(),
    ];
    objects.extend(font_objects);
    assemble(objects)
}

#[test]
fn a_to_unicode_destination_that_is_not_utf8_leaves_the_page_text() {
    let cmap = b"1 beginbfchar <01> <000\xFF> endbfchar";
    let pdf = page_with_damaged_font(
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/ToUnicode 7 0 R>>",
        vec![stream_object(&format!("<</Length {}>>", cmap.len()), cmap)],
    );
    let text = unpdf::parse_bytes(&pdf).unwrap().plain_text();
    assert!(text.contains("Kept"), "{text:?}");
}

#[test]
fn an_embedded_truetype_cmap_cut_short_of_its_declared_length_leaves_the_page_text() {
    let mut program = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    program.extend(b"cmap");
    program.extend([0; 4]);
    program.extend(28u32.to_be_bytes());
    program.extend(1000u32.to_be_bytes());
    program.extend([0, 0, 0, 1]);
    let pdf = page_with_damaged_font(
        b"<</Type/Font/Subtype/Type0/BaseFont/Damaged/Encoding/Identity-H/DescendantFonts[7 0 R]>>",
        vec![
            b"<</Type/Font/Subtype/CIDFontType2/BaseFont/Damaged\
              /CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>\
              /FontDescriptor 8 0 R>>"
                .to_vec(),
            b"<</Type/FontDescriptor/FontName/Damaged/Flags 4/FontFile2 9 0 R>>".to_vec(),
            stream_object(&format!("<</Length {}>>", program.len()), &program),
        ],
    );
    let text = unpdf::parse_bytes(&pdf).unwrap().plain_text();
    assert!(text.contains("Kept"), "{text:?}");
}
