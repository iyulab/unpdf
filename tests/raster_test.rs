//! Page rasterization: what lands where, in what color, and what is reported as not painted.
#![cfg(feature = "raster")]

mod common;

use common::{page_pdf, page_pdf_with_xobjects, stream_object};
use unpdf::parser::raster::{RasterOptions, RasteredPage};
use unpdf::parser::PdfParser;

const A4: &str = "/MediaBox[0 0 595 842]";
/// One pixel per point.
const PT: RasterOptions = RasterOptions { dpi: 72.0 };

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
    let page = render(&page_pdf("", A4, b""), &RasterOptions { dpi: 144.0 });
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
