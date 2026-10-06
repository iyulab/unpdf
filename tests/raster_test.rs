//! Page rasterization: what lands where, in what color, and what is reported as not painted.
#![cfg(feature = "raster")]

mod common;

use common::{page_pdf, page_pdf_with_xobjects, stream_object};
use unpdf::parser::raster::{PageRegion, RasterOptions, RasteredPage};
use unpdf::parser::PdfParser;

const A4: &str = "/MediaBox[0 0 595 842]";
/// One pixel per point.
const PT: RasterOptions = RasterOptions {
    dpi: 72.0,
    region: PageRegion::Crop,
};

fn render(pdf: &[u8], options: &RasterOptions) -> RasteredPage {
    PdfParser::from_bytes(pdf)
        .unwrap()
        .render_page(1, options)
        .unwrap()
}

/// The RGB of the pixel at `(x, y)`, counted from the top-left.
fn px(page: &RasteredPage, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * page.width + x) * 4) as usize;
    [page.rgba[i], page.rgba[i + 1], page.rgba[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 6)
}

const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];

#[test]
fn a_filled_rectangle_lands_where_the_page_puts_it() {
    // A red square 100 pt from the left and 100 pt from the bottom.
    let page = render(&page_pdf("", A4, b"1 0 0 rg 100 100 50 50 re f"), &PT);
    assert_eq!((page.width, page.height), (595, 842));
    assert!(near(px(&page, 125, 842 - 125), RED));
    assert!(near(px(&page, 10, 10), WHITE));
    assert!(near(px(&page, 125, 842 - 200), WHITE));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn resolution_scales_the_page() {
    let page = render(&page_pdf("", A4, b""), &RasterOptions { dpi: 144.0, ..PT });
    assert_eq!((page.width, page.height), (1190, 1684));
}

#[test]
fn a_rotated_page_is_painted_as_displayed() {
    // /Rotate 90 turns the page clockwise: its bottom-left corner goes to the top-left.
    let page = render(
        &page_pdf("", &format!("{A4}/Rotate 90"), b"1 0 0 rg 0 0 50 50 re f"),
        &PT,
    );
    assert_eq!((page.width, page.height), (842, 595));
    assert!(near(px(&page, 25, 25), RED), "{:?}", px(&page, 25, 25));
    assert!(near(px(&page, 842 - 25, 595 - 25), WHITE));
}

#[test]
fn a_page_box_off_the_origin_starts_at_its_own_corner() {
    let page = render(
        &page_pdf(
            "",
            "/MediaBox[100 100 695 942]",
            b"1 0 0 rg 100 100 50 50 re f",
        ),
        &PT,
    );
    assert_eq!((page.width, page.height), (595, 842));
    // The box's lower-left corner is the bottom-left pixel.
    assert!(near(px(&page, 25, 842 - 25), RED));
}

#[test]
fn the_clip_bounds_what_is_painted() {
    // Clip to a 100 x 100 square, then fill the whole page.
    let page = render(
        &page_pdf(
            "",
            A4,
            b"q 200 200 100 100 re W n 1 0 0 rg 0 0 595 842 re f Q",
        ),
        &PT,
    );
    assert!(near(px(&page, 250, 842 - 250), RED));
    assert!(near(px(&page, 100, 842 - 100), WHITE));
}

#[test]
fn a_stroke_draws_the_outline_only() {
    let page = render(
        &page_pdf("", A4, b"0 0 1 RG 4 w 100 100 m 300 100 l S"),
        &PT,
    );
    assert!(near(px(&page, 200, 842 - 100), [0, 0, 255]));
    assert!(near(px(&page, 200, 842 - 120), WHITE));
}

#[test]
fn colors_in_each_device_space() {
    let page = render(
        &page_pdf(
            "",
            A4,
            b"0.5 g 0 0 100 100 re f  0 1 0 rg 100 0 100 100 re f  0 0 0 1 k 200 0 100 100 re f",
        ),
        &PT,
    );
    assert!(near(px(&page, 50, 842 - 50), [128, 128, 128]));
    assert!(near(px(&page, 150, 842 - 50), [0, 255, 0]));
    assert!(near(px(&page, 250, 842 - 50), [0, 0, 0]));
}

#[test]
fn a_full_page_image_covers_the_page() {
    // `/Im0` is one gray (0x80) pixel.
    let page = render(&page_pdf("", A4, b"q 595 0 0 842 0 0 cm /Im0 Do Q"), &PT);
    assert!(near(px(&page, 300, 400), [128, 128, 128]));
    assert!(page.gaps.is_empty());
}

#[test]
fn a_jpeg_image_is_decoded() {
    let jpeg = include_bytes!("fixtures/solid_red_4x2.jpg");
    let image = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 4/Height 2/ColorSpace/DeviceRGB\
              /BitsPerComponent 8/Filter/DCTDecode/Length {}>>",
            jpeg.len()
        ),
        jpeg,
    );
    let page = render(
        &page_pdf_with_xobjects(A4, b"q 200 0 0 100 50 50 cm /J Do Q", vec![("J", image)]),
        &PT,
    );
    assert!(
        near(px(&page, 150, 842 - 100), [200, 30, 40]),
        "{:?}",
        px(&page, 150, 742)
    );
    assert!(near(px(&page, 20, 20), WHITE));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_stencil_mask_paints_the_fill_color_where_it_marks() {
    // 2 x 1 stencil: bit 0 (left) marks, bit 1 (right) does not.
    let stencil = stream_object(
        "<</Type/XObject/Subtype/Image/Width 2/Height 1/ImageMask true/Length 1>>",
        &[0b0100_0000],
    );
    let page = render(
        &page_pdf_with_xobjects(
            A4,
            b"0 0 1 rg q 200 0 0 100 100 100 cm /S Do Q",
            vec![("S", stencil)],
        ),
        &PT,
    );
    assert!(
        near(px(&page, 150, 842 - 150), [0, 0, 255]),
        "left half marks"
    );
    assert!(
        near(px(&page, 250, 842 - 150), WHITE),
        "right half does not"
    );
}

#[test]
fn text_is_reported_as_not_painted_except_invisible_text() {
    let page = render(
        &page_pdf(
            "",
            A4,
            b"BT /F1 12 Tf 72 700 Td (Shown text) Tj ET BT 3 Tr /F1 12 Tf 72 680 Td (OCR layer) Tj ET",
        ),
        &PT,
    );
    assert_eq!(page.gaps.text_runs, 1);
}

#[test]
fn the_png_is_the_page() {
    let page = render(&page_pdf("", A4, b"1 0 0 rg 0 0 10 10 re f"), &PT);
    let png = page.to_png();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 595);
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 842);
}

#[test]
fn a_page_out_of_range_is_an_error() {
    let parser = PdfParser::from_bytes(&page_pdf("", A4, b"")).unwrap();
    assert!(matches!(
        parser.render_page(2, &PT),
        Err(unpdf::Error::PageOutOfRange(2, 1))
    ));
}

// ---------------------------------------------------------------------------------------
// Text. The fixture fonts (tests/fixtures/make_square_fonts.py) draw `A` as a filled square
// over 100..900 x 0..800 of a 1000-unit em; `space` has no outline.

const SQUARE_TTF: &[u8] = include_bytes!("fixtures/square.ttf");
const SQUARE_CFF: &[u8] = include_bytes!("fixtures/square.cff");

/// One page drawing `content` with `/F1` = `font` (object 5); `extra` objects follow from 6.
fn font_page(font: &str, extra: Vec<Vec<u8>>, content: &[u8]) -> Vec<u8> {
    let mut objects = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!("<</Type/Page/Parent 2 0 R{A4}/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>")
            .into_bytes(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        font.as_bytes().to_vec(),
    ];
    objects.extend(extra);
    common::assemble(objects)
}

fn font_file(subtype: &str, data: &[u8]) -> Vec<u8> {
    stream_object(&format!("<<{subtype}/Length {}>>", data.len()), data)
}

/// `A` at 100 pt, its square over x 110..190 and y 100..180 of the page.
const SHOW_A: &[u8] = b"BT /F1 100 Tf 100 100 Td (A) Tj ET";

fn assert_square_painted(page: &RasteredPage) {
    assert!(
        near(px(page, 150, 842 - 140), [0, 0, 0]),
        "inside the glyph: {:?}",
        px(page, 150, 702)
    );
    assert!(near(px(page, 105, 842 - 140), WHITE), "left of it");
    assert!(near(px(page, 150, 842 - 190), WHITE), "above it");
    assert_eq!(page.gaps.text_runs, 0, "{:?}", page.gaps);
}

#[test]
fn a_simple_truetype_font_is_drawn() {
    let pdf = font_page(
        "<</Type/Font/Subtype/TrueType/BaseFont/UnpdfSquare/FirstChar 32/LastChar 65\
          /Widths[250 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1000]\
          /Encoding/WinAnsiEncoding/FontDescriptor 6 0 R>>",
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquare/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile2 7 0 R>>"
                .to_vec(),
            font_file("", SQUARE_TTF),
        ],
        SHOW_A,
    );
    assert_square_painted(&render(&pdf, &PT));
}

#[test]
fn a_bare_cff_font_is_drawn() {
    let pdf = font_page(
        "<</Type/Font/Subtype/Type1/BaseFont/UnpdfSquareCFF/FirstChar 65/LastChar 65\
          /Widths[1000]/FontDescriptor 6 0 R>>",
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquareCFF/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile3 7 0 R>>"
                .to_vec(),
            font_file("/Subtype/Type1C", SQUARE_CFF),
        ],
        SHOW_A,
    );
    assert_square_painted(&render(&pdf, &PT));
}

/// A Type 1 program whose `A` is the fixture square, built the way the format stores it:
/// charstrings encrypted with key 4330 inside a private part encrypted with key 55665.
fn square_type1(encoding: &str) -> Vec<u8> {
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
    // hsbw 0 1000 · rmoveto 100 0 · hlineto 800 · vlineto 800 · hlineto -800 · closepath ·
    // endchar — numbers in the charstring encoding.
    let a = [
        139, 250, 124, 13, 239, 139, 21, 249, 180, 6, 249, 180, 7, 253, 180, 6, 9, 14,
    ];
    let notdef = [139, 139, 13, 14];
    let mut private = b"dup /Private 5 dict dup begin /lenIV 4 def /Subrs 0 array\n".to_vec();
    private.extend(b"2 index /CharStrings 2 dict dup begin\n");
    for (name, cs) in [("/.notdef", &notdef[..]), ("/A", &a[..])] {
        let e = encrypt(cs, 4330);
        private.extend(format!("{name} {} RD ", e.len()).as_bytes());
        private.extend(e);
        private.extend(b" ND\n");
    }
    private.extend(b"end end\nmark currentfile closefile\n");
    let mut program = format!(
        "%!FontType1-1.0: UnpdfSquareT1\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n\
         /Encoding {encoding} def\ncurrentfile eexec\n"
    )
    .into_bytes();
    program.extend(encrypt(&private, 55665));
    program
}

fn type1_page(font_encoding: &str, program_encoding: &str, content: &[u8]) -> Vec<u8> {
    let program = square_type1(program_encoding);
    font_page(
        &format!(
            "<</Type/Font/Subtype/Type1/BaseFont/UnpdfSquareT1/FirstChar 65/LastChar 66\
              /Widths[1000 1000]{font_encoding}/FontDescriptor 6 0 R>>"
        ),
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquareT1/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile 7 0 R>>"
                .to_vec(),
            stream_object(
                &format!(
                    "<</Length1 {} /Length2 0 /Length3 0/Length {}>>",
                    program.len(),
                    program.len()
                ),
                &program,
            ),
        ],
        content,
    )
}

#[test]
fn a_type1_font_is_drawn() {
    let pdf = type1_page("/Encoding/WinAnsiEncoding", "StandardEncoding", SHOW_A);
    assert_square_painted(&render(&pdf, &PT));
}

#[test]
fn a_type1_font_without_an_encoding_uses_its_own() {
    // The program's encoding puts `A` at code 66 (`B`); the PDF font names no encoding.
    let pdf = type1_page(
        "",
        "256 array 0 1 255 {1 index exch /.notdef put} for dup 66 /A put readonly",
        b"BT /F1 100 Tf 100 100 Td (B) Tj ET",
    );
    assert_square_painted(&render(&pdf, &PT));
}

#[test]
fn a_type1_glyph_is_found_by_its_differences_name() {
    let pdf = type1_page(
        "/Encoding<</Differences[66/A]>>",
        "StandardEncoding",
        b"BT /F1 100 Tf 100 100 Td (B) Tj ET",
    );
    assert_square_painted(&render(&pdf, &PT));
}

#[test]
fn a_composite_identity_font_is_drawn_by_glyph_index() {
    // Identity-H: the two-byte code is the CID, and CIDToGIDMap /Identity makes it the glyph
    // index — glyph 2 is `A`.
    let pdf = font_page(
        "<</Type/Font/Subtype/Type0/BaseFont/UnpdfSquare/Encoding/Identity-H\
          /DescendantFonts[6 0 R]>>",
        vec![
            b"<</Type/Font/Subtype/CIDFontType2/BaseFont/UnpdfSquare\
               /CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>\
               /FontDescriptor 7 0 R/DW 1000/CIDToGIDMap/Identity>>"
                .to_vec(),
            b"<</Type/FontDescriptor/FontName/UnpdfSquare/Flags 4/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile2 8 0 R>>"
                .to_vec(),
            font_file("", SQUARE_TTF),
        ],
        b"BT /F1 100 Tf 100 100 Td <0002> Tj ET",
    );
    assert_square_painted(&render(&pdf, &PT));
}

/// Glyphs advance by the widths extraction measures with: two `A`s 1000 units wide sit side
/// by side, the second starting 100 pt after the first.
#[test]
fn glyphs_advance_by_their_widths() {
    let pdf = font_page(
        "<</Type/Font/Subtype/TrueType/BaseFont/UnpdfSquare/FirstChar 65/LastChar 65\
          /Widths[1000]/Encoding/WinAnsiEncoding/FontDescriptor 6 0 R>>",
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquare/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile2 7 0 R>>"
                .to_vec(),
            font_file("", SQUARE_TTF),
        ],
        b"BT /F1 100 Tf 100 100 Td (AA) Tj ET",
    );
    let page = render(&pdf, &PT);
    assert!(near(px(&page, 150, 842 - 140), [0, 0, 0]));
    assert!(
        near(px(&page, 200, 842 - 140), WHITE),
        "the gap between the squares"
    );
    assert!(
        near(px(&page, 250, 842 - 140), [0, 0, 0]),
        "the second square"
    );
}

#[test]
fn invisible_text_paints_nothing_and_is_no_gap() {
    let pdf = font_page(
        "<</Type/Font/Subtype/TrueType/BaseFont/UnpdfSquare/FirstChar 65/LastChar 65\
          /Widths[1000]/Encoding/WinAnsiEncoding/FontDescriptor 6 0 R>>",
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquare/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile2 7 0 R>>"
                .to_vec(),
            font_file("", SQUARE_TTF),
        ],
        b"BT 3 Tr /F1 100 Tf 100 100 Td (A) Tj ET",
    );
    let page = render(&pdf, &PT);
    assert!(near(px(&page, 150, 842 - 140), WHITE));
    assert!(page.gaps.is_empty());
}

/// The crop box is what a viewer shows and the default region; the media box is the whole
/// sheet.
#[test]
fn the_crop_box_is_painted_unless_the_media_box_is_asked_for() {
    let pdf = page_pdf(
        "",
        &format!("{A4}/CropBox[100 100 300 400]"),
        b"1 0 0 rg 100 100 20 20 re f",
    );
    let crop = render(&pdf, &PT);
    assert_eq!((crop.width, crop.height), (200, 300));
    assert!(
        near(px(&crop, 10, 300 - 10), RED),
        "the crop box's corner is the raster's"
    );

    let media = render(
        &pdf,
        &RasterOptions {
            region: PageRegion::Media,
            ..PT
        },
    );
    assert_eq!((media.width, media.height), (595, 842));
}
