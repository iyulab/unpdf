//! Arrays nested far deeper than any real PDF are malformed values: reading them never overflows
//! the stack, which would abort the process, and the page keeps the rest of its content.

mod common;

use common::{page_pdf, page_pdf_with_xobjects, stream_object};

const A4: &str = "/MediaBox[0 0 595 842]";

fn nested_arrays(depth: usize) -> String {
    format!("{}{}", "[".repeat(depth), "]".repeat(depth))
}

fn on_small_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(work)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn an_image_dictionary_nested_too_deeply_leaves_the_page_text() {
    let image = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceGray\
             /BitsPerComponent 8/Length 1/Unused {}>>",
            nested_arrays(1000)
        ),
        &[0x80],
    );
    let pdf = page_pdf_with_xobjects(
        A4,
        b"q 10 0 0 10 0 0 cm /Im0 Do Q BT /F1 12 Tf 72 720 Td (Survives) Tj ET",
        vec![("Im0", image)],
    );
    let text = on_small_stack(move || unpdf::parse_bytes(&pdf).unwrap().plain_text());
    assert!(text.contains("Survives"), "{text:?}");
}

#[test]
fn a_content_operand_nested_too_deeply_leaves_the_page_text() {
    let content = format!(
        "BT /F1 12 Tf 72 720 Td {} TJ (Survives) Tj ET",
        nested_arrays(3000)
    );
    let pdf = page_pdf("", A4, content.as_bytes());
    let text = on_small_stack(move || unpdf::parse_bytes(&pdf).unwrap().plain_text());
    assert!(text.contains("Survives"), "{text:?}");
}

#[cfg(feature = "raster")]
#[test]
fn an_inline_image_dictionary_nested_too_deeply_leaves_the_rest_of_the_page() {
    use unpdf::parser::raster::{PageRegion, RasterOptions};
    let mut content =
        format!("BI /W 1 /H 1 /CS /G /BPC 8 /X {} ID ", nested_arrays(1000)).into_bytes();
    content.extend_from_slice(b"\x80 EI 0 1 0 rg 400 400 50 50 re f");
    let pdf = page_pdf("", A4, &content);
    let options = RasterOptions {
        dpi: 72.0,
        region: PageRegion::Crop,
    };
    let page = on_small_stack(move || {
        unpdf::parser::PdfParser::from_bytes(&pdf)
            .unwrap()
            .render_page(1, &options)
            .unwrap()
    });
    assert_eq!(page.gaps.inline_images, 1, "{:?}", page.gaps);
    let green = ((842 - 425) * page.width + 425) as usize * 4;
    assert_eq!(page.rgba[green..green + 3], [0, 255, 0]);
}
