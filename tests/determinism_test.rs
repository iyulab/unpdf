//! Repeated extraction of the same PDF must produce byte-identical output.
//!
//! Resource dictionaries used to be `HashMap`s, whose iteration order is seeded
//! per process, so the XObjects of a page came out in a different order on every
//! run — images were appended to the Markdown in a shuffled order, and the CLI's
//! image dedup (first occurrence wins) even wrote the same picture under a
//! different filename each time.

use unpdf::render::{JsonFormat, RenderOptions};
use unpdf::{parse_bytes_with_options, Block, ParseOptions};

/// Enough images that a shuffled traversal cannot plausibly come out sorted.
const IMAGE_COUNT: usize = 12;

#[test]
fn xobject_order_is_stable_across_runs() {
    let pdf = multi_image_pdf();
    let first = image_ids(&pdf);

    assert_eq!(first.len(), IMAGE_COUNT, "all images must be extracted");
    for _ in 0..4 {
        assert_eq!(image_ids(&pdf), first, "image order must not vary per run");
    }
}

/// The order is not merely stable but defined: XObjects come out sorted by the
/// name they carry in the resource dictionary.
///
/// When images are eventually interleaved by their `Do` operator position, this
/// assertion is the one to rewrite — the stability tests above stay as they are.
#[test]
fn xobject_order_follows_resource_names() {
    let ids = image_ids(&multi_image_pdf());
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

/// The image *blocks* carry the same order — this is what a renderer walks, so
/// it, not just the resource list, decides where each `![](…)` lands.
#[test]
fn image_blocks_match_the_resource_order() {
    let pdf = multi_image_pdf();
    let doc = parse_bytes_with_options(&pdf, options()).unwrap();

    let block_ids: Vec<&str> = doc.pages[0]
        .elements
        .iter()
        .filter_map(|b| match b {
            Block::Image { resource_id, .. } => Some(resource_id.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(block_ids, image_ids(&pdf));
}

fn options() -> ParseOptions {
    ParseOptions::new().with_resources(true)
}

fn image_ids(pdf: &[u8]) -> Vec<String> {
    let doc = parse_bytes_with_options(pdf, options()).unwrap();
    doc.pages[0]
        .images
        .iter()
        .map(|(id, _)| id.clone())
        .collect()
}

/// A one-page PDF whose resource dictionary holds [`IMAGE_COUNT`] distinct JPEGs.
///
/// The data is not real JPEG — nothing decodes it — but the `DCTDecode` filter
/// makes the extractor treat each stream as one, and the bytes differ per image
/// so that dedup never collapses them.
fn multi_image_pdf() -> Vec<u8> {
    let names: Vec<String> = (0..IMAGE_COUNT).map(|i| format!("Im{i}")).collect();

    let mut content = String::new();
    let mut xobject_entries = String::new();
    let mut image_objects: Vec<Vec<u8>> = Vec::new();

    // Images are drawn in reverse name order to keep the content stream from
    // accidentally agreeing with the expected (name-sorted) output order.
    for (idx, name) in names.iter().enumerate().rev() {
        let obj_num = 5 + idx;
        content.push_str(&format!("q 100 0 0 100 0 0 cm /{name} Do Q\n"));
        xobject_entries.push_str(&format!("/{name} {obj_num} 0 R"));
        image_objects.push(stream_object(
            "<</Type/XObject/Subtype/Image/Width 100/Height 100/ColorSpace/DeviceRGB\
              /BitsPerComponent 8/Filter/DCTDecode/Length 4>>",
            &[0xFF, 0xD8, 0xFF, idx as u8],
        ));
    }
    image_objects.reverse();

    let mut objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
             /Resources<</XObject<<{xobject_entries}>>>>/Contents 4 0 R>>"
        )
        .into_bytes(),
        stream_object(
            &format!("<</Length {}>>", content.len()),
            content.as_bytes(),
        ),
    ];
    objects.extend(image_objects);

    assemble(objects)
}

fn stream_object(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut obj = dict.as_bytes().to_vec();
    obj.extend_from_slice(b"\nstream\n");
    obj.extend_from_slice(data);
    obj.extend_from_slice(b"\nendstream");
    obj
}

fn assemble(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }

    let xref_start = pdf.len();
    let size = objects.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in &offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xref_start}\n%%EOF\n")
            .as_bytes(),
    );
    pdf
}

// ---- The text, not only the resources: the same file reads the same on every run ----

/// Fixtures that hold text; each is read many times and must come out the same every time.
const FIXTURES: &[(&str, &[u8])] = &[
    ("latex_resume", include_bytes!("fixtures/latex_resume.pdf")),
    (
        "inline-font-two-pages",
        include_bytes!("fixtures/inline-font-two-pages.pdf"),
    ),
    (
        "inline-font-type0",
        include_bytes!("fixtures/inline-font-type0.pdf"),
    ),
    (
        "signed-number-lines",
        include_bytes!("fixtures/signed-number-lines.pdf"),
    ),
    (
        "cid-korea1-ucs2-h",
        include_bytes!("fixtures/cid-korea1-ucs2-h.pdf"),
    ),
];

#[test]
fn the_same_pdf_gives_the_same_markdown_and_json_every_time() {
    for (name, pdf) in FIXTURES {
        let read = || {
            let doc = unpdf::parse_bytes(pdf).unwrap_or_else(|e| panic!("{name}: {e}"));
            (
                unpdf::render::to_markdown(&doc, &RenderOptions::default()).expect("renders"),
                unpdf::render::to_json(&doc, JsonFormat::Pretty).expect("serializes"),
            )
        };
        let first = read();
        assert!(!first.0.trim().is_empty(), "{name}: no text to compare");
        for run in 1..16 {
            assert_eq!(read(), first, "{name}: run {run} differs from the first");
        }
    }
}

/// A one-page PDF whose lines are set in four sizes, each used for the same number of
/// characters: the most frequent size — which decides what counts as a heading — is a tie.
fn pdf_with_tied_sizes() -> Vec<u8> {
    let mut content = String::new();
    for (i, size) in [10, 12, 14, 18].iter().enumerate() {
        content.push_str(&format!(
            "BT /F1 {size} Tf 72 {} Td (TIE{size}TIE{size}) Tj ET\n",
            700 - 40 * i
        ));
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref_at = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in &offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    out
}

#[test]
fn sizes_used_equally_often_are_decided_the_same_way_every_time() {
    let pdf = pdf_with_tied_sizes();
    let read = || {
        let doc = unpdf::parse_bytes(&pdf).expect("the PDF reads");
        unpdf::render::to_markdown(&doc, &RenderOptions::default()).expect("renders")
    };
    let first = read();
    assert!(first.contains("TIE18"), "the text is read: {first:?}");
    for run in 1..32 {
        assert_eq!(read(), first, "run {run} differs from the first");
    }
}
