//! Collapse resources whose bytes are identical, after all pages are assembled.
//!
//! A PDF can share one image XObject across every page -- a running-header logo is the ordinary
//! case -- but the extractor keys resources as `page{n}_{name}.{ext}`, a key space with no way to
//! say "the same image". So one logo in a 40-page document became 40 resource entries in
//! `page.images`, and (with the `ai` feature) 40 VLM calls for one picture.
//!
//! This runs as a pass over the finished document rather than inside page parsing, and that is
//! deliberate: `parse_single_page` is called from a parallel path in `stream.rs`, so deduping
//! there would make "which occurrence keeps the id" depend on thread scheduling. Walking the
//! assembled pages in order makes the winner the first occurrence in reading order, always.

use std::collections::HashMap;

use crate::model::{Block, Document};

/// Identity for two resources holding the same picture: a digest, then the bytes themselves.
///
/// The digest alone would be a bet, however good the odds; comparing the bytes on a digest hit
/// costs one memcmp per duplicate and removes the question. `md-5` is already in the tree for
/// PDF encryption, so this adds no dependency.
fn digest(bytes: &[u8]) -> [u8; 16] {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Rewrite every reference to a duplicated resource so it points at the first occurrence, and
/// drop the copies. Returns how many entries were removed, for the extraction diagnostics.
pub(crate) fn collapse_identical_resources(document: &mut Document) -> usize {
    // Page order, then order within the page: the same document always elects the same winner.
    let mut winner_by_digest: HashMap<[u8; 16], (String, Vec<u8>)> = HashMap::new();
    // Every id that turned out to be a duplicate, and the id it defers to.
    let mut alias: HashMap<String, String> = HashMap::new();

    for page in &document.pages {
        for (id, resource) in &page.images {
            if resource.data.is_empty() || alias.contains_key(id) {
                continue;
            }
            let d = digest(&resource.data);
            match winner_by_digest.get(&d) {
                Some((winner_id, winner_bytes)) if winner_bytes == &resource.data => {
                    if winner_id != id {
                        alias.insert(id.clone(), winner_id.clone());
                    }
                }
                // A digest collision between different bytes: keep both, which is the
                // conservative answer and the one this cannot get wrong.
                Some(_) => {}
                None => {
                    winner_by_digest.insert(d, (id.clone(), resource.data.clone()));
                }
            }
        }
    }

    if alias.is_empty() {
        return 0;
    }

    let mut removed = 0;
    for page in &mut document.pages {
        for block in &mut page.elements {
            if let Block::Image { resource_id, .. } = block {
                if let Some(winner) = alias.get(resource_id.as_str()) {
                    *resource_id = winner.clone();
                }
            }
        }
        let before = page.images.len();
        page.images.retain(|(id, _)| !alias.contains_key(id));
        removed += before - page.images.len();
    }

    removed
}
