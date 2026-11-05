//! PDF standard encoding tables and Adobe Glyph List.
//!
//! Provides character code → Unicode mappings for WinAnsiEncoding,
//! MacRomanEncoding, and StandardEncoding, plus glyph name → Unicode
//! lookup via the Adobe Glyph List (AGL).

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

/// What each code of a simple font stands for, a ligature's letters included.
pub(crate) type EncodingMap = HashMap<u8, String>;

/// Build a complete encoding map: start from a base encoding,
/// then apply Differences array overrides.
///
/// `differences` is a list of (code, glyph_name) pairs from the PDF
/// `/Differences` array.
pub(crate) fn build_encoding_map(
    base: Option<BaseEncoding>,
    differences: &[(u8, String)],
) -> EncodingMap {
    let mut map = HashMap::new();

    // Populate from base encoding
    let base = base.unwrap_or(BaseEncoding::Standard);
    for code in 0u16..=255 {
        if let Some(ch) = base.decode_char(code as u8) {
            map.insert(code as u8, ch.to_string());
        }
    }

    apply_differences(&mut map, differences);
    map
}

/// Lays `/Differences` over an encoding map.
///
/// A name that gives no text (`g12`, a producer's own names such as dvips' `/BT` at code
/// 65) leaves the code as the base encoding has it. ISO 32000-1 §9.6.6.1 says the code is
/// that glyph, but nothing names the glyph's character, and readers fall back to the code:
/// pdf.js reads an unmapped code as the character of its value (`Font._charToGlyph`). For
/// TeX's bitmap fonts, which name their glyphs this way and carry no `ToUnicode`, that
/// fallback is the text — measured on an olmOCR-Bench page that came out empty without it.
pub(crate) fn apply_differences(map: &mut EncodingMap, differences: &[(u8, String)]) {
    for (code, glyph_name) in differences {
        if let Some(text) = glyph_name_to_text(glyph_name) {
            map.insert(*code, text);
        }
    }
}

/// Decode a byte sequence using an encoding map.
pub(crate) fn decode_with_encoding_map(bytes: &[u8], map: &EncodingMap) -> String {
    decode_with_encoding_map_mapped(bytes, map, &mut None)
}

pub(crate) fn decode_with_encoding_map_mapped(
    bytes: &[u8],
    map: &EncodingMap,
    mapping: &mut Option<Vec<super::decoded_run::SourceText>>,
) -> String {
    if let Some(mapping) = mapping {
        mapping.clear();
    }
    let mut result = String::with_capacity(bytes.len());
    for (i, &b) in bytes.iter().enumerate() {
        super::decoded_run::append(
            &mut result,
            i..i + 1,
            map.get(&b).map_or("", String::as_str),
            mapping,
        );
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
// Adobe Glyph List (AGL) — glyph name → Unicode (`agl_data`, generated)
// ---------------------------------------------------------------------------

/// Look up a glyph name that names one character; a ligature of several (`f_t`) is `None`.
pub(crate) fn glyph_name_to_unicode(name: &str) -> Option<char> {
    single_char(&glyph_name_to_text(name)?)
}

pub(crate) fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

/// Look up a glyph name in the Adobe Glyph List, returning the text it stands for.
///
/// Handles both the static AGL table and the algorithmic `uni`/`u` naming
/// conventions per the AGL specification, including its first step: a suffix
/// after the first period names a variant of the same character (`one.SP`,
/// `a.smcp`, `uni0041.alt`), so it is dropped before the lookup. A name that is
/// nothing but a suffix (`.notdef`) maps to no character. Components joined by
/// underscores (`f_t`) make a ligature of their letters.
pub(crate) fn glyph_name_to_text(name: &str) -> Option<String> {
    let base = match name.find('.') {
        Some(dot) => &name[..dot],
        None => name,
    };
    let text: String = base.split('_').filter_map(agl_component_to_text).collect();
    (!text.is_empty()).then_some(text)
}

/// One underscore-separated component of a glyph name as text.
///
/// A `uni` name of several four-digit groups gives one character per group (the AGL's
/// `uni00660069` → `fi`). A group that is a control character or a surrogate maps the whole
/// component to nothing: producers name glyphs by their index in that form (`uni00000004`),
/// and such a name has always read as no glyph name at all. A single group stays as the AGL
/// specification maps it, control characters included.
fn agl_component_to_text(name: &str) -> Option<String> {
    if let Some(hex) = name.strip_prefix("uni") {
        if hex.len() > 4 && hex.len() % 4 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return (0..hex.len())
                .step_by(4)
                .map(|at| {
                    u32::from_str_radix(&hex[at..at + 4], 16)
                        .ok()
                        .and_then(char::from_u32)
                        .filter(|c| !c.is_control())
                })
                .collect();
        }
    }
    agl_component_to_unicode(name).map(String::from)
}

fn agl_component_to_unicode(name: &str) -> Option<char> {
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

    // The Adobe Glyph List, except the pieces of a stacked delimiter it puts in Adobe's own
    // private-use area.
    if let Some(piece) = stacked_delimiter_piece(name) {
        return Some(piece);
    }
    super::agl_data::AGL
        .binary_search_by(|(n, _)| n.as_bytes().cmp(name.as_bytes()))
        .ok()
        .map(|i| super::agl_data::AGL[i].1)
        .or_else(|| decimal_glyph_name(name))
        .or_else(|| tex_math_extension_glyph(name))
}

/// One piece of a delimiter drawn tall by stacking (`parenlefttp`, `braceex`, …) as the Unicode
/// character made for it.
///
/// The Adobe Glyph List maps these names to Adobe's corporate-use code points
/// (`U+F8EB`–`U+F8FE`), which mean nothing to a reader of the text; Unicode has the pieces as
/// characters of their own (U+239B–U+23AE, "Miscellaneous Technical"), and TeX's glyph list
/// maps them there. The vertical and horizontal arrow extenders map to the line-extension
/// characters of the same block.
fn stacked_delimiter_piece(name: &str) -> Option<char> {
    Some(match name {
        "parenlefttp" => '\u{239B}',
        "parenleftex" => '\u{239C}',
        "parenleftbt" => '\u{239D}',
        "parenrighttp" => '\u{239E}',
        "parenrightex" => '\u{239F}',
        "parenrightbt" => '\u{23A0}',
        "bracketlefttp" => '\u{23A1}',
        "bracketleftex" => '\u{23A2}',
        "bracketleftbt" => '\u{23A3}',
        "bracketrighttp" => '\u{23A4}',
        "bracketrightex" => '\u{23A5}',
        "bracketrightbt" => '\u{23A6}',
        "bracelefttp" => '\u{23A7}',
        "braceleftmid" => '\u{23A8}',
        "braceleftbt" => '\u{23A9}',
        "braceex" => '\u{23AA}',
        "bracerighttp" => '\u{23AB}',
        "bracerightmid" => '\u{23AC}',
        "bracerightbt" => '\u{23AD}',
        "integralex" => '\u{23AE}',
        "arrowhorizex" => '\u{23AF}',
        "arrowvertex" => '\u{23D0}',
        _ => return None,
    })
}

/// A glyph of TeX's math extension fonts (`CMEX10`, `CMEX8`, …) named after the character it
/// draws in a larger size.
///
/// The fonts' own glyph names — `parenleftbig` … `parenleftBigg` for four sizes of a
/// parenthesis, `summationtext` / `summationdisplay` for the two sizes of a sum sign,
/// `hatwide` for an accent over a long base — appear in no glyph list, so a formula's big
/// delimiters and operators were dropped from the text (on twenty benchmark pages the
/// extension font carried 189 of the 197 dropped runs). The pieces that stack up a tall
/// delimiter (`parenlefttp`, `braceex`) are in the Adobe Glyph List already and are not
/// handled here. The n-ary operators map to their n-ary Unicode forms (U+22C0–U+22C3,
/// U+2A00–U+2A06) except the sum, product and integral, which have only one.
fn tex_math_extension_glyph(name: &str) -> Option<char> {
    for size in ["bigg", "Bigg", "big", "Big"] {
        if let Some(base) = name.strip_suffix(size) {
            return match base {
                "parenleft" => Some('('),
                "parenright" => Some(')'),
                "bracketleft" => Some('['),
                "bracketright" => Some(']'),
                "braceleft" => Some('{'),
                "braceright" => Some('}'),
                "angbracketleft" => Some('\u{27E8}'),
                "angbracketright" => Some('\u{27E9}'),
                "floorleft" => Some('\u{230A}'),
                "floorright" => Some('\u{230B}'),
                "ceilingleft" => Some('\u{2308}'),
                "ceilingright" => Some('\u{2309}'),
                "slash" => Some('/'),
                "backslash" => Some('\\'),
                "radical" => Some('\u{221A}'),
                _ => None,
            };
        }
    }
    for size in ["display", "text"] {
        if let Some(base) = name.strip_suffix(size) {
            return match base {
                "summation" => Some('\u{2211}'),
                "product" => Some('\u{220F}'),
                "coproduct" => Some('\u{2210}'),
                "integral" => Some('\u{222B}'),
                "contintegral" => Some('\u{222E}'),
                "union" => Some('\u{22C3}'),
                "intersection" => Some('\u{22C2}'),
                "unionsq" => Some('\u{2A06}'),
                "unionmulti" => Some('\u{2A04}'),
                "logicaland" => Some('\u{22C0}'),
                "logicalor" => Some('\u{22C1}'),
                "circleplus" => Some('\u{2A01}'),
                "circlemultiply" => Some('\u{2A02}'),
                "circledot" => Some('\u{2A00}'),
                _ => None,
            };
        }
    }
    match name {
        "hatwide" | "hatwider" | "hatwidest" => Some('\u{02C6}'),
        "tildewide" | "tildewider" | "tildewidest" => Some('\u{02DC}'),
        _ => None,
    }
}

/// A glyph named by a bare decimal number (`/72`) stands for the character of that value.
///
/// No standard defines the name, but producers that turn bitmap fonts into Type 3 fonts name
/// each glyph after its character code, and MuPDF reads it so
/// (`fz_unicode_from_glyph_name`: a name of only decimal digits is the code point). Control
/// characters stay unnamed, as a `uni` name of one does (`agl_component_to_text`), so a
/// producer that numbers glyphs by index does not put control codes into the text.
fn decimal_glyph_name(name: &str) -> Option<char> {
    if name.is_empty() || name.len() > 7 || !name.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    char::from_u32(name.parse().ok()?).filter(|c| !c.is_control())
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
    fn the_whole_glyph_list_is_known() {
        // Ligatures TeX's fonts name, Greek, and a name from deep in the list.
        assert_eq!(glyph_name_to_unicode("ff"), Some('\u{FB00}'));
        assert_eq!(glyph_name_to_unicode("ffi"), Some('\u{FB03}'));
        assert_eq!(glyph_name_to_unicode("ffl"), Some('\u{FB04}'));
        assert_eq!(glyph_name_to_unicode("alpha"), Some('\u{03B1}'));
        assert_eq!(glyph_name_to_unicode("Gamma"), Some('\u{0393}'));
        assert_eq!(glyph_name_to_unicode("zukatakana"), Some('\u{30BA}'));
        assert_eq!(glyph_name_to_unicode("notaglyphname"), None);
    }

    #[test]
    fn the_glyph_list_is_sorted_for_binary_search() {
        assert!(super::super::agl_data::AGL
            .windows(2)
            .all(|w| w[0].0.as_bytes() < w[1].0.as_bytes()));
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

    /// The glyph names of TeX's math extension fonts, which no glyph list has.
    #[test]
    fn the_names_of_the_math_extension_fonts_are_their_characters() {
        for size in ["big", "Big", "bigg", "Bigg"] {
            assert_eq!(
                glyph_name_to_unicode(&format!("parenleft{size}")),
                Some('(')
            );
            assert_eq!(
                glyph_name_to_unicode(&format!("braceright{size}")),
                Some('}')
            );
            assert_eq!(
                glyph_name_to_unicode(&format!("angbracketleft{size}")),
                Some('\u{27E8}')
            );
            assert_eq!(
                glyph_name_to_unicode(&format!("radical{size}")),
                Some('\u{221A}')
            );
        }
        assert_eq!(glyph_name_to_unicode("summationtext"), Some('\u{2211}'));
        assert_eq!(glyph_name_to_unicode("summationdisplay"), Some('\u{2211}'));
        assert_eq!(glyph_name_to_unicode("integraldisplay"), Some('\u{222B}'));
        assert_eq!(glyph_name_to_unicode("uniondisplay"), Some('\u{22C3}'));
        assert_eq!(glyph_name_to_unicode("circleplustext"), Some('\u{2A01}'));
        assert_eq!(glyph_name_to_unicode("hatwidest"), Some('\u{02C6}'));
        assert_eq!(glyph_name_to_unicode("tildewide"), Some('\u{02DC}'));
        // The Adobe Glyph List's own names keep their meaning, and a suffix alone names nothing.
        assert_eq!(glyph_name_to_unicode("bracelefttp"), Some('\u{23A7}'));
        assert_eq!(glyph_name_to_unicode("big"), None);
        assert_eq!(glyph_name_to_unicode("text"), None);
        assert_eq!(glyph_name_to_unicode("xbig"), None);
        assert_eq!(glyph_name_to_unicode("vextendsingle"), None);
        assert_eq!(glyph_name_to_unicode("bracehtipdownleft"), None);
    }

    /// The pieces of a stacked delimiter are the Unicode characters made for them, not the
    /// Adobe Glyph List's private-use values — and none of the list's other names moves.
    #[test]
    fn the_pieces_of_a_stacked_delimiter_are_unicode_characters() {
        assert_eq!(glyph_name_to_unicode("parenlefttp"), Some('\u{239B}'));
        assert_eq!(glyph_name_to_unicode("parenrightbt"), Some('\u{23A0}'));
        assert_eq!(glyph_name_to_unicode("bracketrightex"), Some('\u{23A5}'));
        assert_eq!(glyph_name_to_unicode("braceex"), Some('\u{23AA}'));
        assert_eq!(glyph_name_to_unicode("bracerightmid"), Some('\u{23AC}'));
        assert_eq!(glyph_name_to_unicode("integralex"), Some('\u{23AE}'));
        // No piece name still reads as a private-use code point.
        for name in [
            "parenlefttp",
            "parenleftex",
            "parenleftbt",
            "parenrighttp",
            "parenrightex",
            "parenrightbt",
            "bracketlefttp",
            "bracketleftex",
            "bracketleftbt",
            "bracketrighttp",
            "bracketrightex",
            "bracketrightbt",
            "bracelefttp",
            "braceleftmid",
            "braceleftbt",
            "braceex",
            "bracerighttp",
            "bracerightmid",
            "bracerightbt",
            "integralex",
        ] {
            let c = glyph_name_to_unicode(name).unwrap();
            assert!(('\u{2320}'..='\u{23FF}').contains(&c), "{name} -> {c:?}");
        }
        assert_eq!(glyph_name_to_unicode("integraltp"), Some('\u{2320}'));
        assert_eq!(glyph_name_to_unicode("parenleft"), Some('('));
    }

    /// A name of only decimal digits is the character of that value (MuPDF's reading);
    /// control characters, values past Unicode and mixed names are not.
    #[test]
    fn a_bare_decimal_glyph_name_is_the_character_of_that_value() {
        assert_eq!(glyph_name_to_unicode("72"), Some('H'));
        assert_eq!(glyph_name_to_unicode("32"), Some(' '));
        assert_eq!(glyph_name_to_unicode("12354"), Some('\u{3042}'));
        assert_eq!(glyph_name_to_unicode("72.alt"), Some('H'));
        assert_eq!(glyph_name_to_unicode("0"), None);
        assert_eq!(glyph_name_to_unicode("7"), None);
        assert_eq!(glyph_name_to_unicode("127"), None);
        assert_eq!(glyph_name_to_unicode("1114112"), None);
        assert_eq!(glyph_name_to_unicode("55296"), None); // a surrogate
        assert_eq!(glyph_name_to_unicode("99999999"), None);
        assert_eq!(glyph_name_to_unicode("72a"), None);
        assert_eq!(glyph_name_to_unicode("-72"), None);
    }

    /// AGL spec step 1: a variant suffix after the first period does not change
    /// the character — lining/old-style figures and small caps are the common case.
    #[test]
    fn test_glyph_name_to_unicode_drops_variant_suffix() {
        assert_eq!(glyph_name_to_unicode("one.SP"), Some('1'));
        assert_eq!(glyph_name_to_unicode("three.OT"), Some('3'));
        assert_eq!(glyph_name_to_unicode("a.smcp"), Some('a'));
        assert_eq!(glyph_name_to_unicode("uni0041.alt"), Some('A'));
        assert_eq!(glyph_name_to_unicode(".notdef"), None);
        assert_eq!(glyph_name_to_unicode("nonexistent.sc"), None);
    }

    #[test]
    fn test_difference_with_variant_suffix_resolves() {
        let map = build_encoding_map(Some(BaseEncoding::WinAnsi), &[(0x13, "one.SP".to_string())]);
        assert_eq!(map.get(&0x13).map(String::as_str), Some("1"));
    }

    #[test]
    fn ligature_names_map_to_their_components() {
        assert_eq!(glyph_name_to_text("T_h").as_deref(), Some("Th"));
        assert_eq!(glyph_name_to_text("f_f_i").as_deref(), Some("ffi"));
        assert_eq!(glyph_name_to_text("s_t.alt").as_deref(), Some("st"));
        assert_eq!(glyph_name_to_text("uni00660069").as_deref(), Some("fi"));
        assert_eq!(glyph_name_to_text("uni0066_uni0074").as_deref(), Some("ft"));
        assert_eq!(glyph_name_to_text("f_nonexistent").as_deref(), Some("f"));
        assert_eq!(glyph_name_to_text("uni0066D800"), None);
        assert_eq!(glyph_name_to_text("uni00000004"), None);
        assert_eq!(glyph_name_to_text("_"), None);
        assert_eq!(glyph_name_to_unicode("f_t"), None);
        let map = build_encoding_map(
            Some(BaseEncoding::WinAnsi),
            &[(0x80, "T_h".to_string()), (0x81, "uni00660069".to_string())],
        );
        assert_eq!(decode_with_encoding_map(&[0x80, b'e', 0x81], &map), "Thefi");
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

        let get = |code| map.get(&code).map(String::as_str);
        assert_eq!(get(0x41), Some("\u{00C1}")); // Á (overridden)
        assert_eq!(get(0x42), Some("\u{00C7}")); // Ç (overridden)
        assert_eq!(get(0x43), Some("C")); // C (from base)
        assert_eq!(get(0x20), Some(" ")); // space (from base)
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
