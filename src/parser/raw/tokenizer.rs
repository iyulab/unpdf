//! PDF object tokenizer/parser.

use crate::error::{Error, Result};
use std::collections::HashMap;

/// A PDF object.
#[derive(Debug, Clone, PartialEq)]
pub enum PdfObject {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<PdfObject>),
    Dict(PdfDict),
    Stream(PdfStream),
    Reference(u32, u16),
}

/// A PDF dictionary.
pub type PdfDict = HashMap<Vec<u8>, PdfObject>;

/// A PDF stream object.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfStream {
    pub dict: PdfDict,
    pub raw_data: Vec<u8>,
}

impl PdfObject {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            PdfObject::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PdfObject::Real(r) => Some(*r),
            PdfObject::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        self.as_f64().map(|f| f as f32)
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            PdfObject::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_str_bytes(&self) -> Option<&[u8]> {
        match self {
            PdfObject::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[PdfObject]> {
        match self {
            PdfObject::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&PdfDict> {
        match self {
            PdfObject::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&PdfStream> {
        match self {
            PdfObject::Stream(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_reference(&self) -> Option<(u32, u16)> {
        match self {
            PdfObject::Reference(n, g) => Some((*n, *g)),
            _ => None,
        }
    }
}

/// Get a value from a PdfDict by key.
pub fn dict_get<'a>(dict: &'a PdfDict, key: &[u8]) -> Option<&'a PdfObject> {
    dict.get(key)
}

// ---------------------------------------------------------------------------
// Tokenizer / parser implementation
// ---------------------------------------------------------------------------

/// Check if byte is a PDF whitespace character.
fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\0' | b'\x0C')
}

/// Check if byte is a PDF delimiter.
fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// Skip whitespace and comments.
fn skip_whitespace(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() {
        if is_whitespace(data[pos]) {
            pos += 1;
        } else if data[pos] == b'%' {
            // Skip comment until end of line
            while pos < data.len() && data[pos] != b'\n' && data[pos] != b'\r' {
                pos += 1;
            }
        } else {
            break;
        }
    }
    pos
}

/// Parse a PDF object starting at position `pos`.
/// Returns the parsed object and the position after it.
pub fn parse_object(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> {
    let pos = skip_whitespace(data, pos);
    if pos >= data.len() {
        return Err(Error::PdfParse("Unexpected end of data".into()));
    }

    match data[pos] {
        b'/' => parse_name(data, pos),
        b'(' => parse_literal_string(data, pos),
        b'<' => parse_hex_or_dict(data, pos),
        b'[' => parse_array(data, pos),
        b't' => {
            // true
            if data.len() >= pos + 4 && &data[pos..pos + 4] == b"true" {
                let end = pos + 4;
                if end >= data.len() || is_whitespace(data[end]) || is_delimiter(data[end]) {
                    Ok((PdfObject::Bool(true), end))
                } else {
                    Err(Error::PdfParse(format!("Invalid token at offset {pos}")))
                }
            } else {
                Err(Error::PdfParse(format!("Invalid token at offset {pos}")))
            }
        }
        b'f' => {
            // false
            if data.len() >= pos + 5 && &data[pos..pos + 5] == b"false" {
                let end = pos + 5;
                if end >= data.len() || is_whitespace(data[end]) || is_delimiter(data[end]) {
                    Ok((PdfObject::Bool(false), end))
                } else {
                    Err(Error::PdfParse(format!("Invalid token at offset {pos}")))
                }
            } else {
                Err(Error::PdfParse(format!("Invalid token at offset {pos}")))
            }
        }
        b'n' => {
            // null
            if data.len() >= pos + 4 && &data[pos..pos + 4] == b"null" {
                let end = pos + 4;
                if end >= data.len() || is_whitespace(data[end]) || is_delimiter(data[end]) {
                    Ok((PdfObject::Null, end))
                } else {
                    Err(Error::PdfParse(format!("Invalid token at offset {pos}")))
                }
            } else {
                Err(Error::PdfParse(format!("Invalid token at offset {pos}")))
            }
        }
        b if b.is_ascii_digit() || b == b'+' || b == b'-' || b == b'.' => parse_number(data, pos),
        _ => Err(Error::PdfParse(format!(
            "Unexpected byte '{}' at offset {pos}",
            data[pos] as char
        ))),
    }
}

/// Parse a number (integer or real), with look-ahead for references and indirect objects.
fn parse_number(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> {
    let start = pos;
    let mut p = pos;
    let mut has_dot = false;

    // optional sign
    if p < data.len() && (data[p] == b'+' || data[p] == b'-') {
        p += 1;
    }
    // digits and optional dot
    while p < data.len() && (data[p].is_ascii_digit() || data[p] == b'.') {
        if data[p] == b'.' {
            has_dot = true;
        }
        p += 1;
    }

    let token = &data[start..p];
    if token.is_empty() || token == b"+" || token == b"-" || token == b"." {
        return Err(Error::PdfParse(format!("Invalid number at offset {start}")));
    }

    if has_dot {
        let s = std::str::from_utf8(token)
            .map_err(|_| Error::PdfParse(format!("Invalid number at offset {start}")))?;
        let val: f64 = s
            .parse()
            .map_err(|_| Error::PdfParse(format!("Invalid real number at offset {start}")))?;
        return Ok((PdfObject::Real(val), p));
    }

    let s = std::str::from_utf8(token)
        .map_err(|_| Error::PdfParse(format!("Invalid number at offset {start}")))?;
    let val: i64 = s
        .parse()
        .map_err(|_| Error::PdfParse(format!("Invalid integer at offset {start}")))?;

    // Look-ahead for "N G R" (reference) or "N G obj" (indirect object).
    // Only valid when val >= 0 and is an integer without sign prefix.
    if val >= 0 && (data[start] != b'+') {
        let saved = p;
        let p2 = skip_whitespace(data, p);
        // Try to parse a second integer (generation number)
        if p2 < data.len() && data[p2].is_ascii_digit() {
            let gen_start = p2;
            let mut p3 = p2;
            while p3 < data.len() && data[p3].is_ascii_digit() {
                p3 += 1;
            }
            let gen_token = &data[gen_start..p3];
            if let Ok(gen_s) = std::str::from_utf8(gen_token) {
                if let Ok(gen) = gen_s.parse::<u32>() {
                    let p4 = skip_whitespace(data, p3);
                    // Check for 'R'
                    if p4 < data.len() && data[p4] == b'R' {
                        let after_r = p4 + 1;
                        if after_r >= data.len()
                            || is_whitespace(data[after_r])
                            || is_delimiter(data[after_r])
                        {
                            return Ok((PdfObject::Reference(val as u32, gen as u16), after_r));
                        }
                    }
                    // Check for 'obj'
                    if p4 + 3 <= data.len() && &data[p4..p4 + 3] == b"obj" {
                        let after_obj = p4 + 3;
                        if after_obj >= data.len()
                            || is_whitespace(data[after_obj])
                            || is_delimiter(data[after_obj])
                        {
                            // Parse the inner object
                            let (inner, inner_end) = parse_object(data, after_obj)?;
                            // Skip to endobj
                            let e = skip_whitespace(data, inner_end);
                            if e + 6 <= data.len() && &data[e..e + 6] == b"endobj" {
                                return Ok((inner, e + 6));
                            }
                            // endobj missing – still return what we got
                            return Ok((inner, inner_end));
                        }
                    }
                }
            }
        }
        // Not a reference or indirect object – revert
        p = saved;
    }

    Ok((PdfObject::Integer(val), p))
}

/// Parse a name object (/Name).
fn parse_name(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> {
    debug_assert_eq!(data[pos], b'/');
    let mut p = pos + 1; // skip '/'
    let mut name = Vec::new();

    while p < data.len() && !is_whitespace(data[p]) && !is_delimiter(data[p]) {
        if data[p] == b'#' && p + 2 < data.len() {
            // Hex escape #XX
            let hi = hex_digit(data[p + 1]);
            let lo = hex_digit(data[p + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                name.push(h << 4 | l);
                p += 3;
                continue;
            }
        }
        name.push(data[p]);
        p += 1;
    }

    Ok((PdfObject::Name(name), p))
}

/// Parse a literal string ((text)).
fn parse_literal_string(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> {
    debug_assert_eq!(data[pos], b'(');
    let mut p = pos + 1;
    let mut depth = 1u32;
    let mut result = Vec::new();

    while p < data.len() && depth > 0 {
        match data[p] {
            b'(' => {
                depth += 1;
                result.push(b'(');
                p += 1;
            }
            b')' => {
                depth -= 1;
                if depth > 0 {
                    result.push(b')');
                }
                p += 1;
            }
            b'\\' => {
                p += 1;
                if p >= data.len() {
                    break;
                }
                match data[p] {
                    b'n' => {
                        result.push(b'\n');
                        p += 1;
                    }
                    b'r' => {
                        result.push(b'\r');
                        p += 1;
                    }
                    b't' => {
                        result.push(b'\t');
                        p += 1;
                    }
                    b'b' => {
                        result.push(0x08);
                        p += 1;
                    }
                    b'f' => {
                        result.push(0x0C);
                        p += 1;
                    }
                    b'(' => {
                        result.push(b'(');
                        p += 1;
                    }
                    b')' => {
                        result.push(b')');
                        p += 1;
                    }
                    b'\\' => {
                        result.push(b'\\');
                        p += 1;
                    }
                    b'\r' => {
                        // line continuation
                        p += 1;
                        if p < data.len() && data[p] == b'\n' {
                            p += 1;
                        }
                    }
                    b'\n' => {
                        // line continuation
                        p += 1;
                    }
                    c if c.is_ascii_digit() && c <= b'7' => {
                        // Octal escape \ddd (1-3 digits)
                        let mut val: u8 = c - b'0';
                        p += 1;
                        if p < data.len() && data[p] >= b'0' && data[p] <= b'7' {
                            val = val * 8 + (data[p] - b'0');
                            p += 1;
                            if p < data.len() && data[p] >= b'0' && data[p] <= b'7' {
                                val = val * 8 + (data[p] - b'0');
                                p += 1;
                            }
                        }
                        result.push(val);
                    }
                    other => {
                        // Unknown escape – just include the character
                        result.push(other);
                        p += 1;
                    }
                }
            }
            other => {
                result.push(other);
                p += 1;
            }
        }
    }

    if depth != 0 {
        return Err(Error::PdfParse(format!(
            "Unterminated literal string at offset {pos}"
        )));
    }

    Ok((PdfObject::Str(result), p))
}

/// Parse a hex string or dictionary based on look-ahead.
fn parse_hex_or_dict(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> {
    debug_assert_eq!(data[pos], b'<');
    if pos + 1 < data.len() && data[pos + 1] == b'<' {
        parse_dict(data, pos)
    } else {
        parse_hex_string(data, pos)
    }
}

/// Parse a hex string <hex>.
fn parse_hex_string(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> {
    debug_assert_eq!(data[pos], b'<');
    let mut p = pos + 1;
    let mut hex_chars = Vec::new();

    while p < data.len() && data[p] != b'>' {
        if is_whitespace(data[p]) {
            p += 1;
            continue;
        }
        hex_chars.push(data[p]);
        p += 1;
    }

    if p >= data.len() {
        return Err(Error::PdfParse(format!(
            "Unterminated hex string at offset {pos}"
        )));
    }
    p += 1; // skip '>'

    // Pad odd length with 0
    if hex_chars.len() % 2 != 0 {
        hex_chars.push(b'0');
    }

    let mut result = Vec::with_capacity(hex_chars.len() / 2);
    for chunk in hex_chars.chunks(2) {
        let hi = hex_digit(chunk[0]).ok_or_else(|| {
            Error::PdfParse(format!(
                "Invalid hex digit '{}' at offset {pos}",
                chunk[0] as char
            ))
        })?;
        let lo = hex_digit(chunk[1]).ok_or_else(|| {
            Error::PdfParse(format!(
                "Invalid hex digit '{}' at offset {pos}",
                chunk[1] as char
            ))
        })?;
        result.push(hi << 4 | lo);
    }

    Ok((PdfObject::Str(result), p))
}

/// Parse a dictionary <<...>>, potentially followed by a stream.
fn parse_dict(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> {
    // Skip '<<'
    let mut p = pos + 2;
    let mut dict = PdfDict::new();

    loop {
        p = skip_whitespace(data, p);
        if p >= data.len() {
            return Err(Error::PdfParse(format!(
                "Unterminated dictionary at offset {pos}"
            )));
        }
        // Check for '>>'
        if p + 1 < data.len() && data[p] == b'>' && data[p + 1] == b'>' {
            p += 2;
            break;
        }
        // Key must be a name
        if data[p] != b'/' {
            return Err(Error::PdfParse(format!(
                "Expected name key in dictionary at offset {p}"
            )));
        }
        let (key_obj, key_end) = parse_name(data, p)?;
        let key = match key_obj {
            PdfObject::Name(n) => n,
            _ => unreachable!(),
        };
        // Value
        let (val, val_end) = parse_object(data, key_end)?;
        dict.insert(key, val);
        p = val_end;
    }

    // Check if followed by 'stream'
    let saved = p;
    let sp = skip_whitespace(data, p);
    if sp + 6 <= data.len() && &data[sp..sp + 6] == b"stream" {
        // Must be followed by \r\n or \n
        let mut stream_start = sp + 6;
        if stream_start < data.len() && data[stream_start] == b'\r' {
            stream_start += 1;
        }
        if stream_start < data.len() && data[stream_start] == b'\n' {
            stream_start += 1;
        }

        // Determine stream length
        let length = dict
            .get(b"Length".as_slice())
            .and_then(|o| o.as_i64())
            .filter(|&l| l >= 0);

        let (raw_data, end_pos) = if let Some(len) = length {
            let len = len as usize;
            let end = stream_start + len;
            if end > data.len() {
                // Length is wrong, fall back to searching
                find_endstream(data, stream_start)?
            } else {
                // Verify endstream follows
                let ep = skip_whitespace(data, end);
                if ep + 9 <= data.len() && &data[ep..ep + 9] == b"endstream" {
                    (data[stream_start..end].to_vec(), ep + 9)
                } else {
                    // Length was wrong, search for endstream
                    find_endstream(data, stream_start)?
                }
            }
        } else {
            find_endstream(data, stream_start)?
        };

        return Ok((PdfObject::Stream(PdfStream { dict, raw_data }), end_pos));
    }

    // Not a stream, revert position
    Ok((PdfObject::Dict(dict), saved))
}

/// Find `endstream` marker and return (raw_data, position after endstream).
fn find_endstream(data: &[u8], start: usize) -> Result<(Vec<u8>, usize)> {
    let marker = b"endstream";
    for i in start..data.len().saturating_sub(marker.len() - 1) {
        if &data[i..i + marker.len()] == marker {
            // Trim trailing whitespace from stream data
            let mut end = i;
            while end > start && (data[end - 1] == b'\r' || data[end - 1] == b'\n') {
                end -= 1;
            }
            return Ok((data[start..end].to_vec(), i + marker.len()));
        }
    }
    Err(Error::PdfParse(format!(
        "Missing endstream marker starting from offset {start}"
    )))
}

/// Parse an array [...].
fn parse_array(data: &[u8], pos: usize) -> Result<(PdfObject, usize)> {
    debug_assert_eq!(data[pos], b'[');
    let mut p = pos + 1;
    let mut elements = Vec::new();

    loop {
        p = skip_whitespace(data, p);
        if p >= data.len() {
            return Err(Error::PdfParse(format!(
                "Unterminated array at offset {pos}"
            )));
        }
        if data[p] == b']' {
            p += 1;
            break;
        }
        let (obj, obj_end) = parse_object(data, p)?;
        elements.push(obj);
        p = obj_end;
    }

    Ok((PdfObject::Array(elements), p))
}

/// Convert a hex digit character to its numeric value.
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_integer() {
        let (obj, _) = parse_object(b"42 ", 0).unwrap();
        assert_eq!(obj.as_i64(), Some(42));
    }

    #[test]
    fn test_parse_negative_integer() {
        let (obj, _) = parse_object(b"-17 ", 0).unwrap();
        assert_eq!(obj.as_i64(), Some(-17));
    }

    #[test]
    fn test_parse_real() {
        let (obj, _) = parse_object(b"3.14 ", 0).unwrap();
        assert!((obj.as_f64().unwrap() - 3.14).abs() < 0.001);
    }

    #[test]
    fn test_parse_bool_true() {
        let (obj, _) = parse_object(b"true ", 0).unwrap();
        assert_eq!(obj, PdfObject::Bool(true));
    }

    #[test]
    fn test_parse_bool_false() {
        let (obj, _) = parse_object(b"false ", 0).unwrap();
        assert_eq!(obj, PdfObject::Bool(false));
    }

    #[test]
    fn test_parse_null() {
        let (obj, _) = parse_object(b"null ", 0).unwrap();
        assert!(matches!(obj, PdfObject::Null));
    }

    #[test]
    fn test_parse_name() {
        let (obj, _) = parse_object(b"/Type ", 0).unwrap();
        assert_eq!(obj.as_name(), Some(b"Type".as_slice()));
    }

    #[test]
    fn test_parse_name_with_hex_escape() {
        let (obj, _) = parse_object(b"/A#20B ", 0).unwrap();
        assert_eq!(obj.as_name(), Some(b"A B".as_slice()));
    }

    #[test]
    fn test_parse_literal_string() {
        let (obj, _) = parse_object(b"(Hello World) ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(b"Hello World".as_slice()));
    }

    #[test]
    fn test_parse_literal_string_escaped() {
        let (obj, _) = parse_object(b"(Hello\\nWorld) ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(b"Hello\nWorld".as_slice()));
    }

    #[test]
    fn test_parse_literal_string_nested_parens() {
        let (obj, _) = parse_object(b"(Hello (World)) ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(b"Hello (World)".as_slice()));
    }

    #[test]
    fn test_parse_hex_string() {
        let (obj, _) = parse_object(b"<48656C6C6F> ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(b"Hello".as_slice()));
    }

    #[test]
    fn test_parse_hex_string_odd_length() {
        let (obj, _) = parse_object(b"<ABC> ", 0).unwrap();
        assert_eq!(obj.as_str_bytes(), Some(&[0xAB, 0xC0][..]));
    }

    #[test]
    fn test_parse_array() {
        let (obj, _) = parse_object(b"[1 2 3] ", 0).unwrap();
        let arr = obj.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_i64(), Some(1));
        assert_eq!(arr[2].as_i64(), Some(3));
    }

    #[test]
    fn test_parse_dict() {
        let (obj, _) = parse_object(b"<< /Type /Page /Count 5 >> ", 0).unwrap();
        let dict = obj.as_dict().unwrap();
        assert_eq!(
            dict_get(dict, b"Type").unwrap().as_name(),
            Some(b"Page".as_slice())
        );
        assert_eq!(dict_get(dict, b"Count").unwrap().as_i64(), Some(5));
    }

    #[test]
    fn test_parse_reference() {
        let (obj, _) = parse_object(b"10 0 R ", 0).unwrap();
        assert_eq!(obj.as_reference(), Some((10, 0)));
    }

    #[test]
    fn test_parse_indirect_object() {
        let (obj, _) = parse_object(b"5 0 obj\n<< /Type /Page >>\nendobj ", 0).unwrap();
        let dict = obj.as_dict().unwrap();
        assert_eq!(
            dict_get(dict, b"Type").unwrap().as_name(),
            Some(b"Page".as_slice())
        );
    }

    #[test]
    fn test_parse_empty_name() {
        let (obj, _) = parse_object(b"/ ", 0).unwrap();
        assert_eq!(obj.as_name(), Some(b"".as_slice()));
    }

    #[test]
    fn test_parse_empty_array() {
        let (obj, _) = parse_object(b"[] ", 0).unwrap();
        assert_eq!(obj.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_parse_nested_array() {
        let (obj, _) = parse_object(b"[[1 2] [3 4]] ", 0).unwrap();
        let arr = obj.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_skip_comments() {
        let (obj, _) = parse_object(b"% this is a comment\n42 ", 0).unwrap();
        assert_eq!(obj.as_i64(), Some(42));
    }
}
