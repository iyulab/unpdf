//! CID-to-Unicode lookup tables for Adobe CJK character collections.
//!
//! Tables are generated at compile time from Adobe's cid2code.txt files.

use std::sync::OnceLock;

/// A predefined CMap's `character code → CID` table, generated from `cid2code.txt`.
pub(crate) struct PredefinedCmap {
    /// Character collection the CIDs belong to (e.g. `KOREA1`).
    pub collection: &'static str,
    /// CMap name without its writing-mode suffix (e.g. `KSC-EUC`).
    pub column: &'static str,
    /// Code → CID pairs, sorted by code. One-byte codes are `0x00..=0xFF`.
    pub codes: &'static [(u16, u16)],
    /// Bytes that always begin a two-byte code, sorted.
    pub lead_bytes: &'static [u8],
}

// Include the generated tables
include!(concat!(env!("OUT_DIR"), "/cmap_tables.rs"));

/// The `CID → Unicode` table of an Adobe character collection, by `/CIDSystemInfo`
/// ordering, with the index of the collection among the four.
fn collection_table(ordering: &str) -> Option<(usize, &'static [(u32, u32)])> {
    match ordering {
        o if o.starts_with("Korea1") => Some((0, CID_TO_UNICODE_KOREA1)),
        o if o.starts_with("Japan1") => Some((1, CID_TO_UNICODE_JAPAN1)),
        o if o.starts_with("CNS1") => Some((2, CID_TO_UNICODE_CNS1)),
        o if o.starts_with("GB1") => Some((3, CID_TO_UNICODE_GB1)),
        _ => None,
    }
}

/// Look up a CID in the specified Adobe character collection.
pub fn lookup_cid(registry: &str, ordering: &str, cid: u32) -> Option<char> {
    if registry != "Adobe" {
        return None;
    }

    let (_, table) = collection_table(ordering)?;

    // Binary search on sorted table
    table
        .binary_search_by_key(&cid, |&(c, _)| c)
        .ok()
        .and_then(|idx| char::from_u32(table[idx].1))
}

/// The CID an Adobe character collection gives `ch`: the lowest CID whose character it
/// is.
///
/// This is the inverse of [`lookup_cid`], and what a collection's Unicode CMaps
/// (`UniKS-UCS2-H`, `UniJIS-UTF16-H`, ...) resolve a code to before a CIDFont's widths
/// can be read. Where a collection holds a character more than once — a proportional
/// and a full-width form, say — the lowest CID is the collection's primary form. That is
/// the CID the collection's own `UTF32` CMap chooses for every character this table
/// carries, except a few hundred Japan1 symbols for which `UniJIS` picks the full-width
/// form instead; only the advance width read for such a character differs.
pub(crate) fn cid_of_char(ordering: &str, ch: char) -> Option<u32> {
    static INDEXES: [OnceLock<Vec<(u32, u32)>>; 4] = [const { OnceLock::new() }; 4];

    let (slot, table) = collection_table(ordering)?;
    let index = INDEXES[slot].get_or_init(|| {
        // `(code point, CID)` sorted by code point, the lowest CID first — so after
        // `dedup_by_key` each code point keeps its lowest CID.
        let mut index: Vec<(u32, u32)> = table.iter().map(|&(cid, cp)| (cp, cid)).collect();
        index.sort_unstable();
        index.dedup_by_key(|entry| entry.0);
        index
    });
    let cp = u32::from(ch);
    index
        .binary_search_by_key(&cp, |&(c, _)| c)
        .ok()
        .map(|idx| index[idx].1)
}

/// Decode a byte sequence using CIDSystemInfo-based CMap lookup.
/// For Identity-H/V encoding, each 2-byte pair is treated as a CID.
pub fn decode_with_cid_system_info(registry: &str, ordering: &str, bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 || !bytes.len().is_multiple_of(2) {
        return None;
    }

    let mut result = String::new();
    let mut any_mapped = false;

    for chunk in bytes.chunks(2) {
        let cid = ((chunk[0] as u32) << 8) | (chunk[1] as u32);
        if let Some(ch) = lookup_cid(registry, ordering, cid) {
            result.push(ch);
            any_mapped = true;
        }
    }

    if any_mapped {
        Some(result)
    } else {
        None
    }
}
