//! One image, one resource, one id.
//!
//! An extracted image is referenced from the rendered output (`![](page1_Im0.jpg)`), listed in
//! the document's resource inventory, and looked up by id through every binding. Those have to
//! be the same id: a consumer joining an image reference to its metadata must not have to know
//! that one spelling carries the extension and the other does not.

mod common;

use unpdf::model::Block;
use unpdf::render::{to_markdown, RenderOptions};
use unpdf::{parse_bytes_with_options, Document, ParseOptions};

fn with_resources(bytes: &[u8]) -> Document {
    parse_bytes_with_options(bytes, ParseOptions::default().with_resources(true)).unwrap()
}

fn image_block_ids(doc: &Document) -> Vec<String> {
    doc.pages
        .iter()
        .flat_map(|page| &page.elements)
        .filter_map(|block| match block {
            Block::Image { resource_id, .. } => Some(resource_id.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_id_an_image_is_referenced_by_is_the_id_it_is_listed_under() {
    let doc = with_resources(&common::text_with_inline_image_pdf());

    let referenced = image_block_ids(&doc);
    let listed: Vec<&str> = doc.resources().map(|(id, _)| id).collect();
    assert_eq!(referenced.len(), 1, "one image block");
    assert_eq!(listed, [referenced[0].as_str()]);
    assert!(doc.get_resource(&referenced[0]).is_some());
}

#[test]
fn the_markdown_reference_resolves_in_the_inventory() {
    let doc = with_resources(&common::text_with_inline_image_pdf());
    let markdown = to_markdown(&doc, &RenderOptions::default()).unwrap();

    let (id, _) = doc.resources().next().expect("one resource");
    assert!(
        markdown.contains(&format!("]({id})")),
        "the markdown references {id:?}; got: {markdown}"
    );
}

#[test]
fn the_inventory_lists_resources_in_reading_order() {
    // Twelve pages, so a lexicographic order (`page10` before `page2`) cannot pass for page order.
    let doc = with_resources(&common::distinct_image_per_page_pdf(12));

    let pages: Vec<u32> = doc.resources().map(|(_, r)| r.page.unwrap()).collect();
    assert_eq!(pages, (1..=12).collect::<Vec<_>>());
    assert_eq!(doc.resource_count(), 12);
}

#[test]
fn a_shared_image_is_one_resource_and_every_reference_points_at_it() {
    let doc = with_resources(&common::repeated_logo_pdf(3));

    assert_eq!(doc.resource_count(), 1);
    let (id, _) = doc.resources().next().unwrap();
    for referenced in image_block_ids(&doc) {
        assert_eq!(
            referenced, id,
            "every page's reference defers to the first occurrence"
        );
    }
}
