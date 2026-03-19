//! PDF cross-reference (xref) table parser.

use std::collections::HashMap;

use crate::error::{Error, Result};

use super::stream;
use super::tokenizer::{self, dict_get, PdfDict, PdfObject};

/// An entry in the xref table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum XrefEntry {
    /// Object at byte offset in file.
    Uncompressed(usize),
    /// Object stored in ObjStm: (stream_obj_number, index_within_stream).
    Compressed(u32, u32),
}

/// Parsed xref table.
#[derive(Debug, Default)]
pub struct XrefTable {
    pub entries: HashMap<(u32, u16), XrefEntry>,
}

/// Find the startxref offset from end of file.
///
/// Searches backwards from end of file for the string `startxref`,
/// then parses the integer that follows it.
pub fn find_startxref(data: &[u8]) -> Result<usize> {
    // Search in the last 1024 bytes (or less for small files)
    let search_start = data.len().saturating_sub(1024);
    let tail = &data[search_start..];

    let marker = b"startxref";
    let mut found = None;

    // Search backwards for the last occurrence
    for i in (0..tail.len().saturating_sub(marker.len() - 1)).rev() {
        if tail[i..].starts_with(marker) {
            found = Some(search_start + i);
            break;
        }
    }

    let pos = found.ok_or_else(|| Error::PdfParse("startxref not found".into()))?;

    // Skip past "startxref" and any whitespace
    let mut p = pos + marker.len();
    while p < data.len() && is_whitespace(data[p]) {
        p += 1;
    }

    // Parse the offset number
    let num_start = p;
    while p < data.len() && data[p].is_ascii_digit() {
        p += 1;
    }

    if p == num_start {
        return Err(Error::PdfParse("no offset after startxref".into()));
    }

    let s = std::str::from_utf8(&data[num_start..p])
        .map_err(|_| Error::PdfParse("invalid startxref offset".into()))?;
    let offset: usize = s
        .parse()
        .map_err(|_| Error::PdfParse("invalid startxref offset".into()))?;

    Ok(offset)
}

/// Parse the complete xref chain (including incremental updates via /Prev).
/// Returns the merged xref table and the final (newest) trailer dictionary.
pub fn parse_xref_chain(data: &[u8]) -> Result<(XrefTable, PdfDict)> {
    let start_offset = find_startxref(data)?;
    let mut table = XrefTable::default();
    let mut newest_trailer: Option<PdfDict> = None;

    let mut offset = Some(start_offset);

    while let Some(xref_offset) = offset {
        if xref_offset >= data.len() {
            return Err(Error::PdfParse(format!(
                "xref offset {} beyond file size",
                xref_offset
            )));
        }

        let (entries, trailer) = parse_xref_at(data, xref_offset)?;

        // Merge entries: newest wins (don't overwrite existing entries)
        for (key, entry) in entries {
            table.entries.entry(key).or_insert(entry);
        }

        // Get /Prev for the next iteration
        offset = dict_get(&trailer, b"Prev").and_then(|o| o.as_i64()).map(|v| v as usize);

        if newest_trailer.is_none() {
            newest_trailer = Some(trailer);
        }
    }

    let trailer =
        newest_trailer.ok_or_else(|| Error::PdfParse("no trailer dictionary found".into()))?;

    Ok((table, trailer))
}

/// Parse an xref section at the given offset, returning entries and the trailer dict.
/// Handles both traditional xref tables and xref streams.
fn parse_xref_at(
    data: &[u8],
    offset: usize,
) -> Result<(Vec<((u32, u16), XrefEntry)>, PdfDict)> {
    let pos = skip_whitespace_simple(data, offset);

    // Check if this is a traditional xref table or an xref stream
    if pos + 4 <= data.len() && &data[pos..pos + 4] == b"xref" {
        parse_traditional_xref(data, pos)
    } else {
        // Must be an xref stream (indirect object)
        parse_xref_stream(data, pos)
    }
}

/// Parse a traditional xref table starting with the `xref` keyword.
fn parse_traditional_xref(
    data: &[u8],
    pos: usize,
) -> Result<(Vec<((u32, u16), XrefEntry)>, PdfDict)> {
    let mut p = pos + 4; // skip "xref"
    p = skip_whitespace_simple(data, p);

    let mut entries = Vec::new();

    // Parse subsections until we hit "trailer"
    loop {
        p = skip_whitespace_simple(data, p);

        if p >= data.len() {
            return Err(Error::PdfParse("unexpected end of xref table".into()));
        }

        // Check for "trailer"
        if p + 7 <= data.len() && &data[p..p + 7] == b"trailer" {
            p += 7;
            break;
        }

        // Parse subsection header: first_obj_num count
        let (first_obj, after_first) = parse_usize(data, p)?;
        p = skip_whitespace_simple(data, after_first);
        let (count, after_count) = parse_usize(data, p)?;
        p = skip_whitespace_simple(data, after_count);

        // Parse entries in this subsection
        for i in 0..count {
            let obj_num = first_obj + i;

            // Each entry is approximately 20 bytes: "0000000000 65535 f \r\n"
            // But be lenient: skip leading whitespace and parse fields
            p = skip_whitespace_simple(data, p);

            // Parse 10-digit offset
            let (offset_val, after_offset) = parse_usize(data, p)?;
            p = skip_whitespace_simple(data, after_offset);

            // Parse 5-digit generation
            let (gen_val, after_gen) = parse_usize(data, p)?;
            p = skip_whitespace_simple(data, after_gen);

            // Parse type: 'n' or 'f'
            if p >= data.len() {
                return Err(Error::PdfParse("unexpected end in xref entry".into()));
            }

            let entry_type = data[p];
            p += 1;

            // Skip any trailing whitespace/newline after the entry type
            while p < data.len() && (data[p] == b' ' || data[p] == b'\r' || data[p] == b'\n') {
                p += 1;
            }

            match entry_type {
                b'n' => {
                    entries.push(((obj_num as u32, gen_val as u16), XrefEntry::Uncompressed(offset_val)));
                }
                b'f' => {
                    // Free entry, skip
                }
                _ => {
                    return Err(Error::PdfParse(format!(
                        "invalid xref entry type '{}' for object {}",
                        entry_type as char, obj_num
                    )));
                }
            }
        }
    }

    // Parse trailer dictionary
    p = skip_whitespace_simple(data, p);
    let (trailer_obj, _) = tokenizer::parse_object(data, p)?;
    let trailer = match trailer_obj {
        PdfObject::Dict(d) => d,
        _ => {
            return Err(Error::PdfParse("trailer is not a dictionary".into()));
        }
    };

    Ok((entries, trailer))
}

/// Parse an xref stream object at the given position.
fn parse_xref_stream(
    data: &[u8],
    pos: usize,
) -> Result<(Vec<((u32, u16), XrefEntry)>, PdfDict)> {
    // Parse the indirect object (N 0 obj << ... >> stream ... endstream endobj)
    let (obj, _) = tokenizer::parse_object(data, pos)?;

    let stream = match &obj {
        PdfObject::Stream(s) => s,
        _ => {
            return Err(Error::PdfParse(
                "expected xref stream object".into(),
            ));
        }
    };

    // The stream dict serves as the trailer
    let dict = stream.dict.clone();

    // Get /W (field widths)
    let w = dict_get(&dict, b"W")
        .and_then(|o| o.as_array())
        .ok_or_else(|| Error::PdfParse("xref stream missing /W".into()))?;

    if w.len() < 3 {
        return Err(Error::PdfParse("xref stream /W must have 3 entries".into()));
    }

    let w1 = w[0].as_i64().unwrap_or(0) as usize;
    let w2 = w[1].as_i64().unwrap_or(0) as usize;
    let w3 = w[2].as_i64().unwrap_or(0) as usize;
    let entry_size = w1 + w2 + w3;

    if entry_size == 0 {
        return Err(Error::PdfParse("xref stream entry size is 0".into()));
    }

    // Get /Size
    let size = dict_get(&dict, b"Size")
        .and_then(|o| o.as_i64())
        .unwrap_or(0) as usize;

    // Get /Index array (default: [0 Size])
    let index_ranges: Vec<(usize, usize)> = if let Some(idx_arr) = dict_get(&dict, b"Index").and_then(|o| o.as_array()) {
        idx_arr
            .chunks(2)
            .filter_map(|chunk| {
                if chunk.len() == 2 {
                    let first = chunk[0].as_i64()? as usize;
                    let count = chunk[1].as_i64()? as usize;
                    Some((first, count))
                } else {
                    None
                }
            })
            .collect()
    } else {
        vec![(0, size)]
    };

    // Decompress stream data
    let stream_data = stream::decompress(stream)?;

    // Parse entries from decompressed data
    let mut entries = Vec::new();
    let mut data_pos = 0;

    for (first, count) in &index_ranges {
        for i in 0..*count {
            let obj_num = (*first + i) as u32;

            if data_pos + entry_size > stream_data.len() {
                break;
            }

            // Read type field (default 1 if width is 0)
            let entry_type = if w1 == 0 {
                1u64
            } else {
                read_field(&stream_data, data_pos, w1)
            };

            let field2 = read_field(&stream_data, data_pos + w1, w2);
            let field3 = read_field(&stream_data, data_pos + w1 + w2, w3);

            data_pos += entry_size;

            match entry_type {
                0 => {
                    // Free entry, skip
                }
                1 => {
                    // Uncompressed: field2 = byte offset, field3 = generation
                    entries.push((
                        (obj_num, field3 as u16),
                        XrefEntry::Uncompressed(field2 as usize),
                    ));
                }
                2 => {
                    // Compressed: field2 = stream obj number, field3 = index in stream
                    entries.push((
                        (obj_num, 0),
                        XrefEntry::Compressed(field2 as u32, field3 as u32),
                    ));
                }
                _ => {
                    // Unknown type, skip (be lenient)
                }
            }
        }
    }

    Ok((entries, dict))
}

/// Read a big-endian unsigned integer field of `width` bytes from `data` at `offset`.
fn read_field(data: &[u8], offset: usize, width: usize) -> u64 {
    if width == 0 {
        return 0;
    }
    let mut val: u64 = 0;
    for i in 0..width {
        if offset + i < data.len() {
            val = (val << 8) | data[offset + i] as u64;
        }
    }
    val
}

/// Parse an unsigned integer from data at the given position.
/// Returns the parsed value and the position after it.
fn parse_usize(data: &[u8], pos: usize) -> Result<(usize, usize)> {
    let mut p = pos;
    while p < data.len() && data[p].is_ascii_digit() {
        p += 1;
    }
    if p == pos {
        return Err(Error::PdfParse(format!(
            "expected number at offset {}",
            pos
        )));
    }
    let s = std::str::from_utf8(&data[pos..p])
        .map_err(|_| Error::PdfParse(format!("invalid number at offset {}", pos)))?;
    let val: usize = s
        .parse()
        .map_err(|_| Error::PdfParse(format!("invalid number at offset {}", pos)))?;
    Ok((val, p))
}

/// Simple whitespace skipper (no comment handling needed for xref sections).
fn skip_whitespace_simple(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() && is_whitespace(data[pos]) {
        pos += 1;
    }
    pos
}

fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\0' | b'\x0C')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_startxref() {
        let data = b"%PDF-1.4\nsome content\nstartxref\n12345\n%%EOF";
        assert_eq!(find_startxref(data).unwrap(), 12345);
    }

    #[test]
    fn test_find_startxref_with_trailing_whitespace() {
        let data = b"%PDF-1.4\nstartxref\n999\n%%EOF\n\n";
        assert_eq!(find_startxref(data).unwrap(), 999);
    }

    #[test]
    fn test_parse_traditional_xref() {
        // Build data so that the startxref value matches the actual offset of "xref"
        let prefix = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n";
        let xref_offset = prefix.len(); // 45
        let mut data = Vec::new();
        data.extend_from_slice(prefix);
        data.extend_from_slice(b"xref\n0 3\n");
        data.extend_from_slice(b"0000000000 65535 f \r\n");
        data.extend_from_slice(b"0000000009 00000 n \r\n");
        data.extend_from_slice(b"0000000058 00000 n \r\n");
        data.extend_from_slice(b"trailer\n<< /Size 3 /Root 1 0 R >>\n");
        data.extend_from_slice(format!("startxref\n{}\n%%EOF", xref_offset).as_bytes());

        let (table, trailer) = parse_xref_chain(&data).unwrap();
        // Should have 2 in-use entries (obj 1 and 2)
        assert_eq!(table.entries.len(), 2);
        assert!(table.entries.contains_key(&(1, 0)));
        assert!(table.entries.contains_key(&(2, 0)));
        // Trailer should have /Root
        assert!(dict_get(&trailer, b"Root").is_some());
    }

    #[test]
    fn test_find_startxref_not_found() {
        let data = b"%PDF-1.4\nno xref here\n%%EOF";
        assert!(find_startxref(data).is_err());
    }

    #[test]
    fn test_traditional_xref_multiple_subsections() {
        // xref table with two subsections
        let mut buf = Vec::new();
        buf.extend_from_slice(b"%PDF-1.4\n");
        // obj 1 at offset 9 (dummy)
        let xref_start = buf.len();
        buf.extend_from_slice(b"xref\n");
        buf.extend_from_slice(b"0 2\n");
        buf.extend_from_slice(b"0000000000 65535 f \r\n");
        buf.extend_from_slice(b"0000000009 00000 n \r\n");
        buf.extend_from_slice(b"5 1\n");
        buf.extend_from_slice(b"0000000100 00000 n \r\n");
        buf.extend_from_slice(b"trailer\n");
        buf.extend_from_slice(b"<< /Size 6 /Root 1 0 R >>\n");
        buf.extend_from_slice(format!("startxref\n{}\n%%EOF", xref_start).as_bytes());

        let (table, _trailer) = parse_xref_chain(&buf).unwrap();
        assert_eq!(table.entries.len(), 2);
        assert!(table.entries.contains_key(&(1, 0)));
        assert!(table.entries.contains_key(&(5, 0)));
    }

    #[test]
    fn test_read_field() {
        // 2-byte big-endian
        assert_eq!(read_field(&[0x01, 0x00], 0, 2), 256);
        assert_eq!(read_field(&[0xFF], 0, 1), 255);
        assert_eq!(read_field(&[], 0, 0), 0);
    }
}
