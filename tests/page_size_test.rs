//! A page's displayed size, read before painting: the region a raster covers, its `/Rotate`
//! applied, and the pixels a raster of it has.
#![cfg(feature = "raster")]

mod common;

use common::page_pdf;
use unpdf::parser::raster::{PageRegion, PageSize, RasterOptions};
use unpdf::parser::PdfParser;
use unpdf::Error;

const LETTER: &str = "/MediaBox[0 0 612 792]";
const A4: &str = "/MediaBox[0 0 595 842]";

fn parser(pages_entries: &str, page_entries: &str) -> PdfParser {
    PdfParser::from_bytes(&page_pdf(pages_entries, page_entries, b"")).unwrap()
}

fn size(page_entries: &str, region: PageRegion) -> (f32, f32) {
    let size = parser("", page_entries).page_size(1, region).unwrap();
    (size.width, size.height)
}

fn painted(page_entries: &str, region: PageRegion, dpi: f32) -> (u32, u32) {
    let page = parser("", page_entries)
        .render_page(1, &RasterOptions { dpi, region })
        .unwrap();
    (page.width, page.height)
}

fn dpi_for_long_edge(page_entries: &str, region: PageRegion, edge: u32) -> f32 {
    let size = parser("", page_entries).page_size(1, region).unwrap();
    72.0 * edge as f32 / size.width.max(size.height)
}

#[test]
fn a_page_without_a_crop_box_is_the_size_of_its_media_box() {
    assert_eq!(size(LETTER, PageRegion::Crop), (612.0, 792.0));
    assert_eq!(size(LETTER, PageRegion::Media), (612.0, 792.0));
}

#[test]
fn the_crop_box_is_the_size_unless_the_media_box_is_asked_for() {
    let entries = format!("{A4}/CropBox[100 100 300 400]");
    assert_eq!(size(&entries, PageRegion::Crop), (200.0, 300.0));
    assert_eq!(size(&entries, PageRegion::Media), (595.0, 842.0));
}

#[test]
fn a_crop_box_is_clipped_to_the_media_box_and_ignored_outside_it() {
    let overhanging = format!("{A4}/CropBox[-50 -50 300 400]");
    assert_eq!(size(&overhanging, PageRegion::Crop), (300.0, 400.0));
    let apart = format!("{A4}/CropBox[1000 1000 1100 1100]");
    assert_eq!(size(&apart, PageRegion::Crop), (595.0, 842.0));
}

#[test]
fn a_quarter_turn_swaps_the_sides_and_a_half_turn_does_not() {
    let boxed = format!("{A4}/CropBox[100 100 300 400]");
    for (rotate, expected) in [
        (0, (200.0, 300.0)),
        (90, (300.0, 200.0)),
        (180, (200.0, 300.0)),
        (270, (300.0, 200.0)),
    ] {
        let entries = format!("{boxed}/Rotate {rotate}");
        assert_eq!(
            size(&entries, PageRegion::Crop),
            expected,
            "/Rotate {rotate}"
        );
    }
    assert_eq!(
        size(&format!("{A4}/Rotate 90"), PageRegion::Media),
        (842.0, 595.0)
    );
}

#[test]
fn boxes_and_rotation_inherited_from_the_page_tree_count() {
    let parser = parser("/MediaBox[0 0 400 200]/CropBox[0 0 300 200]/Rotate 90", "");
    let size = parser.page_size(1, PageRegion::Crop).unwrap();
    assert_eq!((size.width, size.height), (200.0, 300.0));
}

#[test]
fn a_page_the_document_does_not_have_is_out_of_range() {
    let parser = parser("", LETTER);
    for page in [0, 2] {
        assert!(
            matches!(
                parser.page_size(page, PageRegion::Crop),
                Err(Error::PageOutOfRange(n, 1)) if n == page
            ),
            "page {page}"
        );
    }
}

#[test]
fn a_box_too_large_for_a_finite_size_is_a_render_error() {
    let huge = format!("3{}.0", "0".repeat(38));
    for entries in [
        format!("/MediaBox[0 0 1{}.0 792]", "0".repeat(39)),
        format!("/MediaBox[-{huge} 0 {huge} 792]"),
    ] {
        let parser = parser("", &entries);
        for region in [PageRegion::Crop, PageRegion::Media] {
            assert!(
                matches!(parser.page_size(1, region), Err(Error::Render(_))),
                "{entries} {region:?}"
            );
        }
    }
}

#[test]
fn pixels_are_each_side_rounded_with_one_as_the_least() {
    let size = PageSize {
        width: 612.0,
        height: 792.0,
    };
    assert_eq!(size.pixels(72.0), (612, 792));
    assert_eq!(size.pixels(144.0), (1224, 1584));
    let small = PageSize {
        width: 3.0,
        height: 6.0,
    };
    assert_eq!(small.pixels(130.0), (5, 11));
    assert_eq!(small.pixels(134.0), (6, 11));
    let halves = PageSize {
        width: 5.0,
        height: 9.0,
    };
    assert_eq!(halves.pixels(36.0), (3, 5));
    let sliver = PageSize {
        width: 14400.0,
        height: 1.0,
    };
    assert_eq!(sliver.pixels(10.24), (2048, 1));
}

#[test]
fn the_pixels_of_a_size_are_the_pixels_painted() {
    let pages = [
        (LETTER.to_string(), PageRegion::Crop),
        (A4.to_string(), PageRegion::Media),
        (
            format!("{A4}/CropBox[100 100 300.5 400.25]"),
            PageRegion::Crop,
        ),
        (
            format!("{A4}/CropBox[100 100 300.5 400.25]/Rotate 90"),
            PageRegion::Crop,
        ),
        (
            format!("{A4}/CropBox[10.1 20.3 333.3 444.7]/Rotate 270"),
            PageRegion::Crop,
        ),
        ("/MediaBox[0 0 14400 1]".to_string(), PageRegion::Crop),
        (
            "/MediaBox[0 0 1 14400]/Rotate 90".to_string(),
            PageRegion::Crop,
        ),
    ];
    for (entries, region) in &pages {
        let displayed = parser("", entries).page_size(1, *region).unwrap();
        for dpi in [50.0, 72.0, 93.0, 110.5, 150.0, 211.7, 300.0] {
            assert_eq!(
                displayed.pixels(dpi),
                painted(entries, *region, dpi),
                "{entries} {region:?} at {dpi} dpi"
            );
        }
    }
}

#[test]
fn pixels_scale_by_dpi_over_72_in_f32_as_painting_does() {
    let entries = "/MediaBox[0 0 396 612]";
    let displayed = parser("", entries).page_size(1, PageRegion::Crop).unwrap();
    assert_eq!(painted(entries, PageRegion::Crop, 93.0), (511, 791));
    assert_eq!(displayed.pixels(93.0), (511, 791));
}

#[test]
fn a_dpi_from_the_size_is_a_candidate_pixels_checks() {
    let entries = "/MediaBox[0 0 842 0.00001]";
    let displayed = parser("", entries).page_size(1, PageRegion::Crop).unwrap();
    let dpi = dpi_for_long_edge(entries, PageRegion::Crop, 3_066_127);
    assert_eq!(displayed.pixels(dpi), (3_066_128, 1));
    assert_eq!(painted(entries, PageRegion::Crop, dpi), (3_066_128, 1));
}

#[test]
fn a_dpi_from_the_size_of_a_common_page_paints_the_long_edge_asked_for() {
    for (entries, region) in [
        (LETTER.to_string(), PageRegion::Crop),
        (A4.to_string(), PageRegion::Crop),
        (format!("{A4}/Rotate 90"), PageRegion::Crop),
        (format!("{A4}/CropBox[100 100 300 400]"), PageRegion::Crop),
        (
            format!("{A4}/CropBox[100 100 300 400]/Rotate 270"),
            PageRegion::Crop,
        ),
        (format!("{A4}/CropBox[100 100 300 400]"), PageRegion::Media),
    ] {
        for edge in [256, 777, 1000, 1024, 2048, 4096] {
            let dpi = dpi_for_long_edge(&entries, region, edge);
            let (width, height) = painted(&entries, region, dpi);
            assert_eq!(width.max(height), edge, "{entries} {region:?} at {edge}");
        }
    }
}

#[test]
fn a_letter_page_at_a_long_edge_of_1024_is_791_by_1024() {
    let dpi = dpi_for_long_edge(LETTER, PageRegion::Crop, 1024);
    assert_eq!(painted(LETTER, PageRegion::Crop, dpi), (791, 1024));
}

#[test]
fn a_sliver_page_keeps_one_pixel_on_its_short_side() {
    let entries = "/MediaBox[0 0 14400 1]";
    let dpi = dpi_for_long_edge(entries, PageRegion::Crop, 2048);
    assert_eq!(painted(entries, PageRegion::Crop, dpi), (2048, 1));
}
