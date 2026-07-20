//! Repeated extraction of the same PDF must produce byte-identical output.
//!
//! Resource dictionaries used to be `HashMap`s, whose iteration order is seeded
//! per process, so the XObjects of a page came out in a different order on every
//! run — images were appended to the Markdown in a shuffled order, and the CLI's
//! image dedup (first occurrence wins) even wrote the same picture under a
//! different filename each time.

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
