//! PDF standard encoding tables and Adobe Glyph List.
//!
//! Provides character code → Unicode mappings for WinAnsiEncoding,
//! MacRomanEncoding, and StandardEncoding, plus glyph name → Unicode
//! lookup via a subset of the Adobe Glyph List (AGL).

use std::collections::HashMap;

/// Supported PDF base encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BaseEncoding {
    WinAnsi,
    MacRoman,
    Standard,
}

impl BaseEncoding {
    /// Parse an encoding name from a PDF `/BaseEncoding` or `/Encoding` value.
    pub(crate) fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"WinAnsiEncoding" => Some(Self::WinAnsi),
            b"MacRomanEncoding" => Some(Self::MacRoman),
            b"StandardEncoding" => Some(Self::Standard),
            _ => None,
        }
    }

    /// Look up a character code in this encoding, returning the Unicode char.
    pub(crate) fn decode_char(&self, code: u8) -> Option<char> {
        match self {
            Self::WinAnsi => WIN_ANSI_HIGH[code as usize],
            Self::MacRoman => MAC_ROMAN_HIGH[code as usize],
            Self::Standard => STANDARD_ENC[code as usize],
        }
    }
}

/// Build a complete encoding map: start from a base encoding,
/// then apply Differences array overrides.
///
/// `differences` is a list of (code, glyph_name) pairs from the PDF
/// `/Differences` array.
pub(crate) fn build_encoding_map(
    base: Option<BaseEncoding>,
    differences: &[(u8, String)],
) -> HashMap<u8, char> {
    let mut map = HashMap::new();

    // Populate from base encoding
    let base = base.unwrap_or(BaseEncoding::Standard);
    for code in 0u16..=255 {
        if let Some(ch) = base.decode_char(code as u8) {
            map.insert(code as u8, ch);
        }
    }

    // Apply differences overrides
    for (code, glyph_name) in differences {
        if let Some(ch) = glyph_name_to_unicode(glyph_name) {
            map.insert(*code, ch);
        }
    }

    map
}

/// Decode a byte sequence using an encoding map.
pub(crate) fn decode_with_encoding_map(bytes: &[u8], map: &HashMap<u8, char>) -> String {
    let mut result = String::with_capacity(bytes.len());
    for &b in bytes {
        if let Some(&ch) = map.get(&b) {
            result.push(ch);
        }
        // Skip unmapped codes silently
    }
    result
}

// ---------------------------------------------------------------------------
// WinAnsiEncoding (CP1252-based)
// ---------------------------------------------------------------------------

/// WinAnsiEncoding: character code → Unicode.
/// 0x20-0x7F are ASCII (same as code point). 0x80-0x9F are CP1252-specific.
/// 0xA0-0xFF are Latin-1 Supplement (identity).
const WIN_ANSI_HIGH: [Option<char>; 256] = {
    let mut t: [Option<char>; 256] = [None; 256];
    // 0x20-0x7E: ASCII
    t[0x20] = Some(' ');
    t[0x21] = Some('!');
    t[0x22] = Some('"');
    t[0x23] = Some('#');
    t[0x24] = Some('$');
    t[0x25] = Some('%');
    t[0x26] = Some('&');
    t[0x27] = Some('\'');
    t[0x28] = Some('(');
    t[0x29] = Some(')');
    t[0x2A] = Some('*');
    t[0x2B] = Some('+');
    t[0x2C] = Some(',');
    t[0x2D] = Some('-');
    t[0x2E] = Some('.');
    t[0x2F] = Some('/');
    t[0x30] = Some('0');
    t[0x31] = Some('1');
    t[0x32] = Some('2');
    t[0x33] = Some('3');
    t[0x34] = Some('4');
    t[0x35] = Some('5');
    t[0x36] = Some('6');
    t[0x37] = Some('7');
    t[0x38] = Some('8');
    t[0x39] = Some('9');
    t[0x3A] = Some(':');
    t[0x3B] = Some(';');
    t[0x3C] = Some('<');
    t[0x3D] = Some('=');
    t[0x3E] = Some('>');
    t[0x3F] = Some('?');
    t[0x40] = Some('@');
    t[0x41] = Some('A');
    t[0x42] = Some('B');
    t[0x43] = Some('C');
    t[0x44] = Some('D');
    t[0x45] = Some('E');
    t[0x46] = Some('F');
    t[0x47] = Some('G');
    t[0x48] = Some('H');
    t[0x49] = Some('I');
    t[0x4A] = Some('J');
    t[0x4B] = Some('K');
    t[0x4C] = Some('L');
    t[0x4D] = Some('M');
    t[0x4E] = Some('N');
    t[0x4F] = Some('O');
    t[0x50] = Some('P');
    t[0x51] = Some('Q');
    t[0x52] = Some('R');
    t[0x53] = Some('S');
    t[0x54] = Some('T');
    t[0x55] = Some('U');
    t[0x56] = Some('V');
    t[0x57] = Some('W');
    t[0x58] = Some('X');
    t[0x59] = Some('Y');
    t[0x5A] = Some('Z');
    t[0x5B] = Some('[');
    t[0x5C] = Some('\\');
    t[0x5D] = Some(']');
    t[0x5E] = Some('^');
    t[0x5F] = Some('_');
    t[0x60] = Some('`');
    t[0x61] = Some('a');
    t[0x62] = Some('b');
    t[0x63] = Some('c');
    t[0x64] = Some('d');
    t[0x65] = Some('e');
    t[0x66] = Some('f');
    t[0x67] = Some('g');
    t[0x68] = Some('h');
    t[0x69] = Some('i');
    t[0x6A] = Some('j');
    t[0x6B] = Some('k');
    t[0x6C] = Some('l');
    t[0x6D] = Some('m');
    t[0x6E] = Some('n');
    t[0x6F] = Some('o');
    t[0x70] = Some('p');
    t[0x71] = Some('q');
    t[0x72] = Some('r');
    t[0x73] = Some('s');
    t[0x74] = Some('t');
    t[0x75] = Some('u');
    t[0x76] = Some('v');
    t[0x77] = Some('w');
    t[0x78] = Some('x');
    t[0x79] = Some('y');
    t[0x7A] = Some('z');
    t[0x7B] = Some('{');
    t[0x7C] = Some('|');
    t[0x7D] = Some('}');
    t[0x7E] = Some('~');
    // 0x80-0x9F: CP1252 specials
    t[0x80] = Some('\u{20AC}'); // Euro
    t[0x82] = Some('\u{201A}'); // quotesinglbase
    t[0x83] = Some('\u{0192}'); // florin
    t[0x84] = Some('\u{201E}'); // quotedblbase
    t[0x85] = Some('\u{2026}'); // ellipsis
    t[0x86] = Some('\u{2020}'); // dagger
    t[0x87] = Some('\u{2021}'); // daggerdbl
    t[0x88] = Some('\u{02C6}'); // circumflex
    t[0x89] = Some('\u{2030}'); // perthousand
    t[0x8A] = Some('\u{0160}'); // Scaron
    t[0x8B] = Some('\u{2039}'); // guilsinglleft
    t[0x8C] = Some('\u{0152}'); // OE
    t[0x8E] = Some('\u{017D}'); // Zcaron
    t[0x91] = Some('\u{2018}'); // quoteleft
    t[0x92] = Some('\u{2019}'); // quoteright
    t[0x93] = Some('\u{201C}'); // quotedblleft
    t[0x94] = Some('\u{201D}'); // quotedblright
    t[0x95] = Some('\u{2022}'); // bullet
    t[0x96] = Some('\u{2013}'); // endash
    t[0x97] = Some('\u{2014}'); // emdash
    t[0x98] = Some('\u{02DC}'); // tilde
    t[0x99] = Some('\u{2122}'); // trademark
    t[0x9A] = Some('\u{0161}'); // scaron
    t[0x9B] = Some('\u{203A}'); // guilsinglright
    t[0x9C] = Some('\u{0153}'); // oe
    t[0x9E] = Some('\u{017E}'); // zcaron
    t[0x9F] = Some('\u{0178}'); // Ydieresis
                                // 0xA0-0xFF: Latin-1 Supplement (identity mapping)
    t[0xA0] = Some('\u{00A0}');
    t[0xA1] = Some('\u{00A1}');
    t[0xA2] = Some('\u{00A2}');
    t[0xA3] = Some('\u{00A3}');
    t[0xA4] = Some('\u{00A4}');
    t[0xA5] = Some('\u{00A5}');
    t[0xA6] = Some('\u{00A6}');
    t[0xA7] = Some('\u{00A7}');
    t[0xA8] = Some('\u{00A8}');
    t[0xA9] = Some('\u{00A9}');
    t[0xAA] = Some('\u{00AA}');
    t[0xAB] = Some('\u{00AB}');
    t[0xAC] = Some('\u{00AC}');
    t[0xAD] = Some('\u{00AD}');
    t[0xAE] = Some('\u{00AE}');
    t[0xAF] = Some('\u{00AF}');
    t[0xB0] = Some('\u{00B0}');
    t[0xB1] = Some('\u{00B1}');
    t[0xB2] = Some('\u{00B2}');
    t[0xB3] = Some('\u{00B3}');
    t[0xB4] = Some('\u{00B4}');
    t[0xB5] = Some('\u{00B5}');
    t[0xB6] = Some('\u{00B6}');
    t[0xB7] = Some('\u{00B7}');
    t[0xB8] = Some('\u{00B8}');
    t[0xB9] = Some('\u{00B9}');
    t[0xBA] = Some('\u{00BA}');
    t[0xBB] = Some('\u{00BB}');
    t[0xBC] = Some('\u{00BC}');
    t[0xBD] = Some('\u{00BD}');
    t[0xBE] = Some('\u{00BE}');
    t[0xBF] = Some('\u{00BF}');
    t[0xC0] = Some('\u{00C0}');
    t[0xC1] = Some('\u{00C1}');
    t[0xC2] = Some('\u{00C2}');
    t[0xC3] = Some('\u{00C3}');
    t[0xC4] = Some('\u{00C4}');
    t[0xC5] = Some('\u{00C5}');
    t[0xC6] = Some('\u{00C6}');
    t[0xC7] = Some('\u{00C7}');
    t[0xC8] = Some('\u{00C8}');
    t[0xC9] = Some('\u{00C9}');
    t[0xCA] = Some('\u{00CA}');
    t[0xCB] = Some('\u{00CB}');
    t[0xCC] = Some('\u{00CC}');
    t[0xCD] = Some('\u{00CD}');
    t[0xCE] = Some('\u{00CE}');
    t[0xCF] = Some('\u{00CF}');
    t[0xD0] = Some('\u{00D0}');
    t[0xD1] = Some('\u{00D1}');
    t[0xD2] = Some('\u{00D2}');
    t[0xD3] = Some('\u{00D3}');
    t[0xD4] = Some('\u{00D4}');
    t[0xD5] = Some('\u{00D5}');
    t[0xD6] = Some('\u{00D6}');
    t[0xD7] = Some('\u{00D7}');
    t[0xD8] = Some('\u{00D8}');
    t[0xD9] = Some('\u{00D9}');
    t[0xDA] = Some('\u{00DA}');
    t[0xDB] = Some('\u{00DB}');
    t[0xDC] = Some('\u{00DC}');
    t[0xDD] = Some('\u{00DD}');
    t[0xDE] = Some('\u{00DE}');
    t[0xDF] = Some('\u{00DF}');
    t[0xE0] = Some('\u{00E0}');
    t[0xE1] = Some('\u{00E1}');
    t[0xE2] = Some('\u{00E2}');
    t[0xE3] = Some('\u{00E3}');
    t[0xE4] = Some('\u{00E4}');
    t[0xE5] = Some('\u{00E5}');
    t[0xE6] = Some('\u{00E6}');
    t[0xE7] = Some('\u{00E7}');
    t[0xE8] = Some('\u{00E8}');
    t[0xE9] = Some('\u{00E9}');
    t[0xEA] = Some('\u{00EA}');
    t[0xEB] = Some('\u{00EB}');
    t[0xEC] = Some('\u{00EC}');
    t[0xED] = Some('\u{00ED}');
    t[0xEE] = Some('\u{00EE}');
    t[0xEF] = Some('\u{00EF}');
    t[0xF0] = Some('\u{00F0}');
    t[0xF1] = Some('\u{00F1}');
    t[0xF2] = Some('\u{00F2}');
    t[0xF3] = Some('\u{00F3}');
    t[0xF4] = Some('\u{00F4}');
    t[0xF5] = Some('\u{00F5}');
    t[0xF6] = Some('\u{00F6}');
    t[0xF7] = Some('\u{00F7}');
    t[0xF8] = Some('\u{00F8}');
    t[0xF9] = Some('\u{00F9}');
    t[0xFA] = Some('\u{00FA}');
    t[0xFB] = Some('\u{00FB}');
    t[0xFC] = Some('\u{00FC}');
    t[0xFD] = Some('\u{00FD}');
    t[0xFE] = Some('\u{00FE}');
    t[0xFF] = Some('\u{00FF}');
    t
};

// ---------------------------------------------------------------------------
// MacRomanEncoding
// ---------------------------------------------------------------------------

/// MacRomanEncoding: character code → Unicode.
/// 0x20-0x7E are ASCII (same as WinAnsi). 0x80-0xFF differ significantly.
const MAC_ROMAN_HIGH: [Option<char>; 256] = {
    let mut t: [Option<char>; 256] = [None; 256];
    // 0x20-0x7E: ASCII (same as WinAnsi)
    t[0x20] = Some(' ');
    t[0x21] = Some('!');
    t[0x22] = Some('"');
    t[0x23] = Some('#');
    t[0x24] = Some('$');
    t[0x25] = Some('%');
    t[0x26] = Some('&');
    t[0x27] = Some('\'');
    t[0x28] = Some('(');
    t[0x29] = Some(')');
    t[0x2A] = Some('*');
    t[0x2B] = Some('+');
    t[0x2C] = Some(',');
    t[0x2D] = Some('-');
    t[0x2E] = Some('.');
    t[0x2F] = Some('/');
    t[0x30] = Some('0');
    t[0x31] = Some('1');
    t[0x32] = Some('2');
    t[0x33] = Some('3');
    t[0x34] = Some('4');
    t[0x35] = Some('5');
    t[0x36] = Some('6');
    t[0x37] = Some('7');
    t[0x38] = Some('8');
    t[0x39] = Some('9');
    t[0x3A] = Some(':');
    t[0x3B] = Some(';');
    t[0x3C] = Some('<');
    t[0x3D] = Some('=');
    t[0x3E] = Some('>');
    t[0x3F] = Some('?');
    t[0x40] = Some('@');
    t[0x41] = Some('A');
    t[0x42] = Some('B');
    t[0x43] = Some('C');
    t[0x44] = Some('D');
    t[0x45] = Some('E');
    t[0x46] = Some('F');
    t[0x47] = Some('G');
    t[0x48] = Some('H');
    t[0x49] = Some('I');
    t[0x4A] = Some('J');
    t[0x4B] = Some('K');
    t[0x4C] = Some('L');
    t[0x4D] = Some('M');
    t[0x4E] = Some('N');
    t[0x4F] = Some('O');
    t[0x50] = Some('P');
    t[0x51] = Some('Q');
    t[0x52] = Some('R');
    t[0x53] = Some('S');
    t[0x54] = Some('T');
    t[0x55] = Some('U');
    t[0x56] = Some('V');
    t[0x57] = Some('W');
    t[0x58] = Some('X');
    t[0x59] = Some('Y');
    t[0x5A] = Some('Z');
    t[0x5B] = Some('[');
    t[0x5C] = Some('\\');
    t[0x5D] = Some(']');
    t[0x5E] = Some('^');
    t[0x5F] = Some('_');
    t[0x60] = Some('`');
    t[0x61] = Some('a');
    t[0x62] = Some('b');
    t[0x63] = Some('c');
    t[0x64] = Some('d');
    t[0x65] = Some('e');
    t[0x66] = Some('f');
    t[0x67] = Some('g');
    t[0x68] = Some('h');
    t[0x69] = Some('i');
    t[0x6A] = Some('j');
    t[0x6B] = Some('k');
    t[0x6C] = Some('l');
    t[0x6D] = Some('m');
    t[0x6E] = Some('n');
    t[0x6F] = Some('o');
    t[0x70] = Some('p');
    t[0x71] = Some('q');
    t[0x72] = Some('r');
    t[0x73] = Some('s');
    t[0x74] = Some('t');
    t[0x75] = Some('u');
    t[0x76] = Some('v');
    t[0x77] = Some('w');
    t[0x78] = Some('x');
    t[0x79] = Some('y');
    t[0x7A] = Some('z');
    t[0x7B] = Some('{');
    t[0x7C] = Some('|');
    t[0x7D] = Some('}');
    t[0x7E] = Some('~');
    // 0x80-0xFF: Mac-specific
    t[0x80] = Some('\u{00C4}'); // Adieresis
    t[0x81] = Some('\u{00C5}'); // Aring
    t[0x82] = Some('\u{00C7}'); // Ccedilla
    t[0x83] = Some('\u{00C9}'); // Eacute
    t[0x84] = Some('\u{00D1}'); // Ntilde
    t[0x85] = Some('\u{00D6}'); // Odieresis
    t[0x86] = Some('\u{00DC}'); // Udieresis
    t[0x87] = Some('\u{00E1}'); // aacute
    t[0x88] = Some('\u{00E0}'); // agrave
    t[0x89] = Some('\u{00E2}'); // acircumflex
    t[0x8A] = Some('\u{00E4}'); // adieresis
    t[0x8B] = Some('\u{00E3}'); // atilde
    t[0x8C] = Some('\u{00E5}'); // aring
    t[0x8D] = Some('\u{00E7}'); // ccedilla
    t[0x8E] = Some('\u{00E9}'); // eacute
    t[0x8F] = Some('\u{00E8}'); // egrave
    t[0x90] = Some('\u{00EA}'); // ecircumflex
    t[0x91] = Some('\u{00EB}'); // edieresis
    t[0x92] = Some('\u{00ED}'); // iacute
    t[0x93] = Some('\u{00EC}'); // igrave
    t[0x94] = Some('\u{00EE}'); // icircumflex
    t[0x95] = Some('\u{00EF}'); // idieresis
    t[0x96] = Some('\u{00F1}'); // ntilde
    t[0x97] = Some('\u{00F3}'); // oacute
    t[0x98] = Some('\u{00F2}'); // ograve
    t[0x99] = Some('\u{00F4}'); // ocircumflex
    t[0x9A] = Some('\u{00F6}'); // odieresis
    t[0x9B] = Some('\u{00F5}'); // otilde
    t[0x9C] = Some('\u{00FA}'); // uacute
    t[0x9D] = Some('\u{00F9}'); // ugrave
    t[0x9E] = Some('\u{00FB}'); // ucircumflex
    t[0x9F] = Some('\u{00FC}'); // udieresis
    t[0xA0] = Some('\u{2020}'); // dagger
    t[0xA1] = Some('\u{00B0}'); // degree
    t[0xA2] = Some('\u{00A2}'); // cent
    t[0xA3] = Some('\u{00A3}'); // sterling
    t[0xA4] = Some('\u{00A7}'); // section
    t[0xA5] = Some('\u{2022}'); // bullet
    t[0xA6] = Some('\u{00B6}'); // paragraph
    t[0xA7] = Some('\u{00DF}'); // germandbls
    t[0xA8] = Some('\u{00AE}'); // registered
    t[0xA9] = Some('\u{00A9}'); // copyright
    t[0xAA] = Some('\u{2122}'); // trademark
    t[0xAB] = Some('\u{00B4}'); // acute
    t[0xAC] = Some('\u{00A8}'); // dieresis
                                // 0xAD undefined
    t[0xAE] = Some('\u{00C6}'); // AE
    t[0xAF] = Some('\u{00D8}'); // Oslash
                                // 0xB0 undefined
    t[0xB1] = Some('\u{00B1}'); // plusminus
                                // 0xB2, 0xB3 undefined
    t[0xB4] = Some('\u{00A5}'); // yen
    t[0xB5] = Some('\u{00B5}'); // mu
                                // 0xB6-0xBA undefined
    t[0xBB] = Some('\u{00AA}'); // ordfeminine
    t[0xBC] = Some('\u{00BA}'); // ordmasculine
                                // 0xBD undefined
    t[0xBE] = Some('\u{00E6}'); // ae
    t[0xBF] = Some('\u{00F8}'); // oslash
    t[0xC0] = Some('\u{00BF}'); // questiondown
    t[0xC1] = Some('\u{00A1}'); // exclamdown
    t[0xC2] = Some('\u{00AC}'); // logicalnot
                                // 0xC3 undefined
    t[0xC4] = Some('\u{0192}'); // florin
                                // 0xC5, 0xC6 undefined
    t[0xC7] = Some('\u{00AB}'); // guillemotleft
    t[0xC8] = Some('\u{00BB}'); // guillemotright
    t[0xC9] = Some('\u{2026}'); // ellipsis
    t[0xCA] = Some('\u{00A0}'); // nbspace
    t[0xCB] = Some('\u{00C0}'); // Agrave
    t[0xCC] = Some('\u{00C3}'); // Atilde
    t[0xCD] = Some('\u{00D5}'); // Otilde
    t[0xCE] = Some('\u{0152}'); // OE
    t[0xCF] = Some('\u{0153}'); // oe
    t[0xD0] = Some('\u{2013}'); // endash
    t[0xD1] = Some('\u{2014}'); // emdash
    t[0xD2] = Some('\u{201C}'); // quotedblleft
    t[0xD3] = Some('\u{201D}'); // quotedblright
    t[0xD4] = Some('\u{2018}'); // quoteleft
    t[0xD5] = Some('\u{2019}'); // quoteright
    t[0xD6] = Some('\u{00F7}'); // divide
                                // 0xD7 undefined
    t[0xD8] = Some('\u{00FF}'); // ydieresis
    t[0xD9] = Some('\u{0178}'); // Ydieresis
    t[0xDA] = Some('\u{2044}'); // fraction
    t[0xDB] = Some('\u{00A4}'); // currency
    t[0xDC] = Some('\u{2039}'); // guilsinglleft
    t[0xDD] = Some('\u{203A}'); // guilsinglright
    t[0xDE] = Some('\u{FB01}'); // fi
    t[0xDF] = Some('\u{FB02}'); // fl
    t[0xE0] = Some('\u{2021}'); // daggerdbl
    t[0xE1] = Some('\u{00B7}'); // periodcentered
    t[0xE2] = Some('\u{201A}'); // quotesinglbase
    t[0xE3] = Some('\u{201E}'); // quotedblbase
    t[0xE4] = Some('\u{2030}'); // perthousand
    t[0xE5] = Some('\u{00C2}'); // Acircumflex
    t[0xE6] = Some('\u{00CA}'); // Ecircumflex
    t[0xE7] = Some('\u{00C1}'); // Aacute
    t[0xE8] = Some('\u{00CB}'); // Edieresis
    t[0xE9] = Some('\u{00C8}'); // Egrave
    t[0xEA] = Some('\u{00CD}'); // Iacute
    t[0xEB] = Some('\u{00CE}'); // Icircumflex
    t[0xEC] = Some('\u{00CF}'); // Idieresis
    t[0xED] = Some('\u{00CC}'); // Igrave
    t[0xEE] = Some('\u{00D3}'); // Oacute
    t[0xEF] = Some('\u{00D4}'); // Ocircumflex
                                // 0xF0 undefined
    t[0xF1] = Some('\u{00D2}'); // Ograve
    t[0xF2] = Some('\u{00DA}'); // Uacute
    t[0xF3] = Some('\u{00DB}'); // Ucircumflex
    t[0xF4] = Some('\u{00D9}'); // Ugrave
    t[0xF5] = Some('\u{0131}'); // dotlessi
    t[0xF6] = Some('\u{02C6}'); // circumflex
    t[0xF7] = Some('\u{02DC}'); // tilde
    t[0xF8] = Some('\u{00AF}'); // macron
    t[0xF9] = Some('\u{02D8}'); // breve
    t[0xFA] = Some('\u{02D9}'); // dotaccent
    t[0xFB] = Some('\u{02DA}'); // ring
    t[0xFC] = Some('\u{00B8}'); // cedilla
    t[0xFD] = Some('\u{02DD}'); // hungarumlaut
    t[0xFE] = Some('\u{02DB}'); // ogonek
    t[0xFF] = Some('\u{02C7}'); // caron
    t
};

// ---------------------------------------------------------------------------
// StandardEncoding (PostScript)
// ---------------------------------------------------------------------------

/// StandardEncoding: character code → Unicode.
/// Many codes are undefined. Notably, 0x27 = quoteright (U+2019),
/// 0x60 = quoteleft (U+2018), unlike WinAnsi.
const STANDARD_ENC: [Option<char>; 256] = {
    let mut t: [Option<char>; 256] = [None; 256];
    // 0x20-0x7E: mostly ASCII but with two key differences
    t[0x20] = Some(' ');
    t[0x21] = Some('!');
    t[0x22] = Some('"');
    t[0x23] = Some('#');
    t[0x24] = Some('$');
    t[0x25] = Some('%');
    t[0x26] = Some('&');
    t[0x27] = Some('\u{2019}'); // quoteright (NOT apostrophe!)
    t[0x28] = Some('(');
    t[0x29] = Some(')');
    t[0x2A] = Some('*');
    t[0x2B] = Some('+');
    t[0x2C] = Some(',');
    t[0x2D] = Some('-');
    t[0x2E] = Some('.');
    t[0x2F] = Some('/');
    t[0x30] = Some('0');
    t[0x31] = Some('1');
    t[0x32] = Some('2');
    t[0x33] = Some('3');
    t[0x34] = Some('4');
    t[0x35] = Some('5');
    t[0x36] = Some('6');
    t[0x37] = Some('7');
    t[0x38] = Some('8');
    t[0x39] = Some('9');
    t[0x3A] = Some(':');
    t[0x3B] = Some(';');
    t[0x3C] = Some('<');
    t[0x3D] = Some('=');
    t[0x3E] = Some('>');
    t[0x3F] = Some('?');
    t[0x40] = Some('@');
    t[0x41] = Some('A');
    t[0x42] = Some('B');
    t[0x43] = Some('C');
    t[0x44] = Some('D');
    t[0x45] = Some('E');
    t[0x46] = Some('F');
    t[0x47] = Some('G');
    t[0x48] = Some('H');
    t[0x49] = Some('I');
    t[0x4A] = Some('J');
    t[0x4B] = Some('K');
    t[0x4C] = Some('L');
    t[0x4D] = Some('M');
    t[0x4E] = Some('N');
    t[0x4F] = Some('O');
    t[0x50] = Some('P');
    t[0x51] = Some('Q');
    t[0x52] = Some('R');
    t[0x53] = Some('S');
    t[0x54] = Some('T');
    t[0x55] = Some('U');
    t[0x56] = Some('V');
    t[0x57] = Some('W');
    t[0x58] = Some('X');
    t[0x59] = Some('Y');
    t[0x5A] = Some('Z');
    t[0x5B] = Some('[');
    t[0x5C] = Some('\\');
    t[0x5D] = Some(']');
    t[0x5E] = Some('^');
    t[0x5F] = Some('_');
    t[0x60] = Some('\u{2018}'); // quoteleft (NOT grave!)
    t[0x61] = Some('a');
    t[0x62] = Some('b');
    t[0x63] = Some('c');
    t[0x64] = Some('d');
    t[0x65] = Some('e');
    t[0x66] = Some('f');
    t[0x67] = Some('g');
    t[0x68] = Some('h');
    t[0x69] = Some('i');
    t[0x6A] = Some('j');
    t[0x6B] = Some('k');
    t[0x6C] = Some('l');
    t[0x6D] = Some('m');
    t[0x6E] = Some('n');
    t[0x6F] = Some('o');
    t[0x70] = Some('p');
    t[0x71] = Some('q');
    t[0x72] = Some('r');
    t[0x73] = Some('s');
    t[0x74] = Some('t');
    t[0x75] = Some('u');
    t[0x76] = Some('v');
    t[0x77] = Some('w');
    t[0x78] = Some('x');
    t[0x79] = Some('y');
    t[0x7A] = Some('z');
    t[0x7B] = Some('{');
    t[0x7C] = Some('|');
    t[0x7D] = Some('}');
    t[0x7E] = Some('~');
    // 0xA1-0xFF: scattered definitions
    t[0xA1] = Some('\u{00A1}'); // exclamdown
    t[0xA2] = Some('\u{00A2}'); // cent
    t[0xA3] = Some('\u{00A3}'); // sterling
    t[0xA4] = Some('\u{2044}'); // fraction
    t[0xA5] = Some('\u{00A5}'); // yen
    t[0xA6] = Some('\u{0192}'); // florin
    t[0xA7] = Some('\u{00A7}'); // section
    t[0xA8] = Some('\u{00A4}'); // currency
    t[0xA9] = Some('\''); // quotesingle
    t[0xAA] = Some('\u{201C}'); // quotedblleft
    t[0xAB] = Some('\u{00AB}'); // guillemotleft
    t[0xAC] = Some('\u{2039}'); // guilsinglleft
    t[0xAD] = Some('\u{203A}'); // guilsinglright
    t[0xAE] = Some('\u{FB01}'); // fi
    t[0xAF] = Some('\u{FB02}'); // fl
    t[0xB1] = Some('\u{2013}'); // endash
    t[0xB2] = Some('\u{2020}'); // dagger
    t[0xB3] = Some('\u{2021}'); // daggerdbl
    t[0xB4] = Some('\u{00B7}'); // periodcentered
    t[0xB6] = Some('\u{00B6}'); // paragraph
    t[0xB7] = Some('\u{2022}'); // bullet
    t[0xB8] = Some('\u{201A}'); // quotesinglbase
    t[0xB9] = Some('\u{201E}'); // quotedblbase
    t[0xBA] = Some('\u{201D}'); // quotedblright
    t[0xBB] = Some('\u{00BB}'); // guillemotright
    t[0xBC] = Some('\u{2026}'); // ellipsis
    t[0xBD] = Some('\u{2030}'); // perthousand
    t[0xBF] = Some('\u{00BF}'); // questiondown
    t[0xC1] = Some('\u{0060}'); // grave
    t[0xC2] = Some('\u{00B4}'); // acute
    t[0xC3] = Some('\u{02C6}'); // circumflex
    t[0xC4] = Some('\u{02DC}'); // tilde
    t[0xC5] = Some('\u{00AF}'); // macron
    t[0xC6] = Some('\u{02D8}'); // breve
    t[0xC7] = Some('\u{02D9}'); // dotaccent
    t[0xC8] = Some('\u{00A8}'); // dieresis
    t[0xCA] = Some('\u{02DA}'); // ring
    t[0xCB] = Some('\u{00B8}'); // cedilla
    t[0xCD] = Some('\u{02DD}'); // hungarumlaut
    t[0xCE] = Some('\u{02DB}'); // ogonek
    t[0xCF] = Some('\u{02C7}'); // caron
    t[0xD0] = Some('\u{2014}'); // emdash
    t[0xE1] = Some('\u{00C6}'); // AE
    t[0xE3] = Some('\u{00AA}'); // ordfeminine
    t[0xE8] = Some('\u{0141}'); // Lslash
    t[0xE9] = Some('\u{00D8}'); // Oslash
    t[0xEA] = Some('\u{0152}'); // OE
    t[0xEB] = Some('\u{00BA}'); // ordmasculine
    t[0xF1] = Some('\u{00E6}'); // ae
    t[0xF5] = Some('\u{0131}'); // dotlessi
    t[0xF8] = Some('\u{0142}'); // lslash
    t[0xF9] = Some('\u{00F8}'); // oslash
    t[0xFA] = Some('\u{0153}'); // oe
    t[0xFB] = Some('\u{00DF}'); // germandbls
    t
};

// ---------------------------------------------------------------------------
// Adobe Glyph List (AGL) — glyph name → Unicode
// ---------------------------------------------------------------------------

/// Look up a glyph name in the Adobe Glyph List, returning its Unicode character.
///
/// Handles both the static AGL table and the algorithmic `uni`/`u` naming
/// conventions per the AGL specification.
pub(crate) fn glyph_name_to_unicode(name: &str) -> Option<char> {
    // Handle "uniXXXX" convention (exactly 4 hex digits)
    if let Some(hex) = name.strip_prefix("uni") {
        if hex.len() == 4 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                return char::from_u32(cp);
            }
        }
    }

    // Handle "uXXXX" to "uXXXXXX" convention (4-6 hex digits)
    if let Some(hex) = name.strip_prefix("u") {
        if (4..=6).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                return char::from_u32(cp);
            }
        }
    }

    // Static AGL lookup
    match name {
        "A" => Some('A'),
        "AE" => Some('\u{00C6}'),
        "Aacute" => Some('\u{00C1}'),
        "Acircumflex" => Some('\u{00C2}'),
        "Adieresis" => Some('\u{00C4}'),
        "Agrave" => Some('\u{00C0}'),
        "Aring" => Some('\u{00C5}'),
        "Atilde" => Some('\u{00C3}'),
        "B" => Some('B'),
        "C" => Some('C'),
        "Ccedilla" => Some('\u{00C7}'),
        "D" => Some('D'),
        "E" => Some('E'),
        "Eacute" => Some('\u{00C9}'),
        "Ecircumflex" => Some('\u{00CA}'),
        "Edieresis" => Some('\u{00CB}'),
        "Egrave" => Some('\u{00C8}'),
        "Eth" => Some('\u{00D0}'),
        "Euro" => Some('\u{20AC}'),
        "F" => Some('F'),
        "G" => Some('G'),
        "H" => Some('H'),
        "I" => Some('I'),
        "Iacute" => Some('\u{00CD}'),
        "Icircumflex" => Some('\u{00CE}'),
        "Idieresis" => Some('\u{00CF}'),
        "Igrave" => Some('\u{00CC}'),
        "J" => Some('J'),
        "K" => Some('K'),
        "L" => Some('L'),
        "Lslash" => Some('\u{0141}'),
        "M" => Some('M'),
        "N" => Some('N'),
        "Ntilde" => Some('\u{00D1}'),
        "O" => Some('O'),
        "OE" => Some('\u{0152}'),
        "Oacute" => Some('\u{00D3}'),
        "Ocircumflex" => Some('\u{00D4}'),
        "Odieresis" => Some('\u{00D6}'),
        "Ograve" => Some('\u{00D2}'),
        "Oslash" => Some('\u{00D8}'),
        "Otilde" => Some('\u{00D5}'),
        "P" => Some('P'),
        "Q" => Some('Q'),
        "R" => Some('R'),
        "S" => Some('S'),
        "Scaron" => Some('\u{0160}'),
        "T" => Some('T'),
        "Thorn" => Some('\u{00DE}'),
        "U" => Some('U'),
        "Uacute" => Some('\u{00DA}'),
        "Ucircumflex" => Some('\u{00DB}'),
        "Udieresis" => Some('\u{00DC}'),
        "Ugrave" => Some('\u{00D9}'),
        "V" => Some('V'),
        "W" => Some('W'),
        "X" => Some('X'),
        "Y" => Some('Y'),
        "Yacute" => Some('\u{00DD}'),
        "Ydieresis" => Some('\u{0178}'),
        "Z" => Some('Z'),
        "Zcaron" => Some('\u{017D}'),
        "a" => Some('a'),
        "aacute" => Some('\u{00E1}'),
        "acircumflex" => Some('\u{00E2}'),
        "acute" => Some('\u{00B4}'),
        "adieresis" => Some('\u{00E4}'),
        "ae" => Some('\u{00E6}'),
        "agrave" => Some('\u{00E0}'),
        "ampersand" => Some('&'),
        "aring" => Some('\u{00E5}'),
        "asciicircum" => Some('^'),
        "asciitilde" => Some('~'),
        "asterisk" => Some('*'),
        "at" => Some('@'),
        "atilde" => Some('\u{00E3}'),
        "b" => Some('b'),
        "backslash" => Some('\\'),
        "bar" => Some('|'),
        "braceleft" => Some('{'),
        "braceright" => Some('}'),
        "bracketleft" => Some('['),
        "bracketright" => Some(']'),
        "breve" => Some('\u{02D8}'),
        "brokenbar" => Some('\u{00A6}'),
        "bullet" => Some('\u{2022}'),
        "c" => Some('c'),
        "caron" => Some('\u{02C7}'),
        "ccedilla" => Some('\u{00E7}'),
        "cedilla" => Some('\u{00B8}'),
        "cent" => Some('\u{00A2}'),
        "circumflex" => Some('\u{02C6}'),
        "colon" => Some(':'),
        "comma" => Some(','),
        "copyright" => Some('\u{00A9}'),
        "currency" => Some('\u{00A4}'),
        "d" => Some('d'),
        "dagger" => Some('\u{2020}'),
        "daggerdbl" => Some('\u{2021}'),
        "degree" => Some('\u{00B0}'),
        "dieresis" => Some('\u{00A8}'),
        "divide" => Some('\u{00F7}'),
        "dollar" => Some('$'),
        "dotaccent" => Some('\u{02D9}'),
        "dotlessi" => Some('\u{0131}'),
        "e" => Some('e'),
        "eacute" => Some('\u{00E9}'),
        "ecircumflex" => Some('\u{00EA}'),
        "edieresis" => Some('\u{00EB}'),
        "egrave" => Some('\u{00E8}'),
        "eight" => Some('8'),
        "ellipsis" => Some('\u{2026}'),
        "emdash" => Some('\u{2014}'),
        "endash" => Some('\u{2013}'),
        "equal" => Some('='),
        "eth" => Some('\u{00F0}'),
        "exclam" => Some('!'),
        "exclamdown" => Some('\u{00A1}'),
        "f" => Some('f'),
        "fi" => Some('\u{FB01}'),
        "five" => Some('5'),
        "fl" => Some('\u{FB02}'),
        "florin" => Some('\u{0192}'),
        "four" => Some('4'),
        "fraction" => Some('\u{2044}'),
        "g" => Some('g'),
        "germandbls" => Some('\u{00DF}'),
        "grave" => Some('`'),
        "greater" => Some('>'),
        "guillemotleft" => Some('\u{00AB}'),
        "guillemotright" => Some('\u{00BB}'),
        "guilsinglleft" => Some('\u{2039}'),
        "guilsinglright" => Some('\u{203A}'),
        "h" => Some('h'),
        "hungarumlaut" => Some('\u{02DD}'),
        "hyphen" => Some('-'),
        "i" => Some('i'),
        "iacute" => Some('\u{00ED}'),
        "icircumflex" => Some('\u{00EE}'),
        "idieresis" => Some('\u{00EF}'),
        "igrave" => Some('\u{00EC}'),
        "j" => Some('j'),
        "k" => Some('k'),
        "l" => Some('l'),
        "less" => Some('<'),
        "logicalnot" => Some('\u{00AC}'),
        "lslash" => Some('\u{0142}'),
        "m" => Some('m'),
        "macron" => Some('\u{00AF}'),
        "minus" => Some('\u{2212}'),
        "mu" => Some('\u{00B5}'),
        "multiply" => Some('\u{00D7}'),
        "n" => Some('n'),
        "nbspace" => Some('\u{00A0}'),
        "nine" => Some('9'),
        "ntilde" => Some('\u{00F1}'),
        "numbersign" => Some('#'),
        "o" => Some('o'),
        "oacute" => Some('\u{00F3}'),
        "ocircumflex" => Some('\u{00F4}'),
        "odieresis" => Some('\u{00F6}'),
        "oe" => Some('\u{0153}'),
        "ograve" => Some('\u{00F2}'),
        "ogonek" => Some('\u{02DB}'),
        "one" => Some('1'),
        "onehalf" => Some('\u{00BD}'),
        "onequarter" => Some('\u{00BC}'),
        "onesuperior" => Some('\u{00B9}'),
        "ordfeminine" => Some('\u{00AA}'),
        "ordmasculine" => Some('\u{00BA}'),
        "oslash" => Some('\u{00F8}'),
        "otilde" => Some('\u{00F5}'),
        "p" => Some('p'),
        "paragraph" => Some('\u{00B6}'),
        "parenleft" => Some('('),
        "parenright" => Some(')'),
        "percent" => Some('%'),
        "period" => Some('.'),
        "periodcentered" => Some('\u{00B7}'),
        "perthousand" => Some('\u{2030}'),
        "plus" => Some('+'),
        "plusminus" => Some('\u{00B1}'),
        "q" => Some('q'),
        "question" => Some('?'),
        "questiondown" => Some('\u{00BF}'),
        "quotedbl" => Some('"'),
        "quotedblbase" => Some('\u{201E}'),
        "quotedblleft" => Some('\u{201C}'),
        "quotedblright" => Some('\u{201D}'),
        "quoteleft" => Some('\u{2018}'),
        "quoteright" => Some('\u{2019}'),
        "quotesinglbase" => Some('\u{201A}'),
        "quotesingle" => Some('\''),
        "r" => Some('r'),
        "registered" => Some('\u{00AE}'),
        "ring" => Some('\u{02DA}'),
        "s" => Some('s'),
        "scaron" => Some('\u{0161}'),
        "section" => Some('\u{00A7}'),
        "semicolon" => Some(';'),
        "seven" => Some('7'),
        "sfthyphen" => Some('\u{00AD}'),
        "six" => Some('6'),
        "slash" => Some('/'),
        "space" => Some(' '),
        "sterling" => Some('\u{00A3}'),
        "t" => Some('t'),
        "thorn" => Some('\u{00FE}'),
        "three" => Some('3'),
        "threequarters" => Some('\u{00BE}'),
        "threesuperior" => Some('\u{00B3}'),
        "tilde" => Some('\u{02DC}'),
        "trademark" => Some('\u{2122}'),
        "two" => Some('2'),
        "twosuperior" => Some('\u{00B2}'),
        "u" => Some('u'),
        "uacute" => Some('\u{00FA}'),
        "ucircumflex" => Some('\u{00FB}'),
        "udieresis" => Some('\u{00FC}'),
        "ugrave" => Some('\u{00F9}'),
        "underscore" => Some('_'),
        "v" => Some('v'),
        "w" => Some('w'),
        "x" => Some('x'),
        "y" => Some('y'),
        "yacute" => Some('\u{00FD}'),
        "ydieresis" => Some('\u{00FF}'),
        "yen" => Some('\u{00A5}'),
        "z" => Some('z'),
        "zcaron" => Some('\u{017E}'),
        "zero" => Some('0'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_win_ansi_ascii() {
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0x41), Some('A'));
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0x7A), Some('z'));
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0x20), Some(' '));
    }

    #[test]
    fn test_win_ansi_cp1252_specials() {
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0x80), Some('\u{20AC}')); // Euro
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0x93), Some('\u{201C}')); // left double quote
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0x94), Some('\u{201D}')); // right double quote
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0x96), Some('\u{2013}')); // en dash
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0x97), Some('\u{2014}')); // em dash
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0x81), None); // undefined
    }

    #[test]
    fn test_win_ansi_latin1_supplement() {
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0xE9), Some('\u{00E9}')); // eacute
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0xFC), Some('\u{00FC}')); // udieresis
        assert_eq!(BaseEncoding::WinAnsi.decode_char(0xFF), Some('\u{00FF}')); // ydieresis
    }

    #[test]
    fn test_mac_roman_high() {
        assert_eq!(BaseEncoding::MacRoman.decode_char(0x80), Some('\u{00C4}')); // Adieresis
        assert_eq!(BaseEncoding::MacRoman.decode_char(0x83), Some('\u{00C9}')); // Eacute
        assert_eq!(BaseEncoding::MacRoman.decode_char(0xDE), Some('\u{FB01}')); // fi ligature
        assert_eq!(BaseEncoding::MacRoman.decode_char(0xDF), Some('\u{FB02}')); // fl ligature
        assert_eq!(BaseEncoding::MacRoman.decode_char(0xD7), None); // undefined
    }

    #[test]
    fn test_standard_encoding_special_quotes() {
        // Key differences from WinAnsi
        assert_eq!(BaseEncoding::Standard.decode_char(0x27), Some('\u{2019}')); // quoteright
        assert_eq!(BaseEncoding::Standard.decode_char(0x60), Some('\u{2018}')); // quoteleft
                                                                                // Standard has fi/fl ligatures
        assert_eq!(BaseEncoding::Standard.decode_char(0xAE), Some('\u{FB01}')); // fi
        assert_eq!(BaseEncoding::Standard.decode_char(0xAF), Some('\u{FB02}')); // fl
    }

    #[test]
    fn test_glyph_name_to_unicode_basic() {
        assert_eq!(glyph_name_to_unicode("space"), Some(' '));
        assert_eq!(glyph_name_to_unicode("A"), Some('A'));
        assert_eq!(glyph_name_to_unicode("Aacute"), Some('\u{00C1}'));
        assert_eq!(glyph_name_to_unicode("Euro"), Some('\u{20AC}'));
        assert_eq!(glyph_name_to_unicode("fi"), Some('\u{FB01}'));
        assert_eq!(glyph_name_to_unicode("emdash"), Some('\u{2014}'));
    }

    #[test]
    fn test_glyph_name_to_unicode_algorithmic() {
        // uniXXXX convention
        assert_eq!(glyph_name_to_unicode("uni0041"), Some('A'));
        assert_eq!(glyph_name_to_unicode("uni20AC"), Some('\u{20AC}'));
        // uXXXX convention
        assert_eq!(glyph_name_to_unicode("u0041"), Some('A'));
        assert_eq!(glyph_name_to_unicode("u20AC"), Some('\u{20AC}'));
        // Unknown name
        assert_eq!(glyph_name_to_unicode("nonexistentglyph"), None);
    }

    #[test]
    fn test_base_encoding_from_name() {
        assert_eq!(
            BaseEncoding::from_name(b"WinAnsiEncoding"),
            Some(BaseEncoding::WinAnsi)
        );
        assert_eq!(
            BaseEncoding::from_name(b"MacRomanEncoding"),
            Some(BaseEncoding::MacRoman)
        );
        assert_eq!(
            BaseEncoding::from_name(b"StandardEncoding"),
            Some(BaseEncoding::Standard)
        );
        assert_eq!(BaseEncoding::from_name(b"UnknownEncoding"), None);
    }

    #[test]
    fn test_build_encoding_map_with_differences() {
        let diffs = vec![
            (0x41, "Aacute".to_string()),  // Override A → Á
            (0x42, "uni00C7".to_string()), // Override B → Ç via uni convention
        ];
        let map = build_encoding_map(Some(BaseEncoding::WinAnsi), &diffs);

        assert_eq!(map.get(&0x41), Some(&'\u{00C1}')); // Á (overridden)
        assert_eq!(map.get(&0x42), Some(&'\u{00C7}')); // Ç (overridden)
        assert_eq!(map.get(&0x43), Some(&'C')); // C (from base)
        assert_eq!(map.get(&0x20), Some(&' ')); // space (from base)
    }

    #[test]
    fn test_decode_with_encoding_map() {
        let map = build_encoding_map(Some(BaseEncoding::WinAnsi), &[]);
        let result = decode_with_encoding_map(&[0x48, 0x65, 0x6C, 0x6C, 0x6F], &map);
        assert_eq!(result, "Hello");
    }

    #[test]
    fn test_decode_with_encoding_map_cp1252() {
        let map = build_encoding_map(Some(BaseEncoding::WinAnsi), &[]);
        // Smart quotes and em dash
        let result = decode_with_encoding_map(&[0x93, 0x48, 0x69, 0x94, 0x97], &map);
        assert_eq!(result, "\u{201C}Hi\u{201D}\u{2014}");
    }
}
