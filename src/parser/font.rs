//! Font decoding utilities for PDF text extraction.
//!
//! Contains ToUnicode CMap parsing, TrueType cmap table parsing,
//! and simple text decoding fallbacks.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Simple text decoding
// ---------------------------------------------------------------------------

/// Simple text decoding fallback when no encoding is available.
pub fn decode_text_simple(bytes: &[u8]) -> String {
    // Try UTF-16BE first (BOM marker)
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let utf16: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter_map(|c| {
                if c.len() == 2 {
                    Some(u16::from_be_bytes([c[0], c[1]]))
                } else {
                    None
                }
            })
            .collect();
        return String::from_utf16(&utf16).unwrap_or_default();
    }

    // Try UTF-8
    if let Ok(s) = String::from_utf8(bytes.to_vec()) {
        return s;
    }

    // Fallback: Latin-1
    bytes.iter().map(|&b| b as char).collect()
}

// ---------------------------------------------------------------------------
// ToUnicode CMap parser
// ---------------------------------------------------------------------------

/// Parsed ToUnicode CMap: maps character codes to Unicode strings.
#[derive(Debug, Clone)]
pub(crate) struct ToUnicodeMap {
    /// Bytes per character code (1 or 2). Determined from codespace range.
    pub(crate) code_width: usize,
    /// Character code → Unicode string mapping.
    pub(crate) mappings: HashMap<u32, String>,
}

impl ToUnicodeMap {
    /// Decode a byte sequence using this CMap.
    pub(crate) fn decode(&self, bytes: &[u8]) -> String {
        let mut result = String::new();
        let mut i = 0;
        while i < bytes.len() {
            if self.code_width == 2 && i + 1 < bytes.len() {
                let code = u32::from(bytes[i]) << 8 | u32::from(bytes[i + 1]);
                if let Some(s) = self.mappings.get(&code) {
                    result.push_str(s);
                }
                // If unmapped, skip silently (common for space/control chars)
                i += 2;
            } else {
                let code = u32::from(bytes[i]);
                if let Some(s) = self.mappings.get(&code) {
                    result.push_str(s);
                }
                i += 1;
            }
        }
        result
    }
}

/// Parse a hex string like "0048" into a u32 value.
fn parse_hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s, 16).ok()
}

/// Filter out Unicode noncharacters and control sentinels that ToUnicode CMaps
/// commonly use to indicate "no mapping" (U+FFFF, U+FFFE, U+FFFD).
/// Also drops other noncharacters (U+FDD0..U+FDEF, U+xFFFE, U+xFFFF).
fn sanitize_unicode(s: String) -> Option<String> {
    let filtered: String = s
        .chars()
        .filter(|&c| {
            let cp = c as u32;
            if cp == 0xFFFD || cp == 0xFFFE || cp == 0xFFFF {
                return false;
            }
            if (0xFDD0..=0xFDEF).contains(&cp) {
                return false;
            }
            // Plane-specific noncharacters: U+xFFFE and U+xFFFF for any plane
            if cp >= 0x10000 && (cp & 0xFFFF) >= 0xFFFE {
                return false;
            }
            // Private Use Area — PDF producers (notably Hancom) sometimes map
            // bullet/custom glyphs to PUA codepoints via the embedded TrueType
            // cmap. These render as tofu in markdown; treat them as unmapped.
            // Supplementary PUA planes are also dropped.
            if (0xE000..=0xF8FF).contains(&cp)
                || (0xF0000..=0xFFFFD).contains(&cp)
                || (0x100000..=0x10FFFD).contains(&cp)
            {
                return false;
            }
            true
        })
        .collect();
    if filtered.is_empty() {
        None
    } else {
        Some(filtered)
    }
}

/// Decode a hex string into a Unicode string.
/// The hex represents UTF-16BE code units (e.g., "0048" → "H", "D800DC00" → surrogate pair).
/// Noncharacter sentinels (U+FFFF etc.) commonly used by PDF producers to indicate
/// "no Unicode mapping" are stripped; returns None if nothing usable remains.
fn hex_to_unicode(hex: &str) -> Option<String> {
    if hex.len() % 4 != 0 && hex.len() == 2 {
        // Single-byte mapping: treat as direct code point
        let cp = u32::from_str_radix(hex, 16).ok()?;
        let s = char::from_u32(cp).map(|c| c.to_string())?;
        return sanitize_unicode(s);
    }

    // Parse as UTF-16BE code units (each 4 hex digits = one u16)
    let mut units = Vec::new();
    let mut i = 0;
    while i + 3 < hex.len() {
        let val = u16::from_str_radix(&hex[i..i + 4], 16).ok()?;
        units.push(val);
        i += 4;
    }
    let s = String::from_utf16(&units).ok()?;
    sanitize_unicode(s)
}

/// Parse a ToUnicode CMap stream into a `ToUnicodeMap`.
pub(crate) fn parse_to_unicode_cmap(data: &[u8]) -> Option<ToUnicodeMap> {
    let text = String::from_utf8_lossy(data);
    let mut mappings = HashMap::new();
    let mut code_width: usize = 2; // default for Identity-H

    // Parse codespace range to determine code width
    if let Some(cs_start) = text.find("begincodespacerange") {
        if let Some(cs_end) = text[cs_start..].find("endcodespacerange") {
            let cs_block = &text[cs_start..cs_start + cs_end];
            // Look for hex strings like <0000> or <00>
            if let Some(first_angle) = cs_block.find('<') {
                if let Some(close_angle) = cs_block[first_angle..].find('>') {
                    let hex_len = close_angle - 1; // length of hex string
                    code_width = hex_len / 2; // 2 hex chars = 1 byte
                    if code_width == 0 {
                        code_width = 1;
                    }
                }
            }
        }
    }

    // Parse beginbfchar sections
    let mut search_pos = 0;
    while let Some(start) = text[search_pos..].find("beginbfchar") {
        let block_start = search_pos + start + "beginbfchar".len();
        if let Some(end) = text[block_start..].find("endbfchar") {
            let block = &text[block_start..block_start + end];
            // Parse lines like: <0003> <0020>
            for line in block.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line
                    .split(['<', '>'])
                    .filter(|s| !s.trim().is_empty())
                    .collect();
                if parts.len() >= 2 {
                    if let Some(code) = parse_hex(parts[0].trim()) {
                        if let Some(unicode_str) = hex_to_unicode(parts[1].trim()) {
                            mappings.insert(code, unicode_str);
                        }
                    }
                }
            }
            search_pos = block_start + end;
        } else {
            break;
        }
    }

    // Parse beginbfrange sections
    search_pos = 0;
    while let Some(start) = text[search_pos..].find("beginbfrange") {
        let block_start = search_pos + start + "beginbfrange".len();
        if let Some(end) = text[block_start..].find("endbfrange") {
            let block = &text[block_start..block_start + end];
            for line in block.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                // Check for array form: <start> <end> [<u1> <u2> ...]
                if line.contains('[') {
                    let parts: Vec<&str> = line
                        .split(['<', '>', '[', ']'])
                        .filter(|s| !s.trim().is_empty())
                        .collect();
                    if parts.len() >= 3 {
                        if let (Some(lo), Some(hi)) =
                            (parse_hex(parts[0].trim()), parse_hex(parts[1].trim()))
                        {
                            for (i, code) in (lo..=hi).enumerate() {
                                if let Some(unicode_str) =
                                    parts.get(2 + i).and_then(|h| hex_to_unicode(h.trim()))
                                {
                                    mappings.insert(code, unicode_str);
                                }
                            }
                        }
                    }
                } else {
                    // Simple form: <start> <end> <dst_start>
                    let parts: Vec<&str> = line
                        .split(['<', '>'])
                        .filter(|s| !s.trim().is_empty())
                        .collect();
                    if parts.len() >= 3 {
                        if let (Some(lo), Some(hi), Some(dst_start)) = (
                            parse_hex(parts[0].trim()),
                            parse_hex(parts[1].trim()),
                            parse_hex(parts[2].trim()),
                        ) {
                            for (i, code) in (lo..=hi).enumerate() {
                                let dst = dst_start + i as u32;
                                if let Some(c) = char::from_u32(dst) {
                                    if let Some(s) = sanitize_unicode(c.to_string()) {
                                        mappings.insert(code, s);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            search_pos = block_start + end;
        } else {
            break;
        }
    }

    if mappings.is_empty() {
        return None;
    }

    // Auto-correct code_width based on actual mapping keys.
    // Some CMaps declare a 2-byte codespace (e.g., <0000> <FFFF>) but
    // actually only use single-byte codes (max key <= 0xFF). This is common
    // for Type1 fonts with custom encodings.
    if code_width == 2 {
        let max_key = mappings.keys().copied().max().unwrap_or(0);
        if max_key <= 0xFF {
            code_width = 1;
        }
    }

    Some(ToUnicodeMap {
        code_width,
        mappings,
    })
}

// ---------------------------------------------------------------------------
// TrueType cmap table parser
// ---------------------------------------------------------------------------

/// Parse a TrueType font's cmap table to build a GID→Unicode mapping.
///
/// For Identity-H CID fonts, the character codes in the content stream are 2-byte
/// glyph IDs (GIDs). The TrueType cmap table maps Unicode code points → GIDs.
/// We reverse this to get GID → Unicode.
pub(crate) fn parse_truetype_cmap_table(data: &[u8]) -> Option<ToUnicodeMap> {
    if data.len() < 12 {
        return None;
    }

    // Read TrueType offset table
    let num_tables = u16::from_be_bytes([data[4], data[5]]) as usize;

    // Find the 'cmap' table
    let mut cmap_offset = 0u32;
    let mut cmap_length = 0u32;
    for i in 0..num_tables {
        let record_offset = 12 + i * 16;
        if record_offset + 16 > data.len() {
            break;
        }
        let tag = &data[record_offset..record_offset + 4];
        if tag == b"cmap" {
            cmap_offset = u32::from_be_bytes([
                data[record_offset + 8],
                data[record_offset + 9],
                data[record_offset + 10],
                data[record_offset + 11],
            ]);
            cmap_length = u32::from_be_bytes([
                data[record_offset + 12],
                data[record_offset + 13],
                data[record_offset + 14],
                data[record_offset + 15],
            ]);
            break;
        }
    }

    if cmap_offset == 0 || cmap_offset as usize + 4 > data.len() {
        return None;
    }

    let cmap = &data[cmap_offset as usize..];
    let cmap_len = cmap_length as usize;
    if cmap_len < 4 {
        return None;
    }

    // cmap header: version (u16), numTables (u16)
    let num_subtables = u16::from_be_bytes([cmap[2], cmap[3]]) as usize;

    // Find the best subtable: prefer (3,1) Windows Unicode BMP, fallback to (0,3) Unicode BMP
    let mut best_offset: Option<u32> = None;
    let mut best_priority = 0u8;

    for i in 0..num_subtables {
        let rec = 4 + i * 8;
        if rec + 8 > cmap_len {
            break;
        }
        let platform_id = u16::from_be_bytes([cmap[rec], cmap[rec + 1]]);
        let encoding_id = u16::from_be_bytes([cmap[rec + 2], cmap[rec + 3]]);
        let offset =
            u32::from_be_bytes([cmap[rec + 4], cmap[rec + 5], cmap[rec + 6], cmap[rec + 7]]);

        let priority = match (platform_id, encoding_id) {
            (3, 1) => 3, // Windows Unicode BMP — best
            (0, 3) => 2, // Unicode BMP
            (0, _) => 1, // Any Unicode
            _ => 0,
        };

        if priority > best_priority {
            best_priority = priority;
            best_offset = Some(offset);
        }
    }

    let subtable_offset = best_offset? as usize;
    if subtable_offset + 2 > cmap_len {
        return None;
    }

    let subtable = &cmap[subtable_offset..];
    let format = u16::from_be_bytes([subtable[0], subtable[1]]);

    // Build Unicode→GID map, then reverse to GID→Unicode
    let unicode_to_gid = match format {
        4 => parse_cmap_format4(subtable)?,
        12 => parse_cmap_format12(subtable)?,
        _ => {
            log::debug!("Unsupported cmap subtable format {}", format);
            return None;
        }
    };

    // Reverse: GID → Unicode
    let mut gid_to_unicode: HashMap<u32, String> = HashMap::new();
    for (unicode_cp, gid) in &unicode_to_gid {
        if *gid > 0 {
            // Only keep the first mapping for each GID (lowest Unicode code point)
            if let Some(s) = char::from_u32(*unicode_cp)
                .map(|c| c.to_string())
                .and_then(sanitize_unicode)
            {
                gid_to_unicode.entry(*gid as u32).or_insert(s);
            }
        }
    }

    if gid_to_unicode.is_empty() {
        return None;
    }

    log::debug!(
        "Parsed embedded TrueType cmap: {} GID→Unicode mappings",
        gid_to_unicode.len()
    );

    Some(ToUnicodeMap {
        code_width: 2, // Identity-H always uses 2-byte codes
        mappings: gid_to_unicode,
    })
}

/// Parse cmap format 4 (Segment mapping to delta values).
/// Returns a map of Unicode code point → glyph ID.
fn parse_cmap_format4(data: &[u8]) -> Option<HashMap<u32, u16>> {
    if data.len() < 14 {
        return None;
    }

    let seg_count_x2 = u16::from_be_bytes([data[6], data[7]]) as usize;
    let seg_count = seg_count_x2 / 2;

    // Offsets into the subtable
    let end_codes_offset = 14;
    let start_codes_offset = end_codes_offset + seg_count_x2 + 2; // +2 for reservedPad
    let id_delta_offset = start_codes_offset + seg_count_x2;
    let id_range_offset_offset = id_delta_offset + seg_count_x2;

    let needed = id_range_offset_offset + seg_count_x2;
    if needed > data.len() {
        return None;
    }

    let mut result = HashMap::new();

    for seg in 0..seg_count {
        let end_code = u16::from_be_bytes([
            data[end_codes_offset + seg * 2],
            data[end_codes_offset + seg * 2 + 1],
        ]);
        let start_code = u16::from_be_bytes([
            data[start_codes_offset + seg * 2],
            data[start_codes_offset + seg * 2 + 1],
        ]);
        let id_delta = i16::from_be_bytes([
            data[id_delta_offset + seg * 2],
            data[id_delta_offset + seg * 2 + 1],
        ]);
        let id_range_offset = u16::from_be_bytes([
            data[id_range_offset_offset + seg * 2],
            data[id_range_offset_offset + seg * 2 + 1],
        ]);

        if start_code == 0xFFFF {
            break;
        }

        for code in start_code..=end_code {
            let gid = if id_range_offset == 0 {
                (code as i32 + id_delta as i32) as u16
            } else {
                // glyphId = *(idRangeOffset[i]/2 + (c - startCode[i]) + &idRangeOffset[i])
                let glyph_idx_offset = id_range_offset_offset
                    + seg * 2
                    + id_range_offset as usize
                    + (code - start_code) as usize * 2;
                if glyph_idx_offset + 1 < data.len() {
                    let gid_raw =
                        u16::from_be_bytes([data[glyph_idx_offset], data[glyph_idx_offset + 1]]);
                    if gid_raw != 0 {
                        (gid_raw as i32 + id_delta as i32) as u16
                    } else {
                        0
                    }
                } else {
                    0
                }
            };

            if gid != 0 {
                result.insert(code as u32, gid);
            }
        }
    }

    Some(result)
}

/// Parse cmap format 12 (Segmented coverage, 32-bit).
/// Returns a map of Unicode code point → glyph ID.
fn parse_cmap_format12(data: &[u8]) -> Option<HashMap<u32, u16>> {
    if data.len() < 16 {
        return None;
    }

    let n_groups = u32::from_be_bytes([data[12], data[13], data[14], data[15]]) as usize;
    let mut result = HashMap::new();

    for i in 0..n_groups {
        let offset = 16 + i * 12;
        if offset + 12 > data.len() {
            break;
        }
        let start_char = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        let end_char = u32::from_be_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        let start_gid = u32::from_be_bytes([
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
            data[offset + 11],
        ]);

        // Limit range to prevent excessive memory usage
        if end_char - start_char > 0x10000 {
            continue;
        }

        for code in start_char..=end_char {
            let gid = start_gid + (code - start_char);
            if gid != 0 && gid <= 0xFFFF {
                result.insert(code, gid as u16);
            }
        }
    }

    Some(result)
}

/// Check if a decoded string is likely binary garbage (CID bytes interpreted as Latin-1).
///
/// Heuristic: if a significant proportion of characters are in the Latin-1 supplement
/// range (0x80–0xFF) and not common accented characters, it's probably garbage.
pub(crate) fn is_likely_binary(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    let total_chars = text.chars().count();
    if total_chars == 0 {
        return false;
    }

    let suspicious_count = text
        .chars()
        .filter(|&c| {
            let code = c as u32;
            // Control characters (except common whitespace)
            (code < 0x20 && !matches!(c, '\n' | '\r' | '\t'))
        // High Latin-1 supplement characters that rarely appear in real text
        || (0x80..0xA0).contains(&code)
        // Private Use Area
        || (0xE000..=0xF8FF).contains(&code)
        })
        .count();

    // If more than 30% of characters are suspicious, it's likely garbage
    suspicious_count as f32 / total_chars as f32 > 0.3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_text_simple_utf8() {
        assert_eq!(decode_text_simple(b"Hello"), "Hello");
    }

    #[test]
    fn test_decode_text_simple_latin1() {
        // 0xE9 = 'é' in Latin-1
        let bytes = vec![0x48, 0x65, 0x6C, 0x6C, 0xE9];
        let text = decode_text_simple(&bytes);
        assert_eq!(text, "Hellé");
    }

    #[test]
    fn test_decode_text_simple_utf16be() {
        // UTF-16BE BOM + "Hi"
        let bytes = vec![0xFE, 0xFF, 0x00, 0x48, 0x00, 0x69];
        assert_eq!(decode_text_simple(&bytes), "Hi");
    }

    #[test]
    fn test_parse_to_unicode_cmap_bfchar() {
        let cmap = b"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
1 begincodespacerange
<0000> <ffff>
endcodespacerange
3 beginbfchar
<0003> <0020>
<001C> <0039>
<0024> <0041>
endbfchar
endcmap";
        let map = parse_to_unicode_cmap(cmap).unwrap();
        // code_width auto-corrected to 1 because max key (0x24=36) <= 0xFF
        assert_eq!(map.code_width, 1);
        assert_eq!(map.mappings.get(&0x0003), Some(&" ".to_string()));
        assert_eq!(map.mappings.get(&0x001C), Some(&"9".to_string()));
        assert_eq!(map.mappings.get(&0x0024), Some(&"A".to_string()));
    }

    #[test]
    fn test_parse_to_unicode_cmap_bfchar_2byte() {
        // CMap with codes > 0xFF should keep code_width=2
        let cmap = b"1 begincodespacerange
<0000> <ffff>
endcodespacerange
2 beginbfchar
<0100> <AC00>
<0200> <AD00>
endbfchar";
        let map = parse_to_unicode_cmap(cmap).unwrap();
        assert_eq!(map.code_width, 2);
        assert_eq!(map.mappings.get(&0x0100), Some(&"\u{AC00}".to_string()));
    }

    #[test]
    fn test_parse_to_unicode_cmap_bfrange() {
        let cmap = b"1 begincodespacerange
<00> <FF>
endcodespacerange
1 beginbfrange
<20> <7E> <0020>
endbfrange";
        let map = parse_to_unicode_cmap(cmap).unwrap();
        assert_eq!(map.code_width, 1);
        assert_eq!(map.mappings.get(&0x20), Some(&" ".to_string()));
        assert_eq!(map.mappings.get(&0x41), Some(&"A".to_string()));
        assert_eq!(map.mappings.get(&0x7E), Some(&"~".to_string()));
    }

    #[test]
    fn test_to_unicode_map_decode_1byte() {
        // CMap with small keys: auto-corrected to code_width=1
        let cmap = b"1 begincodespacerange
<0000> <ffff>
endcodespacerange
3 beginbfchar
<0003> <0020>
<001C> <0039>
<0024> <0041>
endbfchar";
        let map = parse_to_unicode_cmap(cmap).unwrap();
        assert_eq!(map.code_width, 1);
        // Decode single-byte codes: 0x03=space, 0x1C='9', 0x24='A'
        let result = map.decode(&[0x03, 0x1C, 0x24]);
        assert_eq!(result, " 9A");
    }

    #[test]
    fn test_to_unicode_map_decode_2byte() {
        // CMap with large keys: stays code_width=2
        let cmap = b"1 begincodespacerange
<0000> <ffff>
endcodespacerange
2 beginbfchar
<0100> <AC00>
<0101> <AC01>
endbfchar";
        let map = parse_to_unicode_cmap(cmap).unwrap();
        assert_eq!(map.code_width, 2);
        // Decode 2-byte codes
        let result = map.decode(&[0x01, 0x00, 0x01, 0x01]);
        assert_eq!(result, "\u{AC00}\u{AC01}");
    }
}
