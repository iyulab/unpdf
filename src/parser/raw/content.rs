//! PDF content stream parser.

use super::tokenizer::MAX_NESTING_DEPTH;
use crate::error::Result;
use crate::parser::backend::{ContentOp, PdfValue};

/// Parse a content stream into a sequence of operations.
pub fn parse_content_stream(data: &[u8]) -> Result<Vec<ContentOp>> {
    let mut ops = Vec::new();
    let mut operand_stack: Vec<PdfValue> = Vec::new();
    let len = data.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        if is_whitespace(data[i]) {
            i += 1;
            continue;
        }

        // Skip PDF comments (% until end of line)
        if data[i] == b'%' {
            while i < len && data[i] != b'\n' && data[i] != b'\r' {
                i += 1;
            }
            continue;
        }

        // Literal string (...)
        if data[i] == b'(' {
            let (val, next) = parse_literal_string(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Hex string <...>
        if data[i] == b'<' && i + 1 < len && data[i + 1] != b'<' {
            let (val, next) = parse_hex_string(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Array [...]
        if data[i] == b'[' {
            let (val, next) = parse_array(data, i, 1);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Name /Foo
        if data[i] == b'/' {
            let (val, next) = parse_name(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Number (integer or real), or negative sign
        if data[i] == b'-' || data[i] == b'+' || data[i] == b'.' || data[i].is_ascii_digit() {
            let (val, next) = parse_number(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Alphabetic token → could be operator or keyword (true/false/null)
        if is_operator_start(data[i]) {
            let start = i;
            // An operator is a run of regular characters: letters, and digits after the
            // first (`d0`, `d1` in a Type 3 glyph procedure).
            while i < len && data[i].is_ascii_alphanumeric() {
                i += 1;
            }
            // Check for * suffix (T*, b*, B*)
            if i < len && data[i] == b'*' {
                i += 1;
            }

            let token = &data[start..i];
            let token_str = std::str::from_utf8(token).unwrap_or("");

            match token_str {
                "true" | "false" | "null" => {
                    operand_stack.push(PdfValue::Other);
                }
                "BI" => {
                    // An inline image (ISO 32000-1 §8.9.7): its dictionary, `ID`, its data,
                    // `EI`. The body between `BI` and `EI` is the operator's one operand;
                    // reading it is the backend's (`PdfBackend::inline_image`).
                    let (body, next) = inline_image_body(data, i);
                    i = next;
                    operand_stack.clear();
                    ops.push(ContentOp::new(
                        "BI".to_string(),
                        vec![PdfValue::Str(body.to_vec())],
                    ));
                }
                _ => {
                    // It's an operator
                    ops.push(ContentOp::new(
                        token_str.to_string(),
                        std::mem::take(&mut operand_stack),
                    ));
                }
            }
            continue;
        }

        // Special single-char operators: ' and "
        if data[i] == b'\'' || data[i] == b'"' {
            let op = (data[i] as char).to_string();
            i += 1;
            ops.push(ContentOp::new(op, std::mem::take(&mut operand_stack)));
            continue;
        }

        // Skip unknown bytes
        i += 1;
    }

    Ok(ops)
}

fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0 | 12)
}

fn is_operator_start(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    ) || is_whitespace(b)
}

/// Parse a literal string starting at `(`, returns (PdfValue, next_index).
fn parse_literal_string(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip opening '('
    let mut result = Vec::new();
    let mut depth = 1;
    let len = data.len();

    while i < len && depth > 0 {
        match data[i] {
            b'(' => {
                depth += 1;
                result.push(b'(');
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth > 0 {
                    result.push(b')');
                }
                i += 1;
            }
            b'\\' if i + 1 < len => {
                i += 1;
                match data[i] {
                    b'n' => {
                        result.push(b'\n');
                        i += 1;
                    }
                    b'r' => {
                        result.push(b'\r');
                        i += 1;
                    }
                    b't' => {
                        result.push(b'\t');
                        i += 1;
                    }
                    b'b' => {
                        result.push(8); // backspace
                        i += 1;
                    }
                    b'f' => {
                        result.push(12); // form feed
                        i += 1;
                    }
                    b'(' => {
                        result.push(b'(');
                        i += 1;
                    }
                    b')' => {
                        result.push(b')');
                        i += 1;
                    }
                    b'\\' => {
                        result.push(b'\\');
                        i += 1;
                    }
                    b'0'..=b'7' => {
                        let (byte, end) = super::tokenizer::octal_escape(data, i);
                        result.push(byte);
                        i = end;
                    }
                    b'\r' => {
                        // Backslash + CR (+ optional LF) = line continuation
                        i += 1;
                        if i < len && data[i] == b'\n' {
                            i += 1;
                        }
                    }
                    b'\n' => {
                        // Backslash + LF = line continuation
                        i += 1;
                    }
                    _ => {
                        // Unknown escape, just include the character
                        result.push(data[i]);
                        i += 1;
                    }
                }
            }
            _ => {
                result.push(data[i]);
                i += 1;
            }
        }
    }

    (PdfValue::Str(result), i)
}

/// Parse a hex string starting at `<`, returns (PdfValue, next_index).
fn parse_hex_string(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip '<'
    let len = data.len();
    let mut hex_chars = Vec::new();

    while i < len && data[i] != b'>' {
        if !is_whitespace(data[i]) {
            hex_chars.push(data[i]);
        }
        i += 1;
    }
    if i < len {
        i += 1; // skip '>'
    }

    // Pad odd-length hex with trailing 0
    if hex_chars.len() % 2 != 0 {
        hex_chars.push(b'0');
    }

    let mut result = Vec::with_capacity(hex_chars.len() / 2);
    for pair in hex_chars.chunks(2) {
        let hi = hex_digit(pair[0]);
        let lo = hex_digit(pair[1]);
        result.push((hi << 4) | lo);
    }

    (PdfValue::Str(result), i)
}

fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

/// Parse a name starting at `/`, returns (PdfValue, next_index).
fn parse_name(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip '/'
    let len = data.len();
    let mut name = Vec::new();

    while i < len && !is_whitespace(data[i]) && !is_delimiter(data[i]) {
        if data[i] == b'#' && i + 2 < len {
            // Hex escape in name
            let hi = hex_digit(data[i + 1]);
            let lo = hex_digit(data[i + 2]);
            name.push((hi << 4) | lo);
            i += 3;
        } else {
            name.push(data[i]);
            i += 1;
        }
    }

    (PdfValue::Name(name), i)
}

/// Parse a number (integer or real), returns (PdfValue, next_index).
fn parse_number(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start;
    let len = data.len();
    let mut has_dot = false;

    // Sign
    if i < len && (data[i] == b'-' || data[i] == b'+') {
        i += 1;
    }

    // Check for leading dot
    if i < len && data[i] == b'.' {
        has_dot = true;
        i += 1;
    }

    while i < len && (data[i].is_ascii_digit() || data[i] == b'.') {
        if data[i] == b'.' {
            has_dot = true;
        }
        i += 1;
    }

    let s = std::str::from_utf8(&data[start..i]).unwrap_or("0");

    if has_dot {
        let val: f32 = s.parse().unwrap_or(0.0);
        (PdfValue::Real(val), i)
    } else {
        let val: i64 = s.parse().unwrap_or(0);
        (PdfValue::Integer(val), i)
    }
}

/// Parse an array starting at `[`, `depth` arrays deep, returns (PdfValue, next_index).
fn parse_array(data: &[u8], start: usize, depth: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip '['
    let len = data.len();
    let mut elements = Vec::new();

    while i < len {
        // Skip whitespace
        if is_whitespace(data[i]) {
            i += 1;
            continue;
        }

        if data[i] == b']' {
            i += 1;
            break;
        }

        // Literal string
        if data[i] == b'(' {
            let (val, next) = parse_literal_string(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Hex string
        if data[i] == b'<' && i + 1 < len && data[i + 1] != b'<' {
            let (val, next) = parse_hex_string(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Nested array
        if data[i] == b'[' && depth < MAX_NESTING_DEPTH {
            let (val, next) = parse_array(data, i, depth + 1);
            elements.push(val);
            i = next;
            continue;
        }

        // Name
        if data[i] == b'/' {
            let (val, next) = parse_name(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Number
        if data[i] == b'-' || data[i] == b'+' || data[i] == b'.' || data[i].is_ascii_digit() {
            let (val, next) = parse_number(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Keywords (true/false/null) inside arrays
        if data[i].is_ascii_alphabetic() {
            let token_start = i;
            while i < len && data[i].is_ascii_alphabetic() {
                i += 1;
            }
            let token = std::str::from_utf8(&data[token_start..i]).unwrap_or("");
            match token {
                "true" | "false" | "null" => {
                    elements.push(PdfValue::Other);
                }
                _ => {
                    // Names without / in arrays are unusual; treat as Other
                    elements.push(PdfValue::Other);
                }
            }
            continue;
        }

        // Skip unknown
        i += 1;
    }

    (PdfValue::Array(elements), i)
}

/// The body of the inline image whose `BI` ends at `start` — everything up to its `EI`, the
/// dictionary and `ID` included — and the position just past `EI`.
///
/// `ID` is the first one that stands as a token: after whitespace or the end of a dictionary
/// value (`>>`, `]`, `)`), before whitespace. `EI` is the first after it that stands between
/// whitespace and whitespace, a delimiter or the end of the stream.
fn inline_image_body(data: &[u8], start: usize) -> (&[u8], usize) {
    let len = data.len();
    let mut i = start;

    while i + 1 < len {
        let preceded =
            i == 0 || is_whitespace(data[i - 1]) || matches!(data[i - 1], b'>' | b']' | b')');
        let followed = i + 2 >= len || is_whitespace(data[i + 2]);
        if data[i] == b'I' && data[i + 1] == b'D' && preceded && followed {
            i += 2;
            break;
        }
        i += 1;
    }

    while i + 1 < len {
        if data[i] == b'E'
            && data[i + 1] == b'I'
            && (i == 0 || is_whitespace(data[i - 1]))
            && (i + 2 >= len || is_whitespace(data[i + 2]) || is_delimiter(data[i + 2]))
        {
            return (&data[start..i], i + 2);
        }
        i += 1;
    }

    (&data[start.min(len)..], len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operator_may_end_in_digits() {
        let ops = parse_content_stream(b"1000 0 d0 1000 0 0 0 750 750 d1").unwrap();
        let names: Vec<&str> = ops.iter().map(|op| op.operator.as_str()).collect();
        assert_eq!(names, ["d0", "d1"]);
        assert_eq!((ops[0].operands.len(), ops[1].operands.len()), (2, 6));
    }

    #[test]
    fn test_parse_simple_text_ops() {
        let data = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops.len(), 5);
        assert_eq!(ops[0].operator, "BT");
        assert_eq!(ops[0].operands.len(), 0);
        assert_eq!(ops[1].operator, "Tf");
        assert_eq!(ops[1].operands.len(), 2);
        assert_eq!(ops[2].operator, "Td");
        assert_eq!(ops[3].operator, "Tj");
        assert_eq!(ops[4].operator, "ET");
    }

    /// The content stream reads octal escapes as the object parser does: `\777` keeps its low
    /// byte, and `\8` is no escape (it read as byte 8).
    #[test]
    fn literal_string_octal_escapes_match_the_object_parser() {
        let ops = parse_content_stream(b"BT (\\777\\8) Tj ET").unwrap();
        assert!(
            matches!(&ops[1].operands[0], PdfValue::Str(s) if s == b"\xff8"),
            "{:?}",
            ops[1].operands
        );
    }

    #[test]
    fn test_parse_tj_array() {
        let data = b"BT [(Hello) -100 (World)] TJ ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops.len(), 3); // BT, TJ, ET
        let tj = &ops[1];
        assert_eq!(tj.operator, "TJ");
        assert_eq!(tj.operands.len(), 1); // one array operand
    }

    #[test]
    fn test_parse_graphics_ops() {
        let data = b"q 1 0 0 1 72 720 cm Q";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[0].operator, "q");
        assert_eq!(ops[1].operator, "cm");
        assert_eq!(ops[1].operands.len(), 6);
        assert_eq!(ops[2].operator, "Q");
    }

    #[test]
    fn test_parse_real_numbers() {
        let data = b"0.5 0.5 0.5 rg";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[0].operator, "rg");
        assert_eq!(ops[0].operands.len(), 3);
    }

    #[test]
    fn test_empty_content_stream() {
        let ops = parse_content_stream(b"").unwrap();
        assert!(ops.is_empty());
    }

    #[test]
    fn an_array_nested_past_the_bound_is_cut_there() {
        fn depth(value: &PdfValue) -> usize {
            match value {
                PdfValue::Array(items) => 1 + items.first().map_or(0, depth),
                _ => 0,
            }
        }
        for nesting in [MAX_NESTING_DEPTH, MAX_NESTING_DEPTH + 1] {
            let data = format!("{}{} TJ (x) Tj", "[".repeat(nesting), "]".repeat(nesting));
            let ops = parse_content_stream(data.as_bytes()).unwrap();
            let names: Vec<&str> = ops.iter().map(|op| op.operator.as_str()).collect();
            assert_eq!(names, ["TJ", "Tj"]);
            assert_eq!(depth(&ops[0].operands[0]), MAX_NESTING_DEPTH);
        }
    }

    #[test]
    fn test_tstar_operator() {
        let data = b"BT T* ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[1].operator, "T*");
    }
}
