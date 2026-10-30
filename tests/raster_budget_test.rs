//! Painting a page charges a budget the work of each step before it is taken, and stops when it
//! breaks.
#![cfg(feature = "raster")]

mod common;

use std::ops::ControlFlow;

use common::{assemble, stream_object};
use unpdf::parser::raster::{PageRegion, RasterOptions, RasteredPage};
use unpdf::parser::PdfParser;
use unpdf::{Error, Result};

/// One pixel per point.
const PT: RasterOptions = RasterOptions {
    dpi: 72.0,
    region: PageRegion::Crop,
};
/// The pixels of a [`square_page`] at [`PT`].
const PAGE: u64 = 100 * 100;
/// What each operator charges for being applied.
const OP: u64 = 1;

/// A one-page PDF of `media_box` painting `content` with `resources`, `extra` as objects 5 on.
fn one_page(media_box: &str, resources: &str, content: &[u8], extra: Vec<Vec<u8>>) -> Vec<u8> {
    let mut objects = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R/MediaBox{media_box}/Resources<<{resources}>>\
             /Contents 4 0 R>>"
        )
        .into_bytes(),
        stream(content),
    ];
    objects.extend(extra);
    assemble(objects)
}

/// A 100 pt square page.
fn square_page(resources: &str, content: &[u8], extra: Vec<Vec<u8>>) -> Vec<u8> {
    one_page("[0 0 100 100]", resources, content, extra)
}

fn stream(content: &[u8]) -> Vec<u8> {
    stream_object(&format!("<</Length {}>>", content.len()), content)
}

fn form(entries: &str, content: &[u8]) -> Vec<u8> {
    stream_object(
        &format!(
            "<</Type/XObject/Subtype/Form/BBox[0 0 100 100]{entries}/Length {}>>",
            content.len()
        ),
        content,
    )
}

/// A Type 3 font whose 1000-unit glyph space is a text space unit, `a` (code 97) drawn by the
/// procedure in object 6.
fn type3_font() -> Vec<u8> {
    b"<</Type/Font/Subtype/Type3/FontBBox[0 0 1000 1000]/FontMatrix[0.001 0 0 0.001 0 0]\
      /CharProcs<</a 6 0 R>>/Encoding<</Type/Encoding/Differences[97/a]>>/FirstChar 97\
      /LastChar 97/Widths[1000]/Resources<<>>>>"
        .to_vec()
}

/// What a `w` by `h` pixel rectangle's edges add to a step that rasterizes it on a pixmap of
/// one tile: three sides walked between its four points, and each point once.
fn rect_edges(w: u64, h: u64) -> u64 {
    (w + 1) + (h + 1) + (w + 1) + 4
}

/// Paint page 1 of `pdf`, charging `budget`.
fn paint(
    pdf: &[u8],
    options: &RasterOptions,
    budget: &mut dyn FnMut(u64) -> ControlFlow<()>,
) -> Result<RasteredPage> {
    PdfParser::from_bytes(pdf)
        .unwrap()
        .render_page_with(1, options, budget)
}

/// What painting page 1 of `pdf` charges, step by step, under a budget that never stops.
fn charges(pdf: &[u8], options: &RasterOptions) -> Vec<u64> {
    let mut charged = Vec::new();
    paint(pdf, options, &mut |work| {
        charged.push(work);
        ControlFlow::Continue(())
    })
    .unwrap();
    charged
}

/// Paint page 1 of `pdf` under a budget that breaks on its `n`th charge: the outcome, and how
/// many charges were made.
fn stopped_at(pdf: &[u8], n: usize) -> (Result<RasteredPage>, usize) {
    let mut calls = 0;
    let painted = paint(pdf, &PT, &mut |_| {
        calls += 1;
        if calls < n {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    });
    (painted, calls)
}

/// A page of each kind of painting step, and what painting it charges: a fill its pixels on the
/// page and [`rect_edges`], a stroke its widened bounds, its edges and 2 x points x (width +
/// tiles), and a dashed stroke first its dashes x (2 x width + 4) x tiles.
fn painting_kinds() -> Vec<(&'static str, Vec<u8>, Vec<u64>)> {
    let fill = b"0 0 1 rg 10 10 20 30 re f";
    let filled = 20 * 30 + rect_edges(20, 30);
    let image = |entries: &str| {
        stream_object(
            &format!(
                "<</Type/XObject/Subtype/Image/Width 2/Height 3/ColorSpace/DeviceGray\
                  /BitsPerComponent 8{entries}/Length 6>>"
            ),
            &[0x80; 6],
        )
    };
    let jbig2_data: &[u8] = include_bytes!("fixtures/halves_white_black_16x8.jb2");
    let jbig2 = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceGray\
              /BitsPerComponent 1/Filter/JBIG2Decode/Length {}>>",
            jbig2_data.len()
        ),
        jbig2_data,
    );
    let mut inline = b"q 20 0 0 10 30 40 cm BI /W 2 /H 3 /CS /G /BPC 8 ID ".to_vec();
    inline.extend_from_slice(&[0x80; 6]);
    inline.extend_from_slice(b"\nEI Q");
    let shading = "/Shading<</Sh<</ShadingType 2/ColorSpace/DeviceRGB/Coords[0 0 100 0]\
        /Function<</FunctionType 2/Domain[0 1]/C0[1 0 0]/C1[0 0 1]/N 1>>/BBox[10 10 30 40]>>>>";
    let pattern = stream_object(
        "<</Type/Pattern/PatternType 1/PaintType 1/TilingType 1/BBox[0 0 10 10]/XStep 10\
          /YStep 10/Resources<<>>/Length 23>>",
        b"1 0 0 rg 0 0 10 10 re f",
    );
    let glyph = b"1000 0 0 0 1000 1000 d1 0 0 1000 1000 re f";
    let subpaths = [b"0 w ".as_slice(), &b"10 50 m 90 50 l ".repeat(20), b"S"].concat();
    let past_device = [b"1000000 0 0 1000000 0 0 cm ".repeat(7), fill.to_vec()].concat();
    vec![
        (
            "fill",
            square_page("", fill, vec![]),
            vec![PAGE, OP, OP, OP, filled],
        ),
        (
            "stroke",
            square_page("", b"2 M 10 w 20 50 m 80 50 l S", vec![]),
            vec![
                PAGE,
                OP,
                OP,
                OP,
                OP,
                OP,
                82 * 22 + (60 + 1) + 2 + 2 * 2 * (10 + 1),
            ],
        ),
        (
            "stroke of many subpaths",
            square_page("", &subpaths, vec![]),
            [
                vec![PAGE],
                vec![OP; 42],
                vec![82 * 2 + 39 * (80 + 1) + 40 + 40 * 2 * (1 + 1)],
            ]
            .concat(),
        ),
        (
            "stroke in a stretched space",
            square_page("", b"1 0 0 10 0 0 cm 1 w 10 5 m 90 5 l S", vec![]),
            vec![
                PAGE,
                OP,
                OP,
                OP,
                OP,
                OP,
                92 * 100 + (80 + 1) + 2 + 2 * 2 * (10 + 1),
            ],
        ),
        (
            "text-showing array",
            square_page("", b"BT /F9 12 Tf [(ab) 0 ()] TJ ET", vec![]),
            vec![PAGE, OP, OP, OP, 2, 1, 1, OP],
        ),
        (
            "fill past what device space holds",
            square_page("", &past_device, vec![]),
            [vec![PAGE], vec![OP; 10], vec![PAGE + 3 + 4]].concat(),
        ),
        (
            "fill wholly off the page",
            square_page("", b"0 0 1 rg 200 200 10 10 re f", vec![]),
            vec![PAGE, OP, OP, OP, 3 + 4],
        ),
        (
            "dashed stroke on a pixmap of two tiles",
            one_page(
                "[0 0 8200 1]",
                "",
                b"[1 2 4] 0 d 0 0.5 m 100 0.5 l S",
                vec![],
            ),
            vec![
                8200,
                OP,
                3,
                OP,
                OP,
                OP,
                2 * 3,
                (22 + 3 * 2) * (2 + 4) * 2,
                106 + (100 + 1) + 2 * 2 + 2 * 2 * (1 + 2),
            ],
        ),
        (
            "image",
            square_page(
                "/XObject<</Im 5 0 R>>",
                b"q 20 0 0 10 30 40 cm /Im Do Q",
                vec![image("")],
            ),
            vec![PAGE, OP, OP, OP, 20 * 10 + rect_edges(20, 10) + 6 + 6, OP],
        ),
        (
            "image with a soft mask",
            square_page(
                "/XObject<</Im 5 0 R>>",
                b"q 20 0 0 10 30 40 cm /Im Do Q",
                vec![image("/SMask 6 0 R"), image("")],
            ),
            vec![
                PAGE,
                OP,
                OP,
                OP,
                20 * 10 + rect_edges(20, 10) + 2 * (6 + 6),
                OP,
            ],
        ),
        (
            "jbig2 image of a larger page",
            square_page(
                "/XObject<</Im 5 0 R>>",
                b"q 20 0 0 10 30 40 cm /Im Do Q",
                vec![jbig2],
            ),
            vec![
                PAGE,
                OP,
                OP,
                OP,
                20 * 10 + rect_edges(20, 10) + 16 * 8 + jbig2_data.len() as u64,
                OP,
            ],
        ),
        (
            "inline image",
            square_page("", &inline, vec![]),
            vec![PAGE, OP, OP, OP, 20 * 10 + rect_edges(20, 10) + 6 + 6, OP],
        ),
        (
            "shading",
            square_page(shading, b"/Sh sh", vec![]),
            vec![PAGE, OP, filled],
        ),
        (
            "form",
            square_page("/XObject<</Fm 5 0 R>>", b"/Fm Do", vec![form("", fill)]),
            [
                vec![PAGE, OP, OP, OP, OP, PAGE + rect_edges(100, 100)],
                vec![OP, OP, OP, filled, OP],
            ]
            .concat(),
        ),
        (
            "tiling pattern cell",
            square_page(
                "/Pattern<</P0 5 0 R>>",
                b"/Pattern cs /P0 scn 0 0 50 50 re f",
                vec![pattern],
            ),
            vec![
                PAGE,
                OP,
                OP,
                10 * 10,
                10 * 10 + rect_edges(10, 10),
                OP,
                OP,
                OP,
                10 * 10 + rect_edges(10, 10),
                OP,
                OP,
                50 * 50 + rect_edges(50, 50),
            ],
        ),
        (
            "type 3 glyph",
            square_page(
                "/Font<</T3 5 0 R>>",
                b"BT /T3 10 Tf 10 10 Td (a) Tj ET",
                vec![type3_font(), stream(glyph)],
            ),
            vec![
                PAGE,
                OP,
                OP,
                OP,
                OP,
                1,
                OP,
                OP,
                OP,
                10 * 10 + rect_edges(10, 10),
                OP,
            ],
        ),
        (
            "clip",
            square_page("", b"10 10 20 20 re W n", vec![]),
            vec![PAGE, OP, OP, OP, PAGE + rect_edges(20, 20)],
        ),
        (
            "transparency group",
            square_page(
                "/ExtGState<</Half<</ca 0.5>>>>/XObject<</Fm 5 0 R>>",
                b"/Half gs /Fm Do",
                vec![form("/Group<</S/Transparency>>", fill)],
            ),
            [
                vec![PAGE, OP, OP, OP, OP, OP, PAGE + rect_edges(100, 100)],
                vec![OP, PAGE, OP, OP, OP, filled, OP, PAGE, OP],
            ]
            .concat(),
        ),
        (
            "soft mask",
            square_page(
                "/ExtGState<</Mk<</SMask<</Type/Mask/S/Luminosity/G 5 0 R>>>>>>",
                &[b"/Mk gs ".as_slice(), fill].concat(),
                vec![form("/Group<</S/Transparency>>", b"1 g 10 10 20 30 re f")],
            ),
            [
                vec![
                    PAGE,
                    OP,
                    PAGE,
                    PAGE + rect_edges(100, 100),
                    OP,
                    OP,
                    OP,
                    filled,
                ],
                vec![PAGE, OP, OP, OP, filled],
            ]
            .concat(),
        ),
    ]
}

#[test]
fn each_kind_of_painting_charges_the_work_it_does() {
    for (kind, pdf, expected) in painting_kinds() {
        assert_eq!(charges(&pdf, &PT), expected, "{kind}");
    }
}

#[test]
fn painting_stops_at_the_charge_that_breaks_and_gives_no_image() {
    for (kind, pdf, expected) in painting_kinds() {
        for n in 1..=expected.len() {
            let (painted, calls) = stopped_at(&pdf, n);
            assert!(
                matches!(painted, Err(Error::RasterStopped)),
                "{kind} {n}: {painted:?}"
            );
            assert_eq!(calls, n, "{kind}");
        }
    }
}

#[test]
fn a_type_3_glyph_that_shows_itself_charges_every_operator_it_runs() {
    let pdf = square_page(
        "/Font<</T3 5 0 R>>",
        b"BT /T3 10 Tf (aa) Tj ET",
        vec![type3_font(), stream(b"0 0 d0 BT /T3 1 Tf (aa) Tj ET")],
    );
    let procedures = 2 + 4 + 8 + 16;
    let charged = charges(&pdf, &PT);
    let count = |work: u64| charged.iter().filter(|&&c| c == work).count();
    assert_eq!((count(OP), count(2)), (4 + procedures * 5, 1 + procedures));
    assert_eq!(charged.len(), 1 + (4 + procedures * 5) + (1 + procedures));
    let (painted, calls) = stopped_at(&pdf, 10);
    assert!(matches!(painted, Err(Error::RasterStopped)), "{painted:?}");
    assert_eq!(calls, 10);
}

#[cfg(feature = "standard-fonts")]
#[test]
fn a_glyph_charges_its_outline_filled_stroked_or_kept_for_the_clip() {
    let run = |mode: &str| format!("BT /F1 400 Tf -40 -40 Td {mode} (H) Tj ET ");
    let content = [
        run(""),
        run("1 Tr"),
        "10 w ".into(),
        run("1 Tr"),
        run("7 Tr"),
    ]
    .concat();
    let pdf = square_page(
        "/Font<</F1<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>>>",
        content.as_bytes(),
        vec![],
    );
    let charged = charges(&pdf, &PT);
    let large: Vec<u64> = charged.iter().copied().filter(|&c| c > 1).collect();
    let [PAGE, fill, thin, wide, kept, clip] = large[..] else {
        panic!("{charged:?}");
    };
    assert!(fill > PAGE && clip > PAGE, "{charged:?}");
    assert_eq!((thin - fill, wide - fill), (2 * 2 * kept, 2 * 11 * kept));
}

/// A 1450 pt square page that fills itself blue 20 000 times.
fn full_page_fills() -> Vec<u8> {
    let fills = b"0 0 1 rg 0 0 1450 1450 re f\n".repeat(20_000);
    one_page("[0 0 1450 1450]", "", &fills, vec![])
}

#[test]
fn twenty_thousand_full_page_fills_charge_twenty_thousand_pages() {
    let one_dpi = RasterOptions { dpi: 1.0, ..PT };
    let charged = charges(&full_page_fills(), &one_dpi);
    assert_eq!(charged.len(), 1 + 20_000 * 4);
    let pages = charged.iter().filter(|&&c| c >= 20 * 20).count();
    assert_eq!(pages, 1 + 20_000);
}

#[test]
fn a_budget_of_three_pages_stops_the_fills_at_the_second() {
    let (page, mut spent, mut calls) = (1450 * 1450, 0, 0);
    let painted = paint(&full_page_fills(), &PT, &mut |work| {
        calls += 1;
        spent += work;
        if spent > 3 * page {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    assert!(matches!(painted, Err(Error::RasterStopped)), "{painted:?}");
    assert_eq!(calls, 1 + 4 + 4);
}

#[test]
fn checking_a_missing_glyph_through_the_clip_is_charged_and_cancellable() {
    let pdf = square_page(
        "/Font<</T3 5 0 R>>",
        b"0 0 5 5 re W n BT /T3 20 Tf 10 10 Td (b) Tj ET",
        vec![type3_font(), stream(b"1000 0 0 0 1000 1000 d1")],
    );
    let work = charges(&pdf, &PT);
    let scan = work.len() - 2;
    assert_eq!(work[scan], 22 * 22);
    let (result, calls) = stopped_at(&pdf, scan + 1);
    assert!(matches!(result, Err(Error::RasterStopped)), "{result:?}");
    assert_eq!(calls, scan + 1);
    assert_eq!(
        paint(&pdf, &PT, &mut |_| ControlFlow::Continue(()))
            .unwrap()
            .gaps
            .text_runs,
        0
    );
}
